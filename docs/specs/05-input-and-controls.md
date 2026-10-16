# 05 — Input, Mouse Look and Controls

Owner: build agent (wave 2)
Status: implementation-ready
Depends on: 00 (§4.3, §5.1, §7.4, §7.5 rows for `G_BuildTiccmd`, `G_Responder`, `G_Ticker`, mouse release; §8.1, §8.3), 01, 02

`room` today has no mouse support at all ("No mouse support" in its README; `usemouse = 0`; the platform layer only queues keys). MW2 gunplay needs mouse aim, a vertical axis and four new actions (ADS, reload, sprint, melee). This sheet adds all input plumbing: winit mouse capture and raw motion, Doom `ev_mouse` events, pitch accumulation, the extended ticcmd side channel, and the new bindings.

---

## 1. Purpose

- Mouse turning (yaw) through Doom's existing `ev_mouse` path, so it also works in vanilla mode.
- Mouse pitch accumulated into `PLAYER_EXT[i].pitch_q` in MW2 mode, carried per tic through `ExtTiccmd` (never through `TiccmdT`, whose layout and demo encoding must not change).
- New actions bound in `default.cfg` and delivered as `EXT_BT_*` bits.
- Correct mouse capture/release around menus, pause, focus loss.

## 2. Scope / Out of scope

In scope: `room/src/main.rs` (window/device events, capture), `room/src/platform/mod.rs` (mouse queue), `room/src/doom/i_input.rs` (`I_GetEvent` posts `ev_mouse`), `room/src/doom/g_game.rs` (the three hook call sites in 00 §7.5 plus two read accessors), `room/src/doom/m_controls.rs` and `m_config.rs` (new variables), `room/src/dmw/input.rs`, `room/src/dmw/ext.rs`.

Out of scope: how pitch is rendered (06); what the buttons do (07/08); gamepad support; rebinding UI in Doom's menu (bindings are edited in `default.cfg`).

## 3. Dependencies

| From | What |
|---|---|
| 00 §7.4 | `ExtTiccmd`, `EXT_TICCMDS`, `ext_for_gametic`, `PlayerExt` (this sheet implements them) |
| 00 §8.1 | `EXT_BT_*` values |
| 00 §8.3 | new `default.cfg` variables and defaults |
| 08 | `crate::dmw::mode::level_active() -> bool` (D15). Until 08 lands, a stub returning `false` lives in `mode.rs` (02 created it) |
| room facts | `G_BuildTiccmd` (g_game.rs:991) adds `MOUSEY` to forward and `MOUSEX*8` to `angleturn`; `G_Responder` `ev_mouse` arm sets `MOUSEX/MOUSEY` from `data2/data3` scaled by `(mouseSensitivity + 5) / 10`; `I_GetEvent` (i_input.rs:137) only drains keys and breaks on the first key-up; `MAX_MOUSE_BUTTONS = 8`; `mousebstrafe = 1`, `mousebforward = 2` by default; ticcmd copy into `players[i].cmd` at g_game.rs:1525 with `buf = (gametic / ticdup) % BACKUPTICS`; `BACKUPTICS = 128` |

## 4. File inventory

| Path | Change |
|---|---|
| `room/src/main.rs` | handle `WindowEvent::{MouseInput, MouseWheel, Focused, CursorEntered}`, implement `ApplicationHandler::device_event` for `DeviceEvent::MouseMotion`, grab/release logic each `about_to_wait` |
| `room/src/platform/mod.rs` | `MOUSE` thread-local (`MouseAccum`), `pub(crate) fn take_mouse() -> Option<MouseSample>` |
| `room/src/doom/i_input.rs` | `I_GetEvent` posts at most one `ev_mouse` per call, **before** the key loop |
| `room/src/doom/g_game.rs` | hook calls in `G_Responder` (ev_mouse arm), `G_BuildTiccmd` (end), `G_Ticker` (after net cmd copy); `pub(crate) fn game_key_down(key: c_int) -> bool`, `pub(crate) fn mouse_button_down(n: c_int) -> bool` accessors |
| `room/src/doom/i_video.rs` | `usemouse` default `1` |
| `room/src/doom/m_controls.rs`, `m_config.rs` | 00 §8.3 variables, bound and persisted |
| `room/src/dmw/ext.rs` | 00 §7.4 complete |
| `room/src/dmw/input.rs` | `PitchAccum`, `build_ext()`, `on_mouse_event`, `on_ticcmd_copied`, sensitivity/zoom scaling |
| `room/tests/input_headless.rs` | §9 |

## 5. Detailed design

### 5.1 Platform: capture and raw motion

`MouseAccum { dx: f64, dy: f64, buttons: u8, wheel: i32, dirty: bool }` in a `thread_local!` in `platform/mod.rs`.

- `DeviceEvent::MouseMotion { delta: (dx, dy) }` (raw, unaccelerated) → `dx += delta.0; dy += delta.1` **only while grabbed** and `[input] raw_mouse = true`. If `raw_mouse = false`, use `WindowEvent::CursorMoved` deltas against the window centre and re-centre the cursor.
- `WindowEvent::MouseInput { state, button }` → bit in `buttons`: Left = bit 0, Right = 1, Middle = 2, Back = 3, Forward = 4. If the window is not grabbed, a left/right press grabs instead of being forwarded.
- `WindowEvent::MouseWheel` → `wheel += sign(lines)` (pixel deltas: one notch per 50 px).
- `WindowEvent::Focused(false)` → release grab, clear `buttons` (prevents stuck fire).

Grab policy, evaluated in `about_to_wait` before ticking:

```text
want_grab = focused && gamestate == GS_LEVEL && menuactive == 0 && paused == 0 && !console_like_state
```

On change: `window.set_cursor_grab(CursorGrabMode::Locked)` (fallback `Confined` if Locked fails, log the fallback once at `info`), `window.set_cursor_visible(false)`; release does the reverse. `[input] grab_on_start = false` defers the first grab until a click.

`take_mouse()` returns and clears the accumulated sample: integer counts (`dx.trunc()`, keeping the fractional remainder in the accumulator), buttons, and one pending wheel notch (wheel is delivered as a press of bit 5 (up) or 6 (down) in one sample and its release in the next).

### 5.2 `I_GetEvent` mouse posting

At the start of `I_GetEvent` (before the key loop, because the key loop `break`s on the first key-up):

```rust
if let Some(m) = crate::platform_bridge::take_mouse() {   // see note
    let ev = event_t { type_: 2 /* ev_mouse */, data1: m.buttons as c_int, data2: m.dx, data3: -m.dy, data4: 0 };
    D_PostEvent(&ev);
}
```

`data3 = -dy` matches Doom convention (mouse moved forward/up = positive). Only post when `dx != 0 || dy != 0 || buttons changed || wheel pulse`.

Note: `I_GetEvent` is in the library crate while `platform` is in the binary crate. Use the same pattern as `DG_GetKey`: add a C-ABI callback `DG_GetMouse(dx: *mut i32, dy: *mut i32, buttons: *mut u8) -> i32` exported by the binary's platform module and declared `extern "C"` in `i_input.rs`. The headless test harness (13) provides its own `DG_GetMouse`, exactly like it provides `DG_GetKey` today. `room/tests/demo_playthrough.rs` must keep linking: add a `DG_GetMouse` stub returning 0 to it — this is the only edit allowed in that file and it does not change its behaviour.

### 5.3 `G_Responder` and look

In the `ev_mouse` arm, after vanilla sets `MOUSEX`/`MOUSEY`:

```rust
if crate::dmw::hooks::mouse_look_active() {
    crate::dmw::input::on_mouse_event(ev);   // accumulates pitch from ev.data3
    MOUSEY = 0;                              // mouse Y no longer moves forward/back
}
MOUSEX = crate::dmw::input::scale_yaw(MOUSEX);  // ADS zoom sensitivity; identity when not zoomed
```

`mouse_look_active()` = `mode::level_active() && mouse_dmw_look != 0`. In vanilla mode nothing changes except that mouse X now actually arrives (room never posted it before).

Pitch accumulation (`PitchAccum` in `input.rs`, fractional, deterministic integer math):

```text
counts       = ev.data3 * (mouse_dmw_pitch_sens + 5) / 10      (same scaling Doom uses for MOUSEY)
counts       = -counts if mouse_dmw_invert == 0                 (mouse forward = look up = negative MW2 pitch)
deg_per_count = 8 * 360 / 65536                                 (identical to yaw: angleturn -= MOUSEX * 8)
pending_q   += counts * deg_per_count * PITCH_UNITS_PER_DEGREE * zoom_scale
```

Implement with an `i64` numerator over 65536 so no float drift: `pending_num += counts * 8 * 360 * 128 * zoom_scale_q16 >> 16`, and `pending_q = pending_num / 65536` carried with remainder.

`zoom_scale` (and `scale_yaw`) = `tan(zoom_hfov/2) / tan(DOOM_HFOV/2)` when `PLAYER_EXT[consoleplayer].zoom_hfov_deg` is `Some`, else 1 — the same "zoom sensitivity" idea as MW2's `zoom_sensitivity` (hud_iw4), applied relative to Doom's 90°.

### 5.4 `G_BuildTiccmd` hook (`on_build_ticcmd(cmd, maketic)`)

Called as the last statement of `G_BuildTiccmd`. Builds `ExtTiccmd` and stores it in `EXT_TICCMDS[maketic % BACKUPTICS]`:

```rust
ext.tic = maketic;
ext.pitch_delta = take_pending_q().clamp(i16::MIN as i32, i16::MAX as i32) as i16;
ext.buttons = EXT_BT_VALID
    | bit(EXT_BT_ATTACK, cmd.buttons & BT_ATTACK != 0)
    | bit(EXT_BT_RELOAD, key(key_dmw_reload))
    | bit(EXT_BT_ADS, key(key_dmw_ads) || mouse(mouseb_dmw_ads))
    | bit(EXT_BT_MELEE, key(key_dmw_melee) || mouse(mouseb_dmw_melee))
    | bit(EXT_BT_SWAP_LAST, key(key_dmw_swap));
let sprint_key = key(key_dmw_sprint);
if ads && sprint_key { ext.buttons |= EXT_BT_HOLD_BREATH } else if sprint_key { ext.buttons |= EXT_BT_SPRINT }
```

When `mode::level_active()`:

- Always-run: if the vanilla code computed `speed == 0` because `key_speed` (default Shift) is not held, recompute `forward`/`side` with `speed = 1`. Implement by calling `on_build_ticcmd` with the pre-clamp `forward`/`side` values: in this sheet, `G_BuildTiccmd` gets one more tiny edit — `let speed = speed || crate::dmw::hooks::force_run();` right after the vanilla `speed` computation. (Two call sites in one Doom function, both owned by this sheet.)
- `mousebstrafe`/`mousebforward` are ignored: wrap their uses in `G_BuildTiccmd` by a local `let mb = |n| if mw2 { -1 } else { n };` — again only this sheet edits `G_BuildTiccmd`.
- Mouse-button fire (`mousebfire`, default 0 = left) is unchanged.

The ext is built regardless of mode (cheap) so that a level that turns MW2 on mid-session never reads stale bits; consumers ignore it in vanilla mode.

### 5.5 `G_Ticker` hook (`on_ticcmd_copied(i)`)

Right after `std::ptr::copy_nonoverlapping(netcmds.add(i), cmd, 1)` and before demo read/write:

```rust
let ext = if i == consoleplayer as usize { ext_for_gametic(gametic, ticdup) } else { ExtTiccmd { tic: -1, ..Default::default() } };
PLAYER_EXT[i].cmd = ext;
if mode::level_active() { apply_pitch(i, ext.pitch_delta) }
```

`ext_for_gametic` returns the slot `(gametic / ticdup) % BACKUPTICS` if its `tic == gametic / ticdup`, else a neutral ext (`tic = -1`, no buttons, no pitch) and logs `debug!` once per level (`log_once!`).

`apply_pitch`: `prev_pitch_q = pitch_q; pitch_q = clamp(pitch_q + delta, ±PITCH_LIMIT_DEG * 128)`. While `players[i].playerstate == PST_DEAD`, ignore input and move `pitch_q` toward 0 by `128` (1°) per tic. On level start (08 calls `ext::reset_level()`): `pitch_q = 0`, ring cleared, accumulator cleared.

### 5.6 New bindings

Add to `m_controls.rs` as `#[no_mangle] pub static mut` with the defaults of 00 §8.3, bind them in `M_BindBaseControls` with `M_BindIntVariable(c"key_dmw_reload", …)`, and add rows to the `m_config.rs` defaults table (`DefaultType::Key` for keys, `Int` for mouse buttons and flags). The defaults are written to `default.cfg` on exit like every other variable. Doom key codes: printable keys are their lowercase ASCII; `KEY_RSHIFT` from `doomkeys.rs`.

Conflicts in MW2 mode: if a new binding equals a vanilla binding (`key_dmw_reload == key_use`, etc.), log `warn!` once at level start naming both and let both fire (no silent remap).

### 5.7 Defaults that change for MW2 players

None of the vanilla defaults change in `default.cfg`. In MW2 mode the behaviour changes of §5.4 apply at runtime only, so a vanilla session with the same `default.cfg` behaves like room.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Cursor grab unsupported (Wayland w/o pointer constraints) | fall back to `Confined`, then to un-grabbed relative mode using `CursorMoved` + recentre; `info!` once |
| Focus loss while firing | buttons cleared → Doom sees a release; no stuck fire |
| Huge raw deltas (e.g. after alt-tab) | per-sample clamp to ±2000 counts |
| Stale ext slot (tic mismatch) | neutral ext, `debug!` once |
| Demo playback | ext built but ignored (mode off); `TiccmdT` bytes identical to room |
| Netgame | mode off (D15); pitch never applied |

## 7. Logging

`dmw::input`: `info!` on first grab with mode (`locked`/`confined`/`relative`); `debug!` on grab/release transitions; `trace!` per tic `pitch_q`, `ext.buttons`; `warn!` binding conflicts.

## 8. Licensing and asset notes

None (pure input code).

## 9. Tests

| Id | Test |
|---|---|
| T5.1 | `PitchAccum`: 1000 random `data3` samples sum to the same total as one combined sample (no drift); invert flag; zoom scale halves rate when zoom FOV gives scale 0.5 |
| T5.2 | clamp: pitch never exceeds ±32°·128; dead player recentres at 1°/tic |
| T5.3 | `ext_for_gametic`: correct slot returned; stale slot → neutral |
| T5.4 | headless (`room/tests/input_headless.rs`, harness from 13 §5.2): start E1M1 with `-fixturepack`, inject `DG_GetMouse` deltas of +100 counts Y for 10 tics, assert `PLAYER_EXT[0].pitch_q` matches the formula and `players[0].mo->x` did not move (mouse Y no longer moves the player) |
| T5.5 | headless vanilla: same injection with `-nomw2`: pitch stays 0, the player moves forward (vanilla mouse Y) |
| T5.6 | ext buttons: inject key `r` down → `EXT_BT_RELOAD` in `PLAYER_EXT[0].cmd.buttons` next tic; right mouse → `EXT_BT_ADS`; Shift alone → `SPRINT`; Shift + right mouse → `HOLD_BREATH` and not `SPRINT` |
| T5.7 | `demo_playthrough` passes (stub `DG_GetMouse` added, nothing else changed) |

## 10. Acceptance criteria

- [ ] In `task run -- -fixturepack`, mouse turns the player, mouse Y looks up/down (visible once 06 lands; verified meanwhile through `trace!` logs), menus release the cursor, Escape/focus-loss never leaves fire stuck.
- [ ] In `task run -- -nomw2`, mouse X turns, mouse Y moves forward/back (vanilla), no pitch.
- [ ] `TiccmdT` layout unchanged; C layout guard tests pass.
- [ ] T5.1–T5.7 pass.
- [ ] New variables appear in `default.cfg` after a run.

## 11. Contract changes requested

No contract in sheet 00 had to change for the code. The following spec problems and implementation notes need a lead decision or an amendment of this sheet.

1. **`default.cfg` is never read or written by this engine.** `m_config.rs` (`M_SaveDefaults`, `LoadDefaultCollection`) is a no-op in `room` (`ORIGCODE` is undefined), so section 5.6 ("written to `default.cfg` on exit") and the acceptance line "New variables appear in `default.cfg` after a run" cannot hold, and 00 section 8.3 / D18 ("bindings are edited in `default.cfg`") has no effect. The ten variables are defined, bound with `M_BindVariable` and listed in the `extra_defaults` table, so they will persist once load/save exists. Implementing load/save changes vanilla behaviour (a user's `default.cfg` would start to be honoured), so it was not done here. Decision needed: port `LoadDefaultCollection`/`SaveDefaultCollection` (and call `M_SaveDefaults` on exit, which the window-close path does not reach today), or change D18 to read the bindings from `doommw2.toml`.
2. **`mode.rs` test override.** Added `mode::test_force_level_active(Option<bool>)` behind the new `test-hooks` cargo feature of `room` (enabled for tests by a self dev-dependency). It is the stand-in for `-fixturepack`/`-nomw2` until sheets 03 and 08 land. Sheet 08 owns `mode.rs` and should keep or replace it; sheet 13 section 5.2 already expects a `test-hooks` feature.
3. **`G_Responder` accumulates instead of assigning.** The `ev_mouse` arm does `MOUSEX += ...` / `MOUSEY += ...` (vanilla assigns). Doom can run `G_Responder` several times between two `G_BuildTiccmd` calls, and assigning would drop motion. `G_BuildTiccmd` already zeroes both. No effect without mouse events, so demo parity is unaffected.
4. **Exact pitch accumulation.** `PitchAccum` keeps the `(sensitivity + 5) / 10` factor and the zoom factor as exact denominators instead of truncating `counts` per event (section 5.3). Totals are identical to the spec formula at the default sensitivity and drift-free at every other value (T5.1).
5. **`ext.prev_yaw`** is written by `on_ticcmd_copied` (the console player's `mo->angle` before the tic's turn is applied); sheet 08 can read it instead of tracking its own. `ext::reset_level()` must be called from sheet 08's `on_level_start`; nothing else resets pitch at level start.
6. **`release_mouse()`** (00 section 7.5, `M_Responder`/menu open) has its body (`input::release_mouse`, polled by the platform layer) but no Doom call site: `m_menu.rs` is not in this sheet's file list. The platform grab policy already releases on `menuactive`, pause, non-level states and focus loss, so the hook is redundant today.
7. **`[input]` is read twice.** `main.rs` loads `doommw2.toml` itself (honouring `-mw2config`) to get `raw_mouse` and `grab_on_start` before the engine exists; sheet 08's `on_engine_init` will load it again for the rest. Harmless (one extra `info!` line).
8. **`-nomouse`** is now honoured in `I_InitInput` (it clears `usemouse`, which now defaults to 1 as section 4 requires).
9. **Pitch with `ticdup > 1`** is applied once per command (on the first of the `ticdup` gametics), not `ticdup` times.
12. **Contract change request (00 section 8.3, WASD defaults).** DOOMMW2 is the product, so the movement and use defaults change from Doom's: `[input]` gains `key_up = "w"`, `key_down = "s"`, `key_strafeleft = "a"`, `key_straferight = "d"`, `key_use = "f"` (`InputCfg`, `KeyBind`). `apply_bindings` copies them into the `m_controls` statics (`key_up`, `key_down`, `key_strafeleft`, `key_straferight`, `key_use`) next to the ten `key_dmw_*` values, in MW2 mode and in vanilla mode (`-nomw2`) alike, so they apply unless `doommw2.toml` overrides them. `key_left`/`key_right` are unchanged (arrows still turn). An unbound key (`"none"`) is stored as code 0 because `G_BuildTiccmd` indexes `gamekeydown` with these values directly. Demos replay ticcmds, so `demo_playthrough` is unaffected. Conflicts: the automap's `key_map_follow` is also `f` but is only read while the automap is open; `key_mission` (`w`) is a Heretic/Strife binding and unused here; reload `r`, melee `v` and swap `q` do not collide with the new keys. Existing headless tests that moved with the arrow keys or Use (`KEY_UPARROW`, `KEY_USE`) now press `w` and `f`.
