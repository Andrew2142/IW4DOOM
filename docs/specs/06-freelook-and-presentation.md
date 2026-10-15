# 06 — Freelook Renderer, FOV Zoom and Presentation

Owner: build agent (wave 2)
Status: implementation-ready
Depends on: 00 (§2 D10, D11, D16; §5.1–5.2; §7.4 `PlayerExt`; §7.5 rows `R_SetupFrame`, `R_ExecuteSetViewSize`, `R_DrawPlayerSprites` vanilla loop, `gpu.rs`), 01, 02, 05 (pitch source)

Doom's software renderer has no pitch. This sheet adds Heretic-style y-shearing driven by `PLAYER_EXT.pitch_q` plus the visual camera kick, a variable horizontal FOV for MW2 ADS zoom and sniper scopes, and 4:3 presentation of the 320×200 frame.

---

## 1. Purpose

- Looking up/down moves the horizon (`centery`) so the world appears to pitch, within ±32° (00 §5.1), at no measurable cost.
- MW2's visual camera kick (`PlayerExt::view_kick_deg`, written by 08 from `TicOutput`) perturbs the rendered view without touching aim.
- ADS zoom and scopes narrow Doom's 90° FOV smoothly (`PlayerExt::zoom_hfov_deg`).
- The frame is shown at the 4:3 aspect Doom was drawn for, which the viewmodel projection (sheet 11) assumes.

## 2. Scope / Out of scope

In scope: `room/src/dmw/freelook.rs`; hook call sites in `R_SetupFrame`, `R_ExecuteSetViewSize`, `R_DrawPlayerSprites` (vanilla loop wrapper only); parametric FOV in `R_InitTextureMapping` and `R_ExecuteSetViewSize` (`projection`, `yslope`); sky handling under shear and zoom; `gpu.rs` letterbox and resizable window; restoring vanilla state when MW2 mode is off.

Out of scope: drawing the viewmodel (11), crosshair (12), mouse input (05), roll (Doom cannot roll; kick roll is ignored), uncapped/interpolated rendering.

## 3. Dependencies

| From | What |
|---|---|
| 05 | `PLAYER_EXT[displayplayer].pitch_q` (1/128°, MW2 sign: + = down) |
| 08 | `PLAYER_EXT[displayplayer].view_kick_deg` (pitch, yaw, roll), `zoom_hfov_deg`, `mode::level_active()` |
| room facts | `R_ExecuteSetViewSize` (r_main.rs:844) sets `centery = viewheight/2`, `centerx = viewwidth/2`, `projection = centerxfrac`, `yslope[i] = FixedDiv((viewwidth<<detailshift)/2*FRACUNIT, |i - viewheight/2| + ½)`, `pspritescale`; `R_InitTextureMapping` (r_main.rs:704) builds `viewangletox`/`xtoviewangle`/`clipangle` from `FIELDOFVIEW = 2048` fine angles; `R_SetupFrame` (r_main.rs:1021) sets `viewangle = mo->angle + viewangleoffset`; walls/sprites/planes read `centeryfrac`, `projection`, `yslope`; sky columns use `skytexturemid` and `pspriteiscale>>detailshift`; `R_DrawPSprite` positions with `centeryfrac` via `R_DrawVisSprite` |

## 4. File inventory

| Path | Change |
|---|---|
| `room/src/dmw/freelook.rs` | `ViewParams` cache, `on_setup_frame`, `on_view_size_changed`, `with_unsheared_center`, FOV helpers |
| `room/src/doom/r_main.rs` | hook calls at end of `R_SetupFrame` and `R_ExecuteSetViewSize`; `R_InitTextureMapping` takes the field of view from `freelook::fine_fov()` instead of the constant; `projection`/`yslope` computed through `freelook::focal_fixed()` |
| `room/src/doom/r_plane.rs` / `r_sky.rs` | sky texturemid/iscale adjustment (§5.4) |
| `room/src/doom/r_things.rs` | wrap the vanilla psprite loop in `R_DrawPlayerSprites` with `with_unsheared_center` |
| `room/src/gpu.rs`, `room/src/main.rs` | resizable 4:3 window, letterboxed viewport |
| `room/tests/freelook_headless.rs`, `room/tests/golden/freelook_*.png` | §9 |

## 5. Detailed design

### 5.1 State

```rust
pub struct ViewParams {
    pub shear_px: i32,            // rows the horizon moves; + = horizon moves down (looking up)
    pub hfov_deg: f32,            // 90.0 unless zoomed
    pub base_centery: i32,        // viewheight / 2
    yslope_key: (i32, u32),       // (centery, hfov bits) the yslope table was built for
    fov_key: u32,                 // hfov bits the texture mapping was built for
}
thread_local! { static VIEW: RefCell<ViewParams> }
```

### 5.2 Shear (`on_setup_frame`, end of `R_SetupFrame`)

When `mode::level_active()` is false: if `ViewParams` is not vanilla (shear ≠ 0 or hfov ≠ 90), restore vanilla (`centery = viewheight/2`, `hfov = 90`, rebuild tables) and return. Otherwise:

```text
pitch_deg   = pitch_q / 128 + view_kick_deg[0]                    (MW2 sign, + = down)
pitch_deg   = clamp(pitch_deg, -PITCH_LIMIT_DEG - 5, PITCH_LIMIT_DEG + 5)   (kick may exceed the input clamp a little)
focal_y     = ((viewwidth << detailshift) / 2) / tan(hfov/2)          (pixels; equals projection_y)
shear_px    = round(-tan(pitch_deg) * focal_y)                         (looking down ⇒ negative ⇒ horizon up)
centery     = viewheight/2 + shear_px
centeryfrac = centery << FRACBITS
viewangle  += (view_kick_deg[1] * BAM_PER_DEGREE) as angle_t          (yaw kick, visual only; wrapping_add)
```

Then, if `(centery, hfov)` differs from `yslope_key`, rebuild `yslope[0..viewheight]`:

```text
yslope[i] = FixedDiv(focal_y_fixed, abs(((i - centery) << FRACBITS) + FRACUNIT/2))
```

`viewsin`/`viewcos` are recomputed after the yaw kick (they are derived from `viewangle` in `R_SetupFrame` — the hook runs after them, so it recomputes both).

Aim is unaffected: `mo->angle` and `pitch_q` are not modified by this sheet. The aim direction projects to the **unsheared** centre `(centerx, viewheight/2)`; sheet 11 and 12 use that point (00 §7.3).

`with_unsheared_center(f)`: saves `centery/centeryfrac`, sets them to `viewheight/2`, runs `f`, restores. 06 wraps the vanilla psprite loop in `R_DrawPlayerSprites` with it so the fist/chainsaw sprites do not slide when looking up/down (sheet 11's hook runs before that loop and returns early when the MW2 viewmodel is drawn).

### 5.3 Field of view

`hfov_deg = PLAYER_EXT.zoom_hfov_deg.unwrap_or(90.0)`, clamped to 5..=90.

- `fine_fov()` = `round(hfov_deg / 360 * FINEANGLES)` (2048 at 90°). `R_InitTextureMapping` uses it wherever it used `FIELDOFVIEW`. Rebuild (call `R_InitTextureMapping`) when `fov_key` changes — cost ≈ FINEANGLES/2 + viewwidth iterations, acceptable every tic during an ADS transition.
- `projection = FixedDiv(centerxfrac, tan_fixed(hfov/2))`; `focal_y` as above. `R_ExecuteSetViewSize` calls `freelook::focal_fixed()` for both.
- Sky: `dc_iscale` for sky columns is `pspriteiscale >> detailshift` in vanilla; multiply by `tan(hfov/2)` so the sky zooms with the world.
- Light diminishing uses wall/sprite scale, which grows when zoomed, so zoomed distant surfaces get brighter. Correct for it: the light-index computations that use `scale >> LIGHTSCALESHIFT` (walls: `R_RenderSegLoop`/`rw_scale`; sprites: `R_ProjectSprite` `index = xscale >> (LIGHTSCALESHIFT - detailshift)`; planes: `distscale`/`yslope`-based `planezlight` index) divide the scale by `zoom = tan(45°)/tan(hfov/2)` before indexing. Implement as one helper `freelook::light_scale(scale) -> fixed_t` (identity when not zoomed) called at those three sites. These are additional one-line edits in r_segs.rs, r_things.rs, r_plane.rs owned by this sheet.

### 5.4 Sky under shear

Doom's sky texture is 128 rows; with the horizon lowered by looking up, rows above the texture top wrap (`& 127`) and show a seam. Rule: the 128 texture rows must cover from the horizon up to the top of the view at the maximum up-pitch for the current view size without wrapping.

```text
rows_above_horizon_max = viewheight/2 + max_shear_up      (max_shear_up = tan(PITCH_LIMIT_DEG + 5°) * focal_y)
sky_iscale_y           = (128 * FRACUNIT) / rows_above_horizon_max   (only when larger than the vanilla iscale)
skytexturemid          = 128 * FRACUNIT - ... (anchor texture row 127 at the horizon row centery)
```

The implementer derives the exact `dc_texturemid` so that texture row 127 (bottom of the sky texture) maps to screen row `centery` and row 0 maps to `centery - rows_above_horizon_max`. Vanilla mode keeps vanilla sky math exactly.

### 5.5 Zoom source

08 writes `zoom_hfov_deg` each tic from `ViewState::zoom_hfov_deg` (sheet 07 §5.11 computes it as the Doom-equivalent of MW2's ADS FOV: `doom_hfov = 2·atan(tan(45°) · tan(ads_fov/2) / tan(MW2_CG_FOV/2))`, lerped by MW2's zoom frac). This sheet only consumes it.

### 5.6 Presentation (`gpu.rs`, `main.rs`)

- Window: initial inner size 960×720 (`[render] aspect_correct = true`) or 640×400 (false); `with_resizable(true)`; minimum 320×240.
- Render pass viewport: when `aspect_correct`, the largest rectangle with aspect 4:3 centred in the surface (letterbox/pillarbox bars cleared to black); otherwise the whole surface. Use `render_pass.set_viewport(x, y, w, h, 0.0, 1.0)`. Sampler stays nearest.
- The engine framebuffer stays `DOOMGENERIC_RESX × RESY = 640×400` (`fb_scaling = 2` of 320×200). Nothing in the Doom code changes for presentation.
- Mouse capture (05) is independent of the viewport.

### 5.7 Level transitions

`hooks::on_level_start` (08) calls `freelook::reset()` which sets `ViewParams` to vanilla and sets `setsizeneeded = true` so `R_ExecuteSetViewSize` rebuilds every table from scratch before the first frame of the level.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| `centery` outside `0..viewheight` (big shear with small view) | allowed; `yslope` and wall clipping handle it (rows with `y == centery` use the ½ offset, no division by zero) |
| Low detail mode (`detailshift = 1`) | `focal_y` uses `viewwidth << detailshift`; tested |
| View size changes mid-level | `on_view_size_changed` recomputes from the current pitch/zoom |
| Automap active | `R_RenderPlayerView` not called; nothing to do |
| `zoom_hfov_deg` NaN/out of range | clamp to 5..=90, `log_once!` warn |

## 7. Logging

`dmw::freelook`: `debug!` on FOV table rebuild (hfov, ms); `trace!` per frame `pitch_deg`, `shear_px`, `hfov`.

## 8. Licensing and asset notes

Y-shearing is a well-known technique (Heretic/Hexen, GPL). Write it fresh against room's code; do not copy from other GPL-2.0-only ports without checking licence compatibility (Heretic/Hexen source is GPL-2.0-or-later, so copying is permitted with attribution if useful — record it in provenance).

## 9. Tests

| Id | Test |
|---|---|
| T6.1 | unit: `shear_px` sign and magnitude: pitch −32° at full width → +100 ± 1; pitch +32° → −100 ± 1; pitch 0 → 0 |
| T6.2 | unit: `fine_fov(90) == 2048`; `projection` at 90° equals vanilla `centerxfrac` exactly |
| T6.3 | headless golden: Freedoom MAP01/E1M1 start, MW2 mode with fixture pack, pitch forced to −20°, 0°, +20° via `PLAYER_EXT`; capture `I_VideoBuffer`; compare to `room/tests/golden/freelook_{up,level,down}.png` (palette-expanded). BLESS to regenerate; review by eye on first bless |
| T6.4 | headless: pitch 0, no zoom, MW2 mode on → framebuffer **byte-identical** to the same tic with `-nomw2` (shear code is a no-op at 0) |
| T6.5 | headless: zoom 45° golden; and light parity: a wall's palette index distribution at 90° vs 45° zoom is within tolerance (light correction works) |
| T6.6 | headless: up-pitch max, sky rows contain no wrap seam (no row where the sky column jumps from texture row 0 to 127 between adjacent screen rows) |
| T6.7 | `demo_playthrough` unchanged |

## 10. Acceptance criteria

- [ ] Looking up/down with the mouse pitches the world smoothly; vanilla fist/chainsaw sprites stay put.
- [ ] At pitch 0 and no zoom, MW2-mode frames equal vanilla frames (T6.4).
- [ ] ADS zoom narrows the FOV smoothly without lighting pops (T6.5).
- [ ] No sky seams at the pitch limit (T6.6).
- [ ] Window resizes with a correct 4:3 letterbox; `aspect_correct = false` stretches.
- [ ] T6.1–T6.7 pass.

## 11. Contract changes requested

None.
