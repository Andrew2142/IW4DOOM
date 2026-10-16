// SPDX-License-Identifier: GPL-3.0-or-later
//! HUD glue: crosshair, hitmarker, ammo counter, scope overlay.
//!
//! Implemented by sheet 12; see `docs/specs/00-overview-and-contracts.md` section 6.4.
//!
//! Sheet 09 added the minimal [`on_hit`] entry point it calls for every damaging hit; sheet 12
//! replaces the body with the hitmarker and the `mp_hit_alert` sound. Sheet 08 calls [`reset`]
//! at every active level start and [`on_weapon_acquired`] when a pickup adds an MW2 weapon
//! (08 section 5.4); stubs until sheet 12 lands.

use dmw_format::DmwWeaponId;

/// What a damaging hit looked like (sheet 09 section 5.5, sheet 12 section 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HitFeedback {
    /// The pellet hit a head or helmet location.
    pub headshot: bool,
    /// The pellet took the target from alive to dead.
    pub killed: bool,
}

/// A bullet or knife hit a monster. Stub: logs only; no drawing or sound until sheet 12.
pub fn on_hit(feedback: HitFeedback) {
    log::trace!(target: "dmw::hud", "hit feedback {feedback:?}");
}

/// Level start: clear hit markers, messages and timers. Stub.
pub(crate) fn reset() {}

/// A pickup (or cheat) added `id` to the MW2 inventory (12 section 5.5 pickup message). Stub.
pub(crate) fn on_weapon_acquired(_id: DmwWeaponId) {}
