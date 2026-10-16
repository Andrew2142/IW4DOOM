// SPDX-License-Identifier: GPL-3.0-or-later
//! Headless Doom-integration tests (sheet 08 section 9: T8.1-T8.7, T8.9, T8.10 and the demo half
//! of T8.8). One engine per process, so the scenarios run in sequence through one harness
//! started with `-fixturepack` (the in-memory synthetic pack, no MW2 data).

mod support;

use std::ffi::{c_char, CStr};

use dmw_format::{DmwWeaponId, Pack};
use room::dmw::save::{parse, Trailer};
use room::dmw::{mode, test_api};
use room::doom::d_player::PlayerT;
use room::doom::doomkeys::KEY_FIRE;
use room::doom::g_game::{consoleplayer, players, G_DeferedPlayDemo, G_LoadGame, G_SaveGame};
use room::doom::p_inter::{P_DamageMobj, P_GiveWeapon};
use room::doom::p_saveg::P_SaveGameFile;

use support::locations::OPEN_ROOM;
use support::{find_wad, Harness, SimEvent};

const WP_FIST: i32 = 0;
const WP_PISTOL: i32 = 1;
const WP_SHOTGUN: i32 = 2;
const WP_CHAINGUN: i32 = 3;
const WP_CHAINSAW: i32 = 7;
const READY: i32 = 0; // weapon_iw4::WeaponState::Ready

struct Ids {
    pistol: DmwWeaponId,
    rifle: DmwWeaponId,
    shotgun: DmwWeaponId,
}

fn player_mut() -> &'static mut PlayerT {
    // SAFETY: engine-thread-only static; the test is the engine thread and uses the borrow
    // between engine calls only.
    unsafe {
        &mut *(&raw mut players)
            .cast::<PlayerT>()
            .add(consoleplayer as usize)
    }
}

fn psprites_null(h: &Harness) -> bool {
    h.player().psprites.iter().all(|p| p.state.is_null())
}

fn mag(id: DmwWeaponId) -> i32 {
    test_api::inventory().expect("pack loaded").magazine(id)
}

/// Key down for two tics (a one-tic press can fall between two command builds).
fn tap(h: &mut Harness, key: u8) {
    h.key(key, true);
    h.run_tics(2);
    h.key(key, false);
}

/// One trigger pull: fire held for one tic, then `wait` tics released.
fn fire_once(h: &mut Harness, wait: u32) {
    tap(h, KEY_FIRE);
    h.run_tics(wait);
}

fn count(h: &Harness, f: impl Fn(&SimEvent) -> bool) -> usize {
    h.events().iter().filter(|e| f(&e.event)).count()
}

/// Level tic of the first recorded event matching `f`.
fn first_tic(h: &Harness, f: impl Fn(&SimEvent) -> bool) -> Option<i32> {
    h.events().iter().find(|e| f(&e.event)).map(|e| e.tic)
}

/// Runs single tics until `pred` holds; returns the number of tics run.
fn wait_for(h: &mut Harness, max: u32, what: &str, mut pred: impl FnMut(&Harness) -> bool) -> u32 {
    for n in 0..max {
        if pred(h) {
            return n;
        }
        h.run_tics(1);
    }
    let p = h.player();
    assert!(
        pred(h),
        "timed out after {max} tics waiting for {what} (ready {} pending {} psprite {:?} mw2 {})",
        p.readyweapon,
        p.pendingweapon,
        p.psprites[0].state,
        test_api::mw2_owns_weapon()
    );
    max
}

fn raised(id: DmwWeaponId) -> impl Fn(&SimEvent) -> bool {
    move |e| *e == SimEvent::RaiseStarted(id)
}

fn save_path(slot: i32) -> String {
    // SAFETY: returns a NUL-terminated buffer owned by the engine.
    unsafe {
        CStr::from_ptr(P_SaveGameFile(slot))
            .to_string_lossy()
            .into_owned()
    }
}

fn save(h: &mut Harness, slot: i32) -> Vec<u8> {
    let path = save_path(slot);
    let _ = std::fs::remove_file(&path);
    // SAFETY: engine call on the engine thread with a NUL-terminated description.
    unsafe { G_SaveGame(slot, c"dmw test".as_ptr()) };
    h.run_tics(3);
    std::fs::read(&path).expect("savegame written")
}

fn load(h: &mut Harness, slot: i32) {
    let mut path = save_path(slot).into_bytes();
    path.push(0);
    // SAFETY: NUL-terminated path; G_LoadGame copies it.
    unsafe { G_LoadGame(path.as_mut_ptr().cast::<c_char>()) };
    h.run_tics(3);
}

#[test]
fn integration_headless() {
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
    let pack = Pack::fixture();
    let id = |n: &str| pack.weapon_id(n).expect("fixture weapon");
    let ids = Ids {
        pistol: id("fixture_pistol"),
        rifle: id("fixture_rifle"),
        shotgun: id("fixture_shotgun"),
    };

    // T8.1: level start raises the mapped pistol with MW2 timing; no vanilla psprite.
    assert!(test_api::pack_loaded());
    assert!(mode::weapons_active(), "MW2 mode on: {:?}", mode::reason());
    h.run_tics(3);
    assert!(
        count(&h, raised(ids.pistol)) >= 1,
        "RaiseStarted(pistol): {:?}",
        h.events()
    );
    assert_eq!(h.player().readyweapon, WP_PISTOL);
    assert!(psprites_null(&h));
    assert!(test_api::mw2_owns_weapon());
    OPEN_ROOM.go(&mut h);
    wait_for(&mut h, 60, "pistol ready", |_| {
        test_api::view_state().is_some_and(|v| v.weaponstate == READY)
    });

    // T8.2: 5 shots spend the magazine, not the pool; reload refills from the pool.
    let pool0 = h.player().ammo[0];
    assert_eq!(mag(ids.pistol), 12);
    h.clear_events();
    for _ in 0..5 {
        fire_once(&mut h, 7);
    }
    assert_eq!(count(&h, |e| matches!(e, SimEvent::Shot(_))), 5);
    assert_eq!(mag(ids.pistol), 7);
    assert_eq!(h.player().ammo[0], pool0);
    tap(&mut h, b'r');
    h.run_tics(70);
    assert_eq!(mag(ids.pistol), 12);
    assert_eq!(h.player().ammo[0], pool0 - 5);

    // T8.3: a shotgun pickup adds a full magazine; Doom's auto-switch runs with MW2 timing.
    h.clear_events();
    let t0 = h.level_time();
    // SAFETY: live console player during a running level.
    unsafe { P_GiveWeapon(player_mut(), WP_SHOTGUN, 0) };
    h.run_tics(1);
    assert_eq!(mag(ids.shotgun), 8);
    wait_for(&mut h, 60, "shotgun raise", |h| {
        first_tic(h, raised(ids.shotgun)).is_some()
    });
    let drop_tics = first_tic(&h, raised(ids.shotgun)).expect("raise") - t0;
    // Pistol drop_time 300 ms = 10.5 tics.
    assert!((9..=14).contains(&drop_tics), "drop took {drop_tics} tics");
    assert_eq!(h.player().readyweapon, WP_SHOTGUN);
    assert!(psprites_null(&h));
    h.run_tics(45);

    // T8.4: fist = MW2 drop, then the vanilla fist raises; back to the pistol = fist lowers,
    // MW2 raises.
    h.clear_events();
    tap(&mut h, b'1');
    wait_for(&mut h, 40, "fist hand-off", |h| {
        h.player().readyweapon == WP_FIST && !test_api::mw2_owns_weapon()
    });
    assert_eq!(count(&h, |e| *e == SimEvent::DropFinished(ids.shotgun)), 1);
    assert!(!h.player().psprites[0].state.is_null(), "fist psprite up");
    h.run_tics(25);
    h.clear_events();
    tap(&mut h, b'2');
    wait_for(&mut h, 60, "pistol raise from fist", |h| {
        first_tic(h, raised(ids.pistol)).is_some()
    });
    assert_eq!(h.player().readyweapon, WP_PISTOL);
    assert!(psprites_null(&h) && test_api::mw2_owns_weapon());
    h.run_tics(30);

    // T8.5: empty pistol (magazine and pool) -> shotgun (P_CheckAmmo order); empty shotgun -> fist.
    player_mut().ammo[0] = 0;
    for _ in 0..12 {
        fire_once(&mut h, 7);
    }
    assert_eq!(mag(ids.pistol), 0);
    h.clear_events();
    fire_once(&mut h, 2);
    assert!(count(&h, |e| *e == SimEvent::EmptyClick) >= 1);
    wait_for(&mut h, 60, "auto-switch to shotgun", |h| {
        first_tic(h, raised(ids.shotgun)).is_some()
    });
    assert_eq!(h.player().readyweapon, WP_SHOTGUN);
    h.run_tics(40);
    player_mut().ammo[1] = 0;
    for _ in 0..8 {
        fire_once(&mut h, 27);
    }
    assert_eq!(mag(ids.shotgun), 0);
    h.clear_events();
    fire_once(&mut h, 2);
    wait_for(&mut h, 60, "auto-switch to fist", |h| {
        h.player().readyweapon == WP_FIST && !test_api::mw2_owns_weapon()
    });

    // T8.6: IDKFA (fresh pistol start) -> all mapped weapons owned, full magazines, pools at the
    // DOOMMW2 maxima.
    h.new_game(2, 1, 1);
    h.run_tics(2);
    h.give_all();
    let p = h.player();
    let mult = if p.backpack != 0 { 2 } else { 1 };
    assert_eq!(p.maxammo, [360 * mult, 64 * mult, 300 * mult, 12 * mult]);
    assert_eq!(p.ammo, p.maxammo);
    let inv = test_api::inventory().expect("inventory");
    assert_eq!(inv.owned.len(), 5, "{inv:?}");
    for w in pack.weapon_ids() {
        assert_eq!(
            inv.magazine(w),
            pack.facts(w).combat.clip_size,
            "weapon {}",
            w.0
        );
    }
    OPEN_ROOM.go(&mut h);
    h.run_tics(30);

    // T8.9: rifle fully ADS moves at vanilla x move_speed_scale x ads_move_speed_scale.
    // Slot 1 with the chainsaw owned gives the chainsaw: a vanilla weapon either way.
    tap(&mut h, b'1');
    wait_for(&mut h, 40, "vanilla slot 1", |h| {
        matches!(h.player().readyweapon, WP_FIST | WP_CHAINSAW) && !test_api::mw2_owns_weapon()
    });
    h.run_tics(20);
    h.teleport(OPEN_ROOM.x, OPEN_ROOM.y, OPEN_ROOM.angle_deg);
    h.run_tics(1);
    h.hold(b'w', 35);
    let v_vanilla = h.mobj().momy as f64;
    h.clear_events();
    tap(&mut h, b'4');
    wait_for(&mut h, 60, "rifle raise", |h| {
        first_tic(h, raised(ids.rifle)).is_some()
    });
    h.run_tics(45);
    h.mouse(0, 0, 2);
    wait_for(&mut h, 35, "full ADS", |_| {
        test_api::view_state().is_some_and(|v| v.ads_frac >= 1.0)
    });
    h.run_tics(2);
    h.teleport(OPEN_ROOM.x, OPEN_ROOM.y, OPEN_ROOM.angle_deg);
    h.run_tics(1);
    h.hold(b'w', 35);
    let v_ads = h.mobj().momy as f64;
    h.mouse(0, 0, 0);
    h.run_tics(10);
    let m = pack.facts(ids.rifle).movement;
    let want = f64::from(m.move_speed_scale * m.ads_move_speed_scale);
    let ratio = v_ads / v_vanilla;
    assert!(v_vanilla > 0.0, "moved north");
    assert!(
        (ratio / want - 1.0).abs() < 0.03,
        "speed ratio {ratio:.4}, want {want:.4}"
    );
    assert_eq!(h.player().readyweapon, WP_CHAINGUN);

    // T8.7: save/load round trip restores magazines and the held weapon.
    h.run_tics(20);
    h.key(KEY_FIRE, true);
    h.run_tics(6);
    h.key(KEY_FIRE, false);
    h.run_tics(5);
    let rifle_mag = mag(ids.rifle);
    assert!(rifle_mag < 30);
    let bytes = save(&mut h, 7);
    let pos = bytes
        .windows(4)
        .rposition(|w| w == b"DMW1")
        .expect("extension block present");
    assert_eq!(bytes[pos - 1], 0x1d, "block follows SAVEGAME_EOF");
    let mut rest = bytes[pos..].to_vec();
    let block = match parse(move |n| rest.drain(..n.min(rest.len())).collect()) {
        Trailer::Block(b) => b,
        other => panic!("bad block: {other:?}"),
    };
    assert_eq!(block.sim.held.as_deref(), Some("fixture_rifle"));
    tap(&mut h, b'2');
    h.run_tics(40);
    fire_once(&mut h, 7);
    load(&mut h, 7);
    h.run_tics(2);
    assert_eq!(mag(ids.rifle), rifle_mag);
    assert_eq!(h.player().readyweapon, WP_CHAINGUN);
    let v = test_api::view_state().expect("view");
    assert_eq!(v.weapon, Some(ids.rifle));
    assert_eq!(v.weaponstate, READY, "held weapon restored ready");
    assert!(psprites_null(&h));
    // The vanilla loader stops at SAVEGAME_EOF: loading with the weapon glue off succeeds.
    h.set_mw2_mode(Some(false));
    load(&mut h, 7);
    assert_eq!(
        h.player().readyweapon,
        WP_CHAINGUN,
        "vanilla load of a DOOMMW2 save"
    );
    // A vanilla save (no block) loads in MW2 mode with full magazines.
    let vanilla_bytes = save(&mut h, 6);
    assert_eq!(
        *vanilla_bytes.last().expect("bytes"),
        0x1d,
        "no block without MW2 mode"
    );
    h.set_mw2_mode(None);
    h.new_game(2, 1, 1);
    load(&mut h, 6);
    h.run_tics(2);
    for w in test_api::inventory().expect("inv").owned {
        assert_eq!(mag(w), pack.facts(w).combat.clip_size);
    }
    let _ = std::fs::remove_file(save_path(6));
    let _ = std::fs::remove_file(save_path(7));

    // T8.10: death -> respawn: on_respawn, pistol-only inventory, first raise replays.
    OPEN_ROOM.go(&mut h);
    let respawns0 = test_api::respawns();
    // SAFETY: the player's live map object.
    unsafe {
        P_DamageMobj(
            h.player().mo.cast(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            10_000,
        )
    };
    h.run_tics(5);
    assert_eq!(h.player().playerstate, 1, "dead");
    tap(&mut h, b'f');
    wait_for(&mut h, 200, "respawn", |h| {
        h.player().playerstate == 0 && h.level_time() < 5
    });
    h.clear_events();
    h.run_tics(2);
    assert_eq!(test_api::respawns(), respawns0 + 1);
    let inv = test_api::inventory().expect("inv");
    assert_eq!(inv.owned, vec![ids.pistol]);
    assert_eq!(mag(ids.pistol), 12);
    let raise_tics = wait_for(&mut h, 60, "first raise done", |_| {
        test_api::view_state().is_some_and(|v| v.weaponstate == READY)
    });
    // first_raise_time 800 ms = 28 tics (the ordinary raise is 350 ms).
    assert!(raise_tics >= 24, "raise took {raise_tics} tics");

    // T8.8 (demo half): demo playback keeps every hook inert even with a pack loaded.
    let handled = test_api::handled_tics();
    // SAFETY: engine call with a lump name.
    unsafe { G_DeferedPlayDemo(c"demo1".as_ptr()) };
    h.run_tics(200);
    assert!(!mode::level_active());
    assert_eq!(mode::reason().as_deref(), Some("demo playback"));
    assert_eq!(
        test_api::handled_tics(),
        handled,
        "no MW2 tic during the demo"
    );
}
