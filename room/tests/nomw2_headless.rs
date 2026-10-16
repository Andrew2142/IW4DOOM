// SPDX-License-Identifier: GPL-3.0-or-later
//! T8.8 (`-nomw2` half): with `-nomw2` the pack is never loaded (even with `-fixturepack`) and
//! every sheet 08 hook is inert: Doom's own psprites run, no MW2 tic is handled.

mod support;

use room::dmw::{mode, test_api};
use room::doom::doomkeys::KEY_FIRE;

use support::{find_wad, Harness};

#[test]
fn nomw2_headless() {
    let wad = find_wad();
    let wad = wad.to_str().expect("utf-8 path");
    let mut h = Harness::start(&[
        "-iwad",
        wad,
        "-nomw2",
        "-fixturepack",
        "-skill",
        "3",
        "-warp",
        "1",
        "1",
    ]);
    assert!(!test_api::pack_loaded());
    h.run_tics(20);
    h.key(KEY_FIRE, true);
    h.run_tics(30);
    h.key(KEY_FIRE, false);
    h.key(b'1', true);
    h.run_tics(1);
    h.key(b'1', false);
    h.run_tics(40);
    assert!(!mode::level_active());
    assert_eq!(mode::reason().as_deref(), Some("no pack loaded"));
    assert_eq!(test_api::handled_tics(), 0);
    assert!(
        !h.player().psprites[0].state.is_null(),
        "vanilla psprite runs"
    );
    assert_eq!(h.player().maxammo, [200, 50, 300, 50], "vanilla maxima");
    assert!(h.player().ammo[0] < 50, "the vanilla pistol fired");
}
