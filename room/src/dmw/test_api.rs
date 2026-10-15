// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only accessors (`test-hooks` cargo feature, sheet 08 section 5.13, sheet 13 section 5.2).
//!
//! A ring buffer of the last [`RING`] `SimEvent`s with the tic they happened on, and read
//! accessors for the sim state. Nothing here is compiled into the shipped binary.

use std::collections::VecDeque;

use dmw_weapons::switch::SwitchState;
use dmw_weapons::{Inventory, SimEvent, TicOutput, ViewState};

use super::weapons::{Owner, WeaponMapping};
use super::with_state;

/// Events kept in the ring.
pub const RING: usize = 256;

/// The event ring and counters, kept in `DmwState`.
#[derive(Debug, Default)]
pub struct Recorder {
    events: VecDeque<(i32, SimEvent)>,
}

impl Recorder {
    /// Records one event produced on level tic `tic`.
    pub(crate) fn record(&mut self, tic: i32, ev: &SimEvent) {
        if self.events.len() == RING {
            self.events.pop_front();
        }
        self.events.push_back((tic, ev.clone()));
    }
}

/// The recorded events, oldest first, with their `leveltime`.
pub fn events() -> Vec<(i32, SimEvent)> {
    with_state(|s| s.recorder.events.iter().cloned().collect()).unwrap_or_default()
}

/// Empties the ring.
pub fn clear_events() {
    with_state(|s| s.recorder.events.clear());
}

/// The sim's presentation snapshot (`None` without a pack).
pub fn view_state() -> Option<ViewState> {
    with_state(|s| s.sim.as_ref().map(dmw_weapons::WeaponSim::view_state)).flatten()
}

/// The weapon mapping (`None` before `on_engine_init`).
pub fn mapping() -> Option<WeaponMapping> {
    with_state(|s| s.mapping.clone())
}

/// The MW2 inventory (`None` without a pack).
pub fn inventory() -> Option<Inventory> {
    with_state(|s| s.sim.as_ref().map(|sim| sim.inventory().clone())).flatten()
}

/// The sim's switch state (`None` without a pack).
pub fn switch_state() -> Option<SwitchState> {
    with_state(|s| s.sim.as_ref().map(dmw_weapons::WeaponSim::switch_state)).flatten()
}

/// The last tic's output.
pub fn last_out() -> Option<TicOutput> {
    with_state(|s| s.last_out.clone()).flatten()
}

/// Tics on which `on_player_think` reported that MW2 owns the weapon (since start-up).
pub fn handled_tics() -> u64 {
    with_state(|s| s.driver.handled_tics).unwrap_or(0)
}

/// `WeaponSim::on_respawn` calls made by the driver (since start-up).
pub fn respawns() -> u32 {
    with_state(|s| s.driver.respawns).unwrap_or(0)
}

/// `true` while the MW2 sim (not Doom's psprites) animates the weapon.
pub fn mw2_owns_weapon() -> bool {
    with_state(|s| s.driver.owner == Owner::Mw2).unwrap_or(false)
}

/// The muzzle flash patch timer (sheet 11 reads it) and the current muzzle light.
pub fn muzzle() -> (u8, u8) {
    with_state(|s| (s.muzzle.light, s.muzzle.flash_tics)).unwrap_or((0, 0))
}

/// `true` when a pack is loaded.
pub fn pack_loaded() -> bool {
    with_state(|s| s.pack.is_some()).unwrap_or(false)
}
