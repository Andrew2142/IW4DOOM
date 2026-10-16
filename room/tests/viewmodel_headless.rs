// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 11 engine half, headless with `-fixturepack` (one engine per process):
//!
//! - T11.9: E1M1 start, frame after 60 tics: fixture-gun pixels (colours from the quantized
//!   fixture textures) in the lower right of the view window; the status bar rows match the
//!   golden shared with `viewmodel_nomw2_headless.rs` (the same frame with `-nomw2`), except
//!   the max-ammo column, which shows sheet 08's DOOMMW2 maxima.
//! - T11.8: the fixture pistol posed by `dmw_weapons::ViewmodelAnimator` through the engine's
//!   draw path (`test_api::render_viewmodel`): idle, mid-reload, ADS, plus the fire frame with
//!   the Doom flash patch, at COLORMAP level 8 with directional shading.
//! - Light modes of the hook: sector light, brighter while firing (muzzle `extralight`),
//!   invulnerability colormap, partial-invisibility fuzz.
//! - Per-draw time of the fixture gun through the engine path.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

mod support;

use std::collections::BTreeSet;

use room::dmw::test_api;
use room::dmw::viewmodel::Light;
use room::doom::d_player::PlayerT;
use room::doom::doomkeys::KEY_FIRE;
use room::doom::g_game::{consoleplayer, players};
use room::doom::i_video::test_palette_rgb;
use room::doom::r_draw::{scaledviewwidth, viewheight, viewwindowx, viewwindowy};

use support::golden::{assert_golden_rgb_in, golden_dir};
use support::{find_wad, Frame, Harness, Rect, SimEvent};

const PW_INVULNERABILITY: usize = 0;
const PW_INVISIBILITY: usize = 2;
/// Light level of the T11.8 goldens.
const GOLDEN_LIGHT: Light = Light::Sector {
    level: 8,
    directional: true,
};

fn player_mut() -> &'static mut PlayerT {
    // SAFETY: engine-thread-only static; used between engine calls only.
    unsafe {
        &mut *(&raw mut players)
            .cast::<PlayerT>()
            .add(consoleplayer as usize)
    }
}

/// (x, y, w, h) of the view window.
fn window() -> (usize, usize, usize, usize) {
    // SAFETY: plain reads of renderer globals on the engine thread.
    unsafe {
        (
            viewwindowx as usize,
            viewwindowy as usize,
            scaledviewwidth as usize,
            viewheight as usize,
        )
    }
}

fn tap(h: &mut Harness, key: u8) {
    h.key(key, true);
    h.run_tics(2);
    h.key(key, false);
}

/// A palette index the gun can never produce (background of isolated renders).
fn sentinel(gun: &BTreeSet<u8>) -> u8 {
    (0..=255u8)
        .find(|i| !gun.contains(i))
        .expect("a free index")
}

/// Gun pixels of an isolated render: positions that differ from the background.
fn mask(pixels: &[u8], bg: u8) -> Vec<usize> {
    pixels
        .iter()
        .enumerate()
        .filter(|(_, &p)| p != bg)
        .map(|(i, _)| i)
        .collect()
}

/// Isolated render of the held weapon, compared with `room/tests/golden/<name>.png` (view
/// window crop). Returns the pixels and the mask.
fn golden_render(name: &str, bg: u8) -> (Vec<u8>, Vec<usize>) {
    let (pixels, stats) = test_api::render_viewmodel(bg, GOLDEN_LIGHT).expect("MW2 gun up");
    assert!(stats.pixels > 0, "{name}: nothing drawn");
    let frame = Frame {
        indices: pixels.clone(),
        palette: test_palette_rgb(),
    };
    let (x, y, w, h) = window();
    let (cw, ch, rgb) = frame.to_rgb(Some(Rect::new(x, y, w, h)));
    assert_golden_rgb_in(&golden_dir(), name, cw, ch, &rgb, 0);
    let m = mask(&pixels, bg);
    (pixels, m)
}

/// Mean (x, y) of mask pixels.
fn centroid(m: &[usize]) -> (f32, f32) {
    let n = m.len() as f32;
    let (sx, sy) = m.iter().fold((0.0, 0.0), |(sx, sy), &i| {
        (sx + (i % 320) as f32, sy + (i / 320) as f32)
    });
    (sx / n, sy / n)
}

fn is_red(rgb: [u8; 3]) -> bool {
    rgb[0] >= 150 && rgb[1] <= 60 && rgb[2] <= 60
}

/// Fullbright isolated render split by colour: (hand, olive muzzle box, grey gun) pixels.
fn parts(bg: u8) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let (px, _) = test_api::render_viewmodel(bg, Light::Fixed(0)).expect("gun up");
    let pal = test_palette_rgb();
    let (mut skin, mut olive, mut grey) = (Vec::new(), Vec::new(), Vec::new());
    for i in mask(&px, bg) {
        let [r, g, b] = pal[usize::from(px[i])].map(i32::from);
        if r >= 150 && g >= 100 && r - b >= 50 {
            skin.push(i);
        } else if g > r && g > b + 25 {
            olive.push(i);
        } else if (r - g).abs() < 24 && (g - b).abs() < 24 {
            grey.push(i);
        }
    }
    (skin, olive, grey)
}

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
fn viewmodel_headless() {
    let wad = find_wad();
    let wad = wad.to_str().expect("utf-8 path");
    let mut h = Harness::start(&[
        "-iwad",
        wad,
        "-fixturepack",
        "-skill",
        "3",
        "-warp",
        "1",
        "1",
    ]);
    assert!(test_api::pack_loaded());
    assert!(test_api::mw2_owns_weapon(), "pistol is mapped (usp_mp)");
    let pistol = test_api::mapping()
        .expect("mapping")
        .mapped(1)
        .expect("wp_pistol mapped");

    // ---------------------------------------------------------------- T11.9
    h.run_tics(60);
    let frame = h.frame();
    let (wx, wy, ww, wh) = window();
    assert_eq!((wx, wy, ww, wh), (0, 0, 320, 168), "default view window");
    assert_eq!(h.sim_view().and_then(|v| v.weapon), Some(pistol));

    let gun_set = test_api::viewmodel_palette_set(pistol);
    assert!(!gun_set.is_empty(), "pistol prepared");
    let bg = sentinel(&gun_set);
    let (pixels, stats) = test_api::render_viewmodel(bg, GOLDEN_LIGHT).expect("gun up");
    let m = mask(&pixels, bg);
    assert!(m.len() > 500, "gun covers {} pixels", m.len());
    assert!(
        m.iter().all(|&i| i / 320 < wh),
        "gun stays inside the view window"
    );
    let (cx, cy) = centroid(&m);
    assert!(
        cx > (wx + ww / 2) as f32 && cy > (wy + wh / 2) as f32,
        "gun centroid ({cx:.0}, {cy:.0}) is in the lower right"
    );
    let in_frame = m
        .iter()
        .filter(|&&i| gun_set.contains(&frame.indices[i]))
        .count();
    assert_eq!(
        in_frame,
        m.len(),
        "every gun pixel of the real frame uses a quantized fixture colour"
    );
    match test_api::viewmodel_last_light() {
        Some(Light::Sector {
            directional: true, ..
        }) => {}
        other => panic!("idle light {other:?}"),
    }
    eprintln!(
        "T11.9: {} gun pixels, centroid ({cx:.0}, {cy:.0}), {} triangles in, {} drawn",
        m.len(),
        stats.triangles_in,
        stats.triangles_drawn
    );
    support::assert_golden("viewmodel_e1m1_t60", &frame, None);
    let (sbar, rect) = statusbar_without_maxima(&frame, wh);
    support::assert_golden("e1m1_t60_statusbar", &sbar, Some(rect));

    // ---------------------------------------------------------------- T11.8 idle
    let (idle, idle_mask) = golden_render("viewmodel_fixture_idle", bg);
    // Axes of the engine path (placement origin [forward, right, up], eye x fwd / y left /
    // z up): fullbright render, parts told apart by colour. The left hand (bind (22, -3, -8))
    // is right of and below the centre; the olive box in front of the muzzle sits up and to
    // the left of the gun body's mass: the barrel points into the screen towards the centre.
    // A mirrored or upside-down placement fails these.
    let (skin, olive, grey) = parts(bg);
    let (sx, sy) = centroid(&skin);
    let (ox, oy) = centroid(&olive);
    let (gx, gy) = centroid(&grey);
    eprintln!(
        "idle parts: hand ({sx:.0}, {sy:.0}) n={}, muzzle box ({ox:.0}, {oy:.0}) n={}, \
         gun ({gx:.0}, {gy:.0}) n={}",
        skin.len(),
        olive.len(),
        grey.len()
    );
    assert!(skin.len() > 50 && olive.len() > 20 && grey.len() > 500);
    assert!(
        sx > 160.0 && sy > 120.0,
        "left hand right of and below the centre"
    );
    assert!(ox < gx && oy < gy, "muzzle end up and left of the gun body");
    assert!(
        ox > 160.0 && oy > 84.0,
        "muzzle end right of and below the aim point"
    );
    let pal = test_palette_rgb();
    let reds = idle_mask
        .iter()
        .filter(|&&i| is_red(pal[usize::from(idle[i])]))
        .count();
    eprintln!("idle: {reds} red marker pixels");

    // Determinism: the same pose renders the same bytes.
    let (again, _) = test_api::render_viewmodel(bg, GOLDEN_LIGHT).unwrap();
    assert_eq!(again, idle);

    // Per-draw time through the engine path (pose + rasterizer).
    let mut micros: Vec<u32> = (0..200)
        .map(|_| {
            test_api::render_viewmodel(bg, GOLDEN_LIGHT)
                .unwrap()
                .1
                .micros
        })
        .collect();
    micros.sort_unstable();
    eprintln!(
        "fixture gun draw: median {} us, min {} us, max {} us ({} build)",
        micros[100],
        micros[0],
        micros[199],
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    // ---------------------------------------------------------------- dark sector, firing
    // The start sector is at full light (colormap 0). Dim it to 96: the gun darkens, and the
    // muzzle light of a shot brightens it again.
    let sector = {
        let mo = player_mut().mo.cast::<support::Mobj>();
        // SAFETY: the player's live map object, subsector and sector (engine thread).
        unsafe { (*(*mo).subsector).sector }
    };
    // SAFETY: as above; plain field access between engine calls.
    let bright = unsafe { (*sector).lightlevel };
    // SAFETY: as above.
    unsafe { (*sector).lightlevel = 96 };
    h.run_tics(1);
    let _ = h.frame();
    let idle_level = match test_api::viewmodel_last_light() {
        Some(Light::Sector { level, .. }) => level,
        other => panic!("idle light {other:?}"),
    };
    assert!(
        idle_level > 8,
        "a dark sector darkens the gun: level {idle_level}"
    );
    h.clear_events();
    h.key(KEY_FIRE, true);
    let mut fired_at = None;
    for _ in 0..10 {
        h.run_tics(1);
        if h.events()
            .iter()
            .any(|e| matches!(e.event, SimEvent::MuzzleFlash { .. }))
        {
            fired_at = Some(h.level_time());
            break;
        }
    }
    h.key(KEY_FIRE, false);
    assert!(fired_at.is_some(), "the pistol fired");
    let (_, flash) = test_api::muzzle();
    assert!(flash > 0, "flash timer up");
    let _ = h.frame(); // the frame of the firing tic was drawn by the hook
    let fire_level = match test_api::viewmodel_last_light() {
        Some(Light::Sector { level, .. }) => level,
        other => panic!("fire light {other:?}"),
    };
    assert!(
        fire_level < idle_level,
        "firing brightens the gun: level {fire_level} vs idle {idle_level}"
    );
    eprintln!("colormap level at sector light 96: idle {idle_level}, firing {fire_level}");
    golden_render("viewmodel_fixture_fire", bg);
    // SAFETY: as above.
    unsafe { (*sector).lightlevel = bright };
    h.run_tics(10);

    // ---------------------------------------------------------------- T11.8 mid-reload
    h.clear_events();
    tap(&mut h, b'r');
    let mut started = None;
    for _ in 0..10 {
        if let Some(e) = h
            .events()
            .iter()
            .find(|e| matches!(e.event, SimEvent::ReloadStarted { .. }))
        {
            started = Some(e.tic);
            break;
        }
        h.run_tics(1);
    }
    let started = started.expect("reload started");
    let wait = (started + 14 - h.level_time()).max(0) as u32;
    h.run_tics(wait);
    let (_, reload_mask) = golden_render("viewmodel_fixture_reload", bg);
    assert_ne!(reload_mask, idle_mask, "the reload pose differs from idle");
    h.run_tics(60);

    // ---------------------------------------------------------------- T11.8 ADS
    h.mouse(0, 0, 0b10);
    let mut n = 0;
    while h.sim_view().map_or(0.0, |v| v.ads_frac) < 1.0 {
        h.run_tics(1);
        n += 1;
        assert!(n < 60, "ADS did not complete");
    }
    h.run_tics(5);
    let (_, ads_mask) = golden_render("viewmodel_fixture_ads", bg);
    let (_, olive, grey) = parts(bg);
    let (ox, oy) = centroid(&olive);
    let (gx, gy) = centroid(&grey);
    eprintln!("ads parts: muzzle box ({ox:.0}, {oy:.0}), gun ({gx:.0}, {gy:.0})");
    assert!(
        (ox - 160.0).abs() < 6.0 && oy < gy,
        "ADS: muzzle end centred, above the body"
    );
    let (ax, _) = centroid(&ads_mask);
    let (ix, _) = centroid(&idle_mask);
    assert!(
        (ax - 160.0).abs() < (ix - 160.0).abs() && (ax - 160.0).abs() < 20.0,
        "ADS centres the gun: x {ax:.0} (idle {ix:.0})"
    );
    h.mouse(0, 0, 0);
    h.run_tics(20);

    // ---------------------------------------------------------------- invulnerability
    player_mut().powers[PW_INVULNERABILITY] = 200;
    h.run_tics(2);
    let frame = h.frame();
    assert_eq!(test_api::viewmodel_last_light(), Some(Light::Fixed(32)));
    let pal_tables = test_api::palette().expect("palette");
    let invuln: BTreeSet<u8> = pal_tables.colormaps[32].iter().copied().collect();
    let (pixels, _) = test_api::render_viewmodel(bg, Light::Fixed(32)).unwrap();
    let m = mask(&pixels, bg);
    assert!(
        m.iter().all(|&i| invuln.contains(&frame.indices[i])),
        "invulnerability: gun pixels come from colormap 32"
    );
    player_mut().powers[PW_INVULNERABILITY] = 0;
    h.run_tics(2);

    // ---------------------------------------------------------------- partial invisibility
    player_mut().powers[PW_INVISIBILITY] = 1000;
    h.run_tics(2);
    let _ = h.frame();
    assert_eq!(test_api::viewmodel_last_light(), Some(Light::Fuzz));
    assert!(
        test_api::viewmodel_last_stats().is_some_and(|s| s.pixels > 0),
        "fuzz drew"
    );
    player_mut().powers[PW_INVISIBILITY] = 0;
    h.run_tics(2);
}
