// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared headless test infrastructure (sheet 13 sections 5.2-5.4).
//!
//! A test binary includes this module with `mod support;`. It provides:
//!
//! - [`Harness`]: the real engine driven through its `DG_*` platform callbacks with a virtual
//!   clock, injected keys and mouse samples, and framebuffer capture;
//! - [`golden`]: PNG capture and golden comparison (`BLESS=1` to update);
//! - [`locations`]: pinned map spots on the pinned Freedoom with loud validity checks;
//! - [`alloc`]: an opt-in counting global allocator.
//!
//! Rules (sheet 13 section 5.2): the engine keeps its state in C-style globals, so there is
//! exactly one engine per process. Every `*_headless.rs` file therefore has exactly one
//! `#[test]` that runs its scenarios in sequence through one [`Harness`], restarting levels
//! with [`Harness::new_game`]. [`Harness::start`] panics if it is called twice in a process.
//!
//! The `DG_*` callbacks below are `#[no_mangle]` in the test binary, exactly like
//! `demo_playthrough.rs` (which keeps its own copy and does not use this module).
//! The engine thread is the test thread; all state here is thread-local or process-global
//! behind a mutex.

#![allow(dead_code, non_snake_case, non_upper_case_globals)]
// Thinker dispatch identifies mobjs by comparing `P_MobjThinker`, as the engine itself does.
#![allow(unpredictable_function_pointer_comparisons)]
// The DG_* callbacks are C-ABI entry points that receive pointers from the engine.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod alloc;
pub mod golden;
pub mod locations;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, CString};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[allow(unused_imports)]
pub use golden::{assert_golden, assert_golden_tol, Frame, Rect};

use dmw_format::consts::{FRACUNIT, PITCH_LIMIT_DEG, PITCH_UNITS_PER_DEGREE};
use room::dmw::ext::{PlayerExt, PLAYER_EXT};
use room::doom::d_loop::gametic;
use room::doom::d_player::{consoleplayer, players, PlayerT};
use room::doom::g_game::{gameaction, gamestate, G_DeferedInitNew};
use room::doom::i_video::{test_palette_rgb, usegamma, I_VideoBuffer};
use room::doom::info::MF_COUNTKILL;
use room::doom::p_map::P_TeleportMove;
use room::doom::p_mobj::{P_MobjThinker, P_RemoveMobj, P_SpawnMobj};
use room::doom::p_tick::{leveltime, thinkercap};

/// Doom map object, the full layout (`p_telept::mobj_t`); the type `players[i].mo` points to
/// once cast.
pub use room::doom::p_telept::mobj_t as Mobj;

/// Mirrors `room::headless::TICK_MS` (28 ms per tic, integer-truncated like the C engine).
pub const TICK_MS: u32 = 1000 / 35;

/// `gamestate_t` value for a running level.
const GS_LEVEL: c_int = 0;

// ---------------------------------------------------------------------------
// DG_* platform stubs (replace room/src/platform/mod.rs)
// ---------------------------------------------------------------------------

thread_local! {
    /// Virtual wall clock in ms. Starts at 1000 like the existing headless tests.
    static VIRTUAL_MS: Cell<u32> = const { Cell::new(1000) };
    /// Frames the engine handed to `DG_DrawFrame`.
    static FRAMES: Cell<u64> = const { Cell::new(0) };
    static KEYS: RefCell<VecDeque<(bool, u8)>> = const { RefCell::new(VecDeque::new()) };
    static MOUSE: RefCell<VecDeque<(i32, i32, u8)>> = const { RefCell::new(VecDeque::new()) };
}

#[no_mangle]
pub extern "C" fn DG_Init() {}

#[no_mangle]
pub extern "C" fn DG_DrawFrame() {
    FRAMES.with(|f| f.set(f.get() + 1));
}

/// The harness never sleeps.
#[no_mangle]
pub extern "C" fn DG_SleepMs(_ms: u32) {}

/// Returns the virtual clock and advances it by one tic, so any busy-wait inside the engine
/// (screen wipes, `I_GetTime` loops) terminates without real time passing.
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
// Log capture (sheet 13 section 7)
// ---------------------------------------------------------------------------

/// How many `dmw::*` lines are kept for the failure dump.
const LOG_RING: usize = 200;

static LOG_LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

struct CaptureLogger;

impl log::Log for CaptureLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("dmw") || metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("[{} {}] {}", record.level(), record.target(), record.args());
        if record.target().starts_with("dmw") {
            if let Ok(mut ring) = LOG_LINES.lock() {
                if ring.len() == LOG_RING {
                    ring.pop_front();
                }
                ring.push_back(line);
            }
        } else {
            eprintln!("{line}");
        }
    }

    fn flush(&self) {}
}

static LOGGER: CaptureLogger = CaptureLogger;

fn install_logger() {
    // Another logger (for example env_logger from a test) wins; the dump is then empty.
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

/// The captured `dmw::*` log lines, oldest first (at most 200).
pub fn captured_log() -> Vec<String> {
    LOG_LINES
        .lock()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Simulation state (sheet 07 types, sheet 08 `test-hooks` accessors)
// ---------------------------------------------------------------------------

/// The events the simulation reports (00 section 7.2).
pub use dmw_weapons::SimEvent;
/// The simulation's presentation snapshot (00 section 7.2).
pub use dmw_weapons::ViewState;

/// One `SimEvent` recorded by the engine's `test-hooks` ring buffer (sheet 08 section 5.13).
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedEvent {
    /// Level tic (`leveltime`) the event was produced on.
    pub tic: i32,
    /// The event.
    pub event: SimEvent,
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

static STARTED: AtomicBool = AtomicBool::new(false);

/// Locates the IWAD: `DMW_IWAD`, else `wads/freedoom1.wad`, else `doom1.wad` when it is a real
/// WAD. Panics with the sheet 01 section 5.9 message when none exists (no silent skip).
pub fn find_wad() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().expect("room has a parent directory");
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("DMW_IWAD") {
        candidates.push(PathBuf::from(p));
    }
    candidates.push(root.join("wads/freedoom1.wad"));
    candidates.push(root.join("doom1.wad"));
    for c in candidates {
        // A Git LFS pointer is a text file; only accept real IWADs.
        if std::fs::read(&c).is_ok_and(|b| b.starts_with(b"IWAD")) {
            return c;
        }
    }
    panic!("no IWAD found; run 'task fetch-wads' first");
}

/// The headless engine. See the module docs for the one-engine-per-process rule.
#[derive(Debug)]
pub struct Harness {
    /// Number of `run_tics` tics executed (not counting level-load waits).
    tics_run: u64,
    /// The engine is single-threaded; keep the handle `!Send`.
    _not_send: PhantomData<*const ()>,
}

impl Harness {
    /// Starts the engine once per process.
    ///
    /// `args` are Doom arguments without the program name, for example
    /// `["-iwad", wad, "-fixturepack", "-skill", "3", "-warp", "1", "1", "-nosound"]`.
    /// `-mb 256` (Freedoom needs more than room's 6 MiB zone, sheet 02 section 11, and level
    /// loads leak zone memory in this port),
    /// `-nosound`, `-nomusic` and `-nojoy` are appended unless already present. With `-warp` the
    /// call returns once the level is running; without it the engine is left at the title
    /// screen and [`Harness::new_game`] starts a level.
    ///
    /// `-fixturepack` (in-memory fixture pack, MW2 mode on) and `-nomw2` are honoured by the
    /// engine's `on_engine_init` (sheet 08).
    pub fn start(args: &[&str]) -> Harness {
        assert!(
            !STARTED.swap(true, Ordering::SeqCst),
            "Harness::start called twice in one process: the engine has global state, so each \
             *_headless.rs file must have exactly one #[test] (sheet 13 section 5.2)"
        );
        install_logger();

        let mut full: Vec<String> = vec!["iw4doom".to_owned()];
        full.extend(args.iter().map(|a| (*a).to_owned()));
        let has = |flag: &str| args.contains(&flag);
        if !has("-mb") {
            // The port's Z_FreeTags stops after 2000 blocks, so level memory is not returned
            // between `new_game` calls; 256 MiB (committed lazily) leaves room for dozens.
            full.push("-mb".to_owned());
            full.push("256".to_owned());
        }
        for flag in ["-nosound", "-nomusic", "-nojoy"] {
            if !has(flag) {
                full.push(flag.to_owned());
            }
        }

        let argv: Vec<CString> = full
            .iter()
            .map(|a| CString::new(a.as_str()).expect("no NUL in args"))
            .collect();
        let mut c_argv: Vec<*mut c_char> = argv.iter().map(|s| s.as_ptr().cast_mut()).collect();
        c_argv.push(std::ptr::null_mut());
        // SAFETY: single-threaded; `argv`/`c_argv` are leaked below because the engine keeps
        // `myargv` for the whole process.
        unsafe {
            doomgeneric_sys_singletics(true);
            doomgeneric_sys::doomgeneric_Create((c_argv.len() - 1) as c_int, c_argv.as_mut_ptr());
        }
        std::mem::forget(argv);
        std::mem::forget(c_argv);

        let mut h = Harness {
            tics_run: 0,
            _not_send: PhantomData,
        };
        if has("-warp") {
            h.wait_for_level();
        }
        h
    }

    /// [`Harness::start`] with the usual defaults: the located IWAD, skill 3, E1M1 (`-warp 1 1`).
    pub fn start_default() -> Harness {
        let wad = find_wad();
        let wad = wad.to_str().expect("WAD path is UTF-8");
        Harness::start(&["-iwad", wad, "-skill", "3", "-warp", "1", "1"])
    }

    /// Runs the engine until a level is running and the player exists.
    fn wait_for_level(&mut self) {
        for _ in 0..2000 {
            self.tick_once();
            // SAFETY: reading engine globals on the engine thread.
            let ready = unsafe {
                gamestate == GS_LEVEL && gameaction == 0 && !players[console()].mo.is_null()
            };
            if ready {
                return;
            }
        }
        panic!("level did not start within 2000 ticks");
    }

    /// One `doomgeneric_Tick` (one Doom tic in `singletics` mode).
    fn tick_once(&mut self) {
        // SAFETY: the engine was initialised in `start`.
        unsafe { doomgeneric_sys::doomgeneric_Tick() };
    }

    /// Starts a new game: `G_DeferedInitNew(skill, episode, map)` and runs until the level is
    /// running. `skill` is the engine value `0..=4` (`-skill 3` on the command line is `2`).
    /// Pending key and mouse samples are discarded so scenarios cannot leak into each other.
    pub fn new_game(&mut self, skill: i32, episode: i32, map: i32) {
        KEYS.with_borrow_mut(VecDeque::clear);
        MOUSE.with_borrow_mut(VecDeque::clear);
        // SAFETY: engine initialised; writes the deferred-new-game globals.
        unsafe { G_DeferedInitNew(skill, episode, map) };
        self.wait_for_level();
    }

    /// Advances virtual time by `n` tics: ticks the engine until `gametic` has advanced by
    /// exactly `n`. Panics if the engine stops making progress.
    pub fn run_tics(&mut self, n: u32) {
        let target = self.gametic() + n as i32;
        let mut guard = 0u32;
        while self.gametic() < target {
            self.tick_once();
            guard += 1;
            assert!(
                guard <= n * 4 + 64,
                "gametic stuck at {} while running {n} tics",
                self.gametic()
            );
        }
        assert_eq!(self.gametic(), target, "gametic overshot");
        self.tics_run += u64::from(n);
    }

    /// Queues a key event for `DG_GetKey`.
    pub fn key(&mut self, doom_key: u8, down: bool) {
        KEYS.with_borrow_mut(|q| q.push_back((down, doom_key)));
    }

    /// Queues one mouse sample for `DG_GetMouse` (sheet 05). `buttons` is the bit mask of held
    /// buttons (bit 0 left, bit 1 right, bit 2 middle).
    pub fn mouse(&mut self, dx: i32, dy: i32, buttons: u8) {
        MOUSE.with_borrow_mut(|q| q.push_back((dx, dy, buttons)));
    }

    /// Key down, `tics` tics, key up. The release is delivered by the next `run_tics`.
    pub fn hold(&mut self, doom_key: u8, tics: u32) {
        self.key(doom_key, true);
        self.run_tics(tics);
        self.key(doom_key, false);
    }

    /// `gametic` of the engine.
    pub fn gametic(&self) -> i32 {
        // SAFETY: reading a plain int on the engine thread.
        unsafe { gametic }
    }

    /// `leveltime`: tics since the current level started.
    pub fn level_time(&self) -> i32 {
        // SAFETY: as above.
        unsafe { leveltime }
    }

    /// Tics executed through [`Harness::run_tics`].
    pub fn tics_run(&self) -> u64 {
        self.tics_run
    }

    /// Frames the engine presented through `DG_DrawFrame`.
    pub fn frames_drawn(&self) -> u64 {
        FRAMES.with(Cell::get)
    }

    /// The virtual clock in ms.
    pub fn virtual_ms(&self) -> u32 {
        VIRTUAL_MS.with(Cell::get)
    }

    /// `players[consoleplayer]`.
    pub fn player(&self) -> &PlayerT {
        // SAFETY: `players` is a static array that lives for the whole process; the engine is
        // single-threaded and the returned borrow is only used between engine calls.
        unsafe { &*(&raw const players).cast::<PlayerT>().add(console()) }
    }

    /// The console player's map object.
    pub fn mobj(&self) -> &Mobj {
        // SAFETY: once a level runs, `mo` is a live `mobj_t` allocated in the zone; the cast
        // goes from the opaque handle to the full layout (checked by the C layout guard tests).
        unsafe {
            let mo = self.player().mo.cast::<Mobj>();
            assert!(
                !mo.is_null(),
                "player has no map object (no level running?)"
            );
            &*mo
        }
    }

    /// Player position in Doom map units.
    pub fn pos(&self) -> (f32, f32) {
        let m = self.mobj();
        (m.x as f32 / FRACUNIT as f32, m.y as f32 / FRACUNIT as f32)
    }

    /// Copy of `PLAYER_EXT[consoleplayer]` (sheet 05).
    pub fn player_ext(&self) -> PlayerExt {
        // SAFETY: engine-thread-only static, plain data.
        unsafe { (*(&raw const PLAYER_EXT).cast::<PlayerExt>().add(console())).clone() }
    }

    /// Resets the extension state (pitch, buffered ext commands) as sheet 08's level start will.
    pub fn reset_ext(&mut self) {
        room::dmw::ext::reset_level();
    }

    /// Forces MW2 mode for the level via the `test-hooks` override: `Some(true)` turns input
    /// and freelook on without a pack, `Some(false)` turns everything off (like `-nomw2`),
    /// `None` returns to the engine's gating. The weapon glue needs a real pack: start with
    /// `-fixturepack` for weapon tests.
    pub fn set_mw2_mode(&mut self, forced: Option<bool>) {
        room::dmw::mode::test_force_level_active(forced);
    }

    /// The simulation's view state (`None` without a pack).
    pub fn sim_view(&self) -> Option<ViewState> {
        room::dmw::test_api::view_state()
    }

    /// Moves the player (`P_TeleportMove`, which telefrags whatever stands there) and turns
    /// them to `angle_deg` (counter-clockwise from +x). Positions are Doom map units. The view
    /// height is refreshed by the next tic, so run at least one tic before [`Harness::frame`].
    pub fn teleport(&mut self, x: f32, y: f32, angle_deg: f32) {
        let fx = (x * FRACUNIT as f32).round() as i32;
        let fy = (y * FRACUNIT as f32).round() as i32;
        let angle = dmw_format::convert::degrees_to_bam(f64::from(angle_deg));
        self.teleport_fixed(fx, fy, angle);
    }

    /// [`Harness::teleport`] with fixed-point coordinates and a BAM angle.
    fn teleport_fixed(&mut self, fx: i32, fy: i32, angle: u32) {
        // SAFETY: the player's mobj is live while a level runs; `P_TeleportMove` relinks it.
        unsafe {
            let mo = players[console()].mo.cast::<Mobj>();
            assert!(!mo.is_null(), "teleport without a running level");
            let ok = P_TeleportMove(mo.cast(), fx, fy);
            assert_ne!(ok, 0, "P_TeleportMove to ({fx}, {fy}) fixed failed");
            (*mo).z = (*mo).floorz;
            (*mo).momx = 0;
            (*mo).momy = 0;
            (*mo).momz = 0;
            (*mo).angle = angle;
            players[console()].deltaviewheight = 0;
        }
    }

    /// Sets the console player's pitch in degrees (MW2 convention: positive looks down),
    /// clamped to the freelook limit. Writes `PLAYER_EXT.pitch_q` and `prev_pitch_q`.
    pub fn set_pitch(&mut self, deg: f32) {
        let q =
            (deg.clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG) * PITCH_UNITS_PER_DEGREE).round() as i32;
        // SAFETY: engine-thread-only static, plain data.
        unsafe {
            let ext = &mut *(&raw mut PLAYER_EXT).cast::<PlayerExt>().add(console());
            ext.pitch_q = q;
            ext.prev_pitch_q = q;
        }
    }

    /// `P_SpawnMobj` on the floor at `(x, y)` (Doom map units). Returns the new object.
    pub fn spawn(&mut self, mobjtype: c_int, x: f32, y: f32) -> *mut Mobj {
        let fx = (x * FRACUNIT as f32).round() as i32;
        let fy = (y * FRACUNIT as f32).round() as i32;
        // SAFETY: a level is running (zone memory and sectors exist); `ONFLOORZ` is `i32::MIN`.
        unsafe { P_SpawnMobj(fx, fy, i32::MIN, mobjtype) }
    }

    /// Calls `f` on every live map object (skipping thinkers marked for removal).
    fn for_each_mobj(&self, mut f: impl FnMut(*mut Mobj)) {
        // SAFETY: the thinker list is a circular list headed by `thinkercap`; removal only
        // marks thinkers (the list stays intact while we walk it).
        unsafe {
            let cap = &raw mut thinkercap;
            let mut th = (*cap).next;
            while !std::ptr::eq(th, cap) {
                let is_mobj = (*th).function.acp1
                    == Some(std::mem::transmute::<
                        unsafe extern "C" fn(*mut Mobj),
                        unsafe extern "C" fn(*mut std::ffi::c_void),
                    >(P_MobjThinker));
                let next = (*th).next;
                if is_mobj {
                    f(th.cast::<Mobj>());
                }
                th = next;
            }
        }
    }

    /// `P_RemoveMobj` on every `MF_COUNTKILL` thing (the "empty level" helper of sheet 13
    /// section 5.3). Returns how many were removed.
    pub fn remove_monsters(&mut self) -> usize {
        let mut removed = 0;
        self.for_each_mobj(|m| {
            // SAFETY: `m` is a live mobj from the thinker list.
            unsafe {
                if (*m).flags & MF_COUNTKILL != 0 {
                    P_RemoveMobj(m);
                    removed += 1;
                }
            }
        });
        removed
    }

    /// Copies of the live map objects as `(type, x, y, flags)` in Doom map units.
    pub fn mobjs(&self) -> Vec<(c_int, f32, f32, c_int)> {
        let mut out = Vec::new();
        self.for_each_mobj(|m| {
            // SAFETY: `m` is a live mobj from the thinker list.
            let m = unsafe { &*m };
            out.push((
                m.mobjtype,
                m.x as f32 / FRACUNIT as f32,
                m.y as f32 / FRACUNIT as f32,
                m.flags,
            ));
        });
        out
    }

    /// Gives weapons, ammo, armour and keys through the engine's own `IDKFA` cheat path
    /// (key events), so it works exactly like a player typing it. Needs a non-nightmare skill.
    /// In MW2 mode the next tic's ownership sync gives every mapped MW2 weapon a full magazine
    /// (sheet 08 section 5.4); this runs that tic before returning.
    pub fn give_all(&mut self) {
        // The cheat's letters are real key presses, and `d`, `a` and `f` are strafe right,
        // strafe left and use: put the player back where they stood afterwards.
        let (x, y, angle) = {
            let m = self.mobj();
            (m.x, m.y, m.angle)
        };
        for c in b"idkfa" {
            self.key(*c, true);
            self.run_tics(1);
            self.key(*c, false);
            self.run_tics(1);
        }
        let p = self.player();
        assert!(
            p.weaponowned.iter().all(|&w| w != 0) && p.armorpoints == 200,
            "IDKFA did not take effect (nightmare skill, netgame or a menu is open?)"
        );
        self.teleport_fixed(x, y, angle);
    }

    /// Copy of `I_VideoBuffer` (320x200 palette indices) with the current palette.
    /// Panics if the gamma level is not 0 (goldens are defined at gamma 0).
    pub fn frame(&mut self) -> Frame {
        // SAFETY: `I_VideoBuffer` is a live 320*200 zone allocation after video init.
        let (indices, gamma) = unsafe {
            assert!(!I_VideoBuffer.is_null(), "video not initialised");
            (
                std::slice::from_raw_parts(I_VideoBuffer, golden::FRAME_W * golden::FRAME_H)
                    .to_vec(),
                usegamma,
            )
        };
        assert_eq!(
            gamma, 0,
            "goldens are defined at gamma 0 (usegamma = {gamma})"
        );
        Frame {
            indices,
            palette: test_palette_rgb(),
        }
    }

    /// `SimEvent`s of the last tics (at most 256), recorded by the engine's `test-hooks` ring
    /// buffer, oldest first.
    pub fn events(&self) -> Vec<RecordedEvent> {
        room::dmw::test_api::events()
            .into_iter()
            .map(|(tic, event)| RecordedEvent { tic, event })
            .collect()
    }

    /// Empties the engine's event ring.
    pub fn clear_events(&mut self) {
        room::dmw::test_api::clear_events();
    }

    /// The captured `dmw::*` log lines (also dumped automatically when a test panics).
    pub fn log_lines(&self) -> Vec<String> {
        captured_log()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let lines = captured_log();
            eprintln!("--- last {} dmw::* log lines ---", lines.len());
            for l in lines {
                eprintln!("{l}");
            }
        }
    }
}

fn console() -> usize {
    // SAFETY: plain int read on the engine thread.
    unsafe { consoleplayer as usize }
}

/// Sets the engine's `singletics` flag so each `doomgeneric_Tick` runs exactly one tic, and
/// clears `longtics` (the demo format flag), as the existing headless tests do.
///
/// # Safety
/// Writes engine globals; call on the engine thread before `doomgeneric_Create`.
unsafe fn doomgeneric_sys_singletics(on: bool) {
    extern "C" {
        static mut singletics: std::ffi::c_uint;
        static mut longtics: std::ffi::c_uint;
    }
    // SAFETY: caller contract.
    unsafe {
        singletics = u32::from(on);
        longtics = 0;
    }
}
