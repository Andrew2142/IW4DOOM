// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 07 T7.6–T7.9, T7.12: spread, pellets, determinism, ADS and kick.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use dmw_format::consts::{DOOM_HFOV, MW2_CG_FOV};
use dmw_weapons::{ShotRequest, SimEvent, TicButtons, TicOutput, ViewState};

fn angle_between(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
    d.acos().to_degrees()
}

fn shot(out: &TicOutput) -> Option<&ShotRequest> {
    out.events.iter().find_map(|e| match e {
        SimEvent::Shot(s) => Some(s),
        _ => None,
    })
}

#[test]
fn t7_6_spread_bloom_decay_and_ads() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let c = rig.facts(RIFLE).combat;
    let rest = rig.step(RIFLE, TicButtons::default()).spread_deg;
    assert!(
        (rest - c.hip_spread_stand_min).abs() < 0.05,
        "rest hip spread {rest}"
    );

    let mut fired = 0;
    let mut peak: f32 = 0.0;
    while fired < 10 {
        let out = rig.step(RIFLE, attack());
        fired += shots(&out);
        peak = peak.max(out.spread_deg);
    }
    assert!(peak > rest + 1.0, "spread blooms: {rest} -> {peak}");
    assert!(peak <= c.hip_spread_stand_max + 1e-3);

    let mut last = peak;
    let mut decayed = false;
    for _ in 0..35 {
        let s = rig.step(RIFLE, TicButtons::default()).spread_deg;
        assert!(s <= last + 1e-4, "spread never grows without fire");
        last = s;
        decayed |= (s - c.hip_spread_stand_min).abs() < 0.05;
    }
    assert!(decayed, "spread decays back to the hip minimum, at {last}");

    let mut ads_spread = f32::NAN;
    for _ in 0..35 {
        ads_spread = rig.step(RIFLE, ads()).spread_deg;
    }
    assert!(
        (ads_spread - c.ads_spread).abs() < 0.05,
        "ADS spread {ads_spread}"
    );
}

#[test]
fn t7_7_shotgun_pellets_inside_spread() {
    let mut rig = Rig::new();
    rig.raise(SHOTGUN);
    let c = rig.facts(SHOTGUN).combat;
    let out = rig.step(SHOTGUN, attack());
    let s = shot(&out).expect("shotgun fired");
    assert_eq!(s.pellets.len() as i32, c.pellet_count());
    assert_eq!(s.pellets.len(), 8);
    assert!(!s.projectile);
    assert_eq!(s.base_damage, c.damage);
    assert!((s.max_range_mw2 - c.bullet_range()).abs() < 1e-3);
    let aim = math_iw4::angle_vectors([rig.pitch, rig.yaw, 0.0]).0;
    for p in &s.pellets {
        let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-4);
        assert!(angle_between(*p, aim) <= out.spread_deg + 1e-3);
    }
    let distinct: std::collections::BTreeSet<_> =
        s.pellets.iter().map(|p| p.map(f32::to_bits)).collect();
    assert_eq!(distinct.len(), 8, "pellets differ");

    // The launcher fires one projectile.
    rig.raise(LAUNCHER);
    let out = rig.step(LAUNCHER, attack());
    let s = shot(&out).expect("launcher fired");
    assert!(s.projectile);
    assert_eq!(s.pellets.len(), 1);
}

/// A scripted run touching fire, ADS, reload, switch and sprint.
fn scripted(seed: u32) -> (Vec<TicOutput>, Vec<ViewState>) {
    let mut rig = Rig::with_seed(seed);
    let mut outs = Vec::new();
    let mut views = Vec::new();
    for i in 0..300u32 {
        let weapon = if i < 200 { RIFLE } else { SHOTGUN };
        let mut b = TicButtons {
            attack: (40..90).contains(&i) || (250..300).contains(&i) && i % 3 == 0,
            ads: (70..120).contains(&i),
            reload: i == 130,
            sprint: (150..180).contains(&i),
            ..TicButtons::default()
        };
        if (160..170).contains(&i) {
            b.attack = true;
        }
        let mut input = rig.input(Some(weapon), b);
        input.forwardmove = if (150..180).contains(&i) { 50 } else { 0 };
        input.yaw_deg = 90.0 + (i as f32 * 0.7).sin() * 5.0;
        input.velocity_mw2 = [f32::from(input.forwardmove) * 3.0, 0.0, 0.0];
        outs.push(rig.run(input));
        views.push(rig.sim.view_state());
    }
    (outs, views)
}

#[test]
fn t7_8_determinism() {
    let a = scripted(42);
    let b = scripted(42);
    assert_eq!(a, b, "same inputs and seed give identical outputs");
    let total: usize = a.0.iter().map(shots).sum();
    assert!(total > 5);
    let c = scripted(43);
    assert_ne!(a.0, c.0, "a different seed changes the pellets");
}

#[test]
fn t7_9_ads_fraction_and_zoom() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let f = rig.facts(RIFLE).clone();
    let start = rig.tic;
    let mut full = None;
    for _ in 0..60 {
        let t = rig.tic;
        let out = rig.step(RIFLE, ads());
        if out.ads_frac >= 1.0 {
            full = Some(t);
            break;
        }
    }
    let full = full.expect("ADS completes");
    // ads_in_rate is per millisecond in iw4L (movement_iw4/src/ads_frac.rs:79).
    let ms = (1.0 / f.combat.ads_in_rate).ceil() as i32;
    let expected = start + tics_for_ms(start, ms) - 1;
    assert!(
        full.abs_diff(expected) <= 1,
        "ADS full at {full}, expected {expected}"
    );

    let view = rig.sim.view_state();
    let d = 2.0
        * ((DOOM_HFOV * 0.5).to_radians().tan() * (f.fpv.ads_zoom_fov * 0.5).to_radians().tan()
            / (MW2_CG_FOV * 0.5).to_radians().tan())
        .atan()
        .to_degrees();
    let zoom = view.zoom_hfov_deg.expect("zoomed");
    assert!((zoom - d).abs() < 1e-3, "zoom {zoom} vs {d}");
    assert!((view.viewmodel_fov_deg - f.fpv.ads_zoom_fov).abs() < 1e-3);
    assert!(!view.overlay_active);

    let mut cleared = false;
    for _ in 0..60 {
        let out = rig.step(RIFLE, TicButtons::default());
        if out.ads_frac <= 0.0 {
            cleared = true;
            break;
        }
    }
    assert!(cleared);
    let view = rig.sim.view_state();
    assert!(view.zoom_hfov_deg.is_none());
    assert!((view.viewmodel_fov_deg - 65.0).abs() < 1e-3);

    // Scoped sniper: overlay up and the scope FOV (level 0) once fully aimed.
    rig.raise(SNIPER);
    for _ in 0..60 {
        rig.step(SNIPER, ads());
    }
    let view = rig.sim.view_state();
    assert!(view.overlay_active);
    let scope_fov = rig.facts(SNIPER).fpv.scope_zoom.fov(0).unwrap();
    assert!((view.viewmodel_fov_deg - scope_fov).abs() < 1e-3);
}

#[test]
fn t7_12_view_kick_rises_and_decays() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let mut peak: f32 = 0.0;
    let mut fired = 0;
    while fired < 5 {
        let out = rig.step(RIFLE, attack());
        fired += shots(&out);
        for a in out.view_kick_deg {
            assert!(a.abs() <= weapon_iw4::CAMERA_LIMIT_DEGREES as f32 + 1e-4);
            peak = peak.max(a.abs());
        }
    }
    assert!(peak > 0.0, "firing kicks the camera");
    let mut last = [f32::NAN; 3];
    for _ in 0..35 {
        last = rig.step(RIFLE, TicButtons::default()).view_kick_deg;
    }
    assert!(
        last.iter().all(|a| a.abs() < 0.01),
        "kick settles within 1 s: {last:?}"
    );
}
