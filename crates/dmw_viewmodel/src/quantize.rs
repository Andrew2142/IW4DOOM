// SPDX-License-Identifier: GPL-3.0-or-later
//! RGBA8 → palette-indexed textures with a mip chain and optional ordered dither
//! (sheet 11 section 5.2).

use dmw_format::{AlphaMode, RgbaImage};

use crate::PaletteTables;

/// Colour → index conversion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dither {
    /// Plain nearest colour.
    None,
    /// 4x4 Bayer ordered dither in texture space (stable under motion).
    #[default]
    Bayer4,
}

/// A quantized texture: mip 0 is the source size, the last mip is 1x1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedTexture {
    /// Mip levels, largest first.
    pub mips: Vec<IndexedMip>,
}

/// One mip level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedMip {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Palette index per texel, row-major, top row first.
    pub index: Vec<u8>,
    /// 1 where the texel is drawn, 0 where the alpha test discards it.
    pub opaque: Vec<u8>,
}

impl IndexedMip {
    /// Texel `(x, y)` as `(index, opaque)`; coordinates must be in range.
    #[inline]
    pub fn texel(&self, x: u32, y: u32) -> (u8, bool) {
        let i = y as usize * self.width as usize + x as usize;
        (self.index[i], self.opaque[i] != 0)
    }
}

/// 4x4 Bayer matrix, values 0..15.
const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Next mip level: 2x2 box filter in linear light, alpha averaged; floor division, last
/// row/column clamped for odd sizes.
fn downsample(src: &RgbaImage, to_linear: &[f32; 256]) -> RgbaImage {
    let w = (src.width / 2).max(1);
    let h = (src.height / 2).max(1);
    let mut out = RgbaImage::new(w, h, [0; 4]);
    for y in 0..h {
        for x in 0..w {
            let xs = [(2 * x).min(src.width - 1), (2 * x + 1).min(src.width - 1)];
            let ys = [(2 * y).min(src.height - 1), (2 * y + 1).min(src.height - 1)];
            let mut lin = [0.0f32; 3];
            let mut alpha = 0u32;
            for &sy in &ys {
                for &sx in &xs {
                    let p = src.get(sx, sy);
                    for k in 0..3 {
                        lin[k] += to_linear[usize::from(p[k])];
                    }
                    alpha += u32::from(p[3]);
                }
            }
            out.set(
                x,
                y,
                [
                    linear_to_srgb(lin[0] / 4.0),
                    linear_to_srgb(lin[1] / 4.0),
                    linear_to_srgb(lin[2] / 4.0),
                    ((alpha + 2) / 4) as u8,
                ],
            );
        }
    }
    out
}

fn is_opaque(alpha: AlphaMode, a: u8) -> bool {
    match alpha {
        AlphaMode::Opaque => true,
        AlphaMode::Test { cutoff } => f32::from(a) >= cutoff * 255.0,
        AlphaMode::Blend => a >= 128,
    }
}

fn index_level(
    img: &RgbaImage,
    pal: &PaletteTables,
    dither: Dither,
    alpha: AlphaMode,
) -> IndexedMip {
    let n = img.width as usize * img.height as usize;
    let mut index = Vec::with_capacity(n);
    let mut opaque = Vec::with_capacity(n);
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.get(x, y);
            let rgb = match dither {
                Dither::None => [p[0], p[1], p[2]],
                Dither::Bayer4 => {
                    let b = f32::from(BAYER4[(y & 3) as usize][(x & 3) as usize]);
                    let off = (b - 7.5) * 4.0;
                    let ch = |c: u8| (f32::from(c) + off).round().clamp(0.0, 255.0) as u8;
                    [ch(p[0]), ch(p[1]), ch(p[2])]
                }
            };
            index.push(pal.nearest_fast(rgb));
            opaque.push(u8::from(is_opaque(alpha, p[3])));
        }
    }
    IndexedMip {
        width: img.width,
        height: img.height,
        index,
        opaque,
    }
}

/// Quantizes `image` to the palette, building the full mip chain (down to 1x1) in RGB before
/// quantization. An empty or inconsistent image yields a single 1x1 transparent mip.
pub fn quantize(
    image: &RgbaImage,
    pal: &PaletteTables,
    dither: Dither,
    alpha: AlphaMode,
) -> IndexedTexture {
    if image.width == 0
        || image.height == 0
        || image.pixels.len() != image.width as usize * image.height as usize
    {
        return IndexedTexture {
            mips: vec![IndexedMip {
                width: 1,
                height: 1,
                index: vec![0],
                opaque: vec![0],
            }],
        };
    }
    let mut to_linear = [0.0f32; 256];
    for (i, l) in to_linear.iter_mut().enumerate() {
        *l = srgb_to_linear(i as u8);
    }
    let mut mips = vec![index_level(image, pal, dither, alpha)];
    let mut level = downsample_if_needed(image, &to_linear);
    while let Some(img) = level {
        mips.push(index_level(&img, pal, dither, alpha));
        level = downsample_if_needed(&img, &to_linear);
    }
    IndexedTexture { mips }
}

fn downsample_if_needed(img: &RgbaImage, to_linear: &[f32; 256]) -> Option<RgbaImage> {
    (img.width > 1 || img.height > 1).then(|| downsample(img, to_linear))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trip_is_exact() {
        for c in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(c)), c);
        }
    }

    #[test]
    fn alpha_rules() {
        assert!(is_opaque(AlphaMode::Opaque, 0));
        assert!(is_opaque(AlphaMode::Test { cutoff: 0.5 }, 128));
        assert!(!is_opaque(AlphaMode::Test { cutoff: 0.5 }, 127));
        assert!(is_opaque(AlphaMode::Blend, 128));
        assert!(!is_opaque(AlphaMode::Blend, 127));
    }

    #[test]
    fn linear_light_average_of_black_and_white_is_not_mid_grey() {
        let mut img = RgbaImage::new(2, 1, [0, 0, 0, 255]);
        img.set(1, 0, [255, 255, 255, 255]);
        let mut to_linear = [0.0f32; 256];
        for (i, l) in to_linear.iter_mut().enumerate() {
            *l = srgb_to_linear(i as u8);
        }
        let d = downsample(&img, &to_linear);
        assert_eq!((d.width, d.height), (1, 1));
        // linear 0.5 -> sRGB 188
        assert_eq!(d.get(0, 0), [188, 188, 188, 255]);
    }
}
