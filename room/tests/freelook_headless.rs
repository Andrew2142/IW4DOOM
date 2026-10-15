// SPDX-License-Identifier: GPL-3.0-or-later
//! Headless freelook, view kick and FOV zoom tests (sheet 06 section 9: T6.3-T6.6).
//!
//! MW2 mode is forced through the `test-hooks` override (sheet 05 section 11); sheet 08, which
//! writes `PlayerExt::view_kick_deg` and `zoom_hfov_deg` from the simulation, does not exist
//! yet, so this test writes both fields directly.

mod support;

use room::dmw::ext::{PlayerExt, PLAYER_EXT};
use room::dmw::freelook::{self, VANILLA_FINE_FOV};
use room::doom::d_main::D_Display;
use room::doom::d_player::consoleplayer;
use room::doom::g_game::players;
use room::doom::m_menu::{detailLevel, screenblocks};
use room::doom::r_draw::{viewheight, viewwidth, viewwindowx, viewwindowy};
use room::doom::r_main::{centery, detailshift, viewangle, R_SetViewSize};

use support::golden::{artifact_dir, Rect};
use support::locations::{OPEN_ROOM, SKY_VIEW};
use support::{assert_golden, Frame, Harness};

const FRACUNIT: i32 = 1 << 16;

fn ext_mut() -> &'static mut PlayerExt {
    // SAFETY: engine-thread-only static, plain data; the test is the engine thread and the
    // borrow is used before the next engine call.
    unsafe {
        &mut *(&raw mut PLAYER_EXT)
            .cast::<PlayerExt>()
            .add(consoleplayer as usize)
    }
}

/// Sets the stand-ins for sheet 08's per-tic writes.
fn set_kick_zoom(kick: [f32; 3], zoom: Option<f32>) {
    let ext = ext_mut();
    ext.view_kick_deg = kick;
    ext.zoom_hfov_deg = zoom;
}

/// The 3D view window of the current view size.
fn view_rect() -> Rect {
    // SAFETY: plain int reads on the engine thread.
    unsafe {
        Rect::new(
            viewwindowx as usize,
            viewwindowy as usize,
            (viewwidth << detailshift) as usize,
            viewheight as usize,
        )
    }
}

/// `None` when equal, else "N pixels differ in (x0,y0)-(x1,y1)" relative to the crop.
fn diff(a: &[u8], b: &[u8], r: Rect) -> Option<String> {
    let idx: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    if idx.is_empty() {
        return None;
    }
    let xs = idx.iter().map(|i| i % r.w);
    let ys = idx.iter().map(|i| i / r.w);
    Some(format!(
        "{} pixels differ in x {}..={}, y {}..={}",
        idx.len(),
        xs.clone().min().unwrap_or(0),
        xs.max().unwrap_or(0),
        ys.clone().min().unwrap_or(0),
        ys.max().unwrap_or(0)
    ))
}

/// One tic in the given mode/pitch/kick/zoom, then the frame it rendered.
fn render(h: &mut Harness, mw2: bool, pitch: f32, kick: [f32; 3], zoom: Option<f32>) -> Frame {
    h.set_mw2_mode(Some(mw2));
    h.set_pitch(pitch);
    set_kick_zoom(kick, zoom);
    h.run_tics(1);
    h.frame()
}

/// Redraws the current game state (no tic runs: `D_Display` only renders, and `NetUpdate`
/// does nothing in `singletics` mode) in the given mode/pitch/kick/zoom.
fn redraw(h: &mut Harness, mw2: bool, pitch: f32, kick: [f32; 3], zoom: Option<f32>) -> Frame {
    h.set_mw2_mode(Some(mw2));
    h.set_pitch(pitch);
    set_kick_zoom(kick, zoom);
    D_Display();
    h.frame()
}

/// T6.4: at pitch 0 without zoom or kick, an MW2-mode frame is byte-identical to a vanilla one,
/// also right after a sheared and zoomed MW2 frame (the tables are restored exactly).
fn assert_parity(h: &mut Harness, what: &str) {
    let full = Rect::new(0, 0, 320, 200);
    let check = |a: &Frame, b: &Frame, msg: &str| {
        if let Some(d) = diff(&a.indices, &b.indices, full) {
            let stem: String = what
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            let dir = artifact_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(format!("parity_{stem}.a.png")), a.to_png(None));
            let _ = std::fs::write(dir.join(format!("parity_{stem}.b.png")), b.to_png(None));
            panic!("{what}: {msg}: {d} (frames in {})", dir.display());
        }
    };
    // Everything below renders the same game tic, so frames compare exactly (whole screen).
    render(h, false, 0.0, [0.0; 3], None);
    let off_a = redraw(h, false, 0.0, [0.0; 3], None);
    let off_b = redraw(h, false, 0.0, [0.0; 3], None);
    check(
        &off_a,
        &off_b,
        "redrawing the same tic is not deterministic",
    );
    let on = redraw(h, true, 0.0, [0.0; 3], None);
    check(&on, &off_a, "MW2 frame at pitch 0 differs from vanilla");
    // Dirty every table (shear, zoom, yaw kick), then come back to pitch 0 in MW2 mode.
    let dirty = redraw(h, true, -20.0, [2.0, 3.0, 0.0], Some(40.0));
    assert!(
        diff(&dirty.indices, &off_a.indices, full).is_some(),
        "{what}: dirty frame is dirty"
    );
    let on_after_dirty = redraw(h, true, 0.0, [0.0; 3], None);
    check(
        &on_after_dirty,
        &off_a,
        "MW2 frame at pitch 0 after zoom/shear differs from vanilla",
    );
    // Leaving MW2 mode while sheared and zoomed restores vanilla.
    redraw(h, true, 25.0, [0.0; 3], Some(30.0));
    let off_after = redraw(h, false, 0.0, [0.0; 3], None);
    check(
        &off_after,
        &off_a,
        "vanilla frame after leaving MW2 mode differs",
    );
    // Vanilla mode ignores pitch, kick and zoom entirely.
    let off_pitched = redraw(h, false, -25.0, [3.0, 3.0, 0.0], Some(30.0));
    check(
        &off_pitched,
        &off_a,
        "vanilla mode reacted to pitch/kick/zoom",
    );
    assert_eq!(freelook::view_params().shear_px, 0);
    assert_eq!(freelook::view_params().fine_fov(), VANILLA_FINE_FOV);
    h.set_pitch(0.0);
    set_kick_zoom([0.0; 3], None);
}

/// Pixels where the frame with psprites differs from the frame without them.
fn psprite_mask(h: &mut Harness, pitch: f32) -> Vec<(usize, u8)> {
    let with = redraw(h, true, pitch, [0.0; 3], None);
    // SAFETY: players[console].psprites is plain data; no tic runs while the state is null and
    // the pointer is put back right after the redraw.
    let saved = unsafe {
        let p = &mut players[consoleplayer as usize];
        let s = p.psprites[0].state;
        p.psprites[0].state = std::ptr::null_mut();
        s
    };
    let without = redraw(h, true, pitch, [0.0; 3], None);
    // SAFETY: as above.
    unsafe { players[consoleplayer as usize].psprites[0].state = saved };
    with.indices
        .iter()
        .zip(&without.indices)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, _))| (i, *a))
        .collect()
}

/// Mean palette luminance of `r` in `frame`.
fn mean_luma(frame: &Frame, r: Rect) -> f64 {
    let mut sum = 0.0;
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            let [cr, cg, cb] = frame.palette[usize::from(frame.at(x, y))];
            sum += 0.299 * f64::from(cr) + 0.587 * f64::from(cg) + 0.114 * f64::from(cb);
        }
    }
    sum / (r.w * r.h) as f64
}

#[test]
fn freelook_headless() {
    let mut h = Harness::start_default();
    OPEN_ROOM.go(&mut h);
    h.remove_monsters();
    // Let the pistol finish its raise so the psprite is static.
    h.run_tics(40);

    // ---- T6.4 parity at the default view size (320x168) ----
    // SAFETY: plain int reads on the engine thread.
    assert_eq!(
        unsafe { (viewwidth, viewheight, detailshift) },
        (320, 168, 0)
    );
    assert_parity(&mut h, "OPEN_ROOM 320x168");

    // ---- T6.3 goldens: pitch -20 (up), 0, +20 (down) ----
    let view = view_rect();
    let up = render(&mut h, true, -20.0, [0.0; 3], None);
    let vp = freelook::view_params();
    // tan(20 deg) * 160 = 58.2
    assert_eq!(vp.shear_px, 58, "looking up moves the horizon down");
    // SAFETY: plain int read.
    assert_eq!(unsafe { centery }, 84 + 58);
    assert_golden("freelook_up", &up, Some(view));
    let level = render(&mut h, true, 0.0, [0.0; 3], None);
    assert_eq!(freelook::view_params().shear_px, 0);
    assert_golden("freelook_level", &level, Some(view));
    let down = render(&mut h, true, 20.0, [0.0; 3], None);
    assert_eq!(freelook::view_params().shear_px, -58);
    assert_golden("freelook_down", &down, Some(view));
    // Aim is untouched: the pitch the player built is still the pitch (no kick), mo->angle too.
    assert_eq!(h.player_ext().pitch_q, 20 * 128);

    // ---- Vanilla psprites (the pistol here) do not move with pitch ----
    let mask_level = psprite_mask(&mut h, 0.0);
    assert!(
        mask_level.len() > 200,
        "the pistol covers {} pixels",
        mask_level.len()
    );
    // Every pixel where the level pistol differs from the world is a pistol pixel, whatever the
    // background, so a pistol that stayed put shows exactly those pixels at any pitch. (A pixel
    // where pistol and world happen to share an index drops out of a mask, so the masks
    // themselves may differ by a few pixels.)
    for pitch in [-25.0, 25.0] {
        let pitched = redraw(&mut h, true, pitch, [0.0; 3], None);
        assert_ne!(freelook::view_params().shear_px, 0);
        let moved = mask_level
            .iter()
            .filter(|&&(i, v)| pitched.indices[i] != v)
            .count();
        assert_eq!(
            moved, 0,
            "{moved} pistol pixels moved or changed at pitch {pitch}"
        );
        let mask = psprite_mask(&mut h, pitch);
        let diff = mask.len().abs_diff(mask_level.len());
        assert!(
            diff * 100 <= mask_level.len(),
            "pistol silhouette changed at pitch {pitch}: {} vs {} pixels",
            mask.len(),
            mask_level.len()
        );
    }

    // ---- Mouse look drives the shear smoothly (sheet 05 pitch -> sheet 06 shear) ----
    h.set_mw2_mode(Some(true));
    h.set_pitch(0.0);
    set_kick_zoom([0.0; 3], None);
    h.run_tics(1);
    let mut shears = vec![freelook::view_params().shear_px];
    for _ in 0..12 {
        h.mouse(0, 6, 0);
        h.run_tics(1);
        shears.push(freelook::view_params().shear_px);
    }
    let total = shears[shears.len() - 1];
    assert_ne!(total, 0, "mouse motion moved the horizon: {shears:?}");
    assert!(
        shears
            .windows(2)
            .all(|w| (w[1] - w[0]) * total.signum() >= 0 && (w[1] - w[0]).abs() <= 12),
        "horizon moves monotonically in small steps: {shears:?}"
    );

    // ---- View kick: yaw kick turns only the rendered view; pitch kick adds to the shear ----
    let angle_before = h.mobj().angle;
    render(&mut h, true, 0.0, [-3.0, 10.0, 5.0], None);
    // SAFETY: plain read of a renderer global.
    let rendered = unsafe { viewangle };
    assert_eq!(
        rendered.wrapping_sub(angle_before),
        (10.0f64 * 4294967296.0 / 360.0).round() as u32,
        "yaw kick is added to viewangle"
    );
    assert_eq!(h.mobj().angle, angle_before, "yaw kick never touches aim");
    assert_eq!(h.player_ext().pitch_q, 0, "pitch kick never touches aim");
    // -3 deg kick at pitch 0 = looking up 3 deg: round(tan(3) * 160) = 8
    assert_eq!(freelook::view_params().shear_px, 8);

    // ---- T6.5: 45-degree zoom golden and light parity ----
    render(&mut h, true, 0.0, [0.0; 3], None);
    let r90 = redraw(&mut h, true, 0.0, [0.0; 3], None);
    let z45 = redraw(&mut h, true, 0.0, [0.0; 3], Some(45.0));
    assert_eq!(freelook::view_params().fine_fov(), 1024);
    // The wall straight ahead (736 units away), around the horizon row 84. The zoom magnifies
    // by 1/tan(22.5) = 2.414, so the same patch of wall covers a 2.414x larger crop.
    let c90 = Rect::new(140, 81, 40, 6);
    let c45 = Rect::new(160 - 48, 84 - 7, 96, 14);
    let (l90, l45) = (mean_luma(&r90, c90), mean_luma(&z45, c45));
    let rel = (l45 - l90).abs() / l90;
    assert!(
        rel < 0.05,
        "zoom changed the wall brightness: {l90:.1} at 90 deg vs {l45:.1} at 45 deg ({:.1}%)",
        rel * 100.0
    );
    assert_golden("freelook_zoom45", &z45, Some(view));
    // Out-of-range zoom is clamped, never breaks the renderer.
    render(&mut h, true, 0.0, [0.0; 3], Some(f32::NAN));
    assert_eq!(freelook::view_params().fine_fov(), VANILLA_FINE_FOV);
    render(&mut h, true, 0.0, [0.0; 3], Some(0.5));
    assert_eq!(freelook::view_params().hfov_deg, 5.0);

    // ---- View size change mid-level: full screen, sheared, recomputed ----
    h.set_pitch(-20.0);
    // SAFETY: engine thread; D_Display applies the change before the next frame.
    unsafe { R_SetViewSize(11, 0) };
    render(&mut h, true, -20.0, [0.0; 3], None);
    let vp = freelook::view_params();
    assert_eq!((vp.base_centery, vp.shear_px), (100, 58));
    // SAFETY: plain int read.
    assert_eq!(unsafe { (viewheight, centery) }, (200, 158));
    assert_parity(&mut h, "OPEN_ROOM 320x200");

    // ---- Low detail: focal length uses viewwidth << detailshift ----
    // SAFETY: as above.
    unsafe { R_SetViewSize(10, 1) };
    render(&mut h, true, -20.0, [0.0; 3], None);
    // SAFETY: plain int reads.
    assert_eq!(unsafe { (viewwidth, detailshift) }, (160, 1));
    assert_eq!(freelook::view_params().shear_px, 58);
    assert_parity(&mut h, "OPEN_ROOM low detail");
    // SAFETY: as above; back to the defaults.
    unsafe {
        R_SetViewSize(screenblocks, detailLevel);
    }
    h.run_tics(1);

    // ---- T6.6: sky at the maximum up-pitch (input limit plus 5 deg of kick) has no seam ----
    SKY_VIEW.go(&mut h);
    h.remove_monsters();
    h.run_tics(2);
    assert_parity(&mut h, "SKY_VIEW");
    let sky = render(&mut h, true, -32.0, [-5.0, 0.0, 0.0], None);
    let vp = freelook::view_params();
    // tan(37 deg) * 160 = 120.6
    assert_eq!(vp.shear_px, 121);
    let (iscale, cy) = vp.last_sky().expect("a sky plane was drawn");
    assert_eq!(cy, 84 + 121);
    // Vanilla mapping would wrap: texture row 100 at the horizon, one texel per row.
    assert!(
        cy * FRACUNIT > 100 * FRACUNIT,
        "scenario exercises the wrap case"
    );
    // Screen row 0 maps to texture row >= 0 and the horizon row to row 100: no `& 127` wrap
    // anywhere between them (the sky plane only covers rows above the horizon).
    let top_texel = 100 * FRACUNIT - cy * iscale;
    assert!(
        (0..FRACUNIT).contains(&top_texel),
        "top sky row maps to texel {top_texel}"
    );
    assert_golden("freelook_sky_up", &sky, Some(view_rect()));

    h.set_pitch(0.0);
    set_kick_zoom([0.0; 3], None);
    h.set_mw2_mode(None);
    h.run_tics(1);
    assert_eq!(freelook::view_params().shear_px, 0);
}
