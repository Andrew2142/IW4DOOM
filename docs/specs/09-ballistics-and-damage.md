# 09 — Ballistics and Damage

Owner: build agent (wave 4)
Status: implementation-ready
Depends on: 00 (§2 D11, D14; §5.1; §7.2 `ShotRequest`, `MeleeRequest`; §9.3), 01, 07 (requests), 08 (dispatch, hooks.rs)

MW2 decides that a shot happened and in which directions its pellets go (07). This sheet makes those pellets hit Doom: true 3D hitscan using the freelook pitch, MW2 range falloff, hit-location multipliers derived from where on a monster the bullet struck, the DOOMMW2 balance scale, Doom-style puffs/blood/gun-triggered lines, MW2 melee, and MW2-tuned rockets.

---

## 1. Purpose

- Bullets go exactly where the crosshair (unsheared view centre) points, plus MW2 spread and gun offset — no Doom autoaim.
- Damage uses MW2's numbers: `damage → min_damage` between `max_damage_range` and `min_damage_range`, times the weapon's hit-location table, times `monster_damage_scale`.
- Headshots exist: hitting the top of a monster uses the weapon's `head` multiplier.
- Everything Doom players expect still happens: puffs on walls, blood on monsters, shootable switches/doors (`G1`/`GR` lines), barrels, infighting, pain states.

## 2. Scope / Out of scope

In scope: `room/src/dmw/ballistics.rs` (`fire`, `melee`, 3D trace, damage math, projectile spawn, explosion), the hook bodies for missile explosion and removal, and the HUD hit notification call.

Out of scope: MW2 penetration through walls and bodies (bullets stop at the first solid line or shootable thing); MW2 impact effects/decals; MW2 explosion FX; grenade gravity arcs (grenade launchers fly straight, with a `warn!`); friendly fire rules (single player).

## 3. Dependencies

| From | What |
|---|---|
| 07 | `ShotRequest { weapon, pellets: Vec<[f32;3]>, base_damage, max_range_mw2, projectile }`, `MeleeRequest` |
| 03 | `WeaponFacts::combat` (`damage`, `min_damage`, `max_damage_range`, `min_damage_range`, `location_damage`), `projectile` (`ProjectileFacts`) |
| iw4L `sim/src/bullet.rs:3` (read, ported) | `bullet_damage_at_distance` (20 lines) |
| iw4L `weapon_iw4` | `HITLOC_NAMES` order (0 none, 1 helmet, 2 head, 3 neck, 4 torso_upper, 5 torso_lower, 6–11 arms/hands, 12 right_leg_upper, 13 left_leg_upper, 14 right_leg_lower, 15 left_leg_lower, 16–17 feet, 18 gun, 19 shield), `location_damage_scale` |
| 12 | `hud::on_hit(HitFeedback { headshot, killed })` |
| room facts | `P_LineAttack`/`PTR_ShootTraverse` (p_map.rs:1027/1191) logic, `P_PathTraverse` with `PT_ADDLINES | PT_ADDTHINGS`, `P_ShootSpecialLine`, `P_LineOpening` (`opentop`, `openbottom`), `goto_hitline` sky rule, `P_SpawnPuff`, `P_SpawnBlood`, `P_DamageMobj(target, inflictor, source, damage)`, `P_RadiusAttack`/`PIT_RadiusAttack` (cyberdemon & spider immune to splash), `A_Explode`, `P_RemoveMobj`, `P_CheckMissileSpawn`, `MT_ROCKET`, `MISSILERANGE = 2048*FRACUNIT`, `MELEERANGE = 64*FRACUNIT`, player `viewz` |

## 4. File inventory

| Path | Content |
|---|---|
| `room/src/dmw/ballistics.rs` | §5 |
| `room/src/doom/p_enemy.rs` (`A_Explode`) | first statement: `if crate::dmw::hooks::on_explode(thingy) { return; }` |
| `room/src/doom/p_mobj.rs` (`P_RemoveMobj`) | first statement: `crate::dmw::hooks::on_remove_mobj(mobj);` |
| `room/tests/ballistics_headless.rs` | §9 |

## 5. Detailed design

### 5.1 Direction conversion

Pellet directions are unit vectors in MW2 world axes, which coincide with Doom's world axes (x east, y north, z up; yaw counter-clockwise from +x). For `d = [dx, dy, dz]`:

```text
angle  = BAM(atan2(dy, dx))                         (00 §5.1)
horiz  = sqrt(dx² + dy²)
slope  = if horiz < 1e-6 { ±BIG } else { dz / horiz }  as fixed (×65536), clamped to ±(8·FRACUNIT)
range  = min(max_range_mw2 · DOOM_PER_MW2, 8192.0) Doom units  (fixed)
origin = (mo.x, mo.y, player.viewz)                 (eye, matching the rendered crosshair)
```

### 5.2 3D hitscan (`trace_pellet`)

A new traverse callback `extern "C" fn dmw_shoot_traverse(in_: *mut intercept_t) -> c_uint` passed to `P_PathTraverse(x1, y1, x2, y2, PT_ADDLINES | PT_ADDTHINGS, Some(dmw_shoot_traverse))`. It mirrors `PTR_ShootTraverse` exactly for lines (special-line activation via `P_ShootSpecialLine(shootthing, li)`, two-sided opening slope tests against `openbottom`/`opentop`, one-sided → hit) but with `shootz = player.viewz`, and for things it **records instead of damaging**:

```rust
struct PelletHit {
    kind: HitKind,           // Wall { line } | Thing { mobj } | Nothing
    x: fixed_t, y: fixed_t, z: fixed_t,   // impact point (same 10-unit / 4-unit pull-back rules as Doom)
    dist: fixed_t,           // FixedMul(attackrange, frac) — horizontal distance
    sky: bool,               // wall hit above a sky ceiling → no puff (goto_hitline rule)
}
```

Thing test (same as Doom): skip `shootthing`, skip non-`MF_SHOOTABLE`, slope window `[(th.z − shootz)/dist, (th.z + th.height − shootz)/dist]` must contain `aimslope`. The traverse state (`shootthing`, `attackrange`, `aimslope`, `shootz`) lives in a `thread_local` of `ballistics.rs`, not in p_map.rs statics, so vanilla code paths are untouched.

3D distance for falloff: `dist3 = sqrt(dist² + (z − shootz)²)`, converted to MW2 inches with `MW2_PER_DOOM`.

### 5.3 Damage

```rust
fn pellet_damage(facts: &WeaponCombatFacts, dist3_mw2: f32, hitloc: u8, scale: f32) -> i32 {
    let base = bullet_damage_at_distance(facts, dist3_mw2);      // port of sim/bullet.rs:3
    let mult = weapon_iw4::location_damage_scale(&facts.location_damage, hitloc);
    let d = (base as f32 * mult * scale).round() as i32;
    if base > 0 { d.max(1) } else { 0 }
}
```

`scale = [balance] monster_damage_scale` for monsters and barrels; when the target is a player (only possible through barrels/rockets) bullets cannot hit the shooter (`shootthing` skip).

### 5.4 Hit location from hit height

`h = (hit.z − th.z) / th.height` clamped to 0..1.

| `h` | hit location (index) |
|---|---|
| ≥ 0.85 | head (2) |
| 0.70 – 0.85 | neck (3) |
| 0.45 – 0.70 | torso_upper (4) |
| 0.30 – 0.45 | torso_lower (5) |
| 0.12 – 0.30 | right_leg_upper (12) |
| < 0.12 | right_leg_lower (14) |

Monsters without an anatomical head use torso_upper (4) for every hit: `MT_HEAD` (cacodemon), `MT_SKULL` (lost soul), `MT_PAIN` (pain elemental), `MT_SPIDER` (spider mastermind), `MT_BARREL`, `MT_KEEN`, `MT_BOSSBRAIN`. `[balance] headshot_enabled = false` → always torso_upper. `headshot = hitloc ∈ {1, 2}` is reported to the HUD.

### 5.5 Applying a shot (`fire(player, req)`)

```text
if req.projectile → spawn_projectile(player, req) (§5.7); return
facts = pack.facts(req.weapon).combat
for dir in req.pellets:
    hit = trace_pellet(player, dir, range)
    match hit.kind:
      Wall  → if !hit.sky { P_SpawnPuff(x, y, z) }
      Thing → dmg = pellet_damage(...)
              if th.flags & MF_NOBLOOD { P_SpawnPuff } else { P_SpawnBlood(x, y, z, dmg) }
              health_before = th.health
              P_DamageMobj(th, mo, mo, dmg)
              hud::on_hit(HitFeedback { headshot, killed: health_before > 0 && th.health <= 0 })
      Nothing → nothing
```

Pellets are resolved independently and in order (each may trigger pain states, as Doom's shotgun does). Gun-triggered specials fire at most once per pellet, as in Doom.

### 5.6 Melee (`melee(player, req)`)

`range = req.range_mw2 · DOOM_PER_MW2` (MW2 64 in ≈ 51 units; Doom `MELEERANGE` is 64 — use `max(range, MELEERANGE)` so Doom monsters' radius is reachable). Trace one pellet straight along the view (yaw + pitch, no spread). On a thing hit: `dmg = round(req.damage × melee_damage_scale)` (no falloff, no hit location, no `monster_damage_scale` — MW2 melee is meant to be lethal to fodder), `P_DamageMobj`, blood, `hud::on_hit`, and play the weapon's `melee_hit` sound via 12 (fallback Doom `sfx_punch`). On a wall: `melee_miss`/no puff. Doom's `A_Punch` auto-turn toward the target is **not** applied.

### 5.7 Projectiles

`spawn_projectile(player, req)` for each pellet direction (normally one):

- Spawn `MT_ROCKET` at `(mo.x, mo.y, player.viewz − 8·FRACUNIT)` plus `4 × radius` along the direction (mirrors `P_SpawnPlayerMissile` offset), `target = mo`, `angle` from §5.1, momentum `dir × speed` where `speed = projectile_speed · DOOM_PER_MW2 / 35` units per tic (fixed), `S_StartSound` with Doom's `seesound` replaced by the MW2 fire sound already played by 07 (so pass `None` here), then `P_CheckMissileSpawn`.
- Register it: `MISSILES: BTreeMap<usize /* mobj ptr */, MissileInfo { weapon, spawn_tic }>` in `DmwState`.
- `on_explode(thing) -> bool` (hook in `A_Explode`): if `thing` is registered → remove it, run `mw2_explode(thing, info)`, return `true`; else `false` (vanilla `A_Explode`).
- `on_remove_mobj(mo)`: remove `mo` from `MISSILES` if present (missiles that vanish into sky never explode). `MISSILES` is cleared at level start.
- `mw2_explode`: like `P_RadiusAttack(spot, source, …)` (blockmap iteration over the radius, `P_CheckSight(thing, spot)`, cyberdemon and spider mastermind immune, `MF_SHOOTABLE` only) but with MW2 numbers: `R = explosion_radius · DOOM_PER_MW2`; for distance `d` (Doom's `PIT_RadiusAttack` distance rule: max of |dx|,|dy| minus target radius, clamped ≥ 0) `< R`: `dmg = lerp(explosion_inner_damage, explosion_outer_damage, d / R) × explosive_damage_scale`, rounded, ≥ 1. The shooter takes the same damage (MW2 and Doom both self-damage).
- The rocket's direct-impact damage stays Doom's (`MT_ROCKET` `damage = 20`, `(P_Random()%8+1)×20`) — documented simplification.
- Weapons whose `fire_weapon_kind` is `GrenadeLauncher` use the same path (straight flight), `log_once!` warn.

### 5.8 Doom RNG

Puffs and blood call `P_Random` (Doom's table) exactly as Doom's own guns do. DOOMMW2 mode is not demo-compatible anyway (D15); vanilla mode never reaches this code.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Pellet straight up/down (`horiz ≈ 0`) | slope clamped to ±8; traverse length `range·horiz` would be 0 → use a minimum horizontal step of 1 unit so `P_PathTraverse` is valid; such shots hit floor/ceiling only (no puff on flats — Doom has no flat puffs either) |
| Thing with `height == 0` | treated as torso_upper |
| `facts.min_damage == 0` | falloff uses `damage` (port keeps iw4L's rule) |
| `MISSILES` stale after `mode::fail` | cleared; explosions of in-flight MW2 rockets fall back to vanilla `A_Explode` |
| Player dies from own rocket | normal Doom death |

## 7. Logging

`dmw::ballistics`: `debug!` per shot summary (`weapon, pellets, hits, total_damage, headshots`); `trace!` per pellet (angle, slope, hit kind, dist, hitloc, damage).

## 8. Licensing and asset notes

`bullet_damage_at_distance` port carries the Apache header and a NOTICE row. The traverse callback is adapted from room's GPL `PTR_ShootTraverse` (same project licence family; note it in provenance).

## 9. Tests

| Id | Test |
|---|---|
| T9.1 | unit: `bullet_damage_at_distance` port equals the iw4L function on a grid of distances for the fixture rifle (call iw4L? `sim` is Bevy-bound — instead compare against hand-computed values from the formula) |
| T9.2 | unit: hit-location table boundaries, no-head monster list, `headshot_enabled=false` |
| T9.3 | unit: direction → (angle, slope) for the 6 axis directions and 45° cases |
| T9.4 | headless: spawn a zombieman (`MT_POSSESSED`) 256 units in front of the player on a flat test map region (use E1M1 start room or a tiny generated test map, see 13 §5.3), fire one `fixture_rifle` pellet straight at its head height → damage = round(30 × head mult × 0.35) and `on_hit.headshot == true`; at foot height → legs multiplier |
| T9.5 | headless: pitch 20° up aims above a monster at eye level → miss; level pitch → hit (freelook aim works, no autoaim) |
| T9.6 | headless: shotgun pellets: `pellet_count` traces, total damage equals the sum of individual pellet damages |
| T9.7 | headless: shooting a `G1` (gun-activated) line opens the door |
| T9.8 | headless: `fixture_launcher` rocket explodes with MW2 radius: a monster at 0.5·R takes `lerp(inner, outer, 0.5) × 0.6`; cyberdemon takes none |
| T9.9 | headless: rocket fired into the sky is removed and `MISSILES` is empty afterwards |
| T9.10 | headless: melee kills an imp (60 HP) with fixture melee damage 135 at 40 units; nothing at 120 units |

## 10. Acceptance criteria

- [ ] Shots land on the crosshair at every pitch within the limit (visual check + T9.5).
- [ ] Headshots deal the head multiplier; HUD notified (T9.4).
- [ ] Doom interactions (puffs, blood, shootable lines, barrels, infighting) behave as with vanilla guns.
- [ ] T9.1–T9.10 pass.

## 11. Contract changes requested

1. 00 §7.5: add rows `A_Explode (p_enemy.rs) → on_explode(thing) -> bool` and `P_RemoveMobj (p_mobj.rs) → on_remove_mobj(mo)`, body owner 09. — Resolved in 00 Revision 4.
