// SPDX-License-Identifier: GPL-3.0-or-later
//! T11.9, `-nomw2` half: the E1M1 frame after 60 tics with MW2 mode off. Its status bar rows
//! are compared with the golden that `viewmodel_headless.rs` checks for the same frame with
//! `-fixturepack` (the gun never touches the status bar). The max-ammo column is blanked in
//! both: sheet 08 shows the DOOMMW2 ammo maxima there.

mod support;

use room::dmw::test_api;
use room::doom::r_draw::viewheight;

use support::{find_wad, Frame, Harness, Rect};

/// The status bar rows of `frame` with the max-ammo column (x 300..316, y 172..198) blanked:
/// sheet 08 raises the ammo maxima in MW2 mode (360/64/12/300 instead of 200/50/50/300), the
/// only status bar difference that is not the viewmodel's business.
fn statusbar_without_maxima(frame: &Frame, view_h: usize) -> (Frame, Rect) {
    let mut f = frame.clone();
    for y in 172..198 {
        for x in 300..316 {
            f.indices[y * 320 + x] = 0;
        }
    }
    (f, Rect::new(0, view_h, 320, 200 - view_h))
}

#[test]
fn viewmodel_nomw2_headless() {
    let wad = find_wad();
    let wad = wad.to_str().expect("utf-8 path");
    let mut h = Harness::start(&["-iwad", wad, "-nomw2", "-skill", "3", "-warp", "1", "1"]);
    assert!(!test_api::pack_loaded());
    h.run_tics(60);
    let frame = h.frame();
    // SAFETY: plain read of a renderer global on the engine thread.
    let wh = unsafe { viewheight } as usize;
    assert_eq!(wh, 168, "default view window");
    let (sbar, rect) = statusbar_without_maxima(&frame, wh);
    support::assert_golden("e1m1_t60_statusbar", &sbar, Some(rect));
}
