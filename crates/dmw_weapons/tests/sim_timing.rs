// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 07 T7.1–T7.5, T7.13, T7.14: fire rate, semi-auto, reloads, bolt action, time,
//! and parity against calling `weapon_iw4::weapon_hands` directly.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use dmw_format::consts::{TICRATE, tic_msec, tic_server_time};
use dmw_weapons::{SimEvent, TicButtons};
use weapon_iw4::{WeaponCmd, WeaponHandState, WeaponState, WeaponTickEvent, weapon_hands};

/// The bridge's overshoot carry (sheet 07 §11 item 16): a shot fired on the tic in which the
/// previous cycle ran out starts its cycle shortened by the overrun.
fn carried_weapon_time(fire_time: i32, firing_before: bool, pre: i32, msec: i32) -> i32 {
    if firing_before && pre > 0 && pre <= msec {
        (fire_time.max(1) - (msec - pre)).max(1)
    } else {
        fire_time.max(1)
    }
}

/// Tics (relative to the first held tic) at which a weapon with `fire_time` fires when the
/// trigger is held from `start`: `weapon_time` counts down by `tic_msec`, the gun fires on the
/// tic it reaches 0, and the bridge carries the overrun into the next cycle.
fn predicted_auto_shots(start: u32, tics: u32, fire_time: i32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut weapon_time = 0;
    let mut firing = false;
    for i in 0..tics {
        let msec = tic_msec(start + i);
        let pre = weapon_time;
        weapon_time = (weapon_time - msec).max(0);
        if weapon_time == 0 {
            out.push(i);
            weapon_time = carried_weapon_time(fire_time, firing, pre, msec);
            firing = true;
        }
    }
    out
}

#[test]
fn t7_1_fire_rate_full_auto_rifle() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let start = rig.tic;
    let mut shot_tics = Vec::new();
    for i in 0..35 {
        let out = rig.step(RIFLE, attack());
        if shots(&out) > 0 {
            shot_tics.push(i);
        }
    }
    let fire_time = rig.facts(RIFLE).combat.fire_time_ms;
    assert_eq!(fire_time, 86);
    assert_eq!(shot_tics, predicted_auto_shots(start, 35, fire_time));
    // With the overrun carried, shots fall in the tics containing 0, 86, ..., 946 ms: 12 shots
    // in the 1000 ms from the first (1000 / 86 = 11.6 per second).
    assert_eq!(shot_tics.len(), 12, "shots in 35 tics");
    check_golden(
        "sim_fire_rifle.json",
        &serde_json::json!({ "weapon": "fixture_rifle", "first_tic": start, "shot_tics": shot_tics }),
    );
}

/// Sheet 07 §11 item 16: over 10 s of held fire the average rate is 1000 / fire_time within 1%.
#[test]
fn t7_1b_average_fire_rate_matches_fire_time() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let fire_time = rig.facts(RIFLE).combat.fire_time_ms;
    let mut shot_times = Vec::new();
    for _ in 0..(10 * TICRATE) {
        // Keep the magazine and pool full so no reload interrupts the burst.
        rig.sim.inventory_mut().set_magazine(RIFLE, 30);
        rig.pools[0] = 200;
        let t = rig.tic;
        if shots(&rig.step(RIFLE, attack())) > 0 {
            shot_times.push(tic_server_time(t) + tic_msec(t));
        }
    }
    let first = *shot_times.first().unwrap();
    let last = *shot_times.last().unwrap();
    let rate = (shot_times.len() - 1) as f64 * 1000.0 / f64::from(last - first);
    let want = 1000.0 / f64::from(fire_time);
    assert!(
        ((rate - want) / want).abs() < 0.01,
        "{} shots, {rate:.3}/s vs {want:.3}/s",
        shot_times.len()
    );
    let per_10s = shot_times.len() as f64;
    assert!(
        ((per_10s / 10.0 - want) / want).abs() < 0.01,
        "{per_10s} shots in 10 s vs {want:.3}/s"
    );
}

#[test]
fn t7_2_semi_auto_pistol() {
    let mut rig = Rig::new();
    rig.raise(PISTOL);
    let mut total = 0;
    for _ in 0..40 {
        total += shots(&rig.step(PISTOL, attack()));
    }
    assert_eq!(total, 1, "holding the trigger fires once");

    // Fire, release for one tic, then hold again: the next shot comes when fire_time has run.
    rig.step(PISTOL, TicButtons::default());
    let first = rig.tic;
    assert_eq!(shots(&rig.step(PISTOL, attack())), 1);
    rig.step(PISTOL, TicButtons::default());
    let mut second = None;
    for _ in 0..20 {
        let t = rig.tic;
        if shots(&rig.step(PISTOL, attack())) > 0 {
            second = Some(t);
            break;
        }
    }
    let second = second.expect("second shot");
    let fire_time = rig.facts(PISTOL).combat.fire_time_ms;
    let expected = first + tics_for_ms(first + 1, fire_time);
    assert_eq!(
        second, expected,
        "re-press fires once fire_time ({fire_time} ms) has run"
    );
}

#[test]
fn t7_3_reload_timing_empty_magazine() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    rig.sim.inventory_mut().set_magazine(RIFLE, 0);
    let pool_before = rig.pools[0];
    let start = rig.tic;
    let out = rig.step(RIFLE, reload());
    assert!(has(&out, |e| matches!(
        e,
        SimEvent::ReloadStarted { empty: true }
    )));
    let mut filled = None;
    for _ in 0..200 {
        let t = rig.tic;
        rig.step(RIFLE, TicButtons::default());
        if rig.sim.inventory().magazine(RIFLE) > 0 {
            filled = Some(t);
            break;
        }
    }
    let filled = filled.expect("magazine refilled");
    let empty_time = rig.facts(RIFLE).combat.reload_empty_time_ms;
    let expected = start + tics_for_ms(start + 1, empty_time);
    assert!(
        filled.abs_diff(expected) <= 1,
        "refilled at tic {filled}, expected {expected} (reload_empty_time {empty_time} ms)"
    );
    assert_eq!(rig.sim.inventory().magazine(RIFLE), 30);
    assert_eq!(
        rig.pools[0],
        pool_before - 30,
        "pool pays for the rounds added"
    );
    assert_eq!(rig.sim.view_state().clip, 30);
}

#[test]
fn t7_3_auto_reload_when_firing_empty() {
    let mut rig = Rig::new();
    rig.raise(PISTOL);
    rig.sim.inventory_mut().set_magazine(PISTOL, 0);
    let out = rig.step(PISTOL, attack());
    assert!(has(&out, |e| matches!(
        e,
        SimEvent::ReloadStarted { empty: true }
    )));
}

#[test]
fn t7_3_empty_click_without_stock() {
    let mut rig = Rig::new();
    rig.raise(PISTOL);
    rig.sim.inventory_mut().set_magazine(PISTOL, 0);
    rig.pools[0] = 0;
    let out = rig.step(PISTOL, attack());
    assert!(has(&out, |e| matches!(e, SimEvent::EmptyClick)));
    assert!(has(
        &out,
        |e| matches!(e, SimEvent::Sound(s) if s.alias == "fixture_empty")
    ));
}

#[test]
fn t7_4_segmented_reload_and_interrupt() {
    let mut rig = Rig::new();
    rig.raise(SHOTGUN);
    let c = rig.facts(SHOTGUN).combat;
    rig.sim.inventory_mut().set_magazine(SHOTGUN, 0);
    let start = rig.tic;
    let out = rig.step(SHOTGUN, reload());
    assert!(has(&out, |e| matches!(
        e,
        SimEvent::ReloadStarted { empty: true }
    )));
    let mut inserts = Vec::new();
    let mut finished = None;
    let mut last = 0;
    for _ in 0..400 {
        let t = rig.tic;
        let out = rig.step(SHOTGUN, TicButtons::default());
        let mag = rig.sim.inventory().magazine(SHOTGUN);
        if mag > last {
            assert_eq!(mag, last + c.reload_ammo_add);
            inserts.push(t);
            last = mag;
        }
        if has(&out, |e| matches!(e, SimEvent::ReloadFinished)) {
            finished = Some(t);
            break;
        }
    }
    assert_eq!(inserts.len() as i32, c.clip_size, "one insert per shell");
    let first_expected =
        start + tics_for_ms(start + 1, c.reload_start_time_ms + c.reload_add_time_ms);
    assert!(
        inserts[0].abs_diff(first_expected) <= 1,
        "first insert {} vs {first_expected}",
        inserts[0]
    );
    for pair in inserts.windows(2) {
        let want = tics_for_ms(pair[0], c.reload_add_time_ms);
        assert!(pair[1] - pair[0] <= want + 1 && pair[1] - pair[0] + 1 >= want);
    }
    assert!(finished.unwrap() >= *inserts.last().unwrap());

    // Interrupt: firing during the inserts ends the reload with the shells loaded so far.
    let mut rig = Rig::new();
    rig.raise(SHOTGUN);
    rig.sim.inventory_mut().set_magazine(SHOTGUN, 0);
    rig.step(SHOTGUN, reload());
    while rig.sim.inventory().magazine(SHOTGUN) < 2 {
        rig.step(SHOTGUN, TicButtons::default());
    }
    let mut finished = false;
    let mut fired = false;
    for _ in 0..200 {
        let out = rig.step(SHOTGUN, attack());
        finished |= has(&out, |e| matches!(e, SimEvent::ReloadFinished));
        if shots(&out) > 0 {
            fired = true;
            break;
        }
    }
    assert!(
        finished && fired,
        "attack interrupts the reload, then fires"
    );
    assert!(rig.sim.inventory().magazine(SHOTGUN) < c.clip_size);
}

#[test]
fn t7_5_bolt_action_rechamber() {
    let mut rig = Rig::new();
    rig.raise(SNIPER);
    let c = rig.facts(SNIPER).combat;
    let first = rig.tic;
    assert_eq!(shots(&rig.step(SNIPER, attack())), 1);
    let mut saw_rechamber = false;
    let mut second = None;
    for i in 0..200 {
        let t = rig.tic;
        // Tap the trigger every other tic (semi-auto needs a fresh press).
        let b = if i % 2 == 0 {
            TicButtons::default()
        } else {
            attack()
        };
        let out = rig.step(SNIPER, b);
        saw_rechamber |= rig.sim.hand().weaponstate == WeaponState::Rechambering as i32;
        if shots(&out) > 0 {
            second = Some(t);
            break;
        }
    }
    assert!(saw_rechamber);
    let second = second.expect("second shot");
    let min = tics_for_ms(first + 1, c.fire_time_ms + c.rechamber_time_ms);
    assert!(
        second - first >= min,
        "second shot after {} tics, rechamber needs {min}",
        second - first
    );
}

#[test]
fn t7_13_msec_and_server_time() {
    let total: i32 = (0..TICRATE).map(tic_msec).sum();
    assert_eq!(total, 1000);
    let mut rig = Rig::new();
    let mut last = i32::MIN;
    for _ in 0..200 {
        rig.step(RIFLE, TicButtons::default());
        let t = rig.sim.player_state().command_time;
        assert!(t > last, "server_time must increase");
        assert_eq!(t, tic_server_time(rig.tic - 1) + tic_msec(rig.tic - 1));
        last = t;
    }
}

/// Buttons of the T7.14 script at tic `i`: raise, burst, pause, reload, idle, long burst.
fn parity_script(i: u32) -> (bool, bool) {
    match i {
        40..=60 => (true, false),
        61..=65 => (false, false),
        66 => (false, true),
        150..=200 => (true, false),
        _ => (false, false),
    }
}

#[test]
fn t7_14_weaponstate_parity_with_weapon_iw4() {
    const TICS: u32 = 3 * 35 + 120;
    let mut rig = Rig::new();
    let facts = rig.facts(RIFLE).combat;
    let mut via_sim = Vec::new();
    for i in 0..TICS {
        let (fire, rl) = parity_script(i);
        let b = TicButtons {
            attack: fire,
            reload: rl,
            ..TicButtons::default()
        };
        rig.step(RIFLE, b);
        let h = rig.sim.hand();
        via_sim.push((h.weaponstate, h.weapon_time, h.clip));
    }

    // The same command stream, built by hand and fed straight to weapon_iw4.
    let mut hands = [WeaponHandState::default(); 2];
    hands[0].clip = facts.clip_size;
    hands[0].stock = POOLS[0];
    let mut old_buttons = 0;
    let mut used = false;
    let mut direct = Vec::new();
    let fire_time = facts.fire_time_ms;
    for i in 0..TICS {
        let (fire, rl) = parity_script(i);
        let buttons = (u32::from(fire) * playerstate_iw4::buttons::ATTACK)
            | (u32::from(rl) * playerstate_iw4::buttons::RELOAD);
        let msec = tic_msec(i);
        let mut cmd = WeaponCmd {
            msec,
            server_time: tic_server_time(i) + msec,
            buttons,
            old_buttons,
            cmd_weapon: RIFLE.0,
            cmd_weapon_owned: true,
            switch_raise_time_ms: facts.raise_time_ms,
            switch_quick_raise_time_ms: facts.quick_raise_time_ms,
            switch_first_raise_time_ms: if used { 0 } else { facts.first_raise_time_ms },
            ..WeaponCmd::default()
        };
        let (pre_state, pre_time) = (hands[0].weaponstate, hands[0].weapon_time);
        let events = weapon_hands(&mut hands, &facts, &mut cmd, 0);
        // The bridge's overshoot carry, applied to the direct stream too.
        if events
            .iter()
            .flatten()
            .any(|(_, e)| matches!(e, WeaponTickEvent::ShotAccepted { .. }))
            && hands[0].weapon_time == fire_time.max(1)
        {
            hands[0].weapon_time = carried_weapon_time(
                fire_time,
                pre_state == WeaponState::Firing as i32,
                pre_time,
                msec,
            );
        }
        if events
            .iter()
            .flatten()
            .any(|(_, e)| *e == WeaponTickEvent::RaiseStarted)
        {
            used = true;
        }
        old_buttons = buttons;
        direct.push((hands[0].weaponstate, hands[0].weapon_time, hands[0].clip));
    }
    assert_eq!(via_sim, direct);
    // The script exercises raise, fire, reload and fire again.
    let states: std::collections::BTreeSet<i32> = direct.iter().map(|s| s.0).collect();
    for s in [
        WeaponState::Raising,
        WeaponState::Firing,
        WeaponState::Reloading,
        WeaponState::Ready,
    ] {
        assert!(states.contains(&(s as i32)), "script never reached {s:?}");
    }
}
