// SPDX-License-Identifier: GPL-3.0-or-later
//! Doom <-> MW2 unit conversions (00 section 5.1). Implemented once, here.
//!
//! Angles go through `f64` because a Doom BAM angle has 32 significant bits, which an `f32`
//! cannot hold. The `f32` wrappers are for callers that only need display precision.

use crate::consts::{BAM_PER_DEGREE, DOOM_PER_MW2, FRACUNIT, MW2_PER_DOOM, TICRATE};

/// Doom fixed-point position or length (`fixed_t`) to MW2 inches.
pub fn doom_fixed_to_mw2(x: i32) -> f32 {
    x as f32 / FRACUNIT as f32 * MW2_PER_DOOM
}

/// MW2 range or radius in inches to Doom `fixed_t`.
pub fn mw2_range_to_doom_fixed(inches: f32) -> i32 {
    (inches * DOOM_PER_MW2 * FRACUNIT as f32) as i32
}

/// Doom BAM `angle_t` to MW2 yaw degrees, wrapped to (-180, 180].
pub fn bam_to_degrees(angle: u32) -> f64 {
    let deg = f64::from(angle) / BAM_PER_DEGREE;
    if deg > 180.0 { deg - 360.0 } else { deg }
}

/// MW2 yaw degrees (any value) to a Doom BAM `angle_t`.
pub fn degrees_to_bam(yaw: f64) -> u32 {
    (yaw.rem_euclid(360.0) * BAM_PER_DEGREE) as u64 as u32
}

/// [`bam_to_degrees`] narrowed to `f32` (the type `dmw_weapons` carries).
pub fn doom_angle_to_mw2_yaw(angle: u32) -> f32 {
    bam_to_degrees(angle) as f32
}

/// [`degrees_to_bam`] for an `f32` yaw.
pub fn mw2_yaw_to_doom_angle(yaw: f32) -> u32 {
    degrees_to_bam(f64::from(yaw))
}

/// MW2 pitch degrees (positive = looking down) to a Doom slope (`fixed_t`, positive = up).
pub fn mw2_pitch_to_doom_slope(pitch_deg: f32) -> i32 {
    (-pitch_deg.to_radians().tan() * FRACUNIT as f32) as i32
}

/// Doom velocity in `fixed_t` per tic to MW2 inches per second.
pub fn doom_velocity_to_mw2(v: i32) -> f32 {
    v as f32 / FRACUNIT as f32 * MW2_PER_DOOM * TICRATE as f32
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    /// T3.1: BAM <-> degrees round trip within 1 BAM.
    #[test]
    fn bam_degrees_round_trip_within_one_bam() {
        // Deterministic spread over the whole circle, including both ends and the 180 deg seam.
        let mut angles: Vec<u32> = (0..10_000u64).map(|i| (i * 429_496) as u32).collect();
        angles.extend([
            0,
            1,
            u32::MAX,
            1 << 31,
            (1 << 31) - 1,
            (1 << 31) + 1,
            1 << 30,
        ]);
        for a in angles {
            let deg = bam_to_degrees(a);
            assert!(deg > -180.0 && deg <= 180.0, "{a} -> {deg}");
            let back = degrees_to_bam(deg);
            let diff = (i64::from(back) - i64::from(a)).rem_euclid(1 << 32);
            let diff = diff.min((1 << 32) - diff);
            assert!(diff <= 1, "{a} -> {deg} -> {back}");
        }
    }

    #[test]
    fn bam_degrees_known_values() {
        assert_eq!(bam_to_degrees(0), 0.0);
        assert_eq!(bam_to_degrees(1 << 30), 90.0);
        assert_eq!(bam_to_degrees(1 << 31), 180.0);
        assert_eq!(bam_to_degrees(3 << 30), -90.0);
        assert_eq!(degrees_to_bam(-90.0), 3 << 30);
        assert_eq!(degrees_to_bam(450.0), 1 << 30);
        assert_eq!(degrees_to_bam(360.0), 0);
        // Just below a full turn truncates to the last BAM value, one step short of zero.
        assert_eq!(degrees_to_bam(359.999_999_999_999_94), u32::MAX);
        assert_eq!(degrees_to_bam(-1e-12), u32::MAX);
        assert_eq!(mw2_yaw_to_doom_angle(90.0), 1 << 30);
        assert_eq!(doom_angle_to_mw2_yaw(1 << 30), 90.0);
    }

    /// T3.1: looking down (positive MW2 pitch) gives a negative Doom slope.
    #[test]
    fn pitch_to_slope_sign() {
        assert!(mw2_pitch_to_doom_slope(10.0) < 0);
        assert!(mw2_pitch_to_doom_slope(-10.0) > 0);
        assert_eq!(mw2_pitch_to_doom_slope(0.0), 0);
        // tan(45 deg) = 1 -> one FRACUNIT, within f32 error.
        assert!((mw2_pitch_to_doom_slope(-45.0) - FRACUNIT).abs() <= 2);
    }

    /// T3.1: inches <-> fixed.
    #[test]
    fn inches_fixed_round_trip() {
        // One Doom unit is 1.25 inches.
        assert_eq!(doom_fixed_to_mw2(FRACUNIT), 1.25);
        // 1.25 inches is 1 Doom unit (a range goes through DOOM_PER_MW2 = 0.8).
        assert_eq!(mw2_range_to_doom_fixed(1.25), FRACUNIT);
        for x in [-100 * FRACUNIT, -1, 0, 1, 12_345, 56 * FRACUNIT, 1 << 24] {
            let inches = doom_fixed_to_mw2(x);
            let back = mw2_range_to_doom_fixed(inches);
            // f32 has 24 bits; allow a relative error of 2^-22 plus one unit.
            let tol = (f64::from(x).abs() / f64::from(1 << 22)).ceil() as i32 + 1;
            assert!((back - x).abs() <= tol, "{x} -> {inches} -> {back}");
        }
    }

    #[test]
    fn velocity_conversion() {
        // 1 Doom unit per tic = 1.25 in * 35 tics/s.
        assert_eq!(doom_velocity_to_mw2(FRACUNIT), 43.75);
        assert_eq!(doom_velocity_to_mw2(0), 0.0);
        // Terminal run speed (16.667 u/tic) is 729.17 in/s (consts.rs).
        let run = doom_velocity_to_mw2((16.666_667 * FRACUNIT as f32) as i32);
        assert!(
            (run - crate::consts::DOOM_RUN_SPEED_MW2).abs() < 0.05,
            "{run}"
        );
    }
}
