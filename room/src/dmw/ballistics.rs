// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/sim/src/bullet.rs (`bullet_damage_at_distance`), Copyright 2026 vladtrc, licensed under
// the Apache License 2.0.
// Modifications for DOOMMW2: none beyond the module path and doc comments.
//
// The traverse callback `dmw_shoot_traverse` is adapted from room's GPL `PTR_ShootTraverse`
// (room/src/doom/p_map.rs), itself a port of Doom's `p_map.c`.
//! Ballistics and damage (sheet 09): true 3D hitscan, MW2 damage falloff and hit locations,
//! melee, and MW2-tuned rockets.
//!
//! Sheet 08 dispatches `SimEvent::Shot` / `SimEvent::Melee` here:
//!
//! - [`fire`] resolves a [`ShotRequest`] (hitscan pellets, or a rocket when
//!   `ShotRequest::projectile` is set);
//! - [`melee`] resolves a [`MeleeRequest`];
//! - [`on_explode`] / [`on_remove_mobj`] are the bodies of the `A_Explode` and `P_RemoveMobj`
//!   hooks (`hooks.rs` delegates to them).
//!
//! The pack facts and the `[balance]` numbers are not reachable from `DmwState` yet (sheet 08),
//! so sheet 08 hands them over once per level with [`configure`] and calls [`reset_level`] at
//! level start. All state is `thread_local` (sheet 01 section 5.4); the traverse state never
//! touches the `static mut`s of `p_map.rs`, so the vanilla `P_LineAttack` paths are untouched.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{c_int, c_uint, c_void};
use std::sync::Arc;

use dmw_format::consts::{DOOM_PER_MW2, MW2_PER_DOOM};
use dmw_format::convert::degrees_to_bam;
use dmw_format::{DmwWeaponId, Pack};
use dmw_weapons::{MeleeRequest, ShotRequest};
use weapon_iw4::{
    fire_weapon_kind, location_damage_scale, FireWeaponKind, WeaponCombatFacts, BULLET_MAX_RANGE,
};

use crate::dmw::config::BalanceCfg;
use crate::dmw::hud::{self, HitFeedback};
use crate::doom::c_ffi::{
    intercept_t, line_t, mobj_t as CMobj, sector_t, MAPBLOCKSHIFT as BLOCKSHIFT,
};
use crate::doom::d_player::PlayerT;
use crate::doom::info::{
    MF_NOBLOOD, MF_SHOOTABLE, MT_BARREL, MT_BOSSBRAIN, MT_CYBORG, MT_HEAD, MT_KEEN, MT_PAIN,
    MT_ROCKET, MT_SKULL, MT_SPIDER,
};
use crate::doom::m_fixed::{fixed_t, FixedDiv, FixedMul, FRACUNIT};
use crate::doom::p_enemy::P_NoiseAlert;
use crate::doom::p_inter::P_DamageMobj;
use crate::doom::p_maputl::{
    openbottom, opentop, trace, P_BlockThingsIterator, P_LineOpening, P_PathTraverse, PT_ADDLINES,
    PT_ADDTHINGS,
};
use crate::doom::p_mobj::{P_CheckMissileSpawn, P_SpawnBlood, P_SpawnMobj, P_SpawnPuff};
use crate::doom::p_setup::{bmaporgx, bmaporgy};
use crate::doom::p_sight::P_CheckSight;
use crate::doom::p_spec::P_ShootSpecialLine;
use crate::doom::p_telept::mobj_t as Mobj;
use crate::doom::p_tick::leveltime;
use crate::doom::r_sky::skyflatnum;
use crate::doom::s_sound::S_StartSound;
use crate::doom::sounds::Sfx;

const LOG: &str = "dmw::ballistics";

/// Doom `MELEERANGE` in map units.
const MELEERANGE_UNITS: f64 = 64.0;
/// Longest hitscan reach in Doom units (`BULLET_MAX_RANGE`).
const MAX_RANGE_UNITS: f64 = BULLET_MAX_RANGE as f64;
/// Steepest aim slope (rise over horizontal run) the trace accepts.
const MAX_SLOPE: f64 = 8.0;
/// `MAXRADIUS` (largest thing radius) in fixed units, widening the blockmap window.
const MAXRADIUS_FIXED: i32 = 32 * FRACUNIT;

// ---------------------------------------------------------------------------
// Pure helpers (unit tested without the engine)
// ---------------------------------------------------------------------------

/// MW2 damage at `dist` inches: `damage` up to `max_damage_range`, linearly down to
/// `min_damage` at `min_damage_range` (`min_damage == 0` means "no falloff").
///
/// Port of iw4L `sim/src/bullet.rs:3`.
pub fn bullet_damage_at_distance(facts: &WeaponCombatFacts, dist: f32) -> i32 {
    let base = facts.damage;
    let min = if facts.min_damage > 0 {
        facts.min_damage
    } else {
        base
    };
    let mut damage = base;
    let range = facts.min_damage_range - facts.max_damage_range;
    if base != min && range != 0.0 && facts.max_damage_range <= dist {
        if facts.min_damage_range <= dist {
            damage = min;
        } else {
            let lerp = ((dist - facts.max_damage_range) / range).clamp(0.0, 1.0);
            damage = ((base as f32) * (1.0 - lerp) + (min as f32) * lerp) as i32;
        }
    }
    damage.max(0)
}

/// Final damage of one pellet (sheet 09 section 5.3): falloff, hit-location multiplier and the
/// balance scale, rounded, at least 1 when the base damage is positive.
pub fn pellet_damage(facts: &WeaponCombatFacts, dist3_mw2: f32, hitloc: u8, scale: f32) -> i32 {
    let base = bullet_damage_at_distance(facts, dist3_mw2);
    let mult = location_damage_scale(&facts.location_damage, hitloc);
    let d = (base as f32 * mult * scale).round() as i32;
    if base > 0 {
        d.max(1)
    } else {
        0
    }
}

/// Hit-location index (`weapon_iw4::HITLOC_NAMES`) from the relative hit height `h` (0 = feet,
/// 1 = top) on a monster of type `mobjtype` (sheet 09 section 5.4).
pub fn hit_location(h: f32, mobjtype: c_int, headshot_enabled: bool) -> u8 {
    const TORSO_UPPER: u8 = 4;
    if !headshot_enabled || !has_head(mobjtype) {
        return TORSO_UPPER;
    }
    let h = h.clamp(0.0, 1.0);
    if h >= 0.85 {
        2 // head
    } else if h >= 0.70 {
        3 // neck
    } else if h >= 0.45 {
        TORSO_UPPER
    } else if h >= 0.30 {
        5 // torso_lower
    } else if h >= 0.12 {
        12 // right_leg_upper
    } else {
        14 // right_leg_lower
    }
}

/// `false` for the types without an anatomical head (sheet 09 section 5.4).
pub fn has_head(mobjtype: c_int) -> bool {
    !matches!(
        mobjtype,
        MT_HEAD | MT_SKULL | MT_PAIN | MT_SPIDER | MT_BARREL | MT_KEEN | MT_BOSSBRAIN
    )
}

/// `true` for the hit locations that count as a headshot for the HUD (helmet, head).
pub fn is_headshot(hitloc: u8) -> bool {
    matches!(hitloc, 1 | 2)
}

/// Unit direction to Doom `(angle_t, slope)` (sheet 09 section 5.1). The slope is a 16.16 fixed
/// value clamped to +-8; a vertical direction yields +-8 and angle 0.
pub fn direction_to_angle_slope(d: [f32; 3]) -> (u32, i32) {
    let (dx, dy, dz) = (f64::from(d[0]), f64::from(d[1]), f64::from(d[2]));
    let horiz = dx.hypot(dy);
    let angle = if horiz < 1e-6 {
        0
    } else {
        degrees_to_bam(dy.atan2(dx).to_degrees())
    };
    (angle, slope_fixed(dz, horiz))
}

fn slope_fixed(dz: f64, horiz: f64) -> i32 {
    let slope = if horiz < 1e-6 {
        MAX_SLOPE.copysign(dz)
    } else {
        (dz / horiz).clamp(-MAX_SLOPE, MAX_SLOPE)
    };
    (slope * f64::from(FRACUNIT)) as i32
}

fn to_fixed(units: f64) -> i32 {
    (units * f64::from(FRACUNIT)).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Bookkeeping for one in-flight MW2 rocket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissileInfo {
    /// The weapon that fired it (selects `ProjectileFacts`).
    pub weapon: DmwWeaponId,
    /// `leveltime` when it was spawned.
    pub spawn_tic: i32,
}

/// What the traversal found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    /// Nothing within range.
    Nothing,
    /// A solid line (or the closed opening of a two-sided one).
    Wall {
        /// The line struck.
        line: *mut line_t,
    },
    /// A shootable thing.
    Thing {
        /// The thing struck.
        mobj: *mut Mobj,
    },
}

/// Result of [`trace_pellet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PelletHit {
    /// What was hit.
    pub kind: HitKind,
    /// Impact x (fixed), with Doom's pull-back (10 units for things, 4 for walls).
    pub x: fixed_t,
    /// Impact y (fixed).
    pub y: fixed_t,
    /// Impact z (fixed).
    pub z: fixed_t,
    /// Horizontal distance from the muzzle to the intercept (`FixedMul(attackrange, frac)`).
    pub dist: fixed_t,
    /// Wall hit under a sky ceiling: no puff (`goto_hitline` rule).
    pub sky: bool,
}

impl PelletHit {
    const NOTHING: PelletHit = PelletHit {
        kind: HitKind::Nothing,
        x: 0,
        y: 0,
        z: 0,
        dist: 0,
        sky: false,
    };
}

#[derive(Clone, Copy)]
struct TraceState {
    shootthing: *mut Mobj,
    attackrange: fixed_t,
    aimslope: fixed_t,
    shootz: fixed_t,
    hit: PelletHit,
}

#[derive(Clone, Copy)]
struct RadiusState {
    spot: *mut Mobj,
    source: *mut Mobj,
    radius_units: f32,
    inner: f32,
    outer: f32,
    scale: f32,
}

struct Context {
    pack: Option<Arc<Pack>>,
    balance: BalanceCfg,
}

/// One pellet as resolved by [`fire`] / [`melee`], for tests and the debug log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PelletReport {
    /// What was hit.
    pub kind: HitKind,
    /// Wall hit under sky (no puff).
    pub sky: bool,
    /// Impact point (fixed).
    pub x: fixed_t,
    /// Impact point (fixed).
    pub y: fixed_t,
    /// Impact point (fixed).
    pub z: fixed_t,
    /// Horizontal distance to the intercept (fixed).
    pub dist: fixed_t,
    /// Hit location index (0 when nothing was hit).
    pub hitloc: u8,
    /// Damage passed to `P_DamageMobj` (0 for walls and misses).
    pub damage: i32,
    /// The target's health before the pellet.
    pub health_before: i32,
    /// The target's health after the pellet.
    pub health_after: i32,
}

/// What the last [`fire`] / [`melee`] call did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShotReport {
    /// One entry per pellet, in order.
    pub pellets: Vec<PelletReport>,
    /// The `hud::on_hit` notifications, in order.
    pub feedback: Vec<HitFeedback>,
    /// Missiles spawned.
    pub projectiles: usize,
}

/// What the last MW2 explosion did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExplosionReport {
    /// Explosion centre (fixed x, y, z).
    pub spot: (fixed_t, fixed_t, fixed_t),
    /// `(thing address, distance in Doom units after radius, damage dealt)` per damaged thing.
    pub hits: Vec<(usize, f32, i32)>,
}

thread_local! {
    static CONTEXT: RefCell<Context> = RefCell::new(Context {
        pack: None,
        balance: BalanceCfg::default(),
    });
    /// In-flight MW2 rockets by mobj address (sheet 09 section 5.7).
    static MISSILES: RefCell<BTreeMap<usize, MissileInfo>> = const { RefCell::new(BTreeMap::new()) };
    static TRACE: RefCell<TraceState> = const { RefCell::new(TraceState {
        shootthing: std::ptr::null_mut(),
        attackrange: 0,
        aimslope: 0,
        shootz: 0,
        hit: PelletHit::NOTHING,
    }) };
    static RADIUS: RefCell<RadiusState> = const { RefCell::new(RadiusState {
        spot: std::ptr::null_mut(),
        source: std::ptr::null_mut(),
        radius_units: 0.0,
        inner: 0.0,
        outer: 0.0,
        scale: 1.0,
    }) };
    static REPORT: RefCell<ShotReport> = RefCell::new(ShotReport::default());
    static EXPLOSION: RefCell<ExplosionReport> = RefCell::new(ExplosionReport::default());
}

/// Hands ballistics the pack (weapon facts) and the `[balance]` numbers. Sheet 08 calls this at
/// level start, next to [`reset_level`].
pub fn configure(pack: Arc<Pack>, balance: BalanceCfg) {
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        c.pack = Some(pack);
        c.balance = balance;
    });
}

/// Level start: forgets every registered missile and the last reports.
pub fn reset_level() {
    MISSILES.with(|m| m.borrow_mut().clear());
    REPORT.with(|r| *r.borrow_mut() = ShotReport::default());
    EXPLOSION.with(|e| *e.borrow_mut() = ExplosionReport::default());
}

/// Number of registered in-flight MW2 missiles.
pub fn missile_count() -> usize {
    MISSILES.with(|m| m.borrow().len())
}

/// Addresses of the registered missiles, oldest address first (test and debug aid).
pub fn missile_addresses() -> Vec<usize> {
    MISSILES.with(|m| m.borrow().keys().copied().collect())
}

/// What the last [`fire`] / [`melee`] call did.
pub fn last_report() -> ShotReport {
    REPORT.with(|r| r.borrow().clone())
}

/// What the last [`mw2_explode`] did.
pub fn last_explosion() -> ExplosionReport {
    EXPLOSION.with(|e| e.borrow().clone())
}

fn pack() -> Option<Arc<Pack>> {
    CONTEXT.with(|c| c.borrow().pack.clone())
}

fn balance() -> BalanceCfg {
    CONTEXT.with(|c| c.borrow().balance.clone())
}

// ---------------------------------------------------------------------------
// 3D hitscan
// ---------------------------------------------------------------------------

/// Traverse callback for [`P_PathTraverse`]: `PTR_ShootTraverse` with the 3D aim of this module
/// (state in [`TRACE`]), recording the hit instead of damaging it.
///
/// # Safety
///
/// `in_` must point to a valid `intercept_t` produced by `P_PathTraverse`, and [`TRACE`] must
/// hold the shooter, range, slope and origin height of the current trace.
unsafe extern "C" fn dmw_shoot_traverse(in_: *mut intercept_t) -> c_uint {
    let st = TRACE.with(|t| *t.borrow());
    // SAFETY: `in_` is valid for the callback (function contract); the line and sector pointers
    // come from the loaded level; `trace` is written by `P_PathTraverse` before the callbacks.
    let hit = unsafe {
        let in_ = &*in_;
        if in_.isaline != 0 {
            let li = in_.d.line;
            if (*li).special != 0 {
                P_ShootSpecialLine(st.shootthing.cast::<CMobj>(), li);
            }
            if (*li).flags as c_int & crate::doom::c_ffi::LinedefFlag::TWOSIDED as c_int != 0 {
                P_LineOpening(li);
                let dist = FixedMul(st.attackrange, in_.frac);
                let back = (*li).backsector.cast::<sector_t>();
                let front = (*li).frontsector.cast::<sector_t>();
                let mut blocked = false;
                if back.is_null() {
                    blocked = FixedDiv(openbottom - st.shootz, dist) > st.aimslope
                        || FixedDiv(opentop - st.shootz, dist) < st.aimslope;
                } else {
                    if (*front).floorheight != (*back).floorheight
                        && FixedDiv(openbottom - st.shootz, dist) > st.aimslope
                    {
                        blocked = true;
                    }
                    if !blocked
                        && (*front).ceilingheight != (*back).ceilingheight
                        && FixedDiv(opentop - st.shootz, dist) < st.aimslope
                    {
                        blocked = true;
                    }
                }
                if !blocked {
                    return 1;
                }
            }
            wall_hit(&st, li, in_)
        } else {
            let th = in_.d.thing.cast::<Mobj>();
            if th == st.shootthing {
                return 1;
            }
            if (*th).flags & MF_SHOOTABLE == 0 {
                return 1;
            }
            let dist = FixedMul(st.attackrange, in_.frac);
            if FixedDiv((*th).z + (*th).height - st.shootz, dist) < st.aimslope {
                return 1;
            }
            if FixedDiv((*th).z - st.shootz, dist) > st.aimslope {
                return 1;
            }
            let frac = in_.frac - FixedDiv(10 * FRACUNIT, st.attackrange);
            let x = trace.x + FixedMul(trace.dx, frac);
            let y = trace.y + FixedMul(trace.dy, frac);
            let z = st.shootz + FixedMul(st.aimslope, FixedMul(frac, st.attackrange));
            PelletHit {
                kind: HitKind::Thing { mobj: th },
                x,
                y,
                z,
                dist,
                sky: false,
            }
        }
    };
    TRACE.with(|t| t.borrow_mut().hit = hit);
    0
}

/// The wall half of `goto_hitline`: impact point 4 units short of the line, sky rule.
///
/// # Safety
///
/// `li` must be a valid line of the loaded level; `trace` must be current.
unsafe fn wall_hit(st: &TraceState, li: *mut line_t, in_: &intercept_t) -> PelletHit {
    // SAFETY: function contract; sector pointers belong to the loaded level.
    unsafe {
        let frac = in_.frac - FixedDiv(4 * FRACUNIT, st.attackrange);
        let x = trace.x + FixedMul(trace.dx, frac);
        let y = trace.y + FixedMul(trace.dy, frac);
        let z = st.shootz + FixedMul(st.aimslope, FixedMul(frac, st.attackrange));
        let mut sky = false;
        let front = (*li).frontsector.cast::<sector_t>();
        if (*front).ceilingpic as c_int == skyflatnum {
            if z > (*front).ceilingheight {
                sky = true;
            } else {
                let back = (*li).backsector.cast::<sector_t>();
                if !back.is_null() && (*back).ceilingpic as c_int == skyflatnum {
                    sky = true;
                }
            }
        }
        PelletHit {
            kind: HitKind::Wall { line: li },
            x,
            y,
            z,
            dist: FixedMul(st.attackrange, in_.frac),
            sky,
        }
    }
}

/// Traces one ray from `shooter`'s `(x, y)` at height `shootz` along the unit direction `dir`
/// for `range_units` Doom units and returns the first thing or wall struck. Gun-triggered
/// specials on crossed lines fire, as in Doom.
///
/// # Safety
///
/// `shooter` must point to a live map object and a level must be loaded.
pub unsafe fn trace_ray(
    shooter: *mut Mobj,
    shootz: fixed_t,
    dir: [f32; 3],
    range_units: f64,
) -> PelletHit {
    let (dx, dy, dz) = (f64::from(dir[0]), f64::from(dir[1]), f64::from(dir[2]));
    let horiz = dx.hypot(dy);
    // A vertical ray still needs a valid traverse: one unit east (sheet 09 section 6).
    let (ux, uy, len) = if horiz < 1e-6 {
        (1.0, 0.0, 1.0)
    } else {
        (dx / horiz, dy / horiz, (range_units * horiz).max(1.0))
    };
    // SAFETY: function contract.
    unsafe {
        let x1 = (*shooter).x;
        let y1 = (*shooter).y;
        let x2 = to_fixed(f64::from(x1) / f64::from(FRACUNIT) + ux * len);
        let y2 = to_fixed(f64::from(y1) / f64::from(FRACUNIT) + uy * len);
        let ddx = f64::from(x2) - f64::from(x1);
        let ddy = f64::from(y2) - f64::from(y1);
        let attackrange = (ddx.hypot(ddy) as i32).max(1);
        TRACE.with(|t| {
            *t.borrow_mut() = TraceState {
                shootthing: shooter,
                attackrange,
                aimslope: slope_fixed(dz, horiz),
                shootz,
                hit: PelletHit::NOTHING,
            }
        });
        P_PathTraverse(
            x1,
            y1,
            x2,
            y2,
            PT_ADDLINES | PT_ADDTHINGS,
            Some(dmw_shoot_traverse),
        );
    }
    TRACE.with(|t| t.borrow().hit)
}

/// Traces one pellet of `player` along `dir` (MW2 world axes = Doom world axes) for
/// `max_range_mw2` inches, from the eye (`mo.x`, `mo.y`, `viewz`). See [`trace_ray`].
///
/// # Safety
///
/// `player` must point to a player with a live map object.
pub unsafe fn trace_pellet(player: *mut PlayerT, dir: [f32; 3], max_range_mw2: f32) -> PelletHit {
    // SAFETY: function contract.
    unsafe {
        let mo = (*player).mo.cast::<Mobj>();
        let range = (f64::from(max_range_mw2) * f64::from(DOOM_PER_MW2)).min(MAX_RANGE_UNITS);
        trace_ray(mo, (*player).viewz, dir, range)
    }
}

// ---------------------------------------------------------------------------
// Firing
// ---------------------------------------------------------------------------

/// Resolves a shot of `player` (sheet 09 section 5.5): hitscan pellets with MW2 damage, or a
/// rocket when `req.projectile` is set. Called by sheet 08 for every `SimEvent::Shot`.
///
/// # Safety
///
/// `player` must point to a player with a live map object and a level must be loaded.
pub unsafe fn fire(player: *mut PlayerT, req: &ShotRequest) {
    let Some(pack) = pack() else {
        log_once!(
            LOG,
            log::Level::Error,
            "ballistics:nopack",
            "fire without a configured pack"
        );
        return;
    };
    let Some(wf) = pack.try_facts(req.weapon) else {
        log_once!(
            LOG,
            log::Level::Error,
            "ballistics:badweapon",
            "unknown weapon id {}",
            req.weapon.0
        );
        return;
    };
    let bal = balance();
    REPORT.with(|r| *r.borrow_mut() = ShotReport::default());
    // SAFETY: function contract.
    unsafe {
        let mo = (*player).mo.cast::<Mobj>();
        if bal.alert_monsters_on_fire {
            P_NoiseAlert(mo, mo);
        }
        if req.projectile {
            spawn_projectile(player, req, wf);
            return;
        }
        let combat = &wf.combat;
        let mut total = 0;
        let mut headshots = 0;
        for dir in &req.pellets {
            let hit = trace_pellet(player, *dir, req.max_range_mw2);
            let rep = apply_pellet(player, combat, &bal, hit);
            total += rep.damage;
            if matches!(rep.kind, HitKind::Thing { .. }) && is_headshot(rep.hitloc) {
                headshots += 1;
            }
            log::trace!(
                target: LOG,
                "pellet dir={dir:?} hit={:?} dist={} hitloc={} damage={}",
                rep.kind,
                f64::from(rep.dist) / f64::from(FRACUNIT),
                rep.hitloc,
                rep.damage
            );
            REPORT.with(|r| r.borrow_mut().pellets.push(rep));
        }
        let hits = REPORT.with(|r| {
            r.borrow()
                .pellets
                .iter()
                .filter(|p| matches!(p.kind, HitKind::Thing { .. }))
                .count()
        });
        log::debug!(
            target: LOG,
            "shot weapon={} pellets={} hits={} total_damage={} headshots={}",
            wf.name,
            req.pellets.len(),
            hits,
            total,
            headshots
        );
    }
}

/// Applies one traced pellet: puff or blood, damage, HUD notification.
///
/// # Safety
///
/// `player` and the pointers inside `hit` must be live.
unsafe fn apply_pellet(
    player: *mut PlayerT,
    combat: &WeaponCombatFacts,
    bal: &BalanceCfg,
    hit: PelletHit,
) -> PelletReport {
    let mut rep = PelletReport {
        kind: hit.kind,
        sky: hit.sky,
        x: hit.x,
        y: hit.y,
        z: hit.z,
        dist: hit.dist,
        hitloc: 0,
        damage: 0,
        health_before: 0,
        health_after: 0,
    };
    // SAFETY: function contract; `P_Spawn*` and `P_DamageMobj` are the engine's own entry points.
    unsafe {
        match hit.kind {
            HitKind::Nothing => {}
            HitKind::Wall { .. } => {
                if !hit.sky {
                    P_SpawnPuff(hit.x, hit.y, hit.z);
                }
            }
            HitKind::Thing { mobj } => {
                let shootz = (*player).viewz;
                let th = &*mobj;
                let h = if th.height > 0 {
                    (hit.z - th.z) as f32 / th.height as f32
                } else {
                    0.0
                };
                let hitloc = if th.height > 0 {
                    hit_location(h, th.mobjtype, bal.headshot_enabled)
                } else {
                    4
                };
                let dz = f64::from(hit.z - shootz) / f64::from(FRACUNIT);
                let dh = f64::from(hit.dist) / f64::from(FRACUNIT);
                let dist3_mw2 = (dh.hypot(dz) as f32) * MW2_PER_DOOM;
                let dmg = pellet_damage(combat, dist3_mw2, hitloc, bal.monster_damage_scale);
                rep.hitloc = hitloc;
                rep.damage = dmg;
                rep.health_before = th.health;
                if th.flags & MF_NOBLOOD != 0 {
                    P_SpawnPuff(hit.x, hit.y, hit.z);
                } else {
                    P_SpawnBlood(hit.x, hit.y, hit.z, dmg);
                }
                let mo = (*player).mo.cast::<Mobj>();
                P_DamageMobj(mobj, mo, mo, dmg);
                rep.health_after = (*mobj).health;
                let fb = HitFeedback {
                    headshot: is_headshot(hitloc),
                    killed: rep.health_before > 0 && rep.health_after <= 0,
                };
                REPORT.with(|r| r.borrow_mut().feedback.push(fb));
                hud::on_hit(fb);
            }
        }
    }
    rep
}

// ---------------------------------------------------------------------------
// Melee
// ---------------------------------------------------------------------------

/// Resolves an MW2 knife swing (sheet 09 section 5.6): one straight pellet, no falloff, no hit
/// location, `melee_damage_scale` instead of `monster_damage_scale`.
///
/// # Safety
///
/// `player` must point to a player with a live map object and a level must be loaded.
pub unsafe fn melee(player: *mut PlayerT, req: &MeleeRequest) {
    let bal = balance();
    REPORT.with(|r| *r.borrow_mut() = ShotReport::default());
    let range = (f64::from(req.range_mw2) * f64::from(DOOM_PER_MW2)).max(MELEERANGE_UNITS);
    // SAFETY: function contract.
    unsafe {
        let hit = trace_ray((*player).mo.cast::<Mobj>(), (*player).viewz, req.dir, range);
        let mut rep = PelletReport {
            kind: hit.kind,
            sky: hit.sky,
            x: hit.x,
            y: hit.y,
            z: hit.z,
            dist: hit.dist,
            hitloc: 0,
            damage: 0,
            health_before: 0,
            health_after: 0,
        };
        if let HitKind::Thing { mobj } = hit.kind {
            let raw = (req.damage as f32 * bal.melee_damage_scale).round() as i32;
            let dmg = if req.damage > 0 { raw.max(1) } else { 0 };
            rep.damage = dmg;
            rep.health_before = (*mobj).health;
            if (*mobj).flags & MF_NOBLOOD != 0 {
                P_SpawnPuff(hit.x, hit.y, hit.z);
            } else {
                P_SpawnBlood(hit.x, hit.y, hit.z, dmg);
            }
            let mo = (*player).mo.cast::<Mobj>();
            P_DamageMobj(mobj, mo, mo, dmg);
            rep.health_after = (*mobj).health;
            // The weapon's `melee_hit` alias through sheet 12's mixer; Doom's punch is the
            // fallback when the weapon has none or the pack lacks the sound.
            let melee_alias = pack()
                .and_then(|p| p.try_facts(req.weapon).and_then(|f| f.sounds.melee_hit.clone()));
            let cue = melee_alias.map(|alias| dmw_weapons::SoundCue { alias, local: true });
            let played = cue
                .as_ref()
                .is_some_and(|c| crate::dmw::audio::play(c, (*player).mo.cast()));
            if !played {
                S_StartSound(mo.cast::<c_void>(), Sfx::Punch as c_int);
            }
            let fb = HitFeedback {
                headshot: false,
                killed: rep.health_before > 0 && rep.health_after <= 0,
            };
            REPORT.with(|r| r.borrow_mut().feedback.push(fb));
            hud::on_hit(fb);
        }
        log::debug!(
            target: LOG,
            "melee weapon={} hit={:?} damage={}",
            req.weapon.0,
            rep.kind,
            rep.damage
        );
        REPORT.with(|r| r.borrow_mut().pellets.push(rep));
    }
}

// ---------------------------------------------------------------------------
// Projectiles
// ---------------------------------------------------------------------------

/// Spawns one `MT_ROCKET` per pellet direction and registers it (sheet 09 section 5.7).
///
/// # Safety
///
/// `player` must point to a player with a live map object.
unsafe fn spawn_projectile(player: *mut PlayerT, req: &ShotRequest, wf: &dmw_format::WeaponFacts) {
    let Some(pf) = wf.projectile.as_ref() else {
        log_once!(
            LOG,
            log::Level::Error,
            "ballistics:noprojectile",
            "{} has no projectile facts",
            wf.name
        );
        return;
    };
    if fire_weapon_kind(wf.combat.weap_type, wf.combat.weap_class)
        == Some(FireWeaponKind::GrenadeLauncher)
    {
        log_once!(
            LOG,
            log::Level::Warn,
            "ballistics:grenade",
            "{} is a grenade launcher: flying straight (no gravity arc)",
            wf.name
        );
    }
    let speed_units = f64::from(pf.projectile_speed) * f64::from(DOOM_PER_MW2) / 35.0;
    // SAFETY: function contract; `P_SpawnMobj` returns a live, zeroed-then-initialised mobj.
    unsafe {
        let mo = (*player).mo.cast::<Mobj>();
        for dir in &req.pellets {
            let (angle, _) = direction_to_angle_slope(*dir);
            let d = [f64::from(dir[0]), f64::from(dir[1]), f64::from(dir[2])];
            let radius_units = 11.0; // MT_ROCKET radius
            let off = 4.0 * radius_units;
            let fx = |v: i32, c: f64| v.saturating_add(to_fixed(c));
            let x = fx((*mo).x, d[0] * off);
            let y = fx((*mo).y, d[1] * off);
            let z = fx((*player).viewz - 8 * FRACUNIT, d[2] * off);
            let th = P_SpawnMobj(x, y, z, MT_ROCKET);
            (*th).target = mo;
            (*th).angle = angle;
            (*th).momx = to_fixed(d[0] * speed_units);
            (*th).momy = to_fixed(d[1] * speed_units);
            (*th).momz = to_fixed(d[2] * speed_units);
            MISSILES.with(|m| {
                m.borrow_mut().insert(
                    th as usize,
                    MissileInfo {
                        weapon: req.weapon,
                        spawn_tic: leveltime,
                    },
                );
            });
            REPORT.with(|r| r.borrow_mut().projectiles += 1);
            // May explode at once (and run the A_Explode hook), so register first.
            P_CheckMissileSpawn(th);
        }
    }
    log::debug!(
        target: LOG,
        "shot weapon={} projectiles={} speed={speed_units:.1}/tic",
        wf.name,
        req.pellets.len()
    );
}

/// `A_Explode` hook body: `true` when `thing` is a registered MW2 rocket and the MW2 explosion
/// handled it; `false` runs vanilla `A_Explode`.
///
/// # Safety
///
/// `thing` must be null or a live map object.
pub unsafe fn on_explode(thing: *mut Mobj) -> bool {
    if MISSILES.with(|m| m.borrow().is_empty()) {
        return false;
    }
    if !crate::dmw::mode::level_active() {
        // MW2 mode went off (mode::fail): fall back to vanilla explosions.
        MISSILES.with(|m| m.borrow_mut().clear());
        return false;
    }
    let Some(info) = MISSILES.with(|m| m.borrow_mut().remove(&(thing as usize))) else {
        return false;
    };
    // SAFETY: function contract (`thing` is non-null: it was found in the registry).
    unsafe { mw2_explode(thing, &info) };
    true
}

/// `P_RemoveMobj` hook body: forgets a registered missile that vanished without exploding.
///
/// # Safety
///
/// `mo` is only used as an address.
pub unsafe fn on_remove_mobj(mo: *mut Mobj) {
    MISSILES.with(|m| {
        let mut m = m.borrow_mut();
        if !m.is_empty() {
            m.remove(&(mo as usize));
        }
    });
}

/// Radius damage of an MW2 explosion at `spot` (sheet 09 section 5.7): `lerp(inner, outer,
/// d / R) * explosive_damage_scale`, `R = explosion_radius * 0.8`, cyberdemon and spider
/// mastermind immune, line of sight required. The shooter is damaged like anything else.
///
/// # Safety
///
/// `spot` must point to a live map object in a loaded level.
pub unsafe fn mw2_explode(spot: *mut Mobj, info: &MissileInfo) {
    let Some(pack) = pack() else { return };
    let Some(pf) = pack
        .try_facts(info.weapon)
        .and_then(|f| f.projectile.as_ref())
    else {
        return;
    };
    let bal = balance();
    let radius_units = pf.explosion_radius as f32 * DOOM_PER_MW2;
    if radius_units <= 0.0 {
        return;
    }
    // SAFETY: function contract; the blockmap is valid for a loaded level.
    unsafe {
        let source = (*spot).target.cast::<Mobj>();
        RADIUS.with(|r| {
            *r.borrow_mut() = RadiusState {
                spot,
                source,
                radius_units,
                inner: pf.explosion_inner_damage as f32,
                outer: pf.explosion_outer_damage as f32,
                scale: bal.explosive_damage_scale,
            }
        });
        EXPLOSION.with(|e| {
            *e.borrow_mut() = ExplosionReport {
                spot: ((*spot).x, (*spot).y, (*spot).z),
                hits: Vec::new(),
            }
        });
        // Same blockmap window as `P_RadiusAttack`.
        let dist = to_fixed(f64::from(radius_units)) + MAXRADIUS_FIXED;
        let yh = ((*spot).y + dist - bmaporgy) >> BLOCKSHIFT;
        let yl = ((*spot).y - dist - bmaporgy) >> BLOCKSHIFT;
        let xh = ((*spot).x + dist - bmaporgx) >> BLOCKSHIFT;
        let xl = ((*spot).x - dist - bmaporgx) >> BLOCKSHIFT;
        for y in yl..=yh {
            for x in xl..=xh {
                P_BlockThingsIterator(x, y, Some(dmw_radius_thing));
            }
        }
    }
    let n = EXPLOSION.with(|e| e.borrow().hits.len());
    log::debug!(
        target: LOG,
        "explosion weapon={} radius={radius_units:.1} damaged={n}",
        pack.facts(info.weapon).name
    );
}

/// `PIT_RadiusAttack` with MW2 numbers (state in [`RADIUS`]).
///
/// # Safety
///
/// `thing` must be a live map object handed over by the blockmap iterator.
unsafe extern "C" fn dmw_radius_thing(thing: *mut CMobj) -> c_uint {
    let st = RADIUS.with(|r| *r.borrow());
    // SAFETY: the iterator hands over live things; `st.spot` is the live explosion centre.
    unsafe {
        let th = &*thing.cast::<Mobj>();
        if th.flags & MF_SHOOTABLE == 0 || th.mobjtype == MT_CYBORG || th.mobjtype == MT_SPIDER {
            return 1;
        }
        let dx = (th.x - (*st.spot).x).wrapping_abs();
        let dy = (th.y - (*st.spot).y).wrapping_abs();
        let d_fixed = (dx.max(dy) - th.radius).max(0);
        let d = d_fixed as f32 / FRACUNIT as f32;
        if d >= st.radius_units {
            return 1;
        }
        if P_CheckSight(thing.cast(), st.spot.cast()) == 0 {
            return 1;
        }
        let t = d / st.radius_units;
        let dmg = ((st.inner + (st.outer - st.inner) * t) * st.scale).round() as i32;
        let dmg = dmg.max(1);
        P_DamageMobj(thing.cast(), st.spot, st.source, dmg);
        EXPLOSION.with(|e| e.borrow_mut().hits.push((thing as usize, d, dmg)));
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmw_weapons::ShotRequest;

    fn rifle() -> WeaponCombatFacts {
        let pack = Pack::fixture();
        let id = pack.weapon_id("fixture_rifle").unwrap();
        pack.facts(id).combat
    }

    #[test]
    fn t9_1_damage_falloff_matches_the_formula() {
        let f = rifle(); // damage 30, min 20, 1000..2000 inches
        for (dist, want) in [
            (0.0, 30),
            (999.0, 30),
            (1000.0, 30),
            (1250.0, 27),
            (1500.0, 25),
            (1999.0, 20),
            (2000.0, 20),
            (6000.0, 20),
        ] {
            assert_eq!(bullet_damage_at_distance(&f, dist), want, "dist {dist}");
        }
        // min_damage 0 means no falloff.
        let mut flat = f;
        flat.min_damage = 0;
        assert_eq!(bullet_damage_at_distance(&flat, 5000.0), 30);
        // Equal damages and a degenerate range never divide by zero.
        let mut same = f;
        same.min_damage = same.damage;
        assert_eq!(bullet_damage_at_distance(&same, 1500.0), 30);
        let mut degenerate = f;
        degenerate.min_damage_range = degenerate.max_damage_range;
        assert_eq!(bullet_damage_at_distance(&degenerate, 1500.0), 30);
    }

    #[test]
    fn pellet_damage_applies_location_scale_and_floor() {
        let f = rifle();
        assert_eq!(pellet_damage(&f, 100.0, 2, 0.35), 26); // 30 * 2.5 * 0.35 = 26.25
        assert_eq!(pellet_damage(&f, 100.0, 4, 0.35), 11); // 10.5 rounds up
        assert_eq!(pellet_damage(&f, 100.0, 14, 0.35), 8); // 30 * 0.8 * 0.35 = 8.4
        assert_eq!(pellet_damage(&f, 100.0, 4, 0.0), 1); // positive base damage is never 0
        let mut dead = f;
        dead.damage = 0;
        dead.min_damage = 0;
        assert_eq!(pellet_damage(&dead, 100.0, 2, 1.0), 0);
    }

    #[test]
    fn t9_2_hit_location_boundaries() {
        let typ = crate::doom::info::MT_POSSESSED;
        for (h, want) in [
            (1.0, 2),
            (0.85, 2),
            (0.8499, 3),
            (0.70, 3),
            (0.6999, 4),
            (0.45, 4),
            (0.4499, 5),
            (0.30, 5),
            (0.2999, 12),
            (0.12, 12),
            (0.1199, 14),
            (0.0, 14),
            (-3.0, 14),
            (5.0, 2),
        ] {
            assert_eq!(hit_location(h, typ, true), want, "h {h}");
        }
        for t in [
            MT_HEAD,
            MT_SKULL,
            MT_PAIN,
            MT_SPIDER,
            MT_BARREL,
            MT_KEEN,
            MT_BOSSBRAIN,
        ] {
            assert!(!has_head(t));
            assert_eq!(hit_location(0.95, t, true), 4);
            assert_eq!(hit_location(0.01, t, true), 4);
        }
        assert!(has_head(MT_CYBORG));
        assert_eq!(hit_location(0.95, typ, false), 4);
        assert!(is_headshot(1) && is_headshot(2) && !is_headshot(3) && !is_headshot(4));
    }

    #[test]
    fn t9_3_direction_to_angle_and_slope() {
        let one = FRACUNIT;
        for (d, angle_deg, slope) in [
            ([1.0, 0.0, 0.0], 0.0, 0),
            ([0.0, 1.0, 0.0], 90.0, 0),
            ([-1.0, 0.0, 0.0], 180.0, 0),
            ([0.0, -1.0, 0.0], 270.0, 0),
        ] {
            let (a, s) = direction_to_angle_slope(d);
            assert_eq!(a, degrees_to_bam(angle_deg), "{d:?}");
            assert_eq!(s, slope, "{d:?}");
        }
        // Straight up and down clamp to +-8 and keep a defined angle.
        assert_eq!(direction_to_angle_slope([0.0, 0.0, 1.0]), (0, 8 * one));
        assert_eq!(direction_to_angle_slope([0.0, 0.0, -1.0]), (0, -8 * one));
        // 45 degrees: slope 1 (within fixed rounding), angle 45 / 135.
        let r = std::f32::consts::FRAC_1_SQRT_2;
        let (a, s) = direction_to_angle_slope([r, 0.0, r]);
        assert_eq!(a, 0);
        assert!((s - one).abs() <= 2, "slope {s}");
        let (a, s) = direction_to_angle_slope([-0.5, 0.5, -r]);
        assert_eq!(a, degrees_to_bam(135.0));
        assert!((s + one).abs() <= 2 + one / 10, "slope {s}");
        // A steep but not vertical ray clamps at +-8.
        assert_eq!(direction_to_angle_slope([0.01, 0.0, 0.99995]).1, 8 * one);
    }

    #[test]
    fn registry_hooks_do_nothing_when_empty() {
        reset_level();
        // SAFETY: null is never dereferenced while the registry is empty.
        unsafe {
            assert!(!on_explode(std::ptr::null_mut()));
            on_remove_mobj(std::ptr::null_mut());
        }
        assert_eq!(missile_count(), 0);
    }

    #[test]
    fn fire_without_a_pack_is_a_logged_no_op() {
        let req = ShotRequest {
            weapon: DmwWeaponId(1),
            pellets: vec![[1.0, 0.0, 0.0]],
            base_damage: 10,
            max_range_mw2: 100.0,
            projectile: false,
        };
        CONTEXT.with(|c| c.borrow_mut().pack = None);
        // SAFETY: with no pack `fire` returns before touching the player pointer.
        unsafe { fire(std::ptr::null_mut(), &req) };
        assert!(last_report().pellets.is_empty());
    }
}
