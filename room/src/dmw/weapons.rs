// SPDX-License-Identifier: GPL-3.0-or-later
//! Doom-side weapon, inventory and ammo glue around `WeaponSim` (sheet 08 sections 5.3-5.10).
//!
//! Split in two layers:
//!
//! - pure helpers with unit tests: [`WeaponMapping`], [`scale_pickup`], [`scale_thrust`],
//!   [`auto_switch_target`], [`MuzzleState`];
//! - the engine-facing parts that read and write `PlayerT` (`think`, `level_start`,
//!   `bring_up`, `after_load`). They run inside `with_state` and only touch plain Doom
//!   memory there; every Doom *function* they need (`P_SetMobjState`, `P_NoiseAlert`,
//!   `P_BringUpWeapon`, sheet 09/12 calls) is returned as an [`Action`] that `hooks.rs`
//!   executes after the borrow is released (sheet 08 section 5.1, re-entrancy).
//!
//! Doom's `weaponowned[]` and `ammo[]` stay authoritative for ownership and pools; the MW2
//! inventory follows them every tic ([`sync_ownership`]).

use std::collections::BTreeMap;
use std::ffi::c_int;

use dmw_format::convert::{doom_angle_to_mw2_yaw, doom_fixed_to_mw2, doom_velocity_to_mw2};
use dmw_format::{DmwWeaponId, DoomAmmo, DoomWeapon, Pack};
use dmw_weapons::{
    Inventory, LevelStart, MeleeRequest, ShotRequest, SimEvent, SoundCue, TicButtons, TicInput,
    TicOutput, WeaponSim,
};
use log::{debug, error, warn};
use weapon_iw4::{FireWeaponKind, fire_weapon_kind};

use super::config::{AmmoCfg, DmwConfig, WeaponsCfg};
use super::consts::{tic_msec, PITCH_UNITS_PER_DEGREE, SHADOW_VELOCITY_SCALE};
use super::ext::{
    PlayerExt, EXT_BT_ADS, EXT_BT_ATTACK, EXT_BT_HOLD_BREATH, EXT_BT_MELEE, EXT_BT_RELOAD,
    EXT_BT_SPRINT, EXT_BT_SWAP_LAST, PITCH_LIMIT_Q,
};
use super::DmwState;
use crate::doom::d_player::{PlayerT, NUMWEAPONS};
use crate::doom::p_telept::mobj_t;

const TARGET: &str = "dmw::sim";

/// `wp_fist`.
pub const WP_FIST: DoomWeapon = 0;
/// `wp_pistol`.
pub const WP_PISTOL: DoomWeapon = 1;
/// `wp_shotgun`.
pub const WP_SHOTGUN: DoomWeapon = 2;
/// `wp_chaingun`.
pub const WP_CHAINGUN: DoomWeapon = 3;
/// `wp_missile`.
pub const WP_MISSILE: DoomWeapon = 4;
/// `wp_plasma`.
pub const WP_PLASMA: DoomWeapon = 5;
/// `wp_bfg`.
pub const WP_BFG: DoomWeapon = 6;
/// `wp_chainsaw`.
pub const WP_CHAINSAW: DoomWeapon = 7;
/// `wp_supershotgun`.
pub const WP_SUPERSHOTGUN: DoomWeapon = 8;
/// `wp_nochange` (= `NUMWEAPONS`).
pub const WP_NOCHANGE: DoomWeapon = NUMWEAPONS as DoomWeapon;

/// `am_clip`.
pub const AM_CLIP: DoomAmmo = 0;
/// `am_shell`.
pub const AM_SHELL: DoomAmmo = 1;
/// `am_cell`.
pub const AM_CELL: DoomAmmo = 2;
/// `am_misl`.
pub const AM_MISL: DoomAmmo = 3;
/// `am_noammo`.
pub const AM_NOAMMO: DoomAmmo = 5;

/// `PST_LIVE`.
const PST_LIVE: c_int = 0;
/// `GameMode_t` values used by `P_CheckAmmo`.
const SHAREWARE: c_int = 0;
const COMMERCIAL: c_int = 2;

/// Doom weapon names in `weapontype_t` order, for logs.
pub const DOOM_WEAPON_NAMES: [&str; NUMWEAPONS] = [
    "fist",
    "pistol",
    "shotgun",
    "chaingun",
    "missile",
    "plasma",
    "bfg",
    "chainsaw",
    "supershotgun",
];

// ---------------------------------------------------------------------------
// Mapping (section 5.3)
// ---------------------------------------------------------------------------

/// Doom weapon slots to MW2 weapons.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponMapping {
    /// Index = `weapontype_t`; `None` = vanilla Doom weapon.
    pub by_doom: [Option<DmwWeaponId>; NUMWEAPONS],
    /// `DmwWeaponId.0` -> Doom weapon (the first Doom weapon mapped to it wins).
    pub doom_of: BTreeMap<u16, DoomWeapon>,
    /// `DmwWeaponId.0` -> Doom ammo pool (`weaponinfo[doom_of].ammo`, `am_clip` for fist and
    /// chainsaw slots).
    pub ammo_of: BTreeMap<u16, DoomAmmo>,
}

/// The `[weapons]` names in `weapontype_t` order.
pub fn config_names(cfg: &WeaponsCfg) -> [&str; NUMWEAPONS] {
    [
        &cfg.fist,
        &cfg.pistol,
        &cfg.shotgun,
        &cfg.chaingun,
        &cfg.missile,
        &cfg.plasma,
        &cfg.bfg,
        &cfg.chainsaw,
        &cfg.supershotgun,
    ]
}

/// Up to three pack weapon names (or aliases) that look like `name`.
pub fn suggestions(name: &str, pack: &Pack) -> Vec<String> {
    let mut known: Vec<&str> = pack.manifest.weapons.iter().map(|w| w.name.as_str()).collect();
    known.extend(pack.manifest.aliases.keys().map(String::as_str));
    let lower = name.to_ascii_lowercase();
    let stem = lower.split('_').next().unwrap_or("");
    let mut scored: Vec<(usize, &str)> = known
        .into_iter()
        .map(|k| {
            let d = levenshtein(&lower, &k.to_ascii_lowercase());
            let bonus = !stem.is_empty() && k.contains(stem);
            (if bonus { d.saturating_sub(3) } else { d }, k)
        })
        .filter(|&(d, _)| d <= 4)
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(3).map(|(_, k)| k.to_owned()).collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

impl WeaponMapping {
    /// Builds the mapping from `[weapons]` names. `doom_ammo[w]` is `weaponinfo[w].ammo`.
    ///
    /// A name that is not in the pack (aliases included), has no viewmodel, or is not a
    /// bullet/missile/grenade-launcher weapon leaves the slot vanilla; each case is returned
    /// as a warning line (the caller logs it).
    pub fn build(
        names: [&str; NUMWEAPONS],
        pack: &Pack,
        doom_ammo: [DoomAmmo; NUMWEAPONS],
    ) -> (WeaponMapping, Vec<String>) {
        let mut m = WeaponMapping::default();
        let mut warnings = Vec::new();
        for (w, name) in names.iter().enumerate() {
            let slot = DOOM_WEAPON_NAMES[w];
            if name.eq_ignore_ascii_case("vanilla") {
                continue;
            }
            let Some(id) = pack.weapon_id(name) else {
                let near = suggestions(name, pack);
                warnings.push(format!(
                    "[weapons] {slot} = \"{name}\" is not in the pack; slot stays vanilla{}",
                    if near.is_empty() {
                        String::new()
                    } else {
                        format!(" (close matches: {})", near.join(", "))
                    }
                ));
                continue;
            };
            let has_viewmodel = pack.entry(id).is_some_and(|e| e.has_viewmodel);
            if !has_viewmodel {
                warnings.push(format!(
                    "[weapons] {slot} = \"{name}\" has no viewmodel; slot stays vanilla"
                ));
                continue;
            }
            let usable = pack.try_facts(id).is_some_and(|f| {
                matches!(
                    fire_weapon_kind(f.combat.weap_type, f.combat.weap_class),
                    Some(
                        FireWeaponKind::Bullet
                            | FireWeaponKind::Missile
                            | FireWeaponKind::GrenadeLauncher
                    )
                )
            });
            if !usable {
                warnings.push(format!(
                    "[weapons] {slot} = \"{name}\" is not a bullet, missile or grenade-launcher \
                     weapon; slot stays vanilla"
                ));
                continue;
            }
            m.by_doom[w] = Some(id);
        }
        m.rebuild_reverse(doom_ammo);
        (m, warnings)
    }

    fn rebuild_reverse(&mut self, doom_ammo: [DoomAmmo; NUMWEAPONS]) {
        self.doom_of.clear();
        self.ammo_of.clear();
        for (w, id) in self.by_doom.iter().enumerate() {
            let Some(id) = id else { continue };
            if self.doom_of.contains_key(&id.0) {
                continue;
            }
            self.doom_of.insert(id.0, w as DoomWeapon);
            let ammo = if doom_ammo[w] == AM_NOAMMO {
                AM_CLIP
            } else {
                doom_ammo[w]
            };
            self.ammo_of.insert(id.0, ammo);
        }
    }

    /// The MW2 weapon of Doom weapon `w`, `None` for vanilla or an invalid index.
    pub fn mapped(&self, w: DoomWeapon) -> Option<DmwWeaponId> {
        usize::try_from(w)
            .ok()
            .and_then(|i| self.by_doom.get(i).copied())
            .flatten()
    }

    /// Every distinct mapped MW2 weapon, by id.
    pub fn ids(&self) -> Vec<DmwWeaponId> {
        self.doom_of.keys().map(|&k| DmwWeaponId(k)).collect()
    }

    /// Makes every slot mapped to `id` vanilla (viewmodel failed to load).
    pub fn demote(&mut self, id: DmwWeaponId, doom_ammo: [DoomAmmo; NUMWEAPONS]) {
        for slot in &mut self.by_doom {
            if *slot == Some(id) {
                *slot = None;
            }
        }
        self.rebuild_reverse(doom_ammo);
    }

    /// One line per slot for the startup `info!` summary.
    pub fn describe(&self, pack: &Pack) -> String {
        let mut parts = Vec::new();
        for (w, id) in self.by_doom.iter().enumerate() {
            let target = match id {
                Some(id) => pack
                    .try_facts(*id)
                    .map_or_else(|| format!("#{}", id.0), |f| f.name.clone()),
                None => "vanilla".to_owned(),
            };
            parts.push(format!("{}={target}", DOOM_WEAPON_NAMES[w]));
        }
        parts.join(" ")
    }
}

/// `weaponinfo[w].ammo` for every Doom weapon.
pub fn doom_weapon_ammo() -> [DoomAmmo; NUMWEAPONS] {
    let mut out = [AM_NOAMMO; NUMWEAPONS];
    for (w, slot) in out.iter_mut().enumerate() {
        *slot = crate::doom::d_items::weaponinfo[w].ammo;
    }
    out
}

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

/// Pickup scaling (section 5.4): `ceil(num * scale)`, at least 1 when `num > 0`. `num <= 0`
/// (0 is Doom's "half a clip" for dropped weapons) is returned unchanged.
pub fn scale_pickup(num: i32, scale: f32) -> i32 {
    if num <= 0 || !scale.is_finite() {
        return num;
    }
    let scaled = (num as f32 * scale).ceil();
    (scaled.min(i32::MAX as f32) as i32).max(1)
}

/// Movement scaling (section 5.9) for one thrust value (fixed point, `cmd.*move * 2048`),
/// rounded toward zero.
pub fn scale_thrust(thrust: i32, scale: f32) -> i32 {
    if !scale.is_finite() {
        return thrust;
    }
    (thrust as f64 * f64::from(scale)).trunc() as i32
}

/// The `(forward, side)` scales for a tic output: forward uses `move_speed_scale`, side the
/// same without the sprint factor (sprint only boosts forward).
pub fn move_scales(out: &TicOutput, sprint_speed_scale: f32) -> (f32, f32) {
    let forward = out.move_speed_scale;
    let side = if out.sprinting && sprint_speed_scale > 0.0 {
        forward / sprint_speed_scale
    } else {
        forward
    };
    (forward, side)
}

/// What [`auto_switch_target`] knows about the player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AmmoView {
    /// `weaponowned[w] != 0`.
    pub owned: [bool; NUMWEAPONS],
    /// Doom pools.
    pub pools: [i32; 4],
    /// For MW2-mapped weapons: magazine > 0 or pool > 0. `None` for vanilla weapons, which use
    /// Doom's own thresholds.
    pub mw2_has_ammo: [Option<bool>; NUMWEAPONS],
    /// `gamemode`.
    pub gamemode: c_int,
}

/// The weapon to switch to when the held MW2 weapon is completely empty, in `P_CheckAmmo`'s
/// exact priority order: plasma, SSG, chaingun, shotgun, pistol, chainsaw, missile, BFG, fist.
pub fn auto_switch_target(v: &AmmoView) -> DoomWeapon {
    let pool = |a: DoomAmmo| v.pools[a as usize];
    let has = |w: DoomWeapon, vanilla: bool| v.mw2_has_ammo[w as usize].unwrap_or(vanilla);
    let owned = |w: DoomWeapon| v.owned[w as usize];
    if owned(WP_PLASMA) && has(WP_PLASMA, pool(AM_CELL) != 0) && v.gamemode != SHAREWARE {
        WP_PLASMA
    } else if owned(WP_SUPERSHOTGUN)
        && has(WP_SUPERSHOTGUN, pool(AM_SHELL) > 2)
        && v.gamemode == COMMERCIAL
    {
        WP_SUPERSHOTGUN
    } else if owned(WP_CHAINGUN) && has(WP_CHAINGUN, pool(AM_CLIP) != 0) {
        WP_CHAINGUN
    } else if owned(WP_SHOTGUN) && has(WP_SHOTGUN, pool(AM_SHELL) != 0) {
        WP_SHOTGUN
    } else if owned(WP_PISTOL) && has(WP_PISTOL, pool(AM_CLIP) != 0) {
        WP_PISTOL
    } else if owned(WP_CHAINSAW) {
        WP_CHAINSAW
    } else if owned(WP_MISSILE) && has(WP_MISSILE, pool(AM_MISL) != 0) {
        WP_MISSILE
    } else if owned(WP_BFG) && has(WP_BFG, pool(AM_CELL) > 40) && v.gamemode != SHAREWARE {
        WP_BFG
    } else {
        WP_FIST
    }
}

/// Doom's muzzle light (`extralight` 2, 1, 0 over three tics) and the flash patch timer for
/// sheet 11 (section 5.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MuzzleState {
    /// `player.extralight` written at the end of the current tic.
    pub light: u8,
    /// Tics the flash patch stays up (sheet 11 reads it).
    pub flash_tics: u8,
}

impl MuzzleState {
    /// Start of an MW2 tic: advance the sequence.
    pub fn begin_tic(&mut self) {
        self.light = self.light.saturating_sub(1);
        self.flash_tics = self.flash_tics.saturating_sub(1);
    }

    /// A `MuzzleFlash` event.
    pub fn fire(&mut self) {
        self.light = 2;
        self.flash_tics = 2;
    }
}

/// Decodes `ExtTiccmd::buttons` (00 section 8.1).
pub fn decode_buttons(bits: u16) -> TicButtons {
    TicButtons {
        ads: bits & EXT_BT_ADS != 0,
        reload: bits & EXT_BT_RELOAD != 0,
        sprint: bits & EXT_BT_SPRINT != 0,
        melee: bits & EXT_BT_MELEE != 0,
        hold_breath: bits & EXT_BT_HOLD_BREATH != 0,
        swap_last: bits & EXT_BT_SWAP_LAST != 0,
        attack: bits & EXT_BT_ATTACK != 0,
    }
}

// ---------------------------------------------------------------------------
// Driver state and plan
// ---------------------------------------------------------------------------

/// Who animates the weapon this level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Owner {
    /// Doom's psprites (a vanilla weapon is up, or MW2 mode is off).
    #[default]
    Vanilla,
    /// `WeaponSim` (an MW2 weapon is up, raising, or dropping).
    Mw2,
}

/// Per-level bookkeeping of the tic driver.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Driver {
    /// Who animates the weapon.
    pub owner: Owner,
    /// The Doom weapon whose MW2 weapon is wanted in hand.
    pub target_doom: Option<DoomWeapon>,
    /// The vanilla weapon the MW2 weapon is being dropped for (`request_vanilla` sent).
    pub vanilla_requested: Option<DoomWeapon>,
    /// Next MW2 tic is the first of the level (or after a load): no pickup messages.
    pub first_tic: bool,
    /// `readyweapon` seen at the previous tic.
    pub current_doom: Option<DoomWeapon>,
    /// The Doom weapon held before `current_doom` (`EXT_BT_SWAP_LAST` target).
    pub last_doom: Option<DoomWeapon>,
    /// Extended buttons of the previous tic (edge detection).
    pub prev_buttons: u16,
    /// DOOMMW2 ammo maxima are applied to the player (restore vanilla when leaving MW2 mode).
    pub maxima_applied: bool,
    /// `(episode, map)` of the last active level start.
    pub last_level: Option<(c_int, c_int)>,
    /// The level restarted on the same map (death); `on_respawn` runs on the first tic unless
    /// `on_new_game` follows.
    pub maybe_respawn: bool,
    /// Viewmodels were checked and prepared this session.
    pub viewmodels_ready: bool,
    /// Tics on which MW2 owned the weapon (`test-hooks` counters, T8.8).
    pub handled_tics: u64,
    /// `on_respawn` calls (T8.10).
    pub respawns: u32,
}

/// A Doom-side effect to run after the `DmwState` borrow is released.
#[derive(Debug)]
pub enum Action {
    /// Player attack frame and sheet 09 `fire` (which also makes the noise alert when
    /// `[balance] alert_monsters_on_fire` is set).
    Shot(ShotRequest),
    /// Player punch frame and sheet 09 `melee`.
    Melee(MeleeRequest),
    /// Sheet 12 sound.
    Sound(SoundCue),
    /// Sheet 12 pickup message.
    Acquired(DmwWeaponId),
    /// Sheet 10 animation tic (its notetrack sounds are played after it).
    ViewmodelTic {
        /// The sim's presentation snapshot after this tic.
        view: dmw_weapons::ViewState,
        /// `tic_msec(leveltime)`.
        msec: i32,
    },
    /// The MW2 drop finished: raise the vanilla weapon through Doom's own `P_BringUpWeapon`.
    BringUpVanilla,
    /// Switch MW2 mode off (internal inconsistency).
    Fail(String),
}

/// What `think` decided.
#[derive(Debug, Default)]
pub struct Plan {
    /// MW2 owns the weapon this tic (skip `P_MovePsprites`).
    pub handled: bool,
    /// Effects to run, in order.
    pub actions: Vec<Action>,
}

// ---------------------------------------------------------------------------
// Engine-facing parts (run inside `with_state`)
// ---------------------------------------------------------------------------

/// Clip size of `id` from the pack (0 when unknown).
fn clip_size(pack: Option<&Pack>, id: DmwWeaponId) -> i32 {
    pack.and_then(|p| p.try_facts(id))
        .map_or(0, |f| f.combat.clip_size)
}

/// DOOMMW2 maxima (doubled with a backpack); clamps the pools. Idempotent.
pub fn enforce_maxima(p: &mut PlayerT, ammo: &AmmoCfg) {
    let mult = if p.backpack != 0 { 2 } else { 1 };
    for i in 0..4 {
        let want = ammo.max[i].saturating_mul(mult).max(0);
        if p.maxammo[i] != want {
            debug!(target: TARGET, "maxammo[{i}] {} -> {want}", p.maxammo[i]);
            p.maxammo[i] = want;
        }
        p.ammo[i] = p.ammo[i].clamp(0, want);
    }
}

/// Vanilla maxima (global `maxammo[]`, doubled with a backpack), as `G_PlayerReborn` and
/// `P_GiveBackpack` would have left them.
pub fn restore_vanilla_maxima(p: &mut PlayerT) {
    let mult = if p.backpack != 0 { 2 } else { 1 };
    for i in 0..4 {
        // SAFETY: plain read of the global table on the engine thread.
        let base = unsafe { crate::doom::p_inter::maxammo[i] };
        p.maxammo[i] = base * mult;
        p.ammo[i] = p.ammo[i].min(p.maxammo[i]);
    }
}

/// Section 5.4: the MW2 inventory follows Doom's `weaponowned[]`. Newly owned mapped weapons
/// get a full magazine and are returned (for the pickup message); weapons Doom no longer owns
/// are dropped with their magazines.
pub fn sync_ownership(
    inv: &mut Inventory,
    mapping: &WeaponMapping,
    pack: Option<&Pack>,
    owned: &[c_int; NUMWEAPONS],
) -> Vec<DmwWeaponId> {
    let mut wanted: Vec<DmwWeaponId> = Vec::new();
    for (w, &own) in owned.iter().enumerate() {
        if own == 0 {
            continue;
        }
        if let Some(id) = mapping.by_doom[w] {
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
    }
    let mut added = Vec::new();
    for &id in &wanted {
        if !inv.owns(id) {
            inv.give(id, clip_size(pack, id));
            added.push(id);
        }
    }
    inv.owned.retain(|id| wanted.contains(id));
    inv.magazines.retain(|&k, _| wanted.contains(&DmwWeaponId(k)));
    if inv.last_held.is_some_and(|id| !wanted.contains(&id)) {
        inv.last_held = None;
    }
    added
}

/// `true` when `p` owns Doom weapon `w`.
fn owns(p: &PlayerT, w: DoomWeapon) -> bool {
    usize::try_from(w)
        .ok()
        .and_then(|i| p.weaponowned.get(i))
        .is_some_and(|&o| o != 0)
}

/// Clears both psprites (nothing vanilla may draw while an MW2 weapon is held).
fn clear_psprites(p: &mut PlayerT) {
    for psp in &mut p.psprites {
        psp.state = std::ptr::null_mut();
    }
}

/// Builds the sim input for this tic.
///
/// # Safety
/// `p.mo` must point to the player's live map object.
unsafe fn tic_input(
    p: &PlayerT,
    pe: &PlayerExt,
    requested: Option<DmwWeaponId>,
    leveltime: c_int,
) -> TicInput {
    // SAFETY: caller contract; the opaque handle is the full `mobj_t` (C layout guard tests).
    let mo = unsafe { &*(p.mo as *const mobj_t) };
    let vel = |v: c_int| doom_velocity_to_mw2(v) * SHADOW_VELOCITY_SCALE;
    TicInput {
        tic: leveltime.max(0) as u32,
        alive: p.playerstate == PST_LIVE,
        buttons: decode_buttons(pe.cmd.buttons),
        requested_weapon: requested,
        yaw_deg: doom_angle_to_mw2_yaw(mo.angle),
        pitch_deg: pe.pitch_q as f32 / PITCH_UNITS_PER_DEGREE,
        prev_yaw_deg: doom_angle_to_mw2_yaw(pe.prev_yaw),
        prev_pitch_deg: pe.prev_pitch_q as f32 / PITCH_UNITS_PER_DEGREE,
        origin_mw2: [
            doom_fixed_to_mw2(mo.x),
            doom_fixed_to_mw2(mo.y),
            doom_fixed_to_mw2(mo.z.saturating_add(p.viewheight)),
        ],
        velocity_mw2: [vel(mo.momx), vel(mo.momy), vel(mo.momz)],
        on_ground: mo.z <= mo.floorz,
        forwardmove: p.cmd.forwardmove,
        sidemove: p.cmd.sidemove,
        pools: p.ammo,
    }
}

/// Section 5.5: one tic of the driver for the console player.
///
/// # Safety
/// `p` is the console player of a running level (`p.mo` live); `pe` is its `PLAYER_EXT` slot.
pub unsafe fn think(
    s: &mut DmwState,
    p: &mut PlayerT,
    pe: &mut PlayerExt,
    leveltime: c_int,
    gamemode: c_int,
) -> Plan {
    let mut plan = Plan::default();
    let pack = s.pack.clone();
    let Some(sim) = s.sim.as_mut() else {
        return plan;
    };
    let d = &mut s.driver;

    enforce_maxima(p, &s.config.ammo);
    d.maxima_applied = true;

    let first = std::mem::take(&mut d.first_tic);
    if std::mem::take(&mut d.maybe_respawn) {
        sim.on_respawn();
        d.respawns += 1;
        debug!(target: TARGET, "respawn: first-raise memory cleared");
    }
    let added = sync_ownership(sim.inventory_mut(), &s.mapping, pack.as_deref(), &p.weaponowned);
    for id in added {
        if first {
            continue;
        }
        debug!(target: TARGET, "pickup: MW2 weapon {} added with a full magazine", id.0);
        plan.actions.push(Action::Acquired(id));
    }

    // readyweapon history for EXT_BT_SWAP_LAST.
    if d.current_doom != Some(p.readyweapon) {
        if d.current_doom.is_some() {
            d.last_doom = d.current_doom;
        }
        d.current_doom = Some(p.readyweapon);
    }
    let buttons = pe.cmd.buttons;
    let swap_edge = buttons & EXT_BT_SWAP_LAST != 0 && d.prev_buttons & EXT_BT_SWAP_LAST == 0;
    d.prev_buttons = buttons;
    if swap_edge {
        if let Some(last) = d.last_doom.filter(|&w| w != p.readyweapon && owns(p, w)) {
            debug!(target: TARGET, "swap: to last weapon {}", DOOM_WEAPON_NAMES[last as usize]);
            p.pendingweapon = last;
        }
    }

    if d.owner == Owner::Vanilla {
        // Doom's psprites run; `on_bring_up_weapon` takes over when Doom raises a mapped weapon.
        return plan;
    }

    // Section 5.6: translate Doom's pending weapon.
    let pw = p.pendingweapon;
    if pw != WP_NOCHANGE && owns(p, pw) {
        if s.mapping.mapped(pw).is_some() {
            if d.target_doom != Some(pw) || d.vanilla_requested.is_some() {
                debug!(target: TARGET, "switch: to {}", DOOM_WEAPON_NAMES[pw as usize]);
            }
            d.target_doom = Some(pw);
            d.vanilla_requested = None;
        } else if d.vanilla_requested != Some(pw) {
            debug!(target: TARGET, "switch: to vanilla {}", DOOM_WEAPON_NAMES[pw as usize]);
            sim.request_vanilla(pw);
            d.vanilla_requested = Some(pw);
        }
    }
    let target = d.target_doom.unwrap_or(p.readyweapon);
    let requested = if d.vanilla_requested.is_some() {
        None
    } else {
        s.mapping.mapped(target)
    };

    // SAFETY: caller contract.
    let input = unsafe { tic_input(p, pe, requested, leveltime) };
    let out = sim.tic(&input);

    for (i, pool) in p.ammo.iter_mut().enumerate() {
        *pool = out.pools[i].clamp(0, p.maxammo[i].max(0));
    }

    // Reflect the weapon in hand into Doom.
    if let Some(held) = out.held {
        let doom = if s.mapping.mapped(target) == Some(held) {
            Some(target)
        } else if s.mapping.mapped(p.readyweapon) == Some(held) {
            Some(p.readyweapon)
        } else {
            s.mapping.doom_of.get(&held.0).copied()
        };
        if let Some(w) = doom {
            p.readyweapon = w;
        }
        if p.pendingweapon == p.readyweapon
            || (s.mapping.mapped(p.pendingweapon) == Some(held)
                && sim.switch_state() == dmw_weapons::switch::SwitchState::Idle)
        {
            p.pendingweapon = WP_NOCHANGE;
        }
    }
    clear_psprites(p);

    let view = sim.view_state();
    let mut kick = out.view_kick_deg;
    if view.overlay_active {
        // Scope sway moves the world (sheet 12 section 5.4).
        kick[0] += view.gun_angle_offset[0];
        kick[1] += view.gun_angle_offset[1];
    }
    pe.view_kick_deg = kick;
    pe.zoom_hfov_deg = view.zoom_hfov_deg;

    s.muzzle.begin_tic();
    for ev in &out.events {
        #[cfg(feature = "test-hooks")]
        s.recorder.record(leveltime, ev);
        match ev {
            SimEvent::Shot(req) => plan.actions.push(Action::Shot(req.clone())),
            SimEvent::Melee(req) => plan.actions.push(Action::Melee(req.clone())),
            SimEvent::Sound(cue) => plan.actions.push(Action::Sound(cue.clone())),
            SimEvent::MuzzleFlash { .. } => s.muzzle.fire(),
            SimEvent::EmptyClick => {
                if let Some(w) = empty_switch(sim, &s.mapping, p, gamemode) {
                    debug!(target: TARGET, "auto-switch: out of ammo, to {}", DOOM_WEAPON_NAMES[w as usize]);
                    p.pendingweapon = w;
                }
            }
            SimEvent::ReloadStarted { empty } => {
                debug!(target: TARGET, "reload started (empty {empty})");
            }
            SimEvent::ReloadFinished => debug!(target: TARGET, "reload finished"),
            SimEvent::RaiseStarted(id) => debug!(target: TARGET, "raise started: {}", id.0),
            SimEvent::DropFinished(id) => debug!(target: TARGET, "drop finished: {}", id.0),
        }
    }
    p.extralight = c_int::from(s.muzzle.light);

    if out.weapon_lowered_for_vanilla {
        match d.vanilla_requested.take() {
            Some(w) => {
                debug!(target: TARGET, "hand-off: vanilla {} raises", DOOM_WEAPON_NAMES[w as usize]);
                p.readyweapon = w;
                p.pendingweapon = WP_NOCHANGE;
                p.extralight = 0;
                s.muzzle = MuzzleState::default();
                pe.view_kick_deg = [0.0; 3];
                pe.zoom_hfov_deg = None;
                d.owner = Owner::Vanilla;
                d.target_doom = None;
                plan.actions.push(Action::BringUpVanilla);
            }
            None => plan.actions.push(Action::Fail(
                "the sim lowered the weapon for a vanilla hand-off nobody requested".to_owned(),
            )),
        }
    }

    plan.actions.push(Action::ViewmodelTic {
        view,
        msec: tic_msec(leveltime.max(0) as u32),
    });
    s.last_out = Some(out);
    d.handled_tics += 1;
    plan.handled = true;
    plan
}

/// Section 5.6 auto-switch: `Some(weapon)` when the held MW2 weapon has an empty magazine and
/// an empty pool.
fn empty_switch(
    sim: &WeaponSim,
    mapping: &WeaponMapping,
    p: &PlayerT,
    gamemode: c_int,
) -> Option<DoomWeapon> {
    let held = sim.view_state().weapon?;
    let pool_of = |id: DmwWeaponId| {
        mapping
            .ammo_of
            .get(&id.0)
            .and_then(|&a| usize::try_from(a).ok())
            .and_then(|i| p.ammo.get(i).copied())
            .unwrap_or(0)
    };
    let inv = sim.inventory();
    if inv.magazine(held) > 0 || pool_of(held) > 0 {
        return None;
    }
    let mut v = AmmoView {
        pools: p.ammo,
        gamemode,
        ..AmmoView::default()
    };
    for w in 0..NUMWEAPONS {
        v.owned[w] = p.weaponowned[w] != 0;
        v.mw2_has_ammo[w] = mapping
            .mapped(w as DoomWeapon)
            .map(|id| inv.magazine(id) > 0 || pool_of(id) > 0);
    }
    Some(auto_switch_target(&v)).filter(|&w| w != p.readyweapon)
}

/// Section 5.6 vanilla -> MW2 (`P_BringUpWeapon` hook): `true` when the pending weapon is
/// mapped and MW2 raises it instead of the psprite.
pub fn bring_up(s: &mut DmwState, p: &mut PlayerT) -> bool {
    if s.sim.is_none() {
        return false;
    }
    let pw = if p.pendingweapon == WP_NOCHANGE {
        p.readyweapon
    } else {
        p.pendingweapon
    };
    if s.mapping.mapped(pw).is_none() {
        return false;
    }
    debug!(target: TARGET, "hand-off: MW2 raises {}", DOOM_WEAPON_NAMES[pw as usize]);
    p.readyweapon = pw;
    p.pendingweapon = WP_NOCHANGE;
    clear_psprites(p);
    let d = &mut s.driver;
    d.owner = Owner::Mw2;
    d.target_doom = Some(pw);
    d.vanilla_requested = None;
    true
}

/// Section 5.10 level start for an active level. Returns the warnings/errors already logged.
pub fn level_start(s: &mut DmwState, p: &mut PlayerT, episode: c_int, map: c_int, seed: u32) {
    let Some(pack) = s.pack.clone() else { return };

    if !s.driver.viewmodels_ready {
        s.driver.viewmodels_ready = true;
        let doom_ammo = doom_weapon_ammo();
        for id in s.mapping.ids() {
            if let Err(e) = pack.load_viewmodel(id) {
                let name = pack.try_facts(id).map_or("?", |f| f.name.as_str());
                error!(target: TARGET, "viewmodel of {name} failed to load ({e}); its slots stay vanilla");
                s.mapping.demote(id, doom_ammo);
            }
        }
        super::viewmodel::prepare_all(s);
    }

    enforce_maxima(p, &s.config.ammo);
    let same_map = s.driver.last_level == Some((episode, map));
    let Some(sim) = s.sim.as_mut() else { return };
    let mut inventory = if same_map {
        // Death (or a new game / load on the same map): a fresh life.
        Inventory::default()
    } else {
        sim.inventory().clone()
    };
    let _ = sync_ownership(&mut inventory, &s.mapping, Some(&pack), &p.weaponowned);
    sim.start_level(LevelStart {
        pack: &pack,
        seed,
        inventory,
        ammo_map: s.mapping.ammo_of.clone(),
        sprint_speed_scale: s.config.balance.sprint_speed_scale,
        weapon_move_scales: s.config.balance.weapon_move_scales,
        viewmodel_fov: s.config.render.viewmodel_fov,
    });

    let handled_tics = s.driver.handled_tics;
    let respawns = s.driver.respawns;
    s.driver = Driver {
        maxima_applied: true,
        last_level: Some((episode, map)),
        maybe_respawn: same_map,
        viewmodels_ready: true,
        handled_tics,
        respawns,
        first_tic: true,
        current_doom: Some(p.readyweapon),
        ..Driver::default()
    };
    take_over_if_mapped(s, p);
    s.muzzle = MuzzleState::default();
    s.last_out = None;
    p.extralight = 0;
}

/// After a level start or a load: MW2 owns the weapon when `readyweapon` is mapped.
fn take_over_if_mapped(s: &mut DmwState, p: &mut PlayerT) {
    if s.mapping.mapped(p.readyweapon).is_some() {
        clear_psprites(p);
        s.driver.owner = Owner::Mw2;
        s.driver.target_doom = Some(p.readyweapon);
        p.pendingweapon = WP_NOCHANGE;
    } else {
        s.driver.owner = Owner::Vanilla;
        s.driver.target_doom = None;
    }
}

/// Section 5.11 after `P_UnArchive*`: restores the sim from the extension block, or (vanilla
/// save, malformed block) lets the next tic rebuild the inventory with full magazines.
pub fn after_load(
    s: &mut DmwState,
    p: &mut PlayerT,
    pe: &mut PlayerExt,
    block: Option<dmw_weapons::SimSave>,
    pitch_q: i32,
) {
    let Some(sim) = s.sim.as_mut() else { return };
    match block {
        Some(save) => {
            sim.restore(save);
            pe.pitch_q = pitch_q.clamp(-PITCH_LIMIT_Q, PITCH_LIMIT_Q);
            pe.prev_pitch_q = pe.pitch_q;
        }
        None => {
            *sim.inventory_mut() = Inventory::default();
            sim.on_respawn();
        }
    }
    enforce_maxima(p, &s.config.ammo);
    s.driver.first_tic = true;
    s.driver.maybe_respawn = false;
    s.driver.vanilla_requested = None;
    s.driver.current_doom = Some(p.readyweapon);
    s.driver.last_doom = None;
    take_over_if_mapped(s, p);
    s.muzzle = MuzzleState::default();
    s.last_out = None;
    p.extralight = 0;
}

/// `true` when the MW2 sim draws the weapon this level (sheet 11 may use it to keep vanilla
/// psprites hidden, for example the death lowering `P_DropWeapon` starts).
pub fn mw2_owns_weapon(s: &DmwState) -> bool {
    s.driver.owner == Owner::Mw2
}

/// Startup summary helper: the pickup scale for `ammo` (`None` for `am_noammo`).
pub fn pickup_scale(cfg: &DmwConfig, ammo: DoomAmmo) -> Option<f32> {
    usize::try_from(ammo)
        .ok()
        .and_then(|i| cfg.ammo.pickup_scale.get(i).copied())
}

/// Warns about each mapping problem.
pub fn log_mapping_warnings(warnings: &[String]) {
    for w in warnings {
        warn!(target: TARGET, "{w}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOOM_AMMO: [DoomAmmo; NUMWEAPONS] = [5, 0, 1, 0, 3, 2, 2, 5, 1];

    fn default_mapping(pack: &Pack) -> WeaponMapping {
        let cfg = WeaponsCfg::default();
        let (m, warnings) = WeaponMapping::build(config_names(&cfg), pack, DOOM_AMMO);
        assert!(warnings.is_empty(), "{warnings:?}");
        m
    }

    #[test]
    fn default_mapping_resolves_fixture_aliases() {
        let pack = Pack::fixture();
        let m = default_mapping(&pack);
        let id = |n: &str| pack.weapon_id(n);
        assert_eq!(m.mapped(WP_FIST), None);
        assert_eq!(m.mapped(WP_CHAINSAW), None);
        assert_eq!(m.mapped(WP_PISTOL), id("fixture_pistol"));
        assert_eq!(m.mapped(WP_SHOTGUN), id("fixture_shotgun"));
        assert_eq!(m.mapped(WP_SUPERSHOTGUN), id("fixture_shotgun"));
        assert_eq!(m.mapped(WP_CHAINGUN), id("fixture_rifle"));
        assert_eq!(m.mapped(WP_PLASMA), id("fixture_rifle"));
        assert_eq!(m.mapped(WP_MISSILE), id("fixture_launcher"));
        assert_eq!(m.mapped(WP_BFG), id("fixture_sniper"));
        assert_eq!(m.mapped(WP_NOCHANGE), None);
        assert_eq!(m.mapped(-1), None);
        // First Doom weapon wins for the reverse maps.
        let rifle = id("fixture_rifle").expect("rifle").0;
        assert_eq!(m.doom_of[&rifle], WP_CHAINGUN);
        assert_eq!(m.ammo_of[&rifle], AM_CLIP);
        let shotgun = id("fixture_shotgun").expect("shotgun").0;
        assert_eq!(m.doom_of[&shotgun], WP_SHOTGUN);
        assert_eq!(m.ammo_of[&shotgun], AM_SHELL);
        assert_eq!(m.ids().len(), 5);
    }

    #[test]
    fn unknown_name_stays_vanilla_with_suggestions() {
        let pack = Pack::fixture();
        let cfg = WeaponsCfg {
            pistol: "fixture_pistl".to_owned(),
            fist: "m4_mp".to_owned(),
            ..WeaponsCfg::default()
        };
        let (m, warnings) = WeaponMapping::build(config_names(&cfg), &pack, DOOM_AMMO);
        assert_eq!(m.mapped(WP_PISTOL), None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("fixture_pistol"), "{}", warnings[0]);
        // A fist slot mapped to a gun uses the bullet pool.
        let rifle = pack.weapon_id("fixture_rifle").expect("rifle");
        assert_eq!(m.mapped(WP_FIST), Some(rifle));
        assert_eq!(m.ammo_of[&rifle.0], AM_CLIP);
    }

    #[test]
    fn demote_makes_every_slot_of_a_weapon_vanilla() {
        let pack = Pack::fixture();
        let mut m = default_mapping(&pack);
        let shotgun = pack.weapon_id("fixture_shotgun").expect("shotgun");
        m.demote(shotgun, DOOM_AMMO);
        assert_eq!(m.mapped(WP_SHOTGUN), None);
        assert_eq!(m.mapped(WP_SUPERSHOTGUN), None);
        assert!(!m.doom_of.contains_key(&shotgun.0));
        assert!(m.describe(&pack).contains("shotgun=vanilla"));
    }

    #[test]
    fn pickup_scaling_rounds_up_with_minimum_one() {
        assert_eq!(scale_pickup(1, 3.0), 3);
        assert_eq!(scale_pickup(5, 0.5), 3);
        assert_eq!(scale_pickup(1, 0.5), 1);
        assert_eq!(scale_pickup(1, 0.0), 1);
        assert_eq!(scale_pickup(4, 1.0), 4);
        assert_eq!(scale_pickup(0, 3.0), 0, "half-clip pickups are not scaled");
        assert_eq!(scale_pickup(-2, 3.0), -2);
        assert_eq!(scale_pickup(2, f32::NAN), 2);
    }

    #[test]
    fn thrust_scaling_truncates_toward_zero() {
        assert_eq!(scale_thrust(50 * 2048, 1.0), 50 * 2048);
        assert_eq!(scale_thrust(50 * 2048, 0.5), 25 * 2048);
        assert_eq!(scale_thrust(-3, 0.5), -1);
        assert_eq!(scale_thrust(3, 0.5), 1);
        assert_eq!(scale_thrust(10, f32::NAN), 10);
        let out = TicOutput {
            move_speed_scale: 1.5,
            sprinting: true,
            ..TicOutput::default()
        };
        assert_eq!(move_scales(&out, 1.5), (1.5, 1.0));
        let walk = TicOutput {
            move_speed_scale: 0.8,
            ..TicOutput::default()
        };
        assert_eq!(move_scales(&walk, 1.5), (0.8, 0.8));
    }

    fn view(owned: &[DoomWeapon], pools: [i32; 4], gamemode: c_int) -> AmmoView {
        let mut v = AmmoView {
            pools,
            gamemode,
            ..AmmoView::default()
        };
        for &w in owned {
            v.owned[w as usize] = true;
        }
        v
    }

    #[test]
    fn auto_switch_follows_p_checkammo_priority() {
        let all = [0, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(auto_switch_target(&view(&all, [1, 3, 1, 1], COMMERCIAL)), WP_PLASMA);
        // Shareware: no plasma, no SSG.
        assert_eq!(auto_switch_target(&view(&all, [1, 3, 1, 1], SHAREWARE)), WP_CHAINGUN);
        assert_eq!(auto_switch_target(&view(&all, [0, 3, 0, 1], COMMERCIAL)), WP_SUPERSHOTGUN);
        assert_eq!(auto_switch_target(&view(&all, [0, 2, 0, 1], COMMERCIAL)), WP_SHOTGUN);
        assert_eq!(auto_switch_target(&view(&all, [5, 0, 0, 1], 1)), WP_CHAINGUN);
        assert_eq!(auto_switch_target(&view(&[0, 1], [5, 0, 0, 0], 1)), WP_PISTOL);
        assert_eq!(auto_switch_target(&view(&[0, 1, 7], [0, 0, 0, 0], 1)), WP_CHAINSAW);
        assert_eq!(auto_switch_target(&view(&[0, 1, 4], [0, 0, 0, 2], 1)), WP_MISSILE);
        assert_eq!(auto_switch_target(&view(&[0, 1, 6], [0, 0, 41, 0], 1)), WP_BFG);
        assert_eq!(auto_switch_target(&view(&[0, 1, 6], [0, 0, 40, 0], 1)), WP_FIST);
        assert_eq!(auto_switch_target(&view(&[0, 1], [0, 0, 0, 0], 1)), WP_FIST);
    }

    #[test]
    fn auto_switch_uses_magazines_for_mw2_weapons() {
        // Chaingun mapped with rounds left in its magazine but an empty pool: still has ammo.
        let mut v = view(&[0, 1, 3], [0, 0, 0, 0], 1);
        v.mw2_has_ammo[WP_CHAINGUN as usize] = Some(true);
        v.mw2_has_ammo[WP_PISTOL as usize] = Some(false);
        assert_eq!(auto_switch_target(&v), WP_CHAINGUN);
        v.mw2_has_ammo[WP_CHAINGUN as usize] = Some(false);
        assert_eq!(auto_switch_target(&v), WP_FIST);
    }

    #[test]
    fn muzzle_light_is_two_one_zero() {
        let mut m = MuzzleState::default();
        m.begin_tic();
        m.fire();
        assert_eq!((m.light, m.flash_tics), (2, 2));
        m.begin_tic();
        assert_eq!((m.light, m.flash_tics), (1, 1));
        m.begin_tic();
        assert_eq!((m.light, m.flash_tics), (0, 0));
        m.begin_tic();
        assert_eq!(m.light, 0);
    }

    #[test]
    fn buttons_decode_every_bit() {
        let b = decode_buttons(0xffff);
        assert!(b.ads && b.reload && b.sprint && b.melee && b.hold_breath && b.swap_last && b.attack);
        assert_eq!(decode_buttons(0), TicButtons::default());
        assert!(decode_buttons(EXT_BT_ATTACK).attack);
    }

    #[test]
    fn ownership_sync_adds_full_magazines_and_drops_lost_weapons() {
        let pack = Pack::fixture();
        let m = default_mapping(&pack);
        let pistol = pack.weapon_id("fixture_pistol").expect("pistol");
        let shotgun = pack.weapon_id("fixture_shotgun").expect("shotgun");
        let mut inv = Inventory::default();
        let mut owned = [0; NUMWEAPONS];
        owned[WP_FIST as usize] = 1;
        owned[WP_PISTOL as usize] = 1;
        assert_eq!(sync_ownership(&mut inv, &m, Some(&pack), &owned), vec![pistol]);
        assert_eq!(inv.magazine(pistol), 12);
        inv.set_magazine(pistol, 3);
        owned[WP_SHOTGUN as usize] = 1;
        owned[WP_SUPERSHOTGUN as usize] = 1;
        assert_eq!(sync_ownership(&mut inv, &m, Some(&pack), &owned), vec![shotgun]);
        assert_eq!(inv.magazine(pistol), 3, "existing magazine kept");
        assert_eq!(inv.magazine(shotgun), 8);
        inv.last_held = Some(shotgun);
        owned = [0; NUMWEAPONS];
        owned[WP_PISTOL as usize] = 1;
        assert!(sync_ownership(&mut inv, &m, Some(&pack), &owned).is_empty());
        assert_eq!(inv.owned, vec![pistol]);
        assert!(!inv.magazines.contains_key(&shotgun.0));
        assert_eq!(inv.last_held, None);
    }

    #[test]
    fn maxima_follow_config_and_backpack() {
        // SAFETY: a zeroed PlayerT is a valid value (plain integers and null pointers).
        let mut p: PlayerT = unsafe { std::mem::zeroed() };
        p.ammo = [500, 10, 400, 40];
        let cfg = AmmoCfg::default();
        enforce_maxima(&mut p, &cfg);
        assert_eq!(p.maxammo, [360, 64, 300, 12]);
        assert_eq!(p.ammo, [360, 10, 300, 12]);
        p.backpack = 1;
        enforce_maxima(&mut p, &cfg);
        assert_eq!(p.maxammo, [720, 128, 600, 24]);
        restore_vanilla_maxima(&mut p);
        assert_eq!(p.maxammo, [400, 100, 600, 100]);
        assert_eq!(p.ammo, [360, 10, 300, 12]);
    }
}
