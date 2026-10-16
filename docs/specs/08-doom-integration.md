# 08 — Doom Integration: Weapons, Inventory, Ammo, Movement, Saves

Owner: build agent (wave 4)
Status: implementation-ready
Depends on: 00 (§2 D12–D15, D19; §7; §9.1–9.3; Revision 2), 01 (§5.4–5.5), 02 (config), 05 (ext), 06 (freelook reset), 07 (`WeaponSim`)

This sheet wires `WeaponSim` into Doom: it owns the DOOMMW2 state singleton, startup (config + pack + palette), mode gating, the per-tic call from `P_PlayerThink`, the mapping between Doom weapon slots and MW2 weapons, ammo pools and pickups, the hand-off between MW2 viewmodels and vanilla psprites, movement speed scaling, and savegames. It also owns `room/src/dmw/hooks.rs`, the single file all Doom-side call sites delegate to.

---

## 1. Purpose

Make the game playable end to end in MW2 mode: start a level holding the mapped pistol, pick up a shotgun and have it raise with MW2 timing, fire, reload from the shell pool, switch to the fist and back, die and restart, save and load — all while vanilla mode (and demos) remain exactly room.

## 2. Scope / Out of scope

In scope: `room/src/dmw/{mod,mode,hooks,weapons,save}.rs`; Doom call sites for the 00 §7.5 rows owned by 08; weapon mapping and validation; inventory sync; ammo maxima, pickup scaling, backpack; auto-switch on empty; vanilla↔MW2 hand-off; movement scaling; muzzle-light; player sprite attack frame; noise alert; savegame extension; level/respawn/new-game lifecycle.

Out of scope: input (05), rendering (06, 11), hit resolution and damage (09 — called from here), animation (10), HUD and audio playback (12 — called from here).

## 3. Dependencies

| From | What |
|---|---|
| 00 §7.2 + Rev 2 | `WeaponSim`, `TicInput`/`TicOutput`, `LevelStart`, `request_vanilla`, `on_respawn` |
| 00 §7.4 | `PLAYER_EXT`, `ExtTiccmd` |
| 00 §9.1–9.3 | defaults |
| 02 §5.7 | `DmwConfig` |
| 05 | `ext::reset_level()`, `input` |
| 06 | `freelook::reset()` |
| 09 | `ballistics::fire(player, &ShotRequest)`, `ballistics::melee(player, &MeleeRequest)` |
| 10, 11 | `viewmodel::prepare_all(&state)`, `viewmodel::on_tic(&ViewState, msec) -> Vec<SoundCue>` |
| 12 | `audio::play(&SoundCue, origin)`, `hud::on_shot_result(..)`, `hud::reset()` |
| room facts | `P_PlayerThink` (p_user.rs:349): BT_CHANGE block sets `pendingweapon` (with chainsaw/SSG toggles and shareware plasma/BFG rule) before `P_MovePsprites`; `P_BringUpWeapon` (p_pspr.rs:183) sets the raise psprite for `pendingweapon`; `P_CheckAmmo` (p_pspr.rs:208) chooses the next weapon when out of ammo; `P_GiveAmmo` (p_inter.rs:369) scales by skill and clamps to `maxammo`; cheats set `weaponowned`/`ammo` directly (st_stuff.rs:624/634); `G_PlayerReborn` resets `maxammo` from the global table; `P_MovePlayer` (p_user.rs:232) thrusts `forwardmove*2048` and `sidemove*2048`; `SAVEGAME_EOF = 0x1d`; `P_WriteSaveGameEOF`/`P_ReadSaveGameEOF` (p_saveg.rs:1413/1435) |

## 4. File inventory

| Path | Content |
|---|---|
| `room/src/dmw/mod.rs` | `DmwState`, `with_state`, `log_once!` |
| `room/src/dmw/mode.rs` | D15 gating, `fail(reason)` |
| `room/src/dmw/hooks.rs` | every 00 §7.5 hook function (delegations) |
| `room/src/dmw/weapons.rs` | mapping, inventory sync, tic driver, hand-off, auto-switch, movement scale |
| `room/src/dmw/save.rs` | savegame extension |
| `room/src/doom/{d_main,g_game,p_user,p_pspr,p_inter,p_saveg}.rs` | one-line hook calls (§5.12 lists each) |
| `room/tests/integration_headless.rs` | §9 |

## 5. Detailed design

### 5.1 `DmwState`

```rust
pub struct DmwState {
    pub config: DmwConfig,
    pub config_path: Option<PathBuf>,
    pub pack: Option<Arc<Pack>>,
    pub palette: Option<Arc<PaletteTables>>,       // from the IWAD, sheet 11
    pub mapping: WeaponMapping,                    // §5.3
    pub sim: Option<WeaponSim>,
    pub level_active: bool,
    pub failed: Option<String>,                    // set by mode::fail; cleared at next level start
    pub viewmodels: viewmodel::Registry,           // sheet 10/11 per-weapon prepared data
    pub muzzle: MuzzleState,                       // extralight sequence + flash patch timer (§5.8)
    pub last_out: Option<TicOutput>,               // last tic's output, read by movement scaling and HUD
}
thread_local! { static DMW: RefCell<Option<DmwState>> = const { RefCell::new(None) }; }
pub fn with_state<R>(f: impl FnOnce(&mut DmwState) -> R) -> Option<R>;   // None before init
```

Re-entrancy: no hook may call another hook while holding the `RefCell` borrow. Hooks collect what they need, drop the borrow, then call Doom functions (`P_DamageMobj`, `S_StartSound`, …) — enforced by structuring each hook as "compute plan inside `with_state`, execute plan outside".

### 5.2 Startup (`on_engine_init`, in `D_DoomMain` after WAD init and before the title/`G_InitNew`)

1. `DmwConfig::load(-mw2config)`.
2. Pack: `-nomw2` → none. `-fixturepack` → `Pack::fixture()`. Else `-mw2pack` / `DMW_PACK` / `packs/mw2`; if that fails and `general.allow_fixture_pack` → `packs/fixture`. A failure logs `warn!` with the `PackError` and continues without a pack.
3. Palette: `PaletteTables::from_wad_lumps(PLAYPAL, COLORMAP)` via `W_CacheLumpName`.
4. Mapping (§5.3).
5. `WeaponSim::new(pack)`.
6. `info!` summary: pack, mapping table (slot → name/vanilla), config path.

### 5.3 Weapon mapping

```rust
pub struct WeaponMapping {
    pub by_doom: [Option<DmwWeaponId>; NUMWEAPONS],     // None = vanilla
    pub doom_of: BTreeMap<u16, DoomWeapon>,             // reverse (first Doom weapon wins)
    pub ammo_of: BTreeMap<u16, DoomAmmo>,               // = weaponinfo[doom_weapon].ammo
}
```

For each Doom weapon `w` with config name `n` (`[weapons]`, 00 §9.1): `"vanilla"` → `None`. Else `pack.weapon_id(n)` (aliases included); then require `facts.has_viewmodel`, `fire_weapon_kind(weap_type, weap_class)` ∈ {Bullet, Missile, GrenadeLauncher}, and `pack.load_viewmodel(id)` succeeding (done lazily at level start; failure there demotes the slot to vanilla for the session with `error!`). A weapon with no Doom ammo type (fist, chainsaw slots) mapped to an MW2 gun gets `am_clip`.

Two Doom weapons may map to the same MW2 weapon; they then share the magazine.

### 5.4 Inventory sync

Doom's `weaponowned[]` and `ammo[]` stay authoritative for ownership and pools (they are what pickups, cheats, the status bar and savegames already use).

`sync_ownership(player)` at the start of every MW2 tic: for each Doom weapon `w` owned and mapped to `id`: if `id` not in `sim.inventory().owned` → add it with a full magazine (`combat.clip_size`) and, unless this is the first tic of the level, call `hud::on_weapon_acquired(id)` (12 §5.5 pickup message). This covers pickups, `IDKFA`/`IDFA`, and level-start inventories with one rule. `ViewState::gun_angle_offset` (00 Revision 7) feeds 12's crosshair and the scope-sway rule in §5.5.

Pickup ammo scaling (`scale_ammo_pickup(ammo, num)`, called at the top of `P_GiveAmmo` when `level_active`): `num = ceil(num * pickup_scale[ammo])` (min 1 if `num > 0`). Doom's skill doubling (baby/nightmare) still applies after.

Maxima: at `on_level_start` and when a backpack is picked up (detected in `sync_ownership` by `player.backpack` turning true), set `player.maxammo[i] = cfg.max[i] * (if backpack {2} else {1})` and clamp `ammo[i]`. When `level_active` is false, restore vanilla maxima (`maxammo[]` global table, doubled with backpack).

### 5.5 Per-tic driver (`on_player_think(player) -> bool`)

Called in `P_PlayerThink` immediately before `P_MovePsprites(player)` (after Doom's own BT_CHANGE and BT_USE handling). Returns `true` when MW2 owns the weapon this tic (P_PlayerThink then skips `P_MovePsprites`).

```text
if !level_active → return false
sync_ownership(player)
target = decide_target(player)                    // §5.6
if vanilla weapon is ready and no MW2 target pending → return false (Doom psprites run normally)
input = TicInput {
  tic: leveltime, alive: playerstate == PST_LIVE,
  buttons: decode(PLAYER_EXT[i].cmd.buttons),
  requested_weapon: target.mw2(),
  yaw_deg, prev_yaw_deg (PLAYER_EXT.prev_yaw), pitch_deg, prev_pitch_deg,
  origin_mw2: to_mw2(mo.x, mo.y, mo.z + viewheight),
  velocity_mw2: [momx, momy, momz] → in/s × SHADOW_VELOCITY_SCALE,
  on_ground: mo.z <= mo.floorz,
  forwardmove: cmd.forwardmove, sidemove: cmd.sidemove,
  pools: player.ammo,
}
out = sim.tic(&input)
player.ammo = out.pools (clamped to maxammo)
reflect held weapon into Doom: readyweapon = mapping.doom_of[held] (when held), pendingweapon = wp_nochange once the switch completed
psprites[ps_weapon].state = null; psprites[ps_flash].state = null   (MW2 weapon held: nothing vanilla may draw)
PLAYER_EXT[i].view_kick_deg = out.view_kick_deg (+ [gun_angle_offset pitch, yaw, 0] while view.overlay_active — scope sway moves the world, sheet 12 §5.4); zoom_hfov_deg = view.zoom_hfov_deg; prev_yaw/prev_pitch updated
for ev in out.events: dispatch (§5.7)
if out.weapon_lowered_for_vanilla: readyweapon = target vanilla weapon; P_BringUpWeapon(player) (vanilla path) → return false next tic
cues = viewmodel::on_tic(&sim.view_state(), tic_msec(leveltime)); audio::play(each)   (notetrack sounds, sheet 10)
last_out = Some(out)
return true
```

`yaw_deg` from `mo->angle` via 00 §5.1; `pitch_deg = pitch_q / 128`.

### 5.6 Choosing the target weapon

Doom's BT_CHANGE block has already computed `player.pendingweapon` (including the chainsaw-on-fist and SSG-on-shotgun toggles, and the shareware rule). This sheet only adds:

- `EXT_BT_SWAP_LAST` → `pendingweapon = doom_of[inventory.last_held]` if owned.
- Auto-switch on empty: when an `EmptyClick` event occurs and both magazine and pool of the held weapon are 0, choose the next weapon with ammo using `P_CheckAmmo`'s exact priority order (plasma, SSG, chaingun, shotgun, pistol, chainsaw, missile, BFG, fist — for MW2-mapped weapons "has ammo" = magazine > 0 or pool > 0) and set `pendingweapon`.
- Translation: `pendingweapon` mapped → `TicInput::requested_weapon = Some(id)`; unmapped (vanilla) while an MW2 weapon is held → `sim.request_vanilla(pendingweapon)` once.

Vanilla → MW2 (`on_bring_up_weapon(player) -> bool`, hook at the start of `P_BringUpWeapon`): when `level_active` and `pendingweapon` is mapped, set `readyweapon = pendingweapon`, clear both psprites, remember the MW2 target so the next `on_player_think` passes it as `requested_weapon`, and return `true` (skip the vanilla raise). Doom has already lowered the vanilla weapon (`A_Lower` calls `P_BringUpWeapon` at the bottom), so the visual sequence is: Doom sprite lowers → MW2 viewmodel raises with MW2 timing. At level start `P_SetupPsprites` → `P_BringUpWeapon` takes the same path.

### 5.7 Event dispatch

| `SimEvent` | Action |
|---|---|
| `Shot(req)` | `P_SetMobjState(mo, S_PLAY_ATK2)`; muzzle (§5.8); `ballistics::fire(player, &req)` (09); if `alert_monsters_on_fire` → `P_NoiseAlert(mo, mo)` |
| `Melee(req)` | `P_SetMobjState(mo, S_PLAY_ATK1)`; `ballistics::melee(player, &req)` (09) |
| `Sound(cue)` | `audio::play(&cue, player mo)` (12) |
| `MuzzleFlash { .. }` | §5.8 |
| `EmptyClick` | auto-switch check (§5.6) |
| `ReloadStarted`/`ReloadFinished`, `RaiseStarted`/`DropFinished` | `debug!` only |

### 5.8 Muzzle light and flash

Doom brightens the view on fire through `player.extralight` (A_Light1/A_Light2 then A_Light0 in the weapon states). MW2 mode reproduces it: on `MuzzleFlash`, `extralight = 2` for 1 tic, then 1 for 1 tic, then 0 (`MuzzleState { tics_left, flash_tics }`), advanced at the end of each MW2 tic. `flash_tics = 2` drives sheet 11's flash patch.

### 5.9 Movement scaling (`scale_move`, in `P_MovePlayer`)

`P_MovePlayer` runs before `on_player_think` in the same tic, so it uses `last_out` (one tic old — acceptable; documented). When `level_active` and `last_out` is `Some(out)`:

```text
forward_thrust = cmd.forwardmove * 2048 * out.move_speed_scale     (fixed, rounded toward zero)
side_thrust    = cmd.sidemove    * 2048 * (out.move_speed_scale / sprint_factor_if_sprinting)   (sprint only boosts forward)
```

Doom's own back/strafe behaviour is otherwise untouched (no MW2 back/strafe penalties — Doom movement stays Doom).

### 5.10 Lifecycle

| Event | Action |
|---|---|
| `G_InitNew` (new game) | `sim.inventory_mut()` cleared (fresh pistol start) |
| `G_DoLoadLevel` end (`on_level_start`) | evaluate `level_active` (D15 + `failed` cleared); if active: `ext::reset_level()`, `freelook::reset()`, `hud::reset()`, maxima (§5.4), `viewmodel::prepare_all` (first time per session, cached), `sim.start_level(LevelStart { pack, seed: DmwRng seed from (gameskill, gameepisode, gamemap), inventory: current, ammo_map, sprint_speed_scale, weapon_move_scales })`; if inactive: restore vanilla maxima, `freelook::reset()` |
| Player reborn (single player death restarts the level via `G_PlayerReborn` → `G_DoLoadLevel`) | Doom resets `weaponowned`/`ammo` to pistol start; `sync_ownership` rebuilds; `sim.on_respawn()`; magazines of no-longer-owned weapons are dropped |
| `mode::fail(reason)` | `error!` once, `level_active = false`, psprites restored by calling `P_SetupPsprites(player)` (vanilla raise of `readyweapon`, which is always a valid Doom weapon), freelook reset |

### 5.11 Savegame extension (`save.rs`)

- Save (`save_extension()`): called in `G_DoSaveGame` right after `P_WriteSaveGameEOF()` and before the file is closed. Writes `b"DMW1"`, `u32` LE length, postcard of:

```rust
#[derive(Serialize, Deserialize)]
struct SaveBlock { version: u32 /* 1 */, pitch_q: i32, level_active: bool, sim: dmw_weapons::SimSave }
```

`SimSave` (00 §7.2, implemented by 07 §5.12; 08 needs only its serde): inventory (owned, magazines, last_held) and held weapon. Kick, sway, spread, ADS and switch state are not saved (reset to rest on load).
- Load (`load_extension()`): right after `P_ReadSaveGameEOF()` succeeds in `G_DoLoadGame`, read 4 bytes; if they are `DMW1`, read the block and `sim.restore`; otherwise (vanilla save or EOF) rebuild inventory with full magazines via `sync_ownership`. A malformed block → `warn!`, treat as vanilla.
- Vanilla room must still load DOOMMW2 saves: it stops reading at `SAVEGAME_EOF`, so trailing bytes are ignored (verify room's loader does not check file length; if it does, file a contract change).

### 5.12 Call sites (exact)

| File | Function | Edit |
|---|---|---|
| d_main.rs | `D_DoomMain` | after `W_*` init and `R_Init`, before the title loop: `crate::dmw::hooks::on_engine_init();` |
| g_game.rs | `G_DoLoadLevel` | last statement: `crate::dmw::hooks::on_level_start();` |
| g_game.rs | `G_InitNew` | last statement: `crate::dmw::hooks::on_new_game();` |
| g_game.rs | `G_DoSaveGame` / `G_DoLoadGame` | after EOF write / read: `save_extension()` / `load_extension()` |
| p_user.rs | `P_PlayerThink` | `if !crate::dmw::hooks::on_player_think(player) { P_MovePsprites(player); }` |
| p_user.rs | `P_MovePlayer` | thrust values passed through `crate::dmw::hooks::scale_move(player, f, s)` |
| p_pspr.rs | `P_BringUpWeapon` | first statement: `if crate::dmw::hooks::on_bring_up_weapon(player) { return; }` |
| p_inter.rs | `P_GiveAmmo` | first statement: `let num = crate::dmw::hooks::scale_ammo_pickup(ammo, num);` |

### 5.13 Test hooks (`test-hooks` cargo feature, off by default)

For sheet 13's harness: a ring buffer of the last 256 `SimEvent`s with their tic, and read accessors (`dmw::test_api::{events, view_state, mapping}`), all behind `#[cfg(feature = "test-hooks")]`. Nothing in the shipped binary changes.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| No pack | `level_active = false` always; `info!` once at startup |
| Mapped weapon missing from pack | slot vanilla, `warn!` naming slot and name, alias suggestions from `Pack` names |
| Viewmodel load fails at level start | that slot demoted to vanilla for the session, `error!` |
| Panic-free | every hook returns its "not handled" value on any internal error and calls `mode::fail` |
| Demo playback/recording starts mid-session | next `on_level_start` sees it and stays inactive (D15) |
| `pendingweapon` points to an unowned weapon | ignored (Doom's own rule) |
| Shared magazine (two slots → one MW2 weapon) | one magazine, documented |
| Savegame from another pack (weapon names differ) | `SimSave` stores names, not ids; unknown names dropped with `warn!` |

## 7. Logging

`dmw::sim` (driver lines), `dmw::save`: `info!` level mode on/off with reason; `info!` mapping table at startup; `debug!` switches, auto-switch, pickups scaled; `warn!`/`error!` per §6.

## 8. Licensing and asset notes

None beyond 01.

## 9. Tests (headless, fixture pack, Freedoom; harness from 13 §5.2)

| Id | Test |
|---|---|
| T8.1 | E1M1 `-fixturepack`: level starts with `fixture_pistol` raising (`RaiseStarted`), `readyweapon == wp_pistol`, psprites null |
| T8.2 | fire 5 times → magazine 12→7, pool unchanged; reload → magazine 12, pool −5 |
| T8.3 | give shotgun (simulate pickup via `P_GiveWeapon`) → `sync_ownership` adds `fixture_shotgun` with full magazine; Doom auto-switch → MW2 drop+raise timing |
| T8.4 | switch to fist (key 1) → MW2 drop, then vanilla fist psprite raises; back to pistol → fist lowers, MW2 raises |
| T8.5 | empty everything → auto-switch follows Doom priority |
| T8.6 | `IDKFA` cheat → all mapped weapons owned with full magazines, pools at DOOMMW2 maxima |
| T8.7 | save/load round trip: magazines and held weapon restored; vanilla room build (or a parser test) ignores trailing block |
| T8.8 | `-nomw2` and demo playback: hooks inert (counts of hook-handled tics == 0), `demo_playthrough` passes |
| T8.9 | movement: with `fixture_rifle` ADS fully up, forward speed after 35 tics ≈ vanilla × `ads_move_speed_scale` (± 3 %) |
| T8.10 | death → respawn: `on_respawn` called, first-raise replays, inventory pistol-only |

## 10. Acceptance criteria

- [ ] `task run -- -fixturepack -warp 1 1` is playable start to finish of E1M1 with fixture weapons (pickups, switching, reloads, fist), with no vanilla weapon sprite ever visible while an MW2 weapon is held.
- [ ] T8.1–T8.10 pass; `demo_playthrough` passes.
- [ ] Every Doom edit is exactly one of the §5.12 rows.
- [ ] No hook can panic (fuzz: random `TicInput` sequences into `on_player_think` via the harness for 10 000 tics without panic).

## 11. Contract changes requested

1. 00 §7.5: replace the `P_SetupPsprites` row by `P_BringUpWeapon → on_bring_up_weapon(player) -> bool`; remove the `P_GiveWeapon` row (ownership is synced each tic, §5.4); add `G_InitNew → on_new_game()`. — Resolved in 00 Revision 3.
2. 00 §7.2: `SimSave` must store weapon **names**, not ids (§6). — Resolved in 00 Revision 3.

### Build notes (sheet 08 implemented)

No change to a 00 contract is needed for the code. Requests and notes for the lead:

3. **Contract request (00 D18 / §4.4 / §8.3).** The ten DOOMMW2 bindings now live in `doommw2.toml` `[input]` (`key_dmw_*` as a key name, character or code; `mouseb_dmw_*` as button index; `mouse_dmw_look`/`mouse_dmw_invert` as booleans; `mouse_dmw_pitch_sens` 0..=50), per the lead decision. `on_engine_init` copies them into the `m_controls` statics. 00 D18, §4.4 (`default.cfg` row) and §8.3 still say `default.cfg`; please amend them.
4. **`P_GiveAmmo` hook placement.** Every pickup path in room calls the Rust body `p_give_ammo` directly, never the `extern "C"` wrapper `P_GiveAmmo`, so the hook is the first statement of `p_give_ammo` (the C `P_GiveAmmo` body). `num == 0` (Doom's "half a clip" for dropped weapons) is not scaled: the hook works on clip counts as §5.4 says, and half a clip has no clip count.
5. **Mode state lives in `mode.rs`, not `DmwState.level_active`.** Thread-local cells, so `mode::level_active()` works while `DmwState` is borrowed. The decision is tied to `levelstarttic`: during the next `P_SetupLevel` (before `on_level_start`) every hook is inert, which keeps demo starts exact. `G_DoPlayDemo` sets `demoplayback` only after `G_InitNew`, so the gate also treats `precache == 0` (set only by `G_DoPlayDemo` around `G_InitNew`) as demo playback. `on_player_think` also fails safe if a demo or netgame starts mid-level. `mode::weapons_active()` = real evaluation (pack loaded) and no `test_force_level_active(Some(false))`; `level_active()` keeps sheet 05's override semantics for input/freelook tests without a pack.
6. **Level start order.** `P_SetupPsprites → P_BringUpWeapon` runs inside `P_SetupLevel`, before `on_level_start`, so the vanilla raise happens first and `on_level_start` then takes over (clears psprites, requests the mapped weapon) when `readyweapon` is mapped. `on_new_game` runs after the new game's `on_level_start` (G_InitNew calls G_DoLoadLevel last); it clears the MW2 inventory and the first tic's sync refills it with full magazines.
7. **Respawn detection.** A level start on the same map as the previous active start (death, or new game/load on that map) starts from an empty inventory; `on_respawn` runs on the first tic unless `on_new_game`/a load follows. `sim.start_level` already resets the life, so `on_respawn` is belt and braces (counted in `test_api::respawns` for T8.10).
8. **Switch targets.** The driver remembers the Doom weapon chosen (`Driver::target_doom`) so two slots sharing one MW2 weapon (shotgun/SSG, chaingun/plasma) keep their Doom identity. `EXT_BT_SWAP_LAST` uses the driver's own history of `readyweapon` (vanilla weapons included) instead of `Inventory::last_held`, which only knows MW2 weapons.
9. **Vanilla psprite while dead (sheet 11).** `P_KillMobj → P_DropWeapon` sets the lowering psprite of `readyweapon` on death and `P_PlayerThink` returns before `on_player_think` while dead, so a mapped weapon's vanilla sprite can show during the death lowering. Sheet 11's `draw_player_weapon` should return `true` while `weapons::mw2_owns_weapon(state)` (helper provided), which hides it without another Doom edit.
10. **IDKFA and magazines.** Ownership sync gives new weapons a full magazine; magazines of weapons already owned are not refilled by the cheat (the pools are). T8.6 runs IDKFA from a pistol start.
11. **Ext attack bit (sheet 05 file).** `input::on_build_ticcmd` set `EXT_BT_ATTACK` on pause commands (`BT_SPECIAL | BTS_PAUSE`, bit 0). Now masked with `BT_SPECIAL`.
12. **Ballistics wiring (sheet 09).** Dispatch calls `ballistics::fire/melee`; `fire` makes the noise alert, so 08 does not. `on_level_start` calls `ballistics::configure(pack, balance)` and `ballistics::reset_level()` for active levels; `MISSILES` stays in `ballistics.rs`.
13. **Viewmodel (sheet 10/11).** `viewmodel::Registry` holds one `ViewmodelAnimator` per mapped weapon (`prepare_all`, once per session after 08 demotes slots whose viewmodel fails to load); `on_tic` advances the held weapon's animator outside the `DmwState` borrow and its notetrack cues go to `audio::play` (stub until sheet 12). Drawing is sheet 11's engine half.
14. **Savegames.** The block is written only while MW2 mode is active (otherwise saves are byte-identical to room's). Room's loader reads one byte after the archives and closes the file, so trailing bytes are ignored; T8.7 also loads a DOOMMW2 save with the weapon glue off. The block counts toward `vanilla_savegame_limit` when that is set (a few hundred bytes).
15. **Harness (sheet 13).** `support::ViewState`/`SimEvent` are the real types, `sim_view()`/`events()` read `dmw::test_api`, `clear_events()` added, the `-fixturepack`/`-nomw2` TODOs are gone. One-tic key presses can fall between two command builds in the harness; the sheet 08 tests hold keys for two tics.
16. **Tests.** `room/tests/integration_headless.rs` (T8.1–T8.7, T8.9, T8.10, demo half of T8.8), `nomw2_headless.rs` (`-nomw2` half of T8.8), `soak_headless.rs` (10 000 tics of random keys/mouse through the real input path, monsters alive, deaths and respawns included; asserts no psprite while an MW2 weapon is held by a live player and no `mode::fail`). `demo_playthrough` was not run: `doom1.wad` is a Git LFS pointer here.
17. **Not verified.** The acceptance item "`task run -- -fixturepack -warp 1 1` playable start to finish of E1M1" needs an interactive session and sheet 11's drawing (nothing draws the MW2 viewmodel yet); only the headless equivalents above were run.
