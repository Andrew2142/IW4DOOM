// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 12 headless tests T12.2 to T12.5 and T12.7 (T12.1 and T12.6 are unit tests next to
//! the code): the real engine with the synthetic fixture pack, frames read back from
//! `I_VideoBuffer`. One `#[test]` runs every scenario through one `Harness` (sheet 13
//! section 5.2). Goldens are the 320x168 view window; the status bar tests compare the AMMO
//! widget region.

mod support;

use dmw_format::{DmwWeaponId, Pack};
use dmw_weapons::{SoundCue, ViewState};
use room::dmw::hud::{self, crosshair_layout, CrosshairLayout, HitFeedback, HudColours, ViewRect};
use room::dmw::{audio, mode, test_api, with_state};
use room::doom::d_main::D_Display;
use room::doom::d_player::{consoleplayer, players, PlayerT};
use room::doom::doomkeys::{KEY_FIRE, KEY_RSHIFT};
use room::doom::p_inter::P_GiveWeapon;
use support::locations::OPEN_ROOM;
use support::{assert_golden, Frame, Harness, Rect, SimEvent};

const VIEW: ViewRect = ViewRect {
    x: 0,
    y: 0,
    w: 320,
    h: 168,
};
const VIEW_CROP: Rect = Rect::new(0, 0, 320, 168);
const READY: i32 = 0;
const WP_SHOTGUN: i32 = 2;

fn player() -> *mut PlayerT {
    // SAFETY: process-lifetime static, single engine thread.
    unsafe {
        (&raw mut players)
            .cast::<PlayerT>()
            .add(consoleplayer as usize)
    }
}

fn view(h: &Harness) -> ViewState {
    h.sim_view().expect("pack loaded")
}

fn tap(h: &mut Harness, key: u8) {
    h.key(key, true);
    h.run_tics(2);
    h.key(key, false);
}

fn wait_for(h: &mut Harness, max: u32, what: &str, mut pred: impl FnMut(&Harness) -> bool) {
    for _ in 0..max {
        if pred(h) {
            return;
        }
        h.run_tics(1);
    }
    assert!(pred(h), "timed out after {max} tics waiting for {what}");
}

fn shots(h: &Harness) -> usize {
    h.events()
        .iter()
        .filter(|e| matches!(e.event, SimEvent::Shot(_)))
        .count()
}

/// The crosshair layout the HUD computes for the current sim state.
fn layout_of(pack: &Pack, id: DmwWeaponId, v: &ViewState) -> CrosshairLayout {
    crosshair_layout(
        VIEW,
        v.zoom_hfov_deg.unwrap_or(90.0),
        v.spread_deg,
        v.gun_angle_offset,
        &pack.facts(id).hud,
    )
}

/// The twelve pixels of the arms: three per arm starting at the gap.
fn arm_pixels(l: &CrosshairLayout) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for k in l.gap..l.gap + l.length.min(3) {
        for (x, y) in [
            (l.cx, l.cy - k),
            (l.cx, l.cy + k),
            (l.cx - k, l.cy),
            (l.cx + k, l.cy),
        ] {
            out.push((x as usize, y as usize));
        }
    }
    out
}

fn cross_index() -> u8 {
    with_state(|s| HudColours::new(s.palette.as_ref().expect("palette")).cross).expect("state")
}

/// The current game state rendered with the HUD (first) and without it (second).
fn with_and_without(_h: &mut Harness) -> (Frame, Frame) {
    hud::test_set_overlays_enabled(false);
    D_Display();
    let without = grab();
    hud::test_set_overlays_enabled(true);
    D_Display();
    let with = grab();
    (with, without)
}

/// `Harness::frame` without needing the harness borrow.
fn grab() -> Frame {
    // SAFETY: `I_VideoBuffer` is a live 320x200 allocation while a level runs.
    unsafe {
        let indices = std::slice::from_raw_parts(room::doom::i_video::I_VideoBuffer, 320 * 200);
        Frame {
            indices: indices.to_vec(),
            palette: room::doom::i_video::test_palette_rgb(),
        }
    }
}

/// All arm pixels are the crosshair colour in `with` and were something else in `without`.
fn has_crosshair(h: &mut Harness, l: &CrosshairLayout) -> bool {
    let c = cross_index();
    let (with, without) = with_and_without(h);
    arm_pixels(l)
        .iter()
        .all(|&(x, y)| with.at(x, y) == c && without.at(x, y) != c)
}

/// No arm pixel differs between the frames drawn with and without the HUD.
fn no_crosshair(h: &mut Harness, l: &CrosshairLayout) -> bool {
    let (with, without) = with_and_without(h);
    arm_pixels(l)
        .iter()
        .all(|&(x, y)| with.at(x, y) == without.at(x, y))
}

fn lum(f: &Frame, x: usize, y: usize) -> [u8; 3] {
    f.palette[usize::from(f.at(x, y))]
}

#[test]
fn hud_headless() {
    let wad = support::find_wad();
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
    let pack = Pack::fixture();
    let pistol = pack.weapon_id("fixture_pistol").expect("pistol");
    let rifle = pack.weapon_id("fixture_rifle").expect("rifle");
    let sniper = pack.weapon_id("fixture_sniper").expect("sniper");
    assert!(mode::weapons_active(), "MW2 mode on: {:?}", mode::reason());

    // T12.7 (level start): the first raise of the level shows no weapon name.
    h.run_tics(5);
    assert_eq!(hud::last_message(), "", "no message at level start");

    OPEN_ROOM.go(&mut h);
    h.give_all();
    h.run_tics(2);

    // T12.7: switching to the rifle shows its display name.
    h.clear_events();
    tap(&mut h, b'4');
    wait_for(&mut h, 60, "rifle raise", |h| {
        h.events()
            .iter()
            .any(|e| e.event == SimEvent::RaiseStarted(rifle))
    });
    assert_eq!(hud::last_message(), pack.facts(rifle).display_name);
    wait_for(&mut h, 90, "rifle ready", |_| view_ready(&view_state()));
    // Let the message line (about 4 s) and the muzzle-free idle settle before the goldens.
    h.run_tics(170);

    // ---- T12.2: crosshair at rest ----
    let v = view(&h);
    assert_eq!(v.weapon, Some(rifle));
    assert!(!v.overlay_active && v.ads_frac == 0.0);
    let rest = layout_of(&pack, rifle, &v);
    assert!(
        has_crosshair(&mut h, &rest),
        "rest crosshair at {rest:?}, spread {}",
        v.spread_deg
    );
    let frame = h.frame();
    assert_golden("hud_crosshair_rest", &frame, Some(VIEW_CROP));

    // ---- T12.2: ten hip shots bloom the crosshair ----
    h.clear_events();
    h.key(KEY_FIRE, true);
    for _ in 0..80 {
        if shots(&h) >= 10 {
            break;
        }
        h.run_tics(1);
    }
    h.key(KEY_FIRE, false);
    assert!(shots(&h) >= 10, "ten rifle shots");
    h.run_tics(3);
    let v = view(&h);
    let bloom = layout_of(&pack, rifle, &v);
    assert!(
        bloom.gap_f > rest.gap_f + 1.0,
        "bloomed gap {} vs rest {}",
        bloom.gap_f,
        rest.gap_f
    );
    assert!(
        has_crosshair(&mut h, &bloom),
        "bloom crosshair at {bloom:?}"
    );
    let frame = h.frame();
    assert_golden("hud_crosshair_bloom", &frame, Some(VIEW_CROP));

    // It settles again when standing still.
    h.run_tics(120);
    let v = view(&h);
    let settled = layout_of(&pack, rifle, &v);
    assert!(settled.gap_f < bloom.gap_f, "spread decays");
    assert!(has_crosshair(&mut h, &settled));

    // ---- crosshair hides while sprinting and while aiming ----
    h.teleport(OPEN_ROOM.x, OPEN_ROOM.y, OPEN_ROOM.angle_deg);
    h.run_tics(2);
    h.key(KEY_RSHIFT, true);
    h.key(b'w', true);
    wait_for(&mut h, 120, "sprinting", |_| {
        test_api::last_out().is_some_and(|o| o.sprinting)
    });
    h.run_tics(2);
    let v = view(&h);
    let l = layout_of(&pack, rifle, &v);
    assert!(no_crosshair(&mut h, &l), "no crosshair while sprinting");
    h.key(b'w', false);
    h.key(KEY_RSHIFT, false);
    h.run_tics(60);
    h.teleport(OPEN_ROOM.x, OPEN_ROOM.y, OPEN_ROOM.angle_deg);
    h.run_tics(30);
    let l = layout_of(&pack, rifle, &view(&h));
    assert!(has_crosshair(&mut h, &l), "crosshair back after the sprint");
    h.mouse(0, 0, 2);
    wait_for(&mut h, 35, "ads past the crosshair threshold", |_| {
        test_api::view_state().is_some_and(|v| v.ads_frac >= 0.5)
    });
    let v = view(&h);
    let l = layout_of(&pack, rifle, &v);
    assert!(
        no_crosshair(&mut h, &l),
        "no crosshair while aiming (ads {})",
        v.ads_frac
    );
    h.mouse(0, 0, 0);
    h.run_tics(40);

    // ---- T12.3: hitmarker for 6 tics after a hit, 9 after a kill ----
    hitmarker_tics(&mut h, &pack, HitFeedback::default(), 6, false);
    hitmarker_tics(
        &mut h,
        &pack,
        HitFeedback {
            headshot: false,
            killed: true,
        },
        9,
        false,
    );
    hitmarker_tics(
        &mut h,
        &pack,
        HitFeedback {
            headshot: true,
            killed: false,
        },
        6,
        true,
    );

    // ---- reload prompt: absent with a full magazine, blinks at 2 Hz once it is low ----
    let prompt_up = |h: &mut Harness| {
        let v = view(h);
        let l = layout_of(&pack, v.weapon.expect("weapon"), &v);
        let (with, without) = with_and_without(h);
        let cx = l.cx as usize;
        let top = hud::prompt_top(&l) as usize;
        (top..top + 9).any(|y| (cx - 30..cx + 30).any(|x| with.at(x, y) != without.at(x, y)))
    };
    h.run_tics(120);
    let full: Vec<bool> = (0..20)
        .map(|_| {
            h.run_tics(1);
            prompt_up(&mut h)
        })
        .collect();
    assert!(
        full.iter().all(|&b| !b),
        "no prompt with {} rounds",
        view(&h).clip
    );
    while view(&h).clip > 5 {
        tap(&mut h, KEY_FIRE);
        h.run_tics(6);
    }
    assert!(view(&h).clip <= 5 && view(&h).stock > 0);
    h.run_tics(150);
    let low: Vec<bool> = (0..36)
        .map(|_| {
            h.run_tics(1);
            prompt_up(&mut h)
        })
        .collect();
    let lit = low.iter().filter(|&&b| b).count();
    assert!(
        (14..=22).contains(&lit),
        "about half of a second is lit: {lit} of 36: {low:?}"
    );
    let flips = low.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        (3..=5).contains(&flips),
        "about 4 flips in a second: {flips}"
    );

    // ---- T12.4: status bar AMMO shows the magazine, the pools table is unchanged ----
    tap(&mut h, b'2');
    wait_for(&mut h, 90, "pistol ready", |_| {
        view_ready(&view_state()) && view_state().weapon == Some(pistol)
    });
    h.run_tics(30);
    let ammo_box = Rect::new(2, 171, 100, 24);
    let pools_box = Rect::new(268, 171, 52, 29);
    let before_ready = hud::ready_ammo();
    assert_eq!(before_ready, Some(view_state().clip));
    h.run_tics(1);
    let f0 = h.frame();
    assert_eq!(view_state().clip, 12);
    assert_eq!(hud::ready_ammo(), Some(12));
    let pool0 = unsafe { (*player()).ammo[0] };
    tap(&mut h, KEY_FIRE);
    h.run_tics(10);
    let f1 = h.frame();
    assert_eq!(hud::ready_ammo(), Some(11), "magazine after one shot");
    assert_eq!(
        unsafe { (*player()).ammo[0] },
        pool0,
        "the pool is untouched by the shot"
    );
    assert_ne!(
        region(&f0, ammo_box),
        region(&f1, ammo_box),
        "AMMO widget changed 12 -> 11"
    );
    assert_eq!(
        region(&f0, pools_box),
        region(&f1, pools_box),
        "the pools table did not change"
    );
    // A vanilla weapon uses the vanilla widget.
    tap(&mut h, b'1');
    wait_for(&mut h, 60, "vanilla slot 1 (chainsaw after IDKFA)", |h| {
        !test_api::mw2_owns_weapon() && matches!(h.player().readyweapon, 0 | 7)
    });
    h.run_tics(5);
    assert_eq!(
        hud::ready_ammo(),
        None,
        "a vanilla weapon shows the vanilla widget"
    );
    let f2 = h.frame();
    assert_ne!(region(&f1, ammo_box), region(&f2, ammo_box));

    // ---- fullscreen (no status bar): magazine and reserve digits bottom right ----
    tap(&mut h, b'4');
    wait_for(&mut h, 90, "rifle ready", |_| {
        view_ready(&view_state()) && view_state().weapon == Some(rifle)
    });
    h.run_tics(170);
    set_screenblocks(&mut h, 11);
    // SAFETY: plain int reads on the engine thread.
    assert_eq!(
        unsafe { room::doom::r_draw::viewheight },
        200,
        "fullscreen view"
    );
    let (with, without) = with_and_without(&mut h);
    let digits = Rect::new(240, 150, 76, 46);
    let changed = region(&with, digits)
        .iter()
        .zip(region(&without, digits))
        .filter(|(a, b)| **a != *b)
        .count();
    std::fs::create_dir_all(support::golden::artifact_dir()).expect("artifact dir");
    std::fs::write(
        support::golden::artifact_dir().join("hud_fullscreen_counter.png"),
        with.to_png(None),
    )
    .expect("write frame");
    assert!(changed > 60, "fullscreen counter drew {changed} pixels");
    set_screenblocks(&mut h, 10);

    // ---- T12.7: pickup message ----
    hud_message_pickup(&mut h, &pack);

    // ---- T12.5: sniper scope ----
    OPEN_ROOM.go(&mut h);
    h.give_all();
    h.run_tics(2);
    tap(&mut h, b'7');
    wait_for(&mut h, 120, "sniper ready", |_| {
        let v = view_state();
        v.weapon == Some(sniper) && view_ready(&v)
    });
    h.run_tics(30);
    h.mouse(0, 0, 2);
    wait_for(&mut h, 120, "scope overlay up", |_| {
        view_state().overlay_active
    });
    // Wait out the weapon-name message line so the top rows are the scope's.
    h.run_tics(160);
    let v = view(&h);
    assert!(v.overlay_active && v.ads_frac >= 1.0);
    assert!(
        h.player().psprites.iter().all(|p| p.state.is_null()),
        "no vanilla psprite is drawn while the scope is up"
    );
    scope_checks(&mut h);
    h.mouse(0, 0, 0);
    h.run_tics(10);

    // Menus and HUD text draw over the crosshair: the overlays are drawn before HU_Drawer and
    // M_Drawer in D_Display (ordering is by construction); here only audio is left.
    audio_smoke();
}

/// Sets Doom's screen size through the real keys: `=` grows, `-` shrinks (10 = status bar,
/// 11 = fullscreen).
fn set_screenblocks(h: &mut Harness, blocks: i32) {
    for _ in 0..4 {
        // SAFETY: plain int read on the engine thread.
        let now = unsafe { room::doom::m_menu::screenblocks };
        if now == blocks {
            break;
        }
        tap(h, if now < blocks { 0x3d } else { 0x2d });
    }
    h.run_tics(3);
}

/// The sim's current view state.
fn view_state() -> ViewState {
    test_api::view_state().expect("pack loaded")
}

fn view_ready(v: &ViewState) -> bool {
    v.weaponstate == READY
}

/// Pixels of `r` as a flat vector.
fn region(f: &Frame, r: Rect) -> Vec<u8> {
    let mut out = Vec::new();
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            out.push(f.at(x, y));
        }
    }
    out
}

/// Starts the hitmarker through `hud::on_hit` (what sheet 09 calls per damaging hit) and counts
/// the frames it is drawn in.
fn hitmarker_tics(
    h: &mut Harness,
    pack: &Pack,
    feedback: HitFeedback,
    expect_tics: usize,
    headshot: bool,
) {
    h.run_tics(12);
    assert!(!hud::hitmarker_active());
    hud::on_hit(feedback);
    let mut up = 0;
    let mut seen_colour = None;
    for _ in 0..20 {
        h.run_tics(1);
        let v = view(h);
        let id = v.weapon.expect("weapon");
        let l = layout_of(pack, id, &v);
        let (mx, my) = ((l.cx + 4) as usize, (l.cy + 4) as usize);
        let (with, without) = with_and_without(h);
        let drawn = with.at(mx, my) != without.at(mx, my);
        assert_eq!(
            drawn,
            hud::hitmarker_active(),
            "marker state matches the pixels"
        );
        if drawn {
            up += 1;
            seen_colour.get_or_insert(lum(&with, mx, my));
        }
    }
    assert_eq!(up, expect_tics, "hitmarker frames for {feedback:?}");
    let [r, g, b] = seen_colour.expect("marker colour");
    if headshot {
        assert!(
            r > 150 && g < 120 && b < 120,
            "headshot is red: {:?}",
            (r, g, b)
        );
    } else {
        assert!(
            r > 200 && g > 200 && b > 200,
            "plain hit is white: {:?}",
            (r, g, b)
        );
    }
}

/// T12.7 pickup: `P_GiveWeapon` adds the shotgun, the message names it.
fn hud_message_pickup(h: &mut Harness, pack: &Pack) {
    h.new_game(2, 1, 1);
    h.run_tics(5);
    assert_eq!(hud::last_message(), "");
    // SAFETY: live console player during a running level.
    unsafe { P_GiveWeapon(&mut *player(), WP_SHOTGUN, 0) };
    h.run_tics(1);
    let name = &pack
        .facts(pack.weapon_id("fixture_shotgun").expect("shotgun"))
        .display_name;
    assert!(
        hud::last_message() == format!("You got the {name}!") || hud::last_message() == *name,
        "pickup message, then the auto-switch name: {:?}",
        hud::last_message()
    );
}

/// T12.5 on the real frame: columns outside the overlay are black, the centre shows the world.
fn scope_checks(h: &mut Harness) {
    let pal_black =
        with_state(|s| HudColours::new(s.palette.as_ref().expect("palette")).black).expect("state");
    D_Display();
    let with = h.frame();
    hud::test_set_overlays_enabled(false);
    D_Display();
    let without = h.frame();
    hud::test_set_overlays_enabled(true);
    let rect = hud::scope_rect(VIEW, 256, 192, true);
    assert!(
        rect.w < VIEW.w && rect.x > 0,
        "the scope is narrower than the view: {rect:?}"
    );
    for y in 0..168usize {
        for x in (0..rect.x as usize).chain((rect.x + rect.w) as usize..320) {
            assert_eq!(
                with.at(x, y),
                pal_black,
                "column {x} row {y} outside the scope is black"
            );
        }
    }
    // The centre of the scope is transparent: the world shows (identical to the frame drawn
    // without overlays), apart from the one-pixel reticle lines.
    let (cx, cy) = (160usize, 84usize);
    let _ = pal_black;
    let mut compared = 0;
    for y in cy - 30..cy + 30 {
        for x in cx - 30..cx + 30 {
            if x.abs_diff(cx) <= 2 || y.abs_diff(cy) <= 2 {
                continue;
            }
            assert_eq!(with.at(x, y), without.at(x, y), "world shows at ({x}, {y})");
            compared += 1;
        }
    }
    assert!(compared > 2000);
    // The ring and mask do cover the world elsewhere.
    let changed = (0..168usize)
        .flat_map(|y| (0..320usize).map(move |x| (x, y)))
        .filter(|&(x, y)| with.at(x, y) != without.at(x, y))
        .count();
    assert!(changed > 20_000, "overlay changed {changed} pixels");
    assert_golden("hud_scope_fixture", &with, Some(VIEW_CROP));
}

/// Audio runs without a device: aliases resolve, unknown ones are reported, nothing panics.
fn audio_smoke() {
    let known = SoundCue {
        alias: "fixture_fire_pistol".to_owned(),
        local: true,
    };
    assert!(audio::has_alias("fixture_fire_pistol"));
    assert!(audio::play_local(&known), "known alias");
    let positional = SoundCue {
        alias: "fixture_fire_pistol".to_owned(),
        local: false,
    };
    assert!(audio::play_local(&positional));
    let unknown = SoundCue {
        alias: "no_such_alias".to_owned(),
        local: true,
    };
    assert!(!audio::play_local(&unknown), "unknown alias reports false");
    assert!(
        !audio::has_alias(audio::HITMARKER_ALIAS),
        "the fixture has no hit alert"
    );
}
