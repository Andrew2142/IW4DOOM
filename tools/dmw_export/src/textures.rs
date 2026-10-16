// SPDX-License-Identifier: GPL-3.0-or-later
//! Material colour maps → RGBA8 textures (sheet 04 §5.7), following iw4L
//! `assets/src/gltf_export.rs` (single `TS_COLOR_MAP` binding → `decoded_image_top_level_rgba8`).

use std::collections::BTreeMap;

use asset_material::{MaterialDefinitions, TS_COLOR_MAP};
use dmw_format::RgbaImage;

/// Largest exported texture edge (§5.7.2).
pub const MAX_TEXTURE_EDGE: u32 = 1024;

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
    (c * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// One 2× box-filter step: RGB averaged in linear light, alpha averaged directly. An odd edge
/// keeps its last row/column (the box covers what exists).
pub fn halve(image: &RgbaImage) -> RgbaImage {
    let w = (image.width / 2).max(1);
    let h = (image.height / 2).max(1);
    let mut out = RgbaImage::new(w, h, [0; 4]);
    for y in 0..h {
        for x in 0..w {
            let mut rgb = [0.0f32; 3];
            let mut alpha = 0.0f32;
            let mut n = 0.0f32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = x * 2 + dx;
                    let sy = y * 2 + dy;
                    if sx >= image.width || sy >= image.height {
                        continue;
                    }
                    let p = image.get(sx, sy);
                    for c in 0..3 {
                        rgb[c] += srgb_to_linear(p[c]);
                    }
                    alpha += f32::from(p[3]);
                    n += 1.0;
                }
            }
            out.set(
                x,
                y,
                [
                    linear_to_srgb(rgb[0] / n),
                    linear_to_srgb(rgb[1] / n),
                    linear_to_srgb(rgb[2] / n),
                    (alpha / n + 0.5).clamp(0.0, 255.0) as u8,
                ],
            );
        }
    }
    out
}

/// Halves `image` until both edges are ≤ `max_edge`.
pub fn downscale_to(mut image: RgbaImage, max_edge: u32) -> RgbaImage {
    while image.width > max_edge || image.height > max_edge {
        image = halve(&image);
    }
    image
}

/// RGBA8 bytes → `RgbaImage`.
pub fn image_from_rgba(width: u32, height: u32, bytes: &[u8]) -> Option<RgbaImage> {
    let n = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
    if bytes.len() != n.checked_mul(4)? {
        return None;
    }
    Some(RgbaImage {
        width,
        height,
        pixels: bytes
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
    })
}

/// One exported texture.
#[derive(Debug, Clone)]
pub struct ExportedTexture {
    pub image: RgbaImage,
    pub orig_w: u32,
    pub orig_h: u32,
    pub source: String,
}

/// Per-weapon texture set, deduplicated by catalog image index.
#[derive(Debug, Default)]
pub struct TextureSet {
    by_image: BTreeMap<usize, Option<u16>>,
    pub textures: Vec<ExportedTexture>,
    pub warnings: Vec<String>,
}

/// Decodes catalog image `image_index` to top-level RGBA8.
pub fn decode_catalog_image(defs: &MaterialDefinitions, image_index: usize) -> Result<(String, RgbaImage), String> {
    let source = defs
        .images
        .get(image_index)
        .ok_or_else(|| format!("image index {image_index} outside the catalog"))?;
    let name = source.name.as_str().to_owned();
    let decoded = source
        .decoded
        .as_ref()
        .ok_or_else(|| format!("image '{name}' was not decoded by the load (MaterialImageUnavailable)"))?;
    let (w, h, pixels) = asset_material::decoded_image_top_level_rgba8(decoded)
        .map_err(|e| format!("image '{name}': {e}"))?;
    let image = image_from_rgba(w, h, &pixels).ok_or_else(|| format!("image '{name}': bad size"))?;
    Ok((name, image))
}

impl TextureSet {
    /// Texture index for material row `material` (its single colour map), or `None` with a
    /// warning naming the reason.
    pub fn for_material(&mut self, defs: &MaterialDefinitions, material: usize) -> Option<u16> {
        let m = defs.materials.get(material)?;
        let mut colour = m.textures.iter().filter(|b| b.semantic == TS_COLOR_MAP);
        let Some(binding) = colour.next() else {
            self.warnings.push(format!(
                "material '{}': MaterialColorMapUnavailable",
                m.name.as_str()
            ));
            return None;
        };
        if colour.next().is_some() {
            self.warnings.push(format!(
                "material '{}': MaterialColorMapAmbiguous",
                m.name.as_str()
            ));
            return None;
        }
        let Some(image_index) = binding.image else {
            self.warnings.push(format!(
                "material '{}': colour map has no image",
                m.name.as_str()
            ));
            return None;
        };
        if let Some(done) = self.by_image.get(&image_index) {
            return *done;
        }
        let result = match decode_catalog_image(defs, image_index) {
            Ok((source, image)) => {
                let (orig_w, orig_h) = (image.width, image.height);
                let image = downscale_to(image, MAX_TEXTURE_EDGE);
                let n = u16::try_from(self.textures.len()).ok();
                if n.is_some() {
                    self.textures.push(ExportedTexture {
                        image,
                        orig_w,
                        orig_h,
                        source,
                    });
                }
                n
            }
            Err(e) => {
                self.warnings.push(format!("material '{}': {e}", m.name.as_str()));
                None
            }
        };
        self.by_image.insert(image_index, result);
        result
    }
}
