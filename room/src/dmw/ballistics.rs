// SPDX-License-Identifier: GPL-3.0-or-later
//! Ballistics and damage: `P_LineAttack3D`, damage application, projectiles.
//!
//! Implemented by sheet 09; see `docs/specs/00-overview-and-contracts.md` section 6.4.
//! Sheet 08 calls [`fire`] and [`melee`] from its event dispatch (08 section 5.7); until
//! sheet 09 lands they are stubs that do nothing (shots make sound and light, but hit nothing).

use dmw_weapons::{MeleeRequest, ShotRequest};

use crate::doom::d_player::PlayerT;

/// Resolves one MW2 shot fired by `player` (sheet 09). Stub until sheet 09 lands.
///
/// # Safety
/// `player` must point to a live player whose `mo` is a live map object.
pub(crate) unsafe fn fire(_player: *mut PlayerT, _req: &ShotRequest) {}

/// Resolves one MW2 melee swing by `player` (sheet 09). Stub until sheet 09 lands.
///
/// # Safety
/// As [`fire`].
pub(crate) unsafe fn melee(_player: *mut PlayerT, _req: &MeleeRequest) {}
