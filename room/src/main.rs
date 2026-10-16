//! `room` – a Rust port of doomgeneric using winit and wgpu.
//!
//! This binary provides the platform back-end for the doomgeneric Doom engine.
//! The engine is compiled from the vendored C source (see `doomgeneric-sys`)
//! and linked into this binary.  The Rust code provides:
//!
//! - A winit window for display and keyboard input.
//! - A wgpu renderer that blits the engine's 640 × 400 frame buffer to the
//!   window each tick.
//! - Implementations of the six `DG_*` C callbacks required by doomgeneric.
//!
//! ## Usage
//!
//! ```text
//! room -- -iwad /path/to/doom1.wad
//! ```
//!
//! Any arguments following `--` are forwarded to the Doom engine as-is.
//! A DOOM WAD file (`doom1.wad`, `doom.wad`, `doom2.wad`, etc.) must be
//! provided.
//!
//! ## Architecture
//!
//! ```text
//!  main()
//!   └─ EventLoop::run_app(&mut App)
//!       ├─ App::resumed()      → create Window + GpuState, store in TLS
//!       ├─ App::window_event() → push key events into TLS queue
//!       └─ App::about_to_wait()
//!               first call  → doomgeneric_Create(argc, argv)
//!               later calls → doomgeneric_Tick()
//!                                ├─ I_StartTic() → I_GetEvent() → DG_GetKey()
//!                                └─ I_FinishUpdate() → DG_DrawFrame()
//! ```
//!
//! The six `DG_*` functions (defined in `platform/mod.rs`) access window and
//! GPU resources through [`thread_local!`] statics, which is safe because
//! everything runs on the main thread.

/// Heap-profiling global allocator, enabled only when the `dhat-heap`
/// Cargo feature is active.  The matching [`dhat::Profiler::new_heap`] handle
/// in [`main`] enables collection for the lifetime of the binary.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

mod gpu;
mod platform;

use std::ffi::{c_char, c_int, CString};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use room::doom::doomgeneric::{DOOMGENERIC_RESX, DOOMGENERIC_RESY};

use gpu::GpuState;
use platform::keys::to_doom_key;
use platform::{GPU, KEY_QUEUE, QUIT_REQUESTED, WINDOW};

// ---------------------------------------------------------------------------
// App – the winit ApplicationHandler
// ---------------------------------------------------------------------------

/// Application state threaded through the winit event loop.
struct App {
    /// Command-line arguments to forward to `doomgeneric_Create`.
    ///
    /// Stored as `CString`s to ensure they remain alive for the entire
    /// duration of the programme (the engine saves `myargv` permanently).
    /// The field is never read back; its purpose is to prevent the `CString`
    /// heap allocations from being freed while the C engine is running.
    #[allow(dead_code)]
    args: Vec<CString>,

    /// Raw C-style pointer array derived from `args`, passed to
    /// `doomgeneric_Create` as `argv`.
    argv: Vec<*mut c_char>,

    /// `true` after `doomgeneric_Create` has returned and the first tick
    /// has been executed, so that subsequent `about_to_wait` calls use
    /// `doomgeneric_Tick` instead.
    doom_initialized: bool,

    /// `doommw2.toml` `[render] aspect_correct`: 4:3 letterboxed presentation.
    aspect_correct: bool,
}

/// Manual [`Send`] impl: the raw pointers in `argv` reference `CString` data
/// owned by `args` in the same struct, so moving `App` between threads keeps
/// the pointers valid.  In practice `App` is only used on the main thread.
///
/// # Safety
///
/// The raw pointers in `argv` point into the `CString` data owned by `args`,
/// which lives alongside them and is never freed for the duration of the
/// programme.  Concurrent mutation is impossible because `App` is only ever
/// used on the main thread (the winit event loop does not send it across
/// threads), so the non-`Send` raw pointers are fine.
unsafe impl Send for App {}

/// Constructor and helpers for [`App`].
impl App {
    /// Build an `App` from the OS command-line arguments.
    ///
    /// Arguments are converted to null-terminated C strings, and a parallel
    /// `argv` array of raw pointers is built.
    fn new(aspect_correct: bool) -> Self {
        let args: Vec<CString> = std::env::args()
            .map(|a| CString::new(a).expect("argument contained null byte"))
            .collect();

        let mut argv: Vec<*mut c_char> = args.iter().map(|s| s.as_ptr() as *mut c_char).collect();
        // The C standard requires `argv[argc]` to be a null pointer.
        argv.push(std::ptr::null_mut());

        Self {
            args,
            argv,
            doom_initialized: false,
            aspect_correct,
        }
    }
}

/// Winit event-loop integration: window/GPU bring-up, input forwarding, and
/// the per-iteration game tick.
impl ApplicationHandler for App {
    /// Called when the application becomes active (e.g. on launch or resume).
    ///
    /// Creates the OS window, initialises wgpu, and stores both in the
    /// thread-local platform state so the `DG_*` callbacks can reach them.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Sheet 06 section 5.6: 960x720 (the 320x200 frame at 4:3) when aspect-correcting,
        // otherwise the native 640x400; resizable either way, the frame is scaled to fit.
        let (win_w, win_h) = if self.aspect_correct {
            (960, 720)
        } else {
            (DOOMGENERIC_RESX as u32, DOOMGENERIC_RESY as u32)
        };
        log::info!(
            "Creating window ({win_w}×{win_h}) for the {}×{} frame, aspect_correct = {}",
            DOOMGENERIC_RESX,
            DOOMGENERIC_RESY,
            self.aspect_correct
        );

        let window_attrs = Window::default_attributes()
            .with_title("DOOMMW2")
            .with_inner_size(LogicalSize::new(win_w, win_h))
            .with_min_inner_size(LogicalSize::new(320u32, 240u32))
            .with_resizable(true);

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => std::sync::Arc::new(w),
            Err(e) => {
                log::error!("Failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        room::dmw::viewmodel::set_window_size(size.width, size.height);

        // Initialise the GPU renderer.
        match GpuState::new(window.clone(), self.aspect_correct) {
            Ok(gpu) => {
                GPU.with_borrow_mut(|g| *g = Some(gpu));
                WINDOW.with_borrow_mut(|w| *w = Some(window));
                platform::init_start_time();
                log::info!("GPU initialised");
            }
            Err(e) => {
                log::error!("Failed to initialise GPU: {e}");
                event_loop.exit();
            }
        }
    }

    /// Called for each windowed event (keyboard, resize, close request, …).
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested – exiting");
                QUIT_REQUESTED.with(|q| q.set(true));
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                room::dmw::viewmodel::set_window_size(new_size.width, new_size.height);
                GPU.with_borrow_mut(|g| {
                    if let Some(gpu) = g.as_mut() {
                        gpu.resize(new_size);
                    }
                });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                platform::on_mouse_button(button, state == ElementState::Pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => platform::on_mouse_wheel(delta),
            WindowEvent::CursorMoved { position, .. } => {
                platform::on_cursor_moved(position.x, position.y);
            }
            WindowEvent::Focused(focused) => platform::on_focus(focused),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    // Ignore auto-repeat key presses (key held down).
                    if event.repeat {
                        return;
                    }

                    if let Some(doom_key) = to_doom_key(code) {
                        let pressed = event.state == ElementState::Pressed;
                        KEY_QUEUE.with_borrow_mut(|q| q.push_back((pressed, doom_key)));
                    }
                }
            }

            _ => {}
        }
    }

    /// Raw device events: unaccelerated mouse motion while the cursor is captured.
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            platform::on_raw_motion(delta.0, delta.1);
        }
    }

    /// Called when all pending events have been processed.
    ///
    /// This is the main "game loop" hook: on the first call, the Doom engine
    /// is initialised; on subsequent calls, one game tick is executed.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Skip ticking if the GPU is not yet ready.
        let gpu_ready = GPU.with_borrow(|g| g.is_some());
        if !gpu_ready {
            return;
        }

        if QUIT_REQUESTED.with(|q| q.get()) {
            event_loop.exit();
            return;
        }

        // Grab or release the cursor before the tick consumes mouse input.
        platform::update_grab();

        if !self.doom_initialized {
            // First tick: initialise the engine.
            // SAFETY:
            // - `self.args` is alive for the lifetime of the programme.
            // - `self.argv` contains valid pointers into `self.args`.
            // - The `DG_*` callbacks are exported symbols in this binary.
            let argc = (self.argv.len() - 1) as c_int; // exclude trailing null
            unsafe {
                room::doom::doomgeneric::doomgeneric_Create(argc, self.argv.as_mut_ptr());
            }
            self.doom_initialized = true;
        } else {
            // Subsequent ticks: advance the game by one tick.
            room::doom::d_main::doomgeneric_Tick();
        }

        // Request a redraw so winit doesn't throttle to zero FPS while idle.
        WINDOW.with_borrow(|w| {
            if let Some(win) = w.as_ref() {
                win.request_redraw();
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Binary entry point.
///
/// Initialises logging and (optionally) the heap profiler, builds the winit
/// event loop, configures continuous polling so the Doom tick can run as
/// fast as possible, constructs the [`App`], and hands control to winit.  On
/// event-loop failure the process exits with status `1`.
fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    // Initialise the logger.  Set `RUST_LOG=debug` for verbose output.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("DOOMMW2 – room-based Doom with MW2 gunplay");

    // Mouse capture settings live in doommw2.toml `[input]`.
    let explicit_config = platform::mw2config_arg();
    let (config, _) = room::dmw::config::DmwConfig::load(explicit_config.as_deref());
    platform::configure_input(&config.input);

    let event_loop = EventLoop::new().expect("failed to create event loop");

    // Poll continuously so we can tick the game as fast as possible.
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = App::new(config.render.aspect_correct);
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("Event loop error: {e}");
        std::process::exit(1);
    }
}
