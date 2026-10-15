// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/render_anim/src/anim/view_kick_state.rs, Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: Bevy-free; kick facts are dmw_format::KickFacts; recoil samples come from DmwRng.

//! Camera view kick and gun recoil state (port of iw4L `ViewKickState`).

use dmw_format::KickFacts;
use weapon_iw4::{
    CAMERA_LIMIT_DEGREES, CAMERA_PEAK_SECONDS, FireRecoilPsScales, GunKickRange,
    GunRecoilPlacementState, GunRecoilResponse, MODEL_PEAK_SECONDS, RecoilAxis, ViewKickRange,
    calculate_weapon_position_gun_recoil, fire_recoil_gun_range, fire_recoil_view_range,
    recoil_ads_weight, weapon_fire_recoil,
};

use crate::rng::DmwRng;

/// Camera kick springs plus the gun recoil placement state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewKickState {
    camera: [RecoilAxis; 3],
    /// Camera kick (pitch, yaw, roll), degrees. Visual only (00 D11).
    pub kick_angles: [f32; 3],
    /// Gun recoil handed to placement (sheet 10).
    pub gun: GunRecoilPlacementState,
}

impl ViewKickState {
    /// Back to rest (iw4L resets on weapon change, render_anim/src/occupancy/view_kick.rs:146).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Adds the recoil impulse of one accepted shot (view_kick_state.rs:34-52). Draws exactly
    /// four samples from `rng`.
    pub fn seed_fire(
        &mut self,
        kick: &KickFacts,
        frac: f32,
        weap_flags: u32,
        recoil_scale: i32,
        reduce_window_active: bool,
        rng: &mut DmwRng,
    ) {
        self.seed_fire_scaled(
            kick,
            frac,
            FireRecoilPsScales {
                reduce_window_active,
                reduced_percent: 0.0,
                weap_flags,
                recoil_scale,
            },
            rng,
        );
    }

    /// view_kick_state.rs:53-97.
    pub fn seed_fire_scaled(
        &mut self,
        k: &KickFacts,
        frac: f32,
        mut ps: FireRecoilPsScales,
        rng: &mut DmwRng,
    ) {
        let view = fire_recoil_view_range(
            frac,
            ViewKickRange {
                pitch_min: k.hip_view_kick_pitch_min,
                pitch_max: k.hip_view_kick_pitch_max,
                yaw_min: k.hip_view_kick_yaw_min,
                yaw_max: k.hip_view_kick_yaw_max,
            },
            ViewKickRange {
                pitch_min: k.ads_view_kick_pitch_min,
                pitch_max: k.ads_view_kick_pitch_max,
                yaw_min: k.ads_view_kick_yaw_min,
                yaw_max: k.ads_view_kick_yaw_max,
            },
        );
        let gun = fire_recoil_gun_range(
            frac,
            GunKickRange {
                pitch_min: k.hip_gun_kick_pitch_min,
                pitch_max: k.hip_gun_kick_pitch_max,
                yaw_min: k.hip_gun_kick_yaw_min,
                yaw_max: k.hip_gun_kick_yaw_max,
            },
            GunKickRange {
                pitch_min: k.ads_gun_kick_pitch_min,
                pitch_max: k.ads_gun_kick_pitch_max,
                yaw_min: k.ads_gun_kick_yaw_min,
                yaw_max: k.ads_gun_kick_yaw_max,
            },
        );
        if ps.reduce_window_active {
            let t = recoil_ads_weight(frac);
            ps.reduced_percent = k.hip_gun_kick_reduced_kick_percent * (1.0 - t)
                + k.ads_gun_kick_reduced_kick_percent * t;
        }
        let samples = [rng.unit01(), rng.unit01(), rng.unit01(), rng.unit01()];
        let impulse = weapon_fire_recoil(view, gun, ps, samples);
        for (axis, amplitude) in self.camera.iter_mut().zip(impulse.camera_peak) {
            axis.add_peak(f64::from(amplitude), CAMERA_PEAK_SECONDS);
        }
        for (axis, amplitude) in self.gun.axes.iter_mut().zip(impulse.model_peak) {
            axis.add_peak(f64::from(amplitude), MODEL_PEAK_SECONDS);
        }
    }

    /// Advances the springs by `dt_secs` (view_kick_state.rs:98-116).
    pub fn advance(&mut self, kick: &KickFacts, frac: f32, dt_secs: f32) {
        if !dt_secs.is_finite() || dt_secs <= 0.0 {
            return;
        }
        let dt = f64::from(dt_secs);
        for (axis, out) in self.camera.iter_mut().zip(&mut self.kick_angles) {
            axis.advance(dt, CAMERA_PEAK_SECONDS);
            *out = axis.bounded(CAMERA_LIMIT_DEGREES) as f32;
        }
        calculate_weapon_position_gun_recoil(
            &mut self.gun,
            dt as f32,
            frac,
            true,
            GunRecoilResponse::default(),
            GunRecoilResponse::default(),
            [kick.gun_max_pitch, kick.gun_max_yaw],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_kicks_then_settles() {
        let pack = dmw_format::Pack::fixture();
        let facts = pack.facts(dmw_format::DmwWeaponId(2)).kick.clone();
        let mut rng = DmwRng::from_seed(7);
        let mut k = ViewKickState::default();
        k.seed_fire(&facts, 0.0, 0, 0, false, &mut rng);
        k.advance(&facts, 0.0, 0.028);
        let peak: f32 = k.kick_angles.iter().map(|a| a.abs()).sum();
        assert!(peak > 0.0);
        for _ in 0..70 {
            k.advance(&facts, 0.0, 0.028);
        }
        assert!(k.kick_angles.iter().all(|a| a.abs() < 0.01));
    }
}
