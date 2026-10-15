// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/sim/src/combat.rs (spread_direction_on_plane), Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: draws from DmwRng instead of MatchRng; spread helpers added.

//! Aim spread helpers and pellet direction sampling (sheet 07 §5.6).

use playerstate_iw4::PlayerState;
use weapon_iw4::{
    SpreadOverrideState, WeaponCombatFacts, fire_weapon_spread_degrees, get_spread_for_weapon,
    perk_weap_spread_multiplier,
};

use crate::rng::DmwRng;

/// Spread cone half-angle in degrees for the current `ps` (combat.rs:807-821).
pub fn current_spread_degrees(ps: &PlayerState, facts: &WeaponCombatFacts) -> f32 {
    let ads_frac = ps.f_weapon_pos_frac.clamp(0.0, 1.0);
    let cone = get_spread_for_weapon(
        ps.view_height_current,
        ps.spread_override,
        SpreadOverrideState::from_i32(ps.spread_override_state),
        &facts.spread_facts(),
        perk_weap_spread_multiplier(ps.perks[0]),
    );
    fire_weapon_spread_degrees(cone, facts.ads_spread, ads_frac, ps.aim_spread_scale)
}

/// One pellet direction inside a cone of `spread_degrees` around `angles`
/// (port of combat.rs:1125-1150). Draws exactly two values from `rng` when
/// `spread_degrees > 0`, none otherwise.
pub fn spread_direction_on_plane(
    angles: [f32; 3],
    spread_degrees: f32,
    rng: &mut DmwRng,
    plane: f32,
) -> [f32; 3] {
    let (forward, right, up) = math_iw4::angle_vectors(angles);
    if spread_degrees <= 0.0 {
        return forward;
    }
    let unit = |draw: u32| draw as f32 / u32::MAX as f32;
    let radius = unit(rng.next_u32());
    let theta = unit(rng.next_u32()) * core::f32::consts::TAU;
    let lateral = plane * spread_degrees.to_radians().tan() * radius;
    let q = [
        plane * forward[0] + lateral * (theta.cos() * right[0] + theta.sin() * up[0]),
        plane * forward[1] + lateral * (theta.cos() * right[1] + theta.sin() * up[1]),
        plane * forward[2] + lateral * (theta.cos() * right[2] + theta.sin() * up[2]),
    ];
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if len > 0.0 {
        [q[0] / len, q[1] / len, q[2] / len]
    } else {
        forward
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn angle_between(a: [f32; 3], b: [f32; 3]) -> f32 {
        let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
        d.acos().to_degrees()
    }

    #[test]
    fn pellets_stay_inside_the_cone() {
        let mut rng = DmwRng::from_seed(99);
        let angles = [10.0, 45.0, 0.0];
        let (fwd, _, _) = math_iw4::angle_vectors(angles);
        for _ in 0..500 {
            let d = spread_direction_on_plane(angles, 6.0, &mut rng, 1.0);
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4);
            assert!(angle_between(d, fwd) <= 6.0 + 1e-3);
        }
    }

    #[test]
    fn zero_spread_is_forward_and_draws_nothing() {
        let mut rng = DmwRng::from_seed(5);
        let before = rng;
        let d = spread_direction_on_plane([0.0, 90.0, 0.0], 0.0, &mut rng, 1.0);
        assert_eq!(rng, before);
        assert!(d[1] > 0.999);
    }
}
