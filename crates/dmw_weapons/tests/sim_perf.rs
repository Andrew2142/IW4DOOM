// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 07 §10 / 01 §5.8: `WeaponSim::tic` (with placement) within 0.2 ms on the fixture rifle.
//!
//! Release only: `cargo test -p dmw_weapons --release --test sim_perf -- --ignored --nocapture`.
// The 0.2 ms budget is defined for release builds, so this test only exists there.
#![cfg(not(debug_assertions))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

mod common;

use std::time::Instant;

use common::*;
use dmw_weapons::TicButtons;

#[test]
#[ignore = "timing: run in release with --ignored"]
fn tic_budget_fixture_rifle() {
    let mut rig = Rig::new();
    rig.raise(RIFLE);
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    const TICS: u32 = 3500;
    for i in 0..TICS {
        // Auto-fire with ADS toggling every second (13 §5.5 `sim_tic` workload); refill the
        // pool so reloads keep happening.
        let b = TicButtons {
            attack: true,
            ads: (i / 35) % 2 == 1,
            ..TicButtons::default()
        };
        rig.pools[0] = 200;
        let input = rig.input(Some(RIFLE), b);
        let t = Instant::now();
        let out = rig.run(input);
        let dt = t.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(&out);
        total += dt;
        worst = worst.max(dt);
    }
    let mean = total / f64::from(TICS);
    eprintln!("WeaponSim::tic over {TICS} tics: mean {mean:.4} ms, worst {worst:.4} ms");
    assert!(mean <= 0.2, "mean tic {mean:.4} ms exceeds 0.2 ms");
}
