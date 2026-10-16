# Provenance

What was copied from where, and which versions were chosen. Updated by every sheet that ports
code or adds a dependency family (`docs/specs/01-engineering-standards.md` section 5.10).

## Upstream trees

| Tree | URL | Full commit | Use |
|---|---|---|---|
| room | https://github.com/sunsided/room | `904acac5fb7d89d7545082c3fab71408176d2404` | copied into this repository (import commit `chore: import room at 904acac`) |
| iw4L | https://github.com/vladtrc/iw4L | `fc3d8eaf6045cb131eb74cd41954f0c4922b9913` (`IW4L_REV` = `fc3d8ea`) | read-only sibling checkout `../iw4L`, consumed by path dependencies; nothing is copied |

Import date: 2026-10-15 (the commit hashes were read with `git -C ../room rev-parse HEAD` and
`git -C ../iw4L rev-parse HEAD`).

## Import of room

Procedure: `rsync -a` of `../room/` into this directory, then `git init`, first commit unmodified.

Excluded paths: `target/`, `.git/`, `.claude/`, `.agents/`, `.github/`, `CLAUDE.md`, `AGENTS.md`,
`.pre-commit-config.yaml`.

Handled after the import commit:

- room's `docs/specs/` and `docs/plans/` files moved to `docs/room-upstream/` so they do not mix with the DOOMMW2 specs.
- `Taskfile.dist.yaml` replaced by `Taskfile.yml`; `README.md` rewritten.
- `doom1.wad` and `soundfonts/*.sf2` are Git LFS pointers upstream and were copied as pointers. `.gitattributes` keeps the LFS rules.

## Chosen dependency versions

Resolved in `Cargo.lock` on 2026-10-15 for the workspace-level pins in `Cargo.toml`
(major versions as specified in sheet 02 section 5.2; no newer major was needed to build).

| Crate | Requirement | Locked |
|---|---|---|
| glam | `0.33.2` (must equal iw4L's workspace glam) | 0.33.12 |
| serde | `1` | 1.0.228 |
| serde_json | `1` | 1.0.149 |
| postcard | `1` (alloc) | 1.1.3 |
| png | `0.17` (0.18.1 exists; iw4L uses 0.18, not shared with these crates) | 0.17.16 |
| hound | `3` | 3.5.1 |
| thiserror | `2` | 2.0.18 |
| log | `0.4` | 0.4.30 |
| toml | `0.8` | 0.8.23 |

## Ported files

Mirrors the table in `NOTICE`.

| Ported file | Origin |
|---|---|
| `crates/dmw_weapons/src/view_kick.rs` | iw4L crates/render_anim/src/anim/view_kick_state.rs (`ViewKickState`) at fc3d8ea |
| `crates/dmw_weapons/src/view_sway.rs` | iw4L crates/render_anim/src/anim/view_sway.rs (`ViewSwayState`) at fc3d8ea |
| `crates/dmw_weapons/src/breath.rs` | iw4L crates/movement_iw4/src/breath.rs (`update_hold_breath`) at fc3d8ea |
| `crates/dmw_weapons/src/spread.rs` | iw4L crates/sim/src/combat.rs (`spread_direction_on_plane`, lines 1125-1150) at fc3d8ea |
| `crates/dmw_weapons/src/sim.rs` | iw4L crates/sim/src/combat.rs (`advance_weapon_command` call sequence and `WeaponCmd` construction) and crates/sim/src/step.rs (`pmove_context` values) at fc3d8ea |
| `room/src/dmw/ballistics.rs` | iw4L crates/sim/src/bullet.rs (`bullet_damage_at_distance`) at fc3d8ea |
| `crates/dmw_weapons/src/clip_scheduler.rs` | iw4L crates/asset_anim/src/clip_scheduler.rs (`ClipScheduler`) at fc3d8ea |
| `crates/dmw_weapons/src/controller.rs` | iw4L crates/render_anim/src/anim/viewmodel_controller.rs (`ViewmodelController`) and crates/asset_game/src/weapon_animations.rs (`WeaponAnimSlot`, `AdsOverlayConvention`) at fc3d8ea |
| `crates/dmw_weapons/src/anim.rs` | iw4L crates/render_anim/src/anim/fpv.rs (`tick_equipped_fpv_with_extra_events`, `observe_weap_anim_edge_on`) at fc3d8ea |
| `crates/dmw_weapons/src/pose.rs` | iw4L crates/render_anim/src/anim/fpv_rig.rs (`PreparedFpvRig::pose_clips`) at fc3d8ea |
| `crates/dmw_weapons/src/placement.rs` | iw4L crates/render_anim/src/occupancy/fpv_present.rs (`apply_fpv_placement`, lines 1032-1250) and crates/render_anim/src/occupancy/view_kick.rs (`iw_view_placement_to_bevy_camera_local`) at fc3d8ea |

## Adapted from room (GPL)

- `room/src/dmw/ballistics.rs` `dmw_shoot_traverse` and `wall_hit` are adapted from room's `PTR_ShootTraverse` / `goto_hitline` (`room/src/doom/p_map.rs`, GPL-2.0-or-later): same line, opening and thing tests, recording the hit instead of damaging it. `dmw_radius_thing` adapts `PIT_RadiusAttack` with MW2 damage numbers.

## Exporter (sheet 04)

`tools/dmw_export` copies no iw4L code; it calls iw4L's public API (path dependencies, see the
crate's `Cargo.toml`). How each part of the pack is read, for the next person who has to find it:

| Pack part | iw4L call path |
|---|---|
| headless load | `assets::load_prepared_match` with the arguments of `bootstrap/src/launch.rs` `run_export_gltf` (`ContentFlags::IW4`, `find_runtime_common_mp`) |
| weapon names | `WeaponRegistry::resolve_index`; pack names are `script_name_of` (`m4_mp`), the registry's own `name_of` drops the `_mp` |
| display names | `PreparedMatch::strings` is empty after a headless load (the session fills it later), so the exporter reads `asset_game::load_mp_localized_strings` and looks keys up in the weapon's identity namespace like `hud/src/weapon_name.rs` |
| sounds | `PreparedMatch::sound` holds only the map zone's aliases; the session composes the runtime zones at match start. The exporter uses `asset_audio::load_mp_sound_bank` (same composition). Streamed variants are read with `asset_transport::NamespaceSoundIwd::read_sound`, as `audio/src/clip_store.rs` `prepare_streamed` |
| textures | colour maps are already decoded by the load (`match_walk.rs` runs `decode_material_color_maps`, the "images/common_mp FPV" stage), so no extra image decode is driven; `asset_material::decoded_image_top_level_rgba8` as in `assets/src/gltf_export.rs` |
| scope overlay | `WeaponRegistry::overlay_image_of` → `asset_material::decode_ui_image` (`iw4:material/<image>`), as `hud/src/iris.rs` |
| surface hiding | part hide words are model-local, so a surface is hidden when its part bits AND the words are non-zero (base 0, `render_anim/anim/fpv_rig.rs`); `DObj::surface_visible` lifts by the model's bone base and would not match these words |

iw4L writes a decoded-image cache to `./iw4l-artifacts` (or `$IW4L_ARTIFACTS_DIR`). The exporter
changes its working directory to `<out>/../.dmw_export-cache` (a temporary directory for
`--list`) so that MW2-derived bytes never land in the caller's directory.
