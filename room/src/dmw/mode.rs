// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 mode gating (decision D15, sheet 08 section 5.10): active only with a valid pack,
//! `[general] enabled`, no netgame and no demo recording or playback. Evaluated at level start
//! (`on_level_start`) and fixed for the level; [`fail`] switches it off for the rest of the
//! level.
//!
//! The result is tied to the level it was evaluated for through `levelstarttic` (set by
//! `G_DoLoadLevel` before `P_SetupLevel`). While the next level loads, before its
//! `on_level_start`, the previous result therefore no longer counts and every hook is inert.
//! This keeps demo starts exact: `G_DoPlayDemo` sets `demoplayback` only after `G_InitNew`
//! returns, so the gate also treats `precache == 0` (which only `G_DoPlayDemo` sets around
//! `G_InitNew`) as demo playback.
//!
//! The state lives in thread-local cells, not in `DmwState`, so every hook can ask
//! [`level_active`] while another part of the glue holds the `DmwState` borrow.
//!
//! [`test_force_level_active`] (sheet 05, `test-hooks` feature) overrides the answer of
//! [`level_active`] for tests that exercise input and freelook without a pack. The weapon
//! glue additionally needs a real evaluation ([`weapons_active`]).

use std::cell::{Cell, RefCell};

use log::{error, info};

const TARGET: &str = "dmw::sim";

thread_local! {
    /// Result of the last evaluation.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// `levelstarttic` of the level [`ACTIVE`] belongs to.
    static TOKEN: Cell<Option<i32>> = const { Cell::new(None) };
    /// Why MW2 mode is off (`None` while on).
    static REASON: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Inputs of the D15 decision, read from the engine at level start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GateInputs {
    /// A pack is loaded and the sim exists.
    pub pack_loaded: bool,
    /// `[general] enabled`.
    pub enabled: bool,
    /// `netgame != 0`.
    pub netgame: bool,
    /// `demorecording != 0`.
    pub demorecording: bool,
    /// `demoplayback != 0`, or `precache == 0` (`G_DoPlayDemo` is inside `G_InitNew`).
    pub demoplayback: bool,
    /// `mode::fail` was called for this session's pack and not cleared (always `false` today:
    /// the failure is cleared at every level start, sheet 08 section 5.10).
    pub failed: bool,
}

/// The D15 rule. `Ok(())` = MW2 mode on, `Err(reason)` = off.
pub fn evaluate(g: &GateInputs) -> Result<(), &'static str> {
    if !g.pack_loaded {
        return Err("no pack loaded");
    }
    if !g.enabled {
        return Err("[general] enabled = false");
    }
    if g.netgame {
        return Err("netgame");
    }
    if g.demorecording {
        return Err("demo recording");
    }
    if g.demoplayback {
        return Err("demo playback");
    }
    if g.failed {
        return Err("disabled after an error");
    }
    Ok(())
}

/// Current `levelstarttic` of the engine.
fn level_token() -> i32 {
    // SAFETY: plain int read on the single engine thread.
    unsafe { crate::doom::g_game::levelstarttic }
}

/// Stores the decision for the level that is starting (called by `on_level_start`). Logs the
/// result at `info` when it changes or the reason changes.
pub(crate) fn set_level(result: Result<(), &str>) {
    set_level_at(result, level_token());
}

fn set_level_at(result: Result<(), &str>, token: i32) {
    let active = result.is_ok();
    let reason = result.err().map(str::to_owned);
    let changed = ACTIVE.get() != active || REASON.with_borrow(|r| *r != reason);
    ACTIVE.set(active);
    TOKEN.set(Some(token));
    if changed {
        match &reason {
            None => info!(target: TARGET, "MW2 mode on"),
            Some(r) => info!(target: TARGET, "MW2 mode off: {r}"),
        }
    }
    REASON.set(reason);
}

/// `true` when the last evaluation said "on" and it belongs to the running level.
fn evaluated_active() -> bool {
    evaluated_active_at(level_token())
}

fn evaluated_active_at(token: i32) -> bool {
    ACTIVE.get() && TOKEN.get() == Some(token)
}

/// `true` while MW2 mode is active for the current level (input, freelook, weapons).
///
/// With the `test-hooks` override set, the override wins.
pub fn level_active() -> bool {
    #[cfg(feature = "test-hooks")]
    if let Some(forced) = test_override::get() {
        return forced;
    }
    evaluated_active()
}

/// `true` when the weapon glue (sheet 08) runs this level: a real D15 evaluation said "on"
/// and the test override (if any) does not force MW2 mode off.
pub fn weapons_active() -> bool {
    #[cfg(feature = "test-hooks")]
    if test_override::get() == Some(false) {
        return false;
    }
    evaluated_active()
}

/// Why MW2 mode is off, or `None` while it is on.
pub fn reason() -> Option<String> {
    REASON.with_borrow(Clone::clone)
}

/// Switches MW2 mode off for the rest of the level after an internal error (sheet 01
/// section 5.5): logs `error!` once, returns the weapon to Doom's psprites and resets
/// freelook. Safe to call from any hook, also while `DmwState` is borrowed (it only touches
/// Doom globals and the cells of this module).
pub fn fail(reason: &str) {
    if !ACTIVE.get() {
        return;
    }
    error!(target: TARGET, "MW2 mode disabled for this level: {reason}");
    ACTIVE.set(false);
    REASON.set(Some(format!("error: {reason}")));
    crate::dmw::freelook::reset();
    // SAFETY: single engine thread; `consoleplayer` indexes `players`; the player and its map
    // object are live while a level runs, and `P_SetupPsprites` only needs a valid player.
    unsafe {
        use crate::doom::g_game::{consoleplayer, gamestate, players};
        let cp = consoleplayer;
        if gamestate != 0 || cp < 0 || cp as usize >= crate::doom::d_player::MAXPLAYERS {
            return;
        }
        let player = (&raw mut players)
            .cast::<crate::doom::d_player::PlayerT>()
            .add(cp as usize);
        (*player).extralight = 0;
        if !(*player).mo.is_null() && (*player).playerstate == 0 {
            crate::doom::p_pspr::P_SetupPsprites(player);
        }
    }
}

/// Test-only mode override (`test-hooks` feature). `Some(true)` forces MW2 mode on,
/// `Some(false)` forces it off, `None` returns to the normal gating.
///
/// The override is process-wide and is not part of the shipped binary. It changes what
/// [`level_active`] reports (input, freelook). The weapon glue also needs a loaded pack, so
/// `Some(true)` without `-fixturepack` leaves weapons vanilla; `Some(false)` turns them off.
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
    use super::*;

    fn on() -> GateInputs {
        GateInputs {
            pack_loaded: true,
            enabled: true,
            ..GateInputs::default()
        }
    }

    #[test]
    fn mode_is_off_before_any_level() {
        assert!(!super::weapons_active());
    }

    #[test]
    fn gate_follows_d15() {
        assert_eq!(evaluate(&on()), Ok(()));
        let cases = [
            (
                GateInputs {
                    pack_loaded: false,
                    ..on()
                },
                "no pack loaded",
            ),
            (
                GateInputs {
                    enabled: false,
                    ..on()
                },
                "[general] enabled = false",
            ),
            (
                GateInputs {
                    netgame: true,
                    ..on()
                },
                "netgame",
            ),
            (
                GateInputs {
                    demorecording: true,
                    ..on()
                },
                "demo recording",
            ),
            (
                GateInputs {
                    demoplayback: true,
                    ..on()
                },
                "demo playback",
            ),
            (
                GateInputs {
                    failed: true,
                    ..on()
                },
                "disabled after an error",
            ),
        ];
        for (g, want) in cases {
            assert_eq!(evaluate(&g), Err(want));
        }
    }

    #[test]
    fn decision_belongs_to_the_level_it_was_made_for() {
        set_level_at(Ok(()), 100);
        assert!(evaluated_active_at(100));
        assert!(
            !evaluated_active_at(101),
            "a new level load invalidates the decision"
        );
        set_level_at(Err("no pack loaded"), 101);
        assert!(!evaluated_active_at(101));
        assert_eq!(reason().as_deref(), Some("no pack loaded"));
    }
}
