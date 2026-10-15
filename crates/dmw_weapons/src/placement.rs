// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/render_anim/src/occupancy/fpv_present.rs (`apply_fpv_placement`, lines 1032-1250) and
// crates/render_anim/src/occupancy/view_kick.rs (`iw_view_placement_to_bevy_camera_local`),
// Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: Bevy-free; no killcam, dual wield, landing or cg_gun offsets;
// MW2 view axes instead of the Bevy camera fold.

//! Gun placement (sheet 10 §5.3): where the gun sits in front of the eye.
//!
//! Sheet 07 calls [`step`] and [`gun_angle_offset`] at the end of every tic (07 §5.3 step 13).
//! [`step`] runs `weapon_iw4::weapon_placement_assemble` (movement offsets, bob, idle sway, look
//! sway, gun recoil) with the call sequence of iw4L's `apply_fpv_placement`.
//!
//! Convention of the result (from iw4L `iw_view_placement_to_bevy_camera_local`, which maps
//! `[fwd, right, up]` to Bevy `(right, up, -fwd)`): `GunPlacement::origin` is
//! `[forward, right, up]` inches and `angles` are `[pitch, yaw, roll]` degrees relative to the
//! view, turned into a rotation with `math_iw4::angles_to_axis`. [`placement_matrix`] converts
//! both into MW2 view axes (x forward, y left, z up).

use dmw_format::WeaponFacts;
use glam::{Mat4, Vec3, Vec4};
use playerstate_iw4::PlayerState;
use weapon_iw4::{
    GunRecoilPlacementState, GunRecoilResponse, PLACEMENT_ASSEMBLE_STEP_COUNT,
    StanceTransitionFadeGlobals, SwaySpringState, WeaponBobInputs, WeaponBobWaveformInputs,
    WeaponMovementKinematics, WeaponPlacementAssembleStep, WeaponPlacementPsInputs,
    WeaponPlacementState, WeaponStanceStaticOfsInputs, calculate_weapon_movement_bob_waveform,
    viewweapon_save_gun_pitch_yaw, weapon_placement_assemble,
};

use crate::GunPlacement;

/// Placement state carried between tics: exactly the fields iw4L keeps in `SessionViewKick`
/// across frames (fpv_present.rs:1076-1088, 1182-1185). Everything else is rebuilt every tic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacementState {
    movement_origin: [f32; 3],
    movement_angles: [f32; 3],
    weap_idle_time: i32,
    last_idle_factor: f32,
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

/// Advances placement and returns the view-space gun placement (fpv_present.rs:1076-1188).
pub fn step(state: &mut PlacementState, v: &ViewInputs) -> GunPlacement {
    let ps = v.ps;
    let fpv = &v.facts.fpv;
    // 1. Springs and recoil are injected every tic; damage kick is off (damage_time 0).
    let mut inner = WeaponPlacementState {
        sway_springs: v.sway,
        gun_recoil: v.gun_recoil,
        movement_origin: state.movement_origin,
        movement_angles: state.movement_angles,
        weap_idle_time: state.weap_idle_time,
        last_idle_factor: state.last_idle_factor,
        ..WeaponPlacementState::default()
    };
    // 2.
    let ps_in = WeaponPlacementPsInputs {
        e_flags: ps.e_flags,
        weapon_pos_frac: ps.f_weapon_pos_frac,
        weapon_time: ps.weapon_time,
        aim_down_sight: fpv.aim_down_sight,
        overlay_reticle: fpv.overlay_reticle,
        weapon_transition_active: false,
        lean_fraction: 0.0,
    };
    // 3.
    let stance = WeaponStanceStaticOfsInputs {
        ducked_ofs: fpv.ducked_ofs,
        prone_ofs: fpv.prone_ofs,
        ads_aim_pitch: fpv.ads_aim_pitch,
        night_vision_wear_time: 0,
    };
    // 4.
    let xyspeed = math_iw4::vec3_length([ps.velocity[0], ps.velocity[1], 0.0]);
    let kinematics = WeaponMovementKinematics {
        xyspeed,
        speed: ps.speed as f32,
        velocity: ps.velocity,
        viewangles: ps.viewangles,
        weaponstate: ps.weaponstate_primary,
        weaponstate_secondary: 0,
        pm_flags: ps.pm_flags,
        frametime: v.msec as f32 / 1000.0,
    };
    // 5.
    let waveform = calculate_weapon_movement_bob_waveform(WeaponBobWaveformInputs {
        bob_cycle: (ps.bob_cycle as u32 & 0xff) as u8,
        xyspeed,
        view_height_target: ps.view_height_target,
        pm_flags: ps.pm_flags,
        weapon_pos_frac: ps.f_weapon_pos_frac,
    });
    // 6.
    let mut idle = fpv.idle;
    if fpv.can_hold_breath {
        idle.ads_idle_amount *= ps.hold_breath_scale;
    }
    // 7. dt = 0: gun recoil is advanced by ViewKickState::advance (07 step 12).
    let mut steps = [WeaponPlacementAssembleStep::Sway; PLACEMENT_ASSEMBLE_STEP_COUNT];
    let contrib = weapon_placement_assemble(
        &mut inner,
        ps_in,
        stance,
        StanceTransitionFadeGlobals::default(),
        fpv.movement,
        kinematics,
        WeaponBobInputs {
            ads_bob_factor: fpv.ads_bob_factor,
        },
        idle,
        Some(waveform),
        GunRecoilResponse::default(),
        GunRecoilResponse::default(),
        v.facts.kick.gun_max_pitch,
        v.facts.kick.gun_max_yaw,
        0.0,
        &mut steps,
    );
    state.movement_origin = inner.movement_origin;
    state.movement_angles = inner.movement_angles;
    state.weap_idle_time = inner.weap_idle_time;
    state.last_idle_factor = inner.last_idle_factor;
    // 8.
    GunPlacement {
        origin: contrib.origin,
        angles: contrib.angles,
    }
}

/// (pitch, yaw) degrees between the gun axis and the view (fpv_present.rs:1197-1229). Zero
/// unless the weapon is an iron-sight weapon with ADS above 0.
pub fn gun_angle_offset(p: &GunPlacement, v: &ViewInputs) -> [f32; 2] {
    let ps = v.ps;
    let fpv = &v.facts.fpv;
    let refdef = [
        ps.viewangles[0] + v.view_kick_deg[0],
        ps.viewangles[1] + v.view_kick_deg[1],
        0.0,
    ];
    let [gp, gy] = viewweapon_save_gun_pitch_yaw(
        p.angles,
        refdef,
        fpv.aim_down_sight,
        ps.f_weapon_pos_frac,
        fpv.overlay_reticle,
    );
    [
        math_iw4::angle_subtract(gp, ps.viewangles[0]),
        math_iw4::angle_subtract(gy, ps.viewangles[1]),
    ]
}

/// Eye-from-placement transform in MW2 view axes (x forward, y left, z up, inches).
///
/// iw4L places the gun root at Bevy camera-local `(right, up, -fwd)` = `(o[1], o[2], -o[0])`
/// rotated by `fold · R · fold⁻¹` with `R = [fwd | left | up]` from `angles_to_axis`. Without the
/// fold that is the translation `(o[0], -o[1], o[2])` and the rotation `R`.
pub fn placement_matrix(p: &GunPlacement) -> Mat4 {
    let [fwd, left, up] = math_iw4::angles_to_axis(p.angles);
    Mat4::from_cols(
        Vec3::from_array(fwd).extend(0.0),
        Vec3::from_array(left).extend(0.0),
        Vec3::from_array(up).extend(0.0),
        Vec4::new(p.origin[0], -p.origin[1], p.origin[2], 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_matrix_axes() {
        let id = placement_matrix(&GunPlacement::default());
        assert!(id.abs_diff_eq(Mat4::IDENTITY, 1e-6));
        // Origin is [forward, right, up]: +right lands on -y (MW2 y is left).
        let m = placement_matrix(&GunPlacement {
            origin: [1.0, 2.0, 3.0],
            angles: [0.0; 3],
        });
        assert!(
            m.transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(1.0, -2.0, 3.0), 1e-6)
        );
        // Positive yaw turns the muzzle (+x) toward +y (left); positive pitch toward -z (down).
        let yaw = placement_matrix(&GunPlacement {
            origin: [0.0; 3],
            angles: [0.0, 90.0, 0.0],
        });
        assert!(yaw.transform_vector3(Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
        let pitch = placement_matrix(&GunPlacement {
            origin: [0.0; 3],
            angles: [90.0, 0.0, 0.0],
        });
        assert!(
            pitch
                .transform_vector3(Vec3::X)
                .abs_diff_eq(Vec3::NEG_Z, 1e-5)
        );
    }
}
