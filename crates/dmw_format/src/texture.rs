// SPDX-License-Identifier: GPL-3.0-or-later
//! Textures: RGBA8 images and the PNG helpers (sheet 03 section 5.7).

use std::io::Cursor;
use std::path::Path;

use crate::error::PackError;

/// Largest accepted texture edge in pixels.
pub const MAX_TEXTURE_DIM: u32 = 4096;

/// An 8-bit RGBA image, row-major, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height` pixels as `[r, g, b, a]`.
    pub pixels: Vec<[u8; 4]>,
}

impl RgbaImage {
    /// An image filled with `fill`.
    pub fn new(width: u32, height: u32, fill: [u8; 4]) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill; width as usize * height as usize],
        }
    }

    /// Pixel at (`x`, `y`); out of range returns transparent black.
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        if x < self.width && y < self.height {
            self.pixels
                .get(y as usize * self.width as usize + x as usize)
                .copied()
                .unwrap_or([0; 4])
        } else {
            [0; 4]
        }
    }

    /// Sets the pixel at (`x`, `y`); out of range is ignored.
    pub fn set(&mut self, x: u32, y: u32, value: [u8; 4]) {
        if x < self.width
            && y < self.height
            && let Some(p) = self
                .pixels
                .get_mut(y as usize * self.width as usize + x as usize)
        {
            *p = value;
        }
    }
}

/// Reads a PNG file. 8-bit RGBA or RGB is used directly; 16-bit, grey and palette images are
/// converted to RGBA8. Images larger than 4096 by 4096 are rejected.
pub fn load_png(path: &Path) -> Result<RgbaImage, PackError> {
    let bytes =
        std::fs::read(path).map_err(|e| PackError::from_io(path, &path.to_string_lossy(), e))?;
    decode_png(&bytes, &path.display().to_string())
}

/// Decodes PNG bytes; `label` names the source in errors.
pub fn decode_png(bytes: &[u8], label: &str) -> Result<RgbaImage, PackError> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| PackError::invalid(format!("{label}: not a decodable PNG: {e}")))?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if width == 0 || height == 0 || width > MAX_TEXTURE_DIM || height > MAX_TEXTURE_DIM {
        return Err(PackError::invalid(format!(
            "{label}: texture is {width}x{height}, limits are 1..={MAX_TEXTURE_DIM}"
        )));
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| PackError::invalid(format!("{label}: not a decodable PNG: {e}")))?;
    let data = &buf[..frame.buffer_size()];
    let pixels: Vec<[u8; 4]> = match (frame.color_type, frame.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => data
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => data
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => data
            .chunks_exact(2)
            .map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        (png::ColorType::Grayscale, png::BitDepth::Eight) => {
            data.iter().map(|&g| [g, g, g, 255]).collect()
        }
        (color, depth) => {
            return Err(PackError::invalid(format!(
                "{label}: unsupported PNG layout {color:?}/{depth:?} after expansion"
            )));
        }
    };
    if pixels.len() != width as usize * height as usize {
        return Err(PackError::invalid(format!(
            "{label}: PNG pixel count mismatch"
        )));
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

/// Encodes an image as an 8-bit RGBA PNG. The output depends only on the pixels, so it is
/// reproducible.
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, PackError> {
    if image.pixels.len() != image.width as usize * image.height as usize {
        return Err(PackError::invalid(
            "image pixel count does not match its size",
        ));
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| PackError::format(format!("png encode: {e}")))?;
        let flat: Vec<u8> = image.pixels.iter().flatten().copied().collect();
        writer
            .write_image_data(&flat)
            .map_err(|e| PackError::format(format!("png encode: {e}")))?;
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h, [0; 4]);
        for y in 0..h {
            for x in 0..w {
                img.set(
                    x,
                    y,
                    [(x * 7) as u8, (y * 5) as u8, (x + y) as u8, 255 - (x as u8)],
                );
            }
        }
        img
    }

    #[test]
    fn rgba_round_trip_and_reproducible() {
        let img = gradient(17, 9);
        let a = encode_png(&img).unwrap();
        let b = encode_png(&img).unwrap();
        assert_eq!(a, b);
        assert_eq!(decode_png(&a, "t").unwrap(), img);
    }

    fn encode_raw(
        color: png::ColorType,
        depth: png::BitDepth,
        w: u32,
        h: u32,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(depth);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn rgb_grey_and_sixteen_bit_are_converted() {
        let rgb = encode_raw(
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            2,
            1,
            &[1, 2, 3, 4, 5, 6],
        );
        assert_eq!(
            decode_png(&rgb, "rgb").unwrap().pixels,
            vec![[1, 2, 3, 255], [4, 5, 6, 255]]
        );
        let grey = encode_raw(
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            2,
            1,
            &[9, 200],
        );
        assert_eq!(
            decode_png(&grey, "g").unwrap().pixels,
            vec![[9, 9, 9, 255], [200, 200, 200, 255]]
        );
        let ga = encode_raw(
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            1,
            1,
            &[7, 99],
        );
        assert_eq!(decode_png(&ga, "ga").unwrap().pixels, vec![[7, 7, 7, 99]]);
        // 16-bit RGBA keeps the high byte of each sample.
        let wide = encode_raw(
            png::ColorType::Rgba,
            png::BitDepth::Sixteen,
            1,
            1,
            &[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xff, 0xff],
        );
        assert_eq!(
            decode_png(&wide, "w").unwrap().pixels,
            vec![[0x12, 0x56, 0x9a, 0xff]]
        );
    }

    #[test]
    fn palette_png_is_expanded() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 1);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![10, 20, 30, 40, 50, 60]);
            enc.set_trns(vec![128, 255]);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[0, 1]).unwrap();
        }
        assert_eq!(
            decode_png(&out, "p").unwrap().pixels,
            vec![[10, 20, 30, 128], [40, 50, 60, 255]]
        );
    }

    #[test]
    fn oversize_and_garbage_are_invalid() {
        let big = encode_raw(
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            4097,
            1,
            &vec![0u8; 4097],
        );
        assert!(matches!(
            decode_png(&big, "big"),
            Err(PackError::Invalid(_))
        ));
        assert!(matches!(
            decode_png(b"not a png", "x"),
            Err(PackError::Invalid(_))
        ));
        let ok = encode_raw(
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            4096,
            1,
            &vec![0u8; 4096],
        );
        assert!(decode_png(&ok, "edge").is_ok());
    }

    #[test]
    fn get_set_are_bounds_checked() {
        let mut img = RgbaImage::new(2, 2, [1, 1, 1, 1]);
        img.set(5, 5, [9; 4]);
        assert_eq!(img.get(5, 5), [0; 4]);
        img.set(1, 1, [9; 4]);
        assert_eq!(img.get(1, 1), [9; 4]);
        assert_eq!(img.get(0, 1), [1; 4]);
    }
}
