// SPDX-License-Identifier: GPL-3.0-or-later
//! The single file every Doom-module edit calls into
//! (`docs/specs/00-overview-and-contracts.md` section 7.5).
//!
//! Each function has its final signature and a scaffold body that returns the "not handled"
//! value, so a call site in `room/src/doom/*` leaves vanilla behaviour untouched. Sheet 08
//! owns this file and the one-line delegations into the other `dmw` modules. Sheet 05 added
//! the call sites and delegations for its own rows (`on_build_ticcmd`, `mouse_look_active`,
//! `force_run`, `on_ticcmd_copied`, `release_mouse`); sheet 08 adds the rest.
//!
//! Hooks listed in the table but defined elsewhere:
//! - `input::on_mouse_event` and `input::scale_yaw` live in [`super::input`].
//! - `freelook::with_unsheared_center` lives in [`super::freelook`].
//! - `DG_GetMouse` is a C-ABI platform callback added by sheet 05.
//! - `audio::play` is added by sheet 12.

use std::ffi::c_int;

use crate::doom::d_player::{PlayerT, TiccmdT};
use crate::doom::p_telept::mobj_t;

/// `D_DoomMain`, after `W_Init` and before `G_InitNew`: load config, pack, palette tables.
pub(crate) fn on_engine_init() {}

/// `G_DoLoadLevel`, end: evaluate mode gating, `WeaponSim::start_level`, prepare viewmodels.
pub(crate) fn on_level_start() {}

/// `G_BuildTiccmd`, end: mouse pitch, extended buttons, MW2 bindings.
pub(crate) unsafe fn on_build_ticcmd(cmd: *mut TiccmdT, maketic: c_int) {
    // SAFETY: forwarded unchanged; the caller passes the ticcmd it just built.
    unsafe { super::input::on_build_ticcmd(cmd, maketic) }
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
/// owns the weapon this tic (skip `P_MovePsprites` and the `BT_CHANGE` block).
pub(crate) unsafe fn on_player_think(_player: *mut PlayerT) -> bool {
    false
}

/// `P_MovePlayer`, before `P_Thrust`: MW2 move-speed scales. Returns `(forward, side)`.
pub(crate) unsafe fn scale_move(
    _player: *mut PlayerT,
    forward: c_int,
    side: c_int,
) -> (c_int, c_int) {
    (forward, side)
}

/// `P_BringUpWeapon`, first statement: `true` when MW2 raises the weapon instead of psprites.
pub(crate) unsafe fn on_bring_up_weapon(_player: *mut PlayerT) -> bool {
    false
}

/// `G_InitNew`, end: clear the MW2 inventory.
pub(crate) fn on_new_game() {}

/// `P_GiveAmmo`: pickup scaling. Returns the (possibly scaled) amount.
pub(crate) fn scale_ammo_pickup(_ammo: c_int, num: c_int) -> c_int {
    num
}

/// `A_Explode`, first statement: `true` when an MW2 explosion handled the thing.
pub(crate) unsafe fn on_explode(_thing: *mut mobj_t) -> bool {
    false
}

/// `P_RemoveMobj`, first statement: forget MW2 missiles that vanish.
pub(crate) unsafe fn on_remove_mobj(_mo: *mut mobj_t) {}

/// `R_SetupFrame`, end: shear, view kick, FOV zoom.
pub(crate) unsafe fn on_setup_frame(_player: *mut PlayerT) {}

/// `R_ExecuteSetViewSize`, end: rebuild shear and `yslope` tables.
pub(crate) fn on_view_size_changed() {}

/// `R_DrawPlayerSprites`, start: software viewmodel. `true` = skip the vanilla psprites.
pub(crate) fn draw_player_weapon() -> bool {
    false
}

/// `D_Display`, right after `R_RenderPlayerView`: crosshair, hitmarker, scope overlay.
pub(crate) fn draw_view_overlays() {}

/// `ST_Drawer` / `ST_updateWidgets`: magazine count for the AMMO widget, if MW2 owns it.
pub(crate) unsafe fn status_ammo(_player: *mut PlayerT) -> Option<c_int> {
    None
}

/// `G_DoSaveGame`, right after `P_WriteSaveGameEOF`: append the DOOMMW2 extension block.
pub(crate) fn save_extension() {}

/// `G_DoLoadGame`, right after `P_ReadSaveGameEOF` succeeds: read the extension block.
pub(crate) fn load_extension() {}

/// `M_Responder` menu open and window focus loss: release mouse capture.
pub(crate) fn release_mouse() {
    super::input::release_mouse();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_hooks_report_not_handled() {
        assert!(!mouse_look_active());
        assert!(!force_run());
        assert!(!draw_player_weapon());
        assert_eq!(scale_ammo_pickup(0, 10), 10);
        // SAFETY: the scaffold bodies never dereference their pointer arguments.
        unsafe {
            assert!(!on_player_think(std::ptr::null_mut()));
            assert!(!on_bring_up_weapon(std::ptr::null_mut()));
            assert!(!on_explode(std::ptr::null_mut()));
            assert_eq!(scale_move(std::ptr::null_mut(), 25, -5), (25, -5));
            assert_eq!(status_ammo(std::ptr::null_mut()), None);
        }
    }
}
