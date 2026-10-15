// SPDX-License-Identifier: GPL-3.0-or-later
//! HUD glue: crosshair, hitmarker, ammo counter, scope overlay.
//!
//! Implemented by sheet 12; see `docs/specs/00-overview-and-contracts.md` section 6.4.
//! Sheet 08 calls [`reset`] at every active level start and [`on_weapon_acquired`] when a
//! pickup adds an MW2 weapon (08 section 5.4); stubs until sheet 12 lands.

use dmw_format::DmwWeaponId;

/// Level start: clear hit markers, messages and timers. Stub.
pub(crate) fn reset() {}

/// A pickup (or cheat) added `id` to the MW2 inventory (12 section 5.5 pickup message). Stub.
pub(crate) fn on_weapon_acquired(_id: DmwWeaponId) {}
