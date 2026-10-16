// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 10 T10.8 timing half and 01 §5.8: `ViewmodelAnimator::advance` + `pose` on the fixture
//! rifle average ≤ 0.1 ms; `WeaponSim::tic` + `advance` average ≤ 0.2 ms.
//!
//! Release only: `cargo test -p dmw_weapons --release --test anim_perf -- --ignored --nocapture`.
#![cfg(not(debug_assertions))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

mod common;

use std::time::Instant;

use common::*;
use dmw_format::consts::tic_msec;
use dmw_weapons::{TicButtons, ViewmodelAnimator};

#[test]
#[ignore = "timing: run in release with --ignored"]
fn advance_and_pose_budget_fixture_rifle() {
    let mut rig = Rig::new();
    let asset = rig.pack.load_viewmodel(RIFLE).unwrap();
    let mut anim = ViewmodelAnimator::new(asset, rig.facts(RIFLE)).unwrap();
    const CALLS: u32 = 10_000;
    let (mut anim_total, mut anim_worst) = (0.0f64, 0.0f64);
    let mut sim_anim_total = 0.0f64;
    for i in 0..CALLS {
        let phase = i % 400;
        let b = TicButtons {
            attack: (60..200).contains(&phase),
            ads: (i / 35) % 2 == 1,
            reload: phase == 250,
            ..TicButtons::default()
        };
        rig.pools[0] = 200;
        let tic = rig.tic;
        let input = rig.input(Some(RIFLE), b);
        let t0 = Instant::now();
        let out = rig.run(input);
        let view = rig.sim.view_state();
        let t1 = Instant::now();
        let cues = anim.advance(&view, tic_msec(tic));
        let t2 = Instant::now();
        let pose = anim.pose(&view);
        let t3 = Instant::now();
        std::hint::black_box((&out, &cues, &pose));
        let a = (t3 - t1).as_secs_f64() * 1000.0;
        anim_total += a;
        anim_worst = anim_worst.max(a);
        sim_anim_total += (t2 - t0).as_secs_f64() * 1000.0;
    }
    let anim_mean = anim_total / f64::from(CALLS);
    let sim_anim_mean = sim_anim_total / f64::from(CALLS);
    eprintln!(
        "advance+pose over {CALLS} calls: mean {anim_mean:.4} ms, worst {anim_worst:.4} ms; \
         WeaponSim::tic+view_state+advance mean {sim_anim_mean:.4} ms"
    );
    assert!(anim_mean <= 0.1, "advance+pose mean {anim_mean:.4} ms");
    assert!(
        sim_anim_mean <= 0.2,
        "tic+advance mean {sim_anim_mean:.4} ms"
    );
}
