// SPDX-License-Identifier: GPL-3.0-or-later
//! DOOMMW2 pack format types, shared constants and unit conversions.
//!
//! This crate is the single home of the contract constants in
//! `docs/specs/00-overview-and-contracts.md` section 5 and, in later sheets, the
//! pack (de)serialization, validation and the synthetic fixture pack. It depends
//! on no engine or simulation code.
#![forbid(unsafe_code)]

pub mod consts;

#[cfg(test)]
mod tests {
    /// Touches one item from every iw4L and third-party dependency so a broken
    /// path dependency fails `cargo test`.
    #[test]
    fn links() {
        let facts = weapon_iw4::WeaponCombatFacts::none();
        assert_eq!(facts.fire_time_ms, 0);
        assert_eq!(glam::Mat4::IDENTITY.x_axis.x.to_bits(), 1.0f32.to_bits());
    }
}
