// SPDX-License-Identifier: GPL-3.0-or-later
//! COLORMAP level selection, the directional term and Doom's fuzz (sheet 11 section 5.6).

use dmw_format::consts::LIGHT_SHADE_STEPS;
use glam::Vec3;

/// Eye-space light direction before normalization: from the viewer side, above, slightly left.
pub const LIGHT_DIR: [f32; 3] = [-0.35, 0.45, 0.82];

/// The colormap Doom's `R_DrawFuzzColumn` darkens through (`colormaps + 6*256`).
pub const FUZZ_COLORMAP: usize = 6;

/// Darkest light colormap.
const MAX_LEVEL: i32 = 31;

/// Normalized [`LIGHT_DIR`].
pub(crate) fn light_dir() -> Vec3 {
    Vec3::from(LIGHT_DIR).normalize()
}

/// Per-vertex COLORMAP level for sector lighting. `ndl` is `dot(n, L)` with the normal already
/// flipped for back faces.
#[inline]
pub(crate) fn vertex_level(base: u8, directional: bool, ndl: f32) -> f32 {
    let offset = if directional {
        ((0.5 - ndl.max(0.0)) * 2.0 * LIGHT_SHADE_STEPS as f32).round() as i32
    } else {
        0
    };
    (i32::from(base) + offset).clamp(0, MAX_LEVEL) as f32
}

/// Applies Doom's fuzz to every covered pixel of the window, column by column (the order
/// `R_DrawFuzzColumn` uses), reading the pixel at `offset` from the current one, darkening it
/// through `cmap`, and advancing `fuzzpos` per pixel (wrapping at `fuzzoffset.len()`). Like
/// Doom, the first and last rows of the view window are never fuzzed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_fuzz(
    pixels: &mut [u8],
    pitch: usize,
    (x0, y0, ww, wh): (usize, usize, usize, usize),
    (top_row, bottom_row): (usize, usize),
    cover: &[u8],
    cmap: &[u8; 256],
    fuzzoffset: &[i32],
    fuzzpos: &mut usize,
) -> u32 {
    let len = pixels.len() as i64;
    let table = fuzzoffset.len();
    if table > 0 {
        *fuzzpos %= table;
    }
    let mut written = 0;
    for x in 0..ww {
        for y in 0..wh {
            let sy = y0 + y;
            if cover[y * ww + x] == 0 || sy == top_row || sy == bottom_row {
                continue;
            }
            let dest = sy * pitch + x0 + x;
            let off = if table > 0 {
                i64::from(fuzzoffset[*fuzzpos])
            } else {
                0
            };
            let src = (dest as i64 + off).clamp(0, len - 1) as usize;
            pixels[dest] = cmap[usize::from(pixels[src])];
            if table > 0 {
                *fuzzpos = (*fuzzpos + 1) % table;
            }
            written += 1;
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directional_offsets_span_six_steps() {
        assert!((vertex_level(16, true, 1.0) - 10.0).abs() < 1e-6);
        assert!((vertex_level(16, true, 0.5) - 16.0).abs() < 1e-6);
        assert!((vertex_level(16, true, -1.0) - 22.0).abs() < 1e-6);
        assert!((vertex_level(16, false, 1.0) - 16.0).abs() < 1e-6);
        assert!((vertex_level(2, true, 1.0) - 0.0).abs() < 1e-6);
        assert!((vertex_level(30, true, 0.0) - 31.0).abs() < 1e-6);
    }
}
