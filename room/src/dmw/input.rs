// SPDX-License-Identifier: GPL-3.0-or-later
//! Mouse accumulation, new key bindings, pitch and extended-button building.
//!
//! Sheet 05. The Doom call sites reach this module through [`super::hooks`]; the logic here
//! is split so that everything that does not touch Doom globals is a pure function with unit
//! tests (`PitchAccum`, `MouseAccum`, `compose_ext_buttons`, `find_conflicts`).
//!
//! Data flow for mouse look (MW2 mode):
//!
//! ```text
//! platform MouseAccum -> DG_GetMouse -> I_GetEvent -> ev_mouse -> G_Responder
//!     -> on_mouse_event (PitchAccum) -> on_build_ticcmd (ExtTiccmd.pitch_delta, EXT_TICCMDS)
//!     -> G_Ticker -> on_ticcmd_copied (PLAYER_EXT[i].pitch_q)
//! ```

use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, Ordering};

use log::{debug, trace, warn};

use crate::dmw::consts::DOOM_HFOV;
use crate::dmw::ext::{
    apply_pitch_to, ext_for_gametic, store_ext, ExtTiccmd, EXT_BT_ADS, EXT_BT_ATTACK,
    EXT_BT_HOLD_BREATH, EXT_BT_MELEE, EXT_BT_RELOAD, EXT_BT_SPRINT, EXT_BT_SWAP_LAST, EXT_BT_VALID,
    PLAYER_EXT,
};
use crate::dmw::mode;
use crate::doom::d_event::event_t;
use crate::doom::d_loop::{gametic, ticdup};
use crate::doom::d_player::{consoleplayer, players, TiccmdT, MAXPLAYERS};
use crate::doom::g_game::{game_key_down, mouse_button_down};
use crate::doom::m_controls as mc;

const TARGET: &str = "dmw::input";

/// `BT_ATTACK` ticcmd button bit.
const BT_ATTACK: u8 = 1;
/// `BT_SPECIAL` ticcmd button bit (the other bits then mean `BTS_*`).
const BT_SPECIAL: u8 = 128;
/// `PST_DEAD` player state.
const PST_DEAD: c_int = 1;

// ---------------------------------------------------------------------------
// Pitch accumulation
// ---------------------------------------------------------------------------

/// Yaw per mouse count in degrees is `8 * 360 / 65536` (`angleturn -= MOUSEX * 8`, 65536 BAM
/// units = 360 degrees). In `pitch_q` (1/128 degree) units that is `8 * 360 * 128 / 65536`.
const Q_NUM_PER_COUNT: i64 = 8 * 360 * 128;

/// Fixed-point scale of the zoom factor (16 fractional bits).
const ZOOM_ONE_Q16: i64 = 1 << 16;

/// Accumulator denominator: `pending` is in units of `1 / (10 * 2^32)` of a `pitch_q` unit.
///
/// The factor 10 is Doom's `(sensitivity + 5) / 10` scaling and `2^32` is the Q16 zoom factor
/// squared (one `2^16` from the degree conversion's `/ 65536`, one from the zoom). Keeping
/// both as exact denominators makes the accumulation additive: N samples sum to exactly the
/// same total as one combined sample, for any sensitivity and zoom.
const PENDING_DENOM: i64 = 10 * (1 << 32);

/// Fractional, drift-free pitch accumulator (sheet 05 section 5.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PitchAccum {
    pending: i64,
}

impl PitchAccum {
    /// An empty accumulator.
    pub const fn new() -> Self {
        Self { pending: 0 }
    }

    /// Adds one `ev_mouse` sample.
    ///
    /// `data3` is the event's vertical delta (positive = mouse moved forward), `sens` is
    /// `mouse_dmw_pitch_sens`, `invert` is `mouse_dmw_invert != 0` and `zoom_q16` is the zoom
    /// sensitivity scale in Q16 (65536 = no zoom).
    pub fn add(&mut self, data3: i32, sens: i32, invert: bool, zoom_q16: i64) {
        // Mouse forward = look up = negative MW2 pitch, unless inverted.
        let sign: i64 = if invert { 1 } else { -1 };
        let counts_x10 = i64::from(data3) * i64::from(sens + 5);
        self.pending += sign * counts_x10 * Q_NUM_PER_COUNT * zoom_q16;
    }

    /// Removes and returns the whole `pitch_q` units accumulated so far, at most `limit` in
    /// magnitude. The fraction (and anything above `limit`) stays in the accumulator.
    pub fn take_q(&mut self, limit: i32) -> i32 {
        let whole = (self.pending / PENDING_DENOM).clamp(-i64::from(limit), i64::from(limit));
        self.pending -= whole * PENDING_DENOM;
        whole as i32
    }

    /// Drops everything accumulated.
    pub fn clear(&mut self) {
        self.pending = 0;
    }
}

/// Zoom sensitivity scale `tan(zoom_hfov / 2) / tan(DOOM_HFOV / 2)` in Q16, or 1.0 when not
/// zoomed (sheet 05 section 5.3).
pub fn zoom_scale_q16(zoom_hfov_deg: Option<f32>) -> i64 {
    let Some(fov) = zoom_hfov_deg else {
        return ZOOM_ONE_Q16;
    };
    let fov = f64::from(fov).clamp(1.0, 170.0);
    let scale = (fov.to_radians() / 2.0).tan() / (f64::from(DOOM_HFOV).to_radians() / 2.0).tan();
    ((scale * ZOOM_ONE_Q16 as f64).round() as i64).clamp(1, 4 * ZOOM_ONE_Q16)
}

/// Applies a Q16 zoom scale to an integer mouse delta, carrying the fraction in `carry` so
/// small deltas are not lost.
pub fn scale_with_carry(delta: i32, zoom_q16: i64, carry: &mut i64) -> i32 {
    if zoom_q16 == ZOOM_ONE_Q16 {
        *carry = 0;
        return delta;
    }
    let total = i64::from(delta) * zoom_q16 + *carry;
    let out = total / ZOOM_ONE_Q16;
    *carry = total - out * ZOOM_ONE_Q16;
    out as i32
}

thread_local! {
    static PITCH: RefCell<PitchAccum> = const { RefCell::new(PitchAccum::new()) };
    static YAW_CARRY: Cell<i64> = const { Cell::new(0) };
}

/// Zoom scale for the console player from `PLAYER_EXT`.
fn console_zoom_q16() -> i64 {
    // SAFETY: single-threaded engine; `consoleplayer` is a valid index into `PLAYER_EXT`
    // (checked) and the read does not outlive this expression.
    let zoom = unsafe {
        let cp = consoleplayer;
        if cp < 0 || cp as usize >= MAXPLAYERS {
            None
        } else {
            PLAYER_EXT[cp as usize].zoom_hfov_deg
        }
    };
    zoom_scale_q16(zoom)
}

/// Accumulates mouse pitch from an `ev_mouse` event (`ev.data3`). Called from `G_Responder`
/// only when `hooks::mouse_look_active()` is `true`.
///
/// # Safety
/// `ev` must point to a valid `event_t`.
pub(crate) unsafe fn on_mouse_event(ev: *const event_t) {
    // SAFETY: the caller guarantees `ev` is valid.
    let data3 = unsafe { (*ev).data3 };
    if data3 == 0 {
        return;
    }
    // SAFETY: the cvars are plain ints written only at startup/config load on the main thread.
    let (sens, invert) = unsafe { (mc::mouse_dmw_pitch_sens, mc::mouse_dmw_invert != 0) };
    let zoom = console_zoom_q16();
    PITCH.with(|p| p.borrow_mut().add(data3, sens, invert, zoom));
}

/// Scales horizontal mouse input for the ADS zoom sensitivity; identity when not zoomed.
pub(crate) fn scale_yaw(mousex: c_int) -> c_int {
    let zoom = console_zoom_q16();
    YAW_CARRY.with(|c| {
        let mut carry = c.get();
        let out = scale_with_carry(mousex, zoom, &mut carry);
        c.set(carry);
        out
    })
}

/// Removes the pitch accumulated since the last call, in `pitch_q` units, clamped to `i16`.
/// Whole `pitch_q` units accumulated but not yet taken by a ticcmd (sheet 16).
pub(crate) fn peek_pending_q() -> i32 {
    PITCH.with(|p| {
        let mut copy = *p.borrow();
        copy.take_q(i32::from(i16::MAX))
    })
}

fn take_pending_q() -> i16 {
    PITCH.with(|p| p.borrow_mut().take_q(i32::from(i16::MAX))) as i16
}

/// Clears the pitch and yaw accumulators (level start).
pub(crate) fn clear_accumulators() {
    PITCH.with(|p| p.borrow_mut().clear());
    YAW_CARRY.with(|c| c.set(0));
}

// ---------------------------------------------------------------------------
// Extended buttons
// ---------------------------------------------------------------------------

/// Raw booleans that feed [`compose_ext_buttons`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonInputs {
    /// `cmd.buttons & BT_ATTACK`.
    pub attack: bool,
    /// Reload key down.
    pub reload: bool,
    /// ADS key or mouse button down.
    pub ads: bool,
    /// Melee key or mouse button down.
    pub melee: bool,
    /// Last-weapon key down.
    pub swap_last: bool,
    /// Sprint key (shared with hold-breath) down.
    pub sprint_key: bool,
}

/// Builds the `EXT_BT_*` word. While ADS is held the sprint key means hold-breath instead of
/// sprint (sheet 05 section 5.4).
pub fn compose_ext_buttons(b: &ButtonInputs) -> u16 {
    let mut out = EXT_BT_VALID;
    let mut set = |bit: u16, on: bool| {
        if on {
            out |= bit;
        }
    };
    set(EXT_BT_ATTACK, b.attack);
    set(EXT_BT_RELOAD, b.reload);
    set(EXT_BT_ADS, b.ads);
    set(EXT_BT_MELEE, b.melee);
    set(EXT_BT_SWAP_LAST, b.swap_last);
    set(EXT_BT_HOLD_BREATH, b.sprint_key && b.ads);
    set(EXT_BT_SPRINT, b.sprint_key && !b.ads);
    out
}

/// `true` when the key code is bound and held.
fn key(code: c_int) -> bool {
    game_key_down(code)
}

/// `true` when the mouse button is bound and held.
fn mouse(button: c_int) -> bool {
    mouse_button_down(button)
}

/// `G_BuildTiccmd`, end: builds this tic's [`ExtTiccmd`] and stores it in `EXT_TICCMDS`.
///
/// Built whatever the mode so a level that turns MW2 on never reads stale bits; consumers
/// ignore it in vanilla mode.
///
/// # Safety
/// `cmd` must point to the `TiccmdT` just built by `G_BuildTiccmd`.
pub(crate) unsafe fn on_build_ticcmd(cmd: *mut TiccmdT, maketic: c_int) {
    // SAFETY: the caller guarantees `cmd` is valid.
    let buttons = unsafe { (*cmd).buttons };
    // A special command (pause, save) reuses the low bits: BTS_PAUSE is bit 0 (sheet 08).
    let attack = buttons & BT_SPECIAL == 0 && buttons & BT_ATTACK != 0;
    // SAFETY: plain ints written at startup/config load on the main thread.
    let (reload, ads_key, ads_mouse, melee_key, melee_mouse, swap, sprint) = unsafe {
        (
            mc::key_dmw_reload,
            mc::key_dmw_ads,
            mc::mouseb_dmw_ads,
            mc::key_dmw_melee,
            mc::mouseb_dmw_melee,
            mc::key_dmw_swap,
            mc::key_dmw_sprint,
        )
    };

    if mode::level_active() && crate::dmw::log_once_first("input:binding-conflicts") {
        warn_binding_conflicts();
    }

    let buttons = compose_ext_buttons(&ButtonInputs {
        attack,
        reload: key(reload),
        ads: key(ads_key) || mouse(ads_mouse),
        melee: key(melee_key) || mouse(melee_mouse),
        swap_last: key(swap),
        sprint_key: key(sprint),
    });
    let ext = ExtTiccmd {
        tic: maketic,
        pitch_delta: take_pending_q(),
        buttons,
    };
    trace!(
        target: TARGET,
        "tic {maketic}: ext.buttons={:#06x} pitch_delta={}",
        ext.buttons,
        ext.pitch_delta
    );
    store_ext(maketic, ext);
}

/// `G_Ticker`, right after the net command copy: copies the extended command into
/// `PLAYER_EXT[i]` and, in MW2 mode, applies its pitch change.
pub(crate) fn on_ticcmd_copied(player_index: c_int) {
    if player_index < 0 || player_index as usize >= MAXPLAYERS {
        return;
    }
    let i = player_index as usize;
    // SAFETY: single-threaded engine; `i` is in range; the Doom globals are valid after
    // `D_DoomMain` has initialised them, which precedes any `G_Ticker`.
    unsafe {
        let ext = if player_index == consoleplayer {
            ext_for_gametic(gametic, ticdup)
        } else {
            ExtTiccmd::NEUTRAL
        };
        let pe = &mut PLAYER_EXT[i];
        pe.cmd = ext;

        let mo = players[i].mo as *const crate::doom::p_telept::mobj_t;
        if !mo.is_null() {
            pe.prev_yaw = (*mo).angle;
        }

        // With ticdup > 1 the same command runs several gametics; apply its pitch once.
        let first_of_group = ticdup <= 1 || gametic % ticdup == 0;
        if first_of_group && mode::level_active() {
            let dead = players[i].playerstate == PST_DEAD;
            apply_pitch_to(pe, ext.pitch_delta, dead);
        }
    }
}

/// `true` when mouse Y should drive pitch instead of forward/back movement.
pub(crate) fn mouse_look_active() -> bool {
    // SAFETY: plain int written at startup/config load on the main thread.
    mode::level_active() && unsafe { mc::mouse_dmw_look } != 0
}

/// `true` when MW2 mode forces always-run and ignores `mousebstrafe`/`mousebforward`.
pub(crate) fn force_run() -> bool {
    mode::level_active()
}

/// Copies the `iw4doom.toml` `[input]` bindings into the `m_controls` statics (the runtime
/// values read above). Called once by sheet 08's `on_engine_init`.
pub(crate) fn apply_bindings(cfg: &crate::dmw::config::InputCfg) {
    // SAFETY: plain ints on the single engine thread; no reference to them is live.
    unsafe {
        // Doom's own movement and use keys. An unbound key becomes code 0, which no device
        // produces, because `G_BuildTiccmd` indexes `gamekeydown` with these directly.
        let code = |k: crate::dmw::config::KeyBind| k.0.max(0);
        mc::key_up = code(cfg.key_up);
        mc::key_down = code(cfg.key_down);
        mc::key_strafeleft = code(cfg.key_strafeleft);
        mc::key_straferight = code(cfg.key_straferight);
        mc::key_use = code(cfg.key_use);
        mc::key_dmw_reload = cfg.key_dmw_reload.0;
        mc::key_dmw_ads = cfg.key_dmw_ads.0;
        mc::mouseb_dmw_ads = cfg.mouseb_dmw_ads;
        mc::key_dmw_sprint = cfg.key_dmw_sprint.0;
        mc::key_dmw_melee = cfg.key_dmw_melee.0;
        mc::mouseb_dmw_melee = cfg.mouseb_dmw_melee;
        mc::key_dmw_swap = cfg.key_dmw_swap.0;
        mc::mouse_dmw_look = c_int::from(cfg.mouse_dmw_look);
        mc::mouse_dmw_invert = c_int::from(cfg.mouse_dmw_invert);
        mc::mouse_dmw_pitch_sens = cfg.mouse_dmw_pitch_sens;
    }
    debug!(target: TARGET, "bindings from iw4doom.toml applied");
}

// ---------------------------------------------------------------------------
// Binding conflicts
// ---------------------------------------------------------------------------

/// A DOOMMW2 binding that has the same value as a vanilla one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingConflict {
    /// The DOOMMW2 variable.
    pub new_name: &'static str,
    /// The vanilla variable it collides with.
    pub vanilla_name: &'static str,
    /// The shared key code or mouse button.
    pub value: c_int,
}

/// Pure conflict search; negative values mean "unbound" and never conflict.
pub fn find_conflicts(
    new: &[(&'static str, c_int)],
    vanilla: &[(&'static str, c_int)],
) -> Vec<BindingConflict> {
    let mut out = Vec::new();
    for &(new_name, nv) in new {
        if nv < 0 {
            continue;
        }
        for &(vanilla_name, vv) in vanilla {
            if vv == nv {
                out.push(BindingConflict {
                    new_name,
                    vanilla_name,
                    value: nv,
                });
            }
        }
    }
    out
}

/// Logs a `warn!` per binding that equals a vanilla binding; both still fire (sheet 05 5.6).
/// The sprint key shares `key_speed` on purpose and is not reported.
fn warn_binding_conflicts() {
    // SAFETY: plain ints written at startup/config load on the main thread.
    let (new_keys, vanilla_keys, new_mouse, vanilla_mouse) = unsafe {
        (
            [
                ("key_dmw_reload", mc::key_dmw_reload),
                ("key_dmw_ads", mc::key_dmw_ads),
                ("key_dmw_melee", mc::key_dmw_melee),
                ("key_dmw_swap", mc::key_dmw_swap),
            ],
            [
                ("key_right", mc::key_right),
                ("key_left", mc::key_left),
                ("key_up", mc::key_up),
                ("key_down", mc::key_down),
                ("key_strafeleft", mc::key_strafeleft),
                ("key_straferight", mc::key_straferight),
                ("key_fire", mc::key_fire),
                ("key_use", mc::key_use),
                ("key_strafe", mc::key_strafe),
                ("key_speed", mc::key_speed),
                ("key_pause", mc::key_pause),
                ("key_weapon1", mc::key_weapon1),
                ("key_weapon2", mc::key_weapon2),
                ("key_weapon3", mc::key_weapon3),
                ("key_weapon4", mc::key_weapon4),
                ("key_weapon5", mc::key_weapon5),
                ("key_weapon6", mc::key_weapon6),
                ("key_weapon7", mc::key_weapon7),
                ("key_weapon8", mc::key_weapon8),
                ("key_prevweapon", mc::key_prevweapon),
                ("key_nextweapon", mc::key_nextweapon),
            ],
            [
                ("mouseb_dmw_ads", mc::mouseb_dmw_ads),
                ("mouseb_dmw_melee", mc::mouseb_dmw_melee),
            ],
            // mousebstrafe and mousebforward are masked in MW2 mode, so they cannot conflict.
            [
                ("mouseb_fire", mc::mousebfire),
                ("mouseb_use", mc::mousebuse),
                ("mouseb_backward", mc::mousebbackward),
                ("mouseb_strafeleft", mc::mousebstrafeleft),
                ("mouseb_straferight", mc::mousebstraferight),
                ("mouseb_prevweapon", mc::mousebprevweapon),
                ("mouseb_nextweapon", mc::mousebnextweapon),
            ],
        )
    };
    for c in find_conflicts(&new_keys, &vanilla_keys)
        .into_iter()
        .chain(find_conflicts(&new_mouse, &vanilla_mouse))
    {
        warn!(
            target: TARGET,
            "binding conflict: {} and {} are both {}; both will fire",
            c.new_name, c.vanilla_name, c.value
        );
    }
}

// ---------------------------------------------------------------------------
// Platform mouse sample accumulation
// ---------------------------------------------------------------------------

/// Largest per-sample delta handed to Doom (alt-tab and similar spikes are clamped).
pub const MAX_MOUSE_SAMPLE: i32 = 2000;

/// Mouse button bit for wheel-up pulses (`ev_mouse` `data1`).
pub const MOUSE_WHEEL_UP_BIT: u8 = 1 << 5;
/// Mouse button bit for wheel-down pulses.
pub const MOUSE_WHEEL_DOWN_BIT: u8 = 1 << 6;

/// Pixels of a high-resolution (touchpad) scroll that count as one wheel notch.
pub const WHEEL_PIXELS_PER_NOTCH: f64 = 50.0;

/// One integer mouse sample for the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseSample {
    /// Horizontal counts (positive = right).
    pub dx: i32,
    /// Vertical counts (positive = down, screen convention).
    pub dy: i32,
    /// Doom mouse button bits: left 0, right 1, middle 2, back 3, forward 4, wheel up 5,
    /// wheel down 6.
    pub buttons: u8,
}

/// Raw mouse state between winit events and the engine (sheet 05 section 5.1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MouseAccum {
    dx: f64,
    dy: f64,
    /// Held buttons, bits 0..=4.
    buttons: u8,
    /// Pending wheel notches (positive = up).
    wheel: i32,
    /// Wheel bit sent in the previous sample that still needs its release sample.
    wheel_release: u8,
    /// Buttons value of the previously delivered sample.
    last_sent: u8,
}

impl MouseAccum {
    /// An idle accumulator.
    pub const fn new() -> Self {
        Self {
            dx: 0.0,
            dy: 0.0,
            buttons: 0,
            wheel: 0,
            wheel_release: 0,
            last_sent: 0,
        }
    }

    /// Adds raw motion (only call while grabbed).
    pub fn add_motion(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.dx += dx;
            self.dy += dy;
        }
    }

    /// Sets or clears a held button bit (`0..=4`).
    pub fn set_button(&mut self, bit: u8, down: bool) {
        if bit > 4 {
            return;
        }
        if down {
            self.buttons |= 1 << bit;
        } else {
            self.buttons &= !(1 << bit);
        }
    }

    /// Adds wheel notches (positive = up).
    pub fn add_wheel(&mut self, notches: i32) {
        self.wheel = self.wheel.saturating_add(notches);
    }

    /// Releases every button and drops pending motion and wheel input (focus loss, menus).
    /// The next sample reports the release so Doom sees the button go up.
    pub fn release_all(&mut self) {
        self.buttons = 0;
        self.dx = 0.0;
        self.dy = 0.0;
        self.wheel = 0;
        self.wheel_release = 0;
    }

    /// Returns the next sample, or `None` when nothing changed: integer counts (fractions stay
    /// in the accumulator), the held buttons, and at most one wheel pulse (the pulse bit is
    /// set in one sample and cleared in the next).
    pub fn take(&mut self) -> Option<MouseSample> {
        let max = f64::from(MAX_MOUSE_SAMPLE);
        let ix = self.dx.trunc().clamp(-max, max);
        let iy = self.dy.trunc().clamp(-max, max);
        // A clamped spike is dropped rather than replayed over many frames.
        self.dx = if self.dx.abs() > max {
            0.0
        } else {
            self.dx - ix
        };
        self.dy = if self.dy.abs() > max {
            0.0
        } else {
            self.dy - iy
        };

        let wheel_bits = if self.wheel_release != 0 {
            self.wheel_release = 0;
            0
        } else if self.wheel != 0 {
            let bit = if self.wheel > 0 {
                MOUSE_WHEEL_UP_BIT
            } else {
                MOUSE_WHEEL_DOWN_BIT
            };
            self.wheel -= self.wheel.signum();
            self.wheel_release = bit;
            bit
        } else {
            0
        };

        let buttons = self.buttons | wheel_bits;
        let (dx, dy) = (ix as i32, iy as i32);
        if dx == 0 && dy == 0 && buttons == self.last_sent {
            return None;
        }
        self.last_sent = buttons;
        Some(MouseSample { dx, dy, buttons })
    }
}

// ---------------------------------------------------------------------------
// Capture release request
// ---------------------------------------------------------------------------

static RELEASE_REQUESTED: AtomicBool = AtomicBool::new(false);

/// `M_Responder` menu open and window focus loss: asks the platform layer to release mouse
/// capture and clear held buttons. The platform polls [`take_release_request`].
pub(crate) fn release_mouse() {
    debug!(target: TARGET, "mouse release requested");
    RELEASE_REQUESTED.store(true, Ordering::Relaxed);
}

/// Returns `true` once per [`release_mouse`] request (platform side).
pub fn take_release_request() -> bool {
    RELEASE_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Clamped combination of grab conditions (sheet 05 section 5.1):
/// `focused && gamestate == GS_LEVEL && menuactive == 0 && paused == 0`.
pub fn want_grab(focused: bool, gamestate_is_level: bool, menu_active: bool, paused: bool) -> bool {
    focused && gamestate_is_level && !menu_active && !paused
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: i64 = ZOOM_ONE_Q16;

    /// Small deterministic generator for the drift test (no `rand`).
    fn lcg(state: &mut u64) -> i32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 33) as i32 % 401) - 200
    }

    /// T5.1: many samples sum to exactly the same total as one combined sample.
    #[test]
    fn pitch_accumulation_has_no_drift() {
        for sens in [0, 3, 5, 9] {
            let mut seed = 12345u64;
            let mut many = PitchAccum::new();
            let mut total = 0i64;
            let mut taken = 0i64;
            for n in 0..1000 {
                let s = lcg(&mut seed);
                total += i64::from(s);
                many.add(s, sens, false, ONE);
                if n % 7 == 0 {
                    taken += i64::from(many.take_q(i32::MAX));
                }
            }
            taken += i64::from(many.take_q(i32::MAX));
            let mut one = PitchAccum::new();
            one.add(total as i32, sens, false, ONE);
            let combined = i64::from(one.take_q(i32::MAX));
            // Whole units can differ by at most the single leftover fraction.
            assert!(
                (taken - combined).abs() <= 1,
                "sens {sens}: many={taken} combined={combined}"
            );
            // Exact: whole units taken plus the fraction left over are identical.
            assert_eq!(
                taken * PENDING_DENOM + many.pending,
                combined * PENDING_DENOM + one.pending,
                "sens {sens}"
            );
        }
    }

    /// T5.1: default sensitivity gives 5.625 units (0.044 degrees) per count, forward = up.
    #[test]
    fn pitch_matches_the_formula_and_direction() {
        let mut a = PitchAccum::new();
        a.add(-100, 5, false, ONE);
        // data3 = -100 (mouse moved back) looks down: +100 * 5.625 = 562.5 -> 562.
        assert_eq!(a.take_q(i32::MAX), 562);
        // The half unit stays; another identical sample reaches 1125 in total.
        a.add(-100, 5, false, ONE);
        assert_eq!(a.take_q(i32::MAX), 563);
        let mut up = PitchAccum::new();
        up.add(100, 5, false, ONE);
        assert_eq!(up.take_q(i32::MAX), -562);
    }

    /// T5.1: the invert flag flips the sign.
    #[test]
    fn invert_flips_the_sign() {
        let mut plain = PitchAccum::new();
        let mut inv = PitchAccum::new();
        plain.add(64, 5, false, ONE);
        inv.add(64, 5, true, ONE);
        assert_eq!(plain.take_q(i32::MAX), -inv.take_q(i32::MAX));
    }

    /// T5.1: a zoom scale of 0.5 halves the rate.
    #[test]
    fn zoom_scale_halves_the_rate() {
        let mut full = PitchAccum::new();
        let mut half = PitchAccum::new();
        full.add(-160, 5, false, ONE);
        half.add(-160, 5, false, ONE / 2);
        assert_eq!(full.take_q(i32::MAX), 900);
        assert_eq!(half.take_q(i32::MAX), 450);
    }

    #[test]
    fn zoom_scale_follows_the_tangent_ratio() {
        assert_eq!(zoom_scale_q16(None), ONE);
        assert_eq!(zoom_scale_q16(Some(90.0)), ONE);
        // tan(30 deg) / tan(45 deg) = 0.57735
        let s = zoom_scale_q16(Some(60.0)) as f64 / ONE as f64;
        assert!((s - 0.57735).abs() < 1e-4, "{s}");
        let narrow = zoom_scale_q16(Some(10.0));
        assert!(narrow > 0 && narrow < ONE / 8);
    }

    #[test]
    fn take_is_limited_and_keeps_the_excess() {
        let mut a = PitchAccum::new();
        a.add(-10_000, 5, false, ONE); // +56250 units
        assert_eq!(a.take_q(i32::from(i16::MAX)), i32::from(i16::MAX));
        assert_eq!(a.take_q(i32::from(i16::MAX)), 56250 - i32::from(i16::MAX));
        assert_eq!(a.take_q(i32::from(i16::MAX)), 0);
    }

    #[test]
    fn scale_with_carry_is_lossless_over_time() {
        let mut carry = 0;
        let half = ONE / 2;
        let out: i32 = (0..10).map(|_| scale_with_carry(1, half, &mut carry)).sum();
        assert_eq!(out, 5);
        let mut carry = 5;
        assert_eq!(scale_with_carry(7, ONE, &mut carry), 7);
        assert_eq!(carry, 0);
    }

    /// T5.6 (logic half): Shift alone is sprint; Shift with ADS is hold-breath, not sprint.
    #[test]
    fn sprint_key_becomes_hold_breath_while_ads() {
        let base = ButtonInputs::default();
        let none = compose_ext_buttons(&base);
        assert_eq!(none, EXT_BT_VALID);

        let sprint = compose_ext_buttons(&ButtonInputs {
            sprint_key: true,
            ..base
        });
        assert_eq!(sprint, EXT_BT_VALID | EXT_BT_SPRINT);

        let breath = compose_ext_buttons(&ButtonInputs {
            sprint_key: true,
            ads: true,
            ..base
        });
        assert_eq!(breath, EXT_BT_VALID | EXT_BT_ADS | EXT_BT_HOLD_BREATH);
        assert_eq!(breath & EXT_BT_SPRINT, 0);

        let all = compose_ext_buttons(&ButtonInputs {
            attack: true,
            reload: true,
            ads: true,
            melee: true,
            swap_last: true,
            sprint_key: false,
        });
        assert_eq!(
            all,
            EXT_BT_VALID
                | EXT_BT_ATTACK
                | EXT_BT_RELOAD
                | EXT_BT_ADS
                | EXT_BT_MELEE
                | EXT_BT_SWAP_LAST
        );
    }

    #[test]
    fn conflicts_ignore_unbound_and_report_matches() {
        let new = [
            ("key_dmw_reload", 114),
            ("key_dmw_ads", -1),
            ("key_dmw_swap", 113),
        ];
        let vanilla = [("key_use", 32), ("key_x", 114), ("key_unbound", -1)];
        let found = find_conflicts(&new, &vanilla);
        assert_eq!(
            found,
            vec![BindingConflict {
                new_name: "key_dmw_reload",
                vanilla_name: "key_x",
                value: 114
            }]
        );
    }

    #[test]
    fn mouse_accum_keeps_fractions_and_reports_integers() {
        let mut m = MouseAccum::new();
        assert_eq!(m.take(), None);
        m.add_motion(0.6, -0.6);
        assert_eq!(m.take(), None);
        m.add_motion(0.6, -0.6);
        assert_eq!(
            m.take(),
            Some(MouseSample {
                dx: 1,
                dy: -1,
                buttons: 0
            })
        );
        m.add_motion(2.5, 0.0);
        assert_eq!(m.take().map(|s| s.dx), Some(2));
        m.add_motion(0.5, 0.0);
        assert_eq!(m.take().map(|s| s.dx), Some(1));
    }

    #[test]
    fn mouse_accum_reports_button_changes_once() {
        let mut m = MouseAccum::new();
        m.set_button(0, true);
        assert_eq!(m.take().map(|s| s.buttons), Some(1));
        assert_eq!(m.take(), None);
        m.set_button(1, true);
        assert_eq!(m.take().map(|s| s.buttons), Some(3));
        m.set_button(0, false);
        m.set_button(1, false);
        assert_eq!(m.take().map(|s| s.buttons), Some(0));
        assert_eq!(m.take(), None);
        m.set_button(7, true);
        assert_eq!(m.take(), None);
    }

    #[test]
    fn wheel_is_a_press_then_a_release() {
        let mut m = MouseAccum::new();
        m.add_wheel(1);
        assert_eq!(m.take().map(|s| s.buttons), Some(MOUSE_WHEEL_UP_BIT));
        assert_eq!(m.take().map(|s| s.buttons), Some(0));
        assert_eq!(m.take(), None);
        m.add_wheel(-2);
        assert_eq!(m.take().map(|s| s.buttons), Some(MOUSE_WHEEL_DOWN_BIT));
        assert_eq!(m.take().map(|s| s.buttons), Some(0));
        assert_eq!(m.take().map(|s| s.buttons), Some(MOUSE_WHEEL_DOWN_BIT));
        assert_eq!(m.take().map(|s| s.buttons), Some(0));
        assert_eq!(m.take(), None);
    }

    /// Focus loss: held fire is reported as released so Doom does not keep firing.
    #[test]
    fn release_all_reports_the_release() {
        let mut m = MouseAccum::new();
        m.set_button(0, true);
        m.add_motion(5.0, 5.0);
        let _ = m.take();
        m.set_button(0, true);
        m.release_all();
        assert_eq!(m.take().map(|s| s.buttons), Some(0));
        assert_eq!(m.take(), None);
    }

    #[test]
    fn huge_deltas_are_clamped_and_dropped() {
        let mut m = MouseAccum::new();
        m.add_motion(50_000.0, -50_000.0);
        let s = m.take();
        assert_eq!(
            s,
            Some(MouseSample {
                dx: MAX_MOUSE_SAMPLE,
                dy: -MAX_MOUSE_SAMPLE,
                buttons: 0
            })
        );
        assert_eq!(m.take(), None);
        m.add_motion(f64::NAN, 1.0);
        assert_eq!(m.take(), None);
    }

    #[test]
    fn grab_policy_needs_every_condition() {
        assert!(want_grab(true, true, false, false));
        assert!(!want_grab(false, true, false, false));
        assert!(!want_grab(true, false, false, false));
        assert!(!want_grab(true, true, true, false));
        assert!(!want_grab(true, true, false, true));
    }

    #[test]
    fn release_request_is_taken_once() {
        let _ = take_release_request();
        release_mouse();
        assert!(take_release_request());
        assert!(!take_release_request());
    }

    #[test]
    fn scale_yaw_is_identity_without_zoom() {
        clear_accumulators();
        // No Doom state is initialised in unit tests: `consoleplayer` is 0 and PLAYER_EXT has
        // no zoom, so the scale is 1.
        assert_eq!(scale_yaw(-37), -37);
        assert_eq!(scale_yaw(11), 11);
    }
}
