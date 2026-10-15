// SPDX-License-Identifier: GPL-3.0-or-later
//! Projection, near-plane clipping and the perspective-correct, z-buffered triangle raster
//! (sheet 11 sections 5.5 and 5.8).
//!
//! Screen coordinates are snapped to 1/16 pixel and edge functions are evaluated in exact
//! integer arithmetic with the top-left fill rule, so triangles sharing an edge never leave a
//! crack or draw a pixel twice, and the result does not depend on submission order beyond the
//! depth test. Attributes (`1/x`, `u/x`, `v/x`, colormap level) are planes in screen space;
//! `u` and `v` are recovered per pixel by one division, so texturing is exactly
//! perspective-correct.

use std::time::Instant;

use dmw_format::PartRole;
use glam::Vec3;

use crate::prepared::{PreparedSurface, PreparedViewmodel, Winding};
use crate::quantize::IndexedMip;
use crate::shade::{FUZZ_COLORMAP, apply_fuzz, light_dir, vertex_level};
use crate::skin::skin_vertices;
use crate::{DrawStats, DrawTarget, LightParams, ViewmodelDrawParams, Visibility};

/// Near clip plane in eye space (inches along +x).
pub const NEAR_PLANE: f32 = 0.5;

/// Sub-pixel steps per pixel (fixed-point screen coordinates).
const SUB: i64 = 16;
/// Largest accepted screen coordinate magnitude in pixels; triangles beyond are skipped.
const MAX_COORD: f32 = (1 << 23) as f32;

/// A vertex before projection (eye space) with its attributes.
#[derive(Debug, Clone, Copy, Default)]
struct ClipVert {
    p: Vec3,
    u: f32,
    v: f32,
    /// `dot(n, L)` with the normal flipped for back faces.
    ndl: f32,
}

/// A projected vertex.
#[derive(Debug, Clone, Copy, Default)]
struct ScreenVert {
    /// Fixed-point screen position (1/16 pixel).
    x: i64,
    y: i64,
    inv_x: f32,
    u_x: f32,
    v_x: f32,
    u: f32,
    v: f32,
    level: f32,
}

/// Per-vertex projection cache for triangles that need no clipping.
#[derive(Debug, Clone, Copy, Default)]
struct Projected {
    x: i64,
    y: i64,
    inv_x: f32,
    /// In front of the near plane and inside the coordinate limits.
    ok: bool,
}

/// How a triangle's covered pixels are written.
#[derive(Clone, Copy)]
enum PixMode<'a> {
    /// `colormaps[round(level)][texel]`.
    Sector(&'a [[u8; 256]]),
    /// One colormap.
    Fixed(&'a [u8; 256]),
    /// Coverage mask only (fuzz).
    Cover,
}

/// Camera and window constants for one draw.
#[derive(Debug, Clone, Copy)]
struct View {
    fx: f32,
    fy: f32,
    cx: f32,
    cy: f32,
    /// Clipped window in screen pixels, [x0, x1) x [y0, y1).
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl View {
    #[inline]
    fn project(&self, p: Vec3) -> Option<(i64, i64, f32)> {
        let inv_x = 1.0 / p.x;
        let sx = self.cx - self.fx * p.y * inv_x;
        let sy = self.cy - self.fy * p.z * inv_x;
        if !(sx.abs() < MAX_COORD && sy.abs() < MAX_COORD && inv_x.is_finite()) {
            return None;
        }
        Some((
            (sx * SUB as f32).round() as i64,
            (sy * SUB as f32).round() as i64,
            inv_x,
        ))
    }
}

/// The software viewmodel rasterizer. Owns the z-buffer and every scratch buffer, so a
/// steady-state [`Rasterizer::draw`] does not allocate.
#[derive(Debug)]
pub struct Rasterizer {
    zbuf: Vec<f32>,
    cover: Vec<u8>,
    skinned_p: Vec<Vec3>,
    skinned_n: Vec<Vec3>,
    ndl: Vec<f32>,
    /// Per vertex: sector colormap level for the front and the back face.
    levels: Vec<[f32; 2]>,
    proj: Vec<Projected>,
    last_fuzzpos: Option<usize>,
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Is `role` drawn under `vis`?
fn role_visible(role: PartRole, vis: Visibility) -> bool {
    match role {
        PartRole::ScopeLens => false,
        PartRole::Rocket => vis.rocket,
        PartRole::Knife => vis.knife,
        PartRole::Hands | PartRole::Gun | PartRole::Attachment => true,
    }
}

/// Sutherland–Hodgman against `x >= NEAR_PLANE`. Returns the polygon (0, 3 or 4 vertices).
fn clip_near(tri: &[ClipVert; 3]) -> ([ClipVert; 4], usize) {
    let mut out = [ClipVert::default(); 4];
    let mut n = 0;
    for i in 0..3 {
        let a = tri[i];
        let b = tri[(i + 1) % 3];
        let a_in = a.p.x >= NEAR_PLANE;
        let b_in = b.p.x >= NEAR_PLANE;
        if a_in {
            out[n] = a;
            n += 1;
        }
        if a_in != b_in {
            let t = (NEAR_PLANE - a.p.x) / (b.p.x - a.p.x);
            let mut c = ClipVert {
                p: a.p + (b.p - a.p) * t,
                u: a.u + (b.u - a.u) * t,
                v: a.v + (b.v - a.v) * t,
                ndl: a.ndl + (b.ndl - a.ndl) * t,
            };
            c.p.x = NEAR_PLANE;
            out[n] = c;
            n += 1;
        }
    }
    (out, n)
}

/// Mip level for a triangle covering `screen_area` pixels and `texel_area` mip-0 texels.
fn select_mip(texel_area: f32, screen_area: f32, mips: usize) -> usize {
    if !(texel_area > 0.0 && screen_area > 0.0) || mips <= 1 {
        return 0;
    }
    let m = (0.5 * (texel_area / screen_area).log2()).round();
    if m.is_nan() || m < 0.0 {
        0
    } else {
        (m as usize).min(mips - 1)
    }
}

/// Is the directed edge a top or left edge (y grows downward, positive-area orientation)?
#[inline]
fn top_left(ax: i64, ay: i64, bx: i64, by: i64) -> bool {
    let dy = by - ay;
    (dy == 0 && bx > ax) || dy < 0
}

/// Mutable buffers one triangle writes to.
struct Buffers<'a> {
    pixels: &'a mut [u8],
    pitch: usize,
    zbuf: &'a mut [f32],
    cover: &'a mut [u8],
}

/// Texture source of one triangle.
#[derive(Clone, Copy)]
enum Texels<'a> {
    Mip(&'a IndexedMip),
    Flat(u8),
}

/// Scan-converts one projected triangle. Returns pixels written.
///
/// `texels` is called with the doubled triangle area (fixed-point units) only once the
/// triangle is known to contain a pixel centre, so mip selection is skipped for the many
/// sub-pixel triangles of dense meshes.
fn raster_triangle<'t>(
    view: &View,
    buf: &mut Buffers<'_>,
    tri: [ScreenVert; 3],
    texels: impl FnOnce(i64) -> Texels<'t>,
    mode: PixMode<'_>,
) -> u32 {
    let [v0, mut v1, mut v2] = tri;
    let area = (v1.x - v0.x) * (v2.y - v0.y) - (v2.x - v0.x) * (v1.y - v0.y);
    if area == 0 {
        return 0;
    }
    if area < 0 {
        std::mem::swap(&mut v1, &mut v2);
    }

    // Pixels whose centres (16k + 8) lie inside the bounding box, clipped to the window.
    let min_x = v0.x.min(v1.x).min(v2.x);
    let max_x = v0.x.max(v1.x).max(v2.x);
    let min_y = v0.y.min(v1.y).min(v2.y);
    let max_y = v0.y.max(v1.y).max(v2.y);
    let half = SUB / 2;
    let px0 = (min_x - half + SUB - 1)
        .div_euclid(SUB)
        .max(i64::from(view.x0));
    let px1 = (max_x - half).div_euclid(SUB).min(i64::from(view.x1) - 1);
    let py0 = (min_y - half + SUB - 1)
        .div_euclid(SUB)
        .max(i64::from(view.y0));
    let py1 = (max_y - half).div_euclid(SUB).min(i64::from(view.y1) - 1);
    if px0 > px1 || py0 > py1 {
        return 0;
    }
    let texels = texels(area.abs());

    // Edge functions E_ab(p) = (b.x - a.x)(p.y - a.y) - (b.y - a.y)(p.x - a.x), all >= 0
    // inside; non-top-left edges exclude their boundary (bias -1).
    let edges = [(v1, v2), (v2, v0), (v0, v1)];
    let mut e_row = [0i64; 3];
    let mut step_x = [0i64; 3];
    let mut step_y = [0i64; 3];
    let cx = px0 * SUB + SUB / 2;
    let cy = py0 * SUB + SUB / 2;
    for (k, (a, b)) in edges.iter().enumerate() {
        let bias = if top_left(a.x, a.y, b.x, b.y) { 0 } else { -1 };
        e_row[k] = (b.x - a.x) * (cy - a.y) - (b.y - a.y) * (cx - a.x) + bias;
        step_x[k] = -(b.y - a.y) * SUB;
        step_y[k] = (b.x - a.x) * SUB;
    }

    // Attribute planes in pixel units.
    let x0f = v0.x as f32 / SUB as f32;
    let y0f = v0.y as f32 / SUB as f32;
    let (dx1, dy1) = (
        (v1.x - v0.x) as f32 / SUB as f32,
        (v1.y - v0.y) as f32 / SUB as f32,
    );
    let (dx2, dy2) = (
        (v2.x - v0.x) as f32 / SUB as f32,
        (v2.y - v0.y) as f32 / SUB as f32,
    );
    let inv_area = 1.0 / (dx1 * dy2 - dx2 * dy1);
    let plane = |a0: f32, a1: f32, a2: f32| -> (f32, f32, f32) {
        let (d1, d2) = (a1 - a0, a2 - a0);
        let ddx = (d1 * dy2 - d2 * dy1) * inv_area;
        let ddy = (d2 * dx1 - d1 * dx2) * inv_area;
        // value at the centre of pixel (px0, py0)
        let at = a0 + ddx * (px0 as f32 + 0.5 - x0f) + ddy * (py0 as f32 + 0.5 - y0f);
        (at, ddx, ddy)
    };
    let (iz0, izx, izy) = plane(v0.inv_x, v1.inv_x, v2.inv_x);
    let (uz0, uzx, uzy) = plane(v0.u_x, v1.u_x, v2.u_x);
    let (vz0, vzx, vzy) = plane(v0.v_x, v1.v_x, v2.v_x);
    let (l0, lx, ly) = plane(v0.level, v1.level, v2.level);

    let (tw, th, wmask, hmask) = match texels {
        Texels::Mip(m) => {
            let (w, h) = (m.width as i32, m.height as i32);
            let pow2 = (w & (w - 1)) == 0 && (h & (h - 1)) == 0;
            (
                w,
                h,
                if pow2 { w - 1 } else { -1 },
                if pow2 { h - 1 } else { -1 },
            )
        }
        Texels::Flat(_) => (1, 1, 0, 0),
    };
    let (tw_f, th_f) = (tw as f32, th as f32);
    // Texel-space range of the triangle's own uvs. Samples are clamped into it so that a pixel
    // centre lying on (or, after sub-pixel snapping, a hair outside) an edge with u = 1 never
    // wraps to the opposite side of the texture; uvs outside 0..1 still tile.
    let range = |a: f32, b: f32, c: f32, size: f32| {
        let lo = a.min(b).min(c) * size;
        let hi = (a.max(b).max(c) * size - 1.0 / 1024.0).max(lo);
        (lo, hi)
    };
    let (u_lo, u_hi) = range(v0.u, v1.u, v2.u, tw_f);
    let (v_lo, v_hi) = range(v0.v, v1.v, v2.v, th_f);

    let ww = (view.x1 - view.x0) as usize;
    let mut written = 0u32;
    for (row, py) in (py0..=py1).enumerate() {
        let rowf = row as f32;
        let mut e = e_row;
        let mut iz = iz0 + izy * rowf;
        let mut uz = uz0 + uzy * rowf;
        let mut vz = vz0 + vzy * rowf;
        let mut lv = l0 + ly * rowf;
        let zrow = (py as usize - view.y0 as usize) * ww;
        let prow = py as usize * buf.pitch;
        for px in px0..=px1 {
            if (e[0] | e[1] | e[2]) >= 0 {
                let zi = zrow + (px as usize - view.x0 as usize);
                if iz > buf.zbuf[zi] {
                    let (index, opaque) = match texels {
                        Texels::Flat(i) => (i, true),
                        Texels::Mip(m) => {
                            let u = uz / iz;
                            let v = vz / iz;
                            let tx = (u * tw_f).clamp(u_lo, u_hi).floor() as i32;
                            let ty = (v * th_f).clamp(v_lo, v_hi).floor() as i32;
                            let tx = if wmask >= 0 {
                                tx & wmask
                            } else {
                                tx.rem_euclid(tw)
                            };
                            let ty = if hmask >= 0 {
                                ty & hmask
                            } else {
                                ty.rem_euclid(th)
                            };
                            m.texel(tx as u32, ty as u32)
                        }
                    };
                    if opaque {
                        buf.zbuf[zi] = iz;
                        let pi = prow + px as usize;
                        match mode {
                            PixMode::Sector(maps) => {
                                let level = ((lv + 0.5).floor() as i32).clamp(0, 31) as usize;
                                let map = &maps[level.min(maps.len() - 1)];
                                buf.pixels[pi] = map[usize::from(index)];
                            }
                            PixMode::Fixed(map) => buf.pixels[pi] = map[usize::from(index)],
                            PixMode::Cover => buf.cover[zi] = 1,
                        }
                        written += 1;
                    }
                }
            }
            e[0] += step_x[0];
            e[1] += step_x[1];
            e[2] += step_x[2];
            iz += izx;
            uz += uzx;
            vz += vzx;
            lv += lx;
        }
        e_row[0] += step_y[0];
        e_row[1] += step_y[1];
        e_row[2] += step_y[2];
    }
    written
}

impl Rasterizer {
    /// A rasterizer with buffers for a full 320x200 view window.
    pub fn new() -> Self {
        let n = dmw_format::consts::DOOM_SCREEN_W * dmw_format::consts::DOOM_SCREEN_H;
        Self {
            zbuf: vec![0.0; n],
            cover: vec![0; n],
            skinned_p: Vec::new(),
            skinned_n: Vec::new(),
            ndl: Vec::new(),
            levels: Vec::new(),
            proj: Vec::new(),
            last_fuzzpos: None,
        }
    }

    /// Draws `model` into the target's view window. Infallible: anything outside the window,
    /// behind the near plane, degenerate or non-finite is clipped or skipped.
    ///
    /// Allocates only when a buffer must grow (first draw of a model with more vertices than
    /// any before, or a view window larger than 320x200).
    ///
    /// With [`LightParams::Fuzz`] the advanced fuzz position cannot be written back through the
    /// shared `params` reference; it is available from [`Rasterizer::last_fuzzpos`]. Use
    /// [`Rasterizer::draw_mut`] to have it written to `fuzzpos` directly (sheet 11 section 11).
    pub fn draw(
        &mut self,
        target: &mut DrawTarget<'_>,
        model: &PreparedViewmodel,
        params: &ViewmodelDrawParams<'_>,
    ) -> DrawStats {
        let (stats, pos) = self.draw_impl(target, model, params);
        self.last_fuzzpos = pos;
        stats
    }

    /// Like [`Rasterizer::draw`], and with [`LightParams::Fuzz`] also stores the advanced fuzz
    /// position into `fuzzpos`, keeping the effect continuous with the world's fuzz.
    pub fn draw_mut(
        &mut self,
        target: &mut DrawTarget<'_>,
        model: &PreparedViewmodel,
        params: &mut ViewmodelDrawParams<'_>,
    ) -> DrawStats {
        let (stats, pos) = self.draw_impl(target, model, params);
        self.last_fuzzpos = pos;
        if let (Some(p), LightParams::Fuzz { fuzzpos, .. }) = (pos, &mut params.light) {
            **fuzzpos = p;
        }
        stats
    }

    /// The fuzz position reached by the last draw with [`LightParams::Fuzz`] (`None` after a
    /// draw with other lighting).
    pub fn last_fuzzpos(&self) -> Option<usize> {
        self.last_fuzzpos
    }

    fn draw_impl(
        &mut self,
        target: &mut DrawTarget<'_>,
        model: &PreparedViewmodel,
        params: &ViewmodelDrawParams<'_>,
    ) -> (DrawStats, Option<usize>) {
        let started = Instant::now();
        let mut stats = DrawStats::default();
        let pitch = target.pitch;
        if pitch == 0 {
            return (stats, None);
        }
        let rows = (target.pixels.len() / pitch) as i64;
        let x0 = i64::from(target.view_x).max(0);
        let y0 = i64::from(target.view_y).max(0);
        let x1 = (i64::from(target.view_x) + i64::from(target.view_w)).min(pitch as i64);
        let y1 = (i64::from(target.view_y) + i64::from(target.view_h)).min(rows);
        let half_fov = (params.hfov_deg * 0.5).to_radians();
        let fx = (target.view_w as f32 * 0.5) / half_fov.tan();
        let fy = fx * target.pixel_aspect;
        if x0 >= x1
            || y0 >= y1
            || !(fx.is_finite() && fx > 0.0 && fy.is_finite() && fy > 0.0)
            || !(target.center_x.is_finite() && target.center_y.is_finite())
            || params.pal.colormaps.is_empty()
        {
            return (stats, None);
        }
        let view = View {
            fx,
            fy,
            cx: target.view_x as f32 + target.center_x,
            cy: target.view_y as f32 + target.center_y,
            x0: x0 as i32,
            y0: y0 as i32,
            x1: x1 as i32,
            y1: y1 as i32,
        };
        let ww = (x1 - x0) as usize;
        let wh = (y1 - y0) as usize;
        let fuzz = matches!(params.light, LightParams::Fuzz { .. });
        if self.zbuf.len() < ww * wh {
            self.zbuf.resize(ww * wh, 0.0);
        }
        self.zbuf[..ww * wh].fill(0.0);
        if fuzz {
            if self.cover.len() < ww * wh {
                self.cover.resize(ww * wh, 0);
            }
            self.cover[..ww * wh].fill(0);
        }

        let (sector_base, directional) = match params.light {
            LightParams::Sector {
                colormap_level,
                directional,
            } => (colormap_level, directional),
            _ => (0, false),
        };
        skin_vertices(
            model,
            &params.pose.skin,
            &mut self.skinned_p,
            &mut self.skinned_n,
        );
        let l = light_dir();
        let nv = self.skinned_p.len();
        self.ndl.resize(nv, 0.0);
        self.levels.resize(nv, [0.0; 2]);
        self.proj.resize(nv, Projected::default());
        for i in 0..nv {
            let ndl = self.skinned_n[i].dot(l);
            self.ndl[i] = ndl;
            self.levels[i] = [
                vertex_level(sector_base, directional, ndl),
                vertex_level(sector_base, directional, -ndl),
            ];
            let p = self.skinned_p[i];
            self.proj[i] = match (p.is_finite() && p.x >= NEAR_PLANE)
                .then(|| view.project(p))
                .flatten()
            {
                Some((x, y, inv_x)) => Projected {
                    x,
                    y,
                    inv_x,
                    ok: true,
                },
                None => Projected::default(),
            };
        }

        let mode = match &params.light {
            LightParams::Sector { .. } => PixMode::Sector(&params.pal.colormaps),
            LightParams::Fixed { colormap } => {
                PixMode::Fixed(params.pal.colormap(usize::from(*colormap)))
            }
            LightParams::Fuzz { .. } => PixMode::Cover,
        };
        let mut buf = Buffers {
            pixels: target.pixels,
            pitch,
            zbuf: &mut self.zbuf[..ww * wh],
            cover: &mut self.cover[..],
        };
        let ctx = TriCtx {
            view: &view,
            model,
            skinned_p: &self.skinned_p,
            ndl: &self.ndl,
            levels: &self.levels,
            proj: &self.proj,
            sector_base,
            directional,
            mode,
        };
        for surf in &model.surfaces {
            if !role_visible(surf.role, params.visibility) {
                continue;
            }
            ctx.draw_surface(surf, &mut buf, &mut stats);
        }

        let mut fuzz_out = None;
        if let LightParams::Fuzz {
            fuzzoffset,
            fuzzpos,
        } = &params.light
        {
            let mut pos = **fuzzpos;
            stats.pixels = apply_fuzz(
                buf.pixels,
                pitch,
                (x0 as usize, y0 as usize, ww, wh),
                (
                    target.view_y.max(0) as usize,
                    (target.view_y + target.view_h - 1).max(0) as usize,
                ),
                &buf.cover[..ww * wh],
                params.pal.colormap(FUZZ_COLORMAP),
                fuzzoffset,
                &mut pos,
            );
            fuzz_out = Some(pos);
        }
        stats.micros = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
        log::trace!(target: "dmw::raster", "{stats:?}");
        (stats, fuzz_out)
    }
}

/// Read-only state shared by every triangle of one draw.
struct TriCtx<'a> {
    view: &'a View,
    model: &'a PreparedViewmodel,
    skinned_p: &'a [Vec3],
    ndl: &'a [f32],
    levels: &'a [[f32; 2]],
    proj: &'a [Projected],
    sector_base: u8,
    directional: bool,
    mode: PixMode<'a>,
}

impl TriCtx<'_> {
    fn draw_surface(&self, surf: &PreparedSurface, buf: &mut Buffers<'_>, stats: &mut DrawStats) {
        let model = self.model;
        let (mips, flat) = match surf.texture {
            Some(t) => (Some(&model.textures[t as usize].mips), surf.flat_index),
            None => (None, surf.flat_index),
        };
        for t in surf.first_tri..surf.first_tri + surf.tri_count {
            stats.triangles_in += 1;
            let base = t as usize * 3;
            let idx = [
                model.indices[base] as usize,
                model.indices[base + 1] as usize,
                model.indices[base + 2] as usize,
            ];
            let uv = |k: usize| model.uvs[idx[k]];

            // Fast path: all three vertices projected. Facing comes from the snapped screen
            // winding (a counter-clockwise front face appears clockwise on the y-down screen,
            // i.e. with a negative doubled area), which needs no eye-space positions.
            let pr = [self.proj[idx[0]], self.proj[idx[1]], self.proj[idx[2]]];
            if pr[0].ok && pr[1].ok && pr[2].ok {
                let area2 = (pr[1].x - pr[0].x) * (pr[2].y - pr[0].y)
                    - (pr[2].x - pr[0].x) * (pr[1].y - pr[0].y);
                if area2 == 0 {
                    continue;
                }
                let front = match model.winding {
                    Winding::CounterClockwise => area2 < 0,
                    Winding::Clockwise => area2 > 0,
                };
                if surf.cull_back && !front {
                    continue;
                }
                let side = usize::from(!front);
                let sv = |k: usize| {
                    let [u, v] = uv(k);
                    ScreenVert {
                        x: pr[k].x,
                        y: pr[k].y,
                        inv_x: pr[k].inv_x,
                        u_x: u * pr[k].inv_x,
                        v_x: v * pr[k].inv_x,
                        u,
                        v,
                        level: self.levels[idx[k]][side],
                    }
                };
                let tri = [sv(0), sv(1), sv(2)];
                let area = model.tri_texel_area[t as usize];
                let texels = |a2| self.texels(mips, flat, &tri, Some(area), a2);
                stats.triangles_drawn += 1;
                stats.pixels += raster_triangle(self.view, buf, tri, texels, self.mode);
                continue;
            }

            // Slow path: a vertex is behind the near plane (or unprojectable): clip in eye space.
            let p = [
                self.skinned_p[idx[0]],
                self.skinned_p[idx[1]],
                self.skinned_p[idx[2]],
            ];
            if !(p[0].is_finite() && p[1].is_finite() && p[2].is_finite()) {
                continue;
            }
            if p[0].x < NEAR_PLANE && p[1].x < NEAR_PLANE && p[2].x < NEAR_PLANE {
                continue;
            }
            // Facing: geometric normal of the CCW reading points at the eye (origin).
            let d = (p[1] - p[0]).cross(p[2] - p[0]).dot(p[0]);
            if d == 0.0 || d.is_nan() {
                continue;
            }
            let front = match model.winding {
                Winding::CounterClockwise => d < 0.0,
                Winding::Clockwise => d > 0.0,
            };
            if surf.cull_back && !front {
                continue;
            }
            let sign = if front { 1.0 } else { -1.0 };
            let level = |ndl: f32| vertex_level(self.sector_base, self.directional, ndl);
            let cv = |k: usize| {
                let [u, v] = uv(k);
                ClipVert {
                    p: p[k],
                    u,
                    v,
                    ndl: sign * self.ndl[idx[k]],
                }
            };
            let (poly, n) = clip_near(&[cv(0), cv(1), cv(2)]);
            if n < 3 {
                continue;
            }
            let mut sv = [ScreenVert::default(); 4];
            let mut ok = true;
            for k in 0..n {
                match self.view.project(poly[k].p) {
                    Some((x, y, inv_x)) => {
                        sv[k] = ScreenVert {
                            x,
                            y,
                            inv_x,
                            u_x: poly[k].u * inv_x,
                            v_x: poly[k].v * inv_x,
                            u: poly[k].u,
                            v: poly[k].v,
                            level: level(poly[k].ndl),
                        };
                    }
                    None => ok = false,
                }
            }
            if !ok {
                continue;
            }
            for k in 1..n - 1 {
                let tri = [sv[0], sv[k], sv[k + 1]];
                let texels = |a2| self.texels(mips, flat, &tri, None, a2);
                stats.triangles_drawn += 1;
                stats.pixels += raster_triangle(self.view, buf, tri, texels, self.mode);
            }
        }
    }

    /// Picks the mip for a projected triangle (`texel_area` = precomputed mip-0 area, or
    /// `None` to compute it from the vertices' uvs; `area2` = doubled screen area in
    /// fixed-point units).
    fn texels<'m>(
        &self,
        mips: Option<&'m Vec<IndexedMip>>,
        flat: u8,
        tri: &[ScreenVert; 3],
        texel_area: Option<f32>,
        area2: i64,
    ) -> Texels<'m> {
        let Some(mips) = mips else {
            return Texels::Flat(flat);
        };
        let m0 = &mips[0];
        let texel_area = texel_area.unwrap_or_else(|| {
            let cross = (tri[1].u - tri[0].u) * (tri[2].v - tri[0].v)
                - (tri[2].u - tri[0].u) * (tri[1].v - tri[0].v);
            0.5 * cross.abs() * m0.width as f32 * m0.height as f32
        });
        let screen_area = 0.5 * area2.unsigned_abs() as f32 / (SUB * SUB) as f32;
        Texels::Mip(&mips[select_mip(texel_area, screen_area, mips.len())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_selection() {
        assert_eq!(select_mip(64.0 * 64.0, 64.0 * 64.0, 7), 0);
        assert_eq!(select_mip(64.0 * 64.0, 32.0 * 32.0, 7), 1);
        assert_eq!(select_mip(64.0 * 64.0, 1.0, 7), 6);
        assert_eq!(select_mip(64.0 * 64.0, 1e-6, 7), 6);
        assert_eq!(select_mip(1.0, 100.0, 7), 0);
        assert_eq!(select_mip(0.0, 100.0, 7), 0);
    }

    #[test]
    fn clip_keeps_front_part_only() {
        let v = |x: f32| ClipVert {
            p: Vec3::new(x, 0.0, x),
            ..ClipVert::default()
        };
        let (_, n) = clip_near(&[v(-1.0), v(-2.0), v(-3.0)]);
        assert_eq!(n, 0);
        let (poly, n) = clip_near(&[v(2.0), v(-2.0), v(3.0)]);
        assert_eq!(n, 4);
        assert!(poly[..n].iter().all(|c| c.p.x >= NEAR_PLANE));
        let (poly, n) = clip_near(&[v(2.0), v(-2.0), v(-3.0)]);
        assert_eq!(n, 3);
        assert!(poly[..n].iter().all(|c| c.p.x >= NEAR_PLANE));
    }
}
