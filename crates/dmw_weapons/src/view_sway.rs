// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/render_anim/src/anim/view_sway.rs, Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: Bevy-free; doc comments added.

//! Look-driven weapon sway springs (port of iw4L `ViewSwayState`).

use weapon_iw4::{
    LookSway, SwaySpringState, WeaponSwayParams, lerp_sway_params, sway_config, sway_from_drive,
};

/// Sway driver state and its last contribution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewSwayState {
    look: LookSway,
    contribution: SwaySpringState,
}

impl ViewSwayState {
    /// Back to rest.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Current spring contribution, handed to placement (sheet 10 §5.3).
    pub fn springs(&self) -> SwaySpringState {
        self.contribution
    }

    /// Samples the view angles and updates the contribution (view_sway.rs:17-48).
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        hip: WeaponSwayParams,
        ads: WeaponSwayParams,
        angles: [f32; 3],
        frac: f32,
        ads_enabled: bool,
        overlay: bool,
        landing_scale: f32,
        dt: f32,
    ) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let config = sway_config();
        let sample = [f64::from(angles[0]), f64::from(angles[1])];
        if overlay && frac > 0.0 {
            if self.look.observe_only(sample, f64::from(dt), config) {
                self.contribution = SwaySpringState::default();
            }
            return;
        }
        if !self.look.sample(sample, f64::from(dt), config) {
            return;
        }
        let params = if ads_enabled {
            lerp_sway_params(hip, ads, frac)
        } else {
            hip
        };
        self.contribution = sway_from_drive(self.look.drive(config), params, landing_scale);
    }
}
