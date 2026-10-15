# 03 — Asset Pack Format and Synthetic Fixture (`dmw_format`)

Owner: build agent (wave 2)
Status: implementation-ready
Depends on: 00 (§5 constants, §6.1 file map, §7.1 API), 01, 02 (crate skeleton)

The pack is the only thing that crosses from MW2 into DOOMMW2. This sheet fixes its on-disk format (v1), the Rust types both the exporter (04) and the game (07, 10, 11, 12) use, the loader and validator, and a fully synthetic fixture pack that exercises every feature with zero MW2 data.

---

## 1. Purpose

- One versioned, documented format that a human can inspect (`manifest.json`, `facts.json` are readable JSON) and that loads fast where it matters (`viewmodel.bin` is postcard).
- Types that round-trip iw4L's own structs exactly (`weapon_iw4::WeaponCombatFacts` etc.), so sheet 07 feeds `weapon_iw4` the numbers the exporter read, with no hand-copied field lists that can drift.
- A fixture pack good enough to run the whole game loop, golden-test the rasterizer and unit-test the sim, so 21 of 22 decisions in 00 can be built without MW2 (00 D22).

## 2. Scope / Out of scope

In scope: all files in 00 §6.1; pack directory layout; JSON and binary schemas; serde remote mirrors for iw4L types; unit conversion helpers and constants (00 §5); loader with lazy per-weapon loading and caching; validation; PNG/WAV decode; fixture generator (in-memory `Pack::fixture()` and on-disk `dmw-fixture` binary).

Out of scope: reading MW2 (04); interpreting facts (07); building DObj/AnimClip runtime objects from the mirror types (10 §5.2 does the conversion, because it owns the `xmodel_runtime` dependency usage); quantization (11).

## 3. Dependencies

| From | What |
|---|---|
| 00 §5 | constants (this crate is their home) |
| 00 §7.1 | `Pack`, `WeaponFacts`, `ViewmodelAsset`, `SoundAsset`, `RgbaImage`, `PackError` |
| iw4L `weapon_iw4` @ fc3d8ea | `WeaponCombatFacts`, `AimAssistRanges`, `EmptyReloadPolicy`, `DualMagTimes`, `ScopeZoom`, `WeaponSwayParams`, `WeaponIdleInputs`, `WeaponMovementOfsInputs`, `HITLOC_COUNT` (= 20), `WEAPON_ANIM_SLOTS` (= 39), `weap_anim` slot constants |
| iw4L `asset_game` (read for field lists only, not a dependency) | `WeaponKickFacts` (weapon_catalog.rs:364), `WeaponFpvFacts` (weapon_catalog/fpv.rs), `WeaponHudFacts` (weapon_catalog/presentation.rs), `WeaponEventFacts`, `WeaponSoundSlot` list (weapon_catalog.rs:886), `WeaponAnimSlot` (weapon_animations.rs:39), `EquipmentRuntimeFacts` (weapon_iw4/equipment.rs) |
| iw4L `xmodel_runtime` (read for shapes only) | `DObj`, `Bone`, `ModelSlot`, `AnimClip`, `Track`, `Rotation`, `Translation`, `Keyed`, `FrameIndices`, `ClipNotify` (anim_iw4 `Quat = [f32;4]`, `Vec3 = [f32;3]`) |

## 4. File inventory

| Path | Content |
|---|---|
| `crates/dmw_format/src/lib.rs` | module tree, `PACK_FORMAT_VERSION`, re-exports |
| `src/consts.rs` | 00 §5.1–5.4 verbatim |
| `src/convert.rs` | 00 §5.1 conversions + tests |
| `src/manifest.rs` | §5.2 |
| `src/facts.rs` | §5.3 |
| `src/remote.rs` | §5.4 |
| `src/model.rs` | §5.5 |
| `src/anim.rs` | §5.6 |
| `src/texture.rs`, `src/sound.rs` | §5.7 |
| `src/load.rs` | `Pack` (§5.8) |
| `src/validate.rs` | §5.8 |
| `src/fixture/mod.rs`, `fixture/geometry.rs`, `fixture/clips.rs`, `fixture/facts.rs`, `fixture/media.rs` | §5.9 |
| `src/bin/dmw-fixture.rs` | writes the fixture pack to a directory |
| `tests/roundtrip.rs`, `tests/fixture.rs`, `tests/golden/fixture_manifest.json` | §9 |

## 5. Detailed design

### 5.1 Directory layout (format v1)

```text
<pack>/
  manifest.json
  weapons/<name>/facts.json
  weapons/<name>/viewmodel.bin        (absent when the weapon has no FPV assembly; then has_viewmodel=false)
  weapons/<name>/textures/<n>.png     n = 0..textures.len()-1, RGBA8 PNG, referenced by index
  weapons/<name>/overlay.png          (optional, scope overlay image, RGBA8)
  sounds/<alias>/<variant>.wav        PCM 16-bit, mono or stereo, any rate
  report.json                         exporter report; never read by the game
```

`<name>` is the MW2 weapon asset name (`m4_mp`). `<alias>` is the MW2 sound alias name with any character outside `[A-Za-z0-9_.-]` replaced by `_` (the manifest stores the original alias). Paths in JSON are relative to the pack root with `/` separators.

### 5.2 `manifest.json`

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackManifest {
    pub format_version: u32,                 // == PACK_FORMAT_VERSION (1)
    pub generator: String,                   // "dmw_export 0.1.0" | "dmw-fixture 0.1.0"
    pub iw4l_rev: String,                    // "fc3d8ea" | "fixture"
    pub source: PackSource,
    pub created_unix: u64,
    pub fixture: bool,
    pub weapons: Vec<WeaponEntry>,           // order defines DmwWeaponId = index + 1
    pub sounds: Vec<SoundEntry>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,   // requested name -> weapon name (fixture maps usp_mp etc.)
}
pub struct PackSource { pub game: String /* "iw4" | "fixture" */, pub zone: String, pub common_zone: Option<String> }
pub struct WeaponEntry { pub name: String, pub dir: String, pub has_viewmodel: bool, pub has_overlay: bool, pub texture_count: u32 }
pub struct SoundEntry { pub alias: String, pub variants: Vec<SoundVariant> }
pub struct SoundVariant {
    pub file: String,
    #[serde(default = "one")] pub volume: f32,       // linear 0..1 (mean of alias vol min/max when known)
    #[serde(default = "one")] pub pitch: f32,        // playback rate multiplier (mean of pitch min/max)
    #[serde(default)] pub looping: bool,
    #[serde(default)] pub distance_min: f32,          // MW2 inches, 0 = unknown
    #[serde(default)] pub distance_max: f32,
}
```

`Pack::weapon_id(name)` checks `weapons[].name`, then `aliases`. Unknown fields in any JSON file are ignored (`#[serde(default)]` on optional fields, no `deny_unknown_fields`) so v1 readers accept v1.x additions; a breaking change bumps `format_version`.

### 5.3 `facts.json` — `WeaponFacts`

All field names are snake_case copies of the iw4L field names so the exporter is a straight copy.

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WeaponFacts {
    pub name: String,
    pub display_name: String,
    #[serde(with = "remote::WeaponCombatFactsDef")]
    pub combat: weapon_iw4::WeaponCombatFacts,
    pub kick: KickFacts,
    pub sway: SwayFacts,
    pub fpv: FpvFacts,
    pub hud: HudFacts,
    pub movement: MovementScales,
    pub anim_times: AnimTimes,
    pub projectile: Option<ProjectileFacts>,
    pub sounds: WeaponSounds,
    pub has_overlay: bool,
}

/// Every field of asset_game::WeaponKickFacts (weapon_catalog.rs:364), same names and types.
pub struct KickFacts {
    pub f_ads_view_kick_center_speed: f32, pub f_hip_view_kick_center_speed: f32,
    pub gun_max_pitch: f32, pub gun_max_yaw: f32,
    pub ads_gun_kick_reduced_kick_bullets: i32, pub ads_gun_kick_reduced_kick_percent: f32,
    pub ads_gun_kick_pitch_min: f32, pub ads_gun_kick_pitch_max: f32, pub ads_gun_kick_yaw_min: f32, pub ads_gun_kick_yaw_max: f32,
    pub ads_gun_kick_accel: f32, pub ads_gun_kick_speed_max: f32, pub ads_gun_kick_speed_decay: f32, pub ads_gun_kick_static_decay: f32,
    pub ads_view_kick_pitch_min: f32, pub ads_view_kick_pitch_max: f32, pub ads_view_kick_yaw_min: f32, pub ads_view_kick_yaw_max: f32,
    pub hip_gun_kick_reduced_kick_bullets: i32, pub hip_gun_kick_reduced_kick_percent: f32,
    pub hip_gun_kick_pitch_min: f32, pub hip_gun_kick_pitch_max: f32, pub hip_gun_kick_yaw_min: f32, pub hip_gun_kick_yaw_max: f32,
    pub hip_gun_kick_accel: f32, pub hip_gun_kick_speed_max: f32, pub hip_gun_kick_speed_decay: f32, pub hip_gun_kick_static_decay: f32,
    pub hip_view_kick_pitch_min: f32, pub hip_view_kick_pitch_max: f32, pub hip_view_kick_yaw_min: f32, pub hip_view_kick_yaw_max: f32,
    // … plus any remaining fields of WeaponKickFacts at fc3d8ea (the implementer lists them all;
    //     §9 test T3.2 fails if the exporter-side conversion misses one).
}
pub struct SwayFacts {
    #[serde(with = "remote::WeaponSwayParamsDef")] pub hip: weapon_iw4::WeaponSwayParams,
    #[serde(with = "remote::WeaponSwayParamsDef")] pub ads: weapon_iw4::WeaponSwayParams,
}
/// Inputs to weapon_iw4::weapon_placement_assemble that come from the weapon (WeaponFpvFacts).
pub struct FpvFacts {
    pub ads_aim_pitch: f32, pub ads_bob_factor: f32, pub ads_view_bob_mult: f32,
    pub ads_zoom_fov: f32, pub ads_zoom_in_frac: f32, pub ads_zoom_out_frac: f32,
    pub ads_overlay_width: f32, pub ads_overlay_height: f32,
    pub aim_down_sight: bool, pub can_hold_breath: bool,
    pub ducked_ofs: [f32; 3], pub prone_ofs: [f32; 3],
    #[serde(with = "remote::WeaponIdleInputsDef")] pub idle: weapon_iw4::WeaponIdleInputs,
    #[serde(with = "remote::WeaponMovementOfsInputsDef")] pub movement: weapon_iw4::WeaponMovementOfsInputs,
    pub overlay_reticle: i32,
    pub reload_time_ms: i32,
    #[serde(with = "remote::scope_zoom")] pub scope_zoom: weapon_iw4::ScopeZoom,
    pub ads_overlay_convention: String,      // Debug name of asset_game::AdsOverlayConvention, e.g. "PlayAdsAnim"
}
/// Subset of WeaponHudFacts used by sheet 12.
pub struct HudFacts {
    pub ads_crosshair_in_frac: f32, pub ads_crosshair_out_frac: f32,
    pub hip_reticle_side_pos: f32, pub i_reticle_min_ofs: i32, pub i_reticle_side_size: i32,
    pub clip_only: bool, pub clip_size: i32,
    pub ammo_counter_clip: i32,
}
pub struct MovementScales { pub move_speed_scale: f32, pub ads_move_speed_scale: f32, pub sprint_duration_scale: f32 }
/// From asset_game::WeaponAnimations (times the viewmodel controller needs).
pub struct AnimTimes {
    pub fire_time_ms: i32, pub melee_time_ms: i32, pub raise_time_ms: i32, pub drop_time_ms: i32,
    pub quick_raise_time_ms: i32, pub quick_drop_time_ms: i32,
    pub sprint_raise_time_ms: i32, pub sprint_loop_time_ms: i32, pub sprint_drop_time_ms: i32,
    // … every remaining *_time_ms field of WeaponAnimations at fc3d8ea.
}
/// From weapon_iw4::EquipmentRuntimeFacts + WeaponEventFacts, present only when projectile_speed > 0.
pub struct ProjectileFacts {
    pub projectile_speed: i32, pub projectile_speed_up: i32, pub projectile_speed_forward: i32,
    pub explosion_radius: i32, pub explosion_radius_min: i32,
    pub explosion_inner_damage: i32, pub explosion_outer_damage: i32,
    pub impact_damage: i32, pub proj_impact_explode: bool, pub missile_guidance: i32,
    pub ignition_delay_ms: i32,
}
/// One Option<String> per asset_game::WeaponSoundSlot (weapon_catalog.rs:886), same snake_case
/// names: fire, fire_player, empty_fire, empty_fire_player, melee_swipe, melee_swipe_player,
/// melee_hit, melee_miss, pickup, pickup_player, ammo_pickup, ammo_pickup_player, … (all of them),
/// plus `notetracks: BTreeMap<String, String>` (notetrack name -> sound alias, from
/// WeaponRegistry::notetrack_sound_aliases_of / notetrack_actions_of).
pub struct WeaponSounds { /* fields as described */ }
```

### 5.4 `remote.rs` — serde mirrors of iw4L structs

Use serde's remote derive so that a field added or renamed in iw4L becomes a **compile error** here (the mirror must list every field of the remote type):

```rust
#[derive(Serialize, Deserialize)]
#[serde(remote = "weapon_iw4::WeaponCombatFacts")]
pub struct WeaponCombatFactsDef {
    #[serde(with = "AimAssistRangesDef")] pub aim_assist: weapon_iw4::AimAssistRanges,
    pub dual_wield: bool,
    pub fire_melees: bool,
    pub fire_time_ms: i32,
    // … every field of WeaponCombatFacts in tick.rs:179, in declaration order …
    #[serde(with = "EmptyReloadPolicyDef")] pub empty_reload: weapon_iw4::EmptyReloadPolicy,
    #[serde(with = "scope_zoom")] pub scope_zoom: weapon_iw4::ScopeZoom,
    pub location_damage: [f32; weapon_iw4::HITLOC_COUNT],
    #[serde(with = "opt_dual_mag")] pub dual_mag: Option<weapon_iw4::DualMagTimes>,
}
```

Rules:

- `EmptyReloadPolicy` → remote enum `{ Ordinary, Authored }`.
- `ScopeZoom` has private fields: `mod scope_zoom` serializes `Vec<f32>` = `(0..z.levels()).filter_map(|i| z.fov(i as u32))` and deserializes with `ScopeZoom::from_fovs` (padding with 0.0, which `from_fovs` rejects). Empty vec ↔ `ScopeZoom::NONE`.
- `Option<DualMagTimes>`: helper module `opt_dual_mag` using a `DualMagTimesDef` remote.
- `[f32; 20]` arrays serialize natively.
- `WeaponSwayParams`, `WeaponIdleInputs`, `WeaponMovementOfsInputs`, `AimAssistRanges`: plain remote structs.
- `select_requires_ammo`/`offhand_hold_is_cancelable` are `Option<bool>` and serialize as JSON `null`/bool.

### 5.5 `viewmodel.bin` — model and skeleton

Binary file = 8-byte header `b"DMWV"` + `u32` LE version (`1`) + postcard-encoded `ViewmodelFile`:

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ViewmodelFile { pub skeleton: Skeleton, pub mesh: ViewmodelMesh, pub clips: ClipSet }

/// Mirror of xmodel_runtime::DObj for the composed FPV assembly (hands + gun + attachments),
/// plus the assembly's special bones.
pub struct Skeleton {
    pub bones: Vec<BoneData>,
    pub models: Vec<ModelSlotData>,
    pub duplicates: Vec<(u16, u16)>,
    pub view_bone: u16,               // FpvAssembly::view_bone() — "tag_view"
    pub camera_bone: Option<u16>,     // FpvAssembly::camera_bone()
    pub flash_bone: Option<u16>,      // "tag_flash" (FpvAssembly::tags())
    pub brass_bone: Option<u16>,      // "tag_brass"
    pub collapsed_bones: Vec<u16>,    // FpvAssembly::collapsed_bones()
}
pub struct BoneData {
    pub name: String,
    pub model: u16,
    pub parent: Option<u16>,          // always < own index
    pub bind_rotation: [f32; 4],      // x, y, z, w
    pub bind_translation: [f32; 3],
    pub bind_world: [f32; 16],        // column-major Mat4
    pub no_scale: bool,
}
pub struct ModelSlotData { pub name: String, pub base: u16, pub bone_count: u16, pub scale: f32 }

pub struct ViewmodelMesh {
    pub positions: Vec<[f32; 3]>,     // bind (model/world) space, inches; skin = world_pose * inverse(bind_world)
    pub normals: Vec<[f32; 3]>,       // unit length
    pub uvs: Vec<[f32; 2]>,
    pub skin: Vec<VertexSkin>,        // same length as positions
    pub indices: Vec<u32>,            // triangle list, counter-clockwise front faces in MW2 axes (see §5.5.1)
    pub surfaces: Vec<SurfaceData>,
}
pub struct VertexSkin { pub bones: [u16; 4], pub weights: [f32; 4] } // global DObj bone indices; weights sum to 1 (±1e-3); unused = (0, 0.0)
pub struct SurfaceData {
    pub name: String,                 // material name, for logs
    pub index_start: u32, pub index_count: u32,
    pub texture: Option<u16>,         // index into textures/<n>.png; None = untextured (flat grey index)
    pub alpha: AlphaMode,
    pub cull: CullMode,
    pub role: PartRole,
    pub model: u16,                   // ModelSlotData index the surface came from
}
pub enum AlphaMode { Opaque, Test { cutoff: f32 }, Blend }   // Blend is drawn as Test{0.5} by sheet 11
pub enum CullMode { Back, None }
/// iw4L FpvPartRole + ScopeLens. Rocket surfaces are hidden when the rocket is not loaded and
/// Knife surfaces are shown only during melee (sheet 11 §5.10). ScopeLens surfaces are skipped by 11.
pub enum PartRole { Hands, Gun, Attachment, Rocket, Knife, ScopeLens }
```

#### 5.5.1 Winding and axes

Positions are in MW2 model space (x forward, y left, z up, inches) exactly as iw4L's `ModelSkel::positions`. Triangle winding is whatever MW2 authored; the exporter records it and the fixture uses counter-clockwise when viewed from outside. Sheet 11 culls with the winding of the first surface flagged `CullMode::Back` determined by a one-time check in `PreparedViewmodel::new` (majority of triangles facing the bind-pose camera at origin looking +x) and logs which convention it detected.

### 5.6 Animation clips

```rust
pub struct ClipSet {
    pub clips: Vec<ClipData>,                   // unique clips
    pub slots: Vec<Option<u16>>,                // len == weapon_iw4::WEAPON_ANIM_SLOTS (39); index = WeaponAnimSlot
    pub track_maps: Vec<Vec<Option<u16>>>,      // per clip: track index -> DObj bone index (FpvAssembly::compose_tracks)
}
/// Exact mirror of xmodel_runtime::AnimClip.
pub struct ClipData {
    pub name: String,
    pub framerate: f32,
    pub numframes: u16,
    pub looping: bool,
    pub tracks: Vec<TrackData>,
    pub notifies: Vec<NotifyData>,             // ClipNotify { name, time }
    pub delta_translation: TranslationData,
}
pub struct TrackData { pub name: String, pub rotation: RotationData, pub translation: TranslationData }
pub enum RotationData { Default, HalfKeyed(KeyedQuat), FullKeyed(KeyedQuat), HalfConstant([f32; 4]), FullConstant([f32; 4]) }
pub enum TranslationData { Default, SmallKeyed(KeyedVec3), FullKeyed(KeyedVec3), Constant([f32; 3]) }
pub struct KeyedQuat { pub values: Vec<[f32; 4]>, pub frames: FrameIndicesData }
pub struct KeyedVec3 { pub values: Vec<[f32; 3]>, pub frames: FrameIndicesData }
pub enum FrameIndicesData { Dense, Sparse(Vec<u16>) }
pub struct NotifyData { pub name: String, pub time: f32 }   // time in the clip's own units as stored by AnimClip
```

Values are stored exactly as `xmodel_runtime` holds them after decode (already dequantized f32), so sheet 10 can rebuild an `AnimClip` field-for-field and get identical sampling.

### 5.7 Textures and sounds

- `texture.rs`: `pub fn load_png(path) -> Result<RgbaImage, PackError>` (8-bit RGBA or RGB, converted to RGBA; 16-bit and palette PNGs converted). Max 4096×4096; larger → `PackError::Invalid`.
- `sound.rs`: `pub fn load_wav(path) -> Result<SoundAsset, PackError>` using `hound`; accepts 8/16-bit int and 32-bit float, mono/stereo; converts to interleaved `f32` in −1..1.

### 5.8 Loader and validation

`Pack::open(dir)`:

1. Read and parse `manifest.json`; check `format_version == 1` (else `PackError::Version`).
2. For each weapon: parse `facts.json` eagerly (small). Check `facts.name == entry.name`.
3. Run `validate::manifest(&manifest, &facts)`.
4. Return `Pack` with empty caches. `load_viewmodel`, `load_sound`, `load_overlay` read on first call and cache `Arc`s in `RefCell<BTreeMap<..>>` (the engine is single-threaded; `Pack` is `!Sync`, which is fine).

`Pack::fixture()` builds the same structure in memory (§5.9) and never touches the filesystem; its loaders return the generated assets.

Validation (`validate.rs`) — each failure is `PackError::Invalid(String)` naming weapon, file and field:

| Check | Rule |
|---|---|
| combat sanity | `fire_time_ms > 0` for bullet/projectile weapons; `clip_size >= 1`; `damage >= 0`; `min_damage_range >= max_damage_range` when both > 0; `fire_type` in 0..=5; `location_damage` all finite and ≥ 0 |
| skeleton | `bones.len() <= 192` (`anim_iw4::PartBits::CAPACITY`); every `parent < index`; `view_bone < bones.len()`; quaternions finite and normalised to 1 ± 1e-3 |
| mesh | equal lengths of positions/normals/uvs/skin; every index `< positions.len()`; `indices.len() % 3 == 0`; surface ranges inside `indices` and non-overlapping; skin bone indices `< bones.len()`; weights finite, sum 1 ± 1e-3 |
| clips | `slots.len() == 39`; slot clip indices valid; `track_maps[i].len() == clips[i].tracks.len()`; mapped bones valid; `framerate > 0` unless `numframes <= 1`; keyed `values.len()` consistent with `frames` (Dense: `numframes + 1` or `numframes` per xmodel_runtime convention — copy the rule from `xanim_clip.rs` sampling) |
| sounds | every alias referenced by any `WeaponSounds` field exists in `manifest.sounds` **or** is logged as a warning (missing sounds are not fatal) |

### 5.9 Synthetic fixture pack

Purpose: exercise every code path with recognisable, deterministic, non-MW2 content. Generated by code; byte-identical on every run (no randomness, no timestamps — `created_unix = 0`).

Weapons (names, then `aliases` mapping the 00 §9.1 names to them):

| Fixture weapon | Behaviour it exercises | Key combat facts | Aliased from |
|---|---|---|---|
| `fixture_pistol` | semi-auto, fast raise, pistol class, empty-reload clip | fire_type 1, fire_time 120, clip 12, reload 1500 / empty 1800, damage 40→25 over 500..1200 in, weap_class 5 | `usp_mp`, `deserteagle_mp` |
| `fixture_rifle` | full auto, ADS with zoom 50°, kick, sway, sprint | fire_type 0, fire_time 86, clip 30, reload 2200 / 2600, damage 30→20 over 1000..2000, ads_zoom_fov 50, ads_in_rate 1/0.25 s | `m4_mp`, `mg4_mp`, `kriss_mp` |
| `fixture_shotgun` | spread class, 8 pellets, segmented reload (start/insert/end) | fire_type 1, weap_class 4 (`WEAPCLASS_SPREAD`), shots_per_fire 8, clip 8, segmented_reload, reload_start 500, reload_add 600, reload_end 700, damage 40→15, min_damage_range 600 | `spas12_mp`, `ranger_mp`, `aa12_mp` |
| `fixture_sniper` | bolt action, scope overlay, hold breath, 2 zoom levels | fire_type 1, bolt_action, rechamber 1100, clip 5, overlay_reticle 1, has_overlay, can_hold_breath, scope_zoom [20, 10] | `barrett_mp`, `cheytac_mp` |
| `fixture_launcher` | projectile weapon | weap_type 2 (`WEAPTYPE_PROJECTILE`), clip 1, reload 2500, projectile speed 1800, explosion radius 256 / inner 200 / outer 50 | `rpg_mp` |

Every value not listed is a plausible MW2-like value chosen once, written in `fixture/facts.rs` as named constants with a comment saying it is synthetic. All five weapons must pass `WeaponCombatFacts::try_from_captured`-equivalent sanity (§5.8) and the sim smoke tests in sheet 07.

Geometry (identical for all weapons, recoloured per weapon):

- Bones: `tag_origin` (root), `tag_view` (child of root, at origin), `j_gun` (child of `tag_view`, at (14, −5, −6)), `tag_flash` (child of `j_gun`, at barrel tip (+18, 0, +1)), `j_mag` (child of `j_gun`), `j_wrist_ri` (child of `tag_view`), `j_wrist_le` (child of `tag_view`), `tag_brass` (child of `j_gun`). `view_bone = tag_view`, `flash_bone = tag_flash`.
- Mesh: gun body box 16×3×4 in, barrel box 10×1.2×1.2, magazine box 2×2×5 (skinned to `j_mag`), two hand boxes 4×3×3 (rigid to wrists), one 2-bone blended vertex strip between `j_gun` and `j_mag` (exercises weights). About 200 triangles.
- Textures: 64×64 RGBA, per part: gun = dark grey with light stripes and a 1-texel red mark at the muzzle end (orientation check); hands = flat skin tone with darker knuckle band; magazine = olive. One texture uses an alpha-tested hole (exercises `AlphaMode::Test { cutoff: 0.5 }`).
- Overlay (`fixture_sniper`): 256×192 RGBA ring with crosshair lines, transparent centre.

Clips (each a real `ClipData` with keyed tracks so sampling/interpolation is exercised): idle (loop, 2 s, 1° breathing pitch on `j_gun`), fire (0.15 s, `j_gun` −3° pitch kick and return), last_shot, rechamber (sniper only, notetracks `bolt_back`, `bolt_forward`), reload (2.0 s: `j_mag` translates −8 z and back; notetracks `mag_out` at 0.3, `mag_in` at 0.7), reload_empty, reload_start/reload_end (shotgun), raise / drop / first_raise (`j_gun` rotates 60° down↔up), sprint_in / sprint_loop / sprint_out (`j_gun` yaw 30°), ads_up / ads_down (`tag_view`-relative gun moves to centre: (10, 0, −3)), melee (`j_gun` lunges +6 x). Slots without a clip stay `None` (exercises the missing-clip path in sheet 10).

Sounds (generated PCM, 22050 Hz mono, 16-bit): `fixture_fire_*` (white-noise burst with exponential decay, 120 ms, a distinct pitch per weapon), `fixture_empty` (short click), `fixture_mag_out`, `fixture_mag_in`, `fixture_bolt_back`, `fixture_bolt_forward` (clicks of different lengths), `fixture_explode` (300 ms low-passed noise). `WeaponSounds` and `notetracks` reference these.

`dmw-fixture <dir>` writes the pack with the exact v1 layout (PNG via `png`, WAV via `hound`). `Pack::open(<dir>)` of that output must equal `Pack::fixture()` field-for-field (test T3.5).

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| `manifest.json` missing | `PackError::Missing("manifest.json")` |
| Version ≠ 1 | `PackError::Version { found, expected: 1 }` |
| One weapon's `facts.json` invalid | the whole pack fails to open (partial packs hide exporter bugs); the error names the weapon |
| `viewmodel.bin` missing while `has_viewmodel = true` | `load_viewmodel` returns `PackError::Missing`; the engine treats that weapon as unusable (sheet 08 §6) |
| Header magic/version mismatch in `viewmodel.bin` | `PackError::Format` |
| Postcard decode error | `PackError::Format` with the file path |
| PNG > 4096² or not decodable | `PackError::Invalid` |
| WAV with > 2 channels | `PackError::Invalid` |

## 7. Logging

`dmw::pack`: `info!("pack {dir}: {n} weapons, {m} sound aliases, generator {g}, iw4l {rev}{fixture}")` on open; `debug!` per lazy load with byte size and time; `warn!` per missing referenced sound alias (once per alias).

## 8. Licensing and asset notes

The fixture is original work generated by code (GPL-3.0-or-later). It must not imitate MW2 meshes, textures or sounds beyond "a box-shaped gun". Real packs are never committed (`packs/` is gitignored).

## 9. Tests

| Id | Test |
|---|---|
| T3.1 | `convert.rs`: BAM↔degrees round trip within 1 BAM for 10 000 angles; pitch→slope sign (looking down gives negative slope); inches↔fixed |
| T3.2 | `WeaponCombatFacts` round-trip: build a value with every field set to a distinct non-default value (helper `combat_facts_all_distinct()`), JSON round-trip, assert `==` |
| T3.3 | `ScopeZoom` round-trip for 0, 1, 2, 3 levels |
| T3.4 | `ViewmodelFile` postcard round-trip equality on the fixture |
| T3.5 | `dmw-fixture` → temp dir → `Pack::open` equals `Pack::fixture()` (facts, viewmodels, sounds, overlay) |
| T3.6 | fixture determinism: two `dmw-fixture` runs produce byte-identical trees (hash all files) |
| T3.7 | each validation rule in §5.8 rejects a hand-corrupted copy (one test per row) |
| T3.8 | `weapon_id` resolves names and aliases; unknown → `None` |
| T3.9 | golden: `tests/golden/fixture_manifest.json` equals the fixture manifest (BLESS to update) |

## 10. Acceptance criteria

- [ ] All files in §4 exist; `#![forbid(unsafe_code)]`; clippy clean.
- [ ] 00 §7.1 API implemented exactly (names, signatures).
- [ ] Remote mirrors cover every field of the listed iw4L structs at fc3d8ea (compile-enforced) and T3.2/T3.3 pass.
- [ ] Fixture pack contains the five weapons, aliases, clips, textures, overlay and sounds of §5.9; T3.4–T3.6, T3.9 pass.
- [ ] Validation rules of §5.8 each have a failing-input test (T3.7).
- [ ] `task fixture-pack` writes `packs/fixture/`.

## 11. Contract changes requested

None.
