// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared helpers for the dmw_viewmodel integration tests: the synthetic palette and COLORMAP
//! (sheet 13 section 5.4: 6x6x6 cube + 40 greys, linear darkening), mesh builders, frame
//! capture and goldens.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

#[path = "../../../../room/tests/support/golden.rs"]
#[allow(clippy::print_stderr)]
pub mod golden;

use std::sync::OnceLock;

use dmw_format::anim::{ClipSet, WEAPON_ANIM_SLOTS};
use dmw_format::{
    AlphaMode, BoneData, CullMode, ModelSlotData, PartRole, RgbaImage, Skeleton, SurfaceData,
    VertexSkin, ViewmodelAsset, ViewmodelMesh,
};
use dmw_viewmodel::{
    DrawStats, DrawTarget, LightParams, PaletteTables, Pose, PreparedViewmodel, Rasterizer,
    ViewmodelDrawParams, Visibility,
};
use glam::{Mat3, Vec3};

pub const W: usize = 320;
pub const H: usize = 200;
/// Doom's default view window height (status bar visible).
pub const VIEW_H: i32 = 168;
/// Background index of the view window in captured frames (a mid grey of the synthetic palette).
pub const BG: u8 = 236;
/// Index used to paint the "status bar" rows below the view window.
pub const SBAR: u8 = 5;

/// The synthetic PLAYPAL: index `r*36 + g*6 + b` is the cube colour (levels 0, 51, …, 255);
/// 216..255 are 40 greys `round((k+1)*255/41)` (none coincides with a cube level).
pub fn synthetic_playpal() -> Vec<u8> {
    let mut p = Vec::with_capacity(768);
    for r in 0..6u32 {
        for g in 0..6u32 {
            for b in 0..6u32 {
                p.extend_from_slice(&[(r * 51) as u8, (g * 51) as u8, (b * 51) as u8]);
            }
        }
    }
    for k in 0..40u32 {
        let g = ((f64::from(k + 1) * 255.0 / 41.0).round()) as u8;
        p.extend_from_slice(&[g, g, g]);
    }
    p
}

fn exact_nearest(pal: &[[u8; 3]], rgb: [u8; 3]) -> u8 {
    let mut best = 0;
    let mut best_d = u32::MAX;
    for (i, &c) in pal.iter().enumerate() {
        let d = dmw_viewmodel::colour_distance(rgb, c);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best as u8
}

/// The synthetic COLORMAP: maps 0..31 darken linearly toward black (factor `1 - L/32`), map 32
/// is an inverted-luminance grey (invulnerability), map 33 is all index 0.
pub fn synthetic_colormap() -> Vec<u8> {
    let pal: Vec<[u8; 3]> = synthetic_playpal()
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let mut out = Vec::with_capacity(34 * 256);
    for level in 0..32u32 {
        let f = 1.0 - f64::from(level) / 32.0;
        for c in &pal {
            let s = |v: u8| (f64::from(v) * f).round() as u8;
            out.push(exact_nearest(&pal, [s(c[0]), s(c[1]), s(c[2])]));
        }
    }
    for c in &pal {
        let lum = (0.299 * f64::from(c[0]) + 0.587 * f64::from(c[1]) + 0.114 * f64::from(c[2]))
            .round() as u8;
        let inv = 255 - lum;
        out.push(exact_nearest(&pal, [inv, inv, inv]));
    }
    out.extend(std::iter::repeat_n(0u8, 256));
    out
}

/// Shared tables (the LUT build is the expensive part).
pub fn pal() -> &'static PaletteTables {
    static PAL: OnceLock<PaletteTables> = OnceLock::new();
    PAL.get_or_init(|| {
        PaletteTables::from_wad_lumps(&synthetic_playpal(), &synthetic_colormap()).unwrap()
    })
}

/// A 320x200 frame: view window filled with [`BG`], status-bar rows with [`SBAR`].
pub fn blank_frame() -> Vec<u8> {
    let mut f = vec![BG; W * H];
    f[VIEW_H as usize * W..].fill(SBAR);
    f
}

pub fn target(pixels: &mut [u8], view_h: i32) -> DrawTarget<'_> {
    DrawTarget {
        pixels,
        pitch: W,
        view_x: 0,
        view_y: 0,
        view_w: W as i32,
        view_h,
        center_x: W as f32 / 2.0,
        center_y: view_h as f32 / 2.0,
        pixel_aspect: 5.0 / 6.0,
    }
}

/// Draws `model` with `pose` and `light` into `frame` (320x168 view window).
pub fn render(
    r: &mut Rasterizer,
    frame: &mut [u8],
    model: &PreparedViewmodel,
    pose: &Pose,
    light: LightParams<'_>,
    vis: Visibility,
) -> DrawStats {
    let mut t = target(frame, VIEW_H);
    r.draw(
        &mut t,
        model,
        &mut ViewmodelDrawParams {
            pose,
            hfov_deg: 65.0,
            light,
            pal: pal(),
            visibility: vis,
        },
    )
}

/// RGB bytes of a frame through the synthetic palette.
pub fn to_rgb(frame: &[u8]) -> Vec<u8> {
    let pal = &pal().playpal;
    frame.iter().flat_map(|&i| pal[usize::from(i)]).collect()
}

pub fn golden_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Exact golden comparison of a full frame (`BLESS=1` rewrites).
pub fn assert_frame_golden(name: &str, frame: &[u8]) {
    golden::assert_golden_rgb_in(&golden_dir(), name, W, H, &to_rgb(frame), 0);
}

/// Text golden: `tests/golden/<name>.txt`, `BLESS=1` rewrites.
pub fn assert_text_golden(name: &str, text: &str) {
    let path = golden_dir().join(format!("{name}.txt"));
    if std::env::var("BLESS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, text).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "golden {} missing ({e}); rerun with BLESS=1",
            path.display()
        )
    });
    assert_eq!(text, expected, "text golden {} differs", path.display());
}

/// Skeleton with `n` identity bones.
pub fn flat_skeleton(n: usize) -> Skeleton {
    Skeleton {
        bones: (0..n)
            .map(|i| BoneData {
                name: format!("b{i}"),
                model: 0,
                parent: (i > 0).then_some(0),
                bind_rotation: [0.0, 0.0, 0.0, 1.0],
                bind_translation: [0.0; 3],
                bind_world: glam::Mat4::IDENTITY.to_cols_array(),
                no_scale: false,
            })
            .collect(),
        models: vec![ModelSlotData {
            name: "test".into(),
            base: 0,
            bone_count: n as u16,
            scale: 1.0,
        }],
        duplicates: vec![],
        view_bone: 0,
        camera_bone: None,
        flash_bone: None,
        brass_bone: None,
        collapsed_bones: vec![],
    }
}

pub fn empty_clips() -> ClipSet {
    ClipSet {
        clips: vec![],
        slots: vec![None; WEAPON_ANIM_SLOTS],
        track_maps: vec![],
    }
}

/// Mesh builder for hand-made test geometry; every vertex rigidly skinned to bone 0.
#[derive(Default)]
pub struct MeshBuilder {
    pub mesh: Option<ViewmodelMesh>,
}

impl MeshBuilder {
    fn m(&mut self) -> &mut ViewmodelMesh {
        self.mesh.get_or_insert_with(|| ViewmodelMesh {
            positions: vec![],
            normals: vec![],
            uvs: vec![],
            skin: vec![],
            indices: vec![],
            surfaces: vec![],
        })
    }

    /// Adds one triangle surface; the normal is the CCW geometric normal.
    pub fn triangle(
        &mut self,
        p: [Vec3; 3],
        uv: [[f32; 2]; 3],
        texture: Option<u16>,
        cull: CullMode,
    ) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
        let m = self.m();
        let base = m.positions.len() as u32;
        let start = m.indices.len() as u32;
        for k in 0..3 {
            m.positions.push(p[k].to_array());
            m.normals.push(n.to_array());
            m.uvs.push(uv[k]);
            m.skin.push(VertexSkin {
                bones: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            });
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2]);
        m.surfaces.push(SurfaceData {
            name: format!("tri{}", m.surfaces.len()),
            index_start: start,
            index_count: 3,
            texture,
            alpha: AlphaMode::Opaque,
            cull,
            role: PartRole::Gun,
            model: 0,
        });
    }

    /// Adds a square quad centred at `c` with edge `size`, spanned by `u_axis` (u grows) and
    /// `v_axis` (v grows, i.e. down the PNG). The front face is `v_axis x u_axis`, so the quad
    /// faces the eye when `u_axis = -y` (screen right) and `v_axis = -z` (screen down).
    pub fn quad(&mut self, c: Vec3, size: f32, u_axis: Vec3, v_axis: Vec3, texture: Option<u16>) {
        let at = |u: f32, v: f32| c + (u - 0.5) * size * u_axis + (v - 0.5) * size * v_axis;
        let (a, b, cc, d) = (at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0));
        let n = (d - a).cross(cc - a).normalize();
        let m = self.m();
        let base = m.positions.len() as u32;
        let start = m.indices.len() as u32;
        for (p, uv) in [
            (a, [0.0, 0.0]),
            (b, [1.0, 0.0]),
            (cc, [1.0, 1.0]),
            (d, [0.0, 1.0]),
        ] {
            m.positions.push(p.to_array());
            m.normals.push(n.to_array());
            m.uvs.push(uv);
            m.skin.push(VertexSkin {
                bones: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            });
        }
        // (a, d, c) and (a, c, b) wind counter-clockwise around n.
        m.indices
            .extend_from_slice(&[base, base + 3, base + 2, base, base + 2, base + 1]);
        m.surfaces.push(SurfaceData {
            name: format!("quad{}", m.surfaces.len()),
            index_start: start,
            index_count: 6,
            texture,
            alpha: AlphaMode::Opaque,
            cull: CullMode::Back,
            role: PartRole::Gun,
            model: 0,
        });
    }

    pub fn asset(&mut self, textures: Vec<RgbaImage>) -> ViewmodelAsset {
        ViewmodelAsset {
            mesh: self.m().clone(),
            skeleton: flat_skeleton(1),
            clips: empty_clips(),
            textures,
        }
    }
}

/// Rotation about the eye-space z axis.
pub fn rot_z(deg: f32) -> Mat3 {
    Mat3::from_rotation_z(deg.to_radians())
}

/// The fixture viewmodel of weapon `index` (mesh, skeleton, textures; clips left empty — the
/// rasterizer does not read them).
pub fn fixture_asset(index: usize) -> ViewmodelAsset {
    ViewmodelAsset {
        mesh: dmw_format::fixture::geometry::mesh(),
        skeleton: dmw_format::fixture::geometry::skeleton(),
        clips: empty_clips(),
        textures: dmw_format::fixture::media::textures(index),
    }
}

/// Identity skin matrices for every fixture bone (bind pose).
pub fn fixture_bind_pose() -> Pose {
    Pose {
        skin: vec![glam::Mat4::IDENTITY; 8],
        tag_flash: None,
    }
}
