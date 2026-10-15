// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 11 section 9 tests for the crate half: T11.1–T11.7 and the fixture-mesh goldens that
//! stand in for T11.8 until sheet 10 (`dmw_weapons` animation) exists.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

mod common;

use common::*;
use dmw_format::{AlphaMode, CullMode, PartRole, RgbaImage};
use dmw_viewmodel::{
    Dither, LightParams, Pose, PreparedViewmodel, Rasterizer, ViewmodelDrawParams, Visibility,
    Winding, quantize,
};
use glam::{Mat4, Vec3};

fn xorshift(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn identity_pose() -> Pose {
    Pose {
        skin: vec![Mat4::IDENTITY],
        tag_flash: None,
    }
}

/// Reorders a triangle so its counter-clockwise geometric normal points at the eye.
fn facing(p: [Vec3; 3]) -> [Vec3; 3] {
    let n = (p[1] - p[0]).cross(p[2] - p[0]);
    if n.dot(p[0]) < 0.0 {
        p
    } else {
        [p[0], p[2], p[1]]
    }
}

const ALL: Visibility = Visibility {
    rocket: true,
    knife: true,
};

// ---------------------------------------------------------------- T11.1

#[test]
fn t11_1_lut_matches_exact_search() {
    let pal = pal();
    let mut s = 0x1234_5678u32;
    let (mut exact, mut ties, mut near) = (0u32, 0u32, 0u32);
    let mut worst_excess = 0u32;
    for _ in 0..10_000 {
        let v = xorshift(&mut s);
        let rgb = [v as u8, (v >> 8) as u8, (v >> 16) as u8];
        let a = pal.nearest(rgb);
        let b = pal.nearest_fast(rgb);
        if a == b {
            exact += 1;
            continue;
        }
        let da = dmw_viewmodel::colour_distance(rgb, pal.playpal[usize::from(a)]);
        let db = dmw_viewmodel::colour_distance(rgb, pal.playpal[usize::from(b)]);
        if da == db {
            ties += 1;
        } else {
            near += 1;
            worst_excess = worst_excess.max(db - da);
        }
    }
    // Report (visible with --nocapture).
    #[allow(clippy::print_stderr)]
    {
        eprintln!(
            "T11.1: exact {exact}/10000, equal-distance ties {ties}, other {near}, worst excess {worst_excess}"
        );
    }
    assert!(exact >= 9_900, "only {exact} of 10000 exact");
    // A LUT miss is a colour inside a 4x4x4 cell whose centre maps elsewhere: the LUT answer is
    // never worse than the exact one by more than one cell diagonal (redmean-scaled).
    let cell_bound = dmw_viewmodel::colour_distance([0, 0, 0], [4, 4, 4]) * 64;
    assert!(
        worst_excess <= cell_bound,
        "excess {worst_excess} > {cell_bound}"
    );
}

/// `nearest_fast` (LUT plus the exact check of ambiguous cells) agrees with the exact search on
/// a dense lattice of colours (every 3rd value per channel in release, every 15th in debug).
#[test]
fn lut_exact_on_dense_lattice() {
    let pal = pal();
    let step = if cfg!(debug_assertions) { 15 } else { 3 };
    let mut checked = 0u32;
    for r in (0..=255u32).step_by(step) {
        for g in (0..=255u32).step_by(step) {
            for b in (0..=255u32).step_by(step) {
                let rgb = [r as u8, g as u8, b as u8];
                assert_eq!(pal.nearest_fast(rgb), pal.nearest(rgb), "{rgb:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 4000);
}

// ---------------------------------------------------------------- T11.2

fn format_indices(t: &dmw_viewmodel::IndexedTexture) -> String {
    let m = &t.mips[0];
    let mut s = String::new();
    for y in 0..m.height {
        let row: Vec<String> = (0..m.width)
            .map(|x| format!("{:3}", m.texel(x, y).0))
            .collect();
        s.push_str(&row.join(" "));
        s.push('\n');
    }
    s
}

#[test]
fn t11_2_quantize_gradient_and_mips() {
    let mut img = RgbaImage::new(4, 4, [0; 4]);
    for y in 0..4u32 {
        for x in 0..4u32 {
            img.set(x, y, [(x * 60 + 20) as u8, (y * 60 + 20) as u8, 100, 255]);
        }
    }
    let none = quantize(&img, pal(), Dither::None, AlphaMode::Opaque);
    let bayer = quantize(&img, pal(), Dither::Bayer4, AlphaMode::Opaque);
    let text = format!(
        "# 4x4 gradient r = 20 + 60x, g = 20 + 60y, b = 100\n# Dither::None\n{}# Dither::Bayer4\n{}",
        format_indices(&none),
        format_indices(&bayer)
    );
    assert_text_golden("quantize_gradient_4x4", &text);
    assert_ne!(
        none.mips[0].index, bayer.mips[0].index,
        "dither changed nothing"
    );
    assert_eq!(none.mips.len(), 3); // 4x4, 2x2, 1x1

    let big = RgbaImage::new(1024, 512, [90, 120, 30, 255]);
    let t = quantize(&big, pal(), Dither::Bayer4, AlphaMode::Opaque);
    let sizes: Vec<(u32, u32)> = t.mips.iter().map(|m| (m.width, m.height)).collect();
    assert_eq!(
        sizes,
        vec![
            (1024, 512),
            (512, 256),
            (256, 128),
            (128, 64),
            (64, 32),
            (32, 16),
            (16, 8),
            (8, 4),
            (4, 2),
            (2, 1),
            (1, 1)
        ]
    );
    for m in &t.mips {
        assert_eq!(m.index.len(), (m.width * m.height) as usize);
        assert_eq!(m.opaque.len(), m.index.len());
    }

    // Non-power-of-two: floor division, clamped last column.
    let odd = RgbaImage::new(5, 3, [255, 255, 255, 255]);
    let t = quantize(&odd, pal(), Dither::None, AlphaMode::Opaque);
    let sizes: Vec<(u32, u32)> = t.mips.iter().map(|m| (m.width, m.height)).collect();
    assert_eq!(sizes, vec![(5, 3), (2, 1), (1, 1)]);

    // Alpha test.
    let mut holes = RgbaImage::new(2, 1, [200, 0, 0, 255]);
    holes.set(1, 0, [200, 0, 0, 100]);
    let t = quantize(&holes, pal(), Dither::None, AlphaMode::Test { cutoff: 0.5 });
    assert_eq!(t.mips[0].opaque, vec![1, 0]);
}

// ---------------------------------------------------------------- T11.3

/// 8x8 checker of two cube colours with a red texel at u = 1, v = 0 (top-right).
fn checker8() -> RgbaImage {
    let mut img = RgbaImage::new(8, 8, [0; 4]);
    for y in 0..8 {
        for x in 0..8 {
            let c = if (x + y) % 2 == 0 {
                [255, 255, 255, 255]
            } else {
                [0, 51, 153, 255]
            };
            img.set(x, y, c);
        }
    }
    img.set(7, 0, [255, 0, 0, 255]);
    img
}

#[test]
fn t11_3_textured_quad_golden() {
    let mut b = MeshBuilder::default();
    b.quad(
        Vec3::new(30.0, 0.0, 0.0),
        20.0,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, -1.0),
        Some(0),
    );
    let model = PreparedViewmodel::new(&b.asset(vec![checker8()]), pal(), Dither::None).unwrap();
    assert_eq!(model.winding(), Winding::CounterClockwise);
    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    let stats = render(
        &mut r,
        &mut frame,
        &model,
        &identity_pose(),
        LightParams::Fixed { colormap: 0 },
        ALL,
    );
    assert_eq!(stats.triangles_drawn, 2);
    assert!(stats.pixels > 1000);
    // The red marker (u = 1, v = 0) lands in the top-right corner of the quad on screen.
    let red = pal().nearest([255, 0, 0]);
    let reds: Vec<(usize, usize)> = (0..VIEW_H as usize)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| frame[y * W + x] == red)
        .collect();
    assert!(!reds.is_empty());
    assert!(
        reds.iter()
            .all(|&(x, y)| x > W / 2 && y < VIEW_H as usize / 2)
    );
    assert!(frame[VIEW_H as usize * W..].iter().all(|&p| p == SBAR));
    assert_frame_golden("raster_quad", &frame);
}

/// 16x16 texture whose texels all have distinct palette indices.
fn unique_texture() -> (RgbaImage, Vec<Option<(u32, u32)>>) {
    let playpal = &pal().playpal;
    let mut img = RgbaImage::new(16, 16, [0; 4]);
    let mut back = vec![None; 256];
    for y in 0..16u32 {
        for x in 0..16u32 {
            let k = (y * 16 + x) as usize;
            let c = playpal[k];
            img.set(x, y, [c[0], c[1], c[2], 255]);
            back[k] = Some((x, y));
        }
    }
    (img, back)
}

#[test]
fn t11_3_perspective_correct_within_quarter_texel() {
    let (tex, back) = unique_texture();
    let rot = rot_z(70.0);
    let u_axis = rot * Vec3::new(0.0, -1.0, 0.0);
    let v_axis = Vec3::new(0.0, 0.0, -1.0);
    let centre = Vec3::new(30.0, 0.0, 0.0);
    let size = 24.0;
    let mut b = MeshBuilder::default();
    b.quad(centre, size, u_axis, v_axis, Some(0));
    let model = PreparedViewmodel::new(&b.asset(vec![tex]), pal(), Dither::None).unwrap();
    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    render(
        &mut r,
        &mut frame,
        &model,
        &identity_pose(),
        LightParams::Fixed { colormap: 0 },
        ALL,
    );

    // Exact reference per pixel centre: intersect the eye ray with the quad plane.
    let fx = 160.0 / (32.5f32).to_radians().tan();
    let fy = fx * 5.0 / 6.0;
    let (cx, cy) = (160.0, VIEW_H as f32 / 2.0);
    let normal = u_axis.cross(v_axis);
    let origin = centre - 0.5 * size * u_axis - 0.5 * size * v_axis;
    let (mut compared, mut exact, mut outside_tol) = (0u32, 0u32, 0u32);
    let mut worst = 0.0f32;
    for py in 0..VIEW_H as usize {
        for px in 0..W {
            let idx = frame[py * W + px];
            let Some((tx, ty)) = back[usize::from(idx)] else {
                continue;
            };
            if idx == BG {
                continue;
            }
            let dir = Vec3::new(
                1.0,
                -(px as f32 + 0.5 - cx) / fx,
                -(py as f32 + 0.5 - cy) / fy,
            );
            let t = (origin.dot(normal)) / dir.dot(normal);
            let hit = dir * t;
            let u = (hit - origin).dot(u_axis) / size;
            let v = (hit - origin).dot(v_axis) / size;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            compared += 1;
            let (eu, ev) = (u * 16.0, v * 16.0);
            if eu.floor() as u32 == tx && ev.floor() as u32 == ty {
                exact += 1;
                continue;
            }
            // Distance (in texels) from the exact sample point to the sampled texel's square.
            let du = (tx as f32 - eu).max(eu - (tx as f32 + 1.0)).max(0.0);
            let dv = (ty as f32 - ev).max(ev - (ty as f32 + 1.0)).max(0.0);
            let d = du.max(dv);
            worst = worst.max(d);
            if d > 0.25 {
                outside_tol += 1;
            }
        }
    }
    #[allow(clippy::print_stderr)]
    {
        eprintln!(
            "T11.3 perspective: compared {compared}, same texel {exact}, worst distance {worst:.4} texel, beyond 0.25: {outside_tol}"
        );
    }
    assert!(compared > 2000, "only {compared} pixels compared");
    assert_eq!(outside_tol, 0, "worst {worst}");
}

// ---------------------------------------------------------------- T11.4

#[test]
fn t11_4_nearer_wins_regardless_of_order() {
    let red = RgbaImage::new(1, 1, [255, 0, 0, 255]);
    let green = RgbaImage::new(1, 1, [0, 255, 0, 255]);
    // Near triangle (x = 20) uses texture 0, far (x = 30) texture 1; both cover the centre.
    let tri = |x: f32| {
        [
            Vec3::new(x, 6.0, 4.0),
            Vec3::new(x, -6.0, 4.0),
            Vec3::new(x, 0.0, -6.0),
        ]
    };
    let uv = [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]];
    let frames: Vec<Vec<u8>> = [false, true]
        .into_iter()
        .map(|near_first| {
            let mut b = MeshBuilder::default();
            let order = if near_first {
                [20.0, 30.0]
            } else {
                [30.0, 20.0]
            };
            for x in order {
                // Wound so the geometric normal faces the eye.
                let p = facing(tri(x));
                let tex = if x == 20.0 { 0 } else { 1 };
                b.triangle([p[0], p[1], p[2]], uv, Some(tex), CullMode::Back);
            }
            let model = PreparedViewmodel::new(
                &b.asset(vec![red.clone(), green.clone()]),
                pal(),
                Dither::None,
            )
            .unwrap();
            let mut r = Rasterizer::new();
            let mut frame = blank_frame();
            let s = render(
                &mut r,
                &mut frame,
                &model,
                &identity_pose(),
                LightParams::Fixed { colormap: 0 },
                ALL,
            );
            assert_eq!(s.triangles_drawn, 2, "near_first {near_first}");
            frame
        })
        .collect();
    assert_eq!(frames[0], frames[1]);
    let centre = frames[0][(VIEW_H as usize / 2) * W + W / 2];
    assert_eq!(centre, pal().nearest([255, 0, 0]));
    // The far triangle has the same size at 1.5x the distance, so it is smaller on screen and
    // fully hidden: no green pixel survives.
    assert!(!frames[0].contains(&pal().nearest([0, 255, 0])));
}

// ---------------------------------------------------------------- T11.5

#[test]
fn t11_5_near_clip_no_nan_no_pixels_behind() {
    let tex = RgbaImage::new(4, 4, [255, 255, 255, 255]);
    // One vertex behind the eye, two in front, spanning the screen centre from below.
    let p = [
        Vec3::new(-5.0, 0.0, -3.0),
        Vec3::new(12.0, -4.0, -3.0),
        Vec3::new(12.0, 4.0, -3.0),
    ];
    let mut b = MeshBuilder::default();
    // Order so the face (normal +z, plane z = -3, seen from above) faces the eye.
    b.triangle(
        facing(p),
        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        Some(0),
        CullMode::Back,
    );
    // A triangle entirely behind the eye and one with a NaN vertex.
    b.triangle(
        [
            Vec3::new(-5.0, 1.0, 0.0),
            Vec3::new(-5.0, -1.0, 0.0),
            Vec3::new(-5.0, 0.0, 1.0),
        ],
        [[0.0, 0.0]; 3],
        Some(0),
        CullMode::None,
    );
    let model = PreparedViewmodel::new(&b.asset(vec![tex]), pal(), Dither::None).unwrap();
    let mut mesh_nan = b.asset(vec![RgbaImage::new(1, 1, [255; 4])]);
    mesh_nan.mesh.positions[0][0] = f32::NAN;
    let model_nan = PreparedViewmodel::new(&mesh_nan, pal(), Dither::None).unwrap();

    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    let s = render(
        &mut r,
        &mut frame,
        &model,
        &identity_pose(),
        LightParams::Fixed { colormap: 0 },
        ALL,
    );
    assert!(s.triangles_drawn >= 1 && s.triangles_drawn <= 2, "{s:?}");
    assert!(s.pixels > 0);
    // Every drawn pixel must be a point of the plane z = -3 in front of the near plane, i.e.
    // below the horizon and inside the triangle's front part.
    let fx = 160.0 / (32.5f32).to_radians().tan();
    let fy = fx * 5.0 / 6.0;
    let mut drawn = 0;
    for py in 0..VIEW_H as usize {
        for px in 0..W {
            if frame[py * W + px] == BG {
                continue;
            }
            drawn += 1;
            let dz = -(py as f32 + 0.5 - 84.0) / fy;
            assert!(dz < 0.0, "pixel above the horizon at ({px}, {py})");
            let x = -3.0 / dz; // ray (1, dy, dz) hits z = -3 at depth x
            assert!(
                x.is_finite() && x >= dmw_viewmodel::NEAR_PLANE - 0.05,
                "x {x}"
            );
            let y = -(px as f32 + 0.5 - 160.0) / fx * x;
            // Inside the triangle's lateral extent at that depth (edges from (-5,0) to (12,±4)).
            let half = 4.0 * (x + 5.0) / 17.0;
            assert!(y.abs() <= half + 0.2, "({px},{py}) y {y} half {half}");
        }
    }
    assert_eq!(drawn, s.pixels);

    let mut frame2 = blank_frame();
    let s2 = render(
        &mut r,
        &mut frame2,
        &model_nan,
        &identity_pose(),
        LightParams::Fixed { colormap: 0 },
        ALL,
    );
    assert!(s2.triangles_drawn <= 2);
    // Only the clipped triangle can still contribute; its first vertex is NaN now -> skipped.
    assert_eq!(s2.pixels, 0, "{s2:?}");
}

// ---------------------------------------------------------------- T11.6

#[test]
fn t11_6_shading_levels_golden_and_invulnerability() {
    let model = PreparedViewmodel::new(&fixture_asset(1), pal(), Dither::Bayer4).unwrap();
    let pose = fixture_bind_pose();
    let mut r = Rasterizer::new();
    let mut prev: Option<u64> = None;
    for level in [0u8, 16, 31] {
        let mut frame = blank_frame();
        render(
            &mut r,
            &mut frame,
            &model,
            &pose,
            LightParams::Sector {
                colormap_level: level,
                directional: true,
            },
            Visibility::default(),
        );
        // Brightness falls with the level.
        let sum: u64 = frame[..VIEW_H as usize * W]
            .iter()
            .filter(|&&i| i != BG)
            .map(|&i| {
                pal().playpal[usize::from(i)]
                    .iter()
                    .map(|&c| u64::from(c))
                    .sum::<u64>()
            })
            .sum();
        if let Some(p) = prev {
            assert!(sum < p, "level {level} not darker");
        }
        prev = Some(sum);
        assert_frame_golden(&format!("shade_level_{level:02}"), &frame);
    }

    let mut frame = blank_frame();
    let s = render(
        &mut r,
        &mut frame,
        &model,
        &pose,
        LightParams::Fixed { colormap: 32 },
        Visibility::default(),
    );
    assert!(s.pixels > 0);
    let allowed: std::collections::BTreeSet<u8> = pal().colormaps[32].iter().copied().collect();
    let base = blank_frame();
    let mut changed = 0;
    for (a, b) in frame.iter().zip(&base) {
        if a != b {
            changed += 1;
            assert!(allowed.contains(a), "index {a} not in colormap 32");
        }
    }
    assert!(changed > 0);
}

#[test]
fn directional_term_changes_faces() {
    let model = PreparedViewmodel::new(&fixture_asset(1), pal(), Dither::None).unwrap();
    let pose = fixture_bind_pose();
    let mut r = Rasterizer::new();
    let mut flat = blank_frame();
    let mut lit = blank_frame();
    let light = |d| LightParams::Sector {
        colormap_level: 8,
        directional: d,
    };
    render(
        &mut r,
        &mut flat,
        &model,
        &pose,
        light(false),
        Visibility::default(),
    );
    render(
        &mut r,
        &mut lit,
        &model,
        &pose,
        light(true),
        Visibility::default(),
    );
    assert_ne!(flat, lit);
}

// ---------------------------------------------------------------- T11.7

fn doom_fuzzoffset() -> Vec<i32> {
    const F: i32 = 320;
    vec![
        F, -F, F, -F, F, F, -F, F, F, -F, F, F, F, -F, F, F, F, -F, -F, -F, -F, F, -F, -F, F, F, F,
        F, -F, F, -F, F, F, -F, -F, F, F, -F, -F, -F, -F, F, F, F, F, -F, F, F, -F, F,
    ]
}

#[test]
fn t11_7_fuzz_uses_colormap_6_of_neighbours() {
    let model = PreparedViewmodel::new(&fixture_asset(1), pal(), Dither::None).unwrap();
    let pose = fixture_bind_pose();
    let mut r = Rasterizer::new();

    // Silhouette from a normal draw.
    let mut solid = blank_frame();
    render(
        &mut r,
        &mut solid,
        &model,
        &pose,
        LightParams::Fixed { colormap: 0 },
        Visibility::default(),
    );

    let mut base = blank_frame();
    for y in 0..VIEW_H as usize {
        for x in 0..W {
            base[y * W + x] = ((x * 7 + y * 13) % 256) as u8;
        }
    }
    let fuzzoffset = doom_fuzzoffset();
    let mut frame = base.clone();
    let mut fuzzpos = 17usize;
    let stats = {
        let mut t = target(&mut frame, VIEW_H);
        let mut params = ViewmodelDrawParams {
            pose: &pose,
            hfov_deg: 65.0,
            light: LightParams::Fuzz {
                fuzzoffset: &fuzzoffset,
                fuzzpos: &mut fuzzpos,
            },
            pal: pal(),
            visibility: Visibility::default(),
        };
        r.draw_mut(&mut t, &model, &mut params)
    };
    assert!(stats.pixels > 100);
    assert_eq!(
        fuzzpos,
        (17 + stats.pixels as usize) % 50,
        "fuzzpos did not advance"
    );
    assert_eq!(r.last_fuzzpos(), Some(fuzzpos));

    let cmap6 = &pal().colormaps[6];
    let mut changed = 0;
    for i in 0..W * H {
        if frame[i] == base[i] {
            continue;
        }
        changed += 1;
        assert_ne!(solid[i], BG, "fuzz outside the silhouette at {i}");
        let candidates = [i - W, i + W]
            .into_iter()
            .flat_map(|j| [base[j], frame[j]])
            .map(|v| cmap6[usize::from(v)]);
        assert!(
            candidates.into_iter().any(|c| c == frame[i]),
            "pixel {i} = {} is not colormap 6 of a vertical neighbour",
            frame[i]
        );
    }
    assert!(changed > 0);
    assert!(frame[VIEW_H as usize * W..].iter().all(|&p| p == SBAR));

    // The shared-reference `draw` leaves fuzzpos alone but reports the new position.
    let mut pos2 = 3usize;
    let mut f2 = base.clone();
    let mut t = target(&mut f2, VIEW_H);
    let s = r.draw(
        &mut t,
        &model,
        &ViewmodelDrawParams {
            pose: &pose,
            hfov_deg: 65.0,
            light: LightParams::Fuzz {
                fuzzoffset: &fuzzoffset,
                fuzzpos: &mut pos2,
            },
            pal: pal(),
            visibility: Visibility::default(),
        },
    );
    assert_eq!(r.last_fuzzpos(), Some((3 + s.pixels as usize) % 50));
}

// ---------------------------------------------------------------- winding

#[test]
fn winding_detection() {
    let fixture = fixture_asset(0);
    let m = PreparedViewmodel::new(&fixture, pal(), Dither::None).unwrap();
    assert_eq!(m.winding(), Winding::CounterClockwise);

    // Reverse every triangle and flip normals consistently -> clockwise convention... the
    // stored normals keep pointing outward, so the triangle order alone decides.
    let mut cw = fixture.clone();
    for t in cw.mesh.indices.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
    let m_cw = PreparedViewmodel::new(&cw, pal(), Dither::None).unwrap();
    assert_eq!(m_cw.winding(), Winding::Clockwise);

    // The clockwise copy draws exactly the same image.
    let pose = fixture_bind_pose();
    let mut r = Rasterizer::new();
    let mut a = blank_frame();
    let mut b = blank_frame();
    let light = || LightParams::Sector {
        colormap_level: 8,
        directional: true,
    };
    render(&mut r, &mut a, &m, &pose, light(), Visibility::default());
    render(&mut r, &mut b, &m_cw, &pose, light(), Visibility::default());
    assert_eq!(a, b);
}

// ---------------------------------------------------------------- roles and errors

#[test]
fn role_visibility_rules() {
    let asset = fixture_asset(4);
    let model = PreparedViewmodel::new(&asset, pal(), Dither::None).unwrap();
    let pose = fixture_bind_pose();
    let mut r = Rasterizer::new();
    let tris = |role: PartRole| -> u32 {
        asset
            .mesh
            .surfaces
            .iter()
            .filter(|s| s.role == role)
            .map(|s| s.index_count / 3)
            .sum()
    };
    let light = || LightParams::Fixed { colormap: 0 };
    let mut f = blank_frame();
    let none = render(
        &mut r,
        &mut f,
        &model,
        &pose,
        light(),
        Visibility::default(),
    );
    let mut f = blank_frame();
    let all = render(&mut r, &mut f, &model, &pose, light(), ALL);
    let total = (asset.mesh.indices.len() / 3) as u32;
    assert_eq!(
        none.triangles_in,
        total - tris(PartRole::Rocket) - tris(PartRole::Knife) - tris(PartRole::ScopeLens)
    );
    assert_eq!(all.triangles_in, total - tris(PartRole::ScopeLens));
}

#[test]
fn broken_meshes_are_rejected_and_missing_textures_go_flat() {
    let mut a = fixture_asset(0);
    a.mesh.indices.push(0);
    assert!(PreparedViewmodel::new(&a, pal(), Dither::None).is_err());
    let mut a = fixture_asset(0);
    a.mesh.indices[0] = 1_000_000;
    assert!(PreparedViewmodel::new(&a, pal(), Dither::None).is_err());
    let mut a = fixture_asset(0);
    a.mesh.normals.pop();
    assert!(PreparedViewmodel::new(&a, pal(), Dither::None).is_err());
    let mut a = fixture_asset(0);
    a.textures.clear();
    let m = PreparedViewmodel::new(&a, pal(), Dither::None).unwrap();
    assert!(m.textures().is_empty());
    let mut r = Rasterizer::new();
    let mut f = blank_frame();
    let s = render(
        &mut r,
        &mut f,
        &m,
        &fixture_bind_pose(),
        LightParams::Fixed { colormap: 0 },
        Visibility::default(),
    );
    assert!(s.pixels > 0);
    let grey = pal().nearest([128, 128, 128]);
    assert!(f.iter().all(|&p| p == BG || p == grey || p == SBAR));
}

#[test]
fn degenerate_targets_draw_nothing() {
    let model = PreparedViewmodel::new(&fixture_asset(0), pal(), Dither::None).unwrap();
    let pose = fixture_bind_pose();
    let mut r = Rasterizer::new();
    for (vw, vh, fov) in [
        (0, 168, 65.0),
        (320, 0, 65.0),
        (320, 168, 0.0),
        (320, 168, f32::NAN),
    ] {
        let mut f = blank_frame();
        let mut t = target(&mut f, vh);
        t.view_w = vw;
        let s = r.draw(
            &mut t,
            &model,
            &ViewmodelDrawParams {
                pose: &pose,
                hfov_deg: fov,
                light: LightParams::Fixed { colormap: 0 },
                pal: pal(),
                visibility: Visibility::default(),
            },
        );
        assert_eq!(s.pixels, 0);
        assert_eq!(f, blank_frame());
    }
}

// ---------------------------------------------------------------- fixture goldens (T11.8 stand-in)

fn fixture_light() -> LightParams<'static> {
    LightParams::Sector {
        colormap_level: 8,
        directional: true,
    }
}

/// Bind pose: the fixture gun in the lower right, hands below it.
#[test]
fn fixture_bind_golden() {
    let model = PreparedViewmodel::new(&fixture_asset(1), pal(), Dither::Bayer4).unwrap();
    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    let s = render(
        &mut r,
        &mut frame,
        &model,
        &fixture_bind_pose(),
        fixture_light(),
        Visibility::default(),
    );
    assert!(s.pixels > 1000);
    // Lower-right placement: the silhouette's centroid is right of and below the centre.
    let (mut sx, mut sy, mut n) = (0usize, 0usize, 0usize);
    for y in 0..VIEW_H as usize {
        for x in 0..W {
            if frame[y * W + x] != BG {
                sx += x;
                sy += y;
                n += 1;
            }
        }
    }
    assert!(
        sx / n > W / 2 && sy / n > VIEW_H as usize / 2,
        "centroid {} {}",
        sx / n,
        sy / n
    );
    assert!(frame[VIEW_H as usize * W..].iter().all(|&p| p == SBAR));
    assert_frame_golden("fixture_bind", &frame);
}

/// Hand-posed "mid reload": the magazine bone pulled 4 in forward, 3 in left and 3 in up (at
/// bind it hangs below the view window); the blended flap stretches with it.
#[test]
fn fixture_mag_out_golden() {
    use dmw_format::fixture::geometry::bone;
    let model = PreparedViewmodel::new(&fixture_asset(1), pal(), Dither::Bayer4).unwrap();
    let mut pose = fixture_bind_pose();
    pose.skin[usize::from(bone::J_MAG)] = Mat4::from_translation(Vec3::new(4.0, 3.0, 3.0));
    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    render(
        &mut r,
        &mut frame,
        &model,
        &pose,
        fixture_light(),
        Visibility::default(),
    );
    let mut bind = blank_frame();
    render(
        &mut r,
        &mut bind,
        &model,
        &fixture_bind_pose(),
        fixture_light(),
        Visibility::default(),
    );
    assert_ne!(frame, bind);
    assert_frame_golden("fixture_mag_out", &frame);
}

/// Hand-posed "ADS": every bone moved so the gun's axis sits on the view axis, slightly below.
#[test]
fn fixture_centred_golden() {
    let model = PreparedViewmodel::new(&fixture_asset(1), pal(), Dither::Bayer4).unwrap();
    let ads = Mat4::from_translation(Vec3::new(6.0, 5.0, 3.0));
    let pose = Pose {
        skin: vec![ads; 8],
        tag_flash: None,
    };
    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    render(
        &mut r,
        &mut frame,
        &model,
        &pose,
        fixture_light(),
        Visibility::default(),
    );
    // Horizontally centred silhouette.
    let (mut sx, mut n) = (0usize, 0usize);
    for y in 0..VIEW_H as usize {
        for x in 0..W {
            if frame[y * W + x] != BG {
                sx += x;
                n += 1;
            }
        }
    }
    let cx = sx / n;
    assert!((140..=180).contains(&cx), "centroid x {cx}");
    assert_frame_golden("fixture_centred", &frame);
}

/// Drawing is deterministic: the same inputs give identical bytes on every call.
#[test]
fn repeat_draws_identical() {
    let model = PreparedViewmodel::new(&fixture_asset(2), pal(), Dither::Bayer4).unwrap();
    let mut r = Rasterizer::new();
    let mut a = blank_frame();
    let mut b = blank_frame();
    render(
        &mut r,
        &mut a,
        &model,
        &fixture_bind_pose(),
        fixture_light(),
        ALL,
    );
    render(
        &mut r,
        &mut b,
        &model,
        &fixture_bind_pose(),
        fixture_light(),
        ALL,
    );
    assert_eq!(a, b);
}
