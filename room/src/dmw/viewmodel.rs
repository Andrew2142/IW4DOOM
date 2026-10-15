// SPDX-License-Identifier: GPL-3.0-or-later
//! Viewmodel glue: builds `ViewmodelDrawParams`, calls the rasterizer, draws the flash patch.
//!
//! Implemented by sheets 10 and 11; see `docs/specs/00-overview-and-contracts.md` section 6.4.
//! Sheet 08 owns the call sites: [`prepare_all`] once per session at the first active level
//! start, [`on_tic`] after every MW2 tic (08 section 5.5). Until those sheets land the bodies
//! are stubs.

use dmw_weapons::{SoundCue, ViewState};

use super::DmwState;

/// Per-weapon prepared viewmodel data (animators, quantized textures). Filled by sheets 10/11.
#[derive(Debug, Default)]
pub struct Registry {}

/// Prepares every mapped weapon's viewmodel (sheets 10/11). Called once per session, with the
/// state borrowed, after sheet 08 has checked that each mapped viewmodel loads. Stub.
pub(crate) fn prepare_all(_state: &mut DmwState) {}

/// Advances the viewmodel animation by one tic and returns its notetrack sounds (sheet 10).
/// Stub: no sounds.
pub(crate) fn on_tic(_view: &ViewState, _msec: i32) -> Vec<SoundCue> {
    Vec::new()
}
