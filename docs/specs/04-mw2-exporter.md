# 04 — MW2 Asset Exporter (`tools/dmw_export`)

Owner: build agent (wave 3)
Status: implementation-ready, but **cannot be acceptance-tested on the developer machine until an MW2 install is available** (00 D22). Everything in §9 marked "fixture" runs without MW2.
Depends on: 00 (§2 D3, D5; §9.1 roster), 01 (§5.3 boundaries), 02 (§5.11 skeleton), 03 (format v1 — the exporter's only output)

This sheet specifies the offline tool that opens the user's own Modern Warfare 2 (2009) multiplayer installation through iw4L's asset pipeline, headlessly, and writes a DOOMMW2 pack (sheet 03). It is the only code in the project that links Bevy or touches MW2 files.

---

## 1. Purpose

Turn "an MW2 install" into "a `packs/mw2/` directory" for a chosen roster of weapons, with:

- combat, kick, sway, placement, HUD, movement, projectile and sound facts exactly as iw4L publishes them to its own simulation;
- each weapon's composed first-person assembly (view hands + gun + attachments) as one skinned mesh with its DObj skeleton;
- every animation clip bound to a weapon animation slot, with its track→bone map;
- colour-map textures as RGBA8 PNG, the scope overlay image, and every referenced sound as PCM WAV;
- a machine-readable report of everything that was skipped and why.

## 2. Scope / Out of scope

In scope: CLI; MW2 discovery via `DMW_MW2_GAMES`; headless load with `assets::load_prepared_match`; weapon resolution; fact extraction; FPV assembly flattening; clip extraction; texture decode; overlay; sound decode (loaded PCM and streamed IWD WAV); pack writing with atomic replace; report; `--list` mode.

Out of scope: any modification to iw4L; MW3/BO1/BO2 lanes (IW4 only); attachments/camos (base weapon only); akimbo left-hand rigs; world models; FX (muzzle flash particles, tracers); reticle materials other than the scope overlay; localisation beyond English display names.

## 3. Dependencies

| From | What |
|---|---|
| 03 | every output type; `PACK_FORMAT_VERSION`; validation (run before writing) |
| iw4L `bootstrap/src/launch.rs:305` `run_export_gltf` | the exact headless load sequence to copy |
| iw4L `assets/src/gltf_export.rs` | material → colour-map image → `decoded_image_top_level_rgba8` path |
| iw4L `assets` | `load_prepared_match`, `PreparedMatch` (`weapons`, `fpv_meshes`, `xanims`, `materials`, `sound`, `strings`, `lochit_table`) |
| iw4L `asset_game` | `WeaponRegistry` (`resolve_index`, `bind_published_row`, `name_of`, `display_name_key_of`, `fpv_assemblies_of`, `authored_weapon_sound`, `weapon_sound_alias`, `notetrack_actions_of`, `notetrack_sound_aliases_of`, `equipment_facts_of`, `fpv_clip_tracks`, `sz_xanim_edges_of`), `BoundWeapon` (`combat_facts`, `fpv_facts`, `hud_facts`, `event_facts`, `movement_scales`), `WeaponAnimations::from_registry`, `WeaponAnimSlot`, `WeaponSoundSlot::ALL`, `FpvAssembly` (`dobj`, `parts`, `view_bone`, `camera_bone`, `tags`, `collapsed_bones`, `compose_tracks`) |
| iw4L `asset_model` | `FpvMeshCatalog`, `FpvMeshEntry` (`skel`, `material_edges`, `material_present_name`), `ModelSkel` (`positions`, `normals()`, `uvs()`, `indices`, `vert_skin`, `surface_*` ranges, `surface_part_bits`) |
| iw4L `asset_material` | `MaterialDefinitions` (`materials`, `agreed_draw_mode`, `agreed_alpha_test_cutoff`, `cull_face`), `TS_COLOR_MAP`, `decoded_image_top_level_rgba8` |
| iw4L `asset_audio` | `SoundCatalog` (`sound_in`, `pcm_for_variant`, `streamed_for_variant`), `LoadedSoundPcm` (`format()`, `bits()`, `channels()`, `rate`, `samples`, `encoded_bytes()`), `MSS_PCM` |
| iw4L `dpvs_iw4` | `skin_blend_weight(raw: u16) -> f32` |
| iw4L `asset_transport` | `games_root_from_env`, `find_zone_file`, `find_runtime_common_mp`, `zone_game_for_path`, `NamespaceTrees`, `LoadProgress` |
| iw4L `frame`, `master_protocol` | `AssetFamilies::from_inventory(ContentFlags::IW4.0, &UiMenuDvars::default())` |

Every iw4L call above was located in the source at fc3d8ea while writing this sheet; signatures that take extra parameters (e.g. `fpv_assemblies_of(index, axis)`) are noted where used. If a call turns out to be `pub(crate)`, the agent finds the public path iw4L itself uses for the same data (search callers in `render_anim`, `sim`, `assets`) and records it in §11; it never patches iw4L.

## 4. File inventory

| Path | Content |
|---|---|
| `tools/dmw_export/src/main.rs` | CLI (§5.1), orchestration, exit codes |
| `src/load.rs` | headless MW2 load (§5.3) |
| `src/roster.rs` | weapon name resolution, close-match suggestions (§5.2) |
| `src/facts.rs` | `WeaponFacts` extraction (§5.4) |
| `src/viewmodel.rs` | assembly → `ViewmodelFile` (§5.5) |
| `src/clips.rs` | clips + track maps (§5.6) |
| `src/textures.rs` | material → PNG (§5.7) |
| `src/sounds.rs` | alias → WAV (§5.8) |
| `src/write.rs` | atomic pack writing (§5.9) |
| `src/report.rs` | `report.json` (§5.10) |
| `tests/convert.rs` | fixture-level tests (§9) |

## 5. Detailed design

### 5.1 CLI

```text
dmw_export --out <dir> [--zone <zone>] [--weapons <a,b,c>] [--list] [--keep-going] [--axis-hands]
```

| Flag | Default | Meaning |
|---|---|---|
| `--out` | required (Taskfile passes `packs/mw2`) | pack directory, replaced atomically |
| `--zone` | `mp_rust` | map zone whose load brings in `common_mp`; any IW4 MP map works |
| `--weapons` | the 00 §9.1 roster + `cheytac_mp,aa12_mp,kriss_mp,deserteagle_mp` | comma list of `*_mp` names |
| `--list` | off | print every published weapon name with display name and whether it has an FPV assembly, then exit 0 (no pack written) |
| `--keep-going` | off | a weapon that fails is reported and skipped instead of aborting |
| `--axis-hands` | off | pass `axis = true` to `fpv_assemblies_of` (alternate faction view hands) |

Exit codes: 0 success; 1 usage; 2 MW2 not found / wrong game; 3 load refused; 4 a requested weapon failed (without `--keep-going`); 5 write failure.

`DMW_MW2_GAMES` (00 §4.2) is copied into `IW4L_GAMES` in the process environment before any iw4L call (iw4L's `games_root_from_env` reads `IW4L_GAMES`, then its own default). If neither is set, exit 2 with the iw4L "MW2 Multiplayer was not found" style message listing searched roots (`asset_transport::search_roots`).

### 5.2 Roster resolution

For each requested name: `registry.resolve_index(name)`; `Ok(Some(i))` → use `i` as the row index for every `*_of(i)` call and `registry.bind_published_row(i)` for `BoundWeapon`. `Ok(None)`/`Err(UnknownWeaponName)` → error listing up to five closest published names by edit distance (from `registry.published_weapons()` names). The output order of weapons in the manifest is the order of `--weapons`.

### 5.3 Headless load

Copy `run_export_gltf`'s sequence (bootstrap/src/launch.rs:305) with three differences: no `bootstrap` dependency, no `diag` announce, keep the whole `PreparedMatch`:

1. `games = games_root_from_env()?`
2. `found = find_zone_file(&games, zone)?`; `zone_game_for_path(&found.path) == Some(ZoneGame::Iw4)` else exit 2 ("IW4 only").
3. `common_mp = find_runtime_common_mp(&games, &found.path).map(|z| z.path)` (a `Result`; pass `Ok/Err` exactly as `load_prepared_match` expects — mirror launch.rs).
4. `families = frame::AssetFamilies::from_inventory(master_protocol::ContentFlags::IW4.0, &frame::UiMenuDvars::default())?`
5. `bevy::tasks::futures_lite::future::block_on(assets::load_prepared_match(Ok(found.path), common_mp, LoadProgress::default(), families))` → `MatchLoadOutcome::Ready(prepared)` else exit 3 with the reason.
6. Log (stdout, the exporter is a CLI) the zone, common zone, weapon count (`registry.published_weapons().count()`), and load time.

Image decode note: `gltf_export` reads `catalog.images[i].decoded`. If at fc3d8ea images are decoded lazily by a separate image plan (the field is `None` right after load), the exporter must drive the same decode the session uses (look for the image plan executor called by `assets::session_load` and call it synchronously for the needed image indices only). Record what was needed in `docs/provenance.md`.

### 5.4 Facts

Per weapon `i` (bound `b`):

| `WeaponFacts` field | Source |
|---|---|
| `name` | `registry.name_of(i)` |
| `display_name` | `registry.display_name_key_of(i)` looked up in `prepared.strings` (English); fallback `name` |
| `combat` | `b.combat_facts(WeaponHostRules::default(), prepared.lochit_table)` — refusal → weapon error |
| `kick` | `b.fpv_facts()?.kick` (`WeaponKickFacts`, copy every field) |
| `sway.hip` / `sway.ads` | `fpv.sway.hip_params()` / `fpv.sway.ads_params()` |
| `fpv.*` | `WeaponFpvFacts` fields of the same names; `ads_overlay_convention` = `format!("{:?}", fpv.ads_overlay_convention())` |
| `hud` | `b.hud_facts()?` subset |
| `movement` | `b.movement_scales()` → `(move_speed_scale, ads_move_speed_scale, sprint_duration_scale)` |
| `anim_times` | `WeaponAnimations::from_registry(&registry, i, &xanims)` time fields |
| `projectile` | `registry.equipment_facts_of(i)` when `projectile_speed > 0`, plus `b.event_facts()` explosion fields |
| `sounds.<slot>` | `registry.weapon_sound_alias(i, slot, sound_catalog)` for each `WeaponSoundSlot::ALL` (resolved alias); `authored_weapon_sound` as fallback when the bank lookup fails (report it) |
| `sounds.notetracks` | `registry.notetrack_actions_of(i)` → `(note, action.sound_alias)` where present |
| `has_overlay` | §5.7.3 |

A weapon whose `fpv_facts()` is `None` is exported with default kick/sway/fpv values and `has_viewmodel = false`, and reported; sheet 08 refuses to map such a weapon (it would be invisible).

### 5.5 Viewmodel flattening

1. `sides = registry.fpv_assemblies_of(i, axis_hands)` → `FpvSideAssemblies`. Choose the assembly: `rocket` if present, else `melee` if present, else `bare`. Record the choice in the report.
2. `dobj = assembly.dobj()` → `Skeleton.bones` (copy `name`, `model`, `parent`, `bind_rotation` as `[x,y,z,w]`, `bind_translation`, `bind_world.to_cols_array()`, `no_scale`), `models`, `duplicates`. `view_bone`, `camera_bone`, `tags().flash`, `tags().brass`, `collapsed_bones` copied.
3. For each `part` in `assembly.parts()`: `entry = fpv_meshes.get_at(part.model.order())`, `skel = entry.skel` (LOD 0 only: `skel.surfaces_for_lod(0)`). For each surface `s` in LOD 0:
   - Skip if hidden: `part.hide` is `Some(words)` and `!dobj.surface_visible(part_model_slot, &skel.surface_part_bits[s], &HidePartBits::from_words(words))`. Count in report.
   - Vertices `skel.surface_vertex_ranges[s]`: positions from `skel.positions`, normals from `skel.normals()`, uvs from `skel.uvs()`.
   - Skin: `vert_skin[v]`: bone ids are model-local; global = `part.bone_base + local`. Weights: rigid vertex (`has_only_primary_weight()` or `is_rigid` path) → `[1,0,0,0]`; otherwise extras `w_k = dpvs_iw4::skin_blend_weight(extra_weight(k))` for `k` with `has_extra_weight(k)`, primary `w_0 = 1 − Σ w_k`, but `w_0 = 0` when `primary_is_zero()`. Renormalise to sum 1 and record any vertex whose raw sum deviated by > 1e-2.
   - Indices `skel.surface_index_ranges[s]` rebased into the merged vertex array.
   - Material: `entry.material_edges[s].bound_index()` → material row in `prepared.materials.population.materials`; draw mode → `AlphaMode` (`Opaque`, `AlphaTest` → `Test{cutoff}` from `agreed_alpha_test_cutoff`, blend modes → `Blend`); `cull_face` → `CullMode` (`Front` is reported and exported as `None`). Texture per §5.7.
   - `role` from `part.role` (`Hands`, `Gun`, `Attachment`, `Rocket`, `Knife`); a surface whose material name matches iw4L's `leftover_scope_surf_is_lens` rule (render_anim/anim/fpv_rig.rs:132 — reproduce the string rule) gets `ScopeLens`.
4. Run `dmw_format::validate` on the result before writing.

Positions stay in MW2 model space; nothing is converted to Doom units (sheet 10 works in MW2 eye space).

### 5.6 Clips

1. `anims = WeaponAnimations::from_registry(&registry, i, &prepared.xanims)`.
2. For each slot `k` in `0..WEAPON_ANIM_SLOTS` with `anims.clip(WeaponAnimSlot::from_index(k)?)`: deduplicate clips by `Arc::ptr_eq` (or by `clip_orders()[k]`), mirror `xmodel_runtime::AnimClip` into `ClipData` field-for-field (03 §5.6), and `slots[k] = Some(clip_idx)`.
3. Track maps: `assembly.compose_tracks(clip_index, clip, registry.fpv_clip_tracks())` where `clip_index` is the xanim catalog order (`clip_orders()[k]`). Convert `Option<usize>` per track to `Option<u16>`. A clip whose map binds zero bones is dropped from its slot and reported (sheet 10 then treats the slot as missing).
4. Notetracks stay inside `ClipData::notifies`; their sound links live in `WeaponFacts::sounds.notetracks`.

### 5.7 Textures

#### 5.7.1 Colour map

For a material row: exactly one texture binding with `semantic == TS_COLOR_MAP` (else report `MaterialColorMapUnavailable`/`Ambiguous` and export the surface with `texture = None`). Decode with `asset_material::decoded_image_top_level_rgba8(decoded)` (as `gltf_export.rs` does). Deduplicate by image index per weapon; write `textures/<n>.png`.

#### 5.7.2 Size

Downscale any texture larger than 1024 on either axis by repeated 2× box filtering in linear-light RGB (alpha averaged) until ≤ 1024 — the viewmodel covers at most ~200×150 Doom pixels, so larger textures only cost load time. Report original and exported sizes.

#### 5.7.3 Scope overlay

When `fpv.overlay_reticle != 0`: the overlay material is the weapon's ADS overlay material (iw4L `WeaponReticleAssets`, `registry.reticle_of(i)`; follow how `hud`/`render_frontend` resolves the overlay image for `AdsOverlayConvention::Overlay`). Decode its colour map to `overlay.png`, `has_overlay = true`. If it cannot be resolved, `has_overlay = false` and report; sheet 12 then draws a generic Doom-palette ring.

### 5.8 Sounds

Collect the alias set: every `Some` in `WeaponSounds` plus every notetrack alias, plus the fixed extras `["mp_hit_alert"]` (hitmarker, sheet 12 §5.2) when present in the catalog, deduplicated. For each alias:

1. `sound = catalog.sound_in(IW4 namespace, alias)`; for each variant `v` in `0..sound.aliases.len()`:
2. Loaded: `catalog.pcm_for_variant(ns, alias, v)` → if `format() == MSS_PCM`: raw little-endian PCM from `encoded_bytes()` with `bits()` (8 or 16) and `channels()` (1 or 2) at `rate` → WAV 16-bit. Other formats → skip variant, report `UnsupportedLoadedFormat(format)`.
3. Streamed: `catalog.streamed_for_variant(ns, alias, v)` → `(ns, dir, name)`; read the bytes through iw4L's IWD reader the same way `audio/src/clip_store.rs::prepare_streamed` does (`NamespaceSoundIwd::read_sound`); if the bytes are a RIFF WAVE PCM file, re-encode to 16-bit WAV; other codecs → skip and report.
4. Volume/pitch/distances: take the variant's alias fields if exposed by `CapturedAlias` (volume min/max, pitch min/max, distance min/max); otherwise defaults (1.0, 1.0, 0, 0). Record which.
5. An alias with zero exportable variants is omitted from `manifest.sounds` and reported; the game logs a warning when it is referenced (03 §5.8).

### 5.9 Writing

Write into `<out>.tmp-<pid>/`, run `dmw_format::Pack::open` on it (full validation, plus `load_viewmodel` for every weapon and `load_sound` for every alias), then rename the old `<out>` to `<out>.old-<pid>`, rename tmp to `<out>`, delete the old one. Any failure leaves the previous pack untouched.

### 5.10 Report

`report.json`: `{ generator, iw4l_rev, zone, common_zone, started, duration_ms, weapons: [{ name, ok, assembly: "rocket|melee|bare|none", surfaces: {exported, hidden, untextured}, clips: {slots_bound, dropped}, textures: [{n, w, h, orig_w, orig_h}], sounds: {exported, skipped:[{alias, variant, reason}]}, warnings: [..] }], errors: [..] }`. Printed summary: one line per weapon (`ok m4_mp: 7 surfaces, 24 clips, 5 textures, 11 sounds`).

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| No MW2 found | exit 2, list searched roots |
| Zone is not IW4 | exit 2 |
| `load_prepared_match` refused/canceled | exit 3 with reason |
| Unknown weapon name | exit 4 with suggestions (or skip with `--keep-going`) |
| `combat_facts` refusal | weapon error (`WeaponCombatRefusal` Debug text) |
| No FPV assembly | weapon exported with `has_viewmodel=false`, warning |
| Hidden/LOD>0 surfaces | skipped, counted |
| Vertex weights way off (> 1e-2) | renormalised, counted, warning |
| Material without colour map | surface untextured, warning |
| Unsupported sound codec | variant skipped, warning |
| Disk full / permission | exit 5, previous pack intact (§5.9) |

## 7. Logging

The exporter prints to stdout (it is a CLI; `print_stdout` is allowed in `tools/dmw_export/src/main.rs` and `report.rs` only via `#[allow(clippy::print_stdout)]` on those functions). iw4L's own `diag` logging goes wherever iw4L sends it (its log file under `iw4l-artifacts/`); the exporter prints that path if iw4L returns it.

## 8. Licensing and asset notes

The exporter reads a user-owned installation and writes derived data only to a gitignored directory on the same machine. It must not upload, cache elsewhere, or embed any MW2 data in the binary. The pack contains copyrighted material derived from MW2 and is for the user's personal use; `README.md` states this.

## 9. Tests

| Id | Test | Needs MW2 |
|---|---|---|
| T4.1 | weight conversion helper: rigid, two-bone, primary-zero, renormalisation cases (pure function over `VertSkin`-like inputs constructed in the test) | no |
| T4.2 | texture downscale: 2048² → 1024², colours averaged in linear light | no |
| T4.3 | PCM → WAV: 8-bit unsigned, 16-bit signed, stereo interleave, round-trip through `dmw_format::sound::load_wav` | no |
| T4.4 | atomic write: simulate failure after tmp write (inject error) → previous pack intact | no |
| T4.5 | roster suggestion: edit-distance ranking on a fixed name list | no |
| T4.6 | `--list` against a real install prints ≥ 50 weapons including every 00 §9.1 name | **yes** (manual, sheet 13 §5.6) |
| T4.7 | full export of the default roster passes `Pack::open` + all lazy loads; report has zero errors | **yes** (manual) |
| T4.8 | export twice → identical pack bytes except `created_unix` and report timings | **yes** (manual) |

## 10. Acceptance criteria

- [ ] `cd tools/dmw_export && cargo build --release` succeeds against iw4L at `IW4L_REV`.
- [ ] T4.1–T4.5 pass without MW2.
- [ ] Running without `DMW_MW2_GAMES` exits 2 with a clear message.
- [ ] With MW2 present (when available): T4.6–T4.8 pass and the pack plays in the engine (sheet 13 §5.6 checklist).

## 11. Contract changes requested

1. 03 §5.5 `PartRole` must include `Rocket` and `Knife` (iw4L `FpvPartRole` has `Hands, Gun, Attachment, Rocket, Knife`). Sheet 03 already lists the field; the enum must be `Hands, Gun, Attachment, Rocket, Knife, ScopeLens`. — Resolved in 03 by the lead (see 00 Revision 1).

### Build notes (sheet 04 implemented, verified against a real install)

No change to 00 is requested. Deviations from this sheet's text and findings, for the lead:

2. **Games root without `IW4L_GAMES`.** §5.1 says to copy `DMW_MW2_GAMES` into `IW4L_GAMES`; `std::env::set_var` is `unsafe` in edition 2024 and 01 §5.3 keeps `unsafe` out of the exporter. `load::games_root` builds `GamesRoot` from `DMW_MW2_GAMES` (else `IW4L_GAMES`) directly; nothing else on the export path reads the variable. Without either variable: exit 2.
3. **iw4L's image cache.** iw4L writes `./iw4l-artifacts/cache/mips` (about 600 MiB of decoded MW2 images) into the working directory, which under `task export` is `tools/dmw_export/` inside the repository. The exporter moves its working directory to `<out>/../.dmw_export-cache` (`packs/.dmw_export-cache`, gitignored), a temporary directory for `--list`, or leaves it alone when `IW4L_ARTIFACTS_DIR` is set. `--cache-dir` overrides. `--out` and the games root are made absolute first.
4. **Names.** `WeaponRegistry::name_of` returns `m4` (it strips `_mp`); `WeaponFacts::name`, the manifest and `--list` use `script_name_of` (`m4_mp`), which is what `doommw2.toml` names.
5. **Display names and sounds need loads outside `PreparedMatch`** (§5.3/§5.4 assumed it holds them): `prepared.strings` is empty after `load_prepared_match` and `prepared.sound` holds the map zone's 22 aliases only. The exporter reads `asset_game::load_mp_localized_strings` and `asset_audio::load_mp_sound_bank` (the composition the session builds at match start). Details in `docs/provenance.md`.
6. **Surface hiding** uses model-local hide words (base 0), as iw4L's own FPV renderer does; `DObj::surface_visible(slot, ..)` from §5.5 lifts the part bits by the slot's bone base, which does not match model-local hide words (found by reading the code, not by a failing run). Surfaces whose material edge is not bound are skipped (iw4L does not draw them either) and counted as `no_material`.
7. **Overlay.** `overlay_is_hud_iris` is true for the sniper scope overlays (it means "drawn by the HUD iris path"), so it is not a reason to skip. `scope_overlay_m40a3` resolves through `decode_ui_image`; Barrett and Intervention export a 1024×1024 `overlay.png`.
8. **`mp_hit_alert`** is stored under that exact name (the bank holds `MP_hit_alert`; `room/src/dmw/audio.rs` asks for the lower-case name and `Pack::load_sound` compares exactly).
9. **`reload_show_rocket_time_ms`** (sheet 10 §11 item 3) is added to `dmw_format::FpvFacts` with `#[serde(default)]`, exported, and read by `dmw_weapons::ViewmodelAnimator::new` (it passed 0). Fixture value 0, so fixture behaviour is unchanged.
10. **Report** has extra fields: `surface_detail` (material, model, role, alpha, cull, texture, triangles), `clips.unique`, `textures[].source`, `extra_sounds`, `sounds_skipped`, `load_ms`.
11. **Sound slots** `fire_loop`, `fire_loop_player`, `fire_stop`, `fire_stop_player` are not `WeaponSoundSlot`s; they are read from `sounds_of(i)` and checked against the bank. An authored alias missing from the bank is kept in `facts.json` (so the game logs the miss, 03 §5.8) and reported.
12. **Real-install results** (mp_rust + common_mp, default roster): all 11 weapons export, 113 sound aliases, 52 MB pack, report `errors: []`; every weapon's hands texture is 2048×1024 and is halved to 1024×512. Skipped: `weap_putaway` and `weap_cheytac_chamber_close_plr` are null-file aliases; `weap_rpd_mg4_plr` (MG4 notetrack) is not in the bank. Two exports produce identical bytes apart from `created_unix` and report timings.
13. **Engine check** (`room/tests/mw2pack_headless.rs`, ignored, needs `DMW_REAL_PACK`): with doom1.wad (USP, SPAS-12, M4, RPG) and Freedoom2 + IDKFA (all seven mapped weapons, plus a `-mw2config` run mapping Desert Eagle, AA-12, Vector and Intervention) every weapon raises, fires, reloads and aims without MW2 mode dropping. Findings for other sheets: `SimEvent::ReloadFinished` only fires for segmented (shotgun) reloads, because `weapon_iw4` emits `ReloadEnded` only from `ReloadEnd` (sheet 07 `sim.rs`); the Ranger has `aim_down_sight = false`, so it has no ADS.
