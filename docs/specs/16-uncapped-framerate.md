# 16 — Uncapped Frame Rate

Owner: lead
Status: implemented
Depends on: 00 (§7.5 hook points, extended here), 05 (mouse pitch), 06 (freelook), 08

Doom draws one frame per 35 Hz tic and reads the mouse once per tic, so mouse look updates every 28 ms. This sheet draws frames as often as the display allows (like Crispy Doom and the KEX ports) without changing the simulation.

## 1. Behaviour

- `[render] uncapped = true` (default): the main loop stops waiting for the next tic. When no tic is due, `TryRunTics` pumps input (`I_StartTic` + `ProcessEvents`, not `RunMenu`) and returns, and `D_Display` draws a frame. Vsync paces frames; `[render] max_fps` (0 = off) caps them in `main.rs`.
- Each frame, map objects (x, y, z, angle), sector floor/ceiling heights and the display player's `viewz` are blended between the snapshot taken at the start of the last world tic and the current values, by how far real time is into the next tic (`I_GetTimeMS · 35 mod 1000`). Blending only happens while that tic is the latest one (paused or menu-frozen worlds draw as they are). A thing that moved more than 64 units horizontally or 128 vertically in one tic, or whose pointer now holds another type, is drawn unblended.
- The local, living player's view angle is the current `mo->angle` plus the turn of ticcmds built but not yet run plus `MOUSEX · 8`; the rendered pitch adds their `pitch_delta` plus the pending pitch accumulator. Not in demo playback or when viewing another player.
- Off in net games and when the front end has not allowed it (headless tests). With `singletics` (`-timedemo`, the test harness) no early return happens and frames are drawn at the tic itself.

## 2. Hook points (amends 00 §7.5)

| Engine site | Call |
|---|---|
| `TryRunTics`, after `availabletics` | `hooks::uncapped_return_early(availabletics)` |
| `P_Ticker`, after the pause/menu checks | `hooks::on_ticker_start()` (snapshot) |
| `D_Display`, around `R_RenderPlayerView` | `hooks::begin_frame()` / `hooks::end_frame(restore)` |
| `d_loop::maketic()`, `g_game::pending_mousex()` | read-only accessors |

`begin_frame` writes blended values into the live structs and `end_frame` restores them before anything else runs, so the simulation, saves and demos never see them.

## 3. Tests

- `room/tests/interp_headless.rs`: with interpolation allowed, every map object, sector height and `viewz` is bit-identical before and after `D_Display` for 40 walking tics; at least 30 of those frames were blended; off means no blending.
- `demo_playthrough` (vanilla parity) unchanged.
- Unit tests: fixed-point and angle lerp.

## 4. Not done

The MW2 viewmodel animation (bob, sway, recoil) still advances per tic; interpolating the animator pose is a follow-up.
