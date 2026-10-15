// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/sim/src/combat.rs (advance_weapon_command) and crates/sim/src/step.rs (pmove_context), Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: single player, single hand, no offhand; driven by Doom tics; ammo from Doom pools.

//! `WeaponSim`: the per-tic driver (sheet 07 §5.1, §5.3).
//!
//! One tic mirrors, for a single player with a single hand and no offhand, what iw4L does per
//! client command: the weapon-related part of `movement_iw4::pmove`
//! (movement_iw4/src/single.rs) followed by `sim::combat::advance_weapon_command`
//! (sim/src/combat.rs:144-1040), then the presentation-side kick/sway advance of
//! render_anim (render_anim/src/occupancy/view_kick.rs:139-203) and placement (sheet 10).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use dmw_format::consts::{DOOM_HFOV, MW2_CG_FOV, tic_msec, tic_server_time};
use dmw_format::{DmwWeaponId, DoomAmmo, DoomWeapon, Pack, WeaponFacts};
use movement_iw4::{
    AdsFracContext, AdsIntentContext, CmdScaleWalkContext, SprintContext, footsteps_bob_cycle,
    get_max_sprint_time, update_ads_frac, update_ads_intent, update_sprint,
};
use playerstate_iw4::{ENTITYNUM_NONE, PlayerState, pm_flags};
use weapon_iw4::{
    AIM_SPREAD_MOVE_SPEED_THRESHOLD_DEFAULT, AimSpreadMotion, AimSpreadState, FireWeaponKind,
    MeleeChargeState, OffhandCmd, PERK_WEAP_RELOAD_MULTIPLIER_DEFAULT, PLAYER_MELEE_RANGE_DEFAULT,
    WEAPCLASS_PISTOL, WeaponCmd, WeaponCombatFacts, WeaponHandState, WeaponState, WeaponTickEvent,
    add_aim_spread_fire, adjust_aim_spread_scale, ammo_table_key, clip_table_key, fire_weapon_kind,
    set_ammo_not_in_clip, set_clip_for_hand, spawn_weapon_hand, update_scope_zoom, weapon_hands,
    weapon_idle_weap_anim,
};

use crate::breath::update_hold_breath;
use crate::events::{MeleeRequest, ShotRequest, SimEvent, SoundCue};
use crate::inventory::{Inventory, SimSave, pool_index};
use crate::placement::{self, PlacementState, ViewInputs};
use crate::rng::DmwRng;
use crate::shadow::{build_usercmd, fresh_player_state, sync_from_input};
use crate::spread::{current_spread_degrees, spread_direction_on_plane};
use crate::switch::{SwitchState, begin_vanilla_drop, is_dropping};
use crate::view_kick::ViewKickState;
use crate::view_sway::ViewSwayState;
use crate::{GunPlacement, LevelStart, TicInput, TicOutput, ViewState};

const LOG: &str = "dmw::sim";

/// iw4L `pmove_context` `player_sprint_time` (sim/src/step.rs:2430).
const PLAYER_SPRINT_TIME_SECONDS: f32 = 4.0;
/// `FireType::DoubleBarrel` (weapon_iw4/src/weaponstate.rs:133).
const FIRE_TYPE_DOUBLE_BARREL: i32 = 5;
/// Pellet cap per shot (sheet 07 §5.4 step 5).
const MAX_PELLETS: i32 = 64;
/// Gun angle offset clamp (sim/src/combat.rs:773).
const GUN_ANGLE_OFFSET_LIMIT: f32 = 45.0;

/// The MW2 weapon simulation for the local player (00 §7.2).
#[derive(Debug)]
pub struct WeaponSim {
    pack: Arc<Pack>,
    /// Index = `DmwWeaponId.0 - 1`, copied from the pack at `new`.
    combat: Vec<WeaponCombatFacts>,
    ps: PlayerState,
    /// Hand 1 always default (no akimbo).
    hands: [WeaponHandState; 2],
    old_buttons: u32,
    /// Last `UserCmd::angles` (combat.rs:198-202 `old_cmd_angles`).
    old_angles: Option<[i32; 3]>,
    kick: ViewKickState,
    sway: ViewSwayState,
    /// `ps.weapon` the kick/sway state belongs to (view_kick.rs:145-158).
    kick_weapon: u32,
    placement: PlacementState,
    placed: GunPlacement,
    gun_angle_offset: [f32; 2],
    spread_deg: f32,
    rng: DmwRng,
    inv: Inventory,
    /// Weapons raised at least once this life (first-raise memory, combat.rs:1910-1923).
    used: BTreeSet<u16>,
    switch: SwitchState,
    /// `weapon_lowered_for_vanilla` to report on the next tic (vanilla request with no gun up).
    pending_lowered: bool,
    ammo_map: BTreeMap<u16, DoomAmmo>,
    sprint_speed_scale: f32,
    weapon_move_scales: bool,
    viewmodel_fov: f32,
    /// Keys already logged by `warn_once` this level.
    warned: BTreeSet<String>,
    last_view: ViewState,
}

/// One tic's per-weapon context, resolved from the facts of the weapon in hand.
struct WeaponCtx<'a> {
    id: DmwWeaponId,
    facts: &'a WeaponFacts,
}

fn weapon_id(w: u32) -> Option<DmwWeaponId> {
    u16::try_from(w).ok().filter(|&v| v != 0).map(DmwWeaponId)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// MW2 horizontal FOV at 4:3 → Doom horizontal FOV giving the same magnification
/// (sheet 07 §5.11): `2·atan(tan(45°)·tan(f/2)/tan(cg_fov/2))`.
fn mw2_fov_to_doom(f: f32) -> f32 {
    let half_doom = (DOOM_HFOV * 0.5).to_radians().tan();
    let ratio = (f * 0.5).to_radians().tan() / (MW2_CG_FOV * 0.5).to_radians().tan();
    2.0 * (half_doom * ratio).atan().to_degrees()
}

/// Bridge rule (sheet 07 §11 item 16): carry the fire-cycle overshoot into the next cycle.
///
/// `weapon_iw4` clamps `weapon_time` at 0 when a cycle expires inside a tic (tick.rs:985-987)
/// and sets `weapon_time = fire_time` when the next shot fires on that tic (tick.rs:1237), so at
/// 28/29 ms tics the part of the tic past the expiry is lost and an 86 ms gun fires every 3 or 4
/// tics (10 shots per 35 tics instead of 11.6). When a shot fires on the tic in which the
/// previous cycle ran out (`Firing`, `0 < weapon_time <= msec` before the tic), the overrun
/// `msec - weapon_time` is subtracted from the new cycle (never below 1 ms). Shots still land on
/// tics, and the average rate equals `1000 / fire_time` shots per second.
fn carry_fire_overshoot(
    hand: &mut WeaponHandState,
    fire_time_ms: i32,
    pre_state: i32,
    pre_weapon_time: i32,
    msec: i32,
) {
    let fresh_cycle = fire_time_ms.max(1);
    if pre_state == WeaponState::Firing as i32
        && pre_weapon_time > 0
        && pre_weapon_time <= msec
        && hand.weapon_time == fresh_cycle
    {
        hand.weapon_time = (fresh_cycle - (msec - pre_weapon_time)).max(1);
    }
}

impl WeaponSim {
    /// Creates the sim; copies the combat facts of every pack weapon. Never fails (the pack was
    /// validated when it was opened).
    pub fn new(pack: Arc<Pack>) -> Self {
        let combat = pack
            .weapon_ids()
            .filter_map(|id| pack.try_facts(id).map(|f| f.combat))
            .collect();
        Self {
            pack,
            combat,
            ps: fresh_player_state(),
            hands: [WeaponHandState::default(); 2],
            old_buttons: 0,
            old_angles: None,
            kick: ViewKickState::default(),
            sway: ViewSwayState::default(),
            kick_weapon: 0,
            placement: PlacementState::default(),
            placed: GunPlacement::default(),
            gun_angle_offset: [0.0; 2],
            spread_deg: 0.0,
            rng: DmwRng::default(),
            inv: Inventory::default(),
            used: BTreeSet::new(),
            switch: SwitchState::Idle,
            pending_lowered: false,
            ammo_map: BTreeMap::new(),
            sprint_speed_scale: 1.5,
            weapon_move_scales: true,
            viewmodel_fov: dmw_format::consts::VIEWMODEL_FOV_DEFAULT,
            warned: BTreeSet::new(),
            last_view: ViewState {
                viewmodel_fov_deg: dmw_format::consts::VIEWMODEL_FOV_DEFAULT,
                ..ViewState::default()
            },
        }
    }

    /// Resets every per-level state and takes the level's inventory and settings
    /// (sheet 07 §5.2). The hand starts empty; the first tic's `requested_weapon` raises the
    /// weapon Doom holds (with its first-raise animation, as `used` is empty).
    pub fn start_level(&mut self, start: LevelStart<'_>) {
        self.rng = DmwRng::from_seed(start.seed);
        self.inv = start.inventory;
        self.ammo_map = start.ammo_map;
        self.sprint_speed_scale = start.sprint_speed_scale;
        self.weapon_move_scales = start.weapon_move_scales;
        self.viewmodel_fov = start.viewmodel_fov;
        self.warned.clear();
        self.reset_life();
        log::info!(
            target: LOG,
            "level start: {} MW2 weapons owned, seed {:#010x}",
            self.inv.owned.len(),
            start.seed
        );
    }

    /// Drops the MW2 weapon, then hands over to Doom's psprites (sheet 07 §5.8). Idempotent.
    pub fn request_vanilla(&mut self, doom_weapon: DoomWeapon) {
        match self.switch {
            SwitchState::ToVanilla { .. } => {
                self.switch = SwitchState::ToVanilla { doom_weapon };
            }
            SwitchState::VanillaHeld => {}
            SwitchState::Idle | SwitchState::ToMw2 { .. } => {
                if self.hands[0].weapon != 0 {
                    log::debug!(target: LOG, "switch: to vanilla weapon {doom_weapon} (dropping)");
                    self.switch = SwitchState::ToVanilla { doom_weapon };
                } else {
                    log::debug!(target: LOG, "switch: to vanilla weapon {doom_weapon} (hand empty)");
                    self.switch = SwitchState::VanillaHeld;
                    self.pending_lowered = true;
                }
            }
        }
    }

    /// New life: clears first-raise memory and resets kick, sway, spread and the hand.
    pub fn on_respawn(&mut self) {
        self.reset_life();
    }

    /// Owned weapons and magazines.
    pub fn inventory(&self) -> &Inventory {
        &self.inv
    }

    /// Owned weapons and magazines, for sheet 08's pickups.
    pub fn inventory_mut(&mut self) -> &mut Inventory {
        &mut self.inv
    }

    /// Presentation snapshot of the last tic.
    pub fn view_state(&self) -> ViewState {
        self.last_view.clone()
    }

    /// Savegame block (sheet 08 §5.11), by weapon name.
    pub fn save(&self) -> SimSave {
        let name = |id: DmwWeaponId| self.pack.try_facts(id).map(|f| f.name.clone());
        SimSave {
            owned: self.inv.owned.iter().filter_map(|&id| name(id)).collect(),
            magazines: self
                .inv
                .magazines
                .iter()
                .filter_map(|(&k, &v)| Some((name(DmwWeaponId(k))?, v)))
                .collect(),
            last_held: self.inv.last_held.and_then(name),
            held: weapon_id(self.hands[0].weapon).and_then(name),
        }
    }

    /// Restores a savegame block: rebuilds the inventory and puts the held weapon up, ready
    /// (sheet 07 §5.12). Kick, sway, spread, ADS, breath, sprint and switch state reset.
    pub fn restore(&mut self, save: SimSave) {
        let resolve = |name: &str| {
            let id = self.pack.weapon_id(name);
            if id.is_none() {
                log::warn!(target: "dmw::save", "saved MW2 weapon {name} is not in the pack; dropped");
            }
            id
        };
        let mut inv = Inventory::default();
        for name in &save.owned {
            if let Some(id) = resolve(name)
                && !inv.owns(id)
            {
                inv.owned.push(id);
            }
        }
        for (name, &rounds) in &save.magazines {
            if let Some(id) = resolve(name) {
                inv.set_magazine(id, rounds);
            }
        }
        inv.last_held = save.last_held.as_deref().and_then(resolve);
        let held = save.held.as_deref().and_then(resolve);
        self.inv = inv;
        self.reset_life();
        if let Some(id) = held
            && let Some(combat) = self.combat_of(id.0.into())
        {
            // spawn_weapon_hand then force Ready (sheet 07 §5.12; sim/src/step.rs:2537).
            let mut hand = spawn_weapon_hand(id.0.into(), &combat, false);
            hand.weaponstate = WeaponState::Ready as i32;
            hand.weapon_time = 0;
            weapon_idle_weap_anim(&mut hand.weap_anim, 0);
            hand.clip = self.inv.magazine(id);
            self.hands[0] = hand;
            self.ps.weapon = u32::from(id.0);
            self.ps.weaponstate_primary = hand.weaponstate;
            self.ps.weap_anim = hand.weap_anim;
            self.used.insert(id.0);
            self.kick_weapon = self.ps.weapon;
        }
        self.last_view = self.build_view(None);
    }

    fn reset_life(&mut self) {
        self.ps = fresh_player_state();
        self.hands = [WeaponHandState::default(); 2];
        self.old_buttons = 0;
        self.old_angles = None;
        self.kick.reset();
        self.sway.reset();
        self.kick_weapon = 0;
        self.placement = PlacementState::default();
        self.placed = GunPlacement::default();
        self.gun_angle_offset = [0.0; 2];
        self.spread_deg = 0.0;
        self.used.clear();
        self.switch = SwitchState::Idle;
        self.pending_lowered = false;
        self.last_view = self.build_view(None);
    }

    fn combat_of(&self, weapon: u32) -> Option<WeaponCombatFacts> {
        let index = usize::try_from(weapon).ok()?.checked_sub(1)?;
        self.combat.get(index).copied()
    }

    fn warn_once(&mut self, key: String, message: &str) {
        if self.warned.insert(key) {
            log::warn!(target: LOG, "{message}");
        }
    }

    /// Doom pool index of `weapon`'s reserve, `None` for `am_noammo` (sheet 07 §6).
    fn pool_of(&mut self, weapon: DmwWeaponId) -> Option<usize> {
        let ammo = self.ammo_map.get(&weapon.0).copied();
        let index = ammo.and_then(pool_index);
        if index.is_none() && ammo != Some(crate::inventory::AM_NOAMMO) {
            self.warn_once(
                format!("pool:{}", weapon.0),
                &format!(
                    "MW2 weapon {} has no Doom ammo pool; treated as am_noammo",
                    weapon.0
                ),
            );
        }
        index
    }

    /// Applies `TicInput::requested_weapon` to the switch state (sheet 07 §5.8).
    fn resolve_request(&mut self, requested: Option<DmwWeaponId>) {
        let Some(t) = requested else {
            return;
        };
        if !self.inv.owns(t) || self.combat_of(t.0.into()).is_none() {
            log::debug!(target: LOG, "requested MW2 weapon {} is not owned; ignored", t.0);
            return;
        }
        let held = self.hands[0].weapon;
        let next = match self.switch {
            SwitchState::ToMw2 { target } if target == t => return,
            SwitchState::Idle if u32::from(t.0) == held => return,
            _ if u32::from(t.0) == held && !is_dropping(&self.hands[0]) => SwitchState::Idle,
            _ => SwitchState::ToMw2 { target: t },
        };
        if next != self.switch {
            log::debug!(target: LOG, "switch: {:?} -> {:?}", self.switch, next);
            self.switch = next;
        }
    }

    /// Runs one Doom tic (sheet 07 §5.3). Infallible.
    #[allow(clippy::too_many_lines)]
    pub fn tic(&mut self, input: &TicInput) -> TicOutput {
        let pack = Arc::clone(&self.pack);
        // msec clamp as advance_weapon_command (sim/src/combat.rs:152).
        let msec = tic_msec(input.tic).clamp(1, 200);
        let frametime = msec as f32 / 1000.0;
        // IW convention: the command's server_time is the end of the command.
        let server_time = tic_server_time(input.tic) + msec;
        let mut events = Vec::new();
        let mut pools = input.pools;
        let mut weapon_lowered_for_vanilla = std::mem::take(&mut self.pending_lowered);

        self.resolve_request(input.requested_weapon);

        let started = self.hands[0].weapon;
        let cmd_weapon = match self.switch {
            SwitchState::ToMw2 { target } => u32::from(target.0),
            SwitchState::VanillaHeld => 0,
            SwitchState::Idle | SwitchState::ToVanilla { .. } => started,
        };

        // ---- Step 1: UserCmd (sheet 07 §5.3.1). iw4L receives it from the client
        // (playerstate_iw4/src/usercmd.rs); `weapon` is the selected weapon.
        let ucmd = build_usercmd(input, server_time, cmd_weapon);
        // combat.rs:198-202: old angles default to this command's angles.
        let old_angles = self.old_angles.unwrap_or(ucmd.angles);
        let old_buttons = self.old_buttons;

        // ---- Step 2: shadow sync (§5.2). pmove stamps command_time (single.rs:78).
        sync_from_input(&mut self.ps, input);
        self.ps.command_time = server_time;
        self.ps.weapons = [0; 15];
        for (slot, id) in self.ps.weapons.iter_mut().zip(&self.inv.owned) {
            *slot = i32::from(id.0);
        }
        self.ps.weapon = started;

        // The facts weapon is the held weapon, else the commanded one (combat.rs:208-214).
        let facts_weapon = if started != 0 { started } else { cmd_weapon };
        let ctx = weapon_id(facts_weapon)
            .and_then(|id| pack.try_facts(id).map(|facts| WeaponCtx { id, facts }));
        // Weapon-def values pmove reads for ps.weapon (sim/src/step.rs:347-390).
        let held_ctx = weapon_id(started).and_then(|id| pack.try_facts(id));

        // Ammo in (§5.9): magazine from the inventory, stock from the Doom pool. The shadow's
        // clip/ammo tables get the same values so `no_ads_when_mag_empty` sees them
        // (combat.rs:262-279 reads them back the same way).
        let ammo_pool = match &ctx {
            Some(c) => self.pool_of(c.id),
            None => None,
        };
        if let Some(c) = &ctx {
            let combat = c.facts.combat;
            self.hands[0].clip = self.inv.magazine(c.id);
            self.hands[0].stock = ammo_pool.map_or(0, |p| pools[p]);
            let clip_key = clip_table_key(combat.clip_index, facts_weapon);
            let ammo_key = ammo_table_key(combat.ammo_index, facts_weapon);
            let _ = set_clip_for_hand(&mut self.ps.ammoclip, clip_key, 0, self.hands[0].clip);
            let _ = set_ammo_not_in_clip(&mut self.ps.ammo, ammo_key, self.hands[0].stock);
        }

        // ---- pmove context (sim/src/step.rs:347-390, pmove_context at step.rs:2413-2504).
        let (move_scale, ads_move_scale, sprint_duration_scale) = held_ctx.map_or(
            (0.0, 0.0, 1.0), // step.rs world.scales_for default (world.rs:2726)
            |f| {
                (
                    f.movement.move_speed_scale,
                    f.movement.ads_move_speed_scale,
                    f.movement.sprint_duration_scale,
                )
            },
        );
        let held_combat = held_ctx.map(|f| f.combat);
        let ads_allowed = held_combat.is_some_and(|f| {
            let mut allow = f.ads_allow_facts();
            allow.clip_index = clip_table_key(allow.clip_index, started);
            weapon_iw4::is_ads_allowed(
                &self.ps,
                &allow,
                &self.ps.ammoclip,
                self.ps.last_weapon_hand.min(1) as u8,
            )
        });
        let overlay_reticle = held_combat.map_or(0, |f| f.overlay_reticle);
        let (ads_in_rate, ads_out_rate, rechamber_while_ads, ads_fire_only) = held_combat
            .map_or((1.0 / 200.0, 1.0 / 200.0, true, false), |f| {
                f.ads_frac_context()
            });

        // ---- Step 4: ADS intent. iw4L runs it before sprint (movement_iw4/src/single.rs:108).
        let _ = update_ads_intent(
            &mut self.ps,
            &ucmd,
            old_buttons,
            AdsIntentContext {
                ads_allowed,
                weapon_def_scope: overlay_reticle != 0,
                sprint_hold_ads: false,
            },
        );

        // ---- Step 3: sprint (single.rs:122; SprintContext values from step.rs:2471-2480).
        let sprint = update_sprint(
            &mut self.ps,
            &ucmd,
            old_buttons,
            SprintContext {
                weapon_max_sprint_time: get_max_sprint_time(
                    sprint_duration_scale,
                    PLAYER_SPRINT_TIME_SECONDS,
                ),
                sprint_forever: false,
                min_sprint_time_seconds: 1.0,
                sprint_delay_seconds: 0.0,
                sprint_forward_minimum: 105,
                stand_up_clear: true,
                sprint_recharge_pause_seconds: 0.0,
            },
        );
        if sprint != movement_iw4::SprintResult::Unchanged {
            log::debug!(target: LOG, "sprint {sprint:?}");
        }

        // ---- Step 5: ADS fraction (single.rs:156; AdsFracContext from step.rs:2486-2494).
        update_ads_frac(
            &mut self.ps,
            msec,
            AdsFracContext {
                aim_down_sight: held_combat.is_some_and(|f| f.aim_down_sight),
                ads_reload_trans_time_ms: held_combat.map_or(0, |f| f.ads_reload_trans_time_ms),
                segmented_reload: held_combat.is_some_and(|f| f.segmented_reload),
                ads_in_rate,
                ads_out_rate,
                rechamber_while_ads,
                ads_fire_only,
            },
        );

        // ---- Step 6: hold breath (single.rs:157, ported in breath.rs).
        update_hold_breath(
            &mut self.ps,
            ucmd.buttons,
            msec,
            held_combat.is_some_and(|f| f.can_hold_breath),
        );

        // ---- Step 2b: bob cycle (single.rs:262-270; CmdScaleWalkContext from
        // step.rs:2440-2449). Doom never advances MW2's bob_cycle; placement reads it.
        footsteps_bob_cycle(
            &mut self.ps,
            msec,
            ucmd.forwardmove,
            ucmd.rightmove,
            false,
            server_time,
            CmdScaleWalkContext {
                player_back_speed_scale: 0.7,
                player_strafe_speed_scale: 0.8,
                player_sprint_speed_scale: 1.5,
                player_last_stand_crawl_speed_scale: 0.15,
                weapon_move_speed_scale: move_scale,
                weapon_ads_move_speed_scale: ads_move_scale,
                shellshock_affects_movement: false,
            },
        );

        // advance_weapon_command returns at once for a dead client (combat.rs:158-163) and
        // without facts (combat.rs:208-217).
        let run_weapon =
            input.alive && ctx.is_some() && !(self.hands[0].weapon == 0 && cmd_weapon == 0);
        let mut raised: Option<DmwWeaponId> = None;
        let mut dropped: Option<DmwWeaponId> = None;

        if let (true, Some(c)) = (run_weapon, &ctx) {
            let combat = c.facts.combat;

            // ---- Step 7: aim spread (combat.rs:219-249).
            let mut state = AimSpreadState {
                aim_spread_scale: self.ps.aim_spread_scale,
                spread_override: self.ps.spread_override,
                spread_override_state: self.ps.spread_override_state,
            };
            let motion = AimSpreadMotion {
                frametime,
                cmd_angles: ucmd.angles,
                old_angles,
                forwardmove: ucmd.forwardmove,
                rightmove: ucmd.rightmove,
                velocity_xy: [self.ps.velocity[0], self.ps.velocity[1]],
                speed: self.ps.speed,
                move_speed_threshold: AIM_SPREAD_MOVE_SPEED_THRESHOLD_DEFAULT,
            };
            adjust_aim_spread_scale(
                &mut state,
                &combat.spread_facts(),
                &combat.aim_spread_decay_facts(),
                self.ps.ground_entity_num,
                self.ps.pm_type,
                self.ps.e_flags,
                self.ps.f_weapon_pos_frac,
                &motion,
            );
            self.ps.aim_spread_scale = state.aim_spread_scale;
            self.ps.spread_override_state = state.spread_override_state;

            // Vanilla handoff (§5.8): start the normal drop once admitted, then hold it.
            let holster = matches!(self.switch, SwitchState::ToVanilla { .. })
                && begin_vanilla_drop(&mut self.hands[0], &combat, self.ps.pm_flags);

            // ---- Step 8: WeaponCmd, field by field as combat.rs:337-517 (single hand, no
            // offhand, no lock-on, no airdrop marker).
            let cmd_w = if ucmd.weapon != 0 {
                u32::from(ucmd.weapon)
            } else {
                self.ps.weapon
            };
            let cmd_w_facts = self.combat_of(cmd_w);
            let ucmd_weapon_facts = self.combat_of(u32::from(ucmd.weapon));
            let mut wcmd = WeaponCmd {
                msec,
                server_time: ucmd.server_time,
                stun_time: self.ps.stun_time,
                buttons: ucmd.buttons, // combat.rs:329-347
                old_buttons,           // combat.rs:348-354
                cmd_weapon: if ucmd.weapon != 0 {
                    ucmd.weapon
                } else {
                    self.ps.weapon as u16
                }, // combat.rs:356-360
                pm_flags: self.ps.pm_flags,
                weap_flags: self.ps.weap_flags,
                pm_type: self.ps.pm_type,
                e_flags: self.ps.e_flags,
                last_weapon_hand: 0, // num_hands_for_held: no akimbo (combat.rs:260)
                f_weapon_pos_frac: self.ps.f_weapon_pos_frac,
                melee_charge_yaw: ucmd.melee_charge_yaw,
                melee_charge_dist: ucmd.melee_charge_dist,
                player_melee_range: PLAYER_MELEE_RANGE_DEFAULT,
                // is_in_air with almost_ground_plane = 0 (combat.rs:371-391,
                // movement_iw4/src/is_in_air.rs:5-13).
                is_in_air: self.ps.ground_entity_num == ENTITYNUM_NONE
                    && self.ps.pm_type != 1
                    && self.ps.pm_type != 7,
                melee_charge: MeleeChargeState {
                    pm_flags: self.ps.pm_flags,
                    pm_type: self.ps.pm_type,
                    e_flags: self.ps.e_flags,
                    melee_charge_yaw: self.ps.melee_charge_yaw,
                    melee_charge_dist: self.ps.melee_charge_dist,
                    melee_charge_time: self.ps.melee_charge_time,
                }, // combat.rs:392-399
                melee_started: None,
                // combat.rs:402 is the mantle check (never active here); the vanilla handoff
                // reuses it to finish the drop holstered (switch.rs).
                mantle_weapon_inactive: holster,
                mantle_quick_raise: false, // combat.rs:403, no mantle
                cmd_weapon_owned: cmd_w == 0
                    || weapon_id(cmd_w).is_some_and(|id| self.inv.owns(id)), // combat.rs:404-416
                cmd_weapon_pistol_quick: ucmd_weapon_facts
                    .is_some_and(|f| f.weap_class == WEAPCLASS_PISTOL), // combat.rs:418-420
                alternate_switch: ucmd.weapon != 0
                    && (combat.alternate_weapon == u32::from(ucmd.weapon)
                        || (combat.inventory_type == 3
                            && self.ps.weapon_primary == u32::from(ucmd.weapon))), // combat.rs:421-423
                switch_alternate_raise_time_ms: ucmd_weapon_facts
                    .map_or(0, |f| f.alternate_raise_time_ms), // combat.rs:424-426
                switch_raise_time_ms: cmd_w_facts.map_or(0, |f| f.raise_time_ms), // combat.rs:427-437
                switch_quick_raise_time_ms: cmd_w_facts.map_or(0, |f| f.quick_raise_time_ms), // combat.rs:438-448
                switch_first_raise_time_ms: if self.used.contains(&(cmd_w as u16)) {
                    0
                } else {
                    cmd_w_facts.map_or(0, |f| f.first_raise_time_ms.max(0))
                }, // combat.rs:449-462
                perks0: self.ps.perks[0],
                perks1: self.ps.perks[1],
                perk_weap_reload_multiplier: PERK_WEAP_RELOAD_MULTIPLIER_DEFAULT, // combat.rs:465
                offhand: OffhandCmd::default(), // offhand is out of scope (sheet 07 §2)
            };

            // ---- Step 9: run (combat.rs:519); facts are the held weapon's (combat.rs:215).
            let pre_state = self.hands[0].weaponstate;
            let pre_weapon_time = self.hands[0].weapon_time;
            let tick_events = weapon_hands(&mut self.hands, &combat, &mut wcmd, 0);
            let shot = tick_events
                .iter()
                .flatten()
                .any(|(h, e)| *h == 0 && matches!(e, WeaponTickEvent::ShotAccepted { .. }));
            if shot {
                carry_fire_overshoot(
                    &mut self.hands[0],
                    combat.fire_time_ms,
                    pre_state,
                    pre_weapon_time,
                    msec,
                );
            }
            let hand0 = self.hands[0];

            // ---- Step 10: write back (combat.rs:562-597).
            update_scope_zoom(&mut self.ps, ucmd.buttons, old_buttons, combat.scope_zoom);
            self.ps.weapon = hand0.weapon;
            if hand0.weapon != started {
                self.ps.scope_zoom_level = 0;
                self.ps.weapon_primary = if hand0.weapon == u32::from(ucmd.weapon) {
                    u32::from(ucmd.weapon_mapped)
                } else {
                    0
                };
            }
            self.ps.weaponstate_primary = hand0.weaponstate;
            self.ps.weapon_time = hand0.weapon_time;
            self.ps.weapon_delay = hand0.weapon_delay;
            self.ps.weap_anim = hand0.weap_anim;
            self.ps.weapon_restrict_kick_time = hand0.weapon_restrict_kick_time;
            self.ps.last_weapon_hand = 0;
            self.ps.pm_flags = wcmd.pm_flags;
            self.ps.weap_flags = wcmd.weap_flags;
            self.ps.off_hand_index = wcmd.offhand.off_hand_index;
            self.ps.grenade_time_left = wcmd.offhand.grenade_time_left;
            self.ps.melee_charge_yaw = wcmd.melee_charge.melee_charge_yaw;
            self.ps.melee_charge_dist = wcmd.melee_charge.melee_charge_dist;
            self.ps.melee_charge_time = wcmd.melee_charge.melee_charge_time;

            // Ammo out (§5.9; combat.rs:618-650 saves to the weapon that started the tic).
            self.inv.set_magazine(c.id, hand0.clip);
            if let Some(p) = ammo_pool {
                pools[p] = hand0.stock;
            }

            // render_anim resets kick and sway when the viewmodel weapon changes
            // (render_anim/src/occupancy/view_kick.rs:145-158), before seeding this tic's fire.
            if self.ps.weapon != self.kick_weapon {
                self.kick.reset();
                self.sway.reset();
                self.kick_weapon = self.ps.weapon;
            }

            // ---- Step 11: events (combat.rs:672-1027, §5.4-5.5).
            for (hand_index, event) in tick_events.into_iter().flatten() {
                if hand_index != 0 {
                    continue;
                }
                match event {
                    WeaponTickEvent::ShotAccepted { ammo_used } => {
                        self.on_shot(c, ammo_used, &mut events);
                    }
                    WeaponTickEvent::EmptyClick => {
                        events.push(SimEvent::EmptyClick);
                        let s = &c.facts.sounds;
                        if let Some(alias) = s.empty_fire_player.as_ref().or(s.empty_fire.as_ref())
                        {
                            events.push(SimEvent::Sound(SoundCue {
                                alias: alias.clone(),
                                local: true,
                            }));
                        }
                    }
                    WeaponTickEvent::ReloadStarted => {
                        let empty = hand0.clip == 0;
                        log::debug!(target: LOG, "reload start: weapon {} empty {empty}", c.id.0);
                        events.push(SimEvent::ReloadStarted { empty });
                    }
                    WeaponTickEvent::ReloadEnded => {
                        log::debug!(target: LOG, "reload end: weapon {}", c.id.0);
                        events.push(SimEvent::ReloadFinished);
                    }
                    WeaponTickEvent::RaiseStarted => {
                        // combat.rs:977-994: mark_weapon_used for the raised weapon.
                        if let Some(id) = weapon_id(hand0.weapon) {
                            self.used.insert(id.0);
                            log::debug!(target: LOG, "raise start: weapon {}", id.0);
                            events.push(SimEvent::RaiseStarted(id));
                            raised = Some(id);
                        }
                    }
                    WeaponTickEvent::DropFinished => {
                        log::debug!(target: LOG, "drop finished: weapon {}", c.id.0);
                        events.push(SimEvent::DropFinished(c.id));
                        dropped = Some(c.id);
                    }
                    WeaponTickEvent::MeleeFired => {
                        // combat.rs:995-1021; direction = ps.viewangles.
                        let (dir, _, _) = math_iw4::angle_vectors(self.ps.viewangles);
                        events.push(SimEvent::Melee(MeleeRequest {
                            weapon: c.id,
                            damage: combat.melee_damage,
                            range_mw2: wcmd.player_melee_range,
                            dir,
                        }));
                        let s = &c.facts.sounds;
                        if let Some(alias) =
                            s.melee_swipe_player.as_ref().or(s.melee_swipe.as_ref())
                        {
                            events.push(SimEvent::Sound(SoundCue {
                                alias: alias.clone(),
                                local: true,
                            }));
                        }
                    }
                    WeaponTickEvent::ReloadInsert
                    | WeaponTickEvent::ReloadAmmoAdded { .. }
                    | WeaponTickEvent::RaiseFinished
                    | WeaponTickEvent::PutawayStarted
                    | WeaponTickEvent::RechamberWeapon
                    | WeaponTickEvent::EjectBrass => {}
                    other => {
                        let key = format!("event:{other:?}");
                        if self.warned.insert(key) {
                            log::debug!(target: LOG, "ignored weapon event {other:?}");
                        }
                    }
                }
            }
        }

        // Switch bookkeeping (§5.8).
        if let Some(id) = dropped
            && matches!(self.switch, SwitchState::ToVanilla { .. })
        {
            self.hands[0] = WeaponHandState::default();
            self.ps.weapon = 0;
            self.ps.weaponstate_primary = 0;
            self.ps.weapon_time = 0;
            self.ps.weap_anim = 0;
            self.switch = SwitchState::VanillaHeld;
            self.inv.last_held = Some(id);
            weapon_lowered_for_vanilla = true;
        }
        if let Some(id) = raised {
            if let Some(prev) = weapon_id(started).filter(|&p| p != id) {
                self.inv.last_held = Some(prev);
            }
            // The new weapon's magazine and pool, for this tic's view (next tic reloads them).
            self.hands[0].clip = self.inv.magazine(id);
            self.hands[0].stock = self.pool_of(id).map_or(0, |p| pools[p]);
        }
        if let SwitchState::ToMw2 { target } = self.switch
            && self.hands[0].weapon == u32::from(target.0)
            && !is_dropping(&self.hands[0])
        {
            self.switch = SwitchState::Idle;
        }

        // ---- Step 12: kick and sway advance (render_anim view_kick.rs:194-203).
        let held_now = weapon_id(self.hands[0].weapon).and_then(|id| pack.try_facts(id));
        let ads_frac = self.ps.f_weapon_pos_frac;
        if let Some(f) = held_now {
            self.kick.advance(&f.kick, ads_frac, frametime);
            self.sway.advance(
                f.sway.hip,
                f.sway.ads,
                self.ps.viewangles,
                ads_frac,
                f.combat.aim_down_sight,
                f.fpv.overlay_reticle != 0,
                1.0,
                frametime,
            );
        }

        // ---- Step 13: placement (sheet 10 §5.3).
        if let Some(f) = held_now {
            let inputs = ViewInputs {
                ps: &self.ps,
                facts: f,
                sway: self.sway.springs(),
                gun_recoil: self.kick.gun,
                view_kick_deg: self.kick.kick_angles,
                msec,
            };
            self.placed = placement::step(&mut self.placement, &inputs);
            self.gun_angle_offset = placement::gun_angle_offset(&self.placed, &inputs);
        } else {
            self.placed = GunPlacement::default();
            self.gun_angle_offset = [0.0; 2];
        }

        // ---- Step 14: outputs (§5.6, §5.11).
        self.spread_deg = held_now.map_or(0.0, |f| current_spread_degrees(&self.ps, &f.combat));
        self.old_buttons = ucmd.buttons;
        self.old_angles = Some(ucmd.angles);

        let sprinting = self.ps.pm_flags & pm_flags::SPRINTING != 0;
        let move_speed_scale = match held_now {
            Some(f) => {
                let weapon = if self.weapon_move_scales {
                    f.movement.move_speed_scale
                        * lerp(1.0, f.movement.ads_move_speed_scale, ads_frac)
                } else {
                    1.0
                };
                weapon
                    * if sprinting {
                        self.sprint_speed_scale
                    } else {
                        1.0
                    }
            }
            None => 1.0,
        };

        self.last_view = self.build_view(held_now);
        log::trace!(
            target: LOG,
            "tic {}: weaponstate {} weapon_time {} ads {:.3} aim_spread {:.1} pm_flags {:#x}",
            input.tic,
            self.hands[0].weaponstate,
            self.hands[0].weapon_time,
            ads_frac,
            self.ps.aim_spread_scale,
            self.ps.pm_flags
        );

        TicOutput {
            events,
            pools,
            view_kick_deg: self.kick.kick_angles,
            move_speed_scale,
            sprinting,
            ads_frac,
            spread_deg: self.spread_deg,
            held: weapon_id(self.hands[0].weapon),
            weapon_lowered_for_vanilla,
        }
    }

    /// `WeaponTickEvent::ShotAccepted` (sheet 07 §5.4; combat.rs:729-839, phase_emit at
    /// combat.rs:1152-1192).
    fn on_shot(&mut self, c: &WeaponCtx<'_>, ammo_used: i32, events: &mut Vec<SimEvent>) {
        let combat = c.facts.combat;
        let hand = self.hands[0];
        // 1. Recoil seed with the post-tic ps (render_anim view_kick.rs:177-190): 4 draws.
        self.kick.seed_fire(
            &c.facts.kick,
            self.ps.f_weapon_pos_frac,
            self.ps.weap_flags,
            self.ps.recoil_scale,
            hand.weapon_restrict_kick_time > 0,
            &mut self.rng,
        );
        // 2. combat.rs:796-802.
        add_aim_spread_fire(
            &mut self.ps.aim_spread_scale,
            self.ps.f_weapon_pos_frac,
            combat.hip_spread_fire_add,
        );
        // 3. combat.rs:807-821.
        let spread_deg = current_spread_degrees(&self.ps, &combat);
        // 4. combat.rs:770-775.
        let mut angles = self.ps.viewangles;
        for (angle, offset) in angles.iter_mut().zip(self.gun_angle_offset) {
            if offset.is_finite() {
                *angle += offset.clamp(-GUN_ANGLE_OFFSET_LIMIT, GUN_ANGLE_OFFSET_LIMIT);
            }
        }
        // 5. combat.rs:1168-1185: 2 draws per pellet.
        let barrels = if combat.fire_type == FIRE_TYPE_DOUBLE_BARREL {
            ammo_used.max(1)
        } else {
            1
        };
        let n = (combat.pellet_count() * barrels).clamp(1, MAX_PELLETS);
        let pellets = (0..n)
            .map(|_| spread_direction_on_plane(angles, spread_deg, &mut self.rng, 1.0))
            .collect::<Vec<_>>();
        // 6.
        let projectile = matches!(
            fire_weapon_kind(combat.weap_type, combat.weap_class),
            Some(FireWeaponKind::Missile | FireWeaponKind::GrenadeLauncher)
        );
        // 7. combat.rs:776.
        let last_shot = hand.clip == 0 && combat.fire_type != FIRE_TYPE_DOUBLE_BARREL;
        log::debug!(
            target: LOG,
            "shot: weapon {} pellets {} spread {:.2} clip {}",
            c.id.0,
            n,
            spread_deg,
            hand.clip
        );
        events.push(SimEvent::Shot(ShotRequest {
            weapon: c.id,
            pellets,
            base_damage: combat.damage,
            max_range_mw2: combat.bullet_range(),
            projectile,
        }));
        events.push(SimEvent::MuzzleFlash {
            weapon: c.id,
            last_shot,
        });
        let s = &c.facts.sounds;
        let last = if last_shot {
            s.fire_last_player.as_ref().or(s.fire_last.as_ref())
        } else {
            None
        };
        if let Some(alias) = last.or(s.fire_player.as_ref()).or(s.fire.as_ref()) {
            events.push(SimEvent::Sound(SoundCue {
                alias: alias.clone(),
                local: true,
            }));
        }
    }

    /// `ViewState` for the current state (§5.11).
    fn build_view(&self, held: Option<&WeaponFacts>) -> ViewState {
        let hand = self.hands[0];
        let ads_frac = self.ps.f_weapon_pos_frac;
        let mut zoom_hfov_deg = None;
        let mut viewmodel_fov_deg = self.viewmodel_fov;
        let mut overlay_active = false;
        if let Some(f) = held {
            let overlay = f.fpv.overlay_reticle != 0;
            overlay_active = overlay && ads_frac >= 1.0;
            if ads_frac > 0.0 && f.combat.aim_down_sight {
                let scope = f.fpv.scope_zoom;
                let fov = if overlay && scope.levels() > 0 {
                    scope
                        .fov(self.ps.scope_zoom_level)
                        .unwrap_or(f.fpv.ads_zoom_fov)
                } else {
                    f.fpv.ads_zoom_fov
                };
                let intent = self.ps.pm_flags & pm_flags::ADS_INTENT != 0;
                let start = if intent {
                    f.fpv.ads_zoom_in_frac
                } else {
                    f.fpv.ads_zoom_out_frac
                };
                let z = if start < 1.0 {
                    ((ads_frac - start) / (1.0 - start)).clamp(0.0, 1.0)
                } else if ads_frac >= 1.0 {
                    1.0
                } else {
                    0.0
                };
                zoom_hfov_deg = Some(lerp(DOOM_HFOV, mw2_fov_to_doom(fov), z));
                viewmodel_fov_deg = lerp(self.viewmodel_fov, fov, z);
            }
        }
        let stock = match held {
            Some(_) => hand.stock,
            None => 0,
        };
        ViewState {
            weapon: weapon_id(hand.weapon),
            weaponstate: hand.weaponstate,
            weap_anim: hand.weap_anim,
            ads_frac,
            placement: self.placed,
            gun_angle_offset: self.gun_angle_offset,
            view_kick_deg: self.kick.kick_angles,
            spread_deg: self.spread_deg,
            overlay_active,
            zoom_hfov_deg,
            viewmodel_fov_deg,
            clip: hand.clip,
            stock,
        }
    }

    /// The shadow player state (tests and sheet 10 diagnostics).
    pub fn player_state(&self) -> &PlayerState {
        &self.ps
    }

    /// Hand 0 (tests).
    pub fn hand(&self) -> &WeaponHandState {
        &self.hands[0]
    }

    /// Current switch state (tests, sheet 08 diagnostics).
    pub fn switch_state(&self) -> SwitchState {
        self.switch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fov_conversion_is_identity_at_cg_fov() {
        assert!((mw2_fov_to_doom(MW2_CG_FOV) - DOOM_HFOV).abs() < 1e-3);
        assert!(mw2_fov_to_doom(20.0) < 40.0);
    }
}
