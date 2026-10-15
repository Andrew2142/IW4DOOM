// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 weapon simulation bridge (sheet 07) and viewmodel animation (sheet 10).
//!
//! Drives iw4L's retail weapon and movement logic from Doom's 35 Hz tic. This crate
//! is pure, deterministic safe Rust: it never depends on the engine, the rasterizer or Bevy.
//!
//! The public API is 00 §7.2. Sheet 08 owns one [`WeaponSim`], calls
//! [`WeaponSim::start_level`] at level start and [`WeaponSim::tic`] once per Doom tic.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dmw_format::{DmwWeaponId, DoomAmmo, Pack};

pub mod anim;
pub mod breath;
pub mod clip_scheduler;
pub mod controller;
pub mod convert;
pub mod events;
pub mod inventory;
pub mod notetracks;
pub mod placement;
mod pose;
pub mod rng;
pub mod shadow;
mod sim;
pub mod spread;
pub mod switch;
pub mod view_kick;
pub mod view_sway;

pub use anim::{AnimError, SurfaceMask, ViewmodelAnimator, ViewmodelPose};
pub use dmw_format::consts;
pub use events::{MeleeRequest, ShotRequest, SimEvent, SoundCue};
pub use inventory::{Inventory, SimSave};
pub use rng::DmwRng;
pub use sim::WeaponSim;

/// Everything `WeaponSim::start_level` needs (00 §7.2).
#[derive(Debug)]
pub struct LevelStart<'a> {
    /// The pack the sim was created with.
    pub pack: &'a Pack,
    /// `DmwRng` seed from `(gameskill, gameepisode, gamemap)` ([`DmwRng::level_seed`]).
    pub seed: u32,
    /// Owned MW2 weapons and magazines carried into the level.
    pub inventory: Inventory,
    /// `DmwWeaponId.0` → Doom ammo pool.
    pub ammo_map: BTreeMap<u16, DoomAmmo>,
    /// `[balance] sprint_speed_scale`.
    pub sprint_speed_scale: f32,
    /// `[balance] weapon_move_scales`.
    pub weapon_move_scales: bool,
    /// `[render] viewmodel_fov` (MW2 `cg_fov` space).
    pub viewmodel_fov: f32,
}

/// Decoded `ExtTiccmd::buttons` (00 §8.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TicButtons {
    /// Hold to aim down sights.
    pub ads: bool,
    /// Reload.
    pub reload: bool,
    /// Hold to sprint.
    pub sprint: bool,
    /// Melee.
    pub melee: bool,
    /// Hold breath (scoped).
    pub hold_breath: bool,
    /// Switch to the last held weapon (resolved by sheet 08).
    pub swap_last: bool,
    /// Fire.
    pub attack: bool,
}

/// One Doom tic of input, already in MW2 conventions (00 §7.2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TicInput {
    /// `leveltime`.
    pub tic: u32,
    /// Player alive.
    pub alive: bool,
    /// Decoded buttons.
    pub buttons: TicButtons,
    /// MW2 target from Doom's weapon change, already mapped.
    pub requested_weapon: Option<DmwWeaponId>,
    /// Yaw, MW2 convention, from `mo->angle`.
    pub yaw_deg: f32,
    /// Pitch, MW2 convention (positive = down), from `PlayerExt`.
    pub pitch_deg: f32,
    /// Yaw at the previous tic.
    pub prev_yaw_deg: f32,
    /// Pitch at the previous tic.
    pub prev_pitch_deg: f32,
    /// Position, inches.
    pub origin_mw2: [f32; 3],
    /// Velocity, inches/s, already multiplied by `SHADOW_VELOCITY_SCALE`.
    pub velocity_mw2: [f32; 3],
    /// Standing on the floor.
    pub on_ground: bool,
    /// Doom ticcmd forward move.
    pub forwardmove: i8,
    /// Doom ticcmd side move.
    pub sidemove: i8,
    /// Doom ammo pools, indexed by `DoomAmmo`.
    pub pools: [i32; 4],
}

/// What one tic produced (00 §7.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TicOutput {
    /// Events in order.
    pub events: Vec<SimEvent>,
    /// Pools after reloads.
    pub pools: [i32; 4],
    /// Camera kick (pitch, yaw, roll), visual only.
    pub view_kick_deg: [f32; 3],
    /// Product of weapon, ADS and sprint scales (sheet 08 §5.9).
    pub move_speed_scale: f32,
    /// MW2 sprint active.
    pub sprinting: bool,
    /// ADS fraction 0..1.
    pub ads_frac: f32,
    /// Current spread half-angle for the crosshair, degrees.
    pub spread_deg: f32,
    /// Weapon in hand (`None` during the vanilla handoff).
    pub held: Option<DmwWeaponId>,
    /// The MW2 drop finished this tic; Doom may raise a vanilla weapon.
    pub weapon_lowered_for_vanilla: bool,
}

/// View-space gun placement: origin (inches) and angles (degrees), sheet 10.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunPlacement {
    /// Origin, MW2 view axes, inches.
    pub origin: [f32; 3],
    /// Angles (pitch, yaw, roll), degrees.
    pub angles: [f32; 3],
}

/// Snapshot for presentation, produced after `tic()` (00 §7.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    /// Weapon in hand.
    pub weapon: Option<DmwWeaponId>,
    /// `weapon_iw4::WeaponState` as `i32`.
    pub weaponstate: i32,
    /// Raw `weap_anim`; edges are consumed by sheet 10.
    pub weap_anim: i32,
    /// ADS fraction 0..1.
    pub ads_frac: f32,
    /// Gun placement (sheet 10).
    pub placement: GunPlacement,
    /// (pitch, yaw) degrees between gun axis and view (sheet 10 §5.3).
    pub gun_angle_offset: [f32; 2],
    /// Camera kick (pitch, yaw, roll), degrees.
    pub view_kick_deg: [f32; 3],
    /// Spread half-angle, degrees.
    pub spread_deg: f32,
    /// Scope overlay fully up (sheet 12).
    pub overlay_active: bool,
    /// World FOV override in Doom space (sheet 06 §5.5).
    pub zoom_hfov_deg: Option<f32>,
    /// Gun FOV in MW2 space (sheet 07 §5.11).
    pub viewmodel_fov_deg: f32,
    /// Rounds in magazine.
    pub clip: i32,
    /// Rounds in the reserve pool.
    pub stock: i32,
}

#[cfg(test)]
mod tests {
    /// Touches one item from every iw4L dependency so a broken path dependency
    /// fails `cargo test`.
    #[test]
    fn links() {
        let facts = weapon_iw4::WeaponCombatFacts::none();
        assert_eq!(facts.fire_time_ms, 0);
        assert_eq!(movement_iw4::BUTTON_ADS, playerstate_iw4::buttons::ADS);
        let len = math_iw4::vec3_length([3.0, 4.0, 0.0]);
        assert!((len - 5.0).abs() < 1e-4);
        assert_eq!(anim_iw4::PartBits::CAPACITY, 192);
        assert!(std::mem::size_of::<xmodel_runtime::DObjAnimRuntime>() > 0);
        assert_eq!(glam::Mat4::IDENTITY.w_axis.w.to_bits(), 1.0f32.to_bits());
        assert_eq!(dmw_format::consts::TICRATE, 35);
    }
}
