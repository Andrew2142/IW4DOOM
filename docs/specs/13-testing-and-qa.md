# 13 — Testing, Fixtures and QA

Owner: build agent — §5.1–5.4 in wave 2 (shared harness), the rest in wave 6
Status: implementation-ready
Depends on: 00, 01 (§5.6, §5.8, §5.9), 02 (tasks, WADs), 03 (fixture pack); the catalogue (§5.7) references every sheet's §9

This sheet provides the shared test infrastructure every other sheet's tests are written against — a reusable headless engine harness with input injection and framebuffer capture, golden-image tooling, deterministic test locations — plus benchmarks, the complete test catalogue, and the manual QA procedure for the day a real MW2 install is available.

---

## 1. Purpose

- Make "it works" checkable without MW2, a GPU, an audio device or a human.
- Keep vanilla parity provable at all times (`demo_playthrough` + frame-identity tests).
- Give a written, repeatable QA script for the MW2-data-dependent parts (exporter, real weapons).

## 2. Scope / Out of scope

In scope: `room/tests/support/` harness; golden PNG tooling; pinned test locations on Freedoom maps; `room/tests/bench.rs`; counting allocator for allocation tests; test catalogue; manual QA checklist; `task ci` composition.

Out of scope: writing the per-sheet tests themselves (each sheet owns its §9), hosted CI, fuzzing infrastructure beyond the simple random-input soak in 08 §10.

## 3. Dependencies

| From | What |
|---|---|
| room | `room/tests/demo_playthrough.rs` pattern: own `DG_*` stubs, virtual time (`TICK_MS = 1000/35`), `doomgeneric_Create` with args, `doomgeneric_Tick` loop, reading Doom globals |
| 02 | pinned Freedoom (`wads/freedoom1.wad`, `freedoom2.wad`), `doom1.wad` rule, `task` targets |
| 03 | `Pack::fixture()`, `-fixturepack` |
| 05 | `DG_GetMouse` callback |

## 4. File inventory

| Path | Content |
|---|---|
| `room/tests/support/mod.rs` | `Harness` (§5.2) |
| `room/tests/support/golden.rs` | PNG capture/compare/bless (§5.4) |
| `room/tests/support/locations.rs` | pinned map spots (§5.3) |
| `room/tests/support/alloc.rs` | counting global allocator (opt-in per test binary) |
| `room/tests/bench.rs` | §5.5 |
| `room/tests/golden/` | committed goldens (Freedoom-rendered frames are derived from a BSD-licensed IWAD; fixture-gun pixels are our own) |
| `docs/qa/manual-mw2.md` | §5.6 checklist (copied from this sheet, kept current) |

## 5. Detailed design

### 5.1 Test layers

| Layer | Where | Runs in `task test` | Needs |
|---|---|---|---|
| Unit | `crates/*/src`, `room/src/dmw/*` `#[cfg(test)]` | yes | nothing |
| Crate integration + goldens | `crates/*/tests` | yes | nothing (synthetic palette) |
| Headless engine | `room/tests/*_headless.rs` | yes | Freedoom (pinned) |
| Vanilla parity | `room/tests/demo_playthrough.rs` | yes when `doom1.wad` is real (02 §5.4) | shareware IWAD |
| Bench | `room/tests/bench.rs` | no (`task bench`) | Freedoom |
| Manual MW2 QA | `docs/qa/manual-mw2.md` | no | MW2 install |

### 5.2 Headless harness (`room/tests/support/mod.rs`)

One engine per process (Doom globals): every `*_headless.rs` file has exactly **one** `#[test]` that runs its scenarios in sequence through one `Harness`; scenarios restart with `harness.new_game(skill, episode, map)`.

```rust
pub struct Harness { /* virtual clock, input queues, frame counter */ }
impl Harness {
    /// args e.g. ["-iwad", wad, "-fixturepack", "-skill", "3", "-warp", "1", "1", "-nosound"]
    pub fn start(args: &[&str]) -> Harness;
    pub fn new_game(&mut self, skill: i32, episode: i32, map: i32);
    pub fn run_tics(&mut self, n: u32);                 // advances virtual time by n*TICK_MS and ticks until gametic advanced n
    pub fn key(&mut self, doom_key: u8, down: bool);    // queued for DG_GetKey
    pub fn mouse(&mut self, dx: i32, dy: i32, buttons: u8);   // queued for DG_GetMouse (05)
    pub fn hold(&mut self, doom_key: u8, tics: u32);    // key down, run, key up
    pub fn player(&self) -> &PlayerT;                   // players[consoleplayer]
    pub fn player_ext(&self) -> PlayerExt;              // copy of PLAYER_EXT[consoleplayer]
    pub fn sim_view(&self) -> Option<ViewState>;        // via dmw::with_state
    pub fn teleport(&mut self, x: f32, y: f32, angle_deg: f32);   // P_TeleportMove + angle, Doom units
    pub fn set_pitch(&mut self, deg: f32);              // writes PLAYER_EXT pitch_q
    pub fn spawn(&mut self, mobjtype: c_int, x: f32, y: f32) -> *mut mobj_t;   // P_SpawnMobj on the floor
    pub fn give_all(&mut self);                         // IDKFA through the cheat path
    pub fn frame(&mut self) -> Frame;                   // copy of I_VideoBuffer (320×200 indices) + current COLORS palette
    pub fn events(&self) -> Vec<RecordedEvent>;         // SimEvents of the last tics (08 exposes a test-only ring buffer behind cfg(any(test, feature = "test-hooks")))
}
```

`DG_*` stubs live in `support/mod.rs` exactly like `demo_playthrough.rs` (they must be `#[no_mangle]` in the test binary). Audio is disabled (`-nosound` or the audio init failing harmlessly). The harness never sleeps: `DG_SleepMs` is a no-op and `DG_GetTicksMs` returns the virtual clock.

The engine crate gets a `test-hooks` cargo feature (off by default) enabling the event ring buffer and a few read accessors; `room/tests` enable it through `[dev-dependencies]`/`required-features`. No test-only code is compiled into the shipped binary.

### 5.3 Pinned locations (`locations.rs`)

For the pinned Freedoom tag (02 §5.4), the agent finds and records, by running the harness and printing coordinates:

| Name | Map | Requirement |
|---|---|---|
| `OPEN_ROOM` | Freedoom1 E1M1 | player spot with ≥ 512 units of clear, level floor straight ahead, ceiling ≥ 128, no monsters within 1024 (after removing all monsters via a helper that `P_RemoveMobj`s every `MF_COUNTKILL` thing) |
| `DARK_SPOT` | any E1 map | sector light ≤ 96 for lighting goldens |
| `GUN_SWITCH` | any map | a `G1`/`GR` gun-activated line visible from a spot within 256 units (for T9.7); if Freedoom has none, T9.7 builds it by setting a nearby two-sided line's `special` to 46 (`GR` door open) in-memory |
| `SKY_VIEW` | E1M1 | outdoor spot with sky above, for T6.6 |

Each record: map, x, y, angle, a short description, and an assertion helper that fails loudly if the spot no longer satisfies its requirement (catches a Freedoom bump).

### 5.4 Golden images (`golden.rs`)

- `Frame::to_png(&self, crop: Option<Rect>) -> Vec<u8>`: expands indices through the captured palette (gamma 0), writes RGB PNG.
- `assert_golden(name, frame, crop)`: compares against `room/tests/golden/<name>.png` (or `crates/*/tests/golden/`). Exact match required by default; a test may pass a tolerance (`max_differing_pixels`). On mismatch writes `target/test-artifacts/<name>.{actual,diff}.png` and fails with the path. `BLESS=1` overwrites the golden and prints the path.
- Crate-level goldens (11) use a synthetic palette (6×6×6 cube + greys) and a synthetic COLORMAP (linear darkening toward index 0) generated in-code, so they need no WAD.

### 5.5 Benchmarks (`room/tests/bench.rs`, `task bench`)

Release mode; prints a table and fails if a budget in 01 §5.8 is exceeded by > 20 %:

| Bench | Measures |
|---|---|
| `sim_tic` | 3500 tics of fixture rifle auto-fire + ADS toggling; mean/p99 `WeaponSim::tic` |
| `anim_pose` | `ViewmodelAnimator::advance` + `pose` |
| `raster_fixture` | `Rasterizer::draw` of the fixture gun at 4 poses |
| `raster_20k` | synthetic 20 000-triangle mesh |
| `frame_total` | full `doomgeneric_Tick` with MW2 mode vs `-nomw2` at `OPEN_ROOM` (delta reported) |
| `allocations` | counting allocator: steady-state allocations per frame in `Rasterizer::draw` must be 0 |

### 5.6 Manual QA with real MW2 data

Preconditions: MW2 multiplayer installed; `DMW_MW2_GAMES` set; `task iw4l-rev` passes; `task export` exits 0. Record results in `docs/qa/results-YYYY-MM-DD.md`.

1. **Export**: `task export -- --list` lists ≥ 50 weapons including the 00 §9.1 roster; full export report has zero errors; warnings reviewed and listed.
2. **Determinism**: export twice → identical packs except timestamps (T4.8).
3. **Per weapon** (each of the 9 roster weapons + 4 extras, remapped into slots via `doommw2.toml`), at `OPEN_ROOM` with `IDDQD`:
   - Raise, first raise (new life), drop: timings look and sound like MW2; no T-pose or bind-pose frames.
   - Fire: semi/auto/burst/double-barrel behaviour correct; fire rate matches the facts (count shots in 5 s with the HUD/log vs `1000/fire_time_ms`).
   - Recoil: view kick climbs and recentres; gun recoil visible; ADS reduces both.
   - Spread: crosshair blooms with fire and movement, tightens when still; ADS spread tight.
   - Reload and empty reload: animation + notetrack sounds in sync; magazine/pool numbers correct; shotgun inserts one by one and can be interrupted by firing.
   - ADS: sight lines up with the screen centre at full ADS (shots land on the centre dot); FOV zoom smooth; sniper overlay + sway + hold breath.
   - Sprint: weapon lowers, cannot fire until sprint-out finishes.
   - Melee: knife/butt animation plays, kills an imp.
   - Look: textures quantized cleanly (dither on/off compared), lighting follows sector light, invulnerability/invisibility effects, muzzle flash patch at the barrel.
4. **Damage**: zombieman/imp/demon kills-to-shots counts recorded per weapon at 256 units; headshot counts lower than body shots; `monster_damage_scale` adjustments tried at 0.25/0.35/0.5 and a recommendation recorded.
5. **Vanilla parity**: `task run -- -nomw2` plays exactly like room; `demo_playthrough` passes.
6. **Performance**: `task bench` with the real pack: raster of the heaviest viewmodel within budget; report triangle counts.

### 5.7 Test catalogue

| Sheet | Tests | Kind |
|---|---|---|
| 02 | config round-trip, unknown keys, clamps, `links()` per crate, `check-deps.sh` | unit/script |
| 03 | T3.1–T3.9 | unit/golden |
| 04 | T4.1–T4.5 (auto), T4.6–T4.8 (manual, MW2) | unit/manual |
| 05 | T5.1–T5.7 | unit/headless |
| 06 | T6.1–T6.7 | unit/headless golden |
| 07 | T7.1–T7.14 | unit/golden |
| 08 | T8.1–T8.10 + soak | headless |
| 09 | T9.1–T9.10 | unit/headless |
| 10 | T10.1–T10.8 | unit/golden |
| 11 | T11.1–T11.10 | unit/golden/headless/perf |
| 12 | T12.1–T12.7 | unit/headless golden |
| 13 | benches §5.5, manual §5.6 | perf/manual |

`task ci` = `task check && task lint && task test` (+ `task export-check` when `DMW_BUILD_EXPORTER=1`). Every sheet's acceptance requires its rows here to pass.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Freedoom missing | headless tests fail with "run task fetch-wads first" (01 §5.9) |
| Freedoom hash mismatch | `fetch-freedoom.sh` refuses; locations would be wrong |
| Golden missing | test fails and writes the actual image; `BLESS=1` creates it |
| Two `#[test]`s in one headless file | forbidden by review; the harness asserts it is started once per process |

## 7. Logging

Tests run with `RUST_LOG=dmw=debug` captured; on failure the harness prints the last 200 `dmw::*` log lines (custom test logger in `support`).

## 8. Licensing and asset notes

Goldens contain Freedoom-derived pixels (Freedoom is BSD-3-Clause; keep its licence notice in `room/tests/golden/README.md`) and fixture-gun pixels (ours). Never commit frames rendered with an MW2 pack.

## 9. Tests

The harness itself: a smoke test that starts E1M1, runs 35 tics, captures a frame, and asserts `gametic == 35` and a non-uniform frame.

## 10. Acceptance criteria

- [ ] Harness, golden tooling, locations, allocator and bench exist and are used by sheets 05–12.
- [ ] `task ci` green on a machine with Freedoom and without MW2.
- [ ] `docs/qa/manual-mw2.md` exists with §5.6, ready for the first MW2 run.

## 11. Contract changes requested

1. Engine crate feature `test-hooks` (off by default) with a `SimEvent` ring buffer and test accessors (§5.2). Owned by 08; recorded here. — Applied: 08 implements it as part of §5.5 (no API change for shipped code).
