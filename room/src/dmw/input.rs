// SPDX-License-Identifier: GPL-3.0-or-later
//! Mouse accumulation, new key bindings, pitch and extended-button building.
//!
//! Scaffold stub (sheet 02). Implemented by sheet 05. The functions below are the call
//! targets named in `docs/specs/00-overview-and-contracts.md` section 7.5 with their
//! final signatures and "not handled" bodies.

use std::ffi::c_int;

use crate::doom::d_event::event_t;

/// Accumulates mouse pitch from an `ev_mouse` event (`ev.data3`). Called from `G_Responder`
/// only when `hooks::mouse_look_active()` is `true`.
pub(crate) unsafe fn on_mouse_event(_ev: *const event_t) {}

/// Scales horizontal mouse input for the ADS zoom sensitivity; identity when not zoomed.
pub(crate) fn scale_yaw(mousex: c_int) -> c_int {
    mousex
}

#[cfg(test)]
mod tests {
    #[test]
    fn scale_yaw_is_identity_in_the_scaffold() {
        assert_eq!(super::scale_yaw(-37), -37);
    }
}
