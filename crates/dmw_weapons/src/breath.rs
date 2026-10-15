// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/movement_iw4/src/breath.rs, Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: made public (upstream is pub(crate)); doc comments added.

//! Sniper hold-breath (sheet 07 §5.7). `ps.hold_breath_scale` scales the ADS idle sway in
//! placement (sheet 10 §5.3).

use playerstate_iw4::{
    BREATH_GASP_TIME_MS, PlayerState, breath_hold_time_ms, buttons, pm_flags, weap_flags,
};

/// Advances the hold-breath timer and `hold_breath_scale` by `msec`.
///
/// Verbatim port of iw4L `movement_iw4::breath::update_hold_breath` (breath.rs:5-37), which
/// iw4L calls from `pmove` (movement_iw4/src/single.rs:157) right after `update_ads_frac`.
#[allow(clippy::float_cmp)] // verbatim port: exact 1.0 test as upstream
pub fn update_hold_breath(ps: &mut PlayerState, buttons: u32, msec: i32, eligible: bool) {
    let was_holding = ps.weap_flags & weap_flags::HOLD_BREATH != 0;
    let wants_hold = eligible
        && ps.f_weapon_pos_frac == 1.0
        && buttons & buttons::BREATH != 0
        && ps.pm_flags & (pm_flags::SPRINTING | pm_flags::MANTLE | pm_flags::LADDER) == 0
        && ps.weap_flags & weap_flags::OFFHAND_VIEW == 0
        && ps.health > 0;
    let hold_time = breath_hold_time_ms(ps);
    let holding = wants_hold && (was_holding || ps.hold_breath_timer == 0);
    ps.weap_flags &= !weap_flags::HOLD_BREATH;
    if holding {
        ps.hold_breath_timer = ps.hold_breath_timer.saturating_add(msec);
        if ps.hold_breath_timer >= hold_time {
            ps.hold_breath_timer = hold_time + BREATH_GASP_TIME_MS;
        } else {
            ps.weap_flags |= weap_flags::HOLD_BREATH;
        }
    } else {
        ps.hold_breath_timer = ps.hold_breath_timer.saturating_sub(msec).max(0);
    }
    let holding = ps.weap_flags & weap_flags::HOLD_BREATH != 0;
    let target = if holding {
        0.0
    } else {
        1.0 + 3.5 * ps.hold_breath_timer as f32 / (hold_time + BREATH_GASP_TIME_MS) as f32
    };
    let target = 1.0 + (target - 1.0) * ps.f_weapon_pos_frac;
    let rate = if holding { 1.0 } else { 6.0 };
    ps.hold_breath_scale += (target - ps.hold_breath_scale) * (rate * msec as f32 * 0.001).min(1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.f_weapon_pos_frac = 1.0;
        ps.health = 100;
        ps.hold_breath_scale = 1.0;
        ps
    }

    #[test]
    fn holding_sets_flag_and_runs_out_into_gasp() {
        let mut ps = scoped();
        update_hold_breath(&mut ps, buttons::BREATH, 28, true);
        assert_ne!(ps.weap_flags & weap_flags::HOLD_BREATH, 0);
        assert!(ps.hold_breath_scale < 1.0);
        for _ in 0..200 {
            update_hold_breath(&mut ps, buttons::BREATH, 28, true);
        }
        assert_eq!(ps.weap_flags & weap_flags::HOLD_BREATH, 0);
        assert!(ps.hold_breath_timer > 0);
    }

    #[test]
    fn ineligible_never_holds() {
        let mut ps = scoped();
        update_hold_breath(&mut ps, buttons::BREATH, 28, false);
        assert_eq!(ps.weap_flags & weap_flags::HOLD_BREATH, 0);
    }
}
