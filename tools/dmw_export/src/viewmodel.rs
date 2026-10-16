// SPDX-License-Identifier: GPL-3.0-or-later
//! FPV assembly → `Skeleton` + `ViewmodelMesh` (sheet 04 §5.5).
//!
//! Vertex positions stay in each part's own model space: the DObj's `bind_world` for a part's
//! bones is that model's own base matrix (xmodel_runtime `DObj::build`), so
//! `world * inverse(bind_world)` (sheet 10) maps them correctly without any baking here.

use asset_game::{FpvAssembly, FpvPartRole, FpvSideAssemblies};
use asset_material::{MaterialCullFace, MaterialDefinitions, MaterialDrawMode};
use asset_model::FpvMeshCatalog;
use dmw_format::{
    AlphaMode, BoneData, CullMode, ModelSlotData, PartRole, Skeleton, SurfaceData, VertexSkin,
    ViewmodelMesh,
};

use crate::textures::TextureSet;

/// `dpvs_iw4::skin_blend_weight`: raw 16-bit extra weight → fraction (scale 2^-16).
pub fn blend_weight(raw: u16) -> f32 {
    dpvs_iw4::skin_blend_weight(raw)
}

/// What the exporter reads from one `asset_model::VertSkin` (bone ids already global).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawSkin {
    pub bones: [u16; 4],
    /// Raw extra weights for influences 1..=3.
    pub extra_raw: [u16; 3],
    /// `has_extra_weight(k)` for k = 1..=3.
    pub has_extra: [bool; 3],
    /// `primary_is_zero()`.
    pub primary_zero: bool,
    /// `has_only_primary_weight()`: rigid vertex.
    pub only_primary: bool,
}

/// Converts a skin to four normalised weights (§5.5 step 3). Returns the skin and `true` when the
/// raw weights summed to something more than 1e-2 away from 1 (renormalised and counted).
///
/// iw4L skins a blended vertex as `(1 - Σ w_k) * primary + Σ w_k * extra_k`
/// (dpvs_iw4 `skin_packed_mad_extras`); the primary weight is 0 when the vertex was captured
/// with a zero primary weight.
pub fn skin_weights(s: &RawSkin) -> (VertexSkin, bool) {
    if s.only_primary {
        return (
            VertexSkin {
                bones: [s.bones[0], 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            false,
        );
    }
    let mut w = [0.0f32; 4];
    let mut bones = [s.bones[0], 0, 0, 0];
    let mut extra_sum = 0.0f32;
    for k in 0..3 {
        if s.has_extra[k] {
            w[k + 1] = blend_weight(s.extra_raw[k]);
            bones[k + 1] = s.bones[k + 1];
            extra_sum += w[k + 1];
        }
    }
    w[0] = if s.primary_zero {
        0.0
    } else {
        (1.0 - extra_sum).max(0.0)
    };
    let sum: f32 = w.iter().sum();
    if sum <= 1e-6 || !sum.is_finite() {
        return (
            VertexSkin {
                bones: [s.bones[0], 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            true,
        );
    }
    let deviated = (sum - 1.0).abs() > 1e-2;
    for (k, wk) in w.iter_mut().enumerate() {
        *wk /= sum;
        if *wk == 0.0 {
            bones[k] = if k == 0 { bones[0] } else { 0 };
        }
    }
    (VertexSkin { bones, weights: w }, deviated)
}

/// Which assembly of `FpvSideAssemblies` was flattened.
pub fn choose_assembly(sides: &FpvSideAssemblies) -> (&'static str, &FpvAssembly) {
    if let Some(a) = &sides.rocket {
        ("rocket", a)
    } else if let Some(a) = &sides.melee {
        ("melee", a)
    } else {
        ("bare", &sides.bare)
    }
}

/// Mirror of the composed DObj plus the assembly's special bones.
pub fn skeleton(assembly: &FpvAssembly) -> Result<(Skeleton, Vec<String>), String> {
    let dobj = assembly.dobj();
    let to16 = |v: usize, what: &str| u16::try_from(v).map_err(|_| format!("{what} {v} > u16"));
    let mut warnings = Vec::new();
    let mut bones = Vec::with_capacity(dobj.bones.len());
    for (i, b) in dobj.bones.iter().enumerate() {
        let mut q = b.bind_rotation.to_array();
        let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
        if (len - 1.0).abs() > 1e-4 && len > 0.0 {
            warnings.push(format!(
                "bone {i} '{}' bind rotation renormalised (|q| = {len})",
                b.name
            ));
            for c in &mut q {
                *c /= len;
            }
        }
        bones.push(BoneData {
            name: b.name.clone(),
            model: to16(b.model, "bone model")?,
            parent: b.parent.map(|p| to16(p, "bone parent")).transpose()?,
            bind_rotation: q,
            bind_translation: b.bind_translation.to_array(),
            bind_world: b.bind_world.to_cols_array(),
            no_scale: b.no_scale,
        });
    }
    let models = dobj
        .models
        .iter()
        .map(|m| {
            Ok(ModelSlotData {
                name: m.name.clone(),
                base: to16(m.base, "model base")?,
                bone_count: to16(m.bone_count, "model bone count")?,
                scale: m.scale,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let duplicates = dobj
        .duplicates
        .iter()
        .map(|&(d, s)| Ok((to16(d, "duplicate")?, to16(s, "duplicate")?)))
        .collect::<Result<Vec<_>, String>>()?;
    let tags = assembly.tags();
    Ok((
        Skeleton {
            bones,
            models,
            duplicates,
            view_bone: to16(assembly.view_bone(), "view bone")?,
            camera_bone: assembly
                .camera_bone()
                .map(|b| to16(b, "camera bone"))
                .transpose()?,
            flash_bone: tags.flash,
            brass_bone: tags.brass,
            collapsed_bones: assembly
                .collapsed_bones()
                .iter()
                .map(|&b| to16(b, "collapsed bone"))
                .collect::<Result<Vec<_>, String>>()?,
        },
        warnings,
    ))
}

/// Counters for the report.
#[derive(Debug, Default, Clone, Copy)]
pub struct SurfaceCounts {
    pub exported: u32,
    pub hidden: u32,
    pub untextured: u32,
    pub no_material: u32,
    pub weight_fixes: u32,
}

fn role(r: FpvPartRole) -> PartRole {
    match r {
        FpvPartRole::Hands => PartRole::Hands,
        FpvPartRole::Gun => PartRole::Gun,
        FpvPartRole::Attachment => PartRole::Attachment,
        FpvPartRole::Rocket => PartRole::Rocket,
        FpvPartRole::Knife => PartRole::Knife,
    }
}

/// iw4L `render_anim/anim/fpv_rig.rs` `leftover_scope_surf_is_lens`.
pub fn scope_surf_is_lens(material_name: &str) -> bool {
    material_name.contains("lens")
}

fn alpha_mode(defs: &MaterialDefinitions, m: &asset_material::AuthoredMaterial) -> AlphaMode {
    match defs.agreed_draw_mode(m) {
        Some(MaterialDrawMode::Opaque) => AlphaMode::Opaque,
        Some(MaterialDrawMode::AlphaTest { .. }) => AlphaMode::Test {
            cutoff: defs.agreed_alpha_test_cutoff(m).flatten().unwrap_or(0.5),
        },
        Some(_) => AlphaMode::Blend,
        None => match defs.agreed_alpha_test_cutoff(m) {
            Some(Some(cutoff)) => AlphaMode::Test { cutoff },
            _ => AlphaMode::Opaque,
        },
    }
}

/// Model-local surface hiding (iw4L `fpv_rig.rs` uses `surface_hidden(bits, words, 0)`: a part's
/// hide words are indexed by the model's own bones).
fn surface_hidden(part_bits: &[u32; 6], hide: &[u32; 6]) -> bool {
    part_bits.iter().zip(hide).any(|(a, b)| a & b != 0)
}

/// Flattens every LOD-0 surface of every part into one skinned mesh.
pub fn flatten(
    assembly: &FpvAssembly,
    catalog: &FpvMeshCatalog,
    defs: &MaterialDefinitions,
    textures: &mut TextureSet,
    warnings: &mut Vec<String>,
) -> Result<(ViewmodelMesh, SurfaceCounts), String> {
    let mut mesh = ViewmodelMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        skin: Vec::new(),
        indices: Vec::new(),
        surfaces: Vec::new(),
    };
    let mut counts = SurfaceCounts::default();
    let bone_count = assembly.dobj().bones.len();
    for (slot, part) in assembly.parts().iter().enumerate() {
        let entry = catalog
            .get_at(part.model.order())
            .ok_or_else(|| format!("part {slot}: model missing from the FPV catalog"))?;
        let skel = &entry.skel;
        if skel.packed_vertices.len() != skel.positions.len()
            || skel.vert_skin.len() != skel.positions.len()
        {
            return Err(format!(
                "model '{}': {} positions, {} packed vertices, {} skins",
                skel.name,
                skel.positions.len(),
                skel.packed_vertices.len(),
                skel.vert_skin.len()
            ));
        }
        let normals: Vec<[f32; 3]> = skel.normals().collect();
        let uvs: Vec<[f32; 2]> = skel.uvs().collect();
        let model_slot = u16::try_from(slot).map_err(|_| "too many parts".to_owned())?;
        for s in skel.surfaces_for_lod(0) {
            if let (Some(words), Some(bits)) = (part.hide.as_ref(), skel.surface_part_bits.get(s))
                && surface_hidden(bits, words)
            {
                counts.hidden += 1;
                continue;
            }
            let (first, vcount) = *skel
                .surface_vertex_ranges
                .get(s)
                .ok_or_else(|| format!("model '{}' surface {s}: no vertex range", skel.name))?;
            let (istart, icount) = *skel
                .surface_index_ranges
                .get(s)
                .ok_or_else(|| format!("model '{}' surface {s}: no index range", skel.name))?;
            if vcount == 0 || icount == 0 {
                continue;
            }
            let material_name = entry
                .material_keys
                .get(s)
                .and_then(|k| k.as_ref())
                .map(|k| k.name.clone())
                .or_else(|| entry.material_present_name(s).map(str::to_owned))
                .unwrap_or_default();
            let Some(row) = entry.material_edges.get(s).and_then(|e| e.bound_index()) else {
                // iw4L draws only surfaces whose material is bound (fpv_material.rs admit_surface).
                counts.no_material += 1;
                warnings.push(format!(
                    "model '{}' surface {s} ('{material_name}'): material not bound, skipped",
                    skel.name
                ));
                continue;
            };
            let material = defs
                .materials
                .get(row)
                .ok_or_else(|| format!("material row {row} outside the catalog"))?;
            let texture = textures.for_material(defs, row);
            if texture.is_none() {
                counts.untextured += 1;
            }
            let cull = match defs.cull_face(material) {
                Some(MaterialCullFace::Back) => CullMode::Back,
                Some(MaterialCullFace::None) | None => CullMode::None,
                Some(MaterialCullFace::Front) => {
                    warnings.push(format!(
                        "material '{}': front-face culling exported as none",
                        material.name.as_str()
                    ));
                    CullMode::None
                }
            };
            let base = u32::try_from(mesh.positions.len()).map_err(|_| "mesh too large")?;
            for v in first..first + vcount {
                mesh.positions.push(skel.positions[v]);
                mesh.normals.push(normals[v]);
                mesh.uvs.push(uvs[v]);
                let vs = &skel.vert_skin[v];
                let global = |local: u8| -> Result<u16, String> {
                    let g = part.bone_base + usize::from(local);
                    if g >= bone_count {
                        return Err(format!(
                            "model '{}' vertex {v}: bone {g} >= {bone_count}",
                            skel.name
                        ));
                    }
                    u16::try_from(g).map_err(|_| "bone index > u16".to_owned())
                };
                let raw = RawSkin {
                    bones: [
                        global(vs.bones[0])?,
                        if vs.has_extra_weight(1) {
                            global(vs.bones[1])?
                        } else {
                            0
                        },
                        if vs.has_extra_weight(2) {
                            global(vs.bones[2])?
                        } else {
                            0
                        },
                        if vs.has_extra_weight(3) {
                            global(vs.bones[3])?
                        } else {
                            0
                        },
                    ],
                    extra_raw: [
                        if vs.has_extra_weight(1) {
                            vs.extra_weight(1)
                        } else {
                            0
                        },
                        if vs.has_extra_weight(2) {
                            vs.extra_weight(2)
                        } else {
                            0
                        },
                        if vs.has_extra_weight(3) {
                            vs.extra_weight(3)
                        } else {
                            0
                        },
                    ],
                    has_extra: [
                        vs.has_extra_weight(1),
                        vs.has_extra_weight(2),
                        vs.has_extra_weight(3),
                    ],
                    primary_zero: vs.primary_is_zero(),
                    only_primary: vs.has_only_primary_weight(),
                };
                let (skin, fixed) = skin_weights(&raw);
                if fixed {
                    counts.weight_fixes += 1;
                }
                mesh.skin.push(skin);
            }
            let index_start = u32::try_from(mesh.indices.len()).map_err(|_| "mesh too large")?;
            for &g in &skel.indices[istart..istart + icount] {
                let local = (g as usize)
                    .checked_sub(first)
                    .filter(|l| *l < vcount)
                    .ok_or_else(|| {
                        format!(
                            "model '{}' surface {s}: index {g} outside its vertices",
                            skel.name
                        )
                    })?;
                mesh.indices.push(base + local as u32);
            }
            let r = if scope_surf_is_lens(&material_name) {
                PartRole::ScopeLens
            } else {
                role(part.role)
            };
            mesh.surfaces.push(SurfaceData {
                name: material.name.as_str().to_owned(),
                index_start,
                index_count: u32::try_from(icount).map_err(|_| "surface too large")?,
                texture,
                alpha: alpha_mode(defs, material),
                cull,
                role: r,
                model: model_slot,
            });
            counts.exported += 1;
        }
    }
    if counts.weight_fixes > 0 {
        warnings.push(format!(
            "{} vertices had skin weights summing away from 1 (renormalised)",
            counts.weight_fixes
        ));
    }
    Ok((mesh, counts))
}
