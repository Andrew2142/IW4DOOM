// SPDX-License-Identifier: GPL-3.0-or-later
//! 2D indexed blitters (sheet 11 section 5.7): Doom patches (muzzle flash) and quantized
//! images (scope overlay, sheet 12). Both use nearest sampling, clip to the target's view
//! window and never fail: malformed patch data stops the blit at the first bad byte.

use crate::DrawTarget;
use crate::quantize::IndexedMip;

/// Destination rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlitRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width (≤ 0 draws nothing).
    pub w: i32,
    /// Height (≤ 0 draws nothing).
    pub h: i32,
}

/// The target's view window clipped to its buffer: [x0, x1) x [y0, y1).
fn window(target: &DrawTarget<'_>) -> Option<(i32, i32, i32, i32)> {
    if target.pitch == 0 {
        return None;
    }
    let rows = (target.pixels.len() / target.pitch).min(i32::MAX as usize) as i32;
    let cols = target.pitch.min(i32::MAX as usize) as i32;
    let x0 = target.view_x.max(0);
    let y0 = target.view_y.max(0);
    let x1 = target.view_x.saturating_add(target.view_w).min(cols);
    let y1 = target.view_y.saturating_add(target.view_h).min(rows);
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

fn read_i16(b: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// Draws a Doom patch lump (header `width, height, leftoffset, topoffset` as i16, then one u32
/// column offset per column, columns as posts `topdelta, length, pad, data[length], pad`
/// ended by `0xFF`), centred at screen position `(cx, cy)` (the patch offsets are ignored),
/// scaled by `scale` with nearest sampling, each texel mapped through `colormap`.
pub fn blit_doom_patch(
    target: &mut DrawTarget<'_>,
    patch_lump: &[u8],
    cx: f32,
    cy: f32,
    scale: f32,
    colormap: &[u8; 256],
) {
    let Some((wx0, wy0, wx1, wy1)) = window(target) else {
        return;
    };
    let (Some(pw), Some(ph)) = (read_i16(patch_lump, 0), read_i16(patch_lump, 2)) else {
        return;
    };
    if pw <= 0
        || ph <= 0
        || !(scale.is_finite() && scale > 0.0)
        || !cx.is_finite()
        || !cy.is_finite()
    {
        return;
    }
    let (pw, ph) = (i32::from(pw), i32::from(ph));
    let left = cx - pw as f32 * scale * 0.5;
    let top = cy - ph as f32 * scale * 0.5;
    let dx0 = (left.floor() as i32).max(wx0);
    let dx1 = ((left + pw as f32 * scale).ceil() as i32).min(wx1);
    let pitch = target.pitch;
    for dx in dx0..dx1 {
        let col = ((dx as f32 + 0.5 - left) / scale).floor() as i32;
        if !(0..pw).contains(&col) {
            continue;
        }
        let Some(mut at) = read_u32(patch_lump, 8 + 4 * col as usize).map(|o| o as usize) else {
            return;
        };
        // Walk the posts of this column.
        loop {
            let Some(&topdelta) = patch_lump.get(at) else {
                return;
            };
            if topdelta == 0xFF {
                break;
            }
            let Some(&len) = patch_lump.get(at + 1) else {
                return;
            };
            let data = at + 3;
            if data + usize::from(len) > patch_lump.len() {
                return;
            }
            // Destination rows whose sampled source row lies in [topdelta, topdelta + len).
            let src0 = f32::from(topdelta);
            let src1 = src0 + f32::from(len);
            let ry0 = ((top + src0 * scale - 0.5).ceil() as i32).max(wy0);
            let ry1 = ((top + src1 * scale - 0.5).ceil() as i32).min(wy1);
            for dy in ry0..ry1 {
                let row = ((dy as f32 + 0.5 - top) / scale).floor() as i32 - i32::from(topdelta);
                if !(0..i32::from(len)).contains(&row) || row + i32::from(topdelta) >= ph {
                    continue;
                }
                let texel = patch_lump[data + row as usize];
                target.pixels[dy as usize * pitch + dx as usize] = colormap[usize::from(texel)];
            }
            at = data + usize::from(len) + 1;
        }
    }
}

/// Draws an indexed image stretched over `rect` (nearest sampling), skipping texels whose
/// `opaque` flag is 0, each texel mapped through `colormap`.
pub fn blit_indexed(
    target: &mut DrawTarget<'_>,
    tex: &IndexedMip,
    rect: BlitRect,
    colormap: &[u8; 256],
) {
    let Some((wx0, wy0, wx1, wy1)) = window(target) else {
        return;
    };
    let n = tex.width as usize * tex.height as usize;
    if rect.w <= 0 || rect.h <= 0 || n == 0 || tex.index.len() < n || tex.opaque.len() < n {
        return;
    }
    let x0 = rect.x.max(wx0);
    let y0 = rect.y.max(wy0);
    let x1 = rect.x.saturating_add(rect.w).min(wx1);
    let y1 = rect.y.saturating_add(rect.h).min(wy1);
    let pitch = target.pitch;
    for dy in y0..y1 {
        let ty = (i64::from(dy - rect.y) * i64::from(tex.height) / i64::from(rect.h)) as u32;
        for dx in x0..x1 {
            let tx = (i64::from(dx - rect.x) * i64::from(tex.width) / i64::from(rect.w)) as u32;
            let (index, opaque) = tex.texel(tx, ty);
            if opaque {
                target.pixels[dy as usize * pitch + dx as usize] = colormap[usize::from(index)];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(buf: &mut [u8]) -> DrawTarget<'_> {
        DrawTarget {
            pixels: buf,
            pitch: 8,
            view_x: 0,
            view_y: 0,
            view_w: 8,
            view_h: 6,
            center_x: 4.0,
            center_y: 3.0,
            pixel_aspect: 1.0,
        }
    }

    /// 2x3 patch: column 0 = one post rows 0..3 (values 1, 2, 3); column 1 = post at row 1,
    /// length 1 (value 9).
    fn patch() -> Vec<u8> {
        let mut p = Vec::new();
        for v in [2i16, 3, 0, 0] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let c0 = 16u32;
        let c1 = c0 + 3 + 3 + 1 + 1;
        p.extend_from_slice(&c0.to_le_bytes());
        p.extend_from_slice(&c1.to_le_bytes());
        p.extend_from_slice(&[0, 3, 0, 1, 2, 3, 0, 0xFF]);
        p.extend_from_slice(&[1, 1, 0, 9, 0, 0xFF]);
        p
    }

    #[test]
    fn patch_blit_places_columns_and_posts() {
        let mut ident = [0u8; 256];
        for (i, v) in ident.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut buf = vec![0u8; 8 * 8]; // rows 6, 7 are outside the view window
        blit_doom_patch(&mut target(&mut buf), &patch(), 4.0, 3.0, 1.0, &ident);
        // centred: left = 3, top = 1.5 -> rows 2..4 (pixel centres 2.5, 3.5, 4.5 -> src 1, 2, 3?)
        let at = |x: usize, y: usize| buf[y * 8 + x];
        let col0: Vec<u8> = (0..8).map(|y| at(3, y)).collect();
        let col1: Vec<u8> = (0..8).map(|y| at(4, y)).collect();
        assert_eq!(col0, vec![0, 1, 2, 3, 0, 0, 0, 0]);
        assert_eq!(col1, vec![0, 0, 9, 0, 0, 0, 0, 0]);

        // Scale 2 at the bottom edge clips to the view window (rows >= 6 untouched).
        let mut buf = vec![0u8; 8 * 8];
        blit_doom_patch(&mut target(&mut buf), &patch(), 4.0, 5.0, 2.0, &ident);
        assert!(buf[48..].iter().all(|&v| v == 0));
        assert!(buf.contains(&1));

        // Truncated lump: no panic.
        let p = patch();
        for cut in 0..p.len() {
            let mut buf = vec![0u8; 64];
            blit_doom_patch(&mut target(&mut buf), &p[..cut], 4.0, 3.0, 1.0, &ident);
        }
    }

    #[test]
    fn indexed_blit_scales_and_skips_transparent() {
        let tex = IndexedMip {
            width: 2,
            height: 1,
            index: vec![5, 6],
            opaque: vec![1, 0],
        };
        let mut ident = [0u8; 256];
        for (i, v) in ident.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut buf = vec![0u8; 64];
        let rect = BlitRect {
            x: -2,
            y: 0,
            w: 8,
            h: 2,
        };
        blit_indexed(&mut target(&mut buf), &tex, rect, &ident);
        assert_eq!(&buf[0..8], &[5, 5, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&buf[8..16], &[5, 5, 0, 0, 0, 0, 0, 0]);
        assert!(buf[16..].iter().all(|&v| v == 0));
    }
}
