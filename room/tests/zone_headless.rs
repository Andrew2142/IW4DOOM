// SPDX-License-Identifier: GPL-3.0-or-later
//! Zone memory regression: level memory must be freed between levels.
//!
//! room capped `Z_FreeTags` at 2000 blocks, so most `PU_LEVEL` memory survived every level
//! change and a 32 MiB zone ran out after about nine loads. DOOMMW2 restored the C behaviour.
//! This loads 30 levels into a deliberately small zone; with the cap back it fails with
//! `Z_Malloc: failed on allocation`.

mod support;

use support::{find_wad, Harness};

#[test]
fn zone_headless() {
    let wad = find_wad();
    let wad = wad.to_str().expect("utf-8 path");
    // An explicit -mb overrides the harness default of 256.
    let mut h = Harness::start(&[
        "-iwad", wad, "-nomw2", "-skill", "3", "-warp", "1", "1", "-mb", "32",
    ]);
    for i in 0..30 {
        let map = (i % 9) + 1;
        h.new_game(3, 1, map);
        h.run_tics(5);
    }
}
