# 01 — Engineering Standards (Rust, unsafe Doom glue, determinism, licensing)

Owner: build agents (reference sheet; no feature code)
Status: binding for sheets 02–13. Contracts (names, paths, types, constants, hook table) come from `00-overview-and-contracts.md` and are never changed here.

---

## 1. Purpose

Give every build agent one checkable definition of "written the DOOMMW2 way" for a codebase that mixes three very different kinds of Rust:

1. **The Doom engine fork** (`room/src/doom/*`): a line-by-line port of C with `static mut` globals, raw pointers and `#[no_mangle] extern "C"` functions. We touch it as little as possible.
2. **Engine glue** (`room/src/dmw/*`): the only place that is allowed to know both Doom and MW2. Small, unsafe at the edges, defensive.
3. **Pure crates** (`dmw_format`, `dmw_weapons`, `dmw_viewmodel`) and the **exporter** (`tools/dmw_export`): ordinary safe Rust, unit-testable without Doom, without MW2 and without a GPU.

## 2. Scope / Out of scope

In scope: toolchain, formatting, lints, crate boundaries, unsafe policy, error model, logging, determinism, performance budgets, testing rules, licensing/provenance, git conventions, Definition of Done, the root `AGENTS.md` text.

Out of scope: creating the files (sheet 02 copies the configs below verbatim), test catalogue (sheet 13).

## 3. Dependencies

00 §2 (D2–D4, D15, D20, D22), §4 (layout), §6 (module map), §7.5 (hook table), §8.4 (log targets).

## 4. File inventory

This sheet creates no files. It defines content placed by sheet 02:

| File | Content source |
|---|---|
| `rustfmt.toml` | §5.2 |
| `clippy.toml` | §5.2 |
| `[workspace.lints]` in root `Cargo.toml` | §5.2 |
| `AGENTS.md`, `CLAUDE.md` | §5.13 |
| `NOTICE` | §5.10 |

## 5. Detailed design

### 5.1 Toolchain and editions

- `rust-toolchain.toml`: `channel = "stable"`, `components = ["rustfmt", "clippy"]`. Minimum 1.85 (iw4L crates are edition 2024). The developer machine has 1.95.
- The copied engine crate `room`, `doomgeneric-sys` and `c2rust-intermediate` stay on **edition 2021** (changing edition would rewrite `unsafe` and `static mut` semantics across 80 files).
- New crates (`crates/*`, `tools/dmw_export`) use **edition 2024**.
- iw4L crates are consumed as-is through path dependencies; their own `[lints] workspace = true` resolves against `../iw4L/Cargo.toml`. Never copy iw4L's workspace lint table into ours.

### 5.2 Formatting and lints

`rustfmt.toml` (repo root):

```toml
edition = "2024"
max_width = 100
use_field_init_shorthand = true
```

Engine files under `room/src/doom/` are formatted with the same `rustfmt.toml` but only files an agent touches are reformatted (run `rustfmt --edition 2021 <file>` on the touched file; never `cargo fmt --all` in a commit that is not purely formatting).

Root `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "warn"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
dbg_macro = "deny"
todo = "warn"
unwrap_used = "warn"
expect_used = "warn"
print_stdout = "deny"
print_stderr = "deny"
float_cmp = "warn"
```

`crates/*` and `tools/dmw_export` set `[lints] workspace = true`. The engine crate keeps its existing crate-level `#![allow(...)]` list in `room/src/lib.rs` (do not widen it); `room/src/dmw/mod.rs` starts with `#![deny(clippy::dbg_macro, clippy::print_stdout)]` and `#![warn(clippy::undocumented_unsafe_blocks)]`.

`task lint` = `cargo clippy --workspace --all-targets -- -D warnings` for `crates/*` and `room` (the engine crate's existing warnings are allowed by its crate attributes; new warnings in `room/src/dmw` are not).

`unwrap`/`expect` are allowed in tests and in `tools/dmw_export/src/main.rs` argument handling only.

### 5.3 Crate boundaries (enforced in review and by `tools/check-deps.sh`, sheet 02 §5.8)

| Crate | May depend on | Must not depend on |
|---|---|---|
| `dmw_format` | `serde`, `serde_json`, `postcard`, `png`, `hound`, `thiserror`, `weapon_iw4` (types only, for serde remote derives), `glam` | `room`, `dmw_weapons`, `dmw_viewmodel`, any Bevy-dependent iw4L crate |
| `dmw_weapons` | `dmw_format`, `weapon_iw4`, `movement_iw4`, `playerstate_iw4`, `math_iw4`, `xmodel_runtime`, `anim_iw4`, `glam`, `log`, `thiserror`, `serde` | `room`, `dmw_viewmodel`, Bevy, `rand` (use `DmwRng`) |
| `dmw_viewmodel` | `dmw_format`, `glam`, `log`, `thiserror` | `room`, `dmw_weapons` (it receives a `ViewmodelPose`-shaped input defined in `dmw_viewmodel::Pose` — see sheet 11 §5.1), iw4L |
| `room` (engine) | everything above, `toml`, existing deps | Bevy, any Bevy-dependent iw4L crate |
| `tools/dmw_export` | iw4L `assets`, `asset_game`, `asset_model`, `asset_anim`, `asset_audio`, `asset_material`, `asset_transport`, `asset_core`, `frame`, `bevy` (transitively), `dmw_format`, `anyhow`, `clap` | `room`, `dmw_weapons`, `dmw_viewmodel` |

`dmw_weapons` and `dmw_viewmodel` both declare `#![forbid(unsafe_code)]`. `dmw_format` declares `#![forbid(unsafe_code)]`. Only `room` contains `unsafe`.

Note on 00 §7.2/§7.3: `ViewmodelPose` is defined in `dmw_weapons`; `dmw_viewmodel` defines a structurally identical `dmw_viewmodel::Pose { skin: Vec<Mat4>, tag_flash: Option<Mat4> }` and the engine glue converts (a move, no copy of matrices beyond the `Vec`). This keeps the rasterizer free of the sim.

### 5.4 Unsafe and `static mut` policy (engine only)

1. New DOOMMW2 state that Doom code must reach lives in `room/src/dmw/*` as `static mut` **only** when a `#[no_mangle]`/C-shaped caller needs it (00 §7.4 `EXT_TICCMDS`, `PLAYER_EXT`). Everything else lives in one `thread_local!` `RefCell<Option<DmwState>>` accessed through `dmw::with_state(|s| …)` (sheet 08 §5.1).
2. Access `static mut` through `&raw mut` / `&raw const` (`std::ptr::addr_of_mut!` in 2021 code). Never create a `&mut` to a `static mut` that outlives one expression.
3. Every `unsafe` block has a `// SAFETY:` comment naming the invariant (pointer validity source, single-threaded engine, index bounds).
4. Never change a `#[repr(C)]` Doom struct. Extension data goes in parallel arrays indexed the same way (00 §7.4).
5. A hook is one call at the Doom call site, wrapped so vanilla flow is unchanged when the hook returns its "not handled" value. Example shape:

```rust
// in P_PlayerThink, before P_MovePsprites(player):
if !crate::dmw::hooks::on_player_think(player) {
    P_MovePsprites(player);
}
```

6. No hook may call back into a Doom function that re-enters the same hook.
7. The engine is single-threaded (winit main thread). DOOMMW2 code adds no threads except the rodio output thread already owned by `room`'s audio module, and pack loading may use `std::thread::scope` at level start only.

### 5.5 Error model

| Layer | Type | Rule |
|---|---|---|
| `dmw_format` | `PackError` (00 §7.1, `thiserror`) | every I/O and parse failure is a typed error carrying the path and the failing field |
| `dmw_weapons` | `SimError`, `AnimError` (`thiserror`) | only constructors/loaders fail; `WeaponSim::tic` and `ViewmodelAnimator::{advance,pose}` are infallible (bad data was rejected at load) |
| `dmw_viewmodel` | `RasterError` (`thiserror`) | only `PreparedViewmodel::new` fails; `draw` is infallible and clips everything |
| engine glue | none escape | a failure in a hook logs once (`error!`) and switches MW2 mode off for the rest of the level (`mode::fail(reason)`), which returns control to vanilla Doom. Never `panic!` in a tic or render path. |
| exporter | `anyhow::Result` in `main`, typed errors in library modules | exits non-zero with a one-line summary plus the report file path |

A missing pack, wrong pack version or invalid `doommw2.toml` at startup is not fatal: log `warn!`, run vanilla.

### 5.6 Determinism

The MW2 simulation must give identical results for identical inputs (it is what makes golden tests possible, sheet 13):

- No wall-clock, `Instant`, or frame time in `dmw_weapons`. Time = `tic_msec` (00 §5.3) only.
- Randomness only from `DmwRng` (xorshift32, sheet 07 §5.10), seeded at level start from `(gameskill, gameepisode, gamemap)`. Never call Doom's `P_Random`/`M_Random` from MW2 code (it would shift Doom's RNG table index and change monster behaviour).
- No iteration over `HashMap`/`HashSet` where order can reach an output. Use `BTreeMap`, `Vec`, or sort first.
- `f32` everywhere in the sim (matches iw4L). No `f64` accumulators that are later narrowed, except the explicit BAM conversions in 00 §5.1.
- The rasterizer is deterministic for the same inputs (fixed iteration order, no SIMD-width-dependent rounding paths).

### 5.7 Logging

Crate `log` everywhere; engine already initialises `env_logger`. Targets are the fixed list in 00 §8.4 (`log::info!(target: "dmw::pack", ...)`).

| Level | Use |
|---|---|
| `error` | a feature was disabled because of a failure (mode switched off, pack rejected) |
| `warn` | degraded but running (missing optional clip, unknown config key, missing sound alias) |
| `info` | once per lifecycle event: pack loaded (N weapons), level mode on/off and why, config file used |
| `debug` | per gameplay event: shot fired, reload start/end, weapon raise/drop, damage dealt |
| `trace` | per tic / per frame numbers (spread, ads_frac, raster triangle counts) |

`room/src/dmw/mod.rs` provides `log_once!(target, level, key, ...)` that logs a given `key` at most once per level. Per-tic `info!`/`warn!` without `log_once!` fails review.

### 5.8 Performance budgets (release build, dev machine)

| Work | Budget per tic/frame |
|---|---|
| `WeaponSim::tic` + `ViewmodelAnimator::advance` | ≤ 0.2 ms |
| `ViewmodelAnimator::pose` (≤ 128 bones) | ≤ 0.1 ms |
| `Rasterizer::draw` (≤ 20 000 triangles at 320×200) | ≤ 2.0 ms |
| Level-start viewmodel preparation (all mapped weapons) | ≤ 1.5 s total, logged at `info` |

Steady-state tics and frames must not allocate in the rasterizer (buffers reused). The sim may allocate small `Vec`s for events. Measure with `task bench` (sheet 13 §5.5).

### 5.9 Testing rules

- Unit tests live next to the code (`#[cfg(test)] mod tests`). Integration tests for crates live in `crates/<crate>/tests/`.
- Engine tests are headless (`room/tests/*.rs`, the pattern of `demo_playthrough.rs`: own `DG_*` stubs, virtual clock) and need an IWAD: `DMW_IWAD`, else `wads/freedoom1.wad`, else `doom1.wad`. If none exists the test **fails** with the message `run 'task fetch-wads' first` (no silent skip).
- No test may need MW2 data, a GPU, an audio device or the network.
- Golden files (images, numeric baselines) live in `room/tests/golden/` and `crates/*/tests/golden/`; `BLESS=1` rewrites them; a blessed change is reviewed like code.
- The existing `room/tests/demo_playthrough.rs` must stay green unchanged (it proves D15 vanilla parity). If it needs the shareware `doom1.wad` (its baseline is shareware-specific), sheet 02 §5.4 explains how to obtain it.

### 5.10 Licensing and provenance

- Root `LICENSE` = GPL-3.0 text. Keep `LICENSE-GPL-2.0` (from room) and add `LICENSE-APACHE-2.0` (from iw4L).
- New files start with `// SPDX-License-Identifier: GPL-3.0-or-later`.
- Files that port iw4L code start with:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// <original path>, Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: <one line>.
```

  and get a row in `NOTICE` and `docs/provenance.md`.
- Never commit: anything from an MW2 install, anything under `packs/`, `.env`, screenshots or videos showing MW2 assets, IWADs other than the files already tracked by room. `.gitignore` covers `packs/`, `wads/`, `.env`, `*.dsg`, `target/`.
- Never put MW2 asset names, offsets or retail addresses into code comments as "how the game does it" claims taken from decompilation; describe behaviour and cite the iw4L file instead.

### 5.11 Git conventions

- Branch per sheet: `sheet/NN-short-slug` (e.g. `sheet/07-weapon-sim`).
- Conventional commits: `feat(dmw_weapons): …`, `fix(engine): …`, `test(…)`, `docs(specs): …`, `chore(…)`.
- One logical change per commit; a commit that touches `room/src/doom/*.rs` names the Doom function(s) in the body.
- Commit trailer as required by the user's tooling (`Co-Authored-By:` line when an agent commits).

### 5.12 Definition of Done (every sheet)

- [ ] Every item in the sheet's §10 is checked.
- [ ] `task check`, `task lint`, `task test` pass on a clean checkout with `task fetch-wads` done and **no MW2 data present**.
- [ ] `room/tests/demo_playthrough.rs` passes unchanged.
- [ ] No new `#[repr(C)]` layout changes (C layout guard tests in `room/src/doom/c_tests` pass).
- [ ] New public items have doc comments; new `unsafe` blocks have `SAFETY:` comments.
- [ ] Log lines follow §5.7 and 00 §8.4.
- [ ] Ported iw4L code has headers and NOTICE rows (§5.10).
- [ ] The sheet's "Contract changes requested" is either "None" or every item is resolved in a 00 Revision.

### 5.13 `AGENTS.md` (repo root; `CLAUDE.md` contains the single line `Read AGENTS.md.`)

Sheet 02 writes this text verbatim:

```markdown
# DOOMMW2 agent instructions

Read docs/specs/README.md, then docs/specs/00-overview-and-contracts.md, then docs/specs/01-engineering-standards.md, then your sheet.

## Ground rules
- The engine in room/src/doom is a C port. Touch it only at the hook points listed in 00 §7.5; put logic in room/src/dmw.
- Never change a #[repr(C)] struct. Extension state lives in room/src/dmw/ext.rs.
- MW2 data never enters the repository. Use the fixture pack (`-fixturepack`, `Pack::fixture()`) for tests.
- iw4L (../iw4L) is read-only and pinned (IW4L_REV). Run tools/check-iw4l-rev.sh if builds behave oddly.
- When MW2 mode is off the engine must behave exactly like room. demo_playthrough proves it.
- Contracts are in docs/specs/00. Do not change them in code; request changes in your sheet's §11.

## Commands
- task check | task lint | task test | task run | task fetch-wads | task fixture-pack | task export (needs MW2)

## Porting lessons inherited from room (still true)
- Pass NUL-terminated byte strings (c"..." literals) to C-shaped functions.
- Use wrapping_add/wrapping_sub/wrapping_neg for angle_t arithmetic.
- Cast state_t pointers to *mut State before touching fields.
- Do not zero memory with ptr::write_bytes when running under ASan; use explicit loops.
```

## 6. Error handling and edge cases

Covered by §5.5. Every sheet's §6 must name, for each external input it reads (pack file, config key, WAD lump, MW2 asset), what happens when it is missing or malformed.

## 7. Logging

§5.7.

## 8. Licensing and asset notes

§5.10.

## 9. Tests

This sheet has no tests of its own. `tools/check-deps.sh` (sheet 02) enforces §5.3 mechanically with `cargo tree -e normal -p <crate>` and a deny-list per crate.

## 10. Acceptance criteria

- [ ] Sheet 02 placed `rustfmt.toml`, workspace lints, `AGENTS.md`, `CLAUDE.md`, `NOTICE`, licence files exactly as specified here.
- [ ] `tools/check-deps.sh` fails if `dmw_weapons` gains a dependency on `room` or Bevy (verified by a deliberate local experiment, not committed).

## 11. Contract changes requested

None.
