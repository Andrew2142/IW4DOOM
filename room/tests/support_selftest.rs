// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-tests of the engine-free parts of `support`: golden-image tooling and the counting
//! allocator (sheet 13 sections 5.2 and 5.4). No engine is started here, so this file may have
//! more than one `#[test]`.
//!
//! The counting allocator is installed for this whole test binary, as a binary that uses it
//! would do.

mod support;

use std::panic::{catch_unwind, AssertUnwindSafe};

use support::alloc::{self, CountingAlloc};
use support::golden::{
    artifact_dir, assert_golden_rgb_in, decode_png, encode_png, Frame, Rect, FRAME_H, FRAME_W,
};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn gradient_frame() -> Frame {
    let mut palette = [[0u8; 3]; 256];
    for (i, p) in palette.iter_mut().enumerate() {
        *p = [i as u8, 255 - i as u8, (i as u8).wrapping_mul(3)];
    }
    let indices = (0..FRAME_W * FRAME_H)
        .map(|i| ((i % FRAME_W) / 2) as u8)
        .collect();
    Frame { indices, palette }
}

#[test]
fn frame_png_round_trips_through_the_palette() {
    let f = gradient_frame();
    assert_eq!(f.distinct_indices(), 160);
    let (w, h, rgb) = decode_png(&f.to_png(None)).expect("PNG");
    assert_eq!((w, h), (FRAME_W, FRAME_H));
    assert_eq!(&rgb[..3], &f.palette[0]);
    assert_eq!(&rgb[3 * 5..3 * 5 + 3], &f.palette[2]);

    let crop = Rect::new(4, 2, 8, 3);
    let (cw, ch, cropped) = decode_png(&f.to_png(Some(crop))).expect("PNG");
    assert_eq!((cw, ch), (8, 3));
    assert_eq!(&cropped[..3], &f.palette[2]);
    // Deterministic encoding.
    assert_eq!(f.to_png(None), f.to_png(None));
}

#[test]
fn crop_outside_the_frame_fails() {
    let f = gradient_frame();
    assert!(catch_unwind(AssertUnwindSafe(
        || f.to_png(Some(Rect::new(300, 0, 40, 10)))
    ))
    .is_err());
}

/// One test for everything that reads or sets `BLESS`, since the variable is process-wide.
#[test]
fn golden_compare_bless_and_failure_paths() {
    let dir = artifact_dir().join("selftest-goldens");
    let _ = std::fs::remove_dir_all(&dir);
    let name = "selftest";
    let (w, h) = (16, 8);
    let base: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();

    // Missing golden: fails, and the actual image is written.
    std::env::remove_var("BLESS");
    let r = catch_unwind(AssertUnwindSafe(|| {
        assert_golden_rgb_in(&dir, name, w, h, &base, 0);
    }));
    assert!(r.is_err(), "a missing golden must fail");
    assert!(
        artifact_dir().join(format!("{name}.actual.png")).is_file(),
        "actual image written for a missing golden"
    );

    // BLESS=1 creates it.
    std::env::set_var("BLESS", "1");
    assert_golden_rgb_in(&dir, name, w, h, &base, 0);
    std::env::remove_var("BLESS");
    assert!(dir.join(format!("{name}.png")).is_file());

    // Exact match passes.
    assert_golden_rgb_in(&dir, name, w, h, &base, 0);

    // One differing pixel: fails when exact, passes with a tolerance of 1.
    let mut off = base.clone();
    off[0] ^= 0xFF;
    let r = catch_unwind(AssertUnwindSafe(|| {
        assert_golden_rgb_in(&dir, name, w, h, &off, 0);
    }));
    assert!(
        r.is_err(),
        "one differing pixel must fail an exact comparison"
    );
    let actual = artifact_dir().join(format!("{name}.actual.png"));
    let diff = artifact_dir().join(format!("{name}.diff.png"));
    assert!(
        actual.is_file() && diff.is_file(),
        "actual and diff images written"
    );
    let (_, _, diff_rgb) = decode_png(&std::fs::read(&diff).expect("diff")).expect("PNG");
    assert_eq!(
        &diff_rgb[..3],
        &[255, 0, 0],
        "differing pixel is red in the diff"
    );
    assert_golden_rgb_in(&dir, name, w, h, &off, 1);

    // A different size fails even with a huge tolerance.
    let small = &base[..8 * 8 * 3];
    let r = catch_unwind(AssertUnwindSafe(|| {
        assert_golden_rgb_in(&dir, name, 8, 8, small, usize::MAX);
    }));
    assert!(r.is_err(), "a size mismatch must fail");

    // BLESS=1 overwrites a stale golden.
    std::env::set_var("BLESS", "1");
    assert_golden_rgb_in(&dir, name, w, h, &off, 0);
    std::env::remove_var("BLESS");
    assert_golden_rgb_in(&dir, name, w, h, &off, 0);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(actual);
    let _ = std::fs::remove_file(diff);
    // `encode_png` stays usable on its own.
    assert!(!encode_png(1, 1, &[1, 2, 3]).is_empty());
}

#[test]
fn counting_allocator_counts_only_inside_measure() {
    assert!(alloc::installed());

    let ((), quiet) = alloc::measure(|| {
        let mut x = 0u64;
        for i in 0..1000u64 {
            x = std::hint::black_box(x.wrapping_add(i));
        }
        std::hint::black_box(x);
    });
    assert_eq!(quiet.allocations, 0);

    let (v, stats) = alloc::measure(|| std::hint::black_box(Vec::<u8>::with_capacity(4096)));
    assert_eq!(stats.allocations, 1);
    assert!(stats.bytes >= 4096);
    drop(v);

    // Preallocated buffers reused in the closure do not allocate.
    let mut buf = Vec::with_capacity(64);
    alloc::assert_no_allocations("reused buffer", || {
        buf.clear();
        buf.extend_from_slice(&[1u8; 64]);
    });

    // An allocating closure fails the assertion.
    let r = catch_unwind(AssertUnwindSafe(|| {
        alloc::assert_no_allocations("allocates", || std::hint::black_box(vec![0u8; 16]));
    }));
    assert!(
        r.is_err(),
        "an allocating closure must fail assert_no_allocations"
    );
}
