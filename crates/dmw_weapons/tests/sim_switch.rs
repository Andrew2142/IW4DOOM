// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 07 T7.10, T7.11 and the switch, save and life-cycle rules of §5.8 and §5.12.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::*;
use dmw_format::DmwWeaponId;
use dmw_weapons::switch::SwitchState;
use dmw_weapons::{SimEvent, TicButtons, WeaponSim};
use weapon_iw4::WeaponState;

fn sprint_forward(attack: bool) -> TicButtons {
    TicButtons {
        sprint: true,
        attack,
        ..TicButtons::default()
    }
}

#[test]
fn t7_10_sprint_blocks_fire_until_sprint_out() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let mut input = rig.input(Some(RIFLE), sprint_forward(false));
    input.forwardmove = 50;
    input.velocity_mw2 = [190.0, 0.0, 0.0];
    let out = rig.run(input);
    assert!(out.sprinting, "sprinting after one tic");
    assert!((out.move_speed_scale - 0.95 * 1.5).abs() < 1e-4);
    for _ in 0..20 {
        let out = rig.run(input);
        assert!(out.sprinting);
        assert_eq!(shots(&out), 0);
    }
    assert_eq!(rig.sim.hand().weaponstate, WeaponState::SprintLoop as i32);

    // Pull the trigger while still holding sprint: sprint ends, the gun comes out of sprint
    // (sprint_drop_time) and only then fires.
    let pressed = rig.tic;
    let mut fire = input;
    fire.buttons = sprint_forward(true);
    let mut first = None;
    let mut saw_sprint_out = false;
    for _ in 0..60 {
        let t = rig.tic;
        let out = rig.run(fire);
        saw_sprint_out |= rig.sim.hand().weaponstate == WeaponState::SprintOut as i32;
        if shots(&out) > 0 {
            first = Some(t);
            break;
        }
    }
    let first = first.expect("fires after sprint-out");
    assert!(saw_sprint_out);
    let drop = rig.facts(RIFLE).combat.sprint_drop_time_ms;
    let min = tics_for_ms(pressed + 1, drop);
    assert!(
        first - pressed >= min,
        "shot {} tics after the press; sprint-out needs {min}",
        first - pressed
    );
}

/// Switches from `from` (ready) to `to`; returns (tic of RaiseStarted, tic `to` is ready),
/// both relative to the request.
fn switch_timing(rig: &mut Rig, to: DmwWeaponId) -> (u32, u32) {
    let start = rig.tic;
    let mut raise = None;
    for _ in 0..200 {
        let t = rig.tic;
        let out = rig.step(to, TicButtons::default());
        if has(&out, |e| *e == SimEvent::RaiseStarted(to)) {
            raise = Some(t - start);
        }
        if rig.sim.hand().weapon == u32::from(to.0) && rig.sim.hand().weaponstate == 0 {
            return (raise.expect("raise event"), t - start);
        }
    }
    panic!("switch to {} never finished", to.0);
}

#[test]
fn t7_11_mw2_switch_timing() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let rifle = rig.facts(RIFLE).combat;
    let shotgun = rig.facts(SHOTGUN).combat;

    // Rifle -> shotgun: drop_time, then the shotgun's first raise.
    let t0 = rig.tic;
    let (raise, ready) = switch_timing(&mut rig, SHOTGUN);
    let want_raise = tics_for_ms(t0 + 1, rifle.drop_time_ms);
    assert!(
        raise.abs_diff(want_raise) <= 1,
        "raise at {raise}, want {want_raise}"
    );
    let want_ready = tics_for_ms(t0 + 1, rifle.drop_time_ms + shotgun.first_raise_time_ms);
    assert!(
        ready.abs_diff(want_ready) <= 2,
        "ready at {ready}, want {want_ready}"
    );
    assert_eq!(rig.sim.inventory().last_held, Some(RIFLE));

    // Shotgun -> rifle: the rifle was raised before, so its ordinary raise_time.
    let t0 = rig.tic;
    let (_, ready) = switch_timing(&mut rig, RIFLE);
    let want = tics_for_ms(t0 + 1, shotgun.drop_time_ms + rifle.raise_time_ms);
    assert!(ready.abs_diff(want) <= 2, "ready at {ready}, want {want}");

    // Rifle -> pistol: MW2 switches to pistols quickly (WeaponCmd::cmd_weapon_pistol_quick,
    // combat.rs:418): quick_drop_time, then the pistol's first raise.
    let pistol = rig.facts(PISTOL).combat;
    let t0 = rig.tic;
    let (raise, ready) = switch_timing(&mut rig, PISTOL);
    let want_raise = tics_for_ms(t0 + 1, rifle.quick_drop_time_ms);
    assert!(
        raise.abs_diff(want_raise) <= 1,
        "raise at {raise}, want {want_raise}"
    );
    let want = tics_for_ms(
        t0 + 1,
        rifle.quick_drop_time_ms + pistol.first_raise_time_ms,
    );
    assert!(ready.abs_diff(want) <= 2, "ready at {ready}, want {want}");
}

#[test]
fn t7_11_vanilla_handoff_and_back() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let drop = rig.facts(RIFLE).combat.drop_time_ms;
    let start = rig.tic;
    rig.sim.request_vanilla(1);
    let mut lowered = Vec::new();
    let mut dropped = 0;
    for _ in 0..60 {
        let t = rig.tic;
        let input = rig.input(None, TicButtons::default());
        let out = rig.run(input);
        if out.weapon_lowered_for_vanilla {
            lowered.push(t - start);
        }
        dropped += out
            .events
            .iter()
            .filter(|e| **e == SimEvent::DropFinished(RIFLE))
            .count();
        assert_eq!(shots(&out), 0);
    }
    assert_eq!(lowered.len(), 1, "weapon_lowered_for_vanilla exactly once");
    assert_eq!(dropped, 1);
    let want = tics_for_ms(start, drop);
    assert!(
        lowered[0].abs_diff(want) <= 1,
        "lowered after {} tics, want {want}",
        lowered[0]
    );
    assert_eq!(rig.sim.switch_state(), SwitchState::VanillaHeld);
    assert_eq!(rig.sim.view_state().weapon, None);
    assert_eq!(rig.sim.inventory().last_held, Some(RIFLE));

    // Attack while vanilla is up does nothing on the MW2 side.
    let out = rig.run(rig.input(None, attack()));
    assert!(out.events.is_empty());
    assert!(out.held.is_none());
    assert!((out.move_speed_scale - 1.0).abs() < 1e-6);

    // Back to the rifle (already raised this life: ordinary raise_time).
    let raise_time = rig.facts(RIFLE).combat.raise_time_ms;
    let t0 = rig.tic;
    let out = rig.step(RIFLE, TicButtons::default());
    assert!(has(&out, |e| *e == SimEvent::RaiseStarted(RIFLE)));
    let mut ready = None;
    for _ in 0..60 {
        let t = rig.tic;
        rig.step(RIFLE, TicButtons::default());
        if rig.sim.hand().weaponstate == 0 {
            ready = Some(t - t0);
            break;
        }
    }
    let want = tics_for_ms(t0 + 1, raise_time);
    assert!(ready.unwrap().abs_diff(want) <= 1);

    // Vanilla requested with nothing in hand reports the handoff on the next tic.
    let mut rig = Rig::new();
    rig.sim.request_vanilla(0);
    let out = rig.run(rig.input(None, TicButtons::default()));
    assert!(out.weapon_lowered_for_vanilla);
}

#[test]
fn unowned_request_is_ignored() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    rig.sim.inventory_mut().owned.retain(|&w| w != SNIPER);
    for _ in 0..80 {
        rig.step(SNIPER, TicButtons::default());
    }
    assert_eq!(rig.sim.hand().weapon, u32::from(RIFLE.0));
}

#[test]
fn dead_player_does_not_fire() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let mut input = rig.input(Some(RIFLE), attack());
    input.alive = false;
    for _ in 0..20 {
        assert_eq!(shots(&rig.run(input)), 0);
    }
    rig.sim.on_respawn();
    assert!(rig.sim.view_state().weapon.is_none());
    // A new life replays the first raise.
    let first = rig.facts(RIFLE).combat.first_raise_time_ms;
    let t0 = rig.tic;
    let used = rig.raise(RIFLE);
    assert!(used.abs_diff(tics_for_ms(t0, first)) <= 1);
}

#[test]
fn ads_slows_movement_and_scales_switch_off() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let mut out = rig.step(RIFLE, TicButtons::default());
    assert!((out.move_speed_scale - 0.95).abs() < 1e-4);
    for _ in 0..20 {
        out = rig.step(RIFLE, ads());
    }
    assert!((out.ads_frac - 1.0).abs() < 1e-6);
    assert!((out.move_speed_scale - 0.95 * 0.5).abs() < 1e-4);
}

#[test]
fn save_and_restore_by_name() {
    let mut rig = Rig::new();
    rig.raise(SHOTGUN);
    rig.step(SHOTGUN, attack());
    rig.sim.inventory_mut().last_held = Some(RIFLE);
    let save = rig.sim.save();
    assert_eq!(save.held.as_deref(), Some("fixture_shotgun"));
    assert_eq!(save.last_held.as_deref(), Some("fixture_rifle"));
    assert_eq!(save.magazines["fixture_shotgun"], 7);
    assert_eq!(save.owned.len(), 5);

    let mut sim = WeaponSim::new(Arc::clone(&rig.pack));
    let mut bad = save.clone();
    bad.owned.push("not_a_weapon".into());
    sim.restore(bad);
    assert_eq!(sim.inventory().owned.len(), 5);
    assert_eq!(sim.inventory().magazine(SHOTGUN), 7);
    assert_eq!(sim.inventory().last_held, Some(RIFLE));
    let view = sim.view_state();
    assert_eq!(view.weapon, Some(SHOTGUN));
    assert_eq!(view.weaponstate, WeaponState::Ready as i32);
    assert_eq!(view.clip, 7);
    assert_eq!(sim.save(), save);
}
