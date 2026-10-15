# 00 — Overview and Shared Contracts

Owner: lead (not assigned to a build agent)
Status: authoritative. Every other sheet conforms to the names, paths, types, constants and values in this file. A sheet that needs a contract changed lists it in its own "Contract changes requested" section; the lead adds a Revision section at the end of this file. Later revisions win over earlier sections.

Source trees this sheet was written against (read 2026-10-15):

| Tree | Path | Revision |
|---|---|---|
| room (Rust doomgeneric port, GPL-2.0-or-later code) | `/home/andy/Dev/MW2DOOM/room` | `904acac` |
| iw4L (Rust MW2 runtime, Apache-2.0) | `/home/andy/Dev/MW2DOOM/iw4L` | `fc3d8ea` |

---

## 1. Product summary

DOOMMW2 is a single-player Doom source port in which the player's guns are Modern Warfare 2 guns, end to end:

- **Rules from MW2.** Fire rate, burst/semi/auto, bolt rechamber, reload and empty reload timing, segmented shotgun reloads, raise/drop/first-raise timing, ADS transition, hip spread growth and decay, ADS spread, damage with range falloff, hit-location multipliers, view kick, gun recoil, weapon sway, idle sway, movement bob, sprint lowering and sprint-out delay. All of it comes from `iw4L`'s `weapon_iw4` and `movement_iw4` crates (Rust reimplementations of the retail IW4 BG/PM code), fed with numbers exported from the user's own MW2 installation.
- **Real 3D MW2 viewmodels.** The gun and hands are MW2's skinned meshes, animated by MW2's own clips through `iw4L`'s `xmodel_runtime` evaluator.
- **Drawn the Doom way.** A software rasterizer inside Doom's renderer draws the viewmodel straight into Doom's 320×200 8-bit framebuffer. Textures are quantized to the WAD's PLAYPAL at load. Lighting goes through the WAD's COLORMAP using the player's sector light, so the gun darkens, flashes red on damage, turns white under invulnerability and goes fuzzy under partial invisibility exactly like Doom's own sprites. Menus, HUD text and the status bar layer over it because it is drawn where Doom draws its weapon sprites.
- **Doom stays Doom.** Maps, monsters, movement, pickups, sectors and the 35 Hz tic loop are unchanged. Freelook (y-shearing) and mouse aim are added because MW2 recoil and spread need a vertical axis. Vanilla behaviour, including demo playback, is preserved bit-for-bit whenever MW2 mode is off.

Not in scope for this build: multiplayer/netgames, MW2 maps or characters, MW2 movement physics (jump, crouch, prone, mantle, slide), attachments, killstreaks, perks, grenades/equipment (offhand), akimbo, riot shield, penetration through walls, MW2 particle effects. Each is listed in §2 D-rows or in the owning sheet's Out of scope.

## 2. Locked product decisions

| # | Decision | Choice |
|---|---|---|
| D1 | Direction | MW2 weapons are brought **into** the Doom engine (`room` fork). Not Doom-into-iw4L. |
| D2 | Repo | New repository at `/home/andy/Dev/MW2DOOM/doommw2`, created by copying `room` at `904acac` (minus `target/` and `.git/`) and adding new crates. `room` and `iw4L` are never modified by this project. |
| D3 | iw4L consumption | Path dependencies on `../iw4L/crates/<crate>` pinned to `fc3d8ea` (checked by `tools/check-iw4l-rev.sh`, sheet 02). Runtime links only Bevy-free crates: `weapon_iw4`, `movement_iw4`, `playerstate_iw4`, `math_iw4`, `asset_iw4`, `trace_iw4`, `xmodel_runtime`, `anim_iw4` (and their transitive `d3d9_decl`, `d3d9_state`, `libm`, `glam`). Bevy-dependent iw4L crates are linked only by the exporter. |
| D4 | iw4L code that is Bevy-bound but needed at runtime | Ported (copied and adapted, attribution kept) into `dmw_weapons`: `ViewKickState` (render_anim/anim/view_kick_state.rs), `ViewSwayState` (render_anim/anim/view_sway.rs), `ClipScheduler` (asset_anim/clip_scheduler.rs), a reduced `ViewmodelController` (render_anim/anim/viewmodel_controller.rs), the placement call sequence from `apply_fpv_placement` (render_anim/occupancy/fpv_present.rs), and `update_hold_breath` (movement_iw4/src/breath.rs, `pub(crate)` upstream). Movement-scale constants are taken from iw4L `sim/src/step.rs` (`player_sprint_speed_scale 1.5`, `player_back_speed_scale 0.7`, `player_strafe_speed_scale 0.8`, sprint context values). Sheet 01 §5.10 gives the attribution rule. |
| D5 | Asset path | Offline exporter `tools/dmw_export` (own Cargo workspace, links iw4L incl. Bevy) loads the user's MW2 install headlessly with `assets::load_prepared_match` — the same path `iw4l export-gltf` uses — and writes a **pack** (sheet 03) to `packs/mw2/`. The game binary reads only the pack. No MW2 bytes are ever committed. |
| D6 | Viewmodel rendering | CPU software rasterizer (`dmw_viewmodel`) writing palette indices into Doom's view buffer from `R_DrawPlayerSprites`. No GPU viewmodel pass. `gpu.rs` keeps doing a single blit (plus the aspect change in D16). |
| D7 | Colour pipeline | Pack textures are RGBA8. At level load they are quantized to the active IWAD's PLAYPAL (nearest colour, optional ordered dither, mip chain built in RGB before quantization). Shading = COLORMAP level from sector light + extralight + a per-vertex directional term, exactly as specified in sheet 11. |
| D8 | Resolution | The viewmodel is rasterized at Doom's internal 320×200 inside the current view window. No high-resolution mode in this build. |
| D9 | Simulation rate | Doom's 35 Hz tic. MW2 logic receives integer `msec` per tic from the sequence in §5.3 (28 or 29 ms, exactly 1000 ms per 35 tics). Rendering of the viewmodel is tic-locked (no interpolation) in this build. |
| D10 | Vertical aim | Freelook by y-shearing (Heretic-style `centery` offset). Pitch is clamped to ±`PITCH_LIMIT_DEG` (§5.1). MW2-mode weapons never auto-aim. Vanilla weapons keep Doom autoaim. |
| D11 | Which shots use which logic | Shot direction = Doom yaw + freelook pitch + MW2 `gun_angle_offset` (from gun placement) + MW2 spread. Camera view kick is visual only (render offset), mirroring iw4L (`render_anim/occupancy/view_kick.rs`, `sim/combat.rs` shot angles). |
| D12 | Weapon roster | Data-driven mapping from Doom weapon slots to MW2 weapon names (`doommw2.toml` `[weapons]`, defaults §9.1). A slot mapped to `"vanilla"` keeps the original Doom weapon and psprites. |
| D13 | Ammo model | Doom's four ammo pools stay the reserve ("stock"); each MW2 weapon has its own magazine ("clip") stored in the player extension. Reloads move rounds pool → magazine. Pickups add to pools (scaled, §9.2). |
| D14 | Damage | MW2 damage with range falloff and hit-location multipliers, then multiplied by `[balance] monster_damage_scale` (default 0.35) before `P_DamageMobj`. Monster → player damage is untouched Doom. |
| D15 | Mode gating | MW2 mode is active only when: a valid pack is loaded, `[general] enabled = true`, not a netgame, not recording or playing a demo. It is evaluated at level start and fixed for the level. When inactive, the engine must behave bit-for-bit like `room` (the existing `demo_playthrough` test proves it). |
| D16 | Presentation | `gpu.rs` presents the 640×400 buffer at 4:3 (letterboxed) when `[render] aspect_correct = true` (default). The viewmodel projection assumes 4:3 display of the 320×200 frame (pixel aspect 5:6). |
| D17 | Audio | MW2 weapon sounds from the pack (PCM WAV) are played through `room`'s rodio mixer with Doom's distance/stereo model for the listener; the local player's weapon sounds are non-positional. 8 extra channels are reserved for MW2 sounds. |
| D18 | Config | Doom keeps `default.cfg` (`m_config`) for Doom variables and the new key/mouse bindings. Everything else lives in `doommw2.toml` (current directory, then `$XDG_CONFIG_HOME/doommw2/`), parsed with `toml` + `serde`. Unknown keys are a warning, not an error. |
| D19 | Saves | Vanilla savegame bytes are unchanged; a DOOMMW2 extension block is appended after `SAVEGAME_EOF` (sheet 08 §5.11). Loading a vanilla save in MW2 mode gives every owned mapped weapon a full magazine. |
| D20 | Licence | The combined work is distributed as **GPL-3.0-or-later** (Doom/doomgeneric code is GPL-2.0-or-later; iw4L code is Apache-2.0, which is GPLv3-compatible only). New DOOMMW2 code is GPL-3.0-or-later. Files ported from iw4L keep an Apache-2.0 notice header and are listed in `NOTICE`. |
| D21 | Platforms | Linux first (developer machine). Windows/macOS must not be broken knowingly but are not tested in this build. |
| D22 | Dev without MW2 | The developer machine currently has **no MW2 install**. Every sheet except 04 and 13 §5.6 must be buildable and testable with the synthetic fixture pack (sheet 03 §5.9). |

## 3. Spec sheet map and build order

| Sheet | Title | Depends on (contracts) | Wave |
|---|---|---|---|
| 01 | Engineering standards | 00 | reference |
| 02 | Workspace scaffold and tooling | 00, 01 | 1 |
| 03 | Asset pack format and synthetic fixture | 02 | 2 |
| 04 | MW2 asset exporter | 02, 03 | 3 |
| 05 | Input, mouse look and controls | 02 | 2 |
| 06 | Freelook renderer, FOV zoom and presentation | 02, 05 (§7.4 ext types only) | 2 |
| 07 | Weapon simulation bridge (`dmw_weapons` sim half) | 03 | 3 |
| 08 | Doom integration (hooks, inventory, ammo, movement, saves) | 05, 06, 07 | 4 |
| 09 | Ballistics and damage | 07, 08 hook table | 4 |
| 10 | Viewmodel animation and placement (`dmw_weapons` anim half) | 03, 07 | 4 |
| 11 | Software viewmodel rasterizer and retro pipeline | 03 (crate half); 06, 08, 10 (engine half) | 3 / 5 |
| 12 | HUD, crosshair, scope overlays and audio | 07, 08, 10, 11 | 5 |
| 13 | Testing, fixtures and QA | all; §5.1–5.4 in wave 2 | 2 / 6 |

Agents may start a sheet as soon as the **contracts** it depends on are fixed here; implementations are stubbed.

## 4. Fixed paths, files, environment and flags

### 4.1 Repository layout

```text
doommw2/
  Cargo.toml                 workspace (members below; excludes tools/dmw_export)
  Cargo.lock
  rust-toolchain.toml        channel = "stable" (≥ 1.85, edition 2024 deps)
  Taskfile.yml               task runner (sheet 02 §5.6)
  .env.example               DMW_MW2_GAMES, DMW_IWAD (sheet 02 §5.5)
  doommw2.example.toml       documented config with every key at its default (sheet 02 §5.7)
  README.md  AGENTS.md  CLAUDE.md  LICENSE (GPL-3.0)  LICENSE-GPL-2.0  LICENSE-APACHE-2.0  NOTICE
  IW4L_REV                   the pinned iw4L commit hash, one line
  docs/specs/                these sheets
  docs/provenance.md         what was copied from where (sheet 02 §5.10)
  room/                      engine crate (copied from room; lib `room`, bin `doommw2`)
    src/doom/                Doom modules (copied; edited only at hook points, §7.5)
    src/dmw/                 NEW: all engine-side DOOMMW2 glue (§6)
    src/gpu.rs, src/main.rs, src/platform/   (edited by 05/06)
    tests/                   demo_playthrough.rs (kept) + new headless tests (13)
  doomgeneric-sys/  vendor/  c2rust-intermediate/  soundfonts/  tools/   (copied unchanged)
  crates/
    dmw_format/              pack types, (de)serialization, validation, fixture generator (03)
    dmw_weapons/             MW2 weapon sim bridge + viewmodel animation (07, 10)
    dmw_viewmodel/           software rasterizer, palette quantizer, overlays (11)
  tools/
    dmw_export/              separate workspace: MW2 → pack exporter (04)
    check-iw4l-rev.sh        verifies ../iw4L HEAD == IW4L_REV (02)
    fetch-freedoom.sh        downloads Freedoom IWADs for dev/test (02)
  packs/                     gitignored; packs/mw2/ (exported), packs/fixture/ (generated)
```

### 4.2 Environment variables

| Variable | Used by | Meaning | Default |
|---|---|---|---|
| `DMW_MW2_GAMES` | exporter (04), mapped onto iw4L's `IW4L_GAMES` | Games root containing an `iw4` folder/symlink to the MW2 install (iw4L convention, see `iw4L/docs/RUN.md`) | none — exporter refuses to run |
| `DMW_IWAD` | Taskfile run tasks, tests | Path of the IWAD for `task run`/tests | `wads/freedoom1.wad` |
| `DMW_PACK` | engine | Pack directory | `packs/mw2` (then `packs/fixture` if `[general] allow_fixture_pack = true`) |
| `DMW_LOG` | engine | Overrides `RUST_LOG` filter for `dmw::*` targets | unset |
| `BLESS` | tests (13) | `1` rewrites golden files/baselines | unset |

### 4.3 Engine command-line flags (forwarded through Doom's `myargv`, parsed with `M_CheckParm`)

| Flag | Meaning |
|---|---|
| `-iwad <path>` | unchanged Doom flag |
| `-mw2pack <dir>` | overrides `DMW_PACK` |
| `-nomw2` | forces MW2 mode off (vanilla `room` behaviour) |
| `-mw2config <file>` | overrides the `doommw2.toml` search |
| `-fixturepack` | load the synthetic fixture pack from memory (sheet 03 §5.9) — for development without MW2 |

### 4.4 Files at runtime

| File | Owner sheet | Notes |
|---|---|---|
| `doommw2.toml` | 02 (schema), each sheet adds its section | §9 lists every section |
| `default.cfg` | 05 | new bindings §8.3 |
| `packs/mw2/manifest.json` + per-weapon folders | 03 | format v1 |
| `doomsav*.dsg` | 08 | vanilla bytes + DMW extension |

## 5. Constants and unit conversions (exact values)

All constants live in `crates/dmw_format/src/consts.rs` and are re-exported by `dmw_weapons` and the engine's `room::dmw::consts`. Nobody redefines them.

### 5.1 Geometry and angles

```rust
/// Doom map units per MW2 unit (inch). Doom player height 56 / MW2 standing height 70.
pub const DOOM_PER_MW2: f32 = 0.8;
pub const MW2_PER_DOOM: f32 = 1.25;

/// Doom FRACUNIT.
pub const FRACUNIT: i32 = 1 << 16;

/// Doom BAM: angle_t 0..2^32 counter-clockwise from +x. MW2 yaw: degrees counter-clockwise from +x.
pub const BAM_PER_DEGREE: f64 = 4294967296.0 / 360.0;

/// MW2 pitch convention: positive = looking down. Stored in this convention everywhere.
/// Freelook clamp. Shear at full 320 view = tan(32°) * 160 ≈ 100 px = viewheight/2 at 200.
pub const PITCH_LIMIT_DEG: f32 = 32.0;

/// Pitch is carried in ExtTiccmd as i16 in 1/128 degree.
pub const PITCH_UNITS_PER_DEGREE: f32 = 128.0;

/// MW2 standing eye height (inches) used for the shadow PlayerState.
pub const MW2_STAND_VIEW_HEIGHT: f32 = 60.0;

/// Doom terminal running speed: thrust 50*2048/65536 = 1.5625 u/tic, friction 0xE800/0x10000
/// = 0.90625 ⇒ 1.5625 / 0.09375 = 16.667 u/tic = 16.667 * 1.25 * 35 = 729.17 in/s.
pub const DOOM_RUN_SPEED_MW2: f32 = 729.17;
/// MW2 g_speed (in/s).
pub const MW2_RUN_SPEED: f32 = 190.0;
/// Doom moves ~3.8x faster than an MW2 soldier. Every velocity handed to MW2 code (bob cycle,
/// spread motion, sway kinematics) is multiplied by this so Doom running "feels like" MW2 running
/// to the MW2 formulas. Positions are NOT scaled (they only matter as deltas, which are unused).
pub const SHADOW_VELOCITY_SCALE: f32 = MW2_RUN_SPEED / DOOM_RUN_SPEED_MW2; // ≈ 0.2606
```

Conversions (implemented once in `dmw_format::convert`, tested in 03):

| From | To | Formula |
|---|---|---|
| Doom fixed position `x` | MW2 inches | `x as f32 / 65536.0 * MW2_PER_DOOM` |
| Doom `angle_t a` | MW2 yaw degrees | `a as f64 / BAM_PER_DEGREE` wrapped to (-180, 180] |
| MW2 yaw degrees | Doom `angle_t` | `(yaw.rem_euclid(360.0) * BAM_PER_DEGREE) as u64 as u32` |
| MW2 pitch degrees `p` | Doom slope (fixed) | `(-p.to_radians().tan() * 65536.0) as i32` |
| MW2 range/radius inches `r` | Doom fixed | `(r * DOOM_PER_MW2 * 65536.0) as i32` |
| Doom velocity (fixed per tic) | MW2 inches/s | `v / 65536 * MW2_PER_DOOM * 35` |

### 5.2 Rendering

```rust
pub const DOOM_SCREEN_W: usize = 320;
pub const DOOM_SCREEN_H: usize = 200;
/// Displayed pixel aspect (width / height) of a 320x200 frame shown at 4:3.
pub const PIXEL_ASPECT: f32 = 5.0 / 6.0;
/// MW2 cg_fov default (horizontal degrees at 4:3).
pub const MW2_CG_FOV: f32 = 65.0;
/// Doom's fixed horizontal field of view.
pub const DOOM_HFOV: f32 = 90.0;
/// Default viewmodel horizontal FOV (doommw2.toml [render] viewmodel_fov).
pub const VIEWMODEL_FOV_DEFAULT: f32 = 65.0;
pub const NUMCOLORMAPS: usize = 32;
pub const LIGHT_SHADE_STEPS: i32 = 6; // max colormap levels the directional term adds/removes (sheet 11 §5.6)
```

### 5.3 Time

```rust
pub const TICRATE: u32 = 35;
/// msec handed to MW2 logic for Doom tic number `tic` (0-based within the level).
pub const fn tic_msec(tic: u32) -> i32 {
    (((tic as u64 + 1) * 1000 / 35) - (tic as u64 * 1000 / 35)) as i32
}
/// MW2 server_time at the *start* of tic `tic` = tic as u64 * 1000 / 35.
pub const fn tic_server_time(tic: u32) -> i32 { (tic as u64 * 1000 / 35) as i32 }
```

`tic` is `leveltime` (Doom's per-level tic counter), so `server_time` restarts at 0 each level. Every MW2 timer is relative, so the restart is harmless; the shadow PlayerState is re-initialised at level start (sheet 07 §5.2).

### 5.4 Ids

```rust
/// Index into PackManifest::weapons, 1-based. 0 = no weapon. Passed to weapon_iw4 as u32.
#[repr(transparent)] pub struct DmwWeaponId(pub u16);
/// Doom weapontype_t values (wp_fist = 0 … wp_supershotgun = 8); NUMWEAPONS = 9.
pub type DoomWeapon = i32;
/// Doom ammotype_t values (am_clip 0, am_shell 1, am_cell 2, am_misl 3, am_noammo 5).
pub type DoomAmmo = i32;
```

## 6. Crate and module map (file → owner sheet)

### 6.1 `crates/dmw_format` (03)

| File | Content |
|---|---|
| `src/lib.rs` | re-exports; `PACK_FORMAT_VERSION = 1` |
| `src/consts.rs` | §5 constants |
| `src/convert.rs` | §5.1 conversions |
| `src/manifest.rs` | `PackManifest`, `WeaponEntry`, `SoundEntry` |
| `src/facts.rs` | `WeaponFacts` and children; serde remote defs for iw4L types (`remote.rs`) |
| `src/remote.rs` | `#[serde(remote = …)]` mirrors of `weapon_iw4` structs |
| `src/model.rs` | `ViewmodelMesh`, `Skeleton`, `Surface`, `Vertex` |
| `src/anim.rs` | `ClipSet`, `ClipData` (mirror of `xmodel_runtime::AnimClip`), `TrackMap` |
| `src/texture.rs` | `TextureRef`, PNG decode helper |
| `src/sound.rs` | `SoundRef`, WAV decode helper |
| `src/load.rs` | `Pack::open(dir) -> Result<Pack, PackError>`, lazy per-weapon loading |
| `src/validate.rs` | structural validation (sheet 03 §5.8) |
| `src/fixture.rs` + `src/bin/dmw-fixture.rs` | synthetic fixture pack (03 §5.9) |

### 6.2 `crates/dmw_weapons` (07 sim half, 10 anim half)

| File | Owner | Content |
|---|---|---|
| `src/lib.rs` | 07 | public API §7.2 |
| `src/sim.rs` | 07 | `WeaponSim` tic driver |
| `src/shadow.rs` | 07 | shadow `PlayerState` sync |
| `src/inventory.rs` | 07 | `Inventory`, magazines, pools view |
| `src/spread.rs` | 07 | aim spread state + shot direction sampling |
| `src/rng.rs` | 07 | `DmwRng` (xorshift32, seeded per level) |
| `src/events.rs` | 07 | `SimEvent`, `ShotRequest`, `SoundCue` |
| `src/view_kick.rs` | 07 | port of iw4L `ViewKickState` |
| `src/view_sway.rs` | 07 | port of iw4L `ViewSwayState` |
| `src/placement.rs` | 10 | gun placement assembly (port of `apply_fpv_placement` sequence) |
| `src/clip_scheduler.rs` | 10 | port of iw4L `ClipScheduler` |
| `src/controller.rs` | 10 | reduced `ViewmodelController` |
| `src/pose.rs` | 10 | DObj pose → eye-space skin matrices |
| `src/notetracks.rs` | 10 | notetrack → `SoundCue` |

### 6.3 `crates/dmw_viewmodel` (11)

| File | Content |
|---|---|
| `src/lib.rs` | public API §7.3 |
| `src/palette.rs` | `PaletteTables` (PLAYPAL + COLORMAP), nearest-colour LUT |
| `src/quantize.rs` | RGBA8 → indexed texture + mips, dither |
| `src/raster.rs` | triangle setup, clipping, perspective-correct scanline raster |
| `src/shade.rs` | colormap level selection |
| `src/skin.rs` | CPU skinning |
| `src/overlay.rs` | 2D indexed blits (scope overlay, flash patch helper) |

### 6.4 Engine glue `room/src/dmw/` (in the engine crate)

| File | Owner | Content |
|---|---|---|
| `mod.rs` | 08 | module tree, `DmwState` singleton accessor |
| `consts.rs` | 02 | re-export of `dmw_format::consts` |
| `config.rs` | 02 (schema) | `DmwConfig` from `doommw2.toml` |
| `mode.rs` | 08 | mode gating (D15) |
| `ext.rs` | 05 | `ExtTiccmd`, ring buffer, `PlayerExt.pitch` |
| `input.rs` | 05 | mouse accumulation, new bindings, pitch/ext building |
| `hooks.rs` | 08 | **the single file every Doom-module edit calls into** (§7.5) |
| `weapons.rs` | 08 | Doom-side weapon/inventory/ammo glue around `WeaponSim` |
| `ballistics.rs` | 09 | `P_LineAttack3D`, damage application, projectiles |
| `viewmodel.rs` | 11 | builds `ViewmodelDrawParams`, calls rasterizer, flash patch |
| `freelook.rs` | 06 | shear, FOV, yslope rebuild |
| `hud.rs` | 12 | crosshair, hitmarker, ammo counter, scope overlay |
| `audio.rs` | 12 | MW2 sound playback |
| `save.rs` | 08 | savegame extension |

## 7. Cross-sheet Rust contracts

Signatures are binding. Field lists marked "at least" may grow; nothing listed may be removed or renamed without a Revision.

### 7.1 `dmw_format` (owned by 03)

```rust
pub struct Pack { pub dir: PathBuf, pub manifest: PackManifest, /* private caches */ }
impl Pack {
    pub fn open(dir: &Path) -> Result<Pack, PackError>;
    pub fn fixture() -> Pack;                                  // in-memory synthetic pack
    pub fn weapon_id(&self, name: &str) -> Option<DmwWeaponId>;
    pub fn facts(&self, id: DmwWeaponId) -> &WeaponFacts;      // loaded eagerly at open
    pub fn load_viewmodel(&self, id: DmwWeaponId) -> Result<Arc<ViewmodelAsset>, PackError>; // mesh+clips+textures
    pub fn load_sound(&self, alias: &str) -> Result<Arc<SoundAsset>, PackError>;
    pub fn load_overlay(&self, id: DmwWeaponId) -> Result<Option<Arc<RgbaImage>>, PackError>;
}
pub struct WeaponFacts {
    pub name: String,                       // e.g. "m4_mp"
    pub display_name: String,               // localized, e.g. "M4A1"
    pub combat: weapon_iw4::WeaponCombatFacts,
    pub kick: KickFacts,                    // mirror of asset_game::WeaponKickFacts
    pub sway: SwayFacts,                    // hip/ads weapon_iw4::WeaponSwayParams
    pub fpv: FpvFacts,                      // placement inputs (sheet 03 §5.3)
    pub hud: HudFacts,
    pub movement: MovementScales,           // move_speed_scale, ads_move_speed_scale, sprint_duration_scale
    pub anim_times: AnimTimes,
    pub projectile: Option<ProjectileFacts>,
    pub sounds: WeaponSounds,               // alias names per slot; Option<String>
    pub has_overlay: bool,
}
pub struct ViewmodelAsset { pub mesh: ViewmodelMesh, pub skeleton: Skeleton, pub clips: ClipSet, pub textures: Vec<RgbaImage> }
pub struct SoundAsset { pub rate: u32, pub channels: u16, pub samples: Vec<f32> /* interleaved */ }
pub struct RgbaImage { pub width: u32, pub height: u32, pub pixels: Vec<[u8; 4]> }
pub enum PackError { Io(..), Format(String), Version { found: u32, expected: u32 }, Missing(String), Invalid(String) }
```

### 7.2 `dmw_weapons` (owned by 07; anim types by 10)

Final shape, Revisions 2, 3, 5, 6, 7 folded in.

```rust
pub struct WeaponSim { /* shadow PlayerState, hands, kick, sway, placement, rng, inventory */ }

pub struct LevelStart<'a> {
    pub pack: &'a Pack,
    pub seed: u32,                           // DmwRng seed from (gameskill, gameepisode, gamemap)
    pub inventory: Inventory,
    pub ammo_map: BTreeMap<u16, DoomAmmo>,   // DmwWeaponId.0 -> Doom ammo pool
    pub sprint_speed_scale: f32,             // [balance] sprint_speed_scale
    pub weapon_move_scales: bool,            // [balance] weapon_move_scales
    pub viewmodel_fov: f32,                  // [render] viewmodel_fov (MW2 cg_fov space)
}

pub struct TicInput {
    pub tic: u32,                    // leveltime
    pub alive: bool,
    pub buttons: TicButtons,         // §8.1 bits, decoded
    pub requested_weapon: Option<DmwWeaponId>, // MW2 target from Doom's weapon change, already mapped
    pub yaw_deg: f32,                // MW2 convention, from mo->angle
    pub pitch_deg: f32,              // MW2 convention, from PlayerExt
    pub prev_yaw_deg: f32,
    pub prev_pitch_deg: f32,
    pub origin_mw2: [f32; 3],        // converted §5.1
    pub velocity_mw2: [f32; 3],      // inches/s, already multiplied by SHADOW_VELOCITY_SCALE (§5.1)
    pub on_ground: bool,
    pub forwardmove: i8, pub sidemove: i8,   // Doom ticcmd values
    pub pools: [i32; 4],             // Doom ammo pools in, by DoomAmmo
}
pub struct TicOutput {
    pub events: Vec<SimEvent>,
    pub pools: [i32; 4],             // pools after reloads
    pub view_kick_deg: [f32; 3],     // camera kick (pitch, yaw, roll), visual only
    pub move_speed_scale: f32,       // product of weapon, ADS and sprint scales (sheet 08 §5.9)
    pub sprinting: bool,
    pub ads_frac: f32,
    pub spread_deg: f32,             // current spread for crosshair
    pub held: Option<DmwWeaponId>,   // weapon currently in hand (None during vanilla handoff)
    pub weapon_lowered_for_vanilla: bool, // drop finished, Doom may raise a vanilla weapon
}
pub enum SimEvent {
    Shot(ShotRequest),
    Melee(MeleeRequest),
    Sound(SoundCue),
    EmptyClick,
    ReloadStarted { empty: bool },
    ReloadFinished,
    RaiseStarted(DmwWeaponId),
    DropFinished(DmwWeaponId),
    MuzzleFlash { weapon: DmwWeaponId, last_shot: bool },
}
pub struct ShotRequest {
    pub weapon: DmwWeaponId,
    pub pellets: Vec<[f32; 3]>,     // unit directions, MW2 world frame = Doom world axes (x, y, z-up)
    pub base_damage: i32,            // facts.combat.damage
    pub max_range_mw2: f32,          // facts.combat.bullet_range()
    pub projectile: bool,            // true → spawn ProjectileFacts missile instead of hitscan
}
pub struct MeleeRequest { pub weapon: DmwWeaponId, pub damage: i32, pub range_mw2: f32, pub dir: [f32; 3] }
pub struct SoundCue { pub alias: String, pub local: bool }

impl WeaponSim {
    pub fn new(pack: Arc<Pack>) -> Self;
    pub fn start_level(&mut self, start: LevelStart<'_>);
    pub fn tic(&mut self, input: &TicInput) -> TicOutput;
    pub fn request_vanilla(&mut self, doom_weapon: DoomWeapon); // drop the MW2 weapon, then hand over to Doom psprites
    pub fn on_respawn(&mut self);                               // new life: clear first-raise memory, reset kick/sway/spread
    pub fn inventory(&self) -> &Inventory;
    pub fn inventory_mut(&mut self) -> &mut Inventory;
    pub fn view_state(&self) -> ViewState;   // everything sheets 10/11/12 need to draw
    pub fn save(&self) -> SimSave;           // sheet 08 §5.11
    pub fn restore(&mut self, save: SimSave);
}

pub struct Inventory {
    pub owned: Vec<DmwWeaponId>,
    pub magazines: BTreeMap<u16, i32>,       // DmwWeaponId.0 → rounds in magazine
    pub last_held: Option<DmwWeaponId>,
}
/// Stored by name so a save survives a re-export with a different weapon order.
#[derive(Serialize, Deserialize)]
pub struct SimSave {
    pub owned: Vec<String>,
    pub magazines: BTreeMap<String, i32>,
    pub last_held: Option<String>,
    pub held: Option<String>,
}

/// Snapshot for presentation, produced after tic().
pub struct ViewState {
    pub weapon: Option<DmwWeaponId>,
    pub weaponstate: i32,                    // weapon_iw4::WeaponState as i32
    pub weap_anim: i32,                      // raw, edges consumed by sheet 10
    pub ads_frac: f32,
    pub placement: GunPlacement,             // view-space origin (inches) + angles (deg), sheet 10
    pub gun_angle_offset: [f32; 2],          // (pitch, yaw) deg between gun axis and view, sheet 10 §5.3
    pub view_kick_deg: [f32; 3],
    pub spread_deg: f32,
    pub overlay_active: bool,                // scope overlay fully up (sheet 12)
    pub zoom_hfov_deg: Option<f32>,          // world FOV override in Doom space (sheet 06 §5.5)
    pub viewmodel_fov_deg: f32,              // gun FOV in MW2 space (sheet 07 §5.11, 11 §6.2)
    pub clip: i32, pub stock: i32,
}
pub struct GunPlacement { pub origin: [f32; 3], pub angles: [f32; 3] }

// anim half (sheet 10)
/// Eye-space skinning matrices (MW2 view axes: x forward, y left, z up; inches), one per DObj bone,
/// already multiplied by the inverse bind matrix and by the gun placement transform.
pub struct ViewmodelPose { pub skin: Vec<glam::Mat4>, pub tag_flash: Option<glam::Mat4> }
pub struct SurfaceMask { pub rocket: bool, pub knife: bool }
pub struct ViewmodelAnimator { /* ClipScheduler, controller, dobj, clips */ }
impl ViewmodelAnimator {
    pub fn new(asset: Arc<ViewmodelAsset>, facts: &WeaponFacts) -> Result<Self, AnimError>;
    pub fn advance(&mut self, view: &ViewState, msec: i32) -> Vec<SoundCue>; // notetracks
    pub fn pose(&self, view: &ViewState) -> ViewmodelPose;
    pub fn visibility(&self, view: &ViewState) -> SurfaceMask;
}
```

### 7.3 `dmw_viewmodel` (owned by 11)

Final shape, Revision 6 folded in.

```rust
pub struct PaletteTables { pub playpal: [[u8; 3]; 256], pub colormaps: Vec<[u8; 256]> /* 34 */, /* lut6 */ }
impl PaletteTables {
    pub fn from_wad_lumps(playpal: &[u8], colormap: &[u8]) -> Result<Self, RasterError>;
    pub fn nearest(&self, rgb: [u8; 3]) -> u8;
}

pub struct IndexedTexture { pub mips: Vec<IndexedMip> }
pub struct IndexedMip { pub width: u32, pub height: u32, pub index: Vec<u8>, pub opaque: Vec<u8> }
pub enum Dither { None, Bayer4 }
pub fn quantize(image: &RgbaImage, pal: &PaletteTables, dither: Dither, alpha: AlphaMode) -> IndexedTexture;

pub struct PreparedViewmodel { /* quantized textures, mesh refs, winding */ }
impl PreparedViewmodel { pub fn new(asset: &ViewmodelAsset, pal: &PaletteTables, dither: Dither) -> Result<Self, RasterError>; }

pub struct DrawTarget<'a> {
    pub pixels: &'a mut [u8],   // Doom screen buffer, 320*200, row-major
    pub pitch: usize,           // 320
    pub view_x: i32, pub view_y: i32, pub view_w: i32, pub view_h: i32, // Doom view window
    /// Principal point inside the view window, **unsheared**: (view_w/2, view_h/2). Under y-shearing
    /// the aim direction always projects to the unsheared centre, and the viewmodel is camera-attached,
    /// so it must not move with the world's shear (sheet 06 §5.2).
    pub center_x: f32, pub center_y: f32,
    pub pixel_aspect: f32,      // displayed pixel width / height (5/6 at 4:3)
}
/// Same shape as dmw_weapons::ViewmodelPose; the engine converts (01 §5.3). Matrices are eye space,
/// MW2 view axes (x forward, y left, z up), inches.
pub struct Pose { pub skin: Vec<glam::Mat4>, pub tag_flash: Option<glam::Mat4> }
pub struct Visibility { pub rocket: bool, pub knife: bool }
pub enum LightParams<'a> {
    Sector { colormap_level: u8 /* 0 bright … 31 dark */, directional: bool },
    Fixed { colormap: u8 /* index into colormaps: 0 fullbright, 32 invuln */ },
    Fuzz { fuzzoffset: &'a [i32], fuzzpos: &'a mut usize },   // partial invisibility
}
pub struct ViewmodelDrawParams<'a> {
    pub pose: &'a Pose,
    pub hfov_deg: f32,          // ViewState::viewmodel_fov_deg
    pub light: LightParams<'a>,
    pub pal: &'a PaletteTables,
    pub visibility: Visibility,
}
pub struct Rasterizer { /* z-buffer 320x200 f32, scratch */ }
impl Rasterizer {
    pub fn new() -> Self;
    pub fn draw(&mut self, target: &mut DrawTarget<'_>, model: &PreparedViewmodel, params: &ViewmodelDrawParams<'_>) -> DrawStats;
}
```

### 7.4 Engine extension state (owned by 05, extended by 08)

```rust
// room/src/dmw/ext.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtTiccmd {
    pub tic: i32,          // maketic it was built for; -1 = empty slot
    pub pitch_delta: i16,  // 1/128 degree, MW2 convention (positive = down)
    pub buttons: u16,      // EXT_BT_* (§8.1)
}
pub static mut EXT_TICCMDS: [ExtTiccmd; BACKUPTICS];   // indexed by maketic % BACKUPTICS
pub fn ext_for_gametic(gametic: i32, ticdup: i32) -> ExtTiccmd; // neutral (tic=-1) if stale

#[derive(Clone, Debug, Default)]
pub struct PlayerExt {
    pub pitch_q: i32,          // accumulated pitch, 1/128 degree, clamped to ±PITCH_LIMIT_DEG
    pub cmd: ExtTiccmd,        // copied in G_Ticker next to players[i].cmd
    pub prev_yaw: u32,         // mo->angle at previous tic (BAM)
    pub prev_pitch_q: i32,
    pub view_kick_deg: [f32; 3], // written by 08 from TicOutput, read by 06
    pub zoom_hfov_deg: Option<f32>, // written by 08, read by 06
}
pub static mut PLAYER_EXT: [PlayerExt; MAXPLAYERS];
```

`PlayerT`, `TiccmdT`, `mobj_t` and every other `#[repr(C)]` Doom struct keep their exact layout (the C layout guard tests in `room/src/doom/c_tests` must stay green).

### 7.5 Engine hook table (owned by 08; each row's body owned by the listed sheet)

Every edit inside `room/src/doom/*.rs` is a single call into `crate::dmw::hooks` guarded so that the vanilla path is untouched when MW2 mode is off. `hooks.rs` functions are `pub(crate) unsafe fn` taking raw Doom pointers.

| Doom function (file) | Hook call (in `dmw::hooks`) | Effect | Body owner |
|---|---|---|---|
| `D_DoomMain` (d_main.rs), after `W_Init`/before `G_InitNew` | `on_engine_init()` | load config, pack, palette tables | 08 |
| `G_DoLoadLevel` (g_game.rs), end | `on_level_start()` | evaluate D15, `WeaponSim::start_level`, prepare viewmodels | 08 |
| `G_BuildTiccmd` (g_game.rs), end | `on_build_ticcmd(cmd, maketic)` | mouse pitch, ext buttons, MW2 bindings | 05 |
| `G_Responder` (g_game.rs) `ev_mouse` arm | `mouse_look_active() -> bool`, then `input::on_mouse_event(ev)`, `input::scale_yaw(MOUSEX)` | accumulate pitch, zero `MOUSEY`, zoom-scale yaw | 05 |
| `G_BuildTiccmd` (g_game.rs), after `speed` is computed | `force_run() -> bool` | always-run in MW2 mode; also masks `mousebstrafe`/`mousebforward` | 05 |
| `I_GetEvent` (i_input.rs), start | `DG_GetMouse` C-ABI callback (platform) | post one `ev_mouse` | 05 |
| `G_Ticker` (g_game.rs), after net cmd copy | `on_ticcmd_copied(player_index)` | copy ext into `PLAYER_EXT` | 05 |
| `P_PlayerThink` (p_user.rs), before `P_MovePsprites` | `on_player_think(player) -> bool` | run `WeaponSim::tic`; returns `true` when MW2 owns the weapon this tic (skip `P_MovePsprites` and the BT_CHANGE block) | 08 |
| `P_MovePlayer` (p_user.rs), before `P_Thrust` | `scale_move(player, forward, side) -> (i32, i32)` | MW2 move-speed scales | 08 |
| `P_BringUpWeapon` (p_pspr.rs), first statement | `on_bring_up_weapon(player) -> bool` | MW2 raise instead of psprite raise (also covers `P_SetupPsprites`) | 08 |
| `G_InitNew` (g_game.rs), end | `on_new_game()` | clear MW2 inventory | 08 |
| `P_GiveAmmo` (p_inter.rs) | `scale_ammo_pickup(ammo, num) -> c_int` | pickup scaling (§9.2); weapon ownership is synced each tic (08 §5.4) | 08 |
| `P_DamageMobj` (p_inter.rs) | none (09 calls it) | — | — |
| `A_Explode` (p_enemy.rs), first statement | `on_explode(thing) -> bool` | MW2 explosion for MW2 rockets | 09 |
| `P_RemoveMobj` (p_mobj.rs), first statement | `on_remove_mobj(mo)` | forget MW2 missiles that vanish | 09 |
| `R_SetupFrame` (r_main.rs), end | `on_setup_frame(player)` | shear, view kick, FOV zoom | 06 |
| `R_ExecuteSetViewSize` (r_main.rs), end | `on_view_size_changed()` | rebuild shear/yslope tables | 06 |
| `R_DrawPlayerSprites` (r_things.rs), start | `draw_player_weapon() -> bool` | software viewmodel; `true` = skip vanilla psprites | 11 |
| `R_DrawPlayerSprites` (r_things.rs), around the vanilla psprite loop | `freelook::with_unsheared_center(f)` | vanilla psprites (fist, chainsaw) do not move with pitch | 06 |
| `D_Display` (d_main.rs), right after `R_RenderPlayerView` | `draw_view_overlays()` | crosshair, hitmarker, scope overlay | 12 |
| `ST_Drawer`/`ST_updateWidgets` (st_stuff.rs) | `status_ammo(player) -> Option<c_int>` | magazine count in the AMMO widget | 12 |
| `S_StartSound` path (s_sound.rs) | none; 12 adds `dmw::audio::play` | — | 12 |
| `P_ArchivePlayers`… (p_saveg.rs) / `G_DoSaveGame`, `G_DoLoadGame` | `save_extension()`, `load_extension()` | savegame block | 08 |
| `M_Responder`/menu open, window focus loss | `release_mouse()` | mouse capture release | 05 |
| `gpu.rs` `render` | n/a (Rust side) | 4:3 letterbox | 06 |

Sheets 05, 06, 09, 11, 12 write their hook bodies in their own files (`input.rs`, `freelook.rs`, …); 08 owns `hooks.rs` and adds the one-line delegations. Two agents never edit the same Doom module function: if a row above lists your sheet, you own that call site.

## 8. Bit values and enums

### 8.1 Extended buttons (`ExtTiccmd::buttons`)

```rust
pub const EXT_BT_ADS: u16 = 1 << 0;          // hold to aim down sights
pub const EXT_BT_RELOAD: u16 = 1 << 1;
pub const EXT_BT_SPRINT: u16 = 1 << 2;       // hold
pub const EXT_BT_MELEE: u16 = 1 << 3;
pub const EXT_BT_HOLD_BREATH: u16 = 1 << 4;  // sniper scope breath (shares the sprint key while ADS, sheet 05 §5.4)
pub const EXT_BT_SWAP_LAST: u16 = 1 << 5;    // switch to last held weapon
pub const EXT_BT_ATTACK: u16 = 1 << 6;       // mirror of BT_ATTACK so dmw code never decodes Doom bits
pub const EXT_BT_VALID: u16 = 1 << 15;       // slot was written by on_build_ticcmd
```

`TicButtons` in `dmw_weapons` is the decoded form (`ads`, `reload`, `sprint`, `melee`, `hold_breath`, `swap_last`, `attack` booleans).

### 8.2 MW2 button mapping into `WeaponCmd::buttons`

Constants are in `playerstate_iw4/src/usercmd.rs` (`pub mod buttons`), verified at `fc3d8ea`:

| Ext bit | `playerstate_iw4::buttons` | Value | Consumed by |
|---|---|---|---|
| `EXT_BT_ATTACK` | `ATTACK` | `0x1` | `weapon_iw4` via `WeaponCmd::buttons` |
| `EXT_BT_SPRINT` | `SPRINT` | `0x2` | `movement_iw4::update_sprint` (UserCmd) and `WeaponCmd::pm_flags` result |
| `EXT_BT_MELEE` | `MELEE_CHARGE` | `0x4` | IW4's melee button; `weapon_iw4` melee path |
| `EXT_BT_RELOAD` | `RELOAD` | `0x10` | `weapon_iw4` |
| `EXT_BT_ADS` | `ADS` | `0x800` | `movement_iw4::update_ads_intent` (UserCmd) → `pm_flags::ADS_INTENT` |
| `EXT_BT_HOLD_BREATH` | `BREATH` | `0x2000` | sniper breath (sheet 07 §5.7) |

Sheet 07 builds a `playerstate_iw4::UserCmd` and a `weapon_iw4::WeaponCmd` from these each tic.

### 8.3 New `default.cfg` variables (owned by 05)

| Variable | Default | Meaning |
|---|---|---|
| `key_dmw_reload` | `'r'` (114) | reload |
| `key_dmw_ads` | `-1` (unbound key) | ADS on a key |
| `mouseb_dmw_ads` | `1` (right button) | ADS on mouse |
| `key_dmw_sprint` | `KEY_RSHIFT` | sprint / hold breath |
| `key_dmw_melee` | `'v'` (118) | melee |
| `mouseb_dmw_melee` | `3` (mouse4) | melee |
| `key_dmw_swap` | `'q'` (113) | last weapon |
| `mouse_dmw_look` | `1` | mouse Y controls pitch (0 = vanilla forward/back) |
| `mouse_dmw_invert` | `0` | invert pitch |
| `mouse_dmw_pitch_sens` | `5` | pitch sensitivity, same scale as `mouse_sensitivity` |

When MW2 mode is active, `mousebstrafe` and `mousebforward` are ignored (right button = ADS, middle = unbound) and `key_speed` becomes sprint: the player always runs (sheet 05 §5.4).

### 8.4 Log targets

`dmw::config`, `dmw::pack`, `dmw::input`, `dmw::freelook`, `dmw::sim`, `dmw::ballistics`, `dmw::anim`, `dmw::raster`, `dmw::hud`, `dmw::audio`, `dmw::save`, `dmw_export`. Level rules are in sheet 01 §5.7.

## 9. Defaults (values owned by the named sheet, fixed here so every sheet agrees)

### 9.1 Default weapon mapping (`[weapons]`, sheet 08)

| Doom slot | Doom weapon | MW2 weapon (`*_mp` asset name) | Doom ammo pool |
|---|---|---|---|
| 1 | `wp_fist` | `vanilla` | none |
| 1 | `wp_chainsaw` | `vanilla` | none |
| 2 | `wp_pistol` | `usp_mp` (USP .45) | `am_clip` |
| 3 | `wp_shotgun` | `spas12_mp` (SPAS-12) | `am_shell` |
| 3 | `wp_supershotgun` | `ranger_mp` (Ranger) | `am_shell` |
| 4 | `wp_chaingun` | `m4_mp` (M4A1) | `am_clip` |
| 5 | `wp_missile` | `rpg_mp` (RPG-7) | `am_misl` |
| 6 | `wp_plasma` | `mg4_mp` (MG4) | `am_cell` |
| 7 | `wp_bfg` | `barrett_mp` (Barrett .50cal) | `am_cell` |

The exporter's default roster (sheet 04 §5.2) is exactly the non-vanilla names above plus `cheytac_mp`, `aa12_mp`, `kriss_mp`, `deserteagle_mp` (available for remapping). Names are verified against the real registry by 04; a missing name is a hard export error listing close matches.

### 9.2 Ammo (`[ammo]`, sheet 08)

| Pool | Doom max (no backpack) | DOOMMW2 max | Pickup multiplier |
|---|---|---|---|
| `am_clip` (bullets) | 200 | 360 | ×3 |
| `am_shell` | 50 | 64 | ×1 |
| `am_cell` | 300 | 300 | ×0.5 (rounded up, min 1) |
| `am_misl` | 50 | 12 | ×1 |

Backpack doubles the DOOMMW2 max, as in Doom. Rounds per shot: 1 for every bullet weapon (pellets share one shell); 1 rocket per RPG shot; Barrett and MG4 consume 1 cell per round.

### 9.3 Balance (`[balance]`, sheet 09)

| Key | Default | Meaning |
|---|---|---|
| `monster_damage_scale` | `0.35` | multiplier on MW2 damage vs monsters |
| `explosive_damage_scale` | `0.6` | multiplier on MW2 explosion damage |
| `headshot_enabled` | `true` | hit-height → hit location (sheet 09 §5.4) |
| `melee_damage_scale` | `1.0` | MW2 melee (135 in MP) × this, vs monsters |
| `alert_monsters_on_fire` | `true` | `P_NoiseAlert` on every shot (Doom behaviour) |
| `sprint_speed_scale` | `1.5` | MW2 `player_sprintSpeedScale` applied on top of Doom run speed (sheet 08 §5.9) |
| `weapon_move_scales` | `true` | apply MW2 per-weapon move speed and ADS move speed scales to Doom movement |

### 9.4 Render (`[render]`, sheets 06/11)

| Key | Default |
|---|---|
| `aspect_correct` | `true` |
| `viewmodel_fov` | `65.0` |
| `dither` | `"bayer4"` (`"none"` allowed) |
| `directional_shading` | `true` |
| `muzzle_flash` | `"doom"` (Doom flash patch at `tag_flash`) |

## 10. Conventions every sheet must state for its own scope

Each sheet's §6 lists its error cases, §7 its log lines (target + level + message shape), §9 its tests, §10 its acceptance criteria, §11 its contract change requests (or "None").

## 11. Revision log

Every revision below is already folded into the sections it names; the log records why each changed.

### Revision 1 — 2026-10-15 (from sheet 04 §11)

- 03 §5.5 `PartRole` = `Hands, Gun, Attachment, Rocket, Knife, ScopeLens` (iw4L `FpvPartRole` has Rocket and Knife). Applied in sheet 03.

### Revision 2 — 2026-10-15 (from sheet 07 §11)

§7.2 additions (binding):

```rust
pub struct LevelStart<'a> {
    pub pack: &'a Pack, pub seed: u32, pub inventory: Inventory,
    pub ammo_map: BTreeMap<u16, DoomAmmo>,   // DmwWeaponId.0 -> Doom ammo pool
    pub sprint_speed_scale: f32,             // [balance] sprint_speed_scale
    pub weapon_move_scales: bool,            // [balance] weapon_move_scales
}
impl WeaponSim {
    pub fn request_vanilla(&mut self, doom_weapon: DoomWeapon); // drop the MW2 weapon, then hand over to Doom psprites
    pub fn on_respawn(&mut self);                               // new life: clear first-raise memory, reset kick/sway/spread
}
```

### Revision 3 — 2026-10-15 (from sheet 08 §11)

- §7.5 hook table: `P_BringUpWeapon → on_bring_up_weapon` replaces the `P_SetupPsprites` row; the `P_GiveWeapon` row is removed; `G_InitNew → on_new_game` added. Applied in the table above.
- §7.2: `SimSave` is `#[derive(Serialize, Deserialize)]` and stores weapon **names** (`String`), never `DmwWeaponId`s: `pub struct SimSave { pub owned: Vec<String>, pub magazines: BTreeMap<String, i32>, pub last_held: Option<String>, pub held: Option<String> }`.

### Revision 4 — 2026-10-15 (from sheet 09 §11)

- §7.5: rows `A_Explode → on_explode` and `P_RemoveMobj → on_remove_mobj` added (owner 09). Applied in the table.

### Revision 5 — 2026-10-15 (from sheet 10 §11)

- 03 §5.8: skeleton limit is 192 bones (`anim_iw4::PartBits::CAPACITY`). Applied in sheet 03.
- §7.2: `impl ViewmodelAnimator { pub fn visibility(&self, view: &ViewState) -> SurfaceMask }` with `pub struct SurfaceMask { pub rocket: bool, pub knife: bool }` (sheet 10 §5.8); sheet 11 skips `PartRole::Rocket`/`Knife` surfaces whose flag is false and always skips `ScopeLens`.

### Revision 6 — 2026-10-15 (from sheet 11 §11)

§7.3 (binding, supersedes the earlier text where they differ):

```rust
pub struct DrawTarget<'a> {
    pub pixels: &'a mut [u8], pub pitch: usize,
    pub view_x: i32, pub view_y: i32, pub view_w: i32, pub view_h: i32,
    pub center_x: f32, pub center_y: f32,      // unsheared
    pub pixel_aspect: f32,                     // displayed pixel width / height (5/6 at 4:3)
}
pub struct Visibility { pub rocket: bool, pub knife: bool }
pub enum LightParams<'a> {
    Sector { colormap_level: u8, directional: bool },
    Fixed { colormap: u8 },
    Fuzz { fuzzoffset: &'a [i32], fuzzpos: &'a mut usize },
}
pub struct ViewmodelDrawParams<'a> {
    pub pose: &'a Pose, pub hfov_deg: f32, pub light: LightParams<'a>,
    pub pal: &'a PaletteTables, pub visibility: Visibility,
}
pub fn quantize(image: &RgbaImage, pal: &PaletteTables, dither: Dither, alpha: AlphaMode) -> IndexedTexture;
```

§7.2: `ViewState` gains `pub viewmodel_fov_deg: f32` (07 §5.11 computes `lerp([render] viewmodel_fov, ads FOV, zoom progress)`); `LevelStart` gains `pub viewmodel_fov: f32`.

### Revision 7 — 2026-10-15 (from sheet 12)

- §7.2: `ViewState` gains `pub gun_angle_offset: [f32; 2]` (pitch, yaw degrees; 07 step 13 value) so 08 (scope sway) and 12 (crosshair drift) do not recompute it.
- 02 §5.7 `HudCfg.weapon_names` (default `true`); 04 §5.8 always exports `mp_hit_alert` when present. Applied in those sheets.

### Revision 8 — 2026-10-16 (from sheet 11 §11 item 3)

- §7.3: `Rasterizer::draw(&mut self, target: &mut DrawTarget<'_>, model: &PreparedViewmodel, params: &mut ViewmodelDrawParams<'_>) -> DrawStats` — `params` is `&mut` so `LightParams::Fuzz { fuzzpos }` can be written back. The interim `draw_mut` added by the crate half is folded into `draw`; `last_fuzzpos()` may stay as a convenience. The engine half (11 §6) implements against this signature.
