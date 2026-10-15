// SPDX-License-Identifier: GPL-3.0-or-later
//! T11.10: a 20 000-triangle skinned mesh must draw in ≤ 2 ms (release build) and a
//! steady-state `Rasterizer::draw` must not allocate (counting allocator).
//!
//! Run with `cargo test --release -p dmw_viewmodel --test perf -- --nocapture` to see the
//! numbers. The time budget is asserted only in release builds; the allocation check always.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

#[path = "../../../room/tests/support/alloc.rs"]
#[allow(dead_code)]
mod alloc;
mod common;

#[global_allocator]
static ALLOC: alloc::CountingAlloc = alloc::CountingAlloc;

use std::time::Instant;

use common::*;
use dmw_format::{
    AlphaMode, CullMode, PartRole, RgbaImage, SurfaceData, VertexSkin, ViewmodelAsset,
    ViewmodelMesh,
};
use dmw_viewmodel::{
    Dither, LightParams, PaletteTables, Pose, PreparedViewmodel, Rasterizer, Visibility,
};
use glam::{Mat4, Quat, Vec3};

const RINGS: u32 = 100;
const SEGMENTS: u32 = 100;

/// A closed-sided tube along +x in the lower right of the view, `2 * RINGS * SEGMENTS` =
/// 20 000 triangles, skinned to two bones with weights blending along its length.
fn tube_asset() -> ViewmodelAsset {
    let mut mesh = ViewmodelMesh {
        positions: vec![],
        normals: vec![],
        uvs: vec![],
        skin: vec![],
        indices: vec![],
        surfaces: vec![],
    };
    let (x0, len, radius) = (6.0f32, 24.0f32, 2.5f32);
    let centre = Vec3::new(0.0, -5.0, -5.0);
    for r in 0..=RINGS {
        let t = r as f32 / RINGS as f32;
        for s in 0..=SEGMENTS {
            let a = s as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let n = Vec3::new(0.0, a.cos(), a.sin());
            let p = centre + Vec3::new(x0 + t * len, 0.0, 0.0) + n * radius;
            mesh.positions.push(p.to_array());
            mesh.normals.push(n.to_array());
            mesh.uvs.push([s as f32 / SEGMENTS as f32 * 4.0, t * 8.0]);
            mesh.skin.push(VertexSkin {
                bones: [0, 1, 0, 0],
                weights: [1.0 - t, t, 0.0, 0.0],
            });
        }
    }
    let stride = SEGMENTS + 1;
    for r in 0..RINGS {
        for s in 0..SEGMENTS {
            let a = r * stride + s;
            let b = a + 1;
            let c = a + stride + 1;
            let d = a + stride;
            // Outward-facing counter-clockwise: (a, b, c), (a, c, d) with s growing around +x.
            mesh.indices.extend_from_slice(&[a, c, b, a, d, c]);
        }
    }
    mesh.surfaces.push(SurfaceData {
        name: "tube".into(),
        index_start: 0,
        index_count: mesh.indices.len() as u32,
        texture: Some(0),
        alpha: AlphaMode::Test { cutoff: 0.5 },
        cull: CullMode::Back,
        role: PartRole::Gun,
        model: 0,
    });
    let mut tex = RgbaImage::new(256, 256, [0; 4]);
    for y in 0..256u32 {
        for x in 0..256u32 {
            let v = ((x ^ y) & 0xFF) as u8;
            tex.set(
                x,
                y,
                [
                    v,
                    255 - v,
                    (x as u8).wrapping_mul(3),
                    if (x / 8 + y / 8) % 7 == 0 { 0 } else { 255 },
                ],
            );
        }
    }
    ViewmodelAsset {
        mesh,
        skeleton: flat_skeleton(2),
        clips: empty_clips(),
        textures: vec![tex],
    }
}

fn tube_pose() -> Pose {
    Pose {
        skin: vec![
            Mat4::IDENTITY,
            Mat4::from_rotation_translation(Quat::from_rotation_z(0.08), Vec3::new(0.0, 0.5, 0.3)),
        ],
        tag_flash: None,
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

#[test]
fn t11_10_twenty_k_triangles_budget_and_no_allocations() {
    let started = Instant::now();
    let tables =
        PaletteTables::from_wad_lumps(&synthetic_playpal(), &synthetic_colormap()).unwrap();
    let lut_ms = started.elapsed().as_secs_f64() * 1e3;

    let asset = tube_asset();
    let started = Instant::now();
    let model = PreparedViewmodel::new(&asset, &tables, Dither::Bayer4).unwrap();
    let prep_ms = started.elapsed().as_secs_f64() * 1e3;
    assert_eq!(model.triangle_count(), 20_000);

    let pose = tube_pose();
    let mut r = Rasterizer::new();
    let mut frame = blank_frame();
    let light = || LightParams::Sector {
        colormap_level: 8,
        directional: true,
    };
    let draw = |r: &mut Rasterizer, frame: &mut [u8]| {
        let mut t = target(frame, VIEW_H);
        r.draw(
            &mut t,
            &model,
            &dmw_viewmodel::ViewmodelDrawParams {
                pose: &pose,
                hfov_deg: 65.0,
                light: light(),
                pal: &tables,
                visibility: Visibility::default(),
            },
        )
    };
    let first = draw(&mut r, &mut frame);
    assert_eq!(first.triangles_in, 20_000);
    assert!(first.pixels > 2000, "{first:?}");

    // Steady state: zero allocations over 20 draws.
    alloc::assert_no_allocations("Rasterizer::draw steady state", || {
        for _ in 0..20 {
            std::hint::black_box(draw(&mut r, &mut frame));
        }
    });

    // Timing.
    let n = if cfg!(debug_assertions) { 20 } else { 500 };
    let mut ms: Vec<f64> = (0..n)
        .map(|_| {
            let t0 = Instant::now();
            std::hint::black_box(draw(&mut r, &mut frame));
            t0.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    ms.sort_by(f64::total_cmp);
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    eprintln!(
        "T11.10 ({}): 20k tris, drawn {} / pixels {}: mean {mean:.3} ms, min {:.3}, p50 {:.3}, p99 {:.3}, max {:.3} ms over {n} draws; LUT build {lut_ms:.1} ms; prepare {prep_ms:.1} ms",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        first.triangles_drawn,
        first.pixels,
        ms[0],
        percentile(&ms, 0.5),
        percentile(&ms, 0.99),
        ms[ms.len() - 1],
    );

    // Fixture gun for reference.
    let fixture = PreparedViewmodel::new(&fixture_asset(1), &tables, Dither::Bayer4).unwrap();
    let fpose = fixture_bind_pose();
    let mut fr = Rasterizer::new();
    let mut f = blank_frame();
    let fdraw = |r: &mut Rasterizer, f: &mut [u8]| {
        let mut t = target(f, VIEW_H);
        r.draw(
            &mut t,
            &fixture,
            &dmw_viewmodel::ViewmodelDrawParams {
                pose: &fpose,
                hfov_deg: 65.0,
                light: light(),
                pal: &tables,
                visibility: Visibility::default(),
            },
        )
    };
    fdraw(&mut fr, &mut f);
    alloc::assert_no_allocations("fixture draw steady state", || {
        std::hint::black_box(fdraw(&mut fr, &mut f));
    });
    let t0 = Instant::now();
    for _ in 0..n {
        std::hint::black_box(fdraw(&mut fr, &mut f));
    }
    eprintln!(
        "T11.10 fixture gun: mean {:.3} ms",
        t0.elapsed().as_secs_f64() * 1e3 / n as f64
    );

    if !cfg!(debug_assertions) {
        // The median is asserted: the development machine runs other builds concurrently, and
        // the mean picks up their scheduling noise (both are printed above).
        let p50 = percentile(&ms, 0.5);
        assert!(
            p50 <= 2.0,
            "median draw {p50:.3} ms exceeds the 2 ms budget"
        );
        assert!(lut_ms < 100.0, "LUT build {lut_ms:.1} ms exceeds 100 ms");
    }
}
