// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 mode gating (decision D15): active only with a valid pack, `[general] enabled`,
//! no netgame and no demo recording or playback, fixed for the level.
//!
//! Scaffold stub (sheet 02). Sheet 08 owns the real implementation; until then MW2 mode is
//! never active, so the engine behaves exactly like vanilla `room`.

/// `true` while MW2 mode is active for the current level.
pub fn level_active() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_mode_is_off() {
        assert!(!super::level_active());
    }
}
