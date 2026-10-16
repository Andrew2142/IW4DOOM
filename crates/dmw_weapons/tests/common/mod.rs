// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared helpers for the sheet 07 integration tests. Everything runs on `Pack::fixture()`.
// `Pack` is single-threaded (00 §7.1) yet `WeaponSim::new` takes `Arc<Pack>` (00 §7.2).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arc_with_non_send_sync
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use dmw_format::consts::tic_msec;
use dmw_format::{DmwWeaponId, Pack, WeaponFacts};
use dmw_weapons::{Inventory, LevelStart, SimEvent, TicButtons, TicInput, TicOutput, WeaponSim};

pub const PISTOL: DmwWeaponId = DmwWeaponId(1);
pub const RIFLE: DmwWeaponId = DmwWeaponId(2);
pub const SHOTGUN: DmwWeaponId = DmwWeaponId(3);
pub const SNIPER: DmwWeaponId = DmwWeaponId(4);
pub const LAUNCHER: DmwWeaponId = DmwWeaponId(5);

/// Doom pools used by the tests: am_clip, am_shell, am_cell, am_misl.
pub const POOLS: [i32; 4] = [200, 50, 300, 10];

/// A sim on the fixture pack that owns every fixture weapon with a full magazine.
pub struct Rig {
    pub pack: Arc<Pack>,
    pub sim: WeaponSim,
    pub tic: u32,
    pub pools: [i32; 4],
    pub yaw: f32,
    pub pitch: f32,
}

pub fn ammo_map() -> BTreeMap<u16, i32> {
    BTreeMap::from([(1, 0), (2, 0), (3, 1), (4, 2), (5, 3)])
}

impl Rig {
    pub fn new() -> Self {
        Self::with_seed(0x1234_5678)
    }

    pub fn with_seed(seed: u32) -> Self {
        let pack = Arc::new(Pack::fixture());
        let mut inventory = Inventory::default();
        for id in pack.weapon_ids() {
            inventory.give(id, pack.facts(id).combat.clip_size);
        }
        let mut sim = WeaponSim::new(Arc::clone(&pack));
        sim.start_level(LevelStart {
            pack: &pack,
            seed,
            inventory,
            ammo_map: ammo_map(),
            sprint_speed_scale: 1.5,
            weapon_move_scales: true,
            viewmodel_fov: 65.0,
        });
        Self {
            pack,
            sim,
            tic: 0,
            pools: POOLS,
            yaw: 90.0,
            pitch: 0.0,
        }
    }

    pub fn facts(&self, id: DmwWeaponId) -> &WeaponFacts {
        self.pack.facts(id)
    }

    /// The input for the next tic with `buttons`, holding `weapon`.
    pub fn input(&self, weapon: Option<DmwWeaponId>, buttons: TicButtons) -> TicInput {
        TicInput {
            tic: self.tic,
            alive: true,
            buttons,
            requested_weapon: weapon,
            yaw_deg: self.yaw,
            pitch_deg: self.pitch,
            prev_yaw_deg: self.yaw,
            prev_pitch_deg: self.pitch,
            origin_mw2: [0.0, 0.0, 0.0],
            velocity_mw2: [0.0, 0.0, 0.0],
            on_ground: true,
            forwardmove: 0,
            sidemove: 0,
            pools: self.pools,
        }
    }

    /// Runs one tic with `input` (its `tic` and `pools` are overwritten).
    pub fn run(&mut self, mut input: TicInput) -> TicOutput {
        input.tic = self.tic;
        input.pools = self.pools;
        let out = self.sim.tic(&input);
        self.pools = out.pools;
        self.tic += 1;
        out
    }

    /// Runs one tic holding `weapon` with `buttons`.
    pub fn step(&mut self, weapon: DmwWeaponId, buttons: TicButtons) -> TicOutput {
        let input = self.input(Some(weapon), buttons);
        self.run(input)
    }

    /// Raises `weapon` from an empty hand and runs until it is `Ready`. Returns the tics used.
    pub fn raise(&mut self, weapon: DmwWeaponId) -> u32 {
        let start = self.tic;
        for _ in 0..200 {
            self.step(weapon, TicButtons::default());
            if self.sim.hand().weapon == u32::from(weapon.0) && self.sim.hand().weaponstate == 0 {
                return self.tic - start;
            }
        }
        panic!("weapon {} never became ready", weapon.0);
    }
}

pub fn attack() -> TicButtons {
    TicButtons {
        attack: true,
        ..TicButtons::default()
    }
}

pub fn ads() -> TicButtons {
    TicButtons {
        ads: true,
        ..TicButtons::default()
    }
}

pub fn reload() -> TicButtons {
    TicButtons {
        reload: true,
        ..TicButtons::default()
    }
}

pub fn shots(out: &TicOutput) -> usize {
    out.events
        .iter()
        .filter(|e| matches!(e, SimEvent::Shot(_)))
        .count()
}

pub fn has(out: &TicOutput, pred: impl Fn(&SimEvent) -> bool) -> bool {
    out.events.iter().any(pred)
}

/// Number of tics from tic `start` (inclusive) until the summed `tic_msec` reaches `ms`.
pub fn tics_for_ms(start: u32, ms: i32) -> u32 {
    let mut acc = 0;
    let mut t = start;
    while acc < ms {
        acc += tic_msec(t);
        t += 1;
    }
    t - start
}

pub fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// Compares `value` with the golden JSON file `name`; `BLESS=1` rewrites it.
pub fn check_golden(name: &str, value: &serde_json::Value) {
    let path = golden_path(name);
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    if std::env::var("BLESS").as_deref() == Ok("1") {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {}; run with BLESS=1", path.display()));
    assert_eq!(text, want, "golden {name} differs (BLESS=1 to accept)");
}
