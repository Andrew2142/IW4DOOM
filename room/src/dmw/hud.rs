// SPDX-License-Identifier: GPL-3.0-or-later
//! HUD glue (sheet 12): spread crosshair, hitmarker, magazine counter, reload prompt, scope
//! overlay and the weapon messages.
//!
//! Everything is drawn in Doom's visual language: 1-pixel palette lines written straight into
//! `I_VideoBuffer`, Doom's own digit and font patches, and Doom's message line. The geometry
//! (crosshair layout, scope rectangle, prompt rules) is pure and unit-tested; the engine-facing
//! functions only gather inputs and call it.
//!
//! Call sites: `D_Display` calls [`draw_view_overlays`] right after `R_RenderPlayerView`
//! (under `HU_Drawer` text and the menus), `ST_updateWidgets` asks [`status_ammo`] for the
//! AMMO widget, sheet 08's driver calls [`reset`], [`on_tic`], [`on_weapon_raised`] and
//! [`on_weapon_acquired`], and sheet 09's ballistics calls [`on_hit`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{c_int, CString};
use std::sync::Arc;

use dmw_format::consts::{DOOM_HFOV, PIXEL_ASPECT};
use dmw_format::facts::HudFacts;
use dmw_format::{AlphaMode, DmwWeaponId};
use dmw_viewmodel::overlay::{blit_indexed, BlitRect};
use dmw_viewmodel::{quantize, Dither, DrawTarget, IndexedTexture, PaletteTables};
use dmw_weapons::{SoundCue, ViewState};
use log::{debug, trace};
use weapon_iw4::WeaponState;

use super::config::DitherCfg;
use super::{mode, with_state};
use crate::doom::d_player::PlayerT;

const TARGET: &str = "dmw::hud";

/// Tics the hitmarker stays up after a hit (about 0.17 s).
pub const HIT_TICS: u32 = 6;
/// Tics the hitmarker stays up after a kill.
pub const KILL_TICS: u32 = 9;
/// Screen width of Doom's frame buffer.
const SCREEN_W: usize = 320;
/// Screen height of Doom's frame buffer.
const SCREEN_H: usize = 200;
/// Blink half period of the reload prompt: 2 Hz at 35 tics per second is 17.5 tics per cycle.
const BLINK_HALF_TICS: u32 = 9;

/// What a damaging hit looked like (sheet 09 section 5.5, sheet 12 section 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HitFeedback {
    /// The pellet hit a head or helmet location.
    pub headshot: bool,
    /// The pellet took the target from alive to dead.
    pub killed: bool,
}

/// The magazine count shown in the status bar AMMO widget (`w_ready.num` points here while
/// [`status_ammo`] returns `Some`).
#[allow(non_upper_case_globals)]
pub static mut DMW_READY_AMMO: c_int = 0;

// ---------------------------------------------------------------------------
// Pure geometry
// ---------------------------------------------------------------------------

/// The view window on the 320x200 screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRect {
    /// `viewwindowx`.
    pub x: i32,
    /// `viewwindowy`.
    pub y: i32,
    /// `scaledviewwidth`.
    pub w: i32,
    /// `viewheight`.
    pub h: i32,
}

/// Where and how large the four crosshair arms are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairLayout {
    /// Centre column.
    pub cx: i32,
    /// Centre row.
    pub cy: i32,
    /// Distance from the centre to the first arm pixel, unrounded.
    pub gap_f: f32,
    /// Distance from the centre to the first arm pixel.
    pub gap: i32,
    /// Arm length in pixels.
    pub length: i32,
}

/// Horizontal focal length in pixels of a view `view_w` wide at `world_hfov_deg`.
pub fn focal_px(view_w: i32, world_hfov_deg: f32) -> f32 {
    (view_w as f32 * 0.5) / (world_hfov_deg.clamp(1.0, 179.0).to_radians() * 0.5).tan()
}

/// Section 5.1: the crosshair centre drifts with the gun axis (`gun_angle_offset`, pitch and
/// yaw in degrees, pitch positive down), the gap follows the spread and the MW2 reticle
/// parameters (640-wide virtual pixels).
///
/// The vertical drift uses the same focal length as the horizontal one: Doom renders the world
/// with square pixels (the 4:3 stretch happens at presentation), so a pitch of `p` degrees
/// lands `tan(p) * focal` rows from the centre.
pub fn crosshair_layout(
    view: ViewRect,
    world_hfov_deg: f32,
    spread_deg: f32,
    gun_angle_offset: [f32; 2],
    hud: &HudFacts,
) -> CrosshairLayout {
    let fx = focal_px(view.w, world_hfov_deg);
    let dx = -gun_angle_offset[1].to_radians().tan() * fx;
    let dy = gun_angle_offset[0].to_radians().tan() * fx;
    let s = view.w as f32 / 640.0;
    let gap_f = (hud.i_reticle_min_ofs as f32 * s).max(spread_deg.to_radians().tan() * fx);
    let length = ((hud.i_reticle_side_size as f32 * s).round() as i32).max(2);
    CrosshairLayout {
        cx: view.x + view.w / 2 + dx.round() as i32,
        cy: view.y + view.h / 2 + dy.round() as i32,
        gap_f,
        gap: gap_f.round() as i32,
        length,
    }
}

/// Section 5.1: shown with a weapon in hand, alive, no scope overlay, not sprinting, and while
/// the ADS fraction is below the weapon's `ads_crosshair_in_frac`.
pub fn crosshair_visible(
    enabled: bool,
    alive: bool,
    overlay_active: bool,
    sprinting: bool,
    ads_frac: f32,
    hud: &HudFacts,
) -> bool {
    enabled && alive && !overlay_active && !sprinting && ads_frac < hud.ads_crosshair_in_frac
}

/// `true` while the weapon is reloading (any reload state).
pub fn is_reloading(weaponstate: i32) -> bool {
    (WeaponState::Reloading as i32..=WeaponState::ReloadEnd as i32).contains(&weaponstate)
}

/// The text of the low-ammo prompt, if one is due (section 5.3): "Press R to reload" (`R` is
/// `reload_key`, the label of the bound key) when the magazine is at most a quarter full (at
/// least 1) and the pool has rounds, "NO AMMO" when both are empty.
pub fn ammo_prompt(
    clip: i32,
    stock: i32,
    clip_size: i32,
    reloading: bool,
    reload_key: &str,
) -> Option<String> {
    if reloading {
        return None;
    }
    if clip <= 0 && stock <= 0 {
        return Some("NO AMMO".to_owned());
    }
    // Low means a quarter of the magazine or less, but never a full magazine: a one-round
    // weapon (the RPG) only asks for a reload once it is empty.
    let low = clip < clip_size && clip <= (clip_size / 4).max(1);
    (low && stock > 0).then(|| format!("Press {reload_key} to reload"))
}

/// Rows between the reload prompt and the door prompt below it.
const PROMPT_LINE_H: i32 = 10;

/// Top row of the door prompt: the reload slot when no reload prompt is due, else the row
/// under it (the slot stays reserved through the reload prompt's blink so the text below
/// does not jump).
pub fn use_prompt_top(reload_top: i32, reload_due: bool) -> i32 {
    if reload_due {
        reload_top + PROMPT_LINE_H
    } else {
        reload_top
    }
}

/// Top row of the prompt text: 12 rows below the centre, or four rows under the lower arm when
/// the crosshair has bloomed past that, so the text never overlaps the reticle.
pub fn prompt_top(l: &CrosshairLayout) -> i32 {
    l.cy + (l.gap + l.length + 4).max(12)
}

/// Blink phase of the prompt: on for the first half period of every cycle.
pub fn blink_on(tic: u32) -> bool {
    (tic / BLINK_HALF_TICS).is_multiple_of(2)
}

/// The rectangle the scope overlay texture covers: as tall as the view window, as wide as its
/// aspect asks (`aspect_correct` widens it by the pixel aspect so a round scope looks round),
/// centred horizontally. May be wider than the view (it is clipped when drawn).
pub fn scope_rect(view: ViewRect, tex_w: u32, tex_h: u32, aspect_correct: bool) -> BlitRect {
    let pixel = if aspect_correct { PIXEL_ASPECT } else { 1.0 };
    let ratio = if tex_h == 0 {
        1.0
    } else {
        tex_w as f32 / tex_h as f32
    };
    let w = ((view.h as f32 * ratio) / pixel).round() as i32;
    BlitRect {
        x: view.x + (view.w - w) / 2,
        y: view.y,
        w,
        h: view.h,
    }
}

/// Palette indices the HUD draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudColours {
    /// Crosshair arms.
    pub cross: u8,
    /// Hitmarker.
    pub hit: u8,
    /// Headshot hitmarker.
    pub headshot: u8,
    /// Scope mask.
    pub black: u8,
}

impl HudColours {
    /// Nearest palette entries for the HUD colours of sections 5.1, 5.2 and 5.4.
    pub fn new(pal: &PaletteTables) -> Self {
        Self {
            cross: pal.nearest([240, 240, 240]),
            hit: pal.nearest([255, 255, 255]),
            headshot: pal.nearest([255, 64, 64]),
            black: pal.nearest([0, 0, 0]),
        }
    }
}

// ---------------------------------------------------------------------------
// Pixel drawing (all clipped to the view window)
// ---------------------------------------------------------------------------

/// A clipped pixel writer over a 320x200 buffer.
struct Canvas<'a> {
    pixels: &'a mut [u8],
    pitch: usize,
    clip: ViewRect,
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, colour: u8) {
        let c = self.clip;
        if x < c.x || y < c.y || x >= c.x + c.w || y >= c.y + c.h {
            return;
        }
        if let Some(p) = self.pixels.get_mut(y as usize * self.pitch + x as usize) {
            *p = colour;
        }
    }
}

/// The four crosshair arms.
fn draw_crosshair(cv: &mut Canvas<'_>, l: &CrosshairLayout, colour: u8) {
    for k in l.gap..l.gap + l.length {
        cv.put(l.cx, l.cy - k, colour);
        cv.put(l.cx, l.cy + k, colour);
        cv.put(l.cx - k, l.cy, colour);
        cv.put(l.cx + k, l.cy, colour);
    }
}

/// Four 3-pixel diagonals at offsets 3..=5 from the centre.
fn draw_hitmarker(cv: &mut Canvas<'_>, cx: i32, cy: i32, colour: u8) {
    for d in 3..=5 {
        for (sx, sy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            cv.put(cx + sx * d, cy + sy * d, colour);
        }
    }
}

/// Section 5.4: the scope overlay. With a texture, it is drawn over `scope_rect` and the view
/// columns outside it are black; without one, everything outside a circle of radius
/// `0.48 * viewheight` is black and thin crosshair lines cross the centre.
pub fn draw_scope_overlay(
    pixels: &mut [u8],
    view: ViewRect,
    tex: Option<&IndexedTexture>,
    black: u8,
    aspect_correct: bool,
) {
    let identity: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut cv = Canvas {
        pixels,
        pitch: SCREEN_W,
        clip: view,
    };
    let (cx, cy) = (view.x + view.w / 2, view.y + view.h / 2);
    match tex.and_then(|t| t.mips.first()) {
        Some(mip) => {
            let rect = scope_rect(view, mip.width, mip.height, aspect_correct);
            for y in view.y..view.y + view.h {
                for x in view.x..view.x + view.w {
                    if x < rect.x || x >= rect.x + rect.w {
                        cv.put(x, y, black);
                    }
                }
            }
            let mut target = DrawTarget {
                pixels: cv.pixels,
                pitch: SCREEN_W,
                view_x: view.x,
                view_y: view.y,
                view_w: view.w,
                view_h: view.h,
                center_x: view.w as f32 * 0.5,
                center_y: view.h as f32 * 0.5,
                pixel_aspect: PIXEL_ASPECT,
            };
            blit_indexed(&mut target, mip, rect, &identity);
        }
        None => {
            let pixel = if aspect_correct { PIXEL_ASPECT } else { 1.0 };
            let ry = 0.48 * view.h as f32;
            let rx = ry / pixel;
            for y in view.y..view.y + view.h {
                for x in view.x..view.x + view.w {
                    let (nx, ny) = ((x - cx) as f32 / rx, (y - cy) as f32 / ry);
                    if nx * nx + ny * ny > 1.0 || x == cx || y == cy {
                        cv.put(x, y, black);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Doom digit and font patches, looked up by lump name once.
struct Patches {
    tall: [*mut crate::doom::v_video::patch_t; 10],
    short: [*mut crate::doom::v_video::patch_t; 10],
    font: [*mut crate::doom::v_video::patch_t; 64],
}

#[derive(Default)]
struct HudState {
    /// HUD tic counter (one per MW2 tic).
    tic: u32,
    /// The hitmarker is visible while `tic <= hit_until`.
    hit_until: u32,
    /// The last hit was a headshot.
    hit_headshot: bool,
    /// Last tic that played the hit alert (one per tic).
    hit_sound_tic: Option<u32>,
    /// Quantized scope overlays by `DmwWeaponId.0`.
    overlays: BTreeMap<u16, IndexedTexture>,
    /// Message texts handed to Doom; never removed, so the pointers stay valid.
    messages: BTreeMap<String, CString>,
    /// The text of the last message this module set (cleared at level start).
    last_message: String,
    /// Test knob: skip all drawing (`test-hooks` builds only).
    overlays_off: bool,
    /// Digit and font patches, `None` until looked up; `Some(None)` when a lump is missing.
    patches: Option<Option<Patches>>,
    /// Where this frame's reload prompt sits (set by [`draw_view_overlays`]).
    slot: Option<PromptSlot>,
    /// The door/switch prompt drawn in the last frame, if any.
    use_prompt: Option<String>,
    /// The reload / no-ammo prompt due in the last frame (drawn only in the blink's on phase).
    reload_prompt: Option<String>,
}

/// Layout facts the door prompt needs from the reload prompt's frame.
#[derive(Clone, Copy)]
struct PromptSlot {
    cx: i32,
    top: i32,
    reload_due: bool,
    scoped: bool,
}

thread_local! {
    static HUD: RefCell<HudState> = RefCell::new(HudState::default());
}

/// Level start: clear hit markers, the blink phase and the ready-ammo override, and quantize
/// the scope overlays of the mapped weapons (once per session).
pub(crate) fn reset() {
    HUD.with(|h| {
        let mut h = h.borrow_mut();
        h.tic = 0;
        h.hit_until = 0;
        h.hit_headshot = false;
        h.hit_sound_tic = None;
        h.last_message.clear();
    });
    // SAFETY: single engine thread; a plain integer store.
    unsafe { DMW_READY_AMMO = 0 };
    prepare_overlays();
}

/// Quantizes `pack.load_overlay` of every mapped weapon that has one (sheet 11's registry does
/// not exist in this tree; sheet 12 section 5.4 asks for the same `dmw_viewmodel::quantize`).
fn prepare_overlays() {
    let Some((pack, pal, dither, ids)) = with_state(|s| {
        Some((
            s.pack.clone()?,
            s.palette.clone()?,
            s.config.render.dither,
            s.mapping.ids(),
        ))
    })
    .flatten() else {
        return;
    };
    let dither = match dither {
        DitherCfg::None => Dither::None,
        DitherCfg::Bayer4 => Dither::Bayer4,
    };
    let wanted: Vec<DmwWeaponId> = HUD.with(|h| {
        let h = h.borrow();
        ids.into_iter()
            .filter(|id| !h.overlays.contains_key(&id.0))
            .filter(|id| pack.try_facts(*id).is_some_and(|f| f.has_overlay))
            .collect()
    });
    for id in wanted {
        match pack.load_overlay(id) {
            Ok(Some(img)) => {
                let tex = quantize(&img, &pal, dither, AlphaMode::Test { cutoff: 0.5 });
                debug!(target: TARGET, "scope overlay of weapon {} quantized", id.0);
                HUD.with(|h| h.borrow_mut().overlays.insert(id.0, tex));
            }
            Ok(None) => {}
            Err(e) => {
                log_once!(
                    TARGET,
                    log::Level::Warn,
                    &format!("overlay:{}", id.0),
                    "scope overlay of weapon {} not loaded ({e}); using the procedural one",
                    id.0
                );
            }
        }
    }
}

/// One MW2 tic passed (sheet 08's driver, with the viewmodel tic): advances the HUD clock.
pub(crate) fn on_tic() {
    HUD.with(|h| h.borrow_mut().tic += 1);
}

/// A bullet or knife hit a monster: starts the hitmarker and plays the hit alert (section 5.2).
pub fn on_hit(feedback: HitFeedback) {
    trace!(target: TARGET, "hit feedback {feedback:?}");
    if !with_state(|s| s.config.hud.hitmarker).unwrap_or(false) {
        return;
    }
    let play = HUD.with(|h| {
        let mut h = h.borrow_mut();
        let len = if feedback.killed { KILL_TICS } else { HIT_TICS };
        h.hit_until = h.hit_until.max(h.tic + len);
        h.hit_headshot = feedback.headshot;
        let first = h.hit_sound_tic != Some(h.tic);
        h.hit_sound_tic = Some(h.tic);
        first
    });
    if play && super::audio::has_alias(super::audio::HITMARKER_ALIAS) {
        let cue = SoundCue {
            alias: super::audio::HITMARKER_ALIAS.to_owned(),
            local: true,
        };
        super::audio::play(&cue, std::ptr::null_mut());
    }
}

/// The text of the last message this module put on the message line since level start (Doom
/// clears the line itself within the tic, so this is what tests and diagnostics read).
pub fn last_message() -> String {
    HUD.with(|h| h.borrow().last_message.clone())
}

/// Test knob: `false` makes [`draw_view_overlays`] draw nothing, so a test can render the same
/// game state with and without the HUD. Available with the `test-hooks` feature only.
#[cfg(feature = "test-hooks")]
pub fn test_set_overlays_enabled(on: bool) {
    HUD.with(|h| h.borrow_mut().overlays_off = !on);
}

/// `true` while the hitmarker is up (for tests and the drawing code).
pub fn hitmarker_active() -> bool {
    HUD.with(|h| {
        let h = h.borrow();
        h.tic <= h.hit_until && h.hit_until > 0
    })
}

/// Puts `text` on the console player's message line (Doom's `players[].message`).
fn set_message(text: &str) {
    let ptr = HUD.with(|h| {
        let mut h = h.borrow_mut();
        let Ok(c) = CString::new(text) else {
            return None;
        };
        h.last_message = text.to_owned();
        Some(h.messages.entry(text.to_owned()).or_insert(c).as_ptr())
    });
    let Some(ptr) = ptr else {
        return;
    };
    // SAFETY: engine globals on the engine thread; the string lives in `HUD.messages` for the
    // rest of the process, and Doom only reads `message` and nulls it.
    unsafe {
        let cp = crate::doom::g_game::consoleplayer;
        if let Some(i) = usize::try_from(cp)
            .ok()
            .filter(|&i| i < crate::doom::d_player::MAXPLAYERS)
        {
            let p = (&raw mut crate::doom::g_game::players)
                .cast::<PlayerT>()
                .add(i);
            (*p).message = ptr.cast_mut();
        }
    }
}

/// The display name of `id`, when the pack knows it.
fn display_name(id: DmwWeaponId) -> Option<String> {
    with_state(|s| {
        s.pack
            .as_ref()
            .and_then(|p| p.try_facts(id).map(|f| f.display_name.clone()))
    })
    .flatten()
}

/// The sim started raising `id` (not at level start): shows its name (section 5.5).
pub(crate) fn on_weapon_raised(id: DmwWeaponId) {
    if !with_state(|s| s.config.hud.weapon_names).unwrap_or(false) {
        return;
    }
    if let Some(name) = display_name(id) {
        set_message(&name);
    }
}

/// A pickup (or cheat) added `id` to the MW2 inventory: replaces Doom's message (section 5.5).
pub(crate) fn on_weapon_acquired(id: DmwWeaponId) {
    if let Some(name) = display_name(id) {
        set_message(&format!("You got the {name}!"));
    }
}

// ---------------------------------------------------------------------------
// Engine-facing hooks
// ---------------------------------------------------------------------------

/// What `D_Display` needs this frame, gathered under one state borrow.
struct Frame {
    view: ViewState,
    hud: HudFacts,
    pal: Arc<PaletteTables>,
    sprinting: bool,
    crosshair_enabled: bool,
    hitmarker_enabled: bool,
    ammo_counter: bool,
    aspect_correct: bool,
    /// The held MW2 weapon is animated by the sim.
    holding: bool,
}

fn gather() -> Option<Frame> {
    with_state(|s| {
        let sim = s.sim.as_ref()?;
        let pal = s.palette.clone()?;
        let view = sim.view_state();
        let facts = s.pack.as_ref()?.try_facts(view.weapon?)?;
        Some(Frame {
            hud: facts.hud.clone(),
            sprinting: s.last_out.as_ref().is_some_and(|o| o.sprinting),
            crosshair_enabled: s.config.hud.crosshair,
            hitmarker_enabled: s.config.hud.hitmarker,
            ammo_counter: s.config.hud.ammo_counter,
            aspect_correct: s.config.render.aspect_correct,
            holding: s.driver.owner == super::weapons::Owner::Mw2,
            view,
            pal,
        })
    })
    .flatten()
}

/// The console player and whether they are alive.
fn console_alive() -> bool {
    // SAFETY: engine globals on the engine thread.
    unsafe {
        let cp = crate::doom::g_game::consoleplayer;
        usize::try_from(cp)
            .ok()
            .filter(|&i| i < crate::doom::d_player::MAXPLAYERS)
            .is_some_and(|i| {
                let p = (&raw const crate::doom::g_game::players)
                    .cast::<PlayerT>()
                    .add(i);
                (*p).health > 0 && !(*p).mo.is_null()
            })
    }
}

/// `D_Display`, right after `R_RenderPlayerView`: scope overlay, crosshair, hitmarker, ammo
/// counter and prompt, in that order. Draws nothing unless an MW2 weapon is in hand.
pub(crate) fn draw_view_overlays() {
    HUD.with(|h| {
        let mut h = h.borrow_mut();
        h.slot = None;
        h.reload_prompt = None;
    });
    if !mode::weapons_active() || HUD.with(|h| h.borrow().overlays_off) {
        return;
    }
    let Some(frame) = gather() else {
        return;
    };
    if !frame.holding {
        return;
    }
    // SAFETY: renderer globals and the video buffer on the engine thread; `I_VideoBuffer` is a
    // live 320x200 allocation while a level is displayed.
    let (view_rect, pixels) = unsafe {
        use crate::doom::i_video::I_VideoBuffer;
        use crate::doom::r_draw::{scaledviewwidth, viewheight, viewwindowx, viewwindowy};
        if I_VideoBuffer.is_null() {
            return;
        }
        (
            ViewRect {
                x: viewwindowx,
                y: viewwindowy,
                w: scaledviewwidth,
                h: viewheight,
            },
            std::slice::from_raw_parts_mut(I_VideoBuffer, SCREEN_W * SCREEN_H),
        )
    };
    if view_rect.w <= 0 || view_rect.h <= 0 {
        return;
    }
    let alive = console_alive();
    let colours = HudColours::new(&frame.pal);
    let v = &frame.view;

    if v.overlay_active && alive {
        let id = v.weapon.map_or(0, |w| w.0);
        HUD.with(|h| {
            let h = h.borrow();
            draw_scope_overlay(
                pixels,
                view_rect,
                h.overlays.get(&id),
                colours.black,
                frame.aspect_correct,
            );
        });
    }

    let world_hfov = v.zoom_hfov_deg.unwrap_or(DOOM_HFOV);
    let drift = if v.overlay_active {
        [0.0, 0.0]
    } else {
        v.gun_angle_offset
    };
    let layout = crosshair_layout(view_rect, world_hfov, v.spread_deg, drift, &frame.hud);
    trace!(target: TARGET, "crosshair gap {:.2} centre ({}, {})", layout.gap_f, layout.cx, layout.cy);
    let mut cv = Canvas {
        pixels,
        pitch: SCREEN_W,
        clip: view_rect,
    };
    if crosshair_visible(
        frame.crosshair_enabled,
        alive,
        v.overlay_active,
        frame.sprinting,
        v.ads_frac,
        &frame.hud,
    ) {
        draw_crosshair(&mut cv, &layout, colours.cross);
    }
    if frame.hitmarker_enabled && alive && hitmarker_active() {
        let headshot = HUD.with(|h| h.borrow().hit_headshot);
        let colour = if headshot {
            colours.headshot
        } else {
            colours.hit
        };
        draw_hitmarker(&mut cv, layout.cx, layout.cy, colour);
    }
    let slot = |reload_due| PromptSlot {
        cx: layout.cx,
        top: prompt_top(&layout),
        reload_due,
        scoped: v.overlay_active,
    };
    if !alive || !frame.ammo_counter {
        HUD.with(|h| h.borrow_mut().slot = Some(slot(false)));
        return;
    }
    let reload = ammo_prompt(
        v.clip,
        v.stock,
        frame.hud.clip_size,
        is_reloading(v.weaponstate) || v.overlay_active,
        &super::useprompt::key_label(
            // SAFETY: plain int written at startup on the engine thread.
            unsafe { crate::doom::m_controls::key_dmw_reload },
        ),
    );
    HUD.with(|h| {
        let mut h = h.borrow_mut();
        h.slot = Some(slot(reload.is_some()));
        h.reload_prompt.clone_from(&reload);
    });
    if let Some(text) = reload {
        if blink_on(HUD.with(|h| h.borrow().tic)) {
            draw_text_centred(&text, layout.cx, prompt_top(&layout));
        }
    }
    if view_rect.h >= SCREEN_H as i32 {
        draw_fullscreen_counter(v.clip, v.stock, view_rect);
    }
}

/// The door and switch prompt, drawn after [`draw_view_overlays`] (same `D_Display` call
/// site): what Use would hit this frame, below the reload prompt slot. Nothing is activated.
/// Hidden when dead, scoped, a menu is open, or nothing useful is in reach.
pub(crate) fn draw_use_prompt() {
    HUD.with(|h| h.borrow_mut().use_prompt = None);
    if !mode::level_active() || HUD.with(|h| h.borrow().overlays_off) {
        return;
    }
    // SAFETY: engine globals on the engine thread.
    let (menu, automap, view_rect, use_key) = unsafe {
        use crate::doom::r_draw::{scaledviewwidth, viewheight, viewwindowx, viewwindowy};
        (
            crate::doom::m_menu::menuactive != 0,
            crate::doom::am_map::automapactive != 0,
            ViewRect {
                x: viewwindowx,
                y: viewwindowy,
                w: scaledviewwidth,
                h: viewheight,
            },
            crate::doom::m_controls::key_use,
        )
    };
    if menu || automap || view_rect.w <= 0 || view_rect.h <= 0 || !console_alive() {
        return;
    }
    let slot = HUD.with(|h| h.borrow().slot);
    if slot.is_some_and(|s| s.scoped) {
        return;
    }
    // SAFETY: `console_alive` just checked the index and the map object.
    let player = unsafe {
        let cp = usize::try_from(crate::doom::g_game::consoleplayer).unwrap_or(0);
        &*(&raw const crate::doom::g_game::players)
            .cast::<PlayerT>()
            .add(cp)
    };
    // SAFETY: live console player, engine thread, level loaded.
    let Some(text) = (unsafe { super::useprompt::current(player, use_key) }) else {
        return;
    };
    let (cx, top, due) = slot.map_or(
        (
            view_rect.x + view_rect.w / 2,
            view_rect.y + view_rect.h / 2 + 12,
            false,
        ),
        |s| (s.cx, s.top, s.reload_due),
    );
    draw_text_centred(&text, cx, use_prompt_top(top, due));
    HUD.with(|h| h.borrow_mut().use_prompt = Some(text));
}

/// The reload / no-ammo prompt text due in the last frame, whether or not the blink showed it
/// (`None` when none was due); for tests.
pub fn reload_prompt_text() -> Option<String> {
    HUD.with(|h| h.borrow().reload_prompt.clone())
}

/// The door/switch prompt text drawn in the last frame (`None` when none was); for tests.
pub fn use_prompt_text() -> Option<String> {
    HUD.with(|h| h.borrow().use_prompt.clone())
}

/// `ST_updateWidgets`: the magazine count for the AMMO widget while an MW2 weapon is in hand,
/// else `None` (the vanilla path). The widget points at [`DMW_READY_AMMO`].
pub(crate) unsafe fn status_ammo(_player: *mut PlayerT) -> Option<c_int> {
    if !mode::weapons_active() {
        return None;
    }
    let clip = with_state(|s| {
        if !s.config.hud.ammo_counter || s.driver.owner != super::weapons::Owner::Mw2 {
            return None;
        }
        let view = s.sim.as_ref()?.view_state();
        view.weapon.map(|_| view.clip)
    })
    .flatten()?;
    // SAFETY: single engine thread; a plain integer store.
    unsafe { DMW_READY_AMMO = clip };
    Some(clip)
}

/// The magazine count the AMMO widget shows (`None` when vanilla owns it); for tests.
pub fn ready_ammo() -> Option<c_int> {
    // SAFETY: the null player pointer is never dereferenced by `status_ammo`.
    unsafe { status_ammo(std::ptr::null_mut()) }
}

// ---------------------------------------------------------------------------
// Doom patches: digits and the HU font
// ---------------------------------------------------------------------------

fn lump_patch(name: &str) -> Option<*mut crate::doom::v_video::patch_t> {
    use crate::doom::w_wad::{W_CacheLumpName, W_CheckNumForName};
    use crate::doom::z_zone::PU_STATIC;
    let c = CString::new(name).ok()?;
    if W_CheckNumForName(c.as_ptr()) < 0 {
        return None;
    }
    let p = W_CacheLumpName(c.as_ptr(), PU_STATIC).cast::<crate::doom::v_video::patch_t>();
    (!p.is_null()).then_some(p)
}

fn load_patches() -> Option<Patches> {
    let mut p = Patches {
        tall: [std::ptr::null_mut(); 10],
        short: [std::ptr::null_mut(); 10],
        font: [std::ptr::null_mut(); 64],
    };
    for i in 0..10 {
        p.tall[i] = lump_patch(&format!("STTNUM{i}"))?;
        p.short[i] = lump_patch(&format!("STYSNUM{i}"))?;
    }
    // HU font: STCFN033 ('!') to STCFN095 ('_'); lower case is drawn upper case, as Doom does.
    for (i, slot) in p.font.iter_mut().enumerate() {
        *slot = lump_patch(&format!("STCFN{:03}", 33 + i)).unwrap_or(std::ptr::null_mut());
    }
    Some(p)
}

fn with_patches<R>(f: impl FnOnce(&Patches) -> R) -> Option<R> {
    HUD.with(|h| {
        let mut h = h.borrow_mut();
        let loaded = h.patches.get_or_insert_with(load_patches);
        match loaded {
            Some(p) => Some(f(p)),
            None => {
                log_once!(
                    TARGET,
                    log::Level::Warn,
                    "patches",
                    "STTNUM/STYSNUM/STCFN lumps missing; HUD text and digits are skipped"
                );
                None
            }
        }
    })
}

/// Width of a patch in pixels.
fn patch_width(p: *mut crate::doom::v_video::patch_t) -> i32 {
    if p.is_null() {
        4
    } else {
        // SAFETY: `p` came from `W_CacheLumpName` (`PU_STATIC`, never freed).
        i32::from(unsafe { (*p).width })
    }
}

fn draw_patch(x: i32, y: i32, p: *mut crate::doom::v_video::patch_t) {
    if !p.is_null() {
        crate::doom::v_video::V_DrawPatch(x, y, p);
    }
}

/// Draws `text` (upper-cased) in the HU font, centred on column `cx`, top row `y`.
fn draw_text_centred(text: &str, cx: i32, y: i32) {
    with_patches(|p| {
        let glyph = |c: char| {
            let code = c.to_ascii_uppercase() as usize;
            if (33..=96).contains(&code) {
                p.font[code - 33]
            } else {
                std::ptr::null_mut()
            }
        };
        let width: i32 = text.chars().map(|c| patch_width(glyph(c))).sum();
        let mut x = cx - width / 2;
        for c in text.chars() {
            let g = glyph(c);
            draw_patch(x, y, g);
            x += patch_width(g);
        }
    });
}

/// Draws a non-negative number right-aligned to column `right`, top row `y`.
fn draw_number(n: i32, right: i32, y: i32, digits: &[*mut crate::doom::v_video::patch_t; 10]) {
    let text = n.clamp(0, 9999).to_string();
    let mut x = right;
    for ch in text.bytes().rev() {
        let d = digits[usize::from(ch - b'0')];
        x -= patch_width(d);
        draw_patch(x, y, d);
    }
}

/// Fullscreen (no status bar): magazine in big digits and reserve in small digits at the
/// bottom right of the view.
fn draw_fullscreen_counter(clip: i32, stock: i32, view: ViewRect) {
    with_patches(|p| {
        let right = view.x + view.w - 4;
        let bottom = view.y + view.h - 4;
        draw_number(stock, right, bottom - 6, &p.short);
        draw_number(clip, right, bottom - 6 - 4 - 16, &p.tall);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hud() -> HudFacts {
        HudFacts {
            ads_crosshair_in_frac: 0.5,
            ads_crosshair_out_frac: 0.3,
            hip_reticle_side_pos: 2.0,
            i_reticle_min_ofs: 6,
            i_reticle_side_size: 6,
            clip_only: false,
            clip_size: 30,
            ammo_counter_clip: 0,
        }
    }

    const FULL: ViewRect = ViewRect {
        x: 0,
        y: 0,
        w: 320,
        h: 200,
    };

    #[test]
    fn t12_1_crosshair_gap_at_rest_is_the_reticle_minimum() {
        let l = crosshair_layout(FULL, 90.0, 0.0, [0.0, 0.0], &hud());
        assert_eq!((l.cx, l.cy), (160, 100));
        assert!((l.gap_f - 3.0).abs() < 1e-4, "6 virtual px at 320/640");
        assert_eq!(l.gap, 3);
        assert_eq!(l.length, 3);
    }

    #[test]
    fn t12_1_crosshair_gap_follows_spread() {
        let l = crosshair_layout(FULL, 90.0, 3.0, [0.0, 0.0], &hud());
        let expect = 3f32.to_radians().tan() * 160.0;
        assert!((l.gap_f - expect).abs() < 1e-3, "{} vs {expect}", l.gap_f);
        assert!((l.gap_f - 8.39).abs() < 0.01);
        assert_eq!(l.gap, 8);
    }

    #[test]
    fn crosshair_gap_scales_with_zoom_and_view_size() {
        // 45 degree world FOV doubles-plus the focal length.
        let zoomed = crosshair_layout(FULL, 45.0, 3.0, [0.0, 0.0], &hud());
        let focal = 160.0 / 22.5f32.to_radians().tan();
        assert!((zoomed.gap_f - 3f32.to_radians().tan() * focal).abs() < 1e-3);
        // A half-width view window halves the virtual-pixel scale.
        let small = ViewRect {
            x: 80,
            y: 20,
            w: 160,
            h: 100,
        };
        let l = crosshair_layout(small, 90.0, 0.0, [0.0, 0.0], &hud());
        assert_eq!((l.cx, l.cy), (80 + 80, 20 + 50));
        assert!((l.gap_f - 1.5).abs() < 1e-4);
        assert_eq!(l.length, 2, "never below two pixels");
    }

    #[test]
    fn crosshair_drifts_with_the_gun_axis() {
        // Gun pitched 5 degrees down and yawed 5 degrees left: centre moves down and left.
        let l = crosshair_layout(FULL, 90.0, 0.0, [5.0, 5.0], &hud());
        let px = (5f32.to_radians().tan() * 160.0).round() as i32;
        assert_eq!((l.cx, l.cy), (160 - px, 100 + px));
        let up = crosshair_layout(FULL, 90.0, 0.0, [-5.0, -5.0], &hud());
        assert_eq!((up.cx, up.cy), (160 + px, 100 - px));
    }

    #[test]
    fn t12_1_visibility_rules() {
        let h = hud();
        assert!(crosshair_visible(true, true, false, false, 0.0, &h));
        assert!(crosshair_visible(true, true, false, false, 0.49, &h));
        assert!(
            !crosshair_visible(true, true, false, false, 0.5, &h),
            "hidden at the in fraction"
        );
        assert!(!crosshair_visible(true, true, false, false, 1.0, &h));
        assert!(
            !crosshair_visible(true, true, false, true, 0.0, &h),
            "sprinting"
        );
        assert!(
            !crosshair_visible(true, true, true, false, 0.0, &h),
            "scope overlay"
        );
        assert!(
            !crosshair_visible(true, false, false, false, 0.0, &h),
            "dead"
        );
        assert!(
            !crosshair_visible(false, true, false, false, 0.0, &h),
            "[hud] crosshair = false"
        );
    }

    #[test]
    fn crosshair_draws_four_arms_clipped_to_the_view() {
        let mut buf = vec![0u8; SCREEN_W * SCREEN_H];
        let l = crosshair_layout(FULL, 90.0, 0.0, [0.0, 0.0], &hud());
        let mut cv = Canvas {
            pixels: &mut buf,
            pitch: SCREEN_W,
            clip: FULL,
        };
        draw_crosshair(&mut cv, &l, 7);
        let at = |x: usize, y: usize| buf[y * SCREEN_W + x];
        for k in 3..6 {
            assert_eq!(at(160, 100 - k), 7);
            assert_eq!(at(160, 100 + k), 7);
            assert_eq!(at(160 - k, 100), 7);
            assert_eq!(at(160 + k, 100), 7);
        }
        assert_eq!(at(160, 100), 0, "centre pixel stays clear");
        assert_eq!(at(160, 100 - 6), 0);
        assert_eq!(buf.iter().filter(|&&p| p == 7).count(), 12);
        // Clipped: a view window that excludes the centre rows draws nothing there.
        let mut buf2 = vec![0u8; SCREEN_W * SCREEN_H];
        let mut cv2 = Canvas {
            pixels: &mut buf2,
            pitch: SCREEN_W,
            clip: ViewRect {
                x: 0,
                y: 0,
                w: 320,
                h: 100,
            },
        };
        draw_crosshair(&mut cv2, &l, 7);
        assert_eq!(
            buf2.iter().filter(|&&p| p == 7).count(),
            3,
            "only the up arm is inside rows 0..100; the centre row 100 is clipped"
        );
    }

    #[test]
    fn hitmarker_draws_four_diagonals() {
        let mut buf = vec![0u8; SCREEN_W * SCREEN_H];
        let mut cv = Canvas {
            pixels: &mut buf,
            pitch: SCREEN_W,
            clip: FULL,
        };
        draw_hitmarker(&mut cv, 160, 100, 9);
        assert_eq!(buf.iter().filter(|&&p| p == 9).count(), 12);
        assert_eq!(buf[(100 - 4) * SCREEN_W + 160 - 4], 9);
        assert_eq!(buf[(100 + 5) * SCREEN_W + 160 + 5], 9);
    }

    #[test]
    fn prompt_rules() {
        let p = |c, s, z, r| ammo_prompt(c, s, z, r, "R");
        assert_eq!(p(30, 90, 30, false), None);
        assert_eq!(p(8, 90, 30, false), None);
        assert_eq!(p(7, 90, 30, false).as_deref(), Some("Press R to reload"));
        assert_eq!(
            p(1, 90, 5, false).as_deref(),
            Some("Press R to reload"),
            "at least one round"
        );
        assert_eq!(p(2, 90, 5, false), None);
        assert_eq!(p(0, 0, 30, false).as_deref(), Some("NO AMMO"));
        assert_eq!(p(3, 0, 30, false), None, "nothing to reload with");
        assert_eq!(p(0, 40, 30, true), None, "not while reloading");
        assert_eq!(
            ammo_prompt(7, 90, 30, false, "T").as_deref(),
            Some("Press T to reload"),
            "the letter follows the binding"
        );
        assert_eq!(use_prompt_top(100, false), 100);
        assert_eq!(
            use_prompt_top(100, true),
            110,
            "stacks under the reload prompt"
        );
        assert!(is_reloading(WeaponState::Reloading as i32));
        assert!(is_reloading(WeaponState::ReloadEnd as i32));
        assert!(!is_reloading(WeaponState::Ready as i32));
        assert!(!is_reloading(WeaponState::Firing as i32));
    }

    #[test]
    fn prompt_blinks_at_two_hertz() {
        assert!(blink_on(0) && blink_on(8));
        assert!(!blink_on(9) && !blink_on(17));
        assert!(blink_on(18));
        let on = (0..35).filter(|&t| blink_on(t)).count();
        assert!(
            (16..=19).contains(&on),
            "about half of a second is lit: {on}"
        );
    }

    #[test]
    fn scope_rect_is_view_tall_and_centred() {
        let v = ViewRect {
            x: 0,
            y: 0,
            w: 320,
            h: 168,
        };
        let r = scope_rect(v, 256, 256, true);
        assert_eq!((r.y, r.h), (0, 168));
        assert_eq!(r.w, (168.0f32 / PIXEL_ASPECT).round() as i32);
        assert_eq!(r.x, (320 - r.w) / 2);
        let square = scope_rect(v, 256, 256, false);
        assert_eq!(square.w, 168);
    }

    #[test]
    fn procedural_scope_masks_outside_the_circle_and_keeps_the_centre_lines() {
        let view = ViewRect {
            x: 0,
            y: 0,
            w: 320,
            h: 168,
        };
        let mut buf = vec![5u8; SCREEN_W * SCREEN_H];
        draw_scope_overlay(&mut buf, view, None, 0, true);
        let at = |x: usize, y: usize| buf[y * SCREEN_W + x];
        assert_eq!(at(0, 0), 0);
        assert_eq!(at(319, 167), 0);
        assert_eq!(at(160, 84 + 20), 0, "vertical reticle line");
        assert_eq!(at(160 + 20, 84), 0, "horizontal reticle line");
        assert_eq!(at(160 + 10, 84 + 10), 5, "world shows inside the circle");
        assert_eq!(at(10, 190), 5, "rows below the view window are untouched");
    }
}

#[cfg(test)]
mod reload_prompt_tests {
    use super::ammo_prompt;

    #[test]
    fn one_round_weapon_prompts_only_when_empty() {
        assert_eq!(ammo_prompt(1, 5, 1, false, "R"), None, "loaded RPG: no prompt");
        assert_eq!(ammo_prompt(0, 5, 1, false, "R").as_deref(), Some("Press R to reload"));
        assert_eq!(ammo_prompt(0, 0, 1, false, "R").as_deref(), Some("NO AMMO"));
    }

    #[test]
    fn full_magazine_never_prompts_and_low_one_does() {
        assert_eq!(ammo_prompt(2, 5, 2, false, "R"), None, "full 2-round magazine");
        assert_eq!(ammo_prompt(1, 5, 2, false, "R").as_deref(), Some("Press R to reload"));
        assert_eq!(ammo_prompt(3, 60, 12, false, "R").as_deref(), Some("Press R to reload"));
        assert_eq!(ammo_prompt(4, 60, 12, false, "R"), None);
    }
}
