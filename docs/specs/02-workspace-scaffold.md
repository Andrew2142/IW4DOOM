# 02 — Workspace Scaffold and Tooling

Owner: build agent (wave 1)
Status: implementation-ready
Depends on: 00 (§2 D2–D3, D18–D22, §4, §5, §6), 01 (all)

This sheet turns the empty `/home/andy/Dev/MW2DOOM/doommw2` folder (which today holds only `docs/specs/`) into a building, testing workspace: the `room` engine copied in, three empty-but-compiling new crates, a separate exporter workspace, config schema, task runner, IWAD fetching, licence files and guard scripts. It writes no gameplay code.

---

## 1. Purpose

Every later sheet must be able to start by running `task check && task test` and get green, with no MW2 data on the machine. This sheet makes that true and fixes the mechanical conventions (paths, tasks, config loading) every other sheet relies on.

## 2. Scope / Out of scope

In scope:

- Copying `room` at `904acac` into `doommw2/` and initialising a fresh git repository.
- Root `Cargo.toml` workspace, `rust-toolchain.toml`, `rustfmt.toml`, lints (01 §5.2).
- Renaming the binary to `doommw2` and the window title to `DOOMMW2`.
- Empty crates `crates/dmw_format`, `crates/dmw_weapons`, `crates/dmw_viewmodel` with their final dependency lists (01 §5.3), compiling with one smoke test each.
- `room/src/dmw/` module skeleton with every file from 00 §6.4 present (stubs), `consts.rs` re-export and the full `config.rs` (schema + loader).
- `tools/dmw_export/` separate workspace skeleton that builds against iw4L (prints its version, does nothing else).
- Path dependencies on iw4L and the revision pin check.
- IWAD acquisition (`task fetch-wads`), `.env.example`, `doommw2.example.toml`.
- `Taskfile.yml`, `tools/check-iw4l-rev.sh`, `tools/check-deps.sh`, `tools/fetch-freedoom.sh`.
- Licence files, `NOTICE`, `docs/provenance.md`, `README.md`, `AGENTS.md`, `CLAUDE.md`, `.gitignore`.

Out of scope:

- Any hook body, pack type, sim or renderer code (sheets 03–12).
- CI configuration (no hosted CI in this build; `task ci` runs locally).

## 3. Dependencies

| From | What |
|---|---|
| 00 §4.1 | directory layout |
| 00 §4.2–4.4 | env vars, flags, runtime files |
| 00 §6 | crate and module file lists |
| 00 §9 | config sections and defaults |
| 01 §5.1–5.3, §5.10, §5.13 | toolchain, lints, boundaries, licences, AGENTS.md text |

External facts verified while writing this sheet:

- `room` at `904acac`: workspace members `c2rust-intermediate`, `doomgeneric-sys`, `room`; `doom1.wad` and `soundfonts/*.sf2` are **Git LFS pointers** (the developer machine has no `git-lfs` installed, so the real files are absent). `room/tests/demo_playthrough.rs` looks for `../doom1.wad`.
- `iw4L` at `fc3d8ea`: edition 2024; root `[patch.crates-io]` pins `quinn-proto` to a git rev (needed by `master_protocol`, which the exporter pulls in through `frame`); `[profile.play]` is release minus fat LTO.
- Rust on the developer machine: 1.95.0.

## 4. File inventory

Created or changed by this sheet (paths relative to `doommw2/`):

| Path | Action |
|---|---|
| everything from `room/` at `904acac` except `target/`, `.git/`, `.claude/`, `.agents/`, `.github/`, `CLAUDE.md`, `AGENTS.md`, `.pre-commit-config.yaml` | copied |
| `Cargo.toml` | rewritten (§5.2) |
| `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` | new |
| `room/Cargo.toml` | edited: bin name, new deps (§5.3) |
| `room/src/main.rs` | edited: window title only |
| `room/src/lib.rs` | edited: `pub mod dmw;` |
| `room/src/dmw/{mod,consts,config,mode,ext,input,hooks,weapons,ballistics,viewmodel,freelook,hud,audio,save}.rs` | new stubs; `consts.rs` and `config.rs` complete |
| `crates/dmw_format/{Cargo.toml,src/lib.rs}` | new |
| `crates/dmw_weapons/{Cargo.toml,src/lib.rs}` | new |
| `crates/dmw_viewmodel/{Cargo.toml,src/lib.rs}` | new |
| `tools/dmw_export/{Cargo.toml,src/main.rs}` | new, separate workspace |
| `tools/check-iw4l-rev.sh`, `tools/check-deps.sh`, `tools/fetch-freedoom.sh` | new |
| `Taskfile.yml` (replaces `Taskfile.dist.yaml`) | new |
| `IW4L_REV` | new, content `fc3d8ea` plus newline (full hash recorded in `docs/provenance.md`) |
| `.env.example`, `doommw2.example.toml` | new |
| `.gitignore` | extended |
| `LICENSE` (GPL-3.0), `LICENSE-APACHE-2.0`, `NOTICE`, `docs/provenance.md` | new |
| `README.md`, `AGENTS.md`, `CLAUDE.md` | rewritten |

## 5. Detailed design

### 5.1 Import procedure

1. `rsync -a --exclude target --exclude .git --exclude .claude --exclude .agents --exclude .github --exclude CLAUDE.md --exclude AGENTS.md --exclude .pre-commit-config.yaml /home/andy/Dev/MW2DOOM/room/ /home/andy/Dev/MW2DOOM/doommw2/` — the existing `docs/specs/` in `doommw2` must survive (room's own `docs/plans`, `docs/specs` files land next to ours; move room's into `docs/room-upstream/` to avoid confusion).
2. `git init -b main`, commit 1: `chore: import room at 904acac` containing only the copied files **as they were** (so later diffs against upstream are readable).
3. Commit 2 onward: everything else in this sheet.
4. Record in `docs/provenance.md`: upstream URLs, the two full commit hashes (`git -C ../room rev-parse HEAD`, `git -C ../iw4L rev-parse HEAD`), date, and the excluded paths.

LFS pointers (`doom1.wad`, `soundfonts/*.sf2`) are copied as pointers. §5.4 explains how to obtain the real files. `.gitattributes` keeps the LFS lines so a later `git lfs` install works.

### 5.2 Root `Cargo.toml`

```toml
[workspace]
members = [
    "c2rust-intermediate",
    "doomgeneric-sys",
    "room",
    "crates/dmw_format",
    "crates/dmw_weapons",
    "crates/dmw_viewmodel",
]
default-members = ["doomgeneric-sys", "room", "crates/dmw_format", "crates/dmw_weapons", "crates/dmw_viewmodel"]
exclude = ["tools/dmw_export"]
resolver = "2"

[workspace.package]
version = "0.1.0"
license = "GPL-3.0-or-later"
repository = "local"

[workspace.dependencies]
# iw4L, pinned by IW4L_REV (tools/check-iw4l-rev.sh). Bevy-free crates only.
weapon_iw4 = { path = "../iw4L/crates/weapon_iw4" }
movement_iw4 = { path = "../iw4L/crates/movement_iw4" }
playerstate_iw4 = { path = "../iw4L/crates/playerstate_iw4" }
math_iw4 = { path = "../iw4L/crates/math_iw4" }
xmodel_runtime = { path = "../iw4L/crates/xmodel_runtime" }
anim_iw4 = { path = "../iw4L/crates/anim_iw4" }
# shared third-party versions
glam = "0.33.2"          # must equal iw4L's workspace glam so xmodel_runtime types are the same type
serde = { version = "1", features = ["derive"] }
serde_json = "1"
postcard = { version = "1", features = ["alloc"] }
png = "0.17"
hound = "3"
thiserror = "2"
log = "0.4"
toml = "0.8"

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

[profile.release]
debug = 1            # keep line tables for perf work
[profile.dev.package."*"]
opt-level = 2        # iw4L math and anim decode are unusably slow at opt-level 0
[profile.dev.package.room]
opt-level = 1
```

Check the latest compatible versions of `png`, `hound`, `postcard`, `toml`, `thiserror` with `cargo search` at implementation time; pin the major versions above unless a newer major is required to build. Record the chosen versions in `docs/provenance.md`.

**Inheritance check (must do first).** iw4L crates use `edition.workspace = true`, `x.workspace = true` and `[lints] workspace = true`, resolved against `../iw4L/Cargo.toml`. Verify with `cargo tree -p weapon_iw4 -p xmodel_runtime -p movement_iw4` from `doommw2/`. If Cargo refuses (it should not, path dependencies resolve their own workspace root), apply the fallback: `tools/vendor-iw4l.sh` copies the eight crates listed in 00 D3 into `third_party/iw4l/<crate>/`, rewrites `*.workspace = true` to the concrete values from `../iw4L/Cargo.toml`, and the `[workspace.dependencies]` paths point there. The fallback is a contract change (00 D3) and must be filed in §11, not silently adopted.

### 5.3 `room/Cargo.toml` changes

```toml
[[bin]]
name = "doommw2"
path = "src/main.rs"

[dependencies]
# … existing dependencies unchanged …
dmw_format = { path = "../crates/dmw_format" }
dmw_weapons = { path = "../crates/dmw_weapons" }
dmw_viewmodel = { path = "../crates/dmw_viewmodel" }
glam.workspace = true
serde.workspace = true
toml.workspace = true
```

The `room` library name stays `room`. `room/src/main.rs`: `.with_title("DOOMMW2")`; the log line becomes `"DOOMMW2 – room-based Doom with MW2 gunplay"`. Nothing else in `main.rs` changes in this sheet.

### 5.4 IWADs

- `tools/fetch-freedoom.sh`: downloads a **pinned** Freedoom release zip from `https://github.com/freedoom/freedoom/releases` (tag in a `FREEDOOM_TAG` variable at the top of the script — 0.13.0 at the time of writing; the agent confirms the newest stable tag, pins it, and records the WAD SHA-256 values in the script), verifies the hashes, extracts `freedoom1.wad` and `freedoom2.wad` into `wads/`. Idempotent. Requires `curl` and `unzip`. Pinning matters: headless tests use map coordinates (sheet 13 §5.3) that change between Freedoom releases.
- `task fetch-wads` runs it. `wads/` is gitignored.
- Shareware `doom1.wad`: needed only by `demo_playthrough` (its BASELINE is shareware-specific). Two supported ways, documented in `README.md`: (a) `sudo apt install git-lfs && git -C ../room lfs pull && cp ../room/doom1.wad ./doom1.wad`; (b) the user supplies `doom1.wad` from their own copy. `task test` runs `demo_playthrough` only when `doom1.wad` is a real WAD (first 4 bytes `IWAD`); otherwise it prints a one-line notice and runs every other test. This is the single permitted exception to 01 §5.9's "no silent skip" rule, and the notice must name the missing file.
- Engine tests other than `demo_playthrough` use `DMW_IWAD`, default `wads/freedoom1.wad`.

### 5.5 `.env.example`

```sh
# Games root containing an `iw4` folder or symlink to your Modern Warfare 2 (2009) install.
# Same layout as iw4L's IW4L_GAMES (see ../iw4L/docs/RUN.md). Needed only by `task export`.
DMW_MW2_GAMES=/path/to/games-root
# IWAD used by `task run` and engine tests.
DMW_IWAD=wads/freedoom1.wad
```

The Taskfile loads `.env` with `dotenv: ['.env']`.

### 5.6 `Taskfile.yml`

| Task | Command(s) |
|---|---|
| `check` | `cargo check --workspace --all-targets` |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` then `tools/check-deps.sh` |
| `fmt` | `cargo fmt -p dmw_format -p dmw_weapons -p dmw_viewmodel` (never the engine wholesale, 01 §5.2) |
| `test` | `cargo test --workspace` (with the `doom1.wad` rule of §5.4 implemented inside the test) |
| `run` | `cargo run --release --bin doommw2 -- -iwad ${DMW_IWAD:-wads/freedoom1.wad} {{.CLI_ARGS}}` |
| `run-fixture` | `task run -- -fixturepack` |
| `fetch-wads` | `tools/fetch-freedoom.sh` |
| `fixture-pack` | `cargo run -p dmw_format --bin dmw-fixture -- packs/fixture` |
| `export` | `tools/check-iw4l-rev.sh && cd tools/dmw_export && cargo run --release -- --out ../../packs/mw2 {{.CLI_ARGS}}` |
| `bench` | `cargo test --release -p room --test bench -- --nocapture` (sheet 13) |
| `ci` | `check`, `lint`, `test` in order |
| `iw4l-rev` | `tools/check-iw4l-rev.sh` |

### 5.7 `room/src/dmw/config.rs` (complete in this sheet)

`DmwConfig` is the full typed schema; later sheets only read it. All fields have defaults (00 §9); unknown keys produce one `warn!(target: "dmw::config", "unknown key {path}")` each and are ignored.

```rust
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields = false)]
pub struct DmwConfig {
    pub general: GeneralCfg,
    pub weapons: WeaponsCfg,
    pub ammo: AmmoCfg,
    pub balance: BalanceCfg,
    pub render: RenderCfg,
    pub input: InputCfg,
    pub audio: AudioCfg,
    pub hud: HudCfg,
}
pub struct GeneralCfg { pub enabled: bool /* true */, pub allow_fixture_pack: bool /* false */ }
pub struct WeaponsCfg {               // value: MW2 weapon name or "vanilla"
    pub fist: String, pub chainsaw: String, pub pistol: String, pub shotgun: String,
    pub supershotgun: String, pub chaingun: String, pub missile: String, pub plasma: String, pub bfg: String,
}
pub struct AmmoCfg { pub max: [i32; 4] /* [360,64,300,12] */, pub pickup_scale: [f32; 4] /* [3.0,1.0,0.5,1.0] */ }
pub struct BalanceCfg { pub monster_damage_scale: f32, pub explosive_damage_scale: f32, pub headshot_enabled: bool,
                        pub melee_damage_scale: f32, pub alert_monsters_on_fire: bool,
                        pub sprint_speed_scale: f32 /* 1.5 */, pub weapon_move_scales: bool /* true */ }
pub struct RenderCfg { pub aspect_correct: bool, pub viewmodel_fov: f32, pub dither: DitherCfg,
                       pub directional_shading: bool, pub muzzle_flash: FlashCfg }
pub enum DitherCfg { None, Bayer4 }            // serde rename_all = "lowercase"
pub enum FlashCfg { Doom, None }               // "doom" | "none"
pub struct InputCfg { pub raw_mouse: bool /* true */, pub grab_on_start: bool /* true */ }
pub struct AudioCfg { pub mw2_volume: f32 /* 1.0 */, pub channels: usize /* 8 */ }
pub struct HudCfg { pub crosshair: bool /* true */, pub hitmarker: bool /* true */, pub ammo_counter: bool /* true */,
                    pub weapon_names: bool /* true */ }
```

Ammo arrays are serialized as tables `{ clip = 360, shell = 64, cell = 300, misl = 12 }` (custom serde) and kept as arrays indexed by `DoomAmmo` in memory.

Loading: `DmwConfig::load(explicit: Option<&Path>) -> (DmwConfig, Option<PathBuf>)` searches `-mw2config` path, `./doommw2.toml`, `$XDG_CONFIG_HOME/doommw2/doommw2.toml` (fallback `~/.config/doommw2/doommw2.toml`). Parse errors → `warn!` with file and line, defaults used. Validation clamps: `viewmodel_fov` to 40..=100, scales to 0.0..=10.0, `channels` to 1..=16; each clamp logs `warn!`.

`doommw2.example.toml` lists every key at its default with a one-line comment; a unit test parses it and asserts it equals `DmwConfig::default()`.

### 5.8 Guard scripts

- `tools/check-iw4l-rev.sh`: compares `git -C ../iw4L rev-parse --short=7 HEAD` with `IW4L_REV`; also fails if `../iw4L` has uncommitted changes under `crates/` (`git -C ../iw4L status --porcelain crates`). Exit 1 with both values printed.
- `tools/check-deps.sh`: for each crate in 01 §5.3 runs `cargo tree -e normal -p <crate> --prefix none --format '{p}'` and greps the deny list (`bevy`, `room `, `dmw_weapons`, …). Exits 1 naming the crate and the offending dependency.

### 5.9 Crate skeletons

Each new crate: `Cargo.toml` with `edition = "2024"`, `version.workspace`, `license.workspace`, `[lints] workspace = true`, the dependency list from 01 §5.3 (all of it, so later sheets do not touch manifests unless they need something new), `src/lib.rs` with the SPDX header, `#![forbid(unsafe_code)]`, a crate doc comment stating its purpose from 00 §6, and one `#[test] fn links()` that touches one item from each iw4L dependency (e.g. `weapon_iw4::WeaponCombatFacts::none()`, `glam::Mat4::IDENTITY`) so a broken path dependency fails `task test`.

`room/src/dmw/mod.rs` declares all submodules, defines `log_once!` (01 §5.7) and `with_state` as a stub returning `None`. `hooks.rs` contains every function of 00 §7.5 with its final signature and a body returning the "not handled" value (`false`, `None`, unchanged inputs). No Doom file is edited in this sheet; sheet 08 adds the call sites.

### 5.10 Licence and provenance files

- `LICENSE`: GPL-3.0 full text (from https://www.gnu.org/licenses/gpl-3.0.txt; the agent may download it or copy from a local system licence directory such as `/usr/share/common-licenses/GPL-3`).
- `LICENSE-GPL-2.0`: kept from room. `LICENSE-APACHE-2.0`: copied from `../iw4L/LICENSE`.
- `NOTICE`: project line, "Contains code from room (GPL-2.0-or-later) …", "Contains code ported from IW4L (Apache-2.0) … Copyright 2026 vladtrc", a table header `Ported file | Origin` (rows added by later sheets), and the statement that no game assets are distributed.
- `docs/provenance.md`: §5.1 item 4 plus a "Ported files" table mirroring NOTICE.

### 5.11 `tools/dmw_export` skeleton

Own workspace:

```toml
[package]
name = "dmw_export"
edition = "2024"
license = "GPL-3.0-or-later"

[workspace]   # standalone

[dependencies]
dmw_format = { path = "../../crates/dmw_format" }
assets = { path = "../../../iw4L/crates/assets" }
asset_game = { path = "../../../iw4L/crates/asset_game" }
asset_model = { path = "../../../iw4L/crates/asset_model" }
asset_anim = { path = "../../../iw4L/crates/asset_anim" }
asset_audio = { path = "../../../iw4L/crates/asset_audio" }
asset_material = { path = "../../../iw4L/crates/asset_material" }
asset_transport = { path = "../../../iw4L/crates/asset_transport" }
asset_core = { path = "../../../iw4L/crates/asset_core" }
frame = { path = "../../../iw4L/crates/frame" }
master_protocol = { path = "../../../iw4L/crates/master_protocol" }
weapon_iw4 = { path = "../../../iw4L/crates/weapon_iw4" }
xmodel_runtime = { path = "../../../iw4L/crates/xmodel_runtime" }
dpvs_iw4 = { path = "../../../iw4L/crates/dpvs_iw4" }
bevy = { version = "0.20", default-features = false }   # must match iw4L's workspace bevy version exactly
anyhow = "1"
clap = { version = "4", features = ["derive"] }
serde_json = "1"

[patch.crates-io]
# copied from ../iw4L/Cargo.toml at IW4L_REV
quinn-proto = { git = "https://github.com/quinn-rs/quinn", rev = "f650e0f213b01ea7bebd60023e5df7df65e5c2bd" }

[profile.dev]
opt-level = 3
[profile.release]
debug = 1
```

`src/main.rs` parses `--out <dir>` and `--help` with clap, prints `dmw_export <version> (iw4L <IW4L_REV>)` and exits 0. Sheet 04 fills it in. `task check` does not build the exporter (it is excluded); `task export-check` = `cd tools/dmw_export && cargo check` and is part of `task ci` only when `DMW_BUILD_EXPORTER=1` (the Bevy build is slow and not needed by most sheets).

### 5.12 `.gitignore` additions

```text
/target
/tools/dmw_export/target
/packs/
/wads/
/.env
*.dsg
/doommw2.toml
```

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| `../iw4L` missing | `cargo check` fails on the path dependency; `README.md` says to clone iw4L next to doommw2 at `IW4L_REV` |
| iw4L at another revision | `task iw4l-rev` fails; `task export` refuses; `task check` still runs (warning printed by the Taskfile precondition) |
| Freedoom download fails | script exits 1 with the curl error; no partial WAD left behind (download to temp, move on success) |
| `doom1.wad` is an LFS pointer | `demo_playthrough` reports and skips per §5.4 |
| Config file with a syntax error | defaults + `warn!` with location (§5.7) |

## 7. Logging

`dmw::config`: `info!("config: {path}")` or `info!("config: defaults (no doommw2.toml found)")` once at startup; `warn!` per unknown key or clamp.

## 8. Licensing and asset notes

01 §5.10. This sheet must not copy any MW2 files, and must not copy iw4L source (only depend on it by path).

## 9. Tests

- `crates/*/src/lib.rs` `links()` tests (§5.9).
- `room/src/dmw/config.rs`: example file round-trip equals default; unknown key warns (capture with a test logger) and still parses; each clamp.
- `tools/check-deps.sh` exits 0 on the scaffold.

## 10. Acceptance criteria

- [ ] `git log` shows the unmodified import commit first.
- [ ] `task check`, `task lint`, `task test` pass with no MW2 data and with Freedoom fetched; `demo_playthrough` passes when a real `doom1.wad` is present and is reported-and-skipped when it is an LFS pointer.
- [ ] `task run -- -warp 1 1` (Freedoom) opens a window titled `DOOMMW2` and plays exactly like room.
- [ ] `cargo tree -p weapon_iw4` resolves from `doommw2/` (inheritance check §5.2) — or the fallback was filed in §11.
- [ ] `cd tools/dmw_export && cargo build` succeeds and prints the version line.
- [ ] Every file in 00 §6.4 exists; every 00 §7.5 hook function exists in `hooks.rs` with its final signature.
- [ ] `doommw2.example.toml` round-trips to `DmwConfig::default()`.
- [ ] Licence files, NOTICE, provenance, AGENTS.md (01 §5.13 verbatim), CLAUDE.md exist.

## 11. Contract changes requested

Contract change (00), for the lead to fold into a Revision:

1. **Minimum Rust version.** 00 §4.1 (`rust-toolchain.toml`: "channel = stable, >= 1.85") and 01 §5.1 ("Minimum 1.85") hold for the engine and the three new crates, which build on 1.95. The exporter does not: iw4L pins Bevy 0.20.0, which declares `rust-version = 1.97.1` (`cargo build` in `tools/dmw_export` refuses on 1.95.0). State the exporter's minimum as 1.97.1 (or "whatever iw4L's Bevy pin requires") in 00 §4.1 / D5. The iw4L inheritance fallback (§5.2) was not needed: path dependencies resolve their own workspace root.

Deviations from this sheet's own text (not 00 contracts), made because the literal text cannot pass on this machine; reported so the lead can amend the sheet:

2. `task check` / `task test` use `--workspace --exclude c2rust-intermediate`. That reference crate has `#![feature(extern_types)]` and needs nightly, so `--workspace` fails on stable (it is also outside room's own `default-members`).
3. `task lint` does not run `cargo clippy --workspace --all-targets -- -D warnings`. The copied Doom port has hundreds of pre-existing findings (static_mut_refs, precedence, unnecessary unsafe) that its crate-level allow list does not cover, and c2rust-intermediate does not build on stable. `task lint` instead runs `-D warnings` on the three new crates, `tools/check-room-lint.sh` (fails on any clippy diagnostic inside `room/src/dmw/`, or a failed build), and `tools/check-deps.sh`.
4. `tools/check-deps.sh` applies the "no iw4L crates" rule for `dmw_viewmodel` to its direct dependencies only, because `dmw_format` (allowed to use `weapon_iw4`) is a dependency of `dmw_viewmodel` and drags it in transitively.
5. `task run` passes `-mb 32`. With room's 6 MiB default zone, Freedoom aborts at startup with `Z_Malloc: failed on allocation` (reproduced on the unmodified import commit). Headless engine tests that load Freedoom (sheet 13) need the same flag, or a change to `DEFAULT_RAM` in `i_system.rs`.
6. `crates/dmw_format/src/consts.rs` (sheet 03's file, 00 §6.1) was created with 00 §5.1–5.4 verbatim because `room/src/dmw/consts.rs` must re-export it in this sheet. Sheet 03 should extend it, not recreate it.
7. `DG_GetMouse` (00 §7.5 row for `I_GetEvent`) is a C-ABI platform callback owned by sheet 05; there is no stub for it in `hooks.rs`.
8. The engine overrides the window title through `DG_SetWindowTitle` once Doom initialises (the title becomes the IWAD name, for example "Freedoom: Phase 1"). Only the initial title is `DOOMMW2`; room does the same with `room`.
