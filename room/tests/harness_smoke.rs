// SPDX-License-Identifier: GPL-3.0-or-later
//! Smoke test of the shared headless harness (sheet 13 section 9): starts E1M1, runs 35 tics,
//! captures a frame, and asserts the clock advanced by exactly 35 tics and the frame is not
//! uniform. It then touches every other harness method once, so a broken helper is found here
//! and not in a later sheet's test.

mod support;

use room::doom::doomkeys::KEY_UPARROW;
use room::doom::info::{MF_COUNTKILL, MT_TROOP};

use support::golden::{decode_png, Rect};
use support::locations::OPEN_ROOM;
use support::{assert_golden, Harness};

#[test]
fn harness_smoke() {
    let mut h = Harness::start_default();

    // Sheet 13 section 9: 35 tics from the running level, a real picture.
    let (gametic0, leveltime0, clock0) = (h.gametic(), h.level_time(), h.virtual_ms());
    h.run_tics(35);
    assert_eq!(h.gametic(), gametic0 + 35, "gametic advanced by 35");
    assert_eq!(h.level_time(), leveltime0 + 35, "leveltime advanced by 35");
    assert_eq!(h.tics_run(), 35);
    assert!(
        h.virtual_ms() >= clock0 + 35 * support::TICK_MS,
        "virtual clock advanced"
    );
    assert!(h.frames_drawn() >= 35, "one frame per tic");
    let frame = h.frame();
    assert!(
        frame.distinct_indices() > 8,
        "frame is nearly uniform ({} distinct colours)",
        frame.distinct_indices()
    );
    assert_eq!(frame.indices.len(), 320 * 200);

    // PNG encoding round-trips through the captured palette.
    let png = frame.to_png(None);
    let (w, hgt, rgb) = decode_png(&png).expect("valid PNG");
    assert_eq!((w, hgt), (320, 200));
    assert_eq!(rgb, frame.to_rgb(None).2);
    let crop = Rect::new(10, 20, 64, 32);
    assert_eq!(frame.to_rgb(Some(crop)).0, 64);

    // Pinned location, empty of monsters.
    OPEN_ROOM.go(&mut h);
    let (x, y) = h.pos();
    assert!((x - OPEN_ROOM.x).abs() < 0.01 && (y - OPEN_ROOM.y).abs() < 0.01);
    assert_eq!(h.mobj().angle, 0x4000_0000, "facing 90 degrees = north");
    assert!(
        h.mobjs().iter().all(|m| m.3 & MF_COUNTKILL == 0),
        "no live monsters"
    );

    // Golden image of the 3D view (status bar excluded: its face animates).
    h.run_tics(2);
    assert_golden(
        "harness_open_room",
        &h.frame(),
        Some(Rect::new(0, 0, 320, 160)),
    );

    // Pitch writes the extension state and clamps at the freelook limit.
    h.set_pitch(10.0);
    assert_eq!(h.player_ext().pitch_q, 10 * 128);
    h.set_pitch(100.0);
    assert_eq!(h.player_ext().pitch_q, 32 * 128);
    h.set_pitch(0.0);

    // Spawning and removing monsters.
    let imp = h.spawn(MT_TROOP, OPEN_ROOM.x, OPEN_ROOM.y + 300.0);
    assert!(!imp.is_null());
    assert!(h.mobjs().iter().any(|m| m.0 == MT_TROOP));
    assert!(h.remove_monsters() >= 1);
    assert!(h.mobjs().iter().all(|m| m.3 & MF_COUNTKILL == 0));

    // Key injection: hold forward for 10 tics moves the player north; the release follows.
    let y0 = h.pos().1;
    h.hold(KEY_UPARROW, 10);
    h.run_tics(2);
    assert!(h.pos().1 > y0 + 10.0, "forward key moved the player north");
    h.run_tics(40); // momentum dies

    // Cheat path.
    h.give_all();
    assert_eq!(h.player().armorpoints, 200);

    // Contracts that later sheets fill in.
    assert!(h.sim_view().is_none(), "no MW2 state yet (sheet 08)");
    assert!(h.events().is_empty(), "no event ring buffer yet (sheet 08)");

    // A second level in the same engine.
    h.new_game(2, 1, 2);
    assert!(h.level_time() < 5, "fresh level");
}
