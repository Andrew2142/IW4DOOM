// SPDX-License-Identifier: GPL-3.0-or-later
//! Uncapped frame rate (sheet 16): the game still runs at 35 tics per second, but a frame is
//! drawn every time the main loop comes round (paced by vsync and `[render] max_fps`).
//!
//! - [`on_ticker_start`] (start of a `P_Ticker` that runs the world) snapshots every map
//!   object's position and angle, every sector's floor and ceiling and the players' `viewz`.
//! - [`begin_frame`] / [`end_frame`] wrap `R_RenderPlayerView` in `D_Display`: positions are
//!   swapped for values blended between the snapshot and the current tic by how far real time
//!   has moved into the next tic, then restored exactly. The simulation never sees them.
//! - The local player's view angle and pitch also get the mouse movement that has not reached
//!   a tic yet (built-but-unrun ticcmds plus the live accumulators), so mouse look responds
//!   every frame instead of every 28 ms.
//!
//! Off unless the window front end calls [`set_allowed`] and in net games; with
//! `-timedemo`/`-singletics` every frame is drawn at the tic itself. While off nothing here touches the engine, so headless tests,
//! demos and vanilla parity are unchanged.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_int, c_void};

use crate::doom::c_ffi::{mobj_t, sector_t, BACKUPTICS};
use crate::doom::d_player::{PlayerT, MAXPLAYERS};
use crate::doom::m_fixed::{fixed_t, FRACUNIT};

/// A thing that moved further than this in one tic teleported: draw it where it is.
const SNAP_XY: i64 = 64 * FRACUNIT as i64;
/// Vertical equivalent (fast falls and lifts stay smooth, teleports between heights snap).
const SNAP_Z: i64 = 128 * FRACUNIT as i64;

#[derive(Clone, Copy, Debug)]
struct ThingPos {
    type_: c_int,
    x: fixed_t,
    y: fixed_t,
    z: fixed_t,
    angle: u32,
}

/// World state at the start of the last tic that ran the world.
#[derive(Default)]
struct Snapshot {
    /// `gametic` while that tic ran; blending happens only while it is the latest tic.
    gametic: c_int,
    valid: bool,
    things: HashMap<usize, ThingPos>,
    sectors: Vec<(fixed_t, fixed_t)>,
    viewz: [fixed_t; MAXPLAYERS],
}

/// Values overwritten by [`begin_frame`], put back by [`end_frame`].
#[derive(Default)]
pub struct Restore {
    things: Vec<(*mut mobj_t, fixed_t, fixed_t, fixed_t, u32)>,
    sectors: Vec<(usize, fixed_t, fixed_t)>,
    player: Option<(*mut PlayerT, fixed_t)>,
}

thread_local! {
    static ALLOWED: Cell<bool> = const { Cell::new(false) };
    static SNAP: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    /// `(tic, angleturn, pitch_delta)` of each ticcmd built, indexed by `maketic % BACKUPTICS`.
    static BUILT: RefCell<[(c_int, i16, i16); BACKUPTICS]> =
        const { RefCell::new([(-1, 0, 0); BACKUPTICS]) };
    /// Pending pitch added to the rendered pitch this frame (`pitch_q` units).
    static FRAME_PITCH_Q: Cell<i32> = const { Cell::new(0) };
    /// Things and sectors blended in the last frame (diagnostics and tests).
    static LAST_BLENDED: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// Things and sectors whose drawn position was blended in the last frame.
pub fn last_blended() -> (usize, usize) {
    LAST_BLENDED.with(Cell::get)
}

/// The window front end turns uncapped rendering on (`[render] uncapped`).
pub fn set_allowed(on: bool) {
    ALLOWED.with(|a| a.set(on));
    log::info!(target: "dmw::interp", "uncapped frame rate {}", if on { "on" } else { "off" });
}

/// Whether frames are drawn between tics right now.
pub fn active() -> bool {
    // SAFETY: plain int reads on the engine thread.
    ALLOWED.with(Cell::get) && unsafe { crate::doom::g_game::netgame == 0 }
}

/// `-timedemo` / `-singletics`: one tic per frame, so there is nothing between tics to draw.
fn single_tics() -> bool {
    // SAFETY: plain int read on the engine thread.
    unsafe { crate::doom::d_loop::singletics != 0 }
}

/// `TryRunTics`: `true` = no tic is due, return and draw a frame instead of waiting.
pub(crate) fn return_early(availabletics: c_int) -> bool {
    availabletics < 1 && active() && !single_tics()
}

/// `P_Ticker`, after the pause checks: remember where everything is before the world moves.
pub(crate) fn on_ticker_start() {
    if !active() {
        return;
    }
    SNAP.with(|s| {
        let mut s = s.borrow_mut();
        s.things.clear();
        // SAFETY: engine thread, between tics; the thinker list and sectors are live.
        unsafe {
            for_each_mobj(|mo| {
                let m = &*mo;
                s.things.insert(
                    mo as usize,
                    ThingPos {
                        type_: m.type_,
                        x: m.x,
                        y: m.y,
                        z: m.z,
                        angle: m.angle,
                    },
                );
            });
            s.sectors.clear();
            for sec in sectors() {
                s.sectors.push((sec.floorheight, sec.ceilingheight));
            }
            let players = (&raw const crate::doom::g_game::players).cast::<PlayerT>();
            for (i, viewz) in s.viewz.iter_mut().enumerate() {
                *viewz = (*players.add(i)).viewz;
            }
            s.gametic = crate::doom::d_loop::gametic;
        }
        s.valid = true;
    });
}

/// Level start: forget the previous level.
pub(crate) fn on_level_start() {
    SNAP.with(|s| s.borrow_mut().valid = false);
    BUILT.with(|b| *b.borrow_mut() = [(-1, 0, 0); BACKUPTICS]);
}

/// `G_BuildTiccmd`, end: remember the turn this command will apply when it runs.
pub(crate) fn on_build_ticcmd(maketic: c_int, angleturn: i16, pitch_delta: i16) {
    BUILT.with(|b| {
        b.borrow_mut()[maketic.rem_euclid(BACKUPTICS as c_int) as usize] =
            (maketic, angleturn, pitch_delta)
    });
}

/// Pitch (`pitch_q` units) not yet applied by a tic, added to the rendered pitch this frame.
pub(crate) fn frame_pitch_q() -> i32 {
    FRAME_PITCH_Q.with(Cell::get)
}

/// How far real time is into the tic after `gametic`, in 16.16 (0..FRACUNIT).
fn tic_fraction() -> fixed_t {
    let ms = i64::from(crate::doom::i_timer::I_GetTimeMS());
    let into = (ms * 35).rem_euclid(1000);
    (into * i64::from(FRACUNIT) / 1000) as fixed_t
}

fn lerp(old: fixed_t, new: fixed_t, frac: fixed_t) -> fixed_t {
    old.wrapping_add((((i64::from(new) - i64::from(old)) * i64::from(frac)) >> 16) as fixed_t)
}

fn lerp_angle(old: u32, new: u32, frac: fixed_t) -> u32 {
    let delta = new.wrapping_sub(old) as i32;
    old.wrapping_add(((i64::from(delta) * i64::from(frac)) >> 16) as i32 as u32)
}

/// Yaw (BAM) and pitch (`pitch_q`) of mouse movement that no tic has applied yet.
fn pending_turn() -> (u32, i32) {
    // SAFETY: plain int read on the engine thread.
    let gametic = unsafe { crate::doom::d_loop::gametic };
    let maketic = crate::doom::d_loop::maketic();
    let (mut turn, mut pitch) = BUILT.with(|b| {
        let b = b.borrow();
        let mut turn = 0i32;
        let mut pitch = 0i32;
        for tic in gametic..maketic.min(gametic + BACKUPTICS as c_int) {
            let (t, a, p) = b[tic.rem_euclid(BACKUPTICS as c_int) as usize];
            if t == tic {
                turn += i32::from(a);
                pitch += i32::from(p);
            }
        }
        (turn, pitch)
    });
    turn -= crate::doom::g_game::pending_mousex() * 8;
    pitch += super::input::peek_pending_q();
    (((turn as u32) << 16), pitch)
}

/// Swaps blended positions in before `R_RenderPlayerView`. Pair with [`end_frame`].
pub(crate) fn begin_frame() -> Restore {
    let mut restore = Restore::default();
    FRAME_PITCH_Q.with(|p| p.set(0));
    if !active() {
        return restore;
    }
    // SAFETY: engine thread inside D_Display; every pointer comes from the live thinker list,
    // sector array or players array and is restored by `end_frame` before anything else runs.
    unsafe {
        let gametic = crate::doom::d_loop::gametic;
        let frac = if single_tics() {
            FRACUNIT
        } else {
            tic_fraction()
        };
        SNAP.with(|s| {
            let s = s.borrow();
            if !s.valid || s.gametic + 1 != gametic {
                return;
            }
            for_each_mobj(|mo| {
                let m = &mut *mo;
                let Some(old) = s.things.get(&(mo as usize)) else {
                    return;
                };
                let jumped = old.type_ != m.type_
                    || (i64::from(m.x) - i64::from(old.x)).abs() > SNAP_XY
                    || (i64::from(m.y) - i64::from(old.y)).abs() > SNAP_XY
                    || (i64::from(m.z) - i64::from(old.z)).abs() > SNAP_Z;
                if jumped {
                    return;
                }
                restore.things.push((mo, m.x, m.y, m.z, m.angle));
                m.x = lerp(old.x, m.x, frac);
                m.y = lerp(old.y, m.y, frac);
                m.z = lerp(old.z, m.z, frac);
                m.angle = lerp_angle(old.angle, m.angle, frac);
            });
            let secs = sectors();
            if secs.len() == s.sectors.len() {
                for (i, (sec, &(f, c))) in secs.iter_mut().zip(&s.sectors).enumerate() {
                    if sec.floorheight != f || sec.ceilingheight != c {
                        restore
                            .sectors
                            .push((i, sec.floorheight, sec.ceilingheight));
                        sec.floorheight = lerp(f, sec.floorheight, frac);
                        sec.ceilingheight = lerp(c, sec.ceilingheight, frac);
                    }
                }
            }
            let dp = crate::doom::g_game::displayplayer as usize;
            if dp < MAXPLAYERS {
                let p = (&raw mut crate::doom::g_game::players)
                    .cast::<PlayerT>()
                    .add(dp);
                restore.player = Some((p, (*p).viewz));
                (*p).viewz = lerp(s.viewz[dp], (*p).viewz, frac);
            }
        });

        // Mouse look not yet applied by a tic: the local, living player's view turns now.
        let cp = crate::doom::g_game::consoleplayer;
        let live = cp == crate::doom::g_game::displayplayer
            && crate::doom::g_game::demoplayback == 0
            && (cp as usize) < MAXPLAYERS;
        if live {
            let p = &*(&raw const crate::doom::g_game::players)
                .cast::<PlayerT>()
                .add(cp as usize);
            if p.playerstate == 0 && !p.mo.is_null() {
                let mo = p.mo.cast::<mobj_t>();
                let (yaw, pitch) = pending_turn();
                // The angle already blended above is the current one for the player (the
                // player's own turn is not blended: it is replaced by current + pending).
                let base = restore
                    .things
                    .iter()
                    .find(|t| t.0 == mo)
                    .map_or((*mo).angle, |t| t.4);
                if !restore.things.iter().any(|t| t.0 == mo) {
                    restore
                        .things
                        .push((mo, (*mo).x, (*mo).y, (*mo).z, (*mo).angle));
                }
                (*mo).angle = base.wrapping_add(yaw);
                FRAME_PITCH_Q.with(|f| f.set(pitch));
            }
        }
    }
    restore
}

/// Puts back everything [`begin_frame`] changed.
pub(crate) fn end_frame(restore: Restore) {
    LAST_BLENDED.with(|b| b.set((restore.things.len(), restore.sectors.len())));
    // SAFETY: the same live pointers `begin_frame` wrote, still on the engine thread and
    // before any tic can run.
    unsafe {
        for (mo, x, y, z, angle) in restore.things.into_iter().rev() {
            (*mo).x = x;
            (*mo).y = y;
            (*mo).z = z;
            (*mo).angle = angle;
        }
        let secs = sectors();
        for (i, f, c) in restore.sectors {
            if let Some(sec) = secs.get_mut(i) {
                sec.floorheight = f;
                sec.ceilingheight = c;
            }
        }
        if let Some((p, viewz)) = restore.player {
            (*p).viewz = viewz;
        }
    }
    FRAME_PITCH_Q.with(|p| p.set(0));
}

/// Calls `f` for every map object in the thinker list.
unsafe fn for_each_mobj(mut f: impl FnMut(*mut mobj_t)) {
    use crate::doom::p_tick::{thinker_t, thinkercap};
    // SAFETY: the caller is on the engine thread with a level loaded; the list is circular.
    unsafe {
        let mobj_fn = crate::doom::p_mobj::P_MobjThinker as *const c_void as usize;
        let head = &raw mut thinkercap;
        let mut th: *mut thinker_t = (*head).next;
        while !th.is_null() && th != head {
            let func = (*th).function.acv.map_or(0, |f| f as usize);
            if func == mobj_fn {
                f(th.cast::<mobj_t>());
            }
            th = (*th).next;
        }
    }
}

/// The current level's sectors (empty before a level is loaded).
unsafe fn sectors<'a>() -> &'a mut [sector_t] {
    // SAFETY: `sectors` holds `numsectors` entries for the loaded level (zone memory).
    unsafe {
        let base = crate::doom::p_setup::sectors;
        let n = usize::try_from(crate::doom::p_setup::numsectors).unwrap_or(0);
        if base.is_null() || n == 0 {
            &mut []
        } else {
            std::slice::from_raw_parts_mut(base, n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_is_exact_at_the_ends_and_halfway() {
        assert_eq!(lerp(0, 100 << 16, 0), 0);
        assert_eq!(lerp(0, 100 << 16, FRACUNIT - 1) >> 16, 99);
        assert_eq!(lerp(-10 << 16, 10 << 16, FRACUNIT / 2), 0);
    }

    #[test]
    fn angle_lerp_takes_the_short_way_round() {
        // From 350 degrees to 10 degrees goes through 0, not 180.
        let deg = |d: f64| (d / 360.0 * 4294967296.0) as u64 as u32;
        let mid = lerp_angle(deg(350.0), deg(10.0), FRACUNIT / 2);
        let back = f64::from(mid) / 4294967296.0 * 360.0;
        assert!(!(1.0..359.0).contains(&back), "{back}");
    }
}
