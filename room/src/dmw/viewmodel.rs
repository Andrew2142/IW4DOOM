// SPDX-License-Identifier: GPL-3.0-or-later
//! Viewmodel glue: builds `ViewmodelDrawParams`, calls the rasterizer, draws the flash patch.
//!
//! Drawing is sheet 11's engine half (00 section 6.4). Sheet 08 owns the call sites:
//! [`prepare_all`] once per session at the first active level start, [`on_tic`] after every MW2
//! tic (08 section 5.5). Both drive sheet 10's `ViewmodelAnimator`; sheet 11 adds the prepared
//! (quantized) data to [`Registry`] and the drawing.

use std::collections::BTreeMap;

use dmw_weapons::{SoundCue, ViewState, ViewmodelAnimator};
use log::{error, info};

use super::{with_state, DmwState};

const TARGET: &str = "dmw::anim";

/// Per-weapon viewmodel data, keyed by `DmwWeaponId.0`.
#[derive(Debug, Default)]
pub struct Registry {
    /// Sheet 10 animators of every mapped weapon whose viewmodel loaded.
    pub animators: BTreeMap<u16, ViewmodelAnimator>,
}

/// Builds an animator for every mapped weapon. Called once per session with the state
/// borrowed, after sheet 08 has checked (and demoted) the mapped viewmodels.
pub(crate) fn prepare_all(state: &mut DmwState) {
    let Some(pack) = state.pack.clone() else {
        return;
    };
    let started = std::time::Instant::now();
    for id in state.mapping.ids() {
        let (Ok(asset), Some(facts)) = (pack.load_viewmodel(id), pack.try_facts(id)) else {
            continue;
        };
        match ViewmodelAnimator::new(asset, facts) {
            Ok(a) => {
                state.viewmodels.animators.insert(id.0, a);
            }
            Err(e) => error!(target: TARGET, "animator for {} not built: {e}", facts.name),
        }
    }
    info!(
        target: TARGET,
        "prepared {} viewmodel animators in {:.1} ms",
        state.viewmodels.animators.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// Advances the held weapon's animation by one tic and returns its notetrack sounds (sheet 10).
/// Runs outside any `DmwState` borrow (it takes its own).
pub(crate) fn on_tic(view: &ViewState, msec: i32) -> Vec<SoundCue> {
    let Some(id) = view.weapon else {
        return Vec::new();
    };
    with_state(|s| {
        s.viewmodels
            .animators
            .get_mut(&id.0)
            .map(|a| a.advance(view, msec))
            .unwrap_or_default()
    })
    .unwrap_or_default()
}
