//! Headless input tests for sheet 05 (T5.4, T5.5, T5.6).
//!
//! Drives the real engine on Freedoom E1M1 with injected `DG_GetKey` / `DG_GetMouse`
//! samples, in the style of `demo_playthrough.rs`. One engine per process (Doom globals), so
//! there is exactly one `#[test]` that runs its scenarios in sequence.
//!
//! MW2 mode is switched with `room::dmw::mode::test_force_level_active` (the `test-hooks`
//! feature, enabled for tests through the self dev-dependency in `room/Cargo.toml`) because
//! sheet 08's real mode gating and sheet 03's `-fixturepack` do not exist on this branch.
//! `Some(false)` stands in for `-nomw2`, `Some(true)` for "pack loaded and enabled".
//!
//! The local `Harness` is intentionally minimal; sheet 13 section 5.2 owns the shared one.

#![allow(non_snake_case, non_upper_case_globals)]
// The DG_* callbacks are C-ABI entry points that receive pointers from the engine.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_uint, CString};
use std::path::PathBuf;

use room::dmw::ext::{
    EXT_BT_ADS, EXT_BT_ATTACK, EXT_BT_HOLD_BREATH, EXT_BT_RELOAD, EXT_BT_SPRINT, EXT_BT_VALID,
    PLAYER_EXT,
};
use room::dmw::mode::test_force_level_active;
use room::doom::d_player::{consoleplayer, players};
use room::doom::doomkeys::{KEY_RSHIFT, KEY_UPARROW};

// ---------------------------------------------------------------------------
// DG_* stubs (replace room/src/platform/mod.rs)
// ---------------------------------------------------------------------------

/// Mirrors `room::headless::TICK_MS`.
const TICK_MS: u32 = 1000 / 35;

thread_local! {
    static VIRTUAL_MS: Cell<u32> = const { Cell::new(1000) };
    static KEYS: RefCell<VecDeque<(bool, u8)>> = const { RefCell::new(VecDeque::new()) };
    static MOUSE: RefCell<VecDeque<(i32, i32, u8)>> = const { RefCell::new(VecDeque::new()) };
}

#[no_mangle]
pub extern "C" fn DG_Init() {}

#[no_mangle]
pub extern "C" fn DG_DrawFrame() {}

#[no_mangle]
pub extern "C" fn DG_SleepMs(_ms: u32) {}

#[no_mangle]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    VIRTUAL_MS.with(|v| {
        let old = v.get();
        v.set(old + TICK_MS);
        old
    })
}

#[no_mangle]
pub extern "C" fn DG_GetKey(pressed: *mut i32, doom_key: *mut u8) -> i32 {
    KEYS.with_borrow_mut(|q| match q.pop_front() {
        Some((down, key)) => {
            // SAFETY: the engine passes valid pointers.
            unsafe {
                *pressed = i32::from(down);
                *doom_key = key;
            }
            1
        }
        None => 0,
    })
}

#[no_mangle]
pub extern "C" fn DG_GetMouse(dx: *mut i32, dy: *mut i32, buttons: *mut u8) -> i32 {
    MOUSE.with_borrow_mut(|q| match q.pop_front() {
        Some((x, y, b)) => {
            // SAFETY: the engine passes valid pointers.
            unsafe {
                *dx = x;
                *dy = y;
                *buttons = b;
            }
            1
        }
        None => 0,
    })
}

#[no_mangle]
pub extern "C" fn DG_SetWindowTitle(_title: *const c_char) {}

// ---------------------------------------------------------------------------
// Engine globals
// ---------------------------------------------------------------------------

extern "C" {
    static gametic: c_int;
    static gamestate: c_int;
    static mut singletics: c_uint;
    static mut longtics: c_uint;
}

/// Prefix of `mobj_s` up to `angle` (see `demo_playthrough.rs`).
#[repr(C)]
struct MobjPrefix {
    thinker_prev: *mut (),
    thinker_next: *mut (),
    thinker_fn: *mut (),
    x: i32,
    y: i32,
    z: i32,
    snext: *mut (),
    sprev: *mut (),
    angle: u32,
}

fn find_wad() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().expect("room has a parent directory");
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("DMW_IWAD") {
        candidates.push(PathBuf::from(p));
    }
    candidates.push(root.join("wads/freedoom1.wad"));
    for c in candidates {
        if c.is_file() {
            return c;
        }
    }
    panic!("no IWAD found; run 'task fetch-wads' first");
}

struct Harness;

impl Harness {
    fn start() -> Harness {
        let wad = find_wad();
        let args = [
            "doommw2",
            "-iwad",
            wad.to_str().expect("WAD path is UTF-8"),
            "-mb",
            "32",
            "-nosound",
            "-nomusic",
            "-nojoy",
            "-skill",
            "3",
            "-warp",
            "1",
            "1",
        ];
        let argv: Vec<CString> = args
            .iter()
            .map(|a| CString::new(*a).expect("no NUL in args"))
            .collect();
        let mut c_argv: Vec<*mut c_char> = argv.iter().map(|s| s.as_ptr().cast_mut()).collect();
        c_argv.push(std::ptr::null_mut());
        // SAFETY: single-threaded test; `argv` outlives the engine (leaked below because the
        // engine keeps `myargv` for the whole process).
        unsafe {
            singletics = 1;
            longtics = 0;
            doomgeneric_sys::doomgeneric_Create((c_argv.len() - 1) as c_int, c_argv.as_mut_ptr());
        }
        std::mem::forget(argv);
        std::mem::forget(c_argv);
        Harness
    }

    fn run_tics(&mut self, n: u32) {
        for _ in 0..n {
            // SAFETY: engine initialised in `start`.
            unsafe { doomgeneric_sys::doomgeneric_Tick() };
        }
    }

    /// Runs until the level is running (`GS_LEVEL`) and the player exists.
    fn wait_for_level(&mut self) {
        for _ in 0..2000 {
            self.run_tics(1);
            // SAFETY: reading engine globals on the engine thread.
            let ready = unsafe { gamestate == 0 && !players[0].mo.is_null() };
            if ready {
                return;
            }
        }
        panic!("level did not start within 2000 ticks");
    }

    fn key(&mut self, doom_key: u8, down: bool) {
        KEYS.with_borrow_mut(|q| q.push_back((down, doom_key)));
    }

    fn mouse(&mut self, dx: i32, dy: i32, buttons: u8) {
        MOUSE.with_borrow_mut(|q| q.push_back((dx, dy, buttons)));
    }

    fn pos(&self) -> (i32, i32) {
        // SAFETY: the player's mobj exists once the level runs; layout matches `mobj_s`.
        unsafe {
            let mo = players[0].mo as *const MobjPrefix;
            ((*mo).x, (*mo).y)
        }
    }

    fn angle(&self) -> u32 {
        // SAFETY: as in `pos`.
        unsafe { (*(players[0].mo as *const MobjPrefix)).angle }
    }

    fn pitch_q(&self) -> i32 {
        // SAFETY: reading a plain int of the engine-thread-only static.
        unsafe { (*(&raw const PLAYER_EXT))[0].pitch_q }
    }

    fn ext_buttons(&self) -> u16 {
        // SAFETY: as above.
        unsafe { (*(&raw const PLAYER_EXT))[0].cmd.buttons }
    }

    fn cmd(&self) -> (i8, i8, i16) {
        // SAFETY: reading the console player's current ticcmd.
        unsafe {
            let c = &players[0].cmd;
            (c.forwardmove, c.sidemove, c.angleturn)
        }
    }

    fn reset_pitch(&mut self) {
        room::dmw::ext::reset_level();
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// T5.4: in MW2 mode mouse Y changes pitch by the formula and does not move the player.
fn mw2_mouse_look(h: &mut Harness) {
    test_force_level_active(Some(true));
    h.reset_pitch();
    h.run_tics(2);
    let before = h.pos();
    let angle_before = h.angle();

    // +50 counts (mouse moved toward the user = look down) for 10 tics.
    for _ in 0..10 {
        h.mouse(0, 50, 0);
        h.run_tics(1);
    }
    h.run_tics(3); // let the last ticcmd reach G_Ticker

    // 500 counts * 8 * 360 / 65536 degrees * 128 units/degree = 2812.5 units, the half unit
    // stays in the accumulator.
    assert_eq!(h.pitch_q(), 2812, "pitch_q after 500 counts");
    assert_eq!(
        h.pos(),
        before,
        "mouse Y must not move the player in MW2 mode"
    );
    assert_eq!(h.angle(), angle_before, "no horizontal motion was injected");
    assert_eq!(h.cmd().0, 0, "no forward move in the ticcmd");

    // Mouse forward looks up (negative pitch) and the clamp holds at 32 degrees.
    for _ in 0..40 {
        h.mouse(0, 100, 0);
        h.run_tics(1);
    }
    h.run_tics(3);
    assert_eq!(h.pitch_q(), 32 * 128, "pitch clamps at +32 degrees");
    for _ in 0..80 {
        h.mouse(0, -100, 0);
        h.run_tics(1);
    }
    h.run_tics(3);
    assert_eq!(h.pitch_q(), -32 * 128, "pitch clamps at -32 degrees");

    // Mouse X still turns the player (yaw path is shared with vanilla).
    h.mouse(40, 0, 0);
    h.run_tics(3);
    assert_ne!(h.angle(), angle_before, "mouse X turns the player");
}

/// T5.5: with MW2 mode off the same input leaves pitch at 0 and mouse Y moves the player.
fn vanilla_mouse(h: &mut Harness) {
    test_force_level_active(Some(false));
    h.reset_pitch();
    h.run_tics(3);
    let before = h.pos();

    for _ in 0..10 {
        h.mouse(0, -20, 0); // mouse moved forward
        h.run_tics(1);
    }
    h.run_tics(3);

    assert_eq!(h.pitch_q(), 0, "no pitch outside MW2 mode");
    assert_ne!(h.pos(), before, "mouse Y moves the player in vanilla mode");
    // Let the momentum die before the next scenario.
    h.run_tics(60);
}

/// T5.6: extended buttons, always-run and the masked strafe/forward mouse buttons.
fn ext_buttons(h: &mut Harness) {
    test_force_level_active(Some(true));
    h.reset_pitch();
    h.run_tics(3);

    // Reload key.
    h.key(b'r', true);
    h.run_tics(3);
    let b = h.ext_buttons();
    assert_ne!(b & EXT_BT_VALID, 0, "slot written by on_build_ticcmd");
    assert_ne!(b & EXT_BT_RELOAD, 0, "r -> reload");
    assert_eq!(b & EXT_BT_ADS, 0);
    h.key(b'r', false);
    h.run_tics(3);
    assert_eq!(h.ext_buttons() & EXT_BT_RELOAD, 0, "reload released");

    // Right mouse = ADS.
    h.mouse(0, 0, 0b10);
    h.run_tics(3);
    assert_ne!(h.ext_buttons() & EXT_BT_ADS, 0, "right button -> ADS");

    // Shift while ADS = hold breath, not sprint.
    h.key(KEY_RSHIFT, true);
    h.run_tics(3);
    let b = h.ext_buttons();
    assert_ne!(b & EXT_BT_HOLD_BREATH, 0, "shift + ADS -> hold breath");
    assert_eq!(b & EXT_BT_SPRINT, 0, "shift + ADS is not sprint");

    // Releasing ADS leaves Shift alone = sprint.
    h.mouse(0, 0, 0);
    h.run_tics(3);
    let b = h.ext_buttons();
    assert_ne!(b & EXT_BT_SPRINT, 0, "shift alone -> sprint");
    assert_eq!(b & (EXT_BT_HOLD_BREATH | EXT_BT_ADS), 0);
    h.key(KEY_RSHIFT, false);
    h.run_tics(3);
    assert_eq!(h.ext_buttons() & EXT_BT_SPRINT, 0, "sprint released");

    // Fire (left mouse) is mirrored.
    h.mouse(0, 0, 0b1);
    h.run_tics(3);
    assert_ne!(
        h.ext_buttons() & EXT_BT_ATTACK,
        0,
        "left button -> attack mirror"
    );
    h.mouse(0, 0, 0);
    h.run_tics(3);

    // Always-run: forward without Shift uses the fast table (0x32) in MW2 mode.
    h.key(KEY_UPARROW, true);
    h.run_tics(3);
    assert_eq!(h.cmd().0, 0x32, "MW2 mode always runs");
    h.key(KEY_UPARROW, false);
    h.run_tics(3);

    // The right button is ADS, not strafe: mouse X turns instead of strafing.
    h.mouse(0, 0, 0b10);
    h.run_tics(2);
    h.mouse(10, 0, 0b10);
    h.run_tics(1);
    let (_, side, turn) = h.cmd();
    assert_eq!(side, 0, "mousebstrafe is ignored in MW2 mode");
    assert_ne!(turn, 0, "mouse X still turns");
    h.mouse(0, 0, 0);
    h.run_tics(3);

    // Vanilla: no always-run, and the right button strafes.
    test_force_level_active(Some(false));
    h.run_tics(60);
    h.key(KEY_UPARROW, true);
    h.run_tics(3);
    assert_eq!(h.cmd().0, 0x19, "vanilla walks without Shift");
    h.key(KEY_UPARROW, false);
    h.run_tics(3);
    h.mouse(0, 0, 0b10);
    h.run_tics(2);
    h.mouse(10, 0, 0b10);
    h.run_tics(1);
    let (_, side, turn) = h.cmd();
    assert_ne!(side, 0, "vanilla right button strafes");
    assert_eq!(turn, 0);
    h.mouse(0, 0, 0);
    h.run_tics(3);
}

#[test]
fn input_headless() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .try_init();
    let mut h = Harness::start();
    h.wait_for_level();
    // SAFETY: engine initialised.
    assert_eq!(unsafe { consoleplayer }, 0);
    assert!(unsafe { gametic } > 0);

    mw2_mouse_look(&mut h);
    vanilla_mouse(&mut h);
    ext_buttons(&mut h);

    test_force_level_active(None);
}
