// SPDX-License-Identifier: GPL-3.0-or-later
//! HUD glue: crosshair, hitmarker, ammo counter, scope overlay.
//!
//! Scaffold stub (sheet 02). Implemented by sheet 12; see `docs/specs/00-overview-and-contracts.md` section 6.4.
//!
//! Sheet 09 added the minimal [`on_hit`] entry point it calls for every damaging hit; sheet 12
//! replaces the body with the hitmarker and the `mp_hit_alert` sound.

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
