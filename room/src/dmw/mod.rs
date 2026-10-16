// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(clippy::dbg_macro, clippy::print_stdout)]
#![warn(clippy::undocumented_unsafe_blocks)]
//! DOOMMW2 engine glue: the only place that knows both Doom and MW2.
//!
//! Every file here is listed in `docs/specs/00-overview-and-contracts.md` section 6.4.
//! Edits to `room/src/doom/*` are single calls into [`hooks`]; all logic lives in this
//! module tree. Until the owning sheets land, bodies are stubs that return the
//! "not handled" value, so the engine behaves exactly like vanilla `room`.

use std::cell::RefCell;
use std::collections::BTreeSet;

thread_local! {
    /// Keys already logged by [`log_once!`] during the current level.
    static LOGGED_ONCE: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Returns `true` the first time `key` is seen since the last [`reset_log_once`].
///
/// Backs the [`log_once!`] macro; not meant to be called directly.
pub fn log_once_first(key: &str) -> bool {
    LOGGED_ONCE.with(|set| set.borrow_mut().insert(key.to_owned()))
}

/// Forgets every [`log_once!`] key. Called at level start (sheet 08).
pub fn reset_log_once() {
    LOGGED_ONCE.with(|set| set.borrow_mut().clear());
}

/// Logs a message at most once per level for a given `key` (sheet 01 section 5.7).
///
/// ```ignore
/// log_once!("dmw::audio", log::Level::Warn, "missing:usp_fire", "missing sound {}", name);
/// ```
#[allow(unused_macros)]
macro_rules! log_once {
    ($target:expr, $level:expr, $key:expr, $($arg:tt)+) => {{
        if $crate::dmw::log_once_first($key) {
            ::log::log!(target: $target, $level, $($arg)+);
        }
    }};
}
#[allow(unused_imports)]
pub(crate) use log_once;

pub mod audio;
pub mod ballistics;
pub mod config;
pub mod consts;
pub mod ext;
pub mod fps;
pub mod freelook;
pub mod hooks;
pub mod hud;
pub mod input;
pub mod interp;
pub mod mode;
pub mod muzzle;
pub mod save;
#[cfg(feature = "test-hooks")]
pub mod test_api;
pub mod useprompt;
pub mod viewmodel;
pub mod weapons;

use std::path::PathBuf;
use std::sync::Arc;

use dmw_format::Pack;
use dmw_viewmodel::PaletteTables;
use dmw_weapons::{TicOutput, WeaponSim};

use config::DmwConfig;

/// Per-process DOOMMW2 state (sheet 08 section 5.1), created by `on_engine_init`.
///
/// Whether MW2 mode is on for the running level is kept by [`mode`] (thread-local cells), not
/// here, so hooks can ask while this state is borrowed.
#[derive(Debug)]
pub struct DmwState {
    /// Parsed `iw4doom.toml`.
    pub config: DmwConfig,
    /// The file the config came from (`None` = defaults).
    pub config_path: Option<PathBuf>,
    /// The loaded pack, if any.
    pub pack: Option<Arc<Pack>>,
    /// PLAYPAL + COLORMAP of the IWAD (sheet 11), built when a pack is loaded.
    pub palette: Option<Arc<PaletteTables>>,
    /// Doom weapon to MW2 weapon mapping (section 5.3).
    pub mapping: weapons::WeaponMapping,
    /// The MW2 simulation (present exactly when a pack is loaded).
    pub sim: Option<WeaponSim>,
    /// Reason of the last `mode::fail`, cleared at the next level start.
    pub failed: Option<String>,
    /// Per-weapon prepared viewmodel data (sheets 10/11).
    pub viewmodels: viewmodel::Registry,
    /// Muzzle light sequence and flash patch timer (section 5.8).
    pub muzzle: weapons::MuzzleState,
    /// Last tic's sim output (movement scaling, HUD).
    pub last_out: Option<TicOutput>,
    /// Per-level driver bookkeeping (hand-off, switch targets, first tic).
    pub driver: weapons::Driver,
    /// `test-hooks` event ring and counters.
    #[cfg(feature = "test-hooks")]
    pub recorder: test_api::Recorder,
}

impl DmwState {
    /// A state with `config` and nothing loaded.
    pub fn new(config: DmwConfig, config_path: Option<PathBuf>) -> Self {
        Self {
            config,
            config_path,
            pack: None,
            palette: None,
            mapping: weapons::WeaponMapping::default(),
            sim: None,
            failed: None,
            viewmodels: viewmodel::Registry::default(),
            muzzle: weapons::MuzzleState::default(),
            last_out: None,
            driver: weapons::Driver::default(),
            #[cfg(feature = "test-hooks")]
            recorder: test_api::Recorder::default(),
        }
    }
}

thread_local! {
    static DMW: std::cell::RefCell<Option<DmwState>> = const { std::cell::RefCell::new(None) };
}

/// Installs the state (`on_engine_init`), replacing any previous one.
pub(crate) fn install_state(state: DmwState) {
    DMW.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => *slot = Some(state),
        Err(_) => log::error!(target: "dmw::sim", "DOOMMW2 state is busy; not installed"),
    });
}

/// Runs `f` with the DOOMMW2 state, or returns `None` before `on_engine_init` and when the
/// state is already borrowed (a re-entrant hook gets "not handled" instead of a panic).
pub fn with_state<R>(f: impl FnOnce(&mut DmwState) -> R) -> Option<R> {
    DMW.with(|cell| {
        let mut slot = cell.try_borrow_mut().ok()?;
        slot.as_mut().map(f)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_once_first_is_true_once_per_key_until_reset() {
        reset_log_once();
        assert!(log_once_first("a"));
        assert!(!log_once_first("a"));
        assert!(log_once_first("b"));
        reset_log_once();
        assert!(log_once_first("a"));
    }

    #[test]
    fn with_state_is_none_before_install_and_never_reenters() {
        assert!(with_state(|_| 1).is_none());
        install_state(DmwState::new(DmwConfig::default(), None));
        assert_eq!(with_state(|_| 1), Some(1));
        let inner = with_state(|_| with_state(|_| 2));
        assert_eq!(inner, Some(None), "a nested borrow is refused, not a panic");
    }
}
