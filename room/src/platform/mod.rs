//! Platform implementation for the doomgeneric engine.
//!
//! This module provides the six C-callable functions that doomgeneric
//! requires from a platform back-end:
//!
//! | C function | Purpose |
//! |------------|---------|
//! | `DG_Init` | Platform initialisation (window / GPU already set up) |
//! | `DG_DrawFrame` | Present the current frame from `DG_ScreenBuffer` |
//! | `DG_SleepMs` | Sleep for a number of milliseconds |
//! | `DG_GetTicksMs` | Return elapsed milliseconds since startup |
//! | `DG_GetKey` | Pop the next keyboard event from the queue |
//! | `DG_GetMouse` | Take the accumulated mouse motion and button state (DOOMMW2) |
//! | `DG_SetWindowTitle` | Update the OS window title |
//!
//! All functions are exported with `#[no_mangle]` so the C linker can
//! resolve the symbols when linking the doomgeneric static library.
//!
//! ## Global state
//!
//! Because these functions are called from C code (which has no concept of
//! Rust ownership), all shared mutable state is stored in [`thread_local!`]
//! statics protected by [`std::cell::RefCell`].  This is safe because
//! **all** calls originate from the main thread (the winit event loop).
//!
//! This design is an intentional *functional approximation* of the original
//! C globals, to be refactored towards idiomatic Rust in future iterations.

pub mod keys;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_char;
use std::ffi::CStr;
use std::sync::Arc;
use std::time::Instant;

use winit::window::{CursorGrabMode, Window};

use room::dmw::config::InputCfg;
use room::dmw::input::{self, MouseAccum};

use crate::gpu::GpuState;

// ---------------------------------------------------------------------------
// Thread-local platform state
// ---------------------------------------------------------------------------

// GPU, window, keyboard and time state used by the DG_* callbacks.
// Doc comments are deliberately omitted on the macro invocation since
// rustdoc does not process comments attached to `thread_local!` calls.
thread_local! {
    // GPU rendering state.  `None` until `GpuState` is created in `resumed`.
    pub(crate) static GPU: RefCell<Option<GpuState>> = const { RefCell::new(None) };

    // OS window handle used for title updates.
    pub(crate) static WINDOW: RefCell<Option<Arc<Window>>> = const { RefCell::new(None) };

    // Keyboard event queue consumed by `DG_GetKey`.
    // Each entry is `(pressed, doom_key_byte)`.
    pub(crate) static KEY_QUEUE: RefCell<VecDeque<(bool, u8)>> =
        const { RefCell::new(VecDeque::new()) };

    // Absolute time of programme launch, used by `DG_GetTicksMs`.
    static START_TIME: Cell<Option<Instant>> = const { Cell::new(None) };

    // Set to `true` when the user asks to close the window.
    pub(crate) static QUIT_REQUESTED: Cell<bool> = const { Cell::new(false) };

    // Raw mouse state consumed by `DG_GetMouse`.
    pub(crate) static MOUSE: RefCell<MouseAccum> = const { RefCell::new(MouseAccum::new()) };

    // Cursor capture state (sheet 05 section 5.1).
    pub(crate) static CAPTURE: RefCell<MouseCapture> = RefCell::new(MouseCapture::new());
}

/// How the cursor is currently captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrabMode {
    /// Not captured.
    None,
    /// `CursorGrabMode::Locked`: the cursor is fixed and raw motion is reported.
    Locked,
    /// `CursorGrabMode::Confined`: the cursor stays inside the window.
    Confined,
    /// Neither is supported: motion comes from `CursorMoved` and the cursor is re-centred.
    Relative,
}

impl GrabMode {
    fn label(self) -> &'static str {
        match self {
            GrabMode::None => "none",
            GrabMode::Locked => "locked",
            GrabMode::Confined => "confined",
            GrabMode::Relative => "relative",
        }
    }
}

/// Cursor capture bookkeeping for the winit event handlers.
#[derive(Debug)]
pub(crate) struct MouseCapture {
    /// The window has keyboard focus.
    pub(crate) focused: bool,
    /// Current capture mode.
    pub(crate) mode: GrabMode,
    /// Grabbing is allowed (`[input] grab_on_start`, or the user has clicked once).
    pub(crate) armed: bool,
    /// `[input] raw_mouse`.
    pub(crate) raw_mouse: bool,
    /// Sub-notch pixel scroll left over from touchpads.
    wheel_pixels: f64,
    /// The capture mode has been logged at `info` once.
    logged_mode: bool,
}

impl MouseCapture {
    fn new() -> Self {
        let cfg = InputCfg::default();
        Self {
            focused: true,
            mode: GrabMode::None,
            armed: cfg.grab_on_start,
            raw_mouse: cfg.raw_mouse,
            wheel_pixels: 0.0,
            logged_mode: false,
        }
    }

    pub(crate) fn grabbed(&self) -> bool {
        self.mode != GrabMode::None
    }
}

/// Applies `[input]` settings (call before the event loop starts).
pub(crate) fn configure_input(cfg: &InputCfg) {
    CAPTURE.with_borrow_mut(|c| {
        c.raw_mouse = cfg.raw_mouse;
        c.armed = cfg.grab_on_start;
    });
}

/// Pre-engine search for `-mw2config <file>` so the platform layer can read `[input]`.
pub(crate) fn mw2config_arg() -> Option<std::path::PathBuf> {
    let mut args = std::env::args_os();
    while let Some(a) = args.next() {
        if a.to_string_lossy().eq_ignore_ascii_case("-mw2config") {
            return args.next().map(std::path::PathBuf::from);
        }
    }
    None
}

/// Raw device motion (only counted while captured with `raw_mouse` on and a real grab).
pub(crate) fn on_raw_motion(dx: f64, dy: f64) {
    let use_raw =
        CAPTURE.with_borrow(|c| c.grabbed() && c.mode != GrabMode::Relative && c.raw_mouse);
    if use_raw {
        MOUSE.with_borrow_mut(|m| m.add_motion(dx, dy));
    }
}

/// Cursor-position motion for `raw_mouse = false` and for the relative fallback: the offset
/// from the window centre is the delta, then the cursor is put back on the centre.
pub(crate) fn on_cursor_moved(x: f64, y: f64) {
    let use_cursor =
        CAPTURE.with_borrow(|c| c.grabbed() && (c.mode == GrabMode::Relative || !c.raw_mouse));
    if !use_cursor {
        return;
    }
    WINDOW.with_borrow(|w| {
        let Some(win) = w.as_ref() else { return };
        let size = win.inner_size();
        let (cx, cy) = (f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
        let (dx, dy) = (x - cx, y - cy);
        if dx != 0.0 || dy != 0.0 {
            MOUSE.with_borrow_mut(|m| m.add_motion(dx, dy));
            if let Err(e) = win.set_cursor_position(winit::dpi::PhysicalPosition::new(cx, cy)) {
                log::debug!(target: "dmw::input", "cannot re-centre cursor: {e}");
            }
        }
    });
}

/// A mouse button press or release. While not captured, a left or right press only arms the
/// grab and is not forwarded.
pub(crate) fn on_mouse_button(button: winit::event::MouseButton, pressed: bool) {
    use winit::event::MouseButton;
    let bit = match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(_) => return,
    };
    CAPTURE.with_borrow_mut(|c| {
        if !c.grabbed() {
            if pressed && bit <= 1 {
                c.armed = true;
            }
            return;
        }
        MOUSE.with_borrow_mut(|m| m.set_button(bit, pressed));
    });
}

/// Mouse wheel: one notch per line, or per 50 pixels of high-resolution scrolling.
pub(crate) fn on_mouse_wheel(delta: winit::event::MouseScrollDelta) {
    use winit::event::MouseScrollDelta;
    CAPTURE.with_borrow_mut(|c| {
        if !c.grabbed() {
            return;
        }
        let notches = match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                if y > 0.0 {
                    1
                } else if y < 0.0 {
                    -1
                } else {
                    0
                }
            }
            MouseScrollDelta::PixelDelta(p) => {
                c.wheel_pixels += p.y;
                let n = (c.wheel_pixels / input::WHEEL_PIXELS_PER_NOTCH).trunc();
                c.wheel_pixels -= n * input::WHEEL_PIXELS_PER_NOTCH;
                n as i32
            }
        };
        if notches != 0 {
            MOUSE.with_borrow_mut(|m| m.add_wheel(notches));
        }
    });
}

/// Focus change. Losing focus releases the capture and clears held buttons so fire cannot stick.
pub(crate) fn on_focus(focused: bool) {
    CAPTURE.with_borrow_mut(|c| c.focused = focused);
    if !focused {
        release_grab();
    }
}

/// Releases the cursor capture and reports every mouse button as released.
pub(crate) fn release_grab() {
    let was_grabbed = CAPTURE.with_borrow_mut(|c| std::mem::replace(&mut c.mode, GrabMode::None))
        != GrabMode::None;
    MOUSE.with_borrow_mut(|m| m.release_all());
    if !was_grabbed {
        return;
    }
    WINDOW.with_borrow(|w| {
        if let Some(win) = w.as_ref() {
            let _ = win.set_cursor_grab(CursorGrabMode::None);
            win.set_cursor_visible(true);
        }
    });
    log::debug!(target: "dmw::input", "mouse released");
}

/// Evaluates the grab policy against the engine state and grabs or releases the cursor.
///
/// `want_grab = focused && gamestate == GS_LEVEL && menuactive == 0 && paused == 0`, plus
/// `[input] grab_on_start` / a first click. Called every `about_to_wait`.
pub(crate) fn update_grab() {
    if input::take_release_request() {
        release_grab();
    }
    const GS_LEVEL: i32 = 0;
    // SAFETY: the engine globals are only touched from the main thread, which is where
    // winit calls `about_to_wait`.
    let (level, menu, paused) = unsafe {
        (
            room::doom::g_game::gamestate == GS_LEVEL,
            room::doom::m_menu::menuactive != 0,
            room::doom::g_game::paused != 0,
        )
    };
    let (focused, armed, grabbed) = CAPTURE.with_borrow(|c| (c.focused, c.armed, c.grabbed()));
    let want = armed && input::want_grab(focused, level, menu, paused);
    match (want, grabbed) {
        (true, false) => acquire_grab(),
        (false, true) => release_grab(),
        _ => {}
    }
}

/// Tries `Locked`, then `Confined`, then the relative fallback, logging the first result.
fn acquire_grab() {
    WINDOW.with_borrow(|w| {
        let Some(win) = w.as_ref() else { return };
        let raw = CAPTURE.with_borrow(|c| c.raw_mouse);
        let mut mode = GrabMode::Relative;
        // Locked reports raw motion only, so `raw_mouse = false` starts at Confined.
        if raw && win.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
            mode = GrabMode::Locked;
        } else if win.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
            mode = GrabMode::Confined;
        }
        win.set_cursor_visible(false);
        let first = CAPTURE.with_borrow_mut(|c| {
            c.mode = mode;
            !std::mem::replace(&mut c.logged_mode, true)
        });
        if first {
            log::info!(target: "dmw::input", "mouse captured ({})", mode.label());
        } else {
            log::debug!(target: "dmw::input", "mouse captured ({})", mode.label());
        }
        if mode == GrabMode::Relative {
            let size = win.inner_size();
            let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(
                f64::from(size.width) / 2.0,
                f64::from(size.height) / 2.0,
            ));
        }
    });
}

// ---------------------------------------------------------------------------
// Initialisation helper
// ---------------------------------------------------------------------------

/// Record the programme start time so [`DG_GetTicksMs`] can compute elapsed
/// milliseconds.  Must be called once, before `doomgeneric_Create`.
pub fn init_start_time() {
    START_TIME.with(|t| t.set(Some(Instant::now())));
}

// ---------------------------------------------------------------------------
// DG_* callbacks (C-callable)
// ---------------------------------------------------------------------------

/// Initialise the platform back-end.
///
/// Called by doomgeneric once, inside `doomgeneric_Create`, before the first
/// game tick.  Because the window and GPU are created by the winit event loop
/// *before* `doomgeneric_Create` is called, this function is a no-op: all
/// resources are already available.
#[no_mangle]
pub extern "C" fn DG_Init() {
    log::debug!("DG_Init: platform already initialised");
}

/// Present the current Doom frame to the screen.
///
/// Called by the engine's `I_FinishUpdate` function at the end of each
/// rendered frame.  Reads pixels from the C-owned `DG_ScreenBuffer` (640 ×
/// 400 pixels in BGRA8 format) and uploads them to the wgpu texture, then
/// issues a render pass that blits the texture to the surface.
///
/// # Safety
///
/// `DG_ScreenBuffer` must be non-null and point to at least
/// `DOOMGENERIC_RESX * DOOMGENERIC_RESY * 4` bytes of valid memory.
/// This is guaranteed by `doomgeneric_Create`, which allocates the buffer
/// before calling `DG_Init`.
#[no_mangle]
pub extern "C" fn DG_DrawFrame() {
    // SAFETY: DG_ScreenBuffer is allocated by doomgeneric_Create and is
    // valid for DOOMGENERIC_PIXELS * 4 bytes.
    let pixel_bytes = unsafe {
        let ptr = room::doom::doomgeneric::DG_ScreenBuffer as *const u8;
        if ptr.is_null() {
            log::warn!("DG_DrawFrame: DG_ScreenBuffer is null, skipping frame");
            return;
        }
        std::slice::from_raw_parts(ptr, room::doom::doomgeneric::DOOMGENERIC_PIXELS * 4)
    };

    GPU.with_borrow(|opt| {
        if let Some(gpu) = opt.as_ref() {
            if let Err(e) = gpu.render(pixel_bytes) {
                log::warn!("DG_DrawFrame: render error: {e}");
            }
        } else {
            log::warn!("DG_DrawFrame: GPU state not ready");
        }
    });
}

/// Sleep the calling thread for `ms` milliseconds.
///
/// Used by the engine to throttle the game loop when running ahead of
/// the target tick rate.
#[no_mangle]
pub extern "C" fn DG_SleepMs(ms: u32) {
    std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
}

/// Return the number of milliseconds elapsed since `doomgeneric_Create`.
///
/// The engine uses this to drive its fixed-rate game loop.
#[no_mangle]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    START_TIME.with(|t| {
        t.get()
            .map(|start| start.elapsed().as_millis() as u32)
            .unwrap_or(0)
    })
}

/// Pop the next keyboard event from the queue.
///
/// Returns `1` if an event was available, `0` if the queue is empty.
/// On success, writes the pressed state (1 = pressed, 0 = released) to
/// `*pressed` and the Doom key byte to `*doom_key`.
///
/// # Safety
///
/// `pressed` and `doom_key` must be valid, non-null pointers to writable
/// memory for their respective types.
#[no_mangle]
pub extern "C" fn DG_GetKey(pressed: *mut i32, doom_key: *mut u8) -> i32 {
    KEY_QUEUE.with_borrow_mut(|q| {
        if let Some((is_pressed, key)) = q.pop_front() {
            // SAFETY: caller guarantees the pointers are valid.
            unsafe {
                *pressed = i32::from(is_pressed);
                *doom_key = key;
            }
            1
        } else {
            0
        }
    })
}

/// Take the accumulated mouse sample (DOOMMW2, sheet 05).
///
/// Returns `1` and fills `*dx`, `*dy` (integer counts, positive `dy` = down) and `*buttons`
/// (bit 0 left, 1 right, 2 middle, 3 back, 4 forward, 5 wheel up, 6 wheel down) when the motion
/// or the button state changed since the last call; returns `0` otherwise.
///
/// # Safety
///
/// `dx`, `dy` and `buttons` must be valid, non-null pointers to writable memory.
#[no_mangle]
pub extern "C" fn DG_GetMouse(dx: *mut i32, dy: *mut i32, buttons: *mut u8) -> i32 {
    MOUSE.with_borrow_mut(|m| match m.take() {
        Some(sample) => {
            // SAFETY: caller guarantees the pointers are valid.
            unsafe {
                *dx = sample.dx;
                *dy = sample.dy;
                *buttons = sample.buttons;
            }
            1
        }
        None => 0,
    })
}

/// Public name shown in the window title.
pub const WINDOW_TITLE: &str = "IW4DOOM";

/// Update the OS window title.
///
/// Called by the engine when it discovers the game description from the
/// loaded WAD file (e.g. "The Ultimate DOOM"); shown as
/// "IW4DOOM - The Ultimate DOOM".
///
/// # Safety
///
/// `title` must be a valid, null-terminated C string for the duration of
/// this call.
#[no_mangle]
pub extern "C" fn DG_SetWindowTitle(title: *const c_char) {
    if title.is_null() {
        return;
    }

    // SAFETY: caller guarantees `title` is a valid C string.
    let s = unsafe { CStr::from_ptr(title) };
    let title_str = format!("{WINDOW_TITLE} - {}", s.to_string_lossy());

    WINDOW.with_borrow(|w| {
        if let Some(window) = w.as_ref() {
            window.set_title(&title_str);
        }
    });
}
