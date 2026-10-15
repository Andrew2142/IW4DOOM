// SPDX-License-Identifier: GPL-3.0-or-later
//! The shadow MW2 `PlayerState` and the per-tic `UserCmd` (sheet 07 §5.2, §5.3 step 1).
//!
//! Doom owns the real player. The shadow carries only what MW2's weapon, ADS, sprint, spread
//! and breath code reads: angles, motion, stance (always standing), weapon timers and flags.

use dmw_format::consts::{MW2_RUN_SPEED, MW2_STAND_VIEW_HEIGHT};
use playerstate_iw4::{ENTITYNUM_NONE, PM_TYPE_DEAD, PlayerState, UserCmd, buttons, other_flags};

use crate::{TicButtons, TicInput};

/// `pm_type` of a living player (normal movement).
pub const PM_TYPE_NORMAL: i32 = 0;

/// A fresh standing shadow (sheet 07 §5.2 table).
pub fn fresh_player_state() -> PlayerState {
    let mut ps = PlayerState::ZERO;
    ps.pm_type = PM_TYPE_NORMAL;
    ps.view_height_current = MW2_STAND_VIEW_HEIGHT;
    ps.view_height_target = MW2_STAND_VIEW_HEIGHT as i32;
    ps.speed = MW2_RUN_SPEED as i32;
    ps.health = 100;
    ps.max_health = 100;
    ps.perks = [0, 0];
    ps.ground_entity_num = 0;
    ps.e_flags = 0;
    // `is_ads_allowed` refuses non-player entities (weapon_iw4/src/ads_allow.rs:32).
    ps.other_flags = other_flags::PLAYER;
    ps.hold_breath_scale = 1.0;
    ps
}

/// Copies Doom's per-tic motion and life into the shadow (sheet 07 §5.2 "per tic").
pub fn sync_from_input(ps: &mut PlayerState, input: &TicInput) {
    ps.origin = input.origin_mw2;
    ps.velocity = input.velocity_mw2;
    ps.viewangles = [input.pitch_deg, input.yaw_deg, 0.0];
    ps.ground_entity_num = if input.on_ground { 0 } else { ENTITYNUM_NONE };
    if input.alive {
        ps.health = ps.max_health.max(1);
        ps.pm_type = PM_TYPE_NORMAL;
    } else {
        ps.health = 0;
        ps.pm_type = PM_TYPE_DEAD;
    }
}

/// `ANGLE2SHORT`: degrees → 16-bit angle (`movement_iw4::ANGLE2SHORT`).
pub fn angle2short(deg: f32) -> i32 {
    ((deg * movement_iw4::ANGLE2SHORT) as i32) & 0xffff
}

/// Doom ticcmd move (run = 50) → MW2 stick (full = 127).
pub fn scale_move(v: i8) -> i8 {
    let scaled = (f32::from(v) * 127.0 / 50.0).round();
    scaled.clamp(-127.0, 127.0) as i8
}

/// `TicButtons` → `playerstate_iw4::buttons` bits (00 §8.2).
pub fn mw2_buttons(b: TicButtons) -> u32 {
    let mut out = 0;
    if b.attack {
        out |= buttons::ATTACK;
    }
    if b.sprint {
        out |= buttons::SPRINT;
    }
    if b.melee {
        out |= buttons::MELEE_CHARGE;
    }
    if b.reload {
        out |= buttons::RELOAD;
    }
    if b.ads {
        out |= buttons::ADS;
    }
    if b.hold_breath {
        out |= buttons::BREATH;
    }
    out
}

/// The tic's `UserCmd` (sheet 07 §5.3 step 1).
pub fn build_usercmd(input: &TicInput, server_time: i32, weapon: u32) -> UserCmd {
    UserCmd {
        server_time,
        buttons: mw2_buttons(input.buttons),
        angles: [angle2short(input.pitch_deg), angle2short(input.yaw_deg), 0],
        weapon: u16::try_from(weapon).unwrap_or(0),
        forwardmove: scale_move(input.forwardmove),
        rightmove: scale_move(input.sidemove),
        ..UserCmd::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_forward_passes_the_sprint_minimum() {
        assert_eq!(scale_move(50), 127);
        assert_eq!(scale_move(-50), -127);
        assert_eq!(scale_move(25), 64);
        assert_eq!(scale_move(0), 0);
        assert!(i32::from(scale_move(50)) > 105);
    }

    #[test]
    fn angle2short_wraps() {
        assert_eq!(angle2short(0.0), 0);
        assert_eq!(angle2short(90.0), 16384);
        assert_eq!(angle2short(-90.0), 49152);
    }

    #[test]
    fn button_mapping_matches_contract() {
        let b = TicButtons {
            ads: true,
            reload: true,
            sprint: true,
            melee: true,
            hold_breath: true,
            swap_last: true,
            attack: true,
        };
        assert_eq!(mw2_buttons(b), 0x1 | 0x2 | 0x4 | 0x10 | 0x800 | 0x2000);
        assert_eq!(mw2_buttons(TicButtons::default()), 0);
    }

    #[test]
    fn fresh_shadow_stands() {
        let ps = fresh_player_state();
        assert_eq!(ps.view_height_target, 60);
        assert_eq!(ps.speed, 190);
        assert_eq!(ps.health, 100);
        assert_ne!(ps.other_flags & other_flags::PLAYER, 0);
    }
}
