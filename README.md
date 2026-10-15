# DOOMMW2

Doom (the `room` Rust engine) with Modern Warfare 2 gunplay: MW2's real weapon rules from iw4L, real 3D MW2 viewmodels drawn by a software rasterizer through Doom's palette and light tables.

Status: workspace scaffold (sheet 02) is in place; gameplay sheets are not built yet. The build specs start at [docs/specs/README.md](docs/specs/README.md). Agents also read [AGENTS.md](AGENTS.md).

MW2 game data never enters this repository. An MW2 installation is only needed to export real weapons (`task export`, sheet 04); everything else is built and tested against a synthetic fixture pack and Freedoom.

## Layout

- `room/`, `doomgeneric-sys/`, `vendor/`, `c2rust-intermediate/`, `soundfonts/`: the engine, copied from `room` at `904acac` (see `docs/provenance.md`).
- `room/src/dmw/`: all DOOMMW2 engine glue.
- `crates/dmw_format`, `crates/dmw_weapons`, `crates/dmw_viewmodel`: pure Rust crates.
- `tools/dmw_export/`: separate workspace, MW2 install to pack exporter (links iw4L and Bevy).
- `docs/specs/`: the build specs. `docs/room-upstream/`: room's own design notes.

## Prerequisites

- Rust stable 1.85 or newer (`rust-toolchain.toml`), a C compiler, `pkg-config`.
- Linux system libraries for the engine: ALSA development files (`libasound2-dev` on Debian and Ubuntu); winit and wgpu load the windowing and GPU libraries at run time.
- [Task](https://taskfile.dev) for the commands below.
- `curl` and `unzip` for `task fetch-wads`.
- A sibling checkout of iw4L, read-only, at the pinned revision in `IW4L_REV`:

  ```sh
  git clone https://github.com/vladtrc/iw4L ../iw4L
  git -C ../iw4L checkout "$(cat IW4L_REV)"
  ```

  Without it `cargo check` fails on the path dependencies. `task iw4l-rev` verifies the checkout.
- The exporter additionally needs the Bevy build dependencies of iw4L (Wayland, xkbcommon, udev development files on Linux) and a Rust toolchain new enough for the Bevy version iw4L pins (Bevy 0.20.0 declares Rust 1.97.1; on older toolchains `cargo build --ignore-rust-version` was the only thing tried).

## Commands

| Command | What it does |
|---|---|
| `task check` | `cargo check --workspace --all-targets` |
| `task lint` | clippy with warnings denied, then `tools/check-deps.sh` (crate boundaries) |
| `task test` | all tests; `demo_playthrough` only when `doom1.wad` is a real WAD |
| `task fetch-wads` | download the pinned Freedoom IWADs into `wads/` |
| `task run -- -warp 1 1` | run the game on Freedoom (`DMW_IWAD` overrides the IWAD); passes `-mb 32` because room's default 6 MiB zone is too small for the Freedoom IWADs |
| `task fixture-pack` | write the synthetic fixture pack (after sheet 03) |
| `task export` | export an MW2 install to `packs/mw2` (needs `DMW_MW2_GAMES`, after sheet 04) |
| `task ci` | `check`, `lint`, `test` |

Copy `.env.example` to `.env` to set `DMW_MW2_GAMES` and `DMW_IWAD`. Copy `doommw2.example.toml` to `doommw2.toml` to change settings.

## Shareware doom1.wad

The repository contains `doom1.wad` as a Git LFS pointer. Only `room/tests/demo_playthrough.rs` needs the real shareware WAD, because its baseline is specific to it. `task test` skips that one test with a notice while `doom1.wad` is not a real WAD (it does not start with `IWAD`). To get it, either:

1. `sudo apt install git-lfs && git -C ../room lfs pull && cp ../room/doom1.wad ./doom1.wad`, or
2. copy `doom1.wad` from your own copy of the shareware release over the pointer file.

Do not commit the real file.

## Licence

GPL-3.0-or-later for the combined work (`LICENSE`). Doom and room code is GPL-2.0-or-later (`LICENSE-GPL-2.0`). Code ported from iw4L is Apache-2.0 (`LICENSE-APACHE-2.0`). See `NOTICE` and `docs/provenance.md`.
