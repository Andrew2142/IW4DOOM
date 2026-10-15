// SPDX-License-Identifier: GPL-3.0-or-later
//! `viewmodel.bin`: skeleton, mesh and clips of the composed first-person assembly
//! (sheet 03 section 5.5).
//!
//! File layout: the 4 bytes `DMWV`, a `u32` little-endian version (`1`), then the postcard
//! encoding of [`ViewmodelFile`].
//!
//! Positions are in MW2 model space (x forward, y left, z up, inches) exactly as iw4L's
//! `ModelSkel::positions`. Triangle winding is whatever MW2 authored; the exporter records it
//! and the fixture uses counter-clockwise when viewed from outside. Sheet 11 detects the
//! convention once in `PreparedViewmodel::new`.

use serde::{Deserialize, Serialize};

use crate::anim::ClipSet;
use crate::error::PackError;

/// First four bytes of `viewmodel.bin`.
pub const VIEWMODEL_MAGIC: [u8; 4] = *b"DMWV";
/// Version number following the magic (independent of the manifest version).
pub const VIEWMODEL_FILE_VERSION: u32 = 1;

/// Everything stored in one weapon's `viewmodel.bin`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ViewmodelFile {
    /// Bones of the composed assembly.
    pub skeleton: Skeleton,
    /// Skinned triangle mesh.
    pub mesh: ViewmodelMesh,
    /// Animation clips and the weapon-anim slot table.
    pub clips: ClipSet,
}

impl ViewmodelFile {
    /// Encodes with the 8-byte header.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PackError> {
        let body = postcard::to_allocvec(self)
            .map_err(|e| PackError::format(format!("viewmodel encode: {e}")))?;
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(&VIEWMODEL_MAGIC);
        out.extend_from_slice(&VIEWMODEL_FILE_VERSION.to_le_bytes());
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Decodes bytes produced by [`ViewmodelFile::to_bytes`]. `label` names the file in errors.
    pub fn from_bytes(bytes: &[u8], label: &str) -> Result<Self, PackError> {
        let Some((header, body)) = bytes.split_first_chunk::<8>() else {
            return Err(PackError::format(format!(
                "{label}: shorter than the 8-byte header"
            )));
        };
        if header[..4] != VIEWMODEL_MAGIC {
            return Err(PackError::format(format!(
                "{label}: bad magic, expected DMWV"
            )));
        }
        let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if version != VIEWMODEL_FILE_VERSION {
            return Err(PackError::format(format!(
                "{label}: viewmodel file version {version}, expected {VIEWMODEL_FILE_VERSION}"
            )));
        }
        postcard::from_bytes(body).map_err(|e| PackError::format(format!("{label}: {e}")))
    }
}

/// Mirror of `xmodel_runtime::DObj` for the composed FPV assembly (hands + gun + attachments),
/// plus the assembly's special bones.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Skeleton {
    /// At most 192 bones (`anim_iw4::PartBits::CAPACITY`); a parent always precedes its child.
    pub bones: Vec<BoneData>,
    /// Model slots the bones belong to.
    pub models: Vec<ModelSlotData>,
    /// `DObj::duplicates`: pairs of bone indices that share a name.
    pub duplicates: Vec<(u16, u16)>,
    /// `FpvAssembly::view_bone()`, `tag_view`.
    pub view_bone: u16,
    /// `FpvAssembly::camera_bone()`.
    pub camera_bone: Option<u16>,
    /// `tag_flash` (`FpvAssembly::tags()`).
    pub flash_bone: Option<u16>,
    /// `tag_brass`.
    pub brass_bone: Option<u16>,
    /// `FpvAssembly::collapsed_bones()`.
    pub collapsed_bones: Vec<u16>,
}

/// One bone (mirror of `xmodel_runtime::Bone`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BoneData {
    /// Bone name, for example `j_gun`.
    pub name: String,
    /// Index into [`Skeleton::models`].
    pub model: u16,
    /// Parent bone, always less than this bone's own index.
    pub parent: Option<u16>,
    /// x, y, z, w.
    pub bind_rotation: [f32; 4],
    /// Local bind translation.
    pub bind_translation: [f32; 3],
    /// Column-major `Mat4`.
    pub bind_world: [f32; 16],
    /// `Bone::no_scale`.
    pub no_scale: bool,
}

/// One model slot (mirror of `xmodel_runtime::ModelSlot`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ModelSlotData {
    /// Model name.
    pub name: String,
    /// Index of its first bone.
    pub base: u16,
    /// Number of bones.
    pub bone_count: u16,
    /// Model scale.
    pub scale: f32,
}

/// Skinned triangle mesh shared by all surfaces.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ViewmodelMesh {
    /// Bind (model/world) space, inches; `skin = world_pose * inverse(bind_world)`.
    pub positions: Vec<[f32; 3]>,
    /// Unit length.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates; `v` grows downward in the PNG.
    pub uvs: Vec<[f32; 2]>,
    /// Same length as `positions`.
    pub skin: Vec<VertexSkin>,
    /// Triangle list.
    pub indices: Vec<u32>,
    /// Draw ranges into `indices`.
    pub surfaces: Vec<SurfaceData>,
}

/// Up to four bone influences of one vertex.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct VertexSkin {
    /// Global DObj bone indices; unused slots are `0`.
    pub bones: [u16; 4],
    /// Sum to 1 (within 1e-3); unused slots are `0.0`.
    pub weights: [f32; 4],
}

/// A contiguous range of triangles with one material.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SurfaceData {
    /// Material name, for logs.
    pub name: String,
    /// First index (a multiple of 3).
    pub index_start: u32,
    /// Number of indices (a multiple of 3).
    pub index_count: u32,
    /// Index into `textures/<n>.png`; `None` = untextured (flat grey index).
    pub texture: Option<u16>,
    /// Alpha handling.
    pub alpha: AlphaMode,
    /// Face culling.
    pub cull: CullMode,
    /// What part of the assembly the surface belongs to.
    pub role: PartRole,
    /// The `ModelSlotData` index the surface came from.
    pub model: u16,
}

/// Material alpha handling.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum AlphaMode {
    /// No alpha.
    Opaque,
    /// Texels with alpha below `cutoff` are not drawn.
    Test {
        /// Alpha threshold in 0..1.
        cutoff: f32,
    },
    /// Drawn as `Test { cutoff: 0.5 }` by sheet 11.
    Blend,
}

/// Face culling.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CullMode {
    /// Cull back faces.
    Back,
    /// Draw both sides.
    None,
}

/// iw4L `FpvPartRole` plus `ScopeLens`. Rocket surfaces are hidden when the rocket is not
/// loaded and Knife surfaces are shown only during melee (sheet 11 section 5.10); ScopeLens
/// surfaces are skipped by sheet 11.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartRole {
    /// Arms and hands.
    Hands,
    /// The weapon body.
    Gun,
    /// An attachment.
    Attachment,
    /// Loadable rocket.
    Rocket,
    /// Melee knife.
    Knife,
    /// Scope lens glass.
    ScopeLens,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tiny() -> ViewmodelFile {
        ViewmodelFile {
            skeleton: Skeleton {
                bones: vec![BoneData {
                    name: "tag_origin".into(),
                    model: 0,
                    parent: None,
                    bind_rotation: [0.0, 0.0, 0.0, 1.0],
                    bind_translation: [0.0; 3],
                    bind_world: glam::Mat4::IDENTITY.to_cols_array(),
                    no_scale: false,
                }],
                models: vec![ModelSlotData {
                    name: "m".into(),
                    base: 0,
                    bone_count: 1,
                    scale: 1.0,
                }],
                duplicates: vec![],
                view_bone: 0,
                camera_bone: None,
                flash_bone: None,
                brass_bone: None,
                collapsed_bones: vec![],
            },
            mesh: ViewmodelMesh {
                positions: vec![],
                normals: vec![],
                uvs: vec![],
                skin: vec![],
                indices: vec![],
                surfaces: vec![],
            },
            clips: ClipSet {
                clips: vec![],
                slots: vec![None; crate::anim::WEAPON_ANIM_SLOTS],
                track_maps: vec![],
            },
        }
    }

    #[test]
    fn header_round_trip_and_rejections() {
        let file = tiny();
        let bytes = file.to_bytes().unwrap();
        assert_eq!(&bytes[..4], b"DMWV");
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        assert_eq!(ViewmodelFile::from_bytes(&bytes, "t").unwrap(), file);

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            ViewmodelFile::from_bytes(&bad_magic, "t"),
            Err(PackError::Format(_))
        ));
        let mut bad_version = bytes.clone();
        bad_version[4] = 2;
        assert!(matches!(
            ViewmodelFile::from_bytes(&bad_version, "t"),
            Err(PackError::Format(_))
        ));
        assert!(ViewmodelFile::from_bytes(&bytes[..5], "t").is_err());
        assert!(ViewmodelFile::from_bytes(&bytes[..bytes.len() - 3], "t").is_err());
    }
}
