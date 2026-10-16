// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 11 section 6.3, headless with `-fixturepack`: the procedural MW2 muzzle flash through
//! the engine's draw path. Mid-flash goldens of the fixture pistol, rifle and shotgun
//! (`muzzle_flash = "mw2"`), the rifle with `"doom"` and `"none"`, size and position checks for
//! every weapon class, and a replay check that a plain re-render is identical.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

mod support;

use std::collections::BTreeSet;

use room::dmw::config::FlashCfg;
use room::dmw::test_api;
use room::dmw::viewmodel::Light;
use room::doom::doomkeys::KEY_FIRE;
use room::doom::i_video::test_palette_rgb;
use room::doom::r_draw::{scaledviewwidth, viewheight, viewwindowx, viewwindowy};

use support::golden::{assert_golden_rgb_in, golden_dir};
use support::{Frame, Harness, Rect, SimEvent, find_wad};

const LIGHT: Light = Light::Sector {
    level: 8,
    directional: true,
};

fn tap(h: &mut Harness, key: u8) {
    h.key(key, true);
    h.run_tics(2);
    h.key(key, false);
}

fn golden(name: &str, pixels: &[u8]) {
    let frame = Frame {
        indices: pixels.to_vec(),
        palette: test_palette_rgb(),
    };
    // SAFETY: plain reads of renderer globals on the engine thread.
    let (x, y, w, h) = unsafe {
        (
            viewwindowx as usize,
            viewwindowy as usize,
            scaledviewwidth as usize,
            viewheight as usize,
        )
    };
    let (cw, ch, rgb) = frame.to_rgb(Some(Rect::new(x, y, w, h)));
    assert_golden_rgb_in(&golden_dir(), name, cw, ch, &rgb, 0);
}

/// (count, bounding width, bounding height, centroid) of the pixels that differ.
fn diff_stats(a: &[u8], b: &[u8]) -> (usize, usize, usize, (f32, f32)) {
    let idx: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    assert!(!idx.is_empty(), "no pixels differ");
    let xs = idx.iter().map(|i| i % 320);
    let ys = idx.iter().map(|i| i / 320);
    let (x0, x1) = (xs.clone().min().unwrap(), xs.clone().max().unwrap());
    let (y0, y1) = (ys.clone().min().unwrap(), ys.clone().max().unwrap());
    let n = idx.len() as f32;
    let cx = xs.map(|x| x as f32).sum::<f32>() / n;
    let cy = ys.map(|y| y as f32).sum::<f32>() / n;
    (idx.len(), x1 - x0 + 1, y1 - y0 + 1, (cx, cy))
}

/// Centroid of the olive muzzle box of the fixture gun (fullbright, flash off): the barrel tip.
fn muzzle_box(bg: u8) -> (f32, f32) {
    test_api::set_muzzle_flash(FlashCfg::None);
    let (px, _) = test_api::render_viewmodel(bg, Light::Fixed(0)).expect("gun up");
    let pal = test_palette_rgb();
    let olive: Vec<usize> = (0..px.len())
        .filter(|&i| {
            let [r, g, b] = pal[usize::from(px[i])].map(i32::from);
            px[i] != bg && g > r && g > b + 25
        })
        .collect();
    assert!(olive.len() > 5, "muzzle box visible");
    let n = olive.len() as f32;
    (
        olive.iter().map(|i| (i % 320) as f32).sum::<f32>() / n,
        olive.iter().map(|i| (i / 320) as f32).sum::<f32>() / n,
    )
}

/// Selects the Doom weapon on `key` and waits for it to be up.
fn select(h: &mut Harness, key: u8, slot: i32) {
    let id = test_api::mapping()
        .expect("mapping")
        .mapped(slot)
        .unwrap_or_else(|| panic!("slot {slot} mapped"));
    for _ in 0..8 {
        if h.sim_view().and_then(|v| v.weapon) == Some(id) {
            break;
        }
        tap(h, key);
        h.run_tics(40);
    }
    assert_eq!(h.sim_view().and_then(|v| v.weapon), Some(id), "slot {slot}");
    h.run_tics(60);
}

/// Fires until the muzzle flash starts; the next draw is the first flash frame.
fn fire(h: &mut Harness) {
    h.clear_events();
    h.key(KEY_FIRE, true);
    let mut fired = false;
    for _ in 0..20 {
        h.run_tics(1);
        if h.events()
            .iter()
            .any(|e| matches!(e.event, SimEvent::MuzzleFlash { .. }))
        {
            fired = true;
            break;
        }
    }
    h.key(KEY_FIRE, false);
    assert!(fired, "weapon fired");
    assert!(test_api::muzzle().1 > 0, "flash timer up");
}

#[test]
fn viewmodel_flash_headless() {
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
    assert!(test_api::mw2_owns_weapon());
    h.run_tics(60);
    h.give_all();

    // (name, key, Doom slot, golden?, max flash width at the fixture's muzzle distance)
    let weapons: [(&str, u8, i32, bool, usize); 5] = [
        ("pistol", b'2', 1, true, 9),
        ("shotgun", b'3', 2, true, 18),
        ("rifle", b'4', 3, true, 14),
        ("rocket", b'5', 4, false, 12),
        ("sniper", b'7', 6, false, 7),
    ];
    let mut sizes = Vec::new();
    for (name, key, slot, want_golden, max_w) in weapons {
        select(&mut h, key, slot);
        let id = h.sim_view().and_then(|v| v.weapon).expect("weapon");
        let gun: BTreeSet<u8> = test_api::viewmodel_palette_set(id);
        let bg = (0..=255u8).find(|i| !gun.contains(i)).expect("free index");
        fire(&mut h);

        test_api::set_muzzle_flash(FlashCfg::None);
        let (none, _) = test_api::render_viewmodel(bg, LIGHT).unwrap();
        test_api::set_muzzle_flash(FlashCfg::Mw2);
        let (mw2, _) = test_api::render_viewmodel(bg, LIGHT).unwrap();
        let (n, w, hgt, (cx, cy)) = diff_stats(&none, &mw2);
        let (bx, by) = muzzle_box(bg);
        eprintln!(
            "{name}: flash {n} px, {w}x{hgt}, centre ({cx:.1}, {cy:.1}), barrel tip ({bx:.1}, {by:.1})"
        );
        assert!(w <= max_w && hgt <= max_w, "{name}: {w}x{hgt} > {max_w}");
        assert!(w >= 5 || name == "sniper", "{name}: visible");
        // The pistol, rifle and shotgun fixtures share the olive muzzle box 3 inches in front of
        // `tag_flash`; the launcher and sniper models differ.
        assert!(
            !want_golden || ((cx - bx).abs() < 10.0 && (cy - by).abs() < 10.0),
            "{name}: flash centred on the barrel tip"
        );
        // Only the flash changed, and a plain re-render is identical.
        test_api::set_muzzle_flash(FlashCfg::Mw2);
        let (again, _) = test_api::render_viewmodel(bg, LIGHT).unwrap();
        assert_eq!(again, mw2, "{name}: deterministic");
        sizes.push((name, w, hgt));

        if want_golden {
            golden(&format!("viewmodel_flash_{name}"), &mw2);
        }
        if name == "rifle" {
            test_api::set_muzzle_flash(FlashCfg::Doom);
            let (doom, _) = test_api::render_viewmodel(bg, LIGHT).unwrap();
            let (_, dw, _, _) = diff_stats(&none, &doom);
            assert!(dw > 14, "the Doom patch is the big one ({dw})");
            golden("viewmodel_flash_rifle_doom", &doom);
            golden("viewmodel_flash_rifle_none", &none);
            test_api::set_muzzle_flash(FlashCfg::Mw2);
        }

        h.run_tics(30);
    }
    eprintln!("flash sizes {sizes:?}");
}
