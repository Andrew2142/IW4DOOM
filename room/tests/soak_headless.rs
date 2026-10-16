// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 08 section 10 "no hook can panic": 10 000 tics of random keys and mouse input through
//! the real input path into `on_player_think` (fixture pack, monsters alive, all weapons),
//! checking after every tic that no vanilla psprite is up while an MW2 weapon is held by a live
//! player, and that MW2 mode was never switched off by an internal error.

mod support;

use room::dmw::{mode, test_api};
use room::doom::doomkeys::{KEY_FIRE, KEY_LEFTARROW, KEY_RIGHTARROW, KEY_RSHIFT};

use support::{find_wad, Harness};

const KEYS: [u8; 18] = [
    KEY_FIRE,
    KEY_FIRE,
    KEY_FIRE,
    b'w',
    b'w',
    b's',
    KEY_LEFTARROW,
    KEY_RIGHTARROW,
    KEY_RSHIFT,
    b'f',
    b'1',
    b'2',
    b'3',
    b'4',
    b'5',
    b'6',
    b'7',
    b'r',
];
const EXTRA: [u8; 2] = [b'v', b'q'];

#[test]
fn soak_headless() {
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
    assert!(mode::weapons_active());
    h.give_all();
    let mut seed = 0x2545_f491_u64;
    let mut rnd = move |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let mut held: Vec<u8> = Vec::new();
    let handled0 = test_api::handled_tics();
    for tic in 0..10_000u32 {
        // Release a held key, press a new one, move the mouse, sometimes toggle ADS.
        if !held.is_empty() && rnd(3) == 0 {
            let k = held.remove(rnd(held.len() as u64) as usize);
            h.key(k, false);
        }
        if rnd(2) == 0 {
            let k = if rnd(10) == 0 {
                EXTRA[rnd(2) as usize]
            } else {
                KEYS[rnd(KEYS.len() as u64) as usize]
            };
            if !held.contains(&k) {
                h.key(k, true);
                held.push(k);
            }
        }
        if rnd(2) == 0 {
            let buttons = if rnd(4) == 0 { 2 } else { 0 } | if rnd(8) == 0 { 8 } else { 0 };
            h.mouse(rnd(41) as i32 - 20, rnd(41) as i32 - 20, buttons as u8);
        }
        h.run_tics(1);
        let p = h.player();
        if p.playerstate == 0 && test_api::mw2_owns_weapon() {
            assert!(
                p.psprites.iter().all(|s| s.state.is_null()),
                "tic {tic}: vanilla psprite up while the MW2 weapon is held"
            );
        }
        assert!(
            (0..9).contains(&p.readyweapon),
            "tic {tic}: readyweapon {}",
            p.readyweapon
        );
        for i in 0..4 {
            assert!(
                (0..=p.maxammo[i]).contains(&p.ammo[i]),
                "tic {tic}: pool {i}"
            );
        }
        if !mode::weapons_active() {
            // A death restarts the level; the mode must come back, never an error reason.
            let reason = mode::reason().unwrap_or_default();
            assert!(!reason.starts_with("error"), "tic {tic}: {reason}");
        }
    }
    assert!(mode::reason().is_none_or(|r| !r.starts_with("error")));
    assert!(
        test_api::handled_tics() > handled0 + 1000,
        "MW2 drove the weapon"
    );
}
