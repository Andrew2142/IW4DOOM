// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 weapon simulation bridge (sheet 07) and viewmodel animation (sheet 10).
//!
//! Drives iw4L's retail weapon and movement logic from Doom's 35 Hz tic. This crate
//! is pure, deterministic safe Rust: it never depends on the engine, the rasterizer or Bevy.
#![forbid(unsafe_code)]

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
