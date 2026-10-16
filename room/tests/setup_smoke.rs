// SPDX-License-Identifier: GPL-3.0-or-later
//! Smoke test used by `iw4doom.sh` / `iw4doom.ps1` step 7 (sheet 14 section 5.2): start the
//! engine headless on an IWAD with a pack, enter the first map, run 70 tics, and require that the
//! pack loaded and MW2 mode is on for the level.
//!
//! `DMW_SMOKE_IWAD` and `DMW_SMOKE_PACK` select the user's IWAD and exported pack (the launcher
//! sets both). Without them the test uses the located test IWAD (`DMW_IWAD`, else
//! `wads/freedoom1.wad`) and the in-memory fixture pack, so it runs in the normal suite and in
//! CI with no MW2 data.
//!
//! On success it prints one `smoke: ok ...` line; on failure it panics with the reason MW2 mode
//! is off and the warnings and errors logged during startup.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

mod support;

use room::dmw::test_api;

use support::Harness;

/// IWADs with MAPxx maps (`-warp 1`); every other IWAD has ExMy maps (`-warp 1 1`).
const DOOM2_STYLE: [&str; 4] = ["doom2", "plutonia", "tnt", "freedoom2"];

fn problems() -> Vec<String> {
    support::captured_log()
        .into_iter()
        .filter(|l| l.starts_with("[ERROR") || l.starts_with("[WARN"))
        .collect()
}

#[test]
fn setup_smoke() {
    let wad = std::env::var("DMW_SMOKE_IWAD")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| support::find_wad());
    let wad = wad.to_str().expect("IWAD path is UTF-8").to_owned();
    let pack = std::env::var("DMW_SMOKE_PACK").ok();

    let stem = std::path::Path::new(&wad)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut args = vec!["-iwad", wad.as_str(), "-skill", "3"];
    if DOOM2_STYLE.contains(&stem.as_str()) {
        args.extend(["-warp", "1"]);
    } else {
        args.extend(["-warp", "1", "1"]);
    }
    match &pack {
        Some(p) => args.extend(["-mw2pack", p.as_str()]),
        None => args.push("-fixturepack"),
    }

    let mut h = Harness::start(&args);
    h.run_tics(70);

    let pack_name = pack.as_deref().unwrap_or("fixture pack");
    assert!(
        test_api::pack_loaded(),
        "smoke: the pack at {pack_name} did not load; log: {:#?}",
        problems()
    );
    assert!(
        room::dmw::mode::level_active(),
        "smoke: MW2 mode is off: {}; log: {:#?}",
        room::dmw::mode::reason().unwrap_or_else(|| "no reason recorded".to_owned()),
        problems()
    );
    assert!(h.tics_run() >= 70, "smoke: fewer than 70 tics ran");
    let weapons =
        test_api::mapping().map_or(0, |m| (0..9).filter(|d| m.mapped(*d).is_some()).count());
    eprintln!(
        "smoke: ok: {wad} with {pack_name}, MW2 mode on, {weapons} Doom weapons mapped to MW2, \
         {} tics",
        h.tics_run()
    );
}
