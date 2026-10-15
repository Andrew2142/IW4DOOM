// SPDX-License-Identifier: GPL-3.0-or-later
//! [`PreparedViewmodel`]: a weapon's mesh with quantized textures, ready to draw every frame
//! (sheet 11 section 5.3).

use dmw_format::{AlphaMode, CullMode, PartRole, ViewmodelAsset};
use glam::Vec3;

use crate::quantize::{Dither, IndexedTexture, quantize};
use crate::{PaletteTables, RasterError};

const LOG: &str = "dmw::raster";

/// Which triangle winding is front-facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winding {
    /// Counter-clockwise seen from the front (the fixture's convention).
    CounterClockwise,
    /// Clockwise seen from the front.
    Clockwise,
}

/// Bone influences of one vertex with zero weights dropped.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SkinRef {
    pub bones: [u16; 4],
    pub weights: [f32; 4],
    /// Number of used entries, 1..=4.
    pub count: u8,
}

/// One drawable surface.
#[derive(Debug, Clone)]
pub(crate) struct PreparedSurface {
    pub first_tri: u32,
    pub tri_count: u32,
    /// Index into `PreparedViewmodel::textures`; `None` = flat colour.
    pub texture: Option<u32>,
    /// Palette index used when `texture` is `None`: `nearest(128, 128, 128)`.
    pub flat_index: u8,
    pub cull_back: bool,
    pub role: PartRole,
}

/// A viewmodel prepared for [`crate::Rasterizer::draw`]: quantized textures, compact skinning
/// data, detected winding and per-triangle texel areas.
#[derive(Debug, Clone)]
pub struct PreparedViewmodel {
    pub(crate) positions: Vec<[f32; 3]>,
    pub(crate) normals: Vec<[f32; 3]>,
    pub(crate) uvs: Vec<[f32; 2]>,
    pub(crate) skin: Vec<SkinRef>,
    pub(crate) indices: Vec<u32>,
    pub(crate) surfaces: Vec<PreparedSurface>,
    pub(crate) textures: Vec<IndexedTexture>,
    /// Per triangle: area in mip-0 texels² (0 for untextured surfaces).
    pub(crate) tri_texel_area: Vec<f32>,
    pub(crate) winding: Winding,
}

fn alpha_key(a: AlphaMode) -> (u8, u32) {
    match a {
        AlphaMode::Opaque => (0, 0),
        AlphaMode::Test { cutoff } => (1, cutoff.to_bits()),
        AlphaMode::Blend => (2, 0),
    }
}

impl PreparedViewmodel {
    /// Quantizes every texture referenced by a surface (with that surface's alpha mode),
    /// copies the mesh arrays into the draw layout and detects the winding.
    ///
    /// Fails only on structural mesh errors. A missing or empty texture makes its surface flat
    /// grey (`warn!`).
    pub fn new(
        asset: &ViewmodelAsset,
        pal: &PaletteTables,
        dither: Dither,
    ) -> Result<Self, RasterError> {
        let mesh = &asset.mesh;
        let nv = mesh.positions.len();
        if mesh.normals.len() != nv || mesh.uvs.len() != nv || mesh.skin.len() != nv {
            return Err(RasterError::Mesh(format!(
                "array lengths differ: positions {nv}, normals {}, uvs {}, skin {}",
                mesh.normals.len(),
                mesh.uvs.len(),
                mesh.skin.len()
            )));
        }
        if !mesh.indices.len().is_multiple_of(3) {
            return Err(RasterError::Mesh(format!(
                "index count {} is not a multiple of 3",
                mesh.indices.len()
            )));
        }
        if let Some(bad) = mesh.indices.iter().find(|&&i| i as usize >= nv) {
            return Err(RasterError::Mesh(format!(
                "index {bad} out of range for {nv} vertices"
            )));
        }

        let flat_index = pal.nearest([128, 128, 128]);
        let mut textures: Vec<IndexedTexture> = Vec::new();
        let mut tex_keys: Vec<(u16, (u8, u32))> = Vec::new();
        let mut surfaces = Vec::with_capacity(mesh.surfaces.len());
        let mut tri_texel_area = vec![0.0f32; mesh.indices.len() / 3];

        for s in &mesh.surfaces {
            let end = u64::from(s.index_start) + u64::from(s.index_count);
            if !s.index_start.is_multiple_of(3)
                || !s.index_count.is_multiple_of(3)
                || end > mesh.indices.len() as u64
            {
                return Err(RasterError::Mesh(format!(
                    "surface '{}' range {}+{} is not a triangle range inside {} indices",
                    s.name,
                    s.index_start,
                    s.index_count,
                    mesh.indices.len()
                )));
            }
            let alpha = s.alpha;
            let texture = match s.texture {
                None => None,
                Some(t) => match asset.textures.get(usize::from(t)) {
                    Some(img)
                        if img.width > 0
                            && img.height > 0
                            && img.pixels.len() == img.width as usize * img.height as usize =>
                    {
                        let key = (t, alpha_key(alpha));
                        let slot = match tex_keys.iter().position(|k| *k == key) {
                            Some(p) => p,
                            None => {
                                textures.push(quantize(img, pal, dither, alpha));
                                tex_keys.push(key);
                                textures.len() - 1
                            }
                        };
                        Some(slot as u32)
                    }
                    _ => {
                        log::warn!(
                            target: LOG,
                            "surface '{}': texture {t} missing or empty, drawing flat grey",
                            s.name
                        );
                        None
                    }
                },
            };
            let first_tri = s.index_start / 3;
            let tri_count = s.index_count / 3;
            if let Some(slot) = texture {
                let m0 = &textures[slot as usize].mips[0];
                let texels = m0.width as f32 * m0.height as f32;
                for t in first_tri..first_tri + tri_count {
                    let i = t as usize * 3;
                    let uv = |k: usize| mesh.uvs[mesh.indices[i + k] as usize];
                    let (a, b, c) = (uv(0), uv(1), uv(2));
                    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
                    tri_texel_area[t as usize] = 0.5 * cross.abs() * texels;
                }
            }
            surfaces.push(PreparedSurface {
                first_tri,
                tri_count,
                texture,
                flat_index,
                cull_back: s.cull == CullMode::Back,
                role: s.role,
            });
        }

        let skin = mesh
            .skin
            .iter()
            .map(|vs| {
                let mut out = SkinRef {
                    bones: [0; 4],
                    weights: [0.0; 4],
                    count: 0,
                };
                for k in 0..4 {
                    if vs.weights[k] > 0.0 {
                        let n = usize::from(out.count);
                        out.bones[n] = vs.bones[k];
                        out.weights[n] = vs.weights[k];
                        out.count += 1;
                    }
                }
                if out.count == 0 {
                    out.bones[0] = vs.bones[0];
                    out.weights[0] = 1.0;
                    out.count = 1;
                }
                out
            })
            .collect();

        let mut prepared = Self {
            positions: mesh.positions.clone(),
            normals: mesh.normals.clone(),
            uvs: mesh.uvs.clone(),
            skin,
            indices: mesh.indices.clone(),
            surfaces,
            textures,
            tri_texel_area,
            winding: Winding::CounterClockwise,
        };
        prepared.winding = prepared.detect_winding();
        Ok(prepared)
    }

    /// The front-face winding detected at preparation.
    pub fn winding(&self) -> Winding {
        self.winding
    }

    /// Total triangle count of the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Quantized textures, in first-use order (one per texture/alpha-mode pair).
    pub fn textures(&self) -> &[IndexedTexture] {
        &self.textures
    }

    /// Winding detection over the `CullMode::Back` surfaces at bind pose (identity skin, eye
    /// at the origin looking +x).
    ///
    /// Primary vote: does the geometric normal of a CCW reading agree with the stored vertex
    /// normals? This is decisive for closed meshes, where the camera-facing count of sheet 11
    /// section 5.3 is close to a tie (see sheet 11 section 11). When the normals do not decide
    /// (fewer than 60 % agree either way), the camera-facing majority is used; a tie there
    /// keeps counter-clockwise.
    fn detect_winding(&self) -> Winding {
        let (mut n_ccw, mut n_cw, mut f_ccw, mut f_cw) = (0u32, 0u32, 0u32, 0u32);
        for s in self.surfaces.iter().filter(|s| s.cull_back) {
            for t in s.first_tri..s.first_tri + s.tri_count {
                let i = t as usize * 3;
                let v = |k: usize| self.indices[i + k] as usize;
                let p = |k: usize| Vec3::from(self.positions[v(k)]);
                let geo = (p(1) - p(0)).cross(p(2) - p(0));
                if !geo.is_finite() || geo.length_squared() < 1e-12 {
                    continue;
                }
                let n = Vec3::from(self.normals[v(0)])
                    + Vec3::from(self.normals[v(1)])
                    + Vec3::from(self.normals[v(2)]);
                let d = geo.dot(n);
                if d > 0.0 {
                    n_ccw += 1;
                } else if d < 0.0 {
                    n_cw += 1;
                }
                // Facing the eye at the origin under a CCW reading: normal points at the eye.
                let f = geo.dot(-p(0));
                if f > 0.0 {
                    f_ccw += 1;
                } else if f < 0.0 {
                    f_cw += 1;
                }
            }
        }
        let total_n = n_ccw + n_cw;
        let (winding, how) = if total_n > 0 && n_ccw * 10 >= total_n * 6 {
            (Winding::CounterClockwise, "normals")
        } else if total_n > 0 && n_cw * 10 >= total_n * 6 {
            (Winding::Clockwise, "normals")
        } else if f_cw > f_ccw {
            (Winding::Clockwise, "camera-facing majority")
        } else {
            (Winding::CounterClockwise, "camera-facing majority")
        };
        log::debug!(
            target: LOG,
            "winding {winding:?} by {how} (normals ccw {n_ccw} / cw {n_cw}, facing ccw {f_ccw} / cw {f_cw})"
        );
        winding
    }
}
