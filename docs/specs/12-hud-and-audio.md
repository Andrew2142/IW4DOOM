# 12 — HUD, Crosshair, Scope Overlays and Audio

Owner: build agent (wave 5)
Status: implementation-ready
Depends on: 00 (§2 D17; §7.2 `ViewState`, `SoundCue`; §7.5 rows `D_Display`, `ST_Drawer`; §9.4), 01, 07, 08, 09 (`hud::on_hit`), 10 (notetrack cues), 11 (`overlay.rs` blitters, palette)

Everything the player reads or hears about their MW2 gun: a spread-driven MW2 hip crosshair that follows the gun, hitmarkers, magazine count in Doom's status bar and a fullscreen ammo counter, a "RELOAD" prompt, weapon names on switch, sniper scope overlays with world sway, and MW2 weapon sounds through room's rodio mixer.

---

## 1. Purpose

Give MW2 feedback inside Doom's visual language: 1-pixel palette lines, Doom digits and fonts, Doom's message line — no modern UI.

## 2. Scope / Out of scope

In scope: `room/src/dmw/hud.rs`, `room/src/dmw/audio.rs`, `room/src/audio/sfx.rs` additions (a zero-copy MW2 source and a separate channel pool), call sites in `D_Display` (after `R_RenderPlayerView`) and the status bar ammo widget, weapon pickup/switch messages.

Out of scope: MW2 HUD art (no MW2 HUD materials are exported except the scope overlay), killfeed, minimap/compass, damage direction indicators, MW2 music.

## 3. Dependencies

| From | What |
|---|---|
| 07 `ViewState` | `spread_deg`, `ads_frac`, `overlay_active`, `zoom_hfov_deg`, `clip`, `stock`, `weapon`, `weaponstate`, `placement` (for crosshair drift via `gun_angle_offset`) |
| 03 `HudFacts` | `ads_crosshair_in_frac`, `hip_reticle_side_pos`, `i_reticle_min_ofs`, `i_reticle_side_size`, `clip_size` |
| 03 `WeaponFacts::display_name`, `sounds` | names, aliases |
| 09 | `on_hit(HitFeedback { headshot, killed })` |
| 10 | notetrack `SoundCue`s (through 08's driver) |
| 11 | `PaletteTables::nearest`, `overlay::{blit_indexed, blit_doom_patch}`, overlay `IndexedTexture` in the registry |
| room facts | `D_Display` order: `ST_Drawer` → `R_RenderPlayerView` → `HU_Drawer` → border → pause → `M_Drawer` (so anything drawn right after `R_RenderPlayerView` is under HUD text and menus); status bar `w_ready.num` points at the ready weapon's ammo pool (st_stuff.rs:956); `tallnum`/`shortnum` digit patches (STTNUM*, STYSNUM*); `players[].message` shown by `HU_Drawer`; `S_CLIPPING_DIST = 1200`, `S_CLOSE_DIST = 200`, `S_STEREO_SWING = 96`, `NORM_SEP = 128`, `sfxVolume` 0–15; `AudioState` (room/src/audio/mod.rs) with 8 Doom channels, `PannedSource`, `gains_from(vol, sep)` |

## 4. File inventory

| Path | Content |
|---|---|
| `room/src/dmw/hud.rs` | `draw_view_overlays()`, `status_ammo()`, `on_hit()`, `reset()`, messages |
| `room/src/dmw/audio.rs` | `play(cue, origin)`, variant choice, positional params |
| `room/src/audio/mod.rs`, `sfx.rs` | `ArcSamples` source, `start_dmw_sound`, separate `dmw_channels` pool |
| `room/src/doom/d_main.rs` | `D_Display`: `crate::dmw::hooks::draw_view_overlays();` right after `R_RenderPlayerView(...)` |
| `room/src/doom/st_stuff.rs` | ammo widget reads `hooks::status_ammo(plyr)` when `Some` |
| `room/tests/hud_headless.rs` | §9 |

## 5. Detailed design

### 5.1 Crosshair

Shown when MW2 weapon held, alive, `!overlay_active`, not sprinting, `ads_frac < hud.ads_crosshair_in_frac` (MW2 hides the hip crosshair as ADS comes up), and `[hud] crosshair`.

```text
centre   = (viewwindowx + scaledviewwidth/2 + dx, viewwindowy + viewheight/2 + dy)      (unsheared centre)
           dx = −tan(gun_yaw_offset) · fx_world, dy = tan(gun_pitch_offset) · fx_world · pixel_aspect
           fx_world = (scaledviewwidth/2) / tan(world_hfov/2)          (world FOV incl. zoom)
           gun_*_offset = ViewState::gun_angle_offset (the crosshair drifts with sway and gun recoil, like MW2's calc_crosshair_position)
gap      = max(i_reticle_min_ofs · s, tan(spread_deg) · fx_world)                         s = scaledviewwidth / 640 (MW2 reticle units are 640-wide virtual pixels)
length   = max(2, i_reticle_side_size · s)
lines    = four 1-pixel lines (up, down, left, right) from gap to gap+length, colour = nearest(240,240,240)
```

Clipped to the view window. Drawn with direct writes to `I_VideoBuffer` (no colormap — HUD is fullbright, like Doom's HUD).

### 5.2 Hitmarker

`on_hit` sets `hit_tics = 6` (≈ 0.17 s; `killed` → 9). While `hit_tics > 0`, draw four 3-pixel diagonal lines at offsets ±(3..5) from the crosshair centre, colour `nearest(255,255,255)` (`headshot` → `nearest(255,64,64)`). Play `HITMARKER_ALIAS = "mp_hit_alert"` once per tic with hits (local) if the pack has it; else nothing. `[hud] hitmarker = false` disables both.

### 5.3 Ammo display

- Status bar (`setblocks ≤ 10`): `status_ammo(plyr) -> Option<c_int>` returns the magazine count of the held MW2 weapon; `ST_updateWidgets`/`ST_createWidgets` point `w_ready.num` at a DOOMMW2-owned `static mut DMW_READY_AMMO: c_int` updated every tic instead of `plyr.ammo[...]` while it is `Some`. The right-hand ammo table (pools/maxima) is unchanged, so "magazine | reserve" is visible at once. Vanilla weapons use the vanilla path.
- Fullscreen (`setblocks == 11`): bottom-right of the view, draw the magazine with `tallnum` digits and the pool with `shortnum` digits below/right, Doom-style, via `V_DrawPatch` (so it is drawn into the frame like the status bar numbers).
- Reload prompt: when `clip ≤ max(1, clip_size / 4)` and pool > 0 and not reloading, draw "RELOAD" with Doom's HU font (STCFN) centred 12 px below the crosshair, blinking at 2 Hz (tic-based). When pool == 0 and clip == 0: "NO AMMO".

### 5.4 Scope overlay

When `ViewState::overlay_active` (07 §5.11): 11 skips the gun; this sheet fills the view window: overlay texture (quantized at level start, 11 §6.1) scaled uniformly to the view height and centred; opaque texels drawn, transparent texels show the world; columns left and right of the scaled overlay are filled with `nearest(0,0,0)`. No pack overlay → procedural fallback: black outside a circle of radius `0.48 · viewheight`, 1-pixel crosshair lines through the centre.

World sway under a fixed reticle: MW2 moves the camera with the gun axis while scoped. 08 writes `PLAYER_EXT.view_kick_deg[0..2] += gun_angle_offset` while `overlay_active` (requested in §11), so 06 sways the world; bullets already follow the gun axis (07 §5.4), which now projects to the reticle centre.

### 5.5 Messages

- Weapon switch: on `SimEvent::RaiseStarted(id)` (not on level start), `players[consoleplayer].message = display_name` (static `CString` cache per weapon) when `[hud] weapon_names` (new key, default `true`, see §11).
- Pickup: when `sync_ownership` (08 §5.4) adds a weapon through a pickup, replace Doom's message with `"You got the {display_name}!"`. 08 exposes `on_weapon_acquired(id)`; 12 sets the message.

### 5.6 Audio

`audio::play(cue: &SoundCue, origin: Option<*mut mobj_t>)`:

1. `entry = pack.manifest.sounds[cue.alias]` — missing → `log_once!` warn, return.
2. Variant: uniform pick with an audio-only xorshift seeded at startup (audio randomness never touches `DmwRng`, 01 §5.6).
3. `asset = pack.load_sound(&variant.file)` (cached `Arc`).
4. Volume: `variant.volume × [audio] mw2_volume × sfxVolume / 15`. Local cues (`cue.local`): centre pan (`sep = NORM_SEP`). Positional: Doom's attenuation/separation formulas re-implemented from `S_AdjustSoundParams` (approx distance, `S_CLOSE_DIST`/`S_CLIPPING_DIST`, `S_STEREO_SWING`) against `players[consoleplayer].mo`; inaudible → skip.
5. Pitch: `variant.pitch` via `Source::speed`.
6. Channel: dedicated pool of `[audio] channels` (default 8) separate from Doom's 8, so MW2 gunfire never cuts monster sounds. Steal the oldest when full. Firing sounds on the same weapon cut the previous fire sound's tail only after 20 ms (iw4L keeps the first 20 ms of weapon attacks on replacement — AUDIO.md).
7. Playback source: `ArcSamples { data: Arc<[f32]>, channels, rate, pos }` implementing `rodio::Source` (zero-copy; mono or stereo; stereo is panned by scaling L/R gains like `PannedSource`).

No audio device (headless) → `AUDIO` is `None` → every call is a no-op.

### 5.7 Reset

`hud::reset()` at level start clears `hit_tics`, blink phase, and `DMW_READY_AMMO`.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Missing STCFN/STTNUM lumps (odd PWAD) | text/digits skipped, `log_once!` warn |
| Overlay texture failed | procedural fallback |
| Alias without variants | skipped (03 guarantees ≥ 1 variant when present) |
| `sfxVolume == 0` | MW2 sounds muted too |

## 7. Logging

`dmw::hud`: `debug!` overlay on/off; `trace!` crosshair gap per frame. `dmw::audio`: `debug!` per played cue (alias, variant, volume, sep), `warn!` once per missing alias.

## 8. Licensing and asset notes

All HUD art is Doom's own lumps or procedural lines; MW2 content limited to the user-exported scope overlay and sounds in the gitignored pack.

## 9. Tests

| Id | Test |
|---|---|
| T12.1 | crosshair geometry: `spread_deg` 0 → gap = min offset; 3° at 90° FOV → `tan(3°)·160 ≈ 8.4` px; hidden at `ads_frac ≥ in_frac`, while sprinting, and when overlay active |
| T12.2 | headless golden: crosshair at rest and after 10 hip-fired rifle shots (bloomed) — `room/tests/golden/hud_crosshair_{rest,bloom}.png` (view-window crop) |
| T12.3 | hitmarker appears for 6 tics after a hit (with T9.4's setup) and 9 after a kill |
| T12.4 | status bar AMMO shows the magazine (fixture pistol: 12 → 11 after a shot) while the pools table is unchanged; vanilla fist shows vanilla |
| T12.5 | scope: `fixture_sniper` fully ADS → gun not drawn, overlay drawn, centre pixel shows the world, columns outside the overlay black |
| T12.6 | audio unit: `ArcSamples` yields the same samples as the source buffer; positional params equal Doom's formula for 5 distances/angles |
| T12.7 | messages: switching to the fixture rifle sets `message` to its display name; pickup message replaced |

## 10. Acceptance criteria

- [ ] In `task run-fixture`: crosshair blooms when firing and walking, shrinks when standing, hides when aiming/sprinting; hitmarkers on hits; AMMO shows the magazine; "RELOAD" prompt; sniper scope overlay with swaying world; MW2 fixture sounds play for fire, reload notetracks and empty clicks without cutting Doom sounds.
- [ ] Menus and HUD messages draw over the crosshair.
- [ ] T12.1–T12.7 pass.

## 11. Contract changes requested

1. 08 §5.5: while `view.overlay_active`, add `gun_angle_offset` (pitch, yaw) to `PLAYER_EXT.view_kick_deg` (scope sway moves the world). — Applied in sheet 08 by the lead.
2. 02 §5.7: `HudCfg` gains `weapon_names: bool` (default `true`). — Applied in sheet 02 by the lead.
3. 04 §5.8: the exporter always includes the alias `mp_hit_alert` (hitmarker) in the pack when it exists in the loaded sound catalog. — Applied in sheet 04 by the lead.
4. 08 exposes `on_weapon_acquired(id)` to 12 (§5.5). — Applied in sheet 08 by the lead.

### Build notes (sheet 12 implemented)

No change to a 00 contract is needed. Requests and deviations for the lead:

5. **Scope overlay registry (sheet 11).** Sheet 11's engine half is not in this tree, so `hud.rs` quantizes `Pack::load_overlay` itself at level start (`prepare_overlays`, once per session, `[render] dither`, `AlphaMode::Test { cutoff: 0.5 }`) and keeps the textures in its own map. When sheet 11's registry lands, `draw_view_overlays` should read the overlay from it and `prepare_overlays` should go.
6. **Crosshair vertical drift has no pixel-aspect factor.** §5.1 wrote `dy = tan(pitch) * fx * pixel_aspect`. Doom renders the world with square pixels (the 4:3 stretch happens at presentation, sheet 06 shears with the same focal length), so a gun pitched `p` lands `tan(p) * fx` rows from the centre. The code uses that; the horizontal formula is as written. The scope overlay does use the pixel aspect (`scope_rect`, and the procedural fallback is an ellipse) so a round scope is round on screen.
7. **Prompt position.** "12 px below the crosshair" is `prompt_top`: `cy + max(12, gap + length + 4)`, so a bloomed crosshair never overlaps the text. The prompt is hidden while reloading and while the scope is up.
8. **Audio API.** `audio::play(cue, origin) -> bool` is `pub(crate)` and returns `true` when the pack knows the alias (so sheet 09's melee hit can fall back to `sfx_punch` otherwise); `audio::play_local`, `has_alias`, `adjust_params`, `mix_params` are public for tests. `Pack::load_sound` takes an alias, so variant choice uses `Pack::load_sound_variant(alias, index)`. Sheet 09's melee hit now plays the weapon's `melee_hit` alias.
9. **Mixer.** `AudioState` gains a separate pool (`[audio] channels`, sized on first use) and `ArcSamples` (zero-copy over `Arc<SoundAsset>`), `StereoGain` (pan for stereo files) and a shared `SampleCtl`. A replay of the same alias shortens the older sound to its first 20 ms by ending its source, with no timer. Sounds with more than two channels are refused.
10. **Hook wiring.** `D_Display` calls `hooks::draw_view_overlays()` after `R_RenderPlayerView`; `ST_updateWidgets` points `w_ready.num` at `hud::DMW_READY_AMMO` when `hooks::status_ammo` is `Some` (`ST_createWidgets` is untouched, `ST_updateWidgets` runs before the first draw). In sheet 08's files: `Action::Raised(id)` (weapon-name message, not on the first tic of a level) and `hud::on_tic()` in the action runner.
11. **Test hooks.** `hud::last_message()` (Doom clears `players[].message` within the tic, so tests read the last text set), `hud::ready_ammo()`, `hud::hitmarker_active()` and, with `test-hooks`, `hud::test_set_overlays_enabled(bool)` to render the same state with and without the HUD.
12. **Not verified.** "Gun not drawn while scoped" (T12.5) and the interactive acceptance run need sheet 11's drawing and a human. Audible playback needs an audio device; only the parameter maths and the sources are tested. `mp_hit_alert` is not in the fixture pack, so the hit sound is untested.
