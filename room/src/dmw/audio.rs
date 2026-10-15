// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 sound playback through the rodio mixer.
//!
//! Implemented by sheet 12; see `docs/specs/00-overview-and-contracts.md` section 6.4.
//! Sheet 08's event dispatch calls [`play`]; stub until sheet 12 lands.

use dmw_weapons::SoundCue;

use crate::doom::p_telept::mobj_t;

/// Plays `cue` from `origin` (the player's map object for local weapon sounds). Stub.
pub(crate) fn play(_cue: &SoundCue, _origin: *mut mobj_t) {}
