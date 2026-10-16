// SPDX-License-Identifier: GPL-3.0-or-later
//! Manual QA with a real exported pack (sheet 04 §10, sheet 13 §5.6). Ignored by default: it
//! needs MW2-derived data, which never enters the repository.
//!
//! ```text
//! DMW_REAL_PACK=/abs/path/packs/mw2 DMW_IWAD=wads/freedoom2.wad \
//!   cargo test -p room --release --test mw2pack_headless -- --ignored --nocapture
//! ```
//!
//! Optional `DMW_REAL_CONFIG=<iw4doom.toml>` passes `-mw2config` (to map other pack weapons).
//!
//! For every Doom weapon slot mapped to an MW2 weapon: select it (IDKFA gives everything),
//! wait for the raise, fire, reload, aim down sights, and check that MW2 mode stays on and the
//! sim reports the shots and the reload. One idle and one ADS frame per weapon (the full game
//! frame and an isolated viewmodel render) are written to `target/test-artifacts/mw2pack/`.
//! Those images are MW2-derived and must never be committed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

mod support;

use room::dmw::test_api;
use room::dmw::viewmodel::Light;
use room::doom::doomkeys::KEY_FIRE;
use room::doom::i_video::test_palette_rgb;

use support::golden::{artifact_dir, encode_png};
use support::{Frame, Harness, SimEvent};

/// Doom weapons with their selection key (`wp_pistol` = 1 … `wp_supershotgun` = 8).
const SLOTS: [(i32, &str, u8); 7] = [
    (1, "pistol", b'2'),
    (2, "shotgun", b'3'),
    (8, "supershotgun", b'3'),
    (3, "chaingun", b'4'),
    (4, "missile", b'5'),
    (5, "plasma", b'6'),
    (6, "bfg", b'7'),
];

const WEAPON_READY: i32 = 0;
/// Isolated renders use a bright sector level with the directional term.
const RENDER_LIGHT: Light = Light::Sector {
    level: 4,
    directional: true,
};

fn save(name: &str, frame: &Frame) {
    let dir = artifact_dir().join("mw2pack");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    std::fs::write(&path, frame.to_png(None)).unwrap();
    eprintln!("wrote {}", path.display());
}

/// The isolated render scaled 3x with a magenta background (easier to inspect).
fn save_isolated(name: &str, id: dmw_format::DmwWeaponId) -> usize {
    // A palette index the weapon can never produce (index 0 is black and real guns use it).
    let used = test_api::viewmodel_palette_set(id);
    let bg = (0..=255u8).rev().find(|i| !used.contains(i)).unwrap_or(0);
    let Some((px, stats)) = test_api::render_viewmodel(bg, RENDER_LIGHT) else {
        eprintln!("{name}: MW2 does not own the weapon, nothing rendered");
        return 0;
    };
    let pal = test_palette_rgb();
    let (w, h, s) = (320usize, 200usize, 3usize);
    let mut rgb = vec![0u8; w * h * s * s * 3];
    let mut n = 0;
    for y in 0..h * s {
        for x in 0..w * s {
            let i = px[(y / s) * w + x / s];
            let c = if i == bg { [255, 0, 255] } else { pal[usize::from(i)] };
            if i != bg && x % s == 0 && y % s == 0 {
                n += 1;
            }
            let o = (y * w * s + x) * 3;
            rgb[o..o + 3].copy_from_slice(&c);
        }
    }
    let dir = artifact_dir().join("mw2pack");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}_vm.png"));
    std::fs::write(&path, encode_png(w * s, h * s, &rgb)).unwrap();
    eprintln!(
        "wrote {} ({n} gun pixels, {} triangles in, {} drawn)",
        path.display(),
        stats.triangles_in,
        stats.triangles_drawn
    );
    n
}

fn wait_ready(h: &mut Harness, max_tics: u32) -> bool {
    for _ in 0..max_tics {
        h.run_tics(1);
        if h.sim_view().is_some_and(|v| v.weaponstate == WEAPON_READY) {
            return true;
        }
    }
    false
}

fn problems() -> Vec<String> {
    support::captured_log()
        .into_iter()
        .filter(|l| l.starts_with("[ERROR") || l.starts_with("[WARN"))
        .collect()
}

#[test]
#[ignore = "needs a real exported MW2 pack (DMW_REAL_PACK); manual QA, sheet 13 §5.6"]
fn mw2pack_headless() {
    let pack = std::env::var("DMW_REAL_PACK").expect("set DMW_REAL_PACK to an exported pack");
    let wad = support::find_wad();
    let wad = wad.to_str().unwrap().to_owned();
    let doom2 = !wad.contains("doom1") && !wad.contains("freedoom1");
    let config = std::env::var("DMW_REAL_CONFIG").ok();
    let mut args = vec!["-iwad", wad.as_str(), "-mw2pack", pack.as_str(), "-skill", "3"];
    if doom2 {
        args.extend(["-warp", "1"]);
    } else {
        args.extend(["-warp", "1", "1"]);
    }
    if let Some(c) = &config {
        args.extend(["-mw2config", c.as_str()]);
    }
    let mut h = Harness::start(&args);
    assert!(test_api::pack_loaded(), "pack did not load: {:?}", problems());
    assert!(room::dmw::mode::level_active(), "MW2 mode is off: {:?}", room::dmw::mode::reason());
    let removed = h.remove_monsters();
    eprintln!("removed {removed} monsters; startup warnings: {:#?}", problems());
    h.give_all();
    h.run_tics(5);

    let mapping = test_api::mapping().expect("mapping");
    let facts_pack = dmw_format::Pack::open(std::path::Path::new(&pack)).expect("pack opens");
    let mut failures = Vec::new();
    let mut tested = Vec::new();
    for (doom, slot, key) in SLOTS {
        let Some(id) = mapping.mapped(doom) else {
            eprintln!("{slot}: vanilla (not mapped or not in this IWAD)");
            continue;
        };
        // wp_supershotgun shares key 3 with the shotgun; press until it is in hand.
        let mut held = false;
        for _ in 0..3 {
            h.key(key, true);
            h.run_tics(2);
            h.key(key, false);
            h.run_tics(2);
            for _ in 0..140 {
                h.run_tics(1);
                if h.sim_view().and_then(|v| v.weapon) == Some(id) {
                    break;
                }
            }
            if h.sim_view().and_then(|v| v.weapon) == Some(id) {
                held = true;
                break;
            }
        }
        if !held {
            // Shareware lacks the super shotgun, plasma gun and BFG.
            if !doom2 && matches!(doom, 5 | 6 | 8) {
                eprintln!("{slot}: not available in this IWAD");
            } else {
                failures.push(format!("{slot}: weapon {id:?} never came into hand"));
            }
            continue;
        }
        if !wait_ready(&mut h, 200) {
            failures.push(format!("{slot}: raise did not finish"));
            continue;
        }
        let name = facts_pack.facts(id).name.clone();
        h.run_tics(20);
        save(&format!("{name}_idle"), &h.frame());
        let idle_px = save_isolated(&format!("{name}_idle"), id);
        if idle_px < 200 {
            failures.push(format!("{name}: idle render covers only {idle_px} pixels"));
        }

        // Fire.
        h.clear_events();
        let clip_before = h.sim_view().map_or(0, |v| v.clip);
        // Hold the trigger until the first shot (launchers may still be settling), then 12 tics.
        let count_shots = |h: &Harness| {
            h.events()
                .iter()
                .filter(|e| matches!(e.event, SimEvent::Shot(_)))
                .count()
        };
        h.key(KEY_FIRE, true);
        for _ in 0..70 {
            h.run_tics(1);
            if count_shots(&h) > 0 {
                break;
            }
        }
        h.run_tics(12);
        h.key(KEY_FIRE, false);
        h.run_tics(4);
        let shots = count_shots(&h);
        let clip_after = h.sim_view().map_or(0, |v| v.clip);
        if shots == 0 {
            failures.push(format!("{name}: no Shot event"));
        }

        // Reload (an empty single-round launcher reloads by itself: events are not cleared, so
        // its ReloadStarted from the fire phase counts).
        wait_ready(&mut h, 200);
        h.key(b'r', true);
        h.run_tics(2);
        h.key(b'r', false);
        let mut started = false;
        let mut finished = false;
        for _ in 0..400 {
            h.run_tics(1);
            for e in h.events() {
                match e.event {
                    SimEvent::ReloadStarted { .. } => started = true,
                    SimEvent::ReloadFinished => finished = true,
                    _ => {}
                }
            }
            if finished && h.sim_view().is_some_and(|v| v.weaponstate == WEAPON_READY) {
                break;
            }
        }
        let clip_reloaded = h.sim_view().map_or(0, |v| v.clip);
        // weapon_iw4 reports ReloadEnded only for segmented reloads; a refilled magazine is the
        // general proof that the reload ran to completion.
        if !started || clip_reloaded <= clip_after.min(clip_before - 1) {
            failures.push(format!(
                "{name}: reload started={started} finished={finished} clip {clip_after} -> {clip_reloaded}"
            ));
        }

        // ADS.
        wait_ready(&mut h, 100);
        h.mouse(0, 0, 0b10);
        let mut ads = 0.0;
        for _ in 0..140 {
            h.run_tics(1);
            h.mouse(0, 0, 0b10);
            ads = h.sim_view().map_or(0.0, |v| v.ads_frac);
            if ads >= 1.0 {
                break;
            }
        }
        h.run_tics(10);
        h.mouse(0, 0, 0b10);
        save(&format!("{name}_ads"), &h.frame());
        let ads_px = save_isolated(&format!("{name}_ads"), id);
        let overlay = h.sim_view().is_some_and(|v| v.overlay_active);
        h.mouse(0, 0, 0);
        h.run_tics(30);
        if ads < 1.0 && facts_pack.facts(id).fpv.aim_down_sight {
            failures.push(format!("{name}: ads_frac reached only {ads}"));
        }
        if !room::dmw::mode::level_active() || !test_api::mw2_owns_weapon() {
            failures.push(format!(
                "{name}: MW2 mode dropped ({:?})",
                room::dmw::mode::reason()
            ));
        }
        eprintln!(
            "{slot} -> {name}: {shots} shots (clip {clip_before} -> {clip_after} -> {clip_reloaded}), \
             reload {started}/{finished}, ads {ads:.2} overlay {overlay}, idle px {idle_px}, ads px {ads_px}"
        );
        tested.push(name);
    }
    let probs = problems();
    eprintln!("tested: {tested:?}");
    eprintln!("dmw warnings/errors in the last log lines: {probs:#?}");
    assert!(!tested.is_empty(), "no mapped weapon was tested");
    assert!(failures.is_empty(), "failures: {failures:#?}");
}
