// SPDX-License-Identifier: GPL-3.0-or-later
//! Gun placement (owned by sheet 10 §5.3).
//!
//! Sheet 07 calls [`step`] and [`gun_angle_offset`] at the end of every tic (07 §5.3 step 13).
//! Until sheet 10 lands, this file only defines the types and signatures of 10 §5.3 with a
//! placeholder body.

use dmw_format::WeaponFacts;
use playerstate_iw4::PlayerState;
use weapon_iw4::{GunRecoilPlacementState, SwaySpringState};

use crate::GunPlacement;

/// Placement state carried between tics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacementState {
    #[allow(dead_code)] // sheet 10 replaces this
    inner: weapon_iw4::WeaponPlacementState,
}

/// Everything placement reads for one tic.
#[derive(Clone, Copy, Debug)]
pub struct ViewInputs<'a> {
    /// Shadow player state after the weapon tic.
    pub ps: &'a PlayerState,
    /// Facts of the weapon in hand.
    pub facts: &'a WeaponFacts,
    /// Sway springs (`ViewSwayState::springs`).
    pub sway: SwaySpringState,
    /// Gun recoil (`ViewKickState::gun`).
    pub gun_recoil: GunRecoilPlacementState,
    /// Camera kick (pitch, yaw, roll), degrees.
    pub view_kick_deg: [f32; 3],
    /// Tic length.
    pub msec: i32,
}

/// Advances placement and returns the view-space gun placement.
// sheet 10 replaces this: placeholder returns the rest placement.
pub fn step(state: &mut PlacementState, v: &ViewInputs) -> GunPlacement {
    let _ = (state, v);
    GunPlacement::default()
}

/// (pitch, yaw) degrees between the gun axis and the view.
// sheet 10 replaces this: placeholder returns no offset.
pub fn gun_angle_offset(p: &GunPlacement, v: &ViewInputs) -> [f32; 2] {
    let _ = (p, v);
    [0.0, 0.0]
}
