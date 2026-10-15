//! Headless input tests for sheet 05 (T5.4, T5.5, T5.6).
//!
//! Drives the real engine on Freedoom E1M1 with injected `DG_GetKey` / `DG_GetMouse`
//! samples through the shared harness of sheet 13 (`support::Harness`). One engine per process
//! (Doom globals), so there is exactly one `#[test]` that runs its scenarios in sequence.
//!
//! MW2 mode is switched with `Harness::set_mw2_mode`, which wraps
//! `room::dmw::mode::test_force_level_active` (the `test-hooks` feature, enabled for tests
//! through the self dev-dependency in `room/Cargo.toml`) because sheet 08's real mode gating
//! and sheet 03's `-fixturepack` do not exist on this branch. `Some(false)` stands in for
//! `-nomw2`, `Some(true)` for "pack loaded and enabled".

mod support;

use room::dmw::ext::{
    EXT_BT_ADS, EXT_BT_ATTACK, EXT_BT_HOLD_BREATH, EXT_BT_RELOAD, EXT_BT_SPRINT, EXT_BT_VALID,
};
use room::doom::doomkeys::{KEY_RSHIFT, KEY_UPARROW};

use support::Harness;

/// Player position in fixed-point map units.
fn pos(h: &Harness) -> (i32, i32) {
    (h.mobj().x, h.mobj().y)
}

fn angle(h: &Harness) -> u32 {
    h.mobj().angle
}

fn pitch_q(h: &Harness) -> i32 {
    h.player_ext().pitch_q
}

fn ext_buttons(h: &Harness) -> u16 {
    h.player_ext().cmd.buttons
}

/// `(forwardmove, sidemove, angleturn)` of the console player's current ticcmd.
fn cmd(h: &Harness) -> (i8, i8, i16) {
    let c = &h.player().cmd;
    (c.forwardmove, c.sidemove, c.angleturn)
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// T5.4: in MW2 mode mouse Y changes pitch by the formula and does not move the player.
fn mw2_mouse_look(h: &mut Harness) {
    h.set_mw2_mode(Some(true));
    h.reset_ext();
    h.run_tics(2);
    let before = pos(h);
    let angle_before = angle(h);

    // +50 counts (mouse moved toward the user = look down) for 10 tics.
    for _ in 0..10 {
        h.mouse(0, 50, 0);
        h.run_tics(1);
    }
    h.run_tics(3); // let the last ticcmd reach G_Ticker

    // 500 counts * 8 * 360 / 65536 degrees * 128 units/degree = 2812.5 units, the half unit
    // stays in the accumulator.
    assert_eq!(pitch_q(h), 2812, "pitch_q after 500 counts");
    assert_eq!(
        pos(h),
        before,
        "mouse Y must not move the player in MW2 mode"
    );
    assert_eq!(angle(h), angle_before, "no horizontal motion was injected");
    assert_eq!(cmd(h).0, 0, "no forward move in the ticcmd");

    // Mouse forward looks up (negative pitch) and the clamp holds at 32 degrees.
    for _ in 0..40 {
        h.mouse(0, 100, 0);
        h.run_tics(1);
    }
    h.run_tics(3);
    assert_eq!(pitch_q(h), 32 * 128, "pitch clamps at +32 degrees");
    for _ in 0..80 {
        h.mouse(0, -100, 0);
        h.run_tics(1);
    }
    h.run_tics(3);
    assert_eq!(pitch_q(h), -32 * 128, "pitch clamps at -32 degrees");

    // Mouse X still turns the player (yaw path is shared with vanilla).
    h.mouse(40, 0, 0);
    h.run_tics(3);
    assert_ne!(angle(h), angle_before, "mouse X turns the player");
}

/// T5.5: with MW2 mode off the same input leaves pitch at 0 and mouse Y moves the player.
fn vanilla_mouse(h: &mut Harness) {
    h.set_mw2_mode(Some(false));
    h.reset_ext();
    h.run_tics(3);
    let before = pos(h);

    for _ in 0..10 {
        h.mouse(0, -20, 0); // mouse moved forward
        h.run_tics(1);
    }
    h.run_tics(3);

    assert_eq!(pitch_q(h), 0, "no pitch outside MW2 mode");
    assert_ne!(pos(h), before, "mouse Y moves the player in vanilla mode");
    // Let the momentum die before the next scenario.
    h.run_tics(60);
}

/// T5.6: extended buttons, always-run and the masked strafe/forward mouse buttons.
fn ext_buttons_scenario(h: &mut Harness) {
    h.set_mw2_mode(Some(true));
    h.reset_ext();
    h.run_tics(3);

    // Reload key.
    h.key(b'r', true);
    h.run_tics(3);
    let b = ext_buttons(h);
    assert_ne!(b & EXT_BT_VALID, 0, "slot written by on_build_ticcmd");
    assert_ne!(b & EXT_BT_RELOAD, 0, "r -> reload");
    assert_eq!(b & EXT_BT_ADS, 0);
    h.key(b'r', false);
    h.run_tics(3);
    assert_eq!(ext_buttons(h) & EXT_BT_RELOAD, 0, "reload released");

    // Right mouse = ADS.
    h.mouse(0, 0, 0b10);
    h.run_tics(3);
    assert_ne!(ext_buttons(h) & EXT_BT_ADS, 0, "right button -> ADS");

    // Shift while ADS = hold breath, not sprint.
    h.key(KEY_RSHIFT, true);
    h.run_tics(3);
    let b = ext_buttons(h);
    assert_ne!(b & EXT_BT_HOLD_BREATH, 0, "shift + ADS -> hold breath");
    assert_eq!(b & EXT_BT_SPRINT, 0, "shift + ADS is not sprint");

    // Releasing ADS leaves Shift alone = sprint.
    h.mouse(0, 0, 0);
    h.run_tics(3);
    let b = ext_buttons(h);
    assert_ne!(b & EXT_BT_SPRINT, 0, "shift alone -> sprint");
    assert_eq!(b & (EXT_BT_HOLD_BREATH | EXT_BT_ADS), 0);
    h.key(KEY_RSHIFT, false);
    h.run_tics(3);
    assert_eq!(ext_buttons(h) & EXT_BT_SPRINT, 0, "sprint released");

    // Fire (left mouse) is mirrored.
    h.mouse(0, 0, 0b1);
    h.run_tics(3);
    assert_ne!(
        ext_buttons(h) & EXT_BT_ATTACK,
        0,
        "left button -> attack mirror"
    );
    h.mouse(0, 0, 0);
    h.run_tics(3);

    // Always-run: forward without Shift uses the fast table (0x32) in MW2 mode.
    h.key(KEY_UPARROW, true);
    h.run_tics(3);
    assert_eq!(cmd(h).0, 0x32, "MW2 mode always runs");
    h.key(KEY_UPARROW, false);
    h.run_tics(3);

    // The right button is ADS, not strafe: mouse X turns instead of strafing.
    h.mouse(0, 0, 0b10);
    h.run_tics(2);
    h.mouse(10, 0, 0b10);
    h.run_tics(1);
    let (_, side, turn) = cmd(h);
    assert_eq!(side, 0, "mousebstrafe is ignored in MW2 mode");
    assert_ne!(turn, 0, "mouse X still turns");
    h.mouse(0, 0, 0);
    h.run_tics(3);

    // Vanilla: no always-run, and the right button strafes.
    h.set_mw2_mode(Some(false));
    h.run_tics(60);
    h.key(KEY_UPARROW, true);
    h.run_tics(3);
    assert_eq!(cmd(h).0, 0x19, "vanilla walks without Shift");
    h.key(KEY_UPARROW, false);
    h.run_tics(3);
    h.mouse(0, 0, 0b10);
    h.run_tics(2);
    h.mouse(10, 0, 0b10);
    h.run_tics(1);
    let (_, side, turn) = cmd(h);
    assert_ne!(side, 0, "vanilla right button strafes");
    assert_eq!(turn, 0);
    h.mouse(0, 0, 0);
    h.run_tics(3);
}

#[test]
fn input_headless() {
    let mut h = Harness::start_default();
    // SAFETY: reading an engine global on the engine thread.
    assert_eq!(unsafe { room::doom::d_player::consoleplayer }, 0);
    assert!(h.gametic() > 0);

    mw2_mouse_look(&mut h);
    vanilla_mouse(&mut h);
    ext_buttons_scenario(&mut h);

    h.set_mw2_mode(None);
}
