// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 weapon ownership and magazines (sheet 07 §5.9, 00 D13).
//!
//! Doom's four ammo pools are the reserve ("stock"); each MW2 weapon keeps its own magazine
//! here. Reloads move rounds pool → magazine; `weapon_iw4` spends rounds from the magazine.

use std::collections::BTreeMap;

use dmw_format::{DmwWeaponId, DoomAmmo};
use serde::{Deserialize, Serialize};

/// Doom `am_noammo`.
pub const AM_NOAMMO: DoomAmmo = 5;

/// Owned MW2 weapons and their magazines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    /// Owned weapons, in pickup order.
    pub owned: Vec<DmwWeaponId>,
    /// `DmwWeaponId.0` → rounds in magazine.
    pub magazines: BTreeMap<u16, i32>,
    /// The weapon held before the current one (`EXT_BT_SWAP_LAST` target, resolved by 08).
    pub last_held: Option<DmwWeaponId>,
}

impl Inventory {
    /// `true` when `id` is owned.
    pub fn owns(&self, id: DmwWeaponId) -> bool {
        self.owned.contains(&id)
    }

    /// Adds `id` (if new) with `magazine` rounds. An already owned weapon keeps its magazine.
    pub fn give(&mut self, id: DmwWeaponId, magazine: i32) {
        if !self.owns(id) {
            self.owned.push(id);
            self.magazines.insert(id.0, magazine.max(0));
        }
    }

    /// Rounds in the magazine of `id` (0 when unknown).
    pub fn magazine(&self, id: DmwWeaponId) -> i32 {
        self.magazines.get(&id.0).copied().unwrap_or(0)
    }

    /// Sets the magazine of `id`.
    pub fn set_magazine(&mut self, id: DmwWeaponId, rounds: i32) {
        self.magazines.insert(id.0, rounds.max(0));
    }
}

/// Stored by name so a save survives a re-export with a different weapon order (00 §7.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimSave {
    /// Owned weapon names.
    pub owned: Vec<String>,
    /// Weapon name → rounds in magazine.
    pub magazines: BTreeMap<String, i32>,
    /// Name of `Inventory::last_held`.
    pub last_held: Option<String>,
    /// Name of the weapon in hand.
    pub held: Option<String>,
}

/// Index into `TicInput::pools` for a Doom ammo type, `None` for `am_noammo` and unknown values.
pub fn pool_index(ammo: DoomAmmo) -> Option<usize> {
    usize::try_from(ammo).ok().filter(|&i| i < 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn give_keeps_existing_magazine() {
        let mut inv = Inventory::default();
        inv.give(DmwWeaponId(2), 30);
        inv.set_magazine(DmwWeaponId(2), 7);
        inv.give(DmwWeaponId(2), 30);
        assert_eq!(inv.magazine(DmwWeaponId(2)), 7);
        assert_eq!(inv.owned.len(), 1);
        assert_eq!(inv.magazine(DmwWeaponId(9)), 0);
    }

    #[test]
    fn pool_index_rejects_noammo() {
        assert_eq!(pool_index(0), Some(0));
        assert_eq!(pool_index(3), Some(3));
        assert_eq!(pool_index(AM_NOAMMO), None);
        assert_eq!(pool_index(-1), None);
    }
}
