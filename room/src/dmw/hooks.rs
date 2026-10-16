// SPDX-License-Identifier: GPL-3.0-or-later
//! The single file every Doom-module edit calls into
//! (`docs/specs/00-overview-and-contracts.md` section 7.5).
//!
//! Sheet 08 owns this file and the bodies of its own rows (engine init, level start, the
//! per-tic weapon driver, movement scaling, the vanilla/MW2 hand-off, new game, pickup
//! scaling, savegames). The other rows are one-line delegations into the owning sheet's file.
//!
//! Rules every hook follows:
//! - With MW2 mode off ([`mode::weapons_active`] / [`mode::level_active`] false) a hook returns
//!   its "not handled" value before touching anything, so the vanilla path is exactly room's.
//! - A hook computes a plan inside [`with_state`] and runs Doom functions only after the
//!   borrow is released (sheet 08 section 5.1).
//! - A hook never panics into Doom: the body runs under `catch_unwind`; a panic switches MW2
//!   mode off for the level ([`mode::fail`]) and the "not handled" value is returned.
//!
//! Hooks listed in the table but defined elsewhere:
//! - `input::on_mouse_event` and `input::scale_yaw` live in [`super::input`].
//! - `freelook::with_unsheared_center` lives in [`super::freelook`], as do the sheet 06 table
//!   helpers the renderer calls directly (`fine_fov`, `focal_fixed`, `focal_y_fixed`,
//!   `light_scale`, `sky_iscale`).
//! - `DG_GetMouse` is a C-ABI platform callback added by sheet 05.
//! - `audio::play` (sheet 12) is called from the action runner below and from sheet 09.

use std::ffi::{c_char, c_int, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dmw_format::{Pack, PackError};
use dmw_weapons::{DmwRng, WeaponSim};
use log::{debug, error, info, warn};

use super::config::DmwConfig;
use super::ext::PLAYER_EXT;
use super::mode::{self, GateInputs};
use super::weapons::{self, Action};
use super::{with_state, DmwState};
use crate::doom::d_player::{PlayerT, TiccmdT, MAXPLAYERS};
use crate::doom::p_telept::mobj_t;

const TARGET: &str = "dmw::sim";
const PACK_TARGET: &str = "dmw::pack";
const SAVE_TARGET: &str = "dmw::save";

/// Runs `f`; on a panic logs it, switches MW2 mode off and returns `fallback`.
fn guarded<R>(hook: &str, fallback: R, f: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            mode::fail(&format!("panic in {hook}: {msg}"));
            fallback
        }
    }
}

/// The console player, when `player` is it and the index is valid.
///
/// # Safety
/// Reads Doom globals on the engine thread.
unsafe fn console_index(player: *mut PlayerT) -> Option<usize> {
    // SAFETY: plain reads of engine globals; the pointer is only compared.
    unsafe {
        let cp = crate::doom::g_game::consoleplayer;
        let cp = usize::try_from(cp).ok().filter(|&i| i < MAXPLAYERS)?;
        let mine = (&raw mut crate::doom::g_game::players)
            .cast::<PlayerT>()
            .add(cp);
        (std::ptr::eq(mine, player) && !player.is_null()).then_some(cp)
    }
}

/// Value of a command-line flag with one argument (`-flag value`), through Doom's `myargv`.
fn arg_value(flag: &CStr) -> Option<String> {
    // SAFETY: `myargv` holds `myargc` NUL-terminated strings for the whole process.
    unsafe {
        let p = crate::doom::m_argv::M_CheckParmWithArgs(flag.as_ptr(), 1);
        if p <= 0 {
            return None;
        }
        let arg: *mut c_char = *crate::doom::m_argv::myargv.add(p as usize + 1);
        (!arg.is_null()).then(|| CStr::from_ptr(arg).to_string_lossy().into_owned())
    }
}

/// `true` when `flag` is on the command line.
fn has_flag(flag: &CStr) -> bool {
    crate::doom::m_argv::M_CheckParm(flag.as_ptr()) > 0
}

/// Opens the pack per sheet 08 section 5.2 step 2.
fn open_pack(cfg: &DmwConfig) -> Option<Pack> {
    if has_flag(c"-nomw2") {
        info!(target: PACK_TARGET, "MW2 mode disabled by -nomw2");
        return None;
    }
    if !cfg.general.enabled {
        info!(target: PACK_TARGET, "MW2 mode disabled by [general] enabled = false");
        return None;
    }
    if has_flag(c"-fixturepack") {
        return Some(Pack::fixture());
    }
    let (dir, explicit) = match arg_value(c"-mw2pack") {
        Some(d) => (PathBuf::from(d), true),
        None => match std::env::var_os("DMW_PACK").filter(|v| !v.is_empty()) {
            Some(d) => (PathBuf::from(d), true),
            None => (PathBuf::from("packs/mw2"), false),
        },
    };
    match Pack::open(&dir) {
        Ok(p) => return Some(p),
        Err(PackError::Missing(what)) if !explicit => {
            info!(target: PACK_TARGET, "no pack at {} ({what} missing)", dir.display());
        }
        Err(e) => warn!(target: PACK_TARGET, "pack {} not loaded: {e}", dir.display()),
    }
    if cfg.general.allow_fixture_pack {
        let fixture = Path::new("packs/fixture");
        match Pack::open(fixture) {
            Ok(p) => return Some(p),
            Err(e) => {
                warn!(target: PACK_TARGET, "fixture pack {} not loaded: {e}", fixture.display())
            }
        }
    }
    info!(target: PACK_TARGET, "no pack loaded; MW2 mode stays off");
    None
}

/// PLAYPAL + COLORMAP from the loaded WADs (sheet 11), `None` if a lump is missing.
fn load_palette() -> Option<dmw_viewmodel::PaletteTables> {
    use crate::doom::w_wad::{W_CacheLumpName, W_CheckNumForName, W_LumpLength};
    use crate::doom::z_zone::PU_STATIC;
    let lump = |name: &CStr| -> Option<&'static [u8]> {
        let num = W_CheckNumForName(name.as_ptr());
        let len = usize::try_from(W_LumpLength(u32::try_from(num).ok()?)).ok()?;
        let ptr = W_CacheLumpName(name.as_ptr(), PU_STATIC).cast::<u8>();
        // SAFETY: `W_CacheLumpName` returns the lump's `len` bytes, kept in the zone with
        // `PU_STATIC` (never freed) for the rest of the process.
        (!ptr.is_null()).then(|| unsafe { std::slice::from_raw_parts(ptr, len) })
    };
    let (Some(playpal), Some(colormap)) = (lump(c"PLAYPAL"), lump(c"COLORMAP")) else {
        warn!(target: PACK_TARGET, "PLAYPAL or COLORMAP missing; no palette tables");
        return None;
    };
    match dmw_viewmodel::PaletteTables::from_wad_lumps(playpal, colormap) {
        Ok(t) => Some(t),
        Err(e) => {
            warn!(target: PACK_TARGET, "palette tables not built: {e}");
            None
        }
    }
}

/// `D_DoomMain`, after `W_Init`/`R_Init` and before the title or `G_InitNew`: load config,
/// pack, palette tables, mapping (sheet 08 section 5.2).
pub(crate) fn on_engine_init() {
    guarded("on_engine_init", (), || {
        let explicit = arg_value(c"-mw2config").map(PathBuf::from);
        let (config, config_path) = DmwConfig::load(explicit.as_deref());
        super::input::apply_bindings(&config.input);

        let mut state = DmwState::new(config, config_path);
        if let Some(pack) = open_pack(&state.config) {
            // `Pack` is `!Sync` (RefCell caches) and the engine is single-threaded; the `Arc`
            // is the shared-ownership type of the 00 section 7.2 contract (`WeaponSim::new`).
            #[allow(clippy::arc_with_non_send_sync)]
            let pack = Arc::new(pack);
            let names = weapons::config_names(&state.config.weapons);
            let (mapping, warnings) =
                weapons::WeaponMapping::build(names, &pack, weapons::doom_weapon_ammo());
            weapons::log_mapping_warnings(&warnings);
            info!(target: PACK_TARGET, "weapon mapping: {}", mapping.describe(&pack));
            state.mapping = mapping;
            state.palette = load_palette().map(Arc::new);
            state.sim = Some(WeaponSim::new(Arc::clone(&pack)));
            state.pack = Some(pack);
        }
        info!(
            target: TARGET,
            "DOOMMW2 init: config {}, pack {}",
            state
                .config_path
                .as_ref()
                .map_or_else(|| "defaults".to_owned(), |p| p.display().to_string()),
            state
                .pack
                .as_ref()
                .map_or_else(|| "none".to_owned(), |p| p.dir.display().to_string()),
        );
        super::install_state(state);
    });
}

/// `G_DoLoadLevel`, end: evaluate mode gating, `WeaponSim::start_level`, prepare viewmodels
/// (sheet 08 section 5.10).
pub(crate) fn on_level_start() {
    super::interp::on_level_start();
    guarded("on_level_start", (), || {
        let was_active = mode::level_active();
        // SAFETY: engine globals read on the engine thread after `P_SetupLevel`.
        let (gate_netgame, recording, playback, episode, map, skill, cp) = unsafe {
            use crate::doom::g_game as g;
            (
                g::netgame != 0,
                g::demorecording != 0,
                g::demoplayback != 0 || g::precache == 0,
                g::gameepisode,
                g::gamemap,
                g::gameskill,
                g::consoleplayer,
            )
        };
        let player = player_ptr(cp);
        let decided = with_state(|s| {
            s.failed = None;
            let gate = GateInputs {
                pack_loaded: s.sim.is_some(),
                enabled: s.config.general.enabled,
                netgame: gate_netgame,
                demorecording: recording,
                demoplayback: playback,
                failed: false,
            };
            let result = match (mode::evaluate(&gate), player) {
                (Ok(()), None) => Err("the console player has no map object"),
                (r, _) => r,
            };
            mode::set_level(result);
            match (result, player, s.pack.clone()) {
                (Ok(()), Some(p), Some(pack)) => {
                    // SAFETY: `player_ptr` returned the live console player.
                    let p = unsafe { &mut *p };
                    let seed = DmwRng::level_seed(skill, episode, map);
                    weapons::level_start(s, p, episode, map, seed);
                    Some((pack, s.config.balance.clone()))
                }
                (_, p, _) => {
                    if std::mem::take(&mut s.driver.maxima_applied) {
                        if let Some(p) = p {
                            // SAFETY: as above.
                            weapons::restore_vanilla_maxima(unsafe { &mut *p });
                        }
                    }
                    s.driver.owner = weapons::Owner::Vanilla;
                    s.last_out = None;
                    None
                }
            }
        });
        match decided {
            None => {
                // No state (engine init did not run) or busy: MW2 mode stays off.
                mode::set_level(Err("DOOMMW2 state unavailable"));
            }
            Some(Some((pack, balance))) => {
                super::ballistics::configure(pack, balance);
                super::ballistics::reset_level();
            }
            Some(None) => {}
        }
        super::reset_log_once();
        if mode::level_active() {
            super::ext::reset_level();
            super::freelook::reset();
            super::hud::reset();
        } else if was_active {
            super::freelook::reset();
        }
    });
}

/// `&mut players[cp]` when `cp` is valid and the player has a map object.
fn player_ptr(cp: c_int) -> Option<*mut PlayerT> {
    let i = usize::try_from(cp).ok().filter(|&i| i < MAXPLAYERS)?;
    // SAFETY: `players` is a static array; the pointer stays valid for the process.
    unsafe {
        let p = (&raw mut crate::doom::g_game::players)
            .cast::<PlayerT>()
            .add(i);
        (!(*p).mo.is_null()).then_some(p)
    }
}

/// `G_BuildTiccmd`, end: mouse pitch, extended buttons, MW2 bindings.
pub(crate) unsafe fn on_build_ticcmd(cmd: *mut TiccmdT, maketic: c_int) {
    // SAFETY: forwarded unchanged; the caller passes the ticcmd it just built.
    unsafe { super::input::on_build_ticcmd(cmd, maketic) }
    // SAFETY: `cmd` is the command just built; the ext ring slot was just written.
    let (turn, pitch) = unsafe {
        let slot = maketic.rem_euclid(crate::doom::c_ffi::BACKUPTICS as c_int) as usize;
        let ext = (&raw const super::ext::EXT_TICCMDS)
            .cast::<super::ext::ExtTiccmd>()
            .add(slot)
            .read();
        ((*cmd).angleturn, ext.pitch_delta)
    };
    super::interp::on_build_ticcmd(maketic, turn, pitch);
}

/// `TryRunTics`: `true` = no tic due and frames are uncapped; return without waiting.
pub(crate) fn uncapped_return_early(availabletics: c_int) -> bool {
    super::interp::return_early(availabletics)
}

/// `P_Ticker`, after the pause checks: snapshot for frame interpolation.
pub(crate) fn on_ticker_start() {
    guarded("on_ticker_start", (), super::interp::on_ticker_start);
}

/// `D_Display`, before `R_RenderPlayerView`: swap in blended positions.
pub(crate) fn begin_frame() -> super::interp::Restore {
    guarded(
        "begin_frame",
        super::interp::Restore::default(),
        super::interp::begin_frame,
    )
}

/// `D_Display`, after `R_RenderPlayerView`: restore the real positions.
pub(crate) fn end_frame(restore: super::interp::Restore) {
    super::interp::end_frame(restore);
}

/// `G_Responder`, `ev_mouse` arm: `true` when mouse Y should drive pitch instead of movement.
pub(crate) fn mouse_look_active() -> bool {
    super::input::mouse_look_active()
}

/// `G_BuildTiccmd`, after `speed` is computed: `true` forces always-run in MW2 mode.
pub(crate) fn force_run() -> bool {
    super::input::force_run()
}

/// `G_Ticker`, after the net command copy: copy the extended command into `PLAYER_EXT`.
pub(crate) fn on_ticcmd_copied(player_index: c_int) {
    super::input::on_ticcmd_copied(player_index);
}

/// `P_PlayerThink`, before `P_MovePsprites`: run `WeaponSim::tic`. Returns `true` when MW2
/// owns the weapon this tic (the caller skips `P_MovePsprites`).
pub(crate) unsafe fn on_player_think(player: *mut PlayerT) -> bool {
    if !mode::weapons_active() {
        return false;
    }
    guarded("on_player_think", false, || {
        // SAFETY: engine globals on the engine thread; `player` is the pointer P_PlayerThink got.
        let Some(cp) = (unsafe { console_index(player) }) else {
            return false;
        };
        // SAFETY: as above.
        let (dynamic_off, leveltime, gamemode) = unsafe {
            use crate::doom::g_game as g;
            (
                g::netgame != 0 || g::demorecording != 0 || g::demoplayback != 0,
                crate::doom::p_tick::leveltime,
                crate::doom::doomstat::gamemode,
            )
        };
        if dynamic_off {
            mode::fail("a netgame or demo started during an MW2 level");
            return false;
        }
        // SAFETY: `player` is the live console player (P_PlayerThink only runs for players
        // with a map object); `PLAYER_EXT[cp]` is its extension slot. Neither reference
        // outlives this call and no Doom function runs while they are live.
        let plan = with_state(|s| unsafe {
            let p = &mut *player;
            let pe = &mut *(&raw mut PLAYER_EXT)
                .cast::<super::ext::PlayerExt>()
                .add(cp);
            weapons::think(s, p, pe, leveltime, gamemode)
        });
        let Some(plan) = plan else {
            return false;
        };
        // SAFETY: the borrow is released; `player` is still the live console player.
        unsafe { run_actions(player, plan.actions) };
        plan.handled
    })
}

/// Executes a plan's Doom-side effects (outside the `DmwState` borrow).
///
/// # Safety
/// `player` is the live console player with a live map object.
unsafe fn run_actions(player: *mut PlayerT, actions: Vec<Action>) {
    use crate::doom::p_mobj::P_SetMobjState;
    use crate::doom::statenum::{S_PLAY_ATK1, S_PLAY_ATK2};
    // SAFETY: caller contract.
    let mo = unsafe { (*player).mo.cast::<mobj_t>() };
    for action in actions {
        match action {
            Action::Shot(req) => {
                // SAFETY: `mo` is the player's live map object; Doom functions on the engine
                // thread, no DOOMMW2 borrow held. `fire` makes the noise alert itself.
                unsafe {
                    P_SetMobjState(mo, S_PLAY_ATK2);
                    super::ballistics::fire(player, &req);
                }
            }
            Action::Melee(req) => {
                // SAFETY: as above.
                unsafe {
                    P_SetMobjState(mo, S_PLAY_ATK1);
                    super::ballistics::melee(player, &req);
                }
            }
            Action::Sound(cue) => {
                super::audio::play(&cue, mo);
            }
            Action::Acquired(id) => super::hud::on_weapon_acquired(id),
            Action::Raised(id) => super::hud::on_weapon_raised(id),
            Action::ViewmodelTic { view, msec } => {
                super::hud::on_tic();
                for cue in super::viewmodel::on_tic(&view, msec) {
                    super::audio::play(&cue, mo);
                }
            }
            Action::BringUpVanilla => {
                // SAFETY: Doom's own raise of `readyweapon` (an unmapped weapon, so
                // `on_bring_up_weapon` returns false inside it).
                unsafe { crate::doom::p_pspr::P_BringUpWeapon(player) };
            }
            Action::Fail(reason) => mode::fail(&reason),
        }
    }
}

/// `P_MovePlayer`, before `P_Thrust`: MW2 move-speed scales (sheet 08 section 5.9). Takes and
/// returns the thrust values (`cmd.forwardmove * 2048`, `cmd.sidemove * 2048`). Uses the
/// previous tic's output (`P_MovePlayer` runs before `on_player_think`).
pub(crate) unsafe fn scale_move(
    player: *mut PlayerT,
    forward: c_int,
    side: c_int,
) -> (c_int, c_int) {
    if !mode::weapons_active() {
        return (forward, side);
    }
    guarded("scale_move", (forward, side), || {
        // SAFETY: engine globals; the pointer is only compared.
        if unsafe { console_index(player) }.is_none() {
            return (forward, side);
        }
        with_state(|s| match (&s.last_out, s.driver.owner) {
            (Some(out), weapons::Owner::Mw2) => {
                let (fs, ss) = weapons::move_scales(out, s.config.balance.sprint_speed_scale);
                (
                    weapons::scale_thrust(forward, fs),
                    weapons::scale_thrust(side, ss),
                )
            }
            _ => (forward, side),
        })
        .unwrap_or((forward, side))
    })
}

/// `P_BringUpWeapon`, first statement: `true` when MW2 raises the weapon instead of psprites.
pub(crate) unsafe fn on_bring_up_weapon(player: *mut PlayerT) -> bool {
    if !mode::weapons_active() {
        return false;
    }
    guarded("on_bring_up_weapon", false, || {
        // SAFETY: engine globals; the pointer is only compared.
        if unsafe { console_index(player) }.is_none() {
            return false;
        }
        // SAFETY: `player` is the live console player; plain field writes only.
        with_state(|s| weapons::bring_up(s, unsafe { &mut *player })).unwrap_or(false)
    })
}

/// `G_InitNew`, end: clear the MW2 inventory (fresh pistol start). Runs after the
/// `on_level_start` of the new game's first level.
pub(crate) fn on_new_game() {
    guarded("on_new_game", (), || {
        with_state(|s| {
            s.driver.maybe_respawn = false;
            s.driver.first_tic = true;
            if let Some(sim) = s.sim.as_mut() {
                *sim.inventory_mut() = dmw_weapons::Inventory::default();
            }
        });
    });
}

/// `P_GiveAmmo`, first statement: pickup scaling (00 section 9.2). Returns the (possibly
/// scaled) clip count.
pub(crate) fn scale_ammo_pickup(ammo: c_int, num: c_int) -> c_int {
    if !mode::weapons_active() {
        return num;
    }
    guarded("scale_ammo_pickup", num, || {
        with_state(|s| match weapons::pickup_scale(&s.config, ammo) {
            Some(scale) => {
                let scaled = weapons::scale_pickup(num, scale);
                debug!(target: TARGET, "pickup: ammo {ammo} x{num} scaled to x{scaled}");
                scaled
            }
            None => num,
        })
        .unwrap_or(num)
    })
}

/// `A_Explode`, first statement: `true` when an MW2 explosion handled the thing.
pub(crate) unsafe fn on_explode(thing: *mut mobj_t) -> bool {
    // SAFETY: forwarded unchanged; the caller passes the thing `A_Explode` received.
    unsafe { super::ballistics::on_explode(thing) }
}

/// `P_RemoveMobj`, first statement: forget MW2 missiles that vanish.
pub(crate) unsafe fn on_remove_mobj(mo: *mut mobj_t) {
    // SAFETY: forwarded unchanged; the address is only used as a registry key.
    unsafe { super::ballistics::on_remove_mobj(mo) }
}

/// `R_SetupFrame`, end: shear, view kick, FOV zoom.
pub(crate) unsafe fn on_setup_frame(player: *mut PlayerT) {
    // SAFETY: forwarded unchanged; the caller passes the player R_SetupFrame received.
    unsafe { super::freelook::on_setup_frame(player) }
}

/// `R_ExecuteSetViewSize`, end: rebuild shear and `yslope` tables.
pub(crate) fn on_view_size_changed() {
    super::freelook::on_view_size_changed();
}

/// `R_DrawPlayerSprites`, start: software viewmodel. `true` = skip the vanilla psprites.
pub(crate) fn draw_player_weapon() -> bool {
    if !mode::weapons_active() {
        return false;
    }
    guarded(
        "draw_player_weapon",
        false,
        super::viewmodel::draw_player_weapon,
    )
}

/// `D_Display`, right after `R_RenderPlayerView`: crosshair, hitmarker, scope overlay.
pub(crate) fn draw_view_overlays() {
    guarded("draw_view_overlays", (), || {
        super::hud::draw_view_overlays();
        super::hud::draw_use_prompt();
        super::fps::on_frame();
    });
}

/// `ST_Drawer` / `ST_updateWidgets`: magazine count for the AMMO widget, if MW2 owns it.
pub(crate) unsafe fn status_ammo(player: *mut PlayerT) -> Option<c_int> {
    if !mode::weapons_active() {
        return None;
    }
    // SAFETY: forwarded unchanged; `status_ammo` never dereferences the pointer.
    guarded("status_ammo", None, || unsafe {
        super::hud::status_ammo(player)
    })
}

/// `G_DoSaveGame`, right after `P_WriteSaveGameEOF`: append the DOOMMW2 extension block
/// (only while MW2 mode is active; otherwise the file is byte-identical to room's).
pub(crate) fn save_extension() {
    if !mode::weapons_active() {
        return;
    }
    guarded("save_extension", (), || {
        // SAFETY: plain read on the engine thread.
        let cp = unsafe { crate::doom::g_game::consoleplayer };
        let Some(cp) = usize::try_from(cp).ok().filter(|&i| i < MAXPLAYERS) else {
            return;
        };
        // SAFETY: plain read of the console player's extension slot.
        let pitch_q = unsafe {
            (*(&raw const PLAYER_EXT)
                .cast::<super::ext::PlayerExt>()
                .add(cp))
            .pitch_q
        };
        let block = with_state(|s| {
            s.sim.as_ref().map(|sim| super::save::SaveBlock {
                version: super::save::VERSION,
                pitch_q,
                level_active: true,
                sim: sim.save(),
            })
        })
        .flatten();
        let Some(bytes) = block.as_ref().and_then(super::save::encode) else {
            return;
        };
        if super::save::write_to_save_stream(&bytes) {
            info!(target: SAVE_TARGET, "saved DOOMMW2 block ({} bytes)", bytes.len());
        } else {
            error!(target: SAVE_TARGET, "could not write the DOOMMW2 save block");
        }
    });
}

/// `G_DoLoadGame`, right after `P_ReadSaveGameEOF` succeeds: read the extension block and
/// restore the sim, or rebuild the inventory with full magazines for a vanilla save.
pub(crate) fn load_extension() {
    if !mode::weapons_active() {
        return;
    }
    guarded("load_extension", (), || {
        use super::save::{parse, read_from_save_stream, Trailer};
        let (block, pitch_q) = match parse(read_from_save_stream) {
            Trailer::Block(b) => {
                info!(target: SAVE_TARGET, "loaded DOOMMW2 block: {} MW2 weapons", b.sim.owned.len());
                (Some(b.sim), b.pitch_q)
            }
            Trailer::Vanilla => {
                info!(target: SAVE_TARGET, "vanilla save: MW2 magazines start full");
                (None, 0)
            }
            Trailer::Malformed(why) => {
                warn!(target: SAVE_TARGET, "DOOMMW2 save block ignored ({why}); treated as vanilla");
                (None, 0)
            }
        };
        // SAFETY: plain read on the engine thread.
        let cp = unsafe { crate::doom::g_game::consoleplayer };
        let Some(player) = player_ptr(cp) else {
            return;
        };
        let cpu = cp as usize;
        // SAFETY: `player` is the live console player; `PLAYER_EXT[cp]` its slot; plain field
        // writes only, no Doom function runs while the references are live.
        with_state(|s| unsafe {
            let pe = &mut *(&raw mut PLAYER_EXT)
                .cast::<super::ext::PlayerExt>()
                .add(cpu);
            weapons::after_load(s, &mut *player, pe, block, pitch_q);
        });
    });
}

/// `M_Responder` menu open and window focus loss: release mouse capture.
pub(crate) fn release_mouse() {
    super::input::release_mouse();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_report_not_handled_while_mw2_mode_is_off() {
        assert!(!mouse_look_active());
        assert!(!force_run());
        assert!(!draw_player_weapon());
        assert_eq!(scale_ammo_pickup(0, 10), 10);
        // SAFETY: with MW2 mode off the hooks return before dereferencing their pointers.
        unsafe {
            assert!(!on_player_think(std::ptr::null_mut()));
            assert!(!on_bring_up_weapon(std::ptr::null_mut()));
            assert!(!on_explode(std::ptr::null_mut()));
            assert_eq!(scale_move(std::ptr::null_mut(), 25, -5), (25, -5));
            assert_eq!(status_ammo(std::ptr::null_mut()), None);
        }
    }

    #[test]
    fn guarded_turns_a_panic_into_the_fallback() {
        #[allow(clippy::panic)]
        let r = guarded("test", 7, || -> i32 { panic!("boom") });
        assert_eq!(r, 7);
    }
}
