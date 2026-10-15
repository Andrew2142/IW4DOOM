# 11 — Software Viewmodel Rasterizer and Retro Pipeline

Owner: build agent — crate half (§5.1–5.8) in wave 3, engine half (§6) in wave 5
Status: implementation-ready
Depends on: 00 (§2 D6–D8, D16; §5.2; §7.3; Revisions 5–6), 01 (§5.6, §5.8), 03 (mesh/texture types); engine half also 06 (view window, unsheared centre), 08 (state, muzzle timer), 10 (pose, visibility)

The MW2 gun is drawn by a small CPU rasterizer straight into Doom's 8-bit 320×200 screen buffer from inside `R_DrawPlayerSprites`, where Doom draws its own weapon sprites. Textures are quantized to the IWAD's PLAYPAL when a level starts; every pixel's brightness goes through the IWAD's COLORMAP using the player's sector light, so the gun is lit, flashes, inverts under invulnerability and goes fuzzy under partial invisibility exactly like Doom's psprites — and everything Doom draws afterwards (HUD messages, menus, the pause sign) lands on top of it for free.

---

## 1. Purpose

- A 3D, skinned, animated MW2 viewmodel that looks like it was drawn by id in 1993: 320×200, 256 colours, COLORMAP lighting, hard alpha edges, nearest texels.
- Deterministic and testable without a GPU (golden images in CI-less tests).
- ≤ 2 ms per frame for ≤ 20 000 triangles (01 §5.8).

## 2. Scope / Out of scope

Crate half (`crates/dmw_viewmodel`, no Doom engine dependency): palette tables, nearest-colour LUT, texture quantization with mips and optional Bayer dither, CPU skinning, projection, near-plane clipping, perspective-correct textured raster with z-buffer and alpha test, COLORMAP shading with a directional term, fixed colormaps, fuzz, Doom patch blitter, overlay blitter.

Engine half (`room/src/dmw/viewmodel.rs`): `draw_player_weapon()` hook body, light level derivation, view-window target construction, prepared-model registry and caching, muzzle flash patch, invisibility/invulnerability/light-amp handling.

Out of scope: crosshair, hitmarker and scope overlay drawing (12 — though 12 uses this crate's `overlay.rs` blitters), world rendering (06), high-resolution mode (D8), particles/FX.

## 3. Dependencies

| From | What |
|---|---|
| 00 §7.3 + Rev 6 | API (`PaletteTables`, `quantize`, `PreparedViewmodel`, `DrawTarget`, `ViewmodelDrawParams`, `Pose`, `LightParams`, `Rasterizer`, `Visibility`) |
| 03 | `ViewmodelMesh` (positions, normals, uvs, skin, indices, surfaces with `AlphaMode`, `CullMode`, `PartRole`, `texture`), `RgbaImage` |
| 10 | `ViewmodelPose { skin, tag_flash }` (eye space, MW2 view axes x fwd / y left / z up, inches), `SurfaceMask` |
| 08 | `DmwState.viewmodels` registry, `MuzzleState.flash_tics`, `level_active`, mapping |
| room facts | `R_DrawPlayerSprites` (r_things.rs:965) light: `lightnum = (sector.lightlevel >> LIGHTSEGSHIFT) + extralight`, `spritelights = scalelight[clamp(lightnum)]`, psprite uses `spritelights[MAXLIGHTSCALE-1]` unless `fixedcolormap`; `scalelight[i][j]` are pointers into `colormaps` (`level*256`); invisibility → `R_DrawFuzzColumn` (`colormaps + 6*256`, `fuzzoffset[]`, `fuzzpos`); screen buffer `I_VideoBuffer` 320×200 with view window `viewwindowx/viewwindowy/scaledviewwidth/viewheight`; flash sprite lumps `PISFA0`, `SHTFA0`, `SHTFB0`, `CHGFA0`, `CHGFB0`, `MISFA0`–`MISFD0`, `PLSFA0`, `BFGFA0` |

## 4. File inventory

| Path | Content |
|---|---|
| `crates/dmw_viewmodel/src/lib.rs` | API |
| `src/palette.rs` | `PaletteTables`, 6-bit RGB → index LUT (§5.1) |
| `src/quantize.rs` | mips, dither, indexed textures (§5.2) |
| `src/prepared.rs` | `PreparedViewmodel` (§5.3) |
| `src/skin.rs` | CPU skinning (§5.4) |
| `src/raster.rs` | projection, clipping, triangle raster, z-buffer (§5.5) |
| `src/shade.rs` | colormap level, directional term, fuzz (§5.6) |
| `src/overlay.rs` | Doom patch blit, RGBA→indexed overlay blit (§5.7) |
| `tests/golden/*.png`, `tests/raster.rs` | §9 |
| `room/src/dmw/viewmodel.rs` | engine half (§6) |

## 5. Detailed design — crate half

### 5.1 Palette tables

```rust
pub struct PaletteTables {
    pub playpal: [[u8; 3]; 256],          // PLAYPAL palette 0 (un-gamma'd lump values)
    pub colormaps: Vec<[u8; 256]>,        // COLORMAP: 34 maps (0..31 light, 32 invulnerability, 33 black)
    lut6: Box<[u8; 64 * 64 * 64]>,        // nearest index for each 6-bit RGB cell centre
}
```

- `from_wad_lumps(playpal: &[u8], colormap: &[u8])` validates lengths (≥ 768, ≥ 34·256).
- Distance: "redmean" weighted RGB (`r̄ = (r1+r2)/2`; `d² = (2 + r̄/256)Δr² + 4Δg² + (2 + (255−r̄)/256)Δb²`). Ties → lowest index.
- `nearest(rgb)` = exact search over 256 (used for tests and LUT build); `nearest_fast(rgb)` = `lut6[(r>>2)<<12 | (g>>2)<<6 | b>>2]`. LUT build ≈ 67 M distance evaluations — build once per IWAD in a background-free loop at engine init (< 100 ms release; measure and log).
- Gamma: quantize against the raw PLAYPAL; Doom's gamma (`usegamma`) is applied later to the whole screen by `I_SetPalette`, so the gun follows the user's gamma setting automatically.

### 5.2 Quantization

```rust
pub fn quantize(image: &RgbaImage, pal: &PaletteTables, dither: Dither, alpha: AlphaMode) -> IndexedTexture;
```

1. Mip chain: level 0 = source; each next level = 2×2 box filter in linear light (sRGB→linear, average, →sRGB) with alpha averaged; stop at 1×1. Non-power-of-two sizes: floor division, last row/column clamped.
2. Per mip texel: `opaque = match alpha { Opaque => true, Test{cutoff} => a >= cutoff·255, Blend => a >= 128 }`.
3. Colour → index: `Dither::None` → `nearest_fast(rgb)`; `Dither::Bayer4` → add `(bayer4[y&3][x&3] − 7.5) × 4` (in 0..255 units) to each channel, clamp, then `nearest_fast`. Dither is in **texture space** (stable under motion, no screen-door crawl).
4. Store `IndexedMip { width, height, index: Vec<u8>, opaque: BitVec or Vec<u8> }`.

### 5.3 Prepared viewmodel

`PreparedViewmodel::new(asset: &ViewmodelAsset, pal: &PaletteTables, dither: Dither) -> Result<Self, RasterError>`:

- quantizes every texture referenced by a surface (with that surface's `AlphaMode`);
- keeps references to mesh arrays (positions, normals, uvs, skin, indices) — owned `Arc`s, no copies;
- per surface: texture index or `None` → flat colour = `nearest(128,128,128)`;
- detects winding (03 §5.5.1): skin the bind pose with identity bone matrices, count triangles of `CullMode::Back` surfaces facing the origin (eye at origin looking +x) under CCW vs CW, pick the majority; log the result at `debug!`.
- precomputes per-triangle texel area at mip 0 for mip selection.

### 5.4 Skinning

For each vertex `v`: `p = Σ_k w_k · (pose.skin[b_k] · position)`, `n = normalize(Σ_k w_k · (mat3(pose.skin[b_k]) · normal))`. Vertices with one weight use one matrix (fast path). Output buffers (`Vec<[f32;3]>`) are owned by the `Rasterizer` and reused every frame.

### 5.5 Projection and raster

Eye space: x forward, y left, z up (inches). For the `DrawTarget` view window of width `W` (pixels) and the params' `hfov_deg`:

```text
fx = (W / 2) / tan(hfov/2)
fy = fx · target.pixel_aspect           (pixel_aspect = displayed pixel width / height; 5/6 for 4:3 display of 320×200)
sx = target.view_x + target.center_x − fx · (y / x)
sy = target.view_y + target.center_y − fy · (z / x)
```

- Near plane `x ≥ 0.5` in; triangles crossing it are clipped (Sutherland–Hodgman against one plane, 0–2 output triangles). No far plane.
- Back-face cull per detected winding for `CullMode::Back` surfaces; `CullMode::None` draws both sides (normals flipped for shading on back faces).
- Surfaces skipped: `PartRole::ScopeLens` always; `Rocket`/`Knife` when the matching `Visibility` flag is false.
- Raster: top-left fill rule, edge functions over the triangle's clipped bounding box in the view window, per-pixel perspective-correct `u/x`, `v/x`, `1/x` interpolation (affine within 16-pixel spans is acceptable if it stays within 0.25 texel of exact; measure in T11.3).
- Depth: z-buffer of `f32` `1/x` sized 320×200, cleared to 0 over the view window at the start of each draw; test `>`. The world is never depth-tested against (the gun is always in front, like psprites).
- Texture: mip `m = clamp(round(0.5 · log2(texel_area_mip0 / screen_area)), 0, last)` per triangle; nearest texel with wrap addressing; skip pixel if not `opaque`.
- Output: `target.pixels[(y)·pitch + x] = shade(level_at_pixel, texel_index)`.

### 5.6 Shading

`LightParams::Sector { colormap_level, directional }`:

```text
L       = normalize([-0.35, 0.45, 0.82])    // eye space: from the viewer side, above, slightly left
n·l     = max(0, dot(n_vertex, L))
offset  = if directional { round((0.5 − n·l) · 2 · LIGHT_SHADE_STEPS) } else { 0 }    // −6 .. +6
level_v = clamp(colormap_level + offset, 0, 31)   (per vertex)
```

`level` is interpolated linearly across the triangle and rounded per pixel → `colormaps[level][texel]`.

`LightParams::Fixed { colormap }` → `colormaps[colormap][texel]`, no directional term (fullbright = 0, invulnerability = 32, light-amp as Doom's `fixedcolormap`).

`LightParams::Fuzz` → raster into a coverage mask only, then for each covered pixel apply Doom's fuzz exactly as `R_DrawFuzzColumn`: `dest = colormaps[6][screen[pixel + fuzzoffset[fuzzpos]]]`, advancing `fuzzpos` per pixel in column order, wrapping at `FUZZTABLE` (the engine passes room's `fuzzoffset` table and current `fuzzpos` in and gets the new `fuzzpos` back so the effect stays continuous with world fuzz).

### 5.7 Overlay blitters (`overlay.rs`)

- `blit_doom_patch(target, patch_lump: &[u8], cx: f32, cy: f32, scale: f32, colormap: &[u8; 256])` — Doom patch format (width, height, offsets, column posts); centred at `(cx, cy)` (offsets ignored), nearest scaling, clipped to the view window.
- `blit_indexed(target, tex: &IndexedMip, rect, colormap)` — used by 12 for the scope overlay (quantized once at level start with `quantize`).

### 5.8 Rasterizer API

```rust
pub struct Rasterizer { zbuf: Vec<f32>, skinned_p: Vec<[f32;3]>, skinned_n: Vec<[f32;3]>, clip_scratch: Vec<ClipVert>, cover: Vec<u8> }
impl Rasterizer {
    pub fn new() -> Self;
    pub fn draw(&mut self, target: &mut DrawTarget<'_>, model: &PreparedViewmodel, params: &ViewmodelDrawParams<'_>) -> DrawStats;
}
pub struct DrawStats { pub triangles_in: u32, pub triangles_drawn: u32, pub pixels: u32, pub micros: u32 }
```

## 6. Detailed design — engine half

### 6.1 Registry and preparation (`room/src/dmw/viewmodel.rs`)

`Registry { by_weapon: BTreeMap<u16, Entry> }`, `Entry { prepared: Arc<PreparedViewmodel>, animator: ViewmodelAnimator, overlay: Option<IndexedTexture> }`. `prepare_all(state)` (called by 08 at the first MW2 level start, and again only if the IWAD palette changes — never in practice): for every mapped weapon, `pack.load_viewmodel`, `PreparedViewmodel::new(asset, palette, cfg.render.dither)`, `ViewmodelAnimator::new(asset, facts)`, overlay quantized if `has_overlay`. Work per weapon is independent; use `std::thread::scope` across weapons (01 §5.4 allows scoped threads at level start). Log total time at `info!`.

`on_tic(view, msec) -> Vec<SoundCue>` (08 §5.5): `entry(view.weapon).animator.advance(view, msec)`.

### 6.2 `draw_player_weapon() -> bool` (hook at the start of `R_DrawPlayerSprites`)

```text
if !level_active → return false                     (vanilla psprites)
p = viewplayer
if readyweapon is not MW2-mapped and no MW2 hand is up → return false
if p.playerstate != PST_LIVE → return true          (dead: no weapon, like MW2)
if view.overlay_active → return true                (scope overlay replaces the gun; 12 draws it)
light:
  if p.fixedcolormap != 0 → Fixed { colormap: p.fixedcolormap }
  else if p.powers[pw_invisibility] > 4*32 || (p.powers[pw_invisibility] & 8) != 0 → Fuzz
  else → Sector { colormap_level: level_of(spritelights[MAXLIGHTSCALE-1]), directional: cfg.render.directional_shading }
     where level_of(ptr) = (ptr − colormaps) / 256, spritelights chosen exactly as R_DrawPlayerSprites does
target = DrawTarget { pixels: I_VideoBuffer[0..320*200], pitch: 320,
                      view_x: viewwindowx, view_y: viewwindowy, view_w: scaledviewwidth, view_h: viewheight,
                      center_x: scaledviewwidth/2, center_y: viewheight/2,       (unsheared, 06 §5.2)
                      pixel_aspect: if cfg.render.aspect_correct { 5/6 } else { (win_w/320)/(win_h/200) } }
pose = entry.animator.pose(view); vis = entry.animator.visibility(view)
rasterizer.draw(target, entry.prepared, ViewmodelDrawParams { pose: &Pose::from(pose), hfov_deg: view.viewmodel_fov_deg, light, pal, visibility: vis })
muzzle flash (§6.3)
return true
```

### 6.3 Muzzle flash (`[render] muzzle_flash = "doom"`)

When `MuzzleState.flash_tics > 0` and `pose.tag_flash` is `Some`:

- Patch by weapon: `weap_class == WEAPCLASS_PISTOL` → `PISFA0`; `WEAPCLASS_SPREAD` → `SHTFA0`/`SHTFB0` alternating per shot; full-auto bullet (`fire_type == FullAuto`) → `CHGFA0`/`CHGFB0` alternating; projectile → `MISFA0`; everything else (semi/bolt rifles) → `SHTFB0`.
- Position: project the `tag_flash` origin with §5.5; skip if behind the near plane.
- Scale: `pspritescale/FRACUNIT · 24 / x_eye` (24 in is a typical muzzle distance; a muzzle closer than that draws a bigger flash), clamped 0.4..2.0.
- Fullbright: `colormaps[0]`, drawn after the gun with no depth test.
- Missing lump (e.g. a PWAD without it) → skip and `log_once!` warn.

### 6.4 Coordination with 06 and 12

The vanilla psprite loop that follows the hook is wrapped by 06's `with_unsheared_center`; when this hook returns `true` the loop does not run. 12's crosshair/overlays are drawn later from `D_Display` (after `R_RenderPlayerView`), i.e. on top of the gun.

## 7. Error handling and edge cases

| Case | Behaviour |
|---|---|
| PLAYPAL/COLORMAP lump missing or short | `PaletteTables` fails → 08 sets `mode::fail` for the session (no MW2 viewmodels possible) |
| Texture failed to load | surface drawn flat grey, `warn!` once |
| Degenerate triangles, NaN vertices | skipped |
| Gun partly outside the view window | clipped per pixel; status bar never overwritten |
| Low detail mode | gun still drawn at full horizontal resolution (cosmetic difference accepted) |
| Wipe/melt transitions | `R_RenderPlayerView` not running → nothing drawn; correct |

## 8. Logging

`dmw::raster`: `info!` LUT build time and per-weapon preparation time; `debug!` detected winding per weapon; `trace!` `DrawStats` per frame.

## 9. Tests

| Id | Test |
|---|---|
| T11.1 | palette: `nearest_fast` equals `nearest` for 10 000 random colours within 1 index distance class (≥ 99 % exact, the rest equal-distance ties) using a synthetic palette (6×6×6 cube + 40 greys) |
| T11.2 | quantize: a 4×4 gradient with `Dither::None` vs `Bayer4` (golden indices); mip chain sizes for 1024×512 |
| T11.3 | raster: a single textured quad at known pose → golden 320×200 PNG (synthetic palette, colormap 0); perspective-correctness: checkerboard on a quad at 70° matches the exact-per-pixel reference within 0.25 texel |
| T11.4 | z-buffer: two overlapping triangles, nearer wins regardless of submission order |
| T11.5 | near clip: triangle crossing x = 0.5 produces no NaN and no pixels behind the eye |
| T11.6 | shading: same model at `colormap_level` 0, 16, 31 → golden; `Fixed{32}` → only colours present in colormap 32 |
| T11.7 | fuzz: covered pixels change only via colormap 6 of neighbours; `fuzzpos` advances |
| T11.8 | fixture gun golden frames: bind/idle, mid-reload (magazine displaced), ADS (gun centred) at level 8 with directional shading — `crates/dmw_viewmodel/tests/golden/fixture_*.png` (rendered through `dmw_weapons` in a dev-dependency-only test harness) |
| T11.9 | engine headless: `-fixturepack` E1M1 start, frame after 60 tics: the view window contains fixture-gun pixels (colours from the fixture texture's quantized set) in the lower right, and the status bar rows are byte-identical to the same frame with `-nomw2` |
| T11.10 | performance: 20 000-triangle synthetic mesh ≤ 2 ms per `draw` (release), 0 allocations after the first call (counting allocator) |

## 10. Acceptance criteria

- [ ] `task run-fixture`: the fixture gun is drawn in Doom's palette, darkens in dark sectors, brightens when firing (extralight + flash patch), turns white under invulnerability (`IDDQD` does not give invulnerability colormap — use the invulnerability sphere or `IDBEHOLDV`), fuzzes with `IDBEHOLDI`.
- [ ] Menus, messages and the pause sign draw over the gun; the status bar is untouched.
- [ ] T11.1–T11.10 pass.

## 11. Contract changes requested

1. 00 §7.3: `DrawTarget` gains `pub pixel_aspect: f32`; `ViewmodelDrawParams` gains `pub visibility: Visibility` (`pub struct Visibility { pub rocket: bool, pub knife: bool }`, converted from 10's `SurfaceMask`) and `LightParams::Fuzz` carries `{ fuzzoffset: &'a [i32], fuzzpos: &'a mut usize }`; `quantize` takes `alpha: AlphaMode`. — Resolved in 00 Revision 6.
2. 00 §7.2 `ViewState` gains `pub viewmodel_fov_deg: f32` = `lerp([render] viewmodel_fov, ads FOV f, z)` with `f` and `z` as in 07 §5.11 (MW2 draws the gun with the same FOV as the world, so the gun magnifies while zooming). Sheet 07 computes it. — Resolved in 00 Revision 6.
