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
pub mod freelook;
pub mod hooks;
pub mod hud;
pub mod input;
pub mod mode;
pub mod save;
pub mod viewmodel;
pub mod weapons;

/// Per-process DOOMMW2 state (pack, simulation, prepared viewmodels). Filled in by sheet 08.
#[derive(Debug, Default)]
pub struct DmwState {}

/// Runs `f` with the DOOMMW2 state, or returns `None` when it does not exist.
///
/// Stub: the `thread_local!` `RefCell<Option<DmwState>>` described in sheet 01 section 5.4
/// arrives with sheet 08, so this currently always returns `None`.
pub fn with_state<R>(_f: impl FnOnce(&mut DmwState) -> R) -> Option<R> {
    None
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
    fn with_state_is_none_in_the_scaffold() {
        assert!(with_state(|_| 1).is_none());
    }
}
