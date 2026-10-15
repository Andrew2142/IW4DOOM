// SPDX-License-Identifier: GPL-3.0-or-later
//! Contract constants from `docs/specs/00-overview-and-contracts.md` section 5.
//!
//! Nobody redefines these; `dmw_weapons` and the engine glue re-export them.

// ---- 5.1 Geometry and angles ------------------------------------------------

/// Doom map units per MW2 unit (inch). Doom player height 56 / MW2 standing height 70.
pub const DOOM_PER_MW2: f32 = 0.8;
/// MW2 units (inches) per Doom map unit.
pub const MW2_PER_DOOM: f32 = 1.25;

/// Doom FRACUNIT.
pub const FRACUNIT: i32 = 1 << 16;

/// Doom BAM: `angle_t` 0..2^32 counter-clockwise from +x. MW2 yaw: degrees counter-clockwise from +x.
pub const BAM_PER_DEGREE: f64 = 4294967296.0 / 360.0;

/// MW2 pitch convention: positive = looking down. Stored in this convention everywhere.
/// Freelook clamp. Shear at full 320 view = tan(32 deg) * 160 = about 100 px = viewheight/2 at 200.
pub const PITCH_LIMIT_DEG: f32 = 32.0;

/// Pitch is carried in `ExtTiccmd` as `i16` in 1/128 degree.
pub const PITCH_UNITS_PER_DEGREE: f32 = 128.0;

/// MW2 standing eye height (inches) used for the shadow `PlayerState`.
pub const MW2_STAND_VIEW_HEIGHT: f32 = 60.0;

/// Doom terminal running speed: thrust 50*2048/65536 = 1.5625 u/tic, friction 0xE800/0x10000
/// = 0.90625, so 1.5625 / 0.09375 = 16.667 u/tic = 16.667 * 1.25 * 35 = 729.17 in/s.
pub const DOOM_RUN_SPEED_MW2: f32 = 729.17;
/// MW2 `g_speed` (in/s).
pub const MW2_RUN_SPEED: f32 = 190.0;
/// Doom moves about 3.8x faster than an MW2 soldier. Every velocity handed to MW2 code (bob cycle,
/// spread motion, sway kinematics) is multiplied by this so Doom running "feels like" MW2 running
/// to the MW2 formulas. Positions are NOT scaled (they only matter as deltas, which are unused).
pub const SHADOW_VELOCITY_SCALE: f32 = MW2_RUN_SPEED / DOOM_RUN_SPEED_MW2; // about 0.2606

// ---- 5.2 Rendering ----------------------------------------------------------

/// Doom internal screen width.
pub const DOOM_SCREEN_W: usize = 320;
/// Doom internal screen height.
pub const DOOM_SCREEN_H: usize = 200;
/// Displayed pixel aspect (width / height) of a 320x200 frame shown at 4:3.
pub const PIXEL_ASPECT: f32 = 5.0 / 6.0;
/// MW2 `cg_fov` default (horizontal degrees at 4:3).
pub const MW2_CG_FOV: f32 = 65.0;
/// Doom's fixed horizontal field of view.
pub const DOOM_HFOV: f32 = 90.0;
/// Default viewmodel horizontal FOV (`doommw2.toml` `[render] viewmodel_fov`).
pub const VIEWMODEL_FOV_DEFAULT: f32 = 65.0;
/// Number of light colormaps in a WAD `COLORMAP` lump (excluding the invulnerability map).
pub const NUMCOLORMAPS: usize = 32;
/// Max colormap levels the directional term adds/removes (sheet 11 section 5.6).
pub const LIGHT_SHADE_STEPS: i32 = 6;

// ---- 5.3 Time ---------------------------------------------------------------

/// Doom tics per second.
pub const TICRATE: u32 = 35;

/// msec handed to MW2 logic for Doom tic number `tic` (0-based within the level).
pub const fn tic_msec(tic: u32) -> i32 {
    (((tic as u64 + 1) * 1000 / 35) - (tic as u64 * 1000 / 35)) as i32
}

/// MW2 `server_time` at the *start* of tic `tic` = `tic * 1000 / 35`.
pub const fn tic_server_time(tic: u32) -> i32 {
    (tic as u64 * 1000 / 35) as i32
}

// ---- 5.4 Ids ----------------------------------------------------------------

/// Index into `PackManifest::weapons`, 1-based. 0 = no weapon. Passed to `weapon_iw4` as `u32`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct DmwWeaponId(pub u16);

/// Doom `weapontype_t` values (`wp_fist` = 0 ... `wp_supershotgun` = 8); `NUMWEAPONS` = 9.
pub type DoomWeapon = i32;
/// Doom `ammotype_t` values (`am_clip` 0, `am_shell` 1, `am_cell` 2, `am_misl` 3, `am_noammo` 5).
pub type DoomAmmo = i32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tic_msec_sums_to_1000_per_35_tics() {
        let total: i32 = (0..TICRATE).map(tic_msec).sum();
        assert_eq!(total, 1000);
        assert!((0..TICRATE * 10).all(|t| matches!(tic_msec(t), 28 | 29)));
    }

    #[test]
    fn tic_server_time_matches_msec_prefix_sums() {
        let mut acc = 0;
        for t in 0..200 {
            assert_eq!(tic_server_time(t), acc);
            acc += tic_msec(t);
        }
    }

    #[test]
    fn shadow_velocity_scale_matches_contract() {
        assert!((SHADOW_VELOCITY_SCALE - 0.2606).abs() < 1e-3);
    }
}
