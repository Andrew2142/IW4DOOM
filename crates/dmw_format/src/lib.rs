// SPDX-License-Identifier: GPL-3.0-or-later
//! DOOMMW2 pack format types, shared constants and unit conversions.
//!
//! This crate is the single home of the contract constants in
//! `docs/specs/00-overview-and-contracts.md` section 5, the pack (de)serialization and
//! validation (`docs/specs/03-asset-pack-format.md`), and the synthetic fixture pack. It
//! depends on no engine or simulation code.
//!
//! Layout of a pack on disk (format v1):
//!
//! ```text
//! <pack>/manifest.json
//! <pack>/weapons/<name>/facts.json
//! <pack>/weapons/<name>/viewmodel.bin        (postcard, see model.rs)
//! <pack>/weapons/<name>/textures/<n>.png
//! <pack>/weapons/<name>/overlay.png          (optional)
//! <pack>/sounds/<alias>/<variant>.wav
//! ```
#![forbid(unsafe_code)]

pub mod anim;
pub mod consts;
pub mod convert;
pub mod error;
pub mod facts;
pub mod fixture;
pub mod load;
pub mod manifest;
pub mod model;
pub mod remote;
pub mod sound;
pub mod texture;
pub mod validate;

/// Version of the on-disk pack format this crate reads and writes.
pub const PACK_FORMAT_VERSION: u32 = 1;

pub use consts::{DmwWeaponId, DoomAmmo, DoomWeapon};
pub use error::PackError;
pub use facts::{
    AnimTimes, FpvFacts, HudFacts, KickFacts, MovementScales, ProjectileFacts, SwayFacts,
    WeaponFacts, WeaponSounds,
};
pub use load::{Pack, ViewmodelAsset};
pub use manifest::{PackManifest, PackSource, SoundEntry, SoundVariant, WeaponEntry};
pub use model::{
    AlphaMode, BoneData, CullMode, ModelSlotData, PartRole, Skeleton, SurfaceData, VertexSkin,
    ViewmodelFile, ViewmodelMesh,
};
pub use sound::SoundAsset;
pub use texture::RgbaImage;

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
