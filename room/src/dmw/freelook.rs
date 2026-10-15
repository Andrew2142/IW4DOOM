// SPDX-License-Identifier: GPL-3.0-or-later
//! Freelook y-shearing, FOV zoom and view-size tables.
//!
//! Scaffold stub (sheet 02). Implemented by sheet 06.

/// Runs `f` with the view centre unsheared, so vanilla psprites (fist, chainsaw) do not move
/// with pitch. The scaffold just runs `f`; sheet 06 saves and restores `centery`.
pub(crate) fn with_unsheared_center<R>(f: impl FnOnce() -> R) -> R {
    f()
}

#[cfg(test)]
mod tests {
    #[test]
    fn with_unsheared_center_returns_the_closure_value() {
        assert_eq!(super::with_unsheared_center(|| 7), 7);
    }
}
