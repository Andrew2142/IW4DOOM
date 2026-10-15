// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 mode gating (decision D15): active only with a valid pack, `[general] enabled`,
//! no netgame and no demo recording or playback, fixed for the level.
//!
//! Scaffold stub (sheet 02). Sheet 08 owns the real implementation; until then MW2 mode is
//! never active, so the engine behaves exactly like vanilla `room`.
//!
//! Sheet 05 adds one test-only addition: [`test_force_level_active`], compiled only with the
//! `test-hooks` cargo feature. It lets headless tests exercise MW2-mode input paths before
//! sheet 08's real gating exists. Sheet 08 keeps it (or replaces it with its own gate) when it
//! rewrites this file.

/// `true` while MW2 mode is active for the current level.
pub fn level_active() -> bool {
    #[cfg(feature = "test-hooks")]
    if let Some(forced) = test_override::get() {
        return forced;
    }
    false
}

/// Test-only mode override (`test-hooks` feature). `Some(true)` forces MW2 mode on,
/// `Some(false)` forces it off, `None` returns to the normal gating.
///
/// The override is process-wide and is not part of the shipped binary.
#[cfg(feature = "test-hooks")]
pub fn test_force_level_active(forced: Option<bool>) {
    test_override::set(forced);
}

#[cfg(feature = "test-hooks")]
mod test_override {
    use std::sync::atomic::{AtomicU8, Ordering};

    /// 0 = no override, 1 = forced off, 2 = forced on.
    static FORCED: AtomicU8 = AtomicU8::new(0);

    pub(super) fn get() -> Option<bool> {
        match FORCED.load(Ordering::Relaxed) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }

    pub(super) fn set(forced: Option<bool>) {
        let v = match forced {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        };
        FORCED.store(v, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_mode_is_off() {
        assert!(!super::level_active());
    }
}
