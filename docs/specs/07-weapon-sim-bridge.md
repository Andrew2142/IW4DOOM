# 07 — Weapon Simulation Bridge (`dmw_weapons`, sim half)

Owner: build agent (wave 3)
Status: implementation-ready
Depends on: 00 (§2 D4, D9, D11, D13; §5; §7.2; §8.1–8.2; §9.3), 01 (§5.3, §5.6), 03 (`Pack`, `WeaponFacts`)

This is the heart of the port: a Doom-agnostic, deterministic, safe-Rust driver that runs iw4L's MW2 weapon logic once per Doom tic for the local player and reports what happened (shots, sounds, reloads, raises) plus everything presentation needs (ADS fraction, spread, kick, zoom). It mirrors the per-client call sequence of iw4L `sim/src/combat.rs::advance_weapon_command` and the presentation-side state of `render_anim` for a single player with a single hand.

---

## 1. Purpose

Given a `TicInput` (buttons, angles, motion, ammo pools — all already converted to MW2 conventions by sheet 08), produce a `TicOutput` such that, for the same weapon facts, MW2's own rules decide: when a shot fires, how many pellets in which directions, when reload starts/ends and how many rounds move, how far spread blooms and recovers, how the camera kicks, how fast ADS comes up, when sprint lowers the gun and how long sprint-out blocks firing, and when a weapon switch completes.

## 2. Scope / Out of scope

In scope (files 00 §6.2 owned by 07): `lib.rs`, `sim.rs`, `shadow.rs`, `inventory.rs`, `spread.rs`, `rng.rs`, `events.rs`, `view_kick.rs`, `view_sway.rs`, plus `breath.rs` (port of `update_hold_breath`).

Out of scope: viewmodel animation, placement and pose (10 — but this sheet calls 10's placement step at the end of each tic, §5.11, so `gun_angle_offset` is available for the next shot); hit resolution and damage (09); Doom-side inventory/pickups/HUD (08, 12); offhand grenades/equipment; akimbo (hand 1); perks (always 0); stun/shellshock; mantle; prone/crouch (MW2 stance is always standing).

## 3. Dependencies

| From | What |
|---|---|
| 00 §7.2 | `WeaponSim`, `TicInput`, `TicOutput`, `SimEvent`, `ShotRequest`, `MeleeRequest`, `SoundCue`, `Inventory`, `ViewState`, `GunPlacement` |
| 03 | `Pack::facts`, `WeaponFacts` (combat, kick, sway, fpv, movement, projectile, sounds) |
| iw4L `weapon_iw4` | `weapon_hands`, `spawn_weapon_hand`, `WeaponHandState`, `WeaponCmd`, `WeaponTickEvent`, `WeaponState`, `FireType`, `get_spread_for_weapon`, `fire_weapon_spread_degrees`, `add_aim_spread_fire`, `adjust_aim_spread_scale`, `AimSpreadState`, `AimSpreadMotion`, `SpreadOverrideState`, `AIM_SPREAD_MOVE_SPEED_THRESHOLD_DEFAULT`, `is_ads_allowed`, `fire_weapon_kind`, `FireWeaponKind`, `weapon_fire_recoil` & friends (via the ported `ViewKickState`), `sway_*`, `LookSway` (via ported `ViewSwayState`), `PLAYER_MELEE_RANGE_DEFAULT`, `begin_reload_event`/`reload_insert_event` (not needed — events are mapped directly) |
| iw4L `movement_iw4` | `update_sprint`, `SprintContext`, `get_max_sprint_time`, `update_ads_intent`, `AdsIntentContext`, `update_ads_frac`, `AdsFracContext` |
| iw4L `playerstate_iw4` | `PlayerState`, `UserCmd`, `buttons`, `pm_flags`, `weap_flags`, `ENTITYNUM_NONE` |
| iw4L `sim/src/combat.rs` (read, not linked) | the call sequence (lines 144–1000): spread adjust → hands → event handling; `spread_direction_on_plane` (line 1125) to port |
| iw4L `sim/src/step.rs:2420–2500` (read) | `SprintContext` values, `AdsIntentContext`, `AdsFracContext` construction, `player_sprint_time = 4.0` |
| iw4L `render_anim/src/anim/view_kick_state.rs`, `view_sway.rs` (read, ported) | `ViewKickState`, `ViewSwayState` |
| 10 | `placement::step(&mut PlacementState, &ViewInputs) -> GunPlacement` and `placement::gun_angle_offset(&GunPlacement, view_angles, facts) -> [f32; 2]` (10 §5.3) |

## 4. File inventory

| Path | Content |
|---|---|
| `crates/dmw_weapons/src/lib.rs` | public API re-exports (00 §7.2) |
| `src/sim.rs` | `WeaponSim`, tic sequence (§5.3) |
| `src/shadow.rs` | shadow `PlayerState` construction and per-tic sync (§5.2) |
| `src/inventory.rs` | `Inventory`, magazine bookkeeping, pools mapping (§5.9) |
| `src/spread.rs` | spread state helpers, `spread_direction_on_plane` port (§5.6) |
| `src/rng.rs` | `DmwRng` (§5.10) |
| `src/events.rs` | `SimEvent` and payloads |
| `src/view_kick.rs` | port of `ViewKickState` |
| `src/view_sway.rs` | port of `ViewSwayState` |
| `src/breath.rs` | port of `update_hold_breath` |
| `src/switch.rs` | weapon switching and vanilla handoff (§5.8) |
| `tests/sim_timing.rs`, `tests/sim_spread.rs`, `tests/sim_switch.rs`, `tests/golden/sim_*.json` | §9 |

## 5. Detailed design

### 5.1 Ownership and lifetime

```rust
pub struct WeaponSim {
    pack: Arc<Pack>,
    combat: Vec<WeaponCombatFacts>,     // index = DmwWeaponId.0 - 1, copied from pack at new()
    ps: PlayerState,                    // shadow, MW2 conventions
    hands: [WeaponHandState; 2],        // hand 1 always default (no akimbo)
    old_buttons: u32,
    kick: ViewKickState,
    sway: ViewSwayState,
    placement: placement::PlacementState,   // owned type from sheet 10
    gun_angle_offset: [f32; 2],         // from last tic's placement (pitch, yaw), degrees
    spread_deg: f32,
    rng: DmwRng,
    inv: Inventory,
    used: BTreeSet<u16>,                // weapons raised at least once this life (first_raise)
    switch: SwitchState,
    last_view: ViewState,
}
```

`WeaponSim::new(pack)` copies facts; it never fails (the pack was validated). One `WeaponSim` lives for the whole session (sheet 08 owns it in `DmwState`).

### 5.2 Shadow `PlayerState`

`start_level` builds `ps = PlayerState::default()` and sets:

| Field | Value |
|---|---|
| `pm_type` | 0 (normal) |
| `view_height_current`, `view_height_target` | `MW2_STAND_VIEW_HEIGHT` (60.0) and 60 |
| `speed` | 190 (MW2 `g_speed`; used by spread motion and bob) |
| `health` | 100 (breath and ADS checks require > 0); set to 0 while `!input.alive` |
| `weapons` | owned weapon ids (as `i32`) in slots 0.., rest 0 |
| `weapon` | held weapon id or 0 |
| `perks` | `[0, 0]` |
| `ground_entity_num` | 0 (world) / `ENTITYNUM_NONE` when airborne, per tic |
| `e_flags` | 0 (always standing) |

Per tic before running logic: `origin`, `velocity` (from `TicInput`, inches and inches/s), `viewangles = [pitch_deg, yaw_deg, 0]`, `ground_entity_num`, `health`, `pm_type = if alive { 0 } else { PM_DEAD (= the value iw4L uses for dead; read it from playerstate_iw4) }`, `command_time = server_time_prev`.

### 5.3 The tic sequence (`WeaponSim::tic`)

`msec = tic_msec(input.tic)`, `server_time = tic_server_time(input.tic) + msec` (end of this command, IW convention).

1. **UserCmd.** `ucmd = UserCmd { server_time, buttons, angles: [angle2short(pitch), angle2short(yaw), 0], weapon: requested_or_current as u16, forwardmove: scale_move(input.forwardmove), rightmove: scale_move(input.sidemove), ..Default }` where `buttons` maps `TicButtons` per 00 §8.2 and `scale_move(v) = clamp(round(v * 127 / 50), -127, 127)` (Doom run = 50 → MW2 full stick 127, so the `sprint_forward_minimum = 105` test passes when running forward). `angle2short(d) = ((d * 65536.0 / 360.0) as i32) & 0xffff`.
2. **Shadow sync** (§5.2).
2b. **Bob cycle.** `movement_iw4::footsteps_bob_cycle(&mut ps, msec, ucmd.forwardmove, ucmd.rightmove, false, server_time, CmdScaleWalkContext { player_back_speed_scale: 0.7, player_strafe_speed_scale: 0.8, player_sprint_speed_scale: 1.5, player_last_stand_crawl_speed_scale: 0.15, weapon_move_speed_scale, weapon_ads_move_speed_scale, shellshock_affects_movement: false })` (values from iw4L `sim/src/step.rs:2441`). Doom never advances MW2's `bob_cycle`, and placement's movement bob (sheet 10) reads it. Because `TicInput::velocity_mw2` is pre-scaled by `SHADOW_VELOCITY_SCALE` (00 §5.1), a Doom player running flat out produces MW2's running bob rate, not 3.8× it.
3. **Sprint.** `movement_iw4::update_sprint(&mut ps, &ucmd, old_buttons, SprintContext { weapon_max_sprint_time: get_max_sprint_time(facts.movement.sprint_duration_scale, 4.0), sprint_forever: false, min_sprint_time_seconds: 1.0, sprint_delay_seconds: 0.0, sprint_forward_minimum: 105, stand_up_clear: true, sprint_recharge_pause_seconds: 0.0 })` — values from iw4L `sim/src/step.rs:2471`. Keep the returned `SprintResult` for logging.
4. **ADS intent.** `update_ads_intent(&mut ps, &ucmd, old_buttons, AdsIntentContext { ads_allowed, weapon_def_scope: fpv.overlay_reticle != 0, sprint_hold_ads: false })` with `ads_allowed = weapon_iw4::is_ads_allowed(&ps, &combat.ads_allow_facts(), &ps.ammoclip, 0)`. Because this sim keeps clips in `hands[0].clip`, write the held weapon's clip into `ps.ammoclip` with `weapon_iw4::set_clip_for_hand` before the call (and `set_ammo_not_in_clip` for stock) so `no_ads_when_mag_empty` works.
5. **ADS frac.** `update_ads_frac(&mut ps, msec, AdsFracContext { aim_down_sight, ads_reload_trans_time_ms, segmented_reload, ads_in_rate, ads_out_rate, rechamber_while_ads, ads_fire_only })` from combat facts.
6. **Hold breath.** `breath::update_hold_breath(&mut ps, ucmd.buttons, msec, fpv.can_hold_breath)` (ported, §5.7).
7. **Aim spread.** Exactly as combat.rs:218–245: `adjust_aim_spread_scale(&mut AimSpreadState{..from ps}, &combat.spread_facts(), &combat.aim_spread_decay_facts(), ps.ground_entity_num, ps.pm_type, ps.e_flags, ps.f_weapon_pos_frac, &AimSpreadMotion { frametime: msec as f32 / 1000.0, cmd_angles: ucmd.angles, old_angles: last ucmd.angles, forwardmove, rightmove, velocity_xy, speed: ps.speed, move_speed_threshold: AIM_SPREAD_MOVE_SPEED_THRESHOLD_DEFAULT })`; write back `aim_spread_scale`, `spread_override_state`.
8. **Weapon command.** Build `WeaponCmd` field by field following combat.rs:300–515 for the single-hand, no-offhand case: `msec`, `server_time`, `buttons = ucmd.buttons`, `old_buttons`, `cmd_weapon` (switch target from §5.8 or 0), `pm_flags = ps.pm_flags`, `weap_flags = ps.weap_flags`, `pm_type`, `e_flags`, `last_weapon_hand = 0`, `f_weapon_pos_frac`, `is_in_air = !on_ground`, `melee_charge` from ps, `cmd_weapon_owned`, `cmd_weapon_pistol_quick = target.weap_class == WEAPCLASS_PISTOL`, `switch_raise_time_ms`, `switch_quick_raise_time_ms`, `switch_first_raise_time_ms` (0 when `used` contains the target), `offhand: OffhandCmd::default()`, perks 0, `perk_weap_reload_multiplier: PERK_WEAP_RELOAD_MULTIPLIER_DEFAULT`, other fields default. Hand 0 state comes from `self.hands[0]` with `clip`/`stock` from §5.9.
9. **Run.** `let events = weapon_hands(&mut self.hands, &combat, &mut wcmd, 0);` (the facts passed are the **held** weapon's facts, as combat.rs does via `facts_weapon`).
10. **Write back** `ps.weapon`, `weaponstate_primary`, `weapon_time`, `weapon_delay`, `weap_anim`, `weapon_restrict_kick_time`, `pm_flags = wcmd.pm_flags`, `weap_flags = wcmd.weap_flags`, `melee_charge_*` (combat.rs:560–585), and `weapon_iw4::update_scope_zoom(&mut ps, ucmd.buttons, old_buttons, combat.scope_zoom)`.
11. **Events** → `SimEvent`s (§5.4–5.5).
12. **Kick and sway advance.** `kick.advance(&facts.kick, ads_frac, msec / 1000)`, `sway.advance(facts.sway.hip, facts.sway.ads, ps.viewangles, ads_frac, combat.aim_down_sight, fpv.overlay_reticle != 0, 1.0, msec / 1000)`.
13. **Placement** (sheet 10 §5.3): `let placed = placement::step(&mut self.placement, &ViewInputs { ps: &ps, facts, sway: self.sway.springs(), gun_recoil: self.kick.gun, msec })`; `self.gun_angle_offset = placement::gun_angle_offset(&placed, ps.viewangles, facts)`.
14. **Outputs.** Fill `TicOutput` and `last_view: ViewState` (§5.11); `old_buttons = ucmd.buttons`.

All of steps 1–14 run even when no weapon is held (empty hands), except 9–11, which are skipped when `hands[0].weapon == 0 && cmd_weapon == 0`.

### 5.4 Shot handling (`WeaponTickEvent::ShotAccepted { ammo_used }`)

Mirror combat.rs:729–815:

1. Seed recoil: `kick.seed_fire(&facts.kick, ads_frac, ps.weap_flags, 0, hands[0].weapon_restrict_kick_time > 0)` (the ported function draws its four samples from `DmwRng`, not its private xorshift, so all randomness is reproducible from the level seed).
2. `add_aim_spread_fire(&mut ps.aim_spread_scale, ads_frac, combat.hip_spread_fire_add)`.
3. `cone = get_spread_for_weapon(ps.view_height_current, ps.spread_override, SpreadOverrideState::from_i32(ps.spread_override_state), &combat.spread_facts(), 1.0)`; `spread_deg = fire_weapon_spread_degrees(cone, combat.ads_spread, ads_frac, ps.aim_spread_scale)`.
4. Shot angles: `[pitch + clamp(gun_angle_offset[0], ±45), yaw + clamp(gun_angle_offset[1], ±45), 0]` (combat.rs:769–775).
5. Pellets: `barrels = if fire_type == DoubleBarrel { ammo_used.max(1) } else { 1 }`; `n = clamp(combat.pellet_count() * barrels, 1, 64)`; each `dir = spread::spread_direction_on_plane(angles, spread_deg, &mut rng, 1.0)` (port of combat.rs:1125, using `DmwRng::next_u32` for both draws, same formula).
6. `projectile = matches!(fire_weapon_kind(weap_type, weap_class), Some(FireWeaponKind::Missile | FireWeaponKind::GrenadeLauncher))`.
7. Emit `SimEvent::Shot(ShotRequest { weapon, pellets, base_damage: combat.damage, max_range_mw2: combat.bullet_range(), projectile })`, `SimEvent::MuzzleFlash { weapon, last_shot: hands[0].clip == 0 && fire_type != DoubleBarrel }`, and `SimEvent::Sound(SoundCue { alias: sounds.fire_player.or(sounds.fire), local: true })` (and `last_shot` variants if the facts have them — use the `WeaponSoundSlot` names present in 03).

### 5.5 Other events

| `WeaponTickEvent` | `SimEvent`(s) |
|---|---|
| `EmptyClick` | `EmptyClick`, `Sound(empty_fire_player or empty_fire)` |
| `ReloadStarted` | `ReloadStarted { empty: hands[0].clip == 0 }` (reload audio comes from clip notetracks, sheet 10) |
| `ReloadInsert` | none (notetracks carry the sound) |
| `ReloadAmmoAdded { shells }` | none (bookkeeping already in `hands[0].clip/stock`) |
| `ReloadEnded` | `ReloadFinished` |
| `RaiseStarted` | `RaiseStarted(id)`; insert into `used` |
| `RaiseFinished` | none |
| `DropFinished` | `DropFinished(id)`; see §5.8 |
| `PutawayStarted` | none |
| `MeleeFired` | `Melee(MeleeRequest { weapon, damage: combat.melee_damage, range_mw2: wcmd.player_melee_range, dir: forward(viewangles) })`, `Sound(melee_swipe_player)` |
| `RechamberWeapon`, `EjectBrass` | none in this build (brass is out of scope) |
| `StunnedStarted`, `Detonated`, `Offhand*`, `AlternateStarted` | ignored, `log_once!` debug |

### 5.6 Spread for the crosshair

`TicOutput::spread_deg` and `ViewState::spread_deg` = `fire_weapon_spread_degrees(cone, ads_spread, ads_frac, ps.aim_spread_scale)` computed every tic (not only on fire), so the crosshair (12) blooms and recovers continuously.

### 5.7 Breath port

`breath.rs` ports `update_hold_breath` verbatim (35 lines, Apache header per 01 §5.10). It needs `playerstate_iw4::{BREATH_GASP_TIME_MS, breath_hold_time_ms, weap_flags}` which are public. Effect: `ps.hold_breath_scale` scales idle sway amplitude in placement (10 §5.3 multiplies `idle.ads_idle_amount` by it, as `apply_fpv_placement` does).

### 5.8 Weapon switching and the vanilla handoff

Doom decides *which* weapon (slot keys, pickups, out-of-ammo auto-switch in 08); MW2 decides *how long* the switch takes.

```rust
enum SwitchState {
    Idle,
    ToMw2 { target: DmwWeaponId },                 // weapon_iw4 is dropping current / raising target
    ToVanilla { doom_weapon: DoomWeapon },         // dropping MW2 weapon; then Doom raises its psprite
    VanillaHeld,                                   // a vanilla weapon is up; hands[0].weapon == 0
}
```

- `TicInput::requested_weapon = Some(t)` with `t != held`: `cmd_weapon = t` until `ps.weapon == t` (weapon_iw4 performs drop → raise using `drop_time_ms`, `raise_time_ms`, `first_raise_time_ms`, `quick_*`).
- Request for a vanilla slot (08 passes `requested_weapon = None` and calls `sim.request_vanilla(doom_weapon)`): `cmd_weapon = 0`… weapon_iw4 has no "switch to nothing" command, so the sim drops by running the current hand into `Dropping` (set `hands[0].weaponstate = Dropping`, `weapon_time = drop_time_ms`, `start_weapon_anim(DROP)`, exactly what `weapon_change.rs` does when beginning a switch — reuse its public helper if one exists, else replicate its three assignments and cite the line). On `DropFinished`, set `hands[0] = WeaponHandState::default()`, `SwitchState::VanillaHeld`, `TicOutput::weapon_lowered_for_vanilla = true` for that tic; 08 then lets Doom's `P_BringUpWeapon` run.
- From `VanillaHeld` to an MW2 weapon: 08 lowers the Doom psprite first (vanilla `A_Lower`), then passes `requested_weapon = Some(t)`; with no hand up, the sim spawns the hand with `spawn_weapon_hand(t, facts, first = !used.contains(t))` and lets it raise.
- `EXT_BT_SWAP_LAST` is resolved by 08 into a request for `inventory.last_held`.
- Death: on `!alive`, `pm_type` dead; weapon_iw4 handles it (weapon goes idle, no fire). On respawn (`start_level` or 08's `on_respawn`), `used` is cleared (MW2 replays first-raise per life).

### 5.9 Ammo bookkeeping

- Doom pools are the reserve: before step 8, `hands[0].stock = input.pools[ammo_of(held)]` where `ammo_of` is the mapping from 08 (passed in `LevelStart` as `ammo_map: BTreeMap<u16, DoomAmmo>`; add this field — see §11). `hands[0].clip = inv.magazines[held]`.
- After step 10: `out.pools[ammo_of(held)] = hands[0].stock`; `inv.magazines[held] = hands[0].clip`.
- `max_ammo`/`start_ammo` in combat facts are **not** used for capacity (Doom pool maxima from 00 §9.2 rule); `start_ammo` is ignored; new weapons get a full magazine on pickup (08 §5.4).
- `ammo_per_shot`: weapon_iw4 subtracts from `hands[0].clip` itself; the pool is touched only by reloads, so "1 round per shot" (00 §9.2) holds automatically.

### 5.10 RNG

```rust
pub struct DmwRng(u32);
impl DmwRng {
    pub fn seeded(skill: i32, episode: i32, map: i32) -> Self;  // splitmix32 of (skill<<16 ^ episode<<8 ^ map), never 0
    pub fn next_u32(&mut self) -> u32;                         // xorshift32 13/17/5, as iw4L's ViewKickState
    pub fn unit01(&mut self) -> f32;                           // next_u32 as f32 / u32::MAX as f32
}
```

Single stream, used in this order every tic: kick samples (4 per shot), then pellet draws (2 per pellet). Any other consumer must take from the same stream in a documented order or get its own seeded stream (sheet 10 does not need randomness).

### 5.11 Outputs

`TicOutput` per 00 §7.2 plus:

| Field | Rule |
|---|---|
| `view_kick_deg` | `kick.kick_angles` |
| `move_speed_scale` | `held.movement.move_speed_scale × lerp(1, held.movement.ads_move_speed_scale, ads_frac) × (sprint_speed_scale if sprinting)`; 1.0 with no MW2 weapon; `[balance] weapon_move_scales = false` drops the first two factors |
| `sprinting` | `ps.pm_flags & pm_flags::SPRINTING != 0` |
| `ads_frac` | `ps.f_weapon_pos_frac` |
| `held` | `Some(id)` if `hands[0].weapon != 0` |

`ViewState` additionally:

| Field | Rule |
|---|---|
| `weaponstate`, `weap_anim` | from `hands[0]` |
| `placement` | step 13 result |
| `overlay_active` | `fpv.overlay_reticle != 0 && ads_frac == 1.0` |
| `zoom_hfov_deg` | `None` when `ads_frac == 0` or `!aim_down_sight`. Otherwise MW2 ADS FOV `f = if overlay && scope_zoom.levels() > 0 { scope_zoom.fov(ps.scope_zoom_level) } else { fpv.ads_zoom_fov }`; zoom progress `z = clamp((ads_frac − ads_zoom_in_frac) / (1 − ads_zoom_in_frac), 0, 1)` while ADS-ing in (use `ads_zoom_out_frac` while coming out, i.e. when ADS intent is off); MW2→Doom: `d = 2·atan(tan(45°)·tan(f/2)/tan(MW2_CG_FOV/2))`; result `Some(lerp(90, d, z))` |
| `viewmodel_fov_deg` | `lerp(LevelStart::viewmodel_fov, f, z)` with `f`, `z` as for `zoom_hfov_deg` (MW2 space, not Doom space); `viewmodel_fov` when not aiming (00 Revision 6) |
| `clip`, `stock` | `hands[0]` |

### 5.12 Save and restore

`save()` returns `SimSave` (00 §7.2) with weapon **names** (`pack.facts(id).name`) for `owned`, `magazines`, `last_held`, `held`. `restore(save)` maps names back through `Pack::weapon_id` (unknown names dropped with `warn!`), rebuilds `Inventory`, and puts the held weapon up in `Ready` state (`spawn_weapon_hand(id, facts, false)` then force `weaponstate = Ready`, `weapon_time = 0`). Kick, sway, spread, ADS fraction, breath, sprint and switch state reset to rest; `used` is cleared.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Requested weapon not owned | ignored, `debug!` |
| Facts with `fire_type` unknown | impossible (03 validation); `debug_assert!` |
| Pool type for a weapon unknown | treated as `am_noammo` (infinite stock 0) and `log_once!` warn |
| `msec` sequence | always 28/29 (00 §5.3); a test asserts `Σ tic_msec(0..35) == 1000` |
| Very long tic gaps (pause) | none: Doom does not advance tics while paused |
| Player dead mid-reload | weapon_iw4 state frozen; reload resumes or resets per weapon_iw4 on respawn (fresh `start_level`/`on_respawn` resets hands to raising) |

## 7. Logging

`dmw::sim`: `debug!` per shot (`weapon, pellets, spread_deg, clip`), reload start/end, raise/drop, switch state transitions; `trace!` per tic `weaponstate, weapon_time, ads_frac, aim_spread_scale, pm_flags`.

## 8. Licensing and asset notes

`view_kick.rs`, `view_sway.rs`, `breath.rs`, and the `spread_direction_on_plane` port carry the Apache header (01 §5.10) and NOTICE rows.

## 9. Tests (all with the fixture pack; MW2-free)

| Id | Test |
|---|---|
| T7.1 | **Fire rate**: hold attack with `fixture_rifle` (fire_time 86 ms) for 35 tics after raise → shots at the tics predicted by integrating `tic_msec` against 86 ms (expected count 11 or 12; exact tic list in golden `sim_fire_rifle.json`) |
| T7.2 | **Semi-auto**: holding attack with `fixture_pistol` fires once; re-press fires again only after `fire_time` |
| T7.3 | **Reload timing**: empty mag, press reload → `ReloadStarted{empty:true}` then magazine refilled exactly when Σmsec ≥ `reload_empty_time_ms` (± one tic); pool decreased by the rounds added |
| T7.4 | **Segmented reload** (`fixture_shotgun`): start → inserts every `reload_add_time_ms` → end; firing during inserts interrupts per weapon_iw4 |
| T7.5 | **Bolt action** (`fixture_sniper`): fire → rechamber state for `rechamber_time_ms`; second shot impossible before |
| T7.6 | **Spread bloom/decay**: hip-fire 10 rounds → `spread_deg` rises toward `hip_spread_stand_max`; release → decays per `hip_spread_decay_rate`; ADS → `ads_spread` |
| T7.7 | **Pellets**: shotgun shot produces exactly `pellet_count` unit vectors within `spread_deg` of the aim direction (angle check) |
| T7.8 | **Determinism**: same input script twice → identical `TicOutput` sequence (serde-JSON compare) |
| T7.9 | **ADS**: holding ADS raises `ads_frac` 0→1 in `1/ads_in_rate` ms ± one tic; `zoom_hfov_deg` reaches the converted FOV; release lowers it |
| T7.10 | **Sprint**: sprint + forward 50 → `sprinting` after one tic; attack during sprint → no shot until sprint-out completes (`sprint_drop_time_ms`) |
| T7.11 | **Switch**: rifle → pistol takes `drop_time + raise_time` (first raise uses `first_raise_time`); to vanilla: `weapon_lowered_for_vanilla` exactly once after `drop_time` |
| T7.12 | **Kick**: firing produces non-zero `view_kick_deg` within MW2 kick ranges and decays to < 0.01° within 1 s of release |
| T7.13 | `tic_msec` sums to 1000 per 35 tics; `server_time` monotonic |
| T7.14 | **Parity check vs iw4L** (doc test style): for `fixture_rifle`, the sequence of `weaponstate` values over a scripted 3 s run equals the sequence produced by calling `weapon_iw4::weapon_hands` directly with hand-built `WeaponCmd`s at the same msec steps (guards against mistakes in `WeaponCmd` construction) |

## 10. Acceptance criteria

- [ ] 00 §7.2 sim API implemented with the exact names; `#![forbid(unsafe_code)]`.
- [ ] Every step of §5.3 implemented in order, each with a comment citing the iw4L line it mirrors.
- [ ] T7.1–T7.14 pass; goldens committed.
- [ ] Ported files carry headers and NOTICE rows.
- [ ] `WeaponSim::tic` + placement ≤ 0.2 ms (01 §5.8) on the fixture rifle in release.

## 11. Contract changes requested

1. 00 §7.2 `LevelStart` needs `ammo_map: BTreeMap<u16, DoomAmmo>` (weapon id → Doom pool) so the sim can pick the reserve pool; and `WeaponSim` needs `request_vanilla(&mut self, doom_weapon: DoomWeapon)` and `on_respawn(&mut self)` (§5.8). MW2-to-MW2 switches keep using `TicInput::requested_weapon`. — Resolved in 00 Revision 2.
