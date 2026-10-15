// SPDX-License-Identifier: GPL-3.0-or-later
//! Golden-image tooling (sheet 13 section 5.4).
//!
//! Self-contained on purpose (no engine access, only the `png` crate): crate-level golden
//! tests in `crates/*/tests` can include this file with
//! `#[path = "../../../room/tests/support/golden.rs"] mod golden;` and use
//! [`assert_golden_rgb_in`] with their own `tests/golden` directory.
//!
//! Behaviour:
//! - exact match by default, or at most `max_differing_pixels` differing pixels;
//! - on mismatch the actual and a diff image are written to
//!   `target/test-artifacts/<name>.{actual,diff}.png` and the test fails with those paths;
//! - a missing golden fails the same way (the actual image is written);
//! - `BLESS=1` overwrites (or creates) the golden and prints its path instead of failing.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Width of a Doom frame.
pub const FRAME_W: usize = 320;
/// Height of a Doom frame.
pub const FRAME_H: usize = 200;

/// Pixel rectangle used to crop a frame before encoding or comparing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: usize,
    /// Top edge.
    pub y: usize,
    /// Width in pixels.
    pub w: usize,
    /// Height in pixels.
    pub h: usize,
}

impl Rect {
    /// Rectangle from its left/top corner and size.
    pub const fn new(x: usize, y: usize, w: usize, h: usize) -> Rect {
        Rect { x, y, w, h }
    }
}

/// One captured engine frame: `I_VideoBuffer` palette indices plus the palette that was active
/// when it was captured (gamma 0, so the colours equal the WAD's PLAYPAL entries).
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    /// `FRAME_W * FRAME_H` palette indices, row-major.
    pub indices: Vec<u8>,
    /// Active palette as RGB.
    pub palette: [[u8; 3]; 256],
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("size", &(FRAME_W, FRAME_H))
            .field("distinct_indices", &self.distinct_indices())
            .finish()
    }
}

impl Frame {
    /// Number of distinct palette indices used (a uniform frame has 1).
    pub fn distinct_indices(&self) -> usize {
        let mut seen = [false; 256];
        for &i in &self.indices {
            seen[i as usize] = true;
        }
        seen.iter().filter(|&&s| s).count()
    }

    /// Index at `(x, y)`.
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.indices[y * FRAME_W + x]
    }

    /// RGB bytes of `crop` (or the whole frame), plus the cropped size.
    pub fn to_rgb(&self, crop: Option<Rect>) -> (usize, usize, Vec<u8>) {
        let r = crop.unwrap_or(Rect::new(0, 0, FRAME_W, FRAME_H));
        assert!(
            r.w > 0 && r.h > 0 && r.x + r.w <= FRAME_W && r.y + r.h <= FRAME_H,
            "crop {r:?} is outside the {FRAME_W}x{FRAME_H} frame"
        );
        let mut rgb = Vec::with_capacity(r.w * r.h * 3);
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                rgb.extend_from_slice(&self.palette[self.at(x, y) as usize]);
            }
        }
        (r.w, r.h, rgb)
    }

    /// Expands the indices through the captured palette (gamma 0) and encodes an RGB PNG.
    /// The encoding is deterministic (no timestamps or ancillary chunks).
    pub fn to_png(&self, crop: Option<Rect>) -> Vec<u8> {
        let (w, h, rgb) = self.to_rgb(crop);
        encode_png(w, h, &rgb)
    }
}

/// Encodes 8-bit RGB pixels as a PNG.
pub fn encode_png(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    assert_eq!(rgb.len(), w * h * 3, "rgb buffer does not match {w}x{h}");
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("PNG header");
        writer.write_image_data(rgb).expect("PNG data");
    }
    out
}

/// Decodes a PNG to 8-bit RGB (alpha, if present, is dropped).
pub fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let mut reader = png::Decoder::new(bytes)
        .read_info()
        .map_err(|e| format!("PNG header: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("PNG data: {e}"))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("unsupported PNG bit depth {:?}", info.bit_depth));
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    let rgb = match info.color_type {
        png::ColorType::Rgb => data.to_vec(),
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        other => return Err(format!("unsupported PNG colour type {other:?}")),
    };
    Ok((w, h, rgb))
}

/// `room/tests/golden` (resolved from this crate's manifest directory).
pub fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// `<cargo target dir>/test-artifacts`, where failing comparisons leave their images.
pub fn artifact_dir() -> PathBuf {
    if let Some(t) = std::env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(t).join("test-artifacts");
    }
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let manifest = dir.join("Cargo.toml");
        if fs::read_to_string(&manifest).is_ok_and(|t| t.contains("[workspace]")) {
            return dir.join("target/test-artifacts");
        }
        if !dir.pop() {
            return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-artifacts");
        }
    }
}

fn bless_requested() -> bool {
    std::env::var("BLESS").is_ok_and(|v| v == "1")
}

fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Number of differing pixels, and a diff image: differing pixels pure red, matching pixels the
/// actual image at quarter brightness.
fn diff_images(actual: &[u8], expected: &[u8]) -> (usize, Vec<u8>) {
    let mut differing = 0;
    let mut diff = Vec::with_capacity(actual.len());
    for (a, e) in actual.chunks_exact(3).zip(expected.chunks_exact(3)) {
        if a == e {
            diff.extend(a.iter().map(|c| c / 4));
        } else {
            differing += 1;
            diff.extend_from_slice(&[255, 0, 0]);
        }
    }
    (differing, diff)
}

/// Compares `rgb` (`w` x `h`) against `<golden_dir>/<name>.png`.
///
/// `max_differing_pixels` is the tolerance (0 = exact). See the module docs for the failure and
/// `BLESS=1` behaviour.
pub fn assert_golden_rgb_in(
    golden_dir: &Path,
    name: &str,
    w: usize,
    h: usize,
    rgb: &[u8],
    max_differing_pixels: usize,
) {
    let golden_path = golden_dir.join(format!("{name}.png"));
    let actual_path = artifact_dir().join(format!("{name}.actual.png"));
    let diff_path = artifact_dir().join(format!("{name}.diff.png"));
    let actual_png = encode_png(w, h, rgb);

    if bless_requested() {
        write_file(&golden_path, &actual_png);
        eprintln!("BLESS: wrote golden {}", golden_path.display());
        return;
    }

    let expected_bytes = match fs::read(&golden_path) {
        Ok(b) => b,
        Err(e) => {
            write_file(&actual_path, &actual_png);
            panic!(
                "golden '{name}' is missing ({}: {e}); actual image written to {}; \
                 rerun with BLESS=1 to create it",
                golden_path.display(),
                actual_path.display()
            );
        }
    };
    let (ew, eh, expected) = decode_png(&expected_bytes)
        .unwrap_or_else(|e| panic!("golden {} is unreadable: {e}", golden_path.display()));
    if (ew, eh) != (w, h) {
        write_file(&actual_path, &actual_png);
        panic!(
            "golden '{name}' is {ew}x{eh} but the actual image is {w}x{h}; actual written to {} \
             (golden: {})",
            actual_path.display(),
            golden_path.display()
        );
    }
    let (differing, diff) = diff_images(rgb, &expected);
    if differing > max_differing_pixels {
        write_file(&actual_path, &actual_png);
        write_file(&diff_path, &encode_png(w, h, &diff));
        panic!(
            "golden '{name}' mismatch: {differing} of {} pixels differ (allowed {max_differing_pixels}); \
             actual: {}  diff: {}  golden: {}",
            w * h,
            actual_path.display(),
            diff_path.display(),
            golden_path.display()
        );
    }
}

/// Exact comparison of a frame (optionally cropped) with `room/tests/golden/<name>.png`.
pub fn assert_golden(name: &str, frame: &Frame, crop: Option<Rect>) {
    assert_golden_tol(name, frame, crop, 0);
}

/// Like [`assert_golden`] with a tolerance of `max_differing_pixels` differing pixels.
pub fn assert_golden_tol(
    name: &str,
    frame: &Frame,
    crop: Option<Rect>,
    max_differing_pixels: usize,
) {
    let (w, h, rgb) = frame.to_rgb(crop);
    assert_golden_rgb_in(&golden_dir(), name, w, h, &rgb, max_differing_pixels);
}
