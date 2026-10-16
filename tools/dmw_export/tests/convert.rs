// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 04 §9 tests that need no MW2 data (T4.1–T4.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use std::fs;
use std::path::PathBuf;

use dmw_export::roster::{edit_distance, suggest};
use dmw_export::sounds::{parse_riff, pcm_to_i16};
use dmw_export::textures::{downscale_to, halve};
use dmw_export::viewmodel::{RawSkin, skin_weights};
use dmw_export::write::{write_atomic, write_file};
use dmw_format::RgbaImage;

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmw_export_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

// ---------------------------------------------------------------- T4.1 skin weights

#[test]
fn t4_1_rigid_vertex_is_one_bone() {
    let (s, fixed) = skin_weights(&RawSkin {
        bones: [7, 0, 0, 0],
        extra_raw: [0; 3],
        has_extra: [false; 3],
        primary_zero: false,
        only_primary: true,
    });
    assert_eq!(s.bones, [7, 0, 0, 0]);
    assert_eq!(s.weights, [1.0, 0.0, 0.0, 0.0]);
    assert!(!fixed);
}

#[test]
fn t4_1_two_bone_blend_gives_primary_the_remainder() {
    // 0x4000 / 65536 = 0.25 on the extra bone; primary gets 0.75.
    let (s, fixed) = skin_weights(&RawSkin {
        bones: [3, 9, 0, 0],
        extra_raw: [0x4000, 0, 0],
        has_extra: [true, false, false],
        primary_zero: false,
        only_primary: false,
    });
    assert_eq!(s.bones, [3, 9, 0, 0]);
    assert!(close(s.weights[0], 0.75) && close(s.weights[1], 0.25));
    assert_eq!(s.weights[2], 0.0);
    assert!(!fixed);
    assert!(close(s.weights.iter().sum(), 1.0));
}

#[test]
fn t4_1_primary_zero_puts_everything_on_the_extras() {
    // Two extras of 0.5 each, primary flagged zero.
    let (s, fixed) = skin_weights(&RawSkin {
        bones: [1, 2, 5, 0],
        extra_raw: [0x8000, 0x8000, 0],
        has_extra: [true, true, false],
        primary_zero: true,
        only_primary: false,
    });
    assert_eq!(s.weights[0], 0.0);
    assert!(close(s.weights[1], 0.5) && close(s.weights[2], 0.5));
    assert_eq!(s.bones[1..3], [2, 5]);
    assert!(!fixed);
}

#[test]
fn t4_1_off_weights_are_renormalised_and_counted() {
    // Primary zero with extras summing to 0.6: renormalised to 1, reported.
    let (s, fixed) = skin_weights(&RawSkin {
        bones: [1, 2, 3, 0],
        extra_raw: [0x4CCD, 0x1999, 0],
        has_extra: [true, true, false],
        primary_zero: true,
        only_primary: false,
    });
    assert!(fixed);
    assert!(close(s.weights.iter().sum(), 1.0));
    assert!(close(
        s.weights[1] / s.weights[2],
        0x4CCD as f32 / 0x1999 as f32
    ));
    // Small deviation (< 1e-2) is renormalised silently.
    let (s, fixed) = skin_weights(&RawSkin {
        bones: [1, 2, 0, 0],
        extra_raw: [0xFF00, 0, 0],
        has_extra: [true, false, false],
        primary_zero: true,
        only_primary: false,
    });
    assert!(!fixed);
    assert!(close(s.weights[1], 1.0));
}

#[test]
fn t4_1_all_zero_falls_back_to_primary() {
    let (s, fixed) = skin_weights(&RawSkin {
        bones: [4, 0, 0, 0],
        extra_raw: [0; 3],
        has_extra: [false; 3],
        primary_zero: true,
        only_primary: false,
    });
    assert!(fixed);
    assert_eq!(s.weights, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(s.bones[0], 4);
}

// ---------------------------------------------------------------- T4.2 downscale

#[test]
fn t4_2_downscale_2048_to_1024_in_linear_light() {
    // Checkerboard of black and white texels; a 2x2 box averages to 50% linear light, which is
    // sRGB 188, not the naive 128. Alpha (0 and 255 alternating columns) averages to 128.
    let mut img = RgbaImage::new(2048, 2048, [0; 4]);
    for y in 0..2048 {
        for x in 0..2048 {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            let a = if x % 2 == 0 { 255 } else { 0 };
            img.set(x, y, [v, v, v, a]);
        }
    }
    let out = downscale_to(img, 1024);
    assert_eq!((out.width, out.height), (1024, 1024));
    let p = out.get(512, 300);
    assert_eq!(p[0], 188);
    assert_eq!(p[1], 188);
    assert_eq!(p[3], 128);
    // Uniform colour survives unchanged.
    let flat = halve(&RgbaImage::new(4, 2, [10, 200, 77, 255]));
    assert_eq!((flat.width, flat.height), (2, 1));
    assert_eq!(flat.get(1, 0), [10, 200, 77, 255]);
    // Only the oversized edge drives the number of halvings; 4096x512 → 1024x128.
    let wide = downscale_to(RgbaImage::new(4096, 512, [1, 2, 3, 4]), 1024);
    assert_eq!((wide.width, wide.height), (1024, 128));
}

// ---------------------------------------------------------------- T4.3 PCM → WAV

fn round_trip(rate: u32, channels: u16, samples: &[i16]) -> dmw_format::SoundAsset {
    let wav = dmw_format::sound::encode_wav_i16(rate, channels, samples).unwrap();
    let dir = tmp_dir(&format!("wav{rate}_{channels}"));
    let path = dir.join("a.wav");
    fs::write(&path, &wav).unwrap();
    let asset = dmw_format::sound::load_wav(&path).unwrap();
    let _ = fs::remove_dir_all(&dir);
    asset
}

#[test]
fn t4_3_eight_bit_unsigned() {
    let s = pcm_to_i16(&[0, 128, 255], 8, 1).unwrap();
    assert_eq!(s, [-32768, 0, 32512]);
    let a = round_trip(22050, 1, &s);
    assert_eq!((a.rate, a.channels), (22050, 1));
    assert_eq!(a.samples.len(), 3);
    assert!((a.samples[0] + 1.0).abs() < 1e-3 && a.samples[1].abs() < 1e-3);
}

#[test]
fn t4_3_sixteen_bit_signed_and_partial_frame() {
    let bytes = [0x00, 0x80, 0xFF, 0x7F, 0x34, 0x12, 0x99];
    let s = pcm_to_i16(&bytes, 16, 1).unwrap();
    assert_eq!(s, [i16::MIN, i16::MAX, 0x1234]);
    assert!(pcm_to_i16(&bytes, 24, 1).is_err());
    assert!(pcm_to_i16(&bytes, 16, 6).is_err());
}

#[test]
fn t4_3_stereo_interleave_round_trips() {
    // L = 1000, R = -1000 for two frames.
    let bytes: Vec<u8> = [1000i16, -1000, 1000, -1000]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let s = pcm_to_i16(&bytes, 16, 2).unwrap();
    let a = round_trip(44100, 2, &s);
    assert_eq!((a.rate, a.channels), (44100, 2));
    assert_eq!(a.samples.len(), 4);
    assert!(a.samples[0] > 0.0 && a.samples[1] < 0.0 && a.samples[2] > 0.0);
    assert!(close(a.samples[0], -a.samples[1]));
}

#[test]
fn t4_3_streamed_riff_is_parsed() {
    let wav = dmw_format::sound::encode_wav_i16(11025, 1, &[5, -5, 7]).unwrap();
    let p = parse_riff(&wav).unwrap().unwrap();
    assert_eq!((p.rate, p.channels, p.bits), (11025, 1, 16));
    assert_eq!(pcm_to_i16(&p.data, p.bits, p.channels).unwrap(), [5, -5, 7]);
    assert!(parse_riff(b"ID3 not a wave").unwrap().is_none());
    let mut adpcm = wav.clone();
    adpcm[20] = 0x11; // IMA ADPCM format tag
    assert!(parse_riff(&adpcm).is_err());
}

// ---------------------------------------------------------------- T4.4 atomic write

#[test]
fn t4_4_failed_write_leaves_previous_pack_intact() {
    let root = tmp_dir("atomic");
    let out = root.join("mw2");
    write_atomic(
        &out,
        |dir| write_file(dir, "manifest.json", b"old").map_err(|e| e.to_string()),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(fs::read(out.join("manifest.json")).unwrap(), b"old");

    // Failure injected after part of the new pack is written.
    let err = write_atomic(
        &out,
        |dir| {
            write_file(dir, "manifest.json", b"new").map_err(|e| e.to_string())?;
            write_file(dir, "weapons/x/facts.json", b"{}").map_err(|e| e.to_string())?;
            Err("injected failure".to_owned())
        },
        |_| Ok(()),
    );
    assert!(err.is_err());
    assert_eq!(fs::read(out.join("manifest.json")).unwrap(), b"old");
    assert!(!out.join("weapons").exists());

    // Verification failure is also rolled back.
    let err = write_atomic(
        &out,
        |dir| write_file(dir, "manifest.json", b"bad").map_err(|e| e.to_string()),
        |_| Err("verify failed".to_owned()),
    );
    assert!(err.is_err());
    assert_eq!(fs::read(out.join("manifest.json")).unwrap(), b"old");

    // No temporary or old directories are left behind.
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["mw2"]);

    // A successful write replaces the pack completely.
    write_atomic(
        &out,
        |dir| write_file(dir, "manifest.json", b"newer").map_err(|e| e.to_string()),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(fs::read(out.join("manifest.json")).unwrap(), b"newer");
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------- T4.5 suggestions

#[test]
fn t4_5_suggestions_rank_by_edit_distance() {
    assert_eq!(edit_distance("m4_mp", "m4_mp"), 0);
    assert_eq!(edit_distance("kitten", "sitting"), 3);
    let names = [
        "m4_mp", "m16_mp", "mp5k_mp", "ak47_mp", "usp_mp", "m40a3_mp", "fal_mp", "mg4_mp",
    ];
    assert_eq!(suggest("m4mp", names, 3), ["m4_mp", "mg4_mp", "m16_mp"]);
    assert_eq!(suggest("USP_MP", names, 1), ["usp_mp"]);
    assert_eq!(suggest("ak74_mp", names, 2), ["ak47_mp", "m4_mp"]);
    assert_eq!(suggest("x", names, 99).len(), names.len());
}
