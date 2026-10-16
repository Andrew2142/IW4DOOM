// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 09 headless tests T9.4 to T9.10: the real engine, the synthetic fixture pack and
//! hand-built `ShotRequest`s (sheet 08's dispatch does not exist in this tree).
//!
//! One `#[test]` runs every scenario through one `Harness` (sheet 13 section 5.2).

mod support;

use std::sync::Arc;

use dmw_format::consts::{DOOM_PER_MW2, FRACUNIT, MW2_PER_DOOM};
use dmw_format::{DmwWeaponId, Pack};
use dmw_weapons::{MeleeRequest, ShotRequest};
use room::dmw::ballistics::{
    self, configure, fire, last_explosion, last_report, melee, missile_addresses, missile_count,
    mw2_explode, pellet_damage, reset_level, HitKind, MissileInfo,
};
use room::dmw::config::BalanceCfg;
use room::doom::c_ffi::sector_t;
use room::doom::d_player::{consoleplayer, players, PlayerT};
use room::doom::info::{MT_BRUISER, MT_CYBORG, MT_POSSESSED, MT_ROCKET, MT_TROOP};
use room::doom::p_mobj::P_RemoveMobj;
use room::doom::p_setup::{lines, numsectors, sectors};
use support::locations::{GUN_SWITCH, OPEN_ROOM, SKY_VIEW};
use support::{Harness, Mobj};

const SCALE: f32 = 0.35; // [balance] monster_damage_scale default

fn player() -> *mut PlayerT {
    // SAFETY: `players` is a process-lifetime static; the engine is single-threaded.
    unsafe {
        (&raw mut players)
            .cast::<PlayerT>()
            .add(consoleplayer as usize)
    }
}

fn eye() -> (f32, f32, f32) {
    // SAFETY: a level is running, so the player and its mobj are live.
    unsafe {
        let p = player();
        let mo = (*p).mo.cast::<Mobj>();
        (
            (*mo).x as f32 / FRACUNIT as f32,
            (*mo).y as f32 / FRACUNIT as f32,
            (*p).viewz as f32 / FRACUNIT as f32,
        )
    }
}

/// Spawns `mobjtype` at `(x, y)` and freezes its state timer so it stays idle (no AI) while the
/// scenario runs.
fn spawn_frozen(h: &mut Harness, mobjtype: i32, x: f32, y: f32) -> *mut Mobj {
    let m = h.spawn(mobjtype, x, y);
    // SAFETY: `spawn` returns a live mobj.
    unsafe { (*m).tics = i32::MAX };
    m
}

fn mobj_z(m: *mut Mobj, frac: f32) -> f32 {
    // SAFETY: live mobj.
    unsafe { ((*m).z as f32 + (*m).height as f32 * frac) / FRACUNIT as f32 }
}

fn health(m: *mut Mobj) -> i32 {
    // SAFETY: live mobj (corpses stay in the world).
    unsafe { (*m).health }
}

/// Unit direction from the eye to `(x, y, z)` in Doom units.
fn aim_at(x: f32, y: f32, z: f32) -> [f32; 3] {
    let (ex, ey, ez) = eye();
    let v = [x - ex, y - ey, z - ez];
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

fn request(pack: &Pack, name: &str, pellets: Vec<[f32; 3]>, projectile: bool) -> ShotRequest {
    let id = pack.weapon_id(name).expect("fixture weapon");
    let c = &pack.facts(id).combat;
    ShotRequest {
        weapon: id,
        pellets,
        base_damage: c.damage,
        max_range_mw2: c.bullet_range(),
        projectile,
    }
}

fn clear_world(h: &mut Harness) {
    h.remove_monsters();
    for addr in missile_addresses() {
        // SAFETY: registered missiles are live until removed.
        unsafe { P_RemoveMobj(addr as *mut Mobj) };
    }
    reset_level();
}

#[test]
fn ballistics_headless() {
    let mut h = Harness::start_default();
    h.set_mw2_mode(Some(true));
    let pack = Arc::new(Pack::fixture());
    configure(Arc::clone(&pack), BalanceCfg::default());
    reset_level();

    t9_4_headshot_and_legs(&mut h, &pack);
    t9_5_freelook_aim(&mut h, &pack);
    t9_6_shotgun_pellets(&mut h, &pack);
    t9_8_rocket_explosion(&mut h, &pack);
    t9_9_rocket_into_the_sky(&mut h, &pack);
    t9_10_melee(&mut h);
    t9_7_gun_switch(&mut h, &pack);

    h.set_mw2_mode(None);
}

/// T9.4: one rifle pellet at head height and at foot height of a zombieman 256 units ahead.
fn t9_4_headshot_and_legs(h: &mut Harness, pack: &Pack) {
    OPEN_ROOM.go(h);
    let rifle = pack.weapon_id("fixture_rifle").expect("rifle");
    let combat = &pack.facts(rifle).combat;
    let mult = |loc: usize| combat.location_damage[loc];

    // Head (95 % of the height).
    let z = spawn_frozen(h, MT_POSSESSED, 864.0, 992.0);
    let dir = aim_at(864.0, 992.0, mobj_z(z, 0.95));
    // SAFETY: a level is running with a live player.
    unsafe { fire(player(), &request(pack, "fixture_rifle", vec![dir], false)) };
    let rep = last_report();
    assert_eq!(rep.pellets.len(), 1);
    let p = rep.pellets[0];
    assert!(
        matches!(p.kind, HitKind::Thing { mobj } if mobj == z),
        "{p:?}"
    );
    assert_eq!(p.hitloc, 2, "head");
    let expect = (combat.damage as f32 * mult(2) * SCALE).round() as i32;
    assert_eq!(expect, 26);
    assert_eq!(p.damage, expect);
    assert_eq!(p.health_before - p.health_after, expect);
    assert_eq!(rep.feedback.len(), 1);
    assert!(rep.feedback[0].headshot, "HUD must see the headshot");
    assert!(rep.feedback[0].killed, "26 damage kills a 20 HP zombieman");

    // Feet (5 % of the height).
    let z = spawn_frozen(h, MT_POSSESSED, 864.0, 992.0);
    let dir = aim_at(864.0, 992.0, mobj_z(z, 0.05));
    // SAFETY: as above.
    unsafe { fire(player(), &request(pack, "fixture_rifle", vec![dir], false)) };
    let rep = last_report();
    let p = rep.pellets[0];
    assert!(
        matches!(p.kind, HitKind::Thing { mobj } if mobj == z),
        "{p:?}"
    );
    assert_eq!(p.hitloc, 14, "leg");
    let expect = (combat.damage as f32 * mult(14) * SCALE).round() as i32;
    assert_eq!(expect, 8);
    assert_eq!(p.damage, expect);
    assert_eq!(health(z), 20 - expect);
    assert!(!rep.feedback[0].headshot && !rep.feedback[0].killed);
    clear_world(h);
}

/// T9.5: pitch decides the hit. No autoaim.
fn t9_5_freelook_aim(h: &mut Harness, pack: &Pack) {
    OPEN_ROOM.go(h);
    let z = spawn_frozen(h, MT_POSSESSED, 864.0, 992.0);
    let pitched = |deg: f32| {
        let r = deg.to_radians();
        [0.0, r.cos(), r.sin()]
    };
    for deg in [20.0, -20.0] {
        // SAFETY: a level is running with a live player.
        unsafe {
            fire(
                player(),
                &request(pack, "fixture_rifle", vec![pitched(deg)], false),
            )
        };
        let p = last_report().pellets[0];
        assert!(
            !matches!(p.kind, HitKind::Thing { .. }),
            "pitch {deg} must miss: {p:?}"
        );
        assert_eq!(health(z), 20, "pitch {deg} must not damage");
    }
    // SAFETY: as above.
    unsafe {
        fire(
            player(),
            &request(pack, "fixture_rifle", vec![pitched(0.0)], false),
        )
    };
    let p = last_report().pellets[0];
    assert!(
        matches!(p.kind, HitKind::Thing { mobj } if mobj == z),
        "level shot hits: {p:?}"
    );
    assert!(health(z) < 20);
    clear_world(h);
}

/// T9.6: eight shotgun pellets, resolved independently; the total is the sum of the parts.
fn t9_6_shotgun_pellets(h: &mut Harness, pack: &Pack) {
    OPEN_ROOM.go(h);
    let sg = pack.weapon_id("fixture_shotgun").expect("shotgun");
    let combat = pack.facts(sg).combat;
    assert_eq!(combat.shots_per_fire, 8);
    let baron = spawn_frozen(h, MT_BRUISER, 864.0, 886.0);
    let start = health(baron);
    // (height fraction, lateral offset) and the hit location each must land in.
    let plan = [
        (0.95, -8.0, 2),
        (0.90, 6.0, 2),
        (0.60, -4.0, 4),
        (0.55, 4.0, 4),
        (0.40, -6.0, 5),
        (0.35, 8.0, 5),
        (0.20, 0.0, 12),
        (0.05, 3.0, 14),
    ];
    let dirs: Vec<[f32; 3]> = plan
        .iter()
        .map(|&(f, dx, _)| aim_at(864.0 + dx, 886.0, mobj_z(baron, f)))
        .collect();
    let mut req = request(pack, "fixture_shotgun", dirs, false);
    assert_eq!(req.pellets.len(), combat.shots_per_fire as usize);
    req.max_range_mw2 = combat.bullet_range();
    // SAFETY: a level is running with a live player.
    unsafe { fire(player(), &req) };
    let rep = last_report();
    assert_eq!(rep.pellets.len(), 8);
    let mut sum = 0;
    for (p, &(_, _, loc)) in rep.pellets.iter().zip(plan.iter()) {
        assert!(
            matches!(p.kind, HitKind::Thing { mobj } if mobj == baron),
            "{p:?}"
        );
        assert_eq!(p.hitloc, loc, "{p:?}");
        // Recompute from the report only: 3D distance of the recorded impact.
        let (_, _, ez) = eye();
        let dz = p.z as f32 / FRACUNIT as f32 - ez;
        let dh = p.dist as f32 / FRACUNIT as f32;
        let d3 = (dh * dh + dz * dz).sqrt() * MW2_PER_DOOM;
        assert_eq!(p.damage, pellet_damage(&combat, d3, loc, SCALE), "{p:?}");
        sum += p.damage;
    }
    assert!(sum > 8, "{sum}");
    assert_eq!(
        start - health(baron),
        sum,
        "total health lost is the sum of the pellets"
    );
    assert_eq!(rep.feedback.len(), 8);
    assert_eq!(rep.feedback.iter().filter(|f| f.headshot).count(), 2);
    clear_world(h);
}

/// T9.8: MW2 explosion radius and damage; the `A_Explode` hook path for a real rocket.
fn t9_8_rocket_explosion(h: &mut Harness, pack: &Pack) {
    OPEN_ROOM.go(h);
    let launcher = pack.weapon_id("fixture_launcher").expect("launcher");
    let pf = pack
        .facts(launcher)
        .projectile
        .clone()
        .expect("projectile facts");
    let r = pf.explosion_radius as f32 * DOOM_PER_MW2;
    let info = MissileInfo {
        weapon: launcher,
        spawn_tic: 0,
    };

    // Direct call at a known centre: a baron at 0.5 R (after its radius), a cyberdemon inside R.
    let spot = h.spawn(MT_ROCKET, 864.0, 1000.0);
    let baron_radius = 24.0;
    let baron = spawn_frozen(h, MT_BRUISER, 864.0, 1000.0 + 0.5 * r + baron_radius);
    let cyber = spawn_frozen(h, MT_CYBORG, 864.0 + 60.0, 1000.0);
    // SAFETY: both are live; the player stands 264 units from the spot, outside R = 204.8.
    unsafe {
        (*spot).target = (*player()).mo.cast::<Mobj>();
        mw2_explode(spot, &info);
    }
    let want = ((pf.explosion_inner_damage as f32 + pf.explosion_outer_damage as f32) / 2.0 * 0.6)
        .round() as i32;
    assert_eq!(want, 75);
    assert_eq!(
        1000 - health(baron),
        want,
        "baron takes lerp(inner, outer, 0.5) * 0.6"
    );
    assert_eq!(health(cyber), 4000, "cyberdemon is immune to splash");
    let ex = last_explosion();
    assert_eq!(ex.hits.len(), 1, "{ex:?}");
    assert!((ex.hits[0].1 - 0.5 * r).abs() < 0.01, "{ex:?}");
    // Outside the radius nothing happens.
    let far = spawn_frozen(h, MT_BRUISER, 864.0, 1000.0 + r + baron_radius + 2.0);
    // SAFETY: as above.
    unsafe { mw2_explode(spot, &info) };
    assert_eq!(health(far), 1000);
    clear_world(h);

    // A real rocket north into the far wall: registered, explodes through the hook, forgotten.
    OPEN_ROOM.go(h);
    let dir = [0.0, 1.0, 0.0];
    // SAFETY: a level is running with a live player.
    let wall = unsafe { ballistics::trace_pellet(player(), dir, 8192.0 * MW2_PER_DOOM) };
    assert!(matches!(wall.kind, HitKind::Wall { .. }), "{wall:?}");
    let wy = wall.y as f32 / FRACUNIT as f32;
    let bystander = spawn_frozen(h, MT_BRUISER, 864.0 + 45.0, wy - 60.0);
    reset_level();
    // SAFETY: as above.
    unsafe {
        fire(
            player(),
            &request(pack, "fixture_launcher", vec![dir], true),
        )
    };
    assert_eq!(missile_count(), 1);
    assert_eq!(last_report().projectiles, 1);
    for _ in 0..120 {
        if missile_count() == 0 {
            break;
        }
        h.run_tics(1);
    }
    assert_eq!(missile_count(), 0, "the rocket exploded through the hook");
    let ex = last_explosion();
    assert_ne!(ex.spot, (0, 0, 0), "mw2_explode ran");
    assert!(
        ex.hits
            .iter()
            .any(|&(a, _, dmg)| a == bystander as usize && dmg >= 1),
        "bystander took splash: {ex:?}"
    );
    assert!(health(bystander) < 1000);
    clear_world(h);
}

/// T9.9: a rocket that never explodes is forgotten; sky flight ends with an empty registry.
fn t9_9_rocket_into_the_sky(h: &mut Harness, pack: &Pack) {
    SKY_VIEW.go(h);
    reset_level();
    let r = 60.0f32.to_radians();
    let dir = [-r.cos(), 0.0, r.sin()];
    // SAFETY: a level is running with a live player.
    unsafe {
        fire(
            player(),
            &request(pack, "fixture_launcher", vec![dir], true),
        )
    };
    assert_eq!(missile_count(), 1);
    for _ in 0..200 {
        if missile_count() == 0 {
            break;
        }
        h.run_tics(1);
    }
    assert_eq!(
        missile_count(),
        0,
        "MISSILES is empty after the rocket left the world"
    );

    // The removal hook on its own: a registered rocket removed without exploding.
    reset_level();
    // SAFETY: as above.
    unsafe {
        fire(
            player(),
            &request(pack, "fixture_launcher", vec![dir], true),
        )
    };
    let addrs = missile_addresses();
    assert_eq!(addrs.len(), 1);
    // SAFETY: the registered address is a live rocket.
    unsafe { P_RemoveMobj(addrs[0] as *mut Mobj) };
    assert_eq!(missile_count(), 0, "P_RemoveMobj forgets the missile");
    assert_eq!(last_explosion().spot, (0, 0, 0), "and it never exploded");
    clear_world(h);
}

/// T9.10: MW2 knife damage 135 kills an imp (60 HP) at 40 units and does nothing at 120.
fn t9_10_melee(h: &mut Harness) {
    OPEN_ROOM.go(h);
    let req = |dir| MeleeRequest {
        weapon: DmwWeaponId(1),
        damage: 135,
        range_mw2: 64.0,
        dir,
    };
    let near = spawn_frozen(h, MT_TROOP, 864.0, 776.0);
    // SAFETY: a level is running with a live player.
    unsafe { melee(player(), &req([0.0, 1.0, 0.0])) };
    let rep = last_report();
    assert!(matches!(rep.pellets[0].kind, HitKind::Thing { mobj } if mobj == near));
    assert_eq!(rep.pellets[0].damage, 135);
    assert!(health(near) <= 0, "imp died");
    assert!(rep.feedback[0].killed && !rep.feedback[0].headshot);

    let far = spawn_frozen(h, MT_TROOP, 864.0, 856.0);
    // SAFETY: as above.
    unsafe { melee(player(), &req([0.0, 1.0, 0.0])) };
    let rep = last_report();
    assert!(
        !matches!(rep.pellets[0].kind, HitKind::Thing { .. }),
        "{:?}",
        rep.pellets[0]
    );
    assert_eq!(health(far), 60);
    assert!(rep.feedback.is_empty());
    clear_world(h);
}

/// T9.7: a bullet crossing a gun-activated (GR) door line opens the door.
fn t9_7_gun_switch(h: &mut Harness, pack: &Pack) {
    GUN_SWITCH.go(h);
    let (x1, y1, x2, y2, special) = GUN_SWITCH.line.expect("pinned line");
    assert_eq!(special, 46);
    // SAFETY: a level is loaded; the arrays are valid for their counts.
    let tag = unsafe {
        let li = (0..room::doom::p_setup::numlines as usize)
            .map(|i| &*lines.add(i))
            .find(|l| {
                l.special == 46 && {
                    let v1 = &*l.v1;
                    let v2 = &*l.v2;
                    let f = FRACUNIT as f32;
                    (v1.x as f32 / f).round() == x1
                        && (v1.y as f32 / f).round() == y1
                        && (v2.x as f32 / f).round() == x2
                        && (v2.y as f32 / f).round() == y2
                }
            })
            .expect("pinned GR line");
        li.tag
    };
    let before = tagged_ceilings_by_tag(tag);
    assert!(!before.is_empty(), "the line has tagged sectors");
    h.run_tics(5);
    assert_eq!(
        tagged_ceilings_by_tag(tag),
        before,
        "nothing moves by itself"
    );

    let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    let (_, _, ez) = eye();
    let dir = aim_at(mx, my, ez);
    // SAFETY: a level is running with a live player.
    unsafe { fire(player(), &request(pack, "fixture_rifle", vec![dir], false)) };
    h.run_tics(35);
    let after = tagged_ceilings_by_tag(tag);
    assert!(
        after.iter().zip(&before).any(|(a, b)| a > b),
        "the door opened: {before:?} -> {after:?}"
    );
}

fn tagged_ceilings_by_tag(tag: i16) -> Vec<i32> {
    // SAFETY: `sectors` is valid for `numsectors` elements while the level is loaded.
    unsafe {
        (0..numsectors as usize)
            .map(|i| &*sectors.add(i))
            .filter(|s: &&sector_t| s.tag == tag)
            .map(|s| s.ceilingheight)
            .collect()
    }
}
