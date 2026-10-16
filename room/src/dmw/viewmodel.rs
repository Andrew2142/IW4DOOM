// SPDX-License-Identifier: GPL-3.0-or-later
//! Viewmodel glue (sheet 11 section 6): the prepared-model registry, the
//! `draw_player_weapon` hook body, light derivation, the view-window `DrawTarget` and the
//! muzzle flash (procedural burst from [`super::muzzle`], or the Doom patch).
//!
//! Sheet 08 owns the call sites of [`prepare_all`] (once per session at the first active level
//! start) and [`on_tic`] (after every MW2 tic). `hooks::draw_player_weapon` delegates to
//! [`draw_player_weapon`] at the start of `R_DrawPlayerSprites`.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::Arc;
use std::time::Instant;

use dmw_format::consts::{DOOM_SCREEN_H, DOOM_SCREEN_W, PIXEL_ASPECT};
use dmw_format::{AlphaMode, DmwWeaponId, Pack, ViewmodelAsset, WeaponFacts};
use dmw_viewmodel::{
    overlay, quantize, Dither, DrawStats, DrawTarget, IndexedTexture, LightParams, PaletteTables,
    Pose, PreparedViewmodel, RasterError, Rasterizer, ViewmodelDrawParams, Visibility, NEAR_PLANE,
};
use dmw_weapons::{SoundCue, ViewState, ViewmodelAnimator};
use glam::{Mat4, Vec3};
use log::{error, info, trace, warn};
use weapon_iw4::{WEAPCLASS_PISTOL, WEAPCLASS_SPREAD, WEAPTYPE_BULLET, WEAPTYPE_PROJECTILE};

use super::config::{DitherCfg, FlashCfg, RenderCfg};
use super::{mode, muzzle, weapons, with_state, DmwState};

const TARGET: &str = "dmw::raster";

/// `LIGHTLEVELS`, `LIGHTSEGSHIFT`, `MAXLIGHTSCALE` of `r_things.rs` / `r_main.rs`.
const LIGHTLEVELS: i32 = 16;
const LIGHTSEGSHIFT: i32 = 4;
const MAXLIGHTSCALE: usize = 48;
/// `pw_invisibility` (d_player powers index).
const PW_INVISIBILITY: usize = 2;
/// `PST_LIVE`.
const PST_LIVE: i32 = 0;
/// Last valid COLORMAP index (34 maps: 0..31 light, 32 invulnerability, 33 black).
const LAST_COLORMAP: i32 = 33;
/// Muzzle distance (inches) at which the flash patch is drawn at psprite scale (section 6.3).
const FLASH_REF_DIST: f32 = 24.0;

/// One mapped weapon's viewmodel: quantized model, animator and quantized scope overlay.
#[derive(Debug)]
pub struct Entry {
    /// Quantized textures and mesh in draw layout (sheet 11 section 5.3).
    pub prepared: Arc<PreparedViewmodel>,
    /// Sheet 10 animator (clips, pose, visibility).
    pub animator: ViewmodelAnimator,
    /// Scope overlay quantized to the IWAD palette (sheet 12 draws it), when the weapon has one.
    pub overlay: Option<IndexedTexture>,
}

/// Per-weapon viewmodel data, keyed by `DmwWeaponId.0`, and the shared rasterizer.
#[derive(Debug, Default)]
pub struct Registry {
    /// Every mapped weapon whose viewmodel loaded and prepared.
    pub by_weapon: BTreeMap<u16, Entry>,
    /// Z-buffer and scratch buffers, reused every frame.
    raster: Rasterizer,
    /// Stats of the last draw (`test-hooks`, diagnostics).
    pub last_stats: Option<DrawStats>,
    /// Light mode of the last hook draw (`test-hooks`, diagnostics).
    pub last_light: Option<Light>,
}

impl Registry {
    /// The entry of `id`, if prepared.
    pub fn entry(&self, id: DmwWeaponId) -> Option<&Entry> {
        self.by_weapon.get(&id.0)
    }
}

thread_local! {
    /// Window inner size, for the pixel aspect with `[render] aspect_correct = false`.
    static WINDOW_SIZE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// Flash patch lumps already looked up (`None` = missing from the WADs).
    static FLASH_LUMPS: RefCell<BTreeMap<&'static CStr, Option<&'static [u8]>>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Records the window's inner size (platform layer, at creation and on every resize).
pub fn set_window_size(width: u32, height: u32) {
    WINDOW_SIZE.set((width > 0 && height > 0).then_some((width, height)));
}

/// Displayed pixel width / height of the 320x200 frame (section 6.2).
fn pixel_aspect(render: &RenderCfg) -> f32 {
    if render.aspect_correct {
        return PIXEL_ASPECT;
    }
    match WINDOW_SIZE.get() {
        Some((w, h)) => (w as f32 / DOOM_SCREEN_W as f32) / (h as f32 / DOOM_SCREEN_H as f32),
        // No window (headless): the default 640x400 window stretches 2:2.
        None => 1.0,
    }
}

fn dither_of(cfg: DitherCfg) -> Dither {
    match cfg {
        DitherCfg::None => Dither::None,
        DitherCfg::Bayer4 => Dither::Bayer4,
    }
}

// ---------------------------------------------------------------------------
// Preparation
// ---------------------------------------------------------------------------

/// Loaded inputs of one weapon, gathered on the engine thread (`Pack` is `!Sync`).
struct Job {
    id: DmwWeaponId,
    name: String,
    asset: Arc<ViewmodelAsset>,
    animator: ViewmodelAnimator,
    overlay: Option<Arc<dmw_format::RgbaImage>>,
}

/// Output of the parallel part.
type Prepared = (
    Result<PreparedViewmodel, RasterError>,
    Option<IndexedTexture>,
    f64,
);

fn prepare_one(
    asset: &ViewmodelAsset,
    overlay: Option<&dmw_format::RgbaImage>,
    pal: &PaletteTables,
    dither: Dither,
) -> Prepared {
    let started = Instant::now();
    let model = PreparedViewmodel::new(asset, pal, dither);
    let overlay = overlay.map(|img| quantize(img, pal, dither, AlphaMode::Blend));
    (model, overlay, started.elapsed().as_secs_f64() * 1000.0)
}

/// Builds the registry for every mapped weapon: animator, quantized model and overlay
/// (sheet 11 section 6.1). Called once per session with the state borrowed, after sheet 08 has
/// checked (and demoted) the mapped viewmodels. A weapon that cannot be prepared is demoted to
/// vanilla, so its Doom slot keeps the original psprite weapon.
pub(crate) fn prepare_all(state: &mut DmwState) {
    let Some(pack) = state.pack.clone() else {
        return;
    };
    let started = Instant::now();
    let doom_ammo = weapons::doom_weapon_ammo();
    let Some(pal) = state.palette.clone() else {
        error!(target: TARGET, "no palette tables: MW2 viewmodels cannot be drawn, every slot stays vanilla");
        for id in state.mapping.ids() {
            state.mapping.demote(id, doom_ammo);
        }
        return;
    };
    let dither = dither_of(state.config.render.dither);

    let mut jobs = Vec::new();
    for id in state.mapping.ids() {
        if let Some(job) = load_job(&pack, id) {
            jobs.push(job);
        } else {
            state.mapping.demote(id, doom_ammo);
        }
    }

    // The quantization is independent per weapon (01 section 5.4 allows scoped threads here).
    let results: Vec<Prepared> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|job| {
                let (asset, overlay, pal) = (&job.asset, job.overlay.as_deref(), &*pal);
                std::thread::Builder::new()
                    .name(format!("dmw-prepare-{}", job.id.0))
                    .spawn_scoped(scope, move || prepare_one(asset, overlay, pal, dither))
                    .map_err(|_| (asset, overlay))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| match h {
                Ok(handle) => handle.join().unwrap_or_else(|_| {
                    (
                        Err(RasterError::Mesh("preparation panicked".to_owned())),
                        None,
                        0.0,
                    )
                }),
                // No thread available: prepare inline.
                Err((asset, overlay)) => prepare_one(asset, overlay, &pal, dither),
            })
            .collect()
    });

    for (job, (model, overlay, ms)) in jobs.into_iter().zip(results) {
        match model {
            Ok(model) => {
                info!(
                    target: TARGET,
                    "prepared viewmodel {}: {} triangles, {:?} winding, {ms:.1} ms",
                    job.name,
                    model.triangle_count(),
                    model.winding()
                );
                state.viewmodels.by_weapon.insert(
                    job.id.0,
                    Entry {
                        prepared: Arc::new(model),
                        animator: job.animator,
                        overlay,
                    },
                );
            }
            Err(e) => {
                error!(target: TARGET, "viewmodel of {} not prepared ({e}); its slots stay vanilla", job.name);
                state.mapping.demote(job.id, doom_ammo);
            }
        }
    }
    info!(
        target: TARGET,
        "prepared {} viewmodels in {:.1} ms",
        state.viewmodels.by_weapon.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// Loads the asset, builds the animator and loads the overlay of `id` (engine thread).
fn load_job(pack: &Pack, id: DmwWeaponId) -> Option<Job> {
    let facts: &WeaponFacts = pack.try_facts(id)?;
    let asset = match pack.load_viewmodel(id) {
        Ok(a) => a,
        Err(e) => {
            error!(target: TARGET, "viewmodel of {} failed to load ({e}); its slots stay vanilla", facts.name);
            return None;
        }
    };
    let animator = match ViewmodelAnimator::new(Arc::clone(&asset), facts) {
        Ok(a) => a,
        Err(e) => {
            error!(target: TARGET, "animator for {} not built ({e}); its slots stay vanilla", facts.name);
            return None;
        }
    };
    let overlay = if facts.has_overlay {
        pack.load_overlay(id).unwrap_or_else(|e| {
            warn!(target: TARGET, "scope overlay of {} not loaded: {e}", facts.name);
            None
        })
    } else {
        None
    };
    Some(Job {
        id,
        name: facts.name.clone(),
        asset,
        animator,
        overlay,
    })
}

/// Advances the held weapon's animation by one tic and returns its notetrack sounds (sheet 10).
/// Runs outside any `DmwState` borrow (it takes its own).
pub(crate) fn on_tic(view: &ViewState, msec: i32) -> Vec<SoundCue> {
    let Some(id) = view.weapon else {
        return Vec::new();
    };
    with_state(|s| {
        s.viewmodels
            .by_weapon
            .get_mut(&id.0)
            .map(|e| e.animator.advance(view, msec))
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Light mode chosen from Doom's renderer state (owned copy; the fuzz references are made at
/// draw time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// Sector light: COLORMAP level and the directional term flag.
    Sector {
        /// 0 bright .. 31 dark.
        level: u8,
        /// Add the per-vertex directional term.
        directional: bool,
    },
    /// Doom's `fixedcolormap` (light amplification, invulnerability).
    Fixed(u8),
    /// Partial invisibility.
    Fuzz,
}

/// Inputs of the light choice, read from Doom globals by [`read_light_inputs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightInputs {
    /// `viewplayer->powers[pw_invisibility]`.
    pub invisibility: i32,
    /// COLORMAP index of the renderer's `fixedcolormap`, `None` when it is null.
    pub fixed: Option<i32>,
    /// COLORMAP index of `spritelights[MAXLIGHTSCALE-1]`, chosen as `R_DrawPlayerSprites` does.
    pub sprite_level: i32,
}

/// The light mode for the gun, in `R_DrawPSprite`'s order: invisibility first, then
/// `fixedcolormap`, then the sector light (section 6.2).
pub fn choose_light(inp: LightInputs, directional: bool) -> Light {
    if inp.invisibility > 4 * 32 || inp.invisibility & 8 != 0 {
        Light::Fuzz
    } else if let Some(c) = inp.fixed {
        Light::Fixed(c.clamp(0, LAST_COLORMAP) as u8)
    } else {
        Light::Sector {
            level: inp.sprite_level.clamp(0, 31) as u8,
            directional,
        }
    }
}

/// COLORMAP index of a light table pointer (`(ptr - colormaps) / 256`).
fn colormap_index(ptr: *const u8, colormaps: *const u8) -> i32 {
    ((ptr as isize - colormaps as isize) / 256) as i32
}

/// Reads the light inputs exactly as `R_DrawPlayerSprites` / `R_DrawPSprite` would.
///
/// # Safety
/// A level is being rendered: `viewplayer`, its map object, subsector and sector are live and
/// `scalelight`/`colormaps` are initialised.
unsafe fn read_light_inputs() -> LightInputs {
    use crate::doom::r_data::colormaps;
    use crate::doom::r_main::{extralight, fixedcolormap, scalelight, viewplayer};
    // SAFETY: caller contract; the same reads R_DrawPlayerSprites makes.
    unsafe {
        let mo = (*viewplayer).mo as *mut crate::doom::c_ffi::mobj_t;
        let sub = (*mo).subsector as *mut crate::doom::r_bsp::subsector_t;
        let sec = (*sub).sector;
        let lightnum = i32::from((*sec).lightlevel >> LIGHTSEGSHIFT as i16) + extralight;
        let row = lightnum.clamp(0, LIGHTLEVELS - 1) as usize;
        let table = (&raw const scalelight[row][MAXLIGHTSCALE - 1]).read();
        LightInputs {
            invisibility: (*viewplayer).powers[PW_INVISIBILITY],
            fixed: (!fixedcolormap.is_null()).then(|| colormap_index(fixedcolormap, colormaps)),
            sprite_level: colormap_index(table, colormaps),
        }
    }
}

/// View window and projection parameters of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// `viewwindowx`.
    pub x: i32,
    /// `viewwindowy`.
    pub y: i32,
    /// `scaledviewwidth`.
    pub w: i32,
    /// `viewheight`.
    pub h: i32,
    /// Displayed pixel width / height.
    pub pixel_aspect: f32,
}

impl Window {
    /// The `DrawTarget` over `pixels` (320 per row) with the unsheared centre (06 section 5.2).
    fn target<'a>(&self, pixels: &'a mut [u8]) -> DrawTarget<'a> {
        DrawTarget {
            pixels,
            pitch: DOOM_SCREEN_W,
            view_x: self.x,
            view_y: self.y,
            view_w: self.w,
            view_h: self.h,
            center_x: self.w as f32 / 2.0,
            center_y: self.h as f32 / 2.0,
            pixel_aspect: self.pixel_aspect,
        }
    }

    /// Screen position of eye-space point `p` (section 5.5), `None` behind the near plane.
    pub fn project(&self, p: Vec3, hfov_deg: f32) -> Option<(f32, f32)> {
        let fx = (self.w as f32 * 0.5) / (hfov_deg * 0.5).to_radians().tan();
        let fy = fx * self.pixel_aspect;
        if !(p.is_finite() && p.x >= NEAR_PLANE && fx.is_finite() && fx > 0.0) {
            return None;
        }
        Some((
            self.x as f32 + self.w as f32 / 2.0 - fx * (p.y / p.x),
            self.y as f32 + self.h as f32 / 2.0 - fy * (p.z / p.x),
        ))
    }
}

/// Reads the view window globals.
fn read_window(render: &RenderCfg) -> Window {
    use crate::doom::r_draw::{scaledviewwidth, viewheight, viewwindowx, viewwindowy};
    // SAFETY: plain reads of renderer globals on the single engine thread.
    unsafe {
        Window {
            x: viewwindowx,
            y: viewwindowy,
            w: scaledviewwidth,
            h: viewheight,
            pixel_aspect: pixel_aspect(render),
        }
    }
}

/// Doom flash sprite for a weapon (section 6.3). `parity` alternates per shot.
pub fn flash_patch(
    weap_class: i32,
    weap_type: i32,
    fire_type: i32,
    projectile: bool,
    parity: bool,
) -> &'static CStr {
    const FULL_AUTO: i32 = weapon_iw4::FireType::FullAuto as i32;
    if weap_class == WEAPCLASS_PISTOL {
        c"PISFA0"
    } else if weap_class == WEAPCLASS_SPREAD {
        if parity {
            c"SHTFB0"
        } else {
            c"SHTFA0"
        }
    } else if projectile || weap_type == WEAPTYPE_PROJECTILE {
        c"MISFA0"
    } else if weap_type == WEAPTYPE_BULLET && fire_type == FULL_AUTO {
        if parity {
            c"CHGFB0"
        } else {
            c"CHGFA0"
        }
    } else {
        c"SHTFB0"
    }
}

/// Doom flash patch scale (`muzzle_flash = "doom"`) for a muzzle `x_eye` inches in front of
/// the eye (section 6.3); `view_scale` is `scaledviewwidth / 320` (`pspritescale / FRACUNIT`
/// at full detail).
pub fn flash_scale(view_scale: f32, x_eye: f32) -> f32 {
    (view_scale * FLASH_REF_DIST / x_eye).clamp(0.4, 2.0)
}

/// A flash to draw after the state borrow is released.
#[derive(Clone, Copy, Debug)]
enum FlashRequest {
    /// Doom flash patch (`muzzle_flash = "doom"`).
    Patch {
        lump: &'static CStr,
        cx: f32,
        cy: f32,
        scale: f32,
    },
    /// Procedural MW2 burst (`muzzle_flash = "mw2"`).
    Burst(muzzle::Burst),
}

/// Draws the held weapon of `view` into `pixels` (320x200) through the registry's rasterizer:
/// the shared body of the hook and the `test-hooks` isolated render. Returns the flash patch
/// to draw, if any, and the draw stats.
fn render_gun(
    s: &mut DmwState,
    view: &ViewState,
    win: Window,
    light: Light,
    pixels: &mut [u8],
    fuzzpos: &mut usize,
) -> Option<(DrawStats, Option<FlashRequest>)> {
    let id = view.weapon?;
    let pal = s.palette.clone()?;
    let flash_cfg = s.config.render.muzzle_flash;
    let flash_on = flash_cfg != FlashCfg::None && s.muzzle.flash_tics > 0;
    let facts = s.pack.as_ref().and_then(|p| p.try_facts(id)).map(|f| {
        (
            f.combat.weap_class,
            f.combat.weap_type,
            f.combat.fire_type,
            f.projectile.is_some(),
        )
    });
    let reg = &mut s.viewmodels;
    let Some(entry) = reg.by_weapon.get(&id.0) else {
        log_once!(
            TARGET,
            log::Level::Warn,
            &format!("noprep:{}", id.0),
            "weapon {} has no prepared viewmodel",
            id.0
        );
        return None;
    };
    let vp = entry.animator.pose(view);
    let mask = entry.animator.visibility(view);
    let pose = Pose {
        skin: vp.skin,
        tag_flash: vp.tag_flash,
    };
    let fuzzoffset = fuzz_table();
    let light = match light {
        Light::Sector { level, directional } => LightParams::Sector {
            colormap_level: level,
            directional,
        },
        Light::Fixed(c) => LightParams::Fixed { colormap: c },
        Light::Fuzz => LightParams::Fuzz {
            fuzzoffset: &fuzzoffset,
            fuzzpos,
        },
    };
    let mut params = ViewmodelDrawParams {
        pose: &pose,
        hfov_deg: view.viewmodel_fov_deg,
        light,
        pal: &pal,
        visibility: Visibility {
            rocket: mask.rocket,
            knife: mask.knife,
        },
    };
    let mut target = win.target(pixels);
    let stats = reg.raster.draw(&mut target, &entry.prepared, &mut params);
    reg.last_stats = Some(stats);
    trace!(target: TARGET, "viewmodel {}: {stats:?}", id.0);

    let flash = match (flash_on, pose.tag_flash, facts) {
        (true, Some(m), Some((class, wtype, fire, proj))) => {
            let origin = flash_origin(&m);
            let view_scale = win.w as f32 / DOOM_SCREEN_W as f32;
            win.project(origin, view.viewmodel_fov_deg)
                .map(|(cx, cy)| match flash_cfg {
                    FlashCfg::Doom => FlashRequest::Patch {
                        lump: flash_patch(class, wtype, fire, proj, view.clip & 1 == 1),
                        cx,
                        cy,
                        scale: flash_scale(view_scale, origin.x),
                    },
                    _ => {
                        let centre = (
                            win.x as f32 + win.w as f32 / 2.0,
                            win.y as f32 + win.h as f32 / 2.0,
                        );
                        FlashRequest::Burst(muzzle::Burst {
                            kind: muzzle::classify(class, wtype, proj),
                            variant: view.clip as u32,
                            scale: muzzle::scale(view_scale, origin.x),
                            cx,
                            cy,
                            fwd: muzzle::direction((cx, cy), centre),
                            pixel_aspect: win.pixel_aspect,
                        })
                    }
                })
        }
        _ => None,
    };
    Some((stats, flash))
}

fn flash_origin(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

/// Copy of Doom's `fuzzoffset` table.
fn fuzz_table() -> [i32; crate::doom::c_ffi::FUZZTABLE] {
    // SAFETY: plain read of a renderer static on the single engine thread.
    unsafe { crate::doom::r_draw::fuzzoffset }
}

/// The lump bytes of a flash patch, looked up once (`None` when the WADs lack it).
fn flash_lump(name: &'static CStr) -> Option<&'static [u8]> {
    use crate::doom::w_wad::{W_CacheLumpNum, W_CheckNumForName, W_LumpLength};
    use crate::doom::z_zone::PU_STATIC;
    if let Some(hit) = FLASH_LUMPS.with_borrow(|m| m.get(name).copied()) {
        return hit;
    }
    let num = W_CheckNumForName(name.as_ptr());
    let bytes = u32::try_from(num).ok().and_then(|n| {
        let len = usize::try_from(W_LumpLength(n)).ok()?;
        let ptr = W_CacheLumpNum(num, PU_STATIC).cast::<u8>();
        // SAFETY: `W_CacheLumpNum` returns the lump's `len` bytes, kept in the zone with
        // `PU_STATIC` (never freed) for the rest of the process.
        (!ptr.is_null()).then(|| unsafe { std::slice::from_raw_parts(ptr.cast_const(), len) })
    });
    FLASH_LUMPS.with_borrow_mut(|m| m.insert(name, bytes));
    bytes
}

/// The Doom screen buffer as a 320x200 slice.
///
/// # Safety
/// Video is initialised and no other reference to `I_VideoBuffer` is live.
unsafe fn video_buffer<'a>() -> Option<&'a mut [u8]> {
    use crate::doom::i_video::I_VideoBuffer;
    // SAFETY: caller contract; `I_VideoBuffer` is a 320*200 zone allocation.
    unsafe {
        (!I_VideoBuffer.is_null())
            .then(|| std::slice::from_raw_parts_mut(I_VideoBuffer, DOOM_SCREEN_W * DOOM_SCREEN_H))
    }
}

/// Draws a flash fullbright over the gun (no depth test), clipped to the view window.
fn draw_flash(req: FlashRequest, win: Window, pal: &PaletteTables, pixels: &mut [u8]) {
    let Some(fullbright) = pal.colormaps.first() else {
        return;
    };
    let (lump_name, cx, cy, scale) = match req {
        FlashRequest::Patch {
            lump,
            cx,
            cy,
            scale,
        } => (lump, cx, cy, scale),
        FlashRequest::Burst(burst) => {
            let tone = |t: muzzle::Tone| fullbright[usize::from(pal.nearest(t.rgb()))];
            let colours = [
                tone(muzzle::Tone::White),
                tone(muzzle::Tone::Pale),
                tone(muzzle::Tone::Yellow),
                tone(muzzle::Tone::Orange),
            ];
            let (x1, y1) = (win.x + win.w, win.y + win.h);
            for (x, y, t) in muzzle::pixels(&burst) {
                let inside = x >= win.x.max(0)
                    && x < x1.min(DOOM_SCREEN_W as i32)
                    && y >= win.y.max(0)
                    && y < y1.min(DOOM_SCREEN_H as i32);
                if inside {
                    pixels[y as usize * DOOM_SCREEN_W + x as usize] = colours[t as usize];
                }
            }
            return;
        }
    };
    let Some(lump) = flash_lump(lump_name) else {
        log_once!(
            TARGET,
            log::Level::Warn,
            &format!("flash:{}", lump_name.to_string_lossy()),
            "muzzle flash patch {} missing from the WADs; no flash drawn",
            lump_name.to_string_lossy()
        );
        return;
    };
    let mut target = win.target(pixels);
    overlay::blit_doom_patch(&mut target, lump, cx, cy, scale, fullbright);
}

/// What the hook decided inside the state borrow.
enum Decision {
    /// Vanilla psprites draw.
    Vanilla,
    /// MW2 owns the weapon; nothing (more) to draw.
    Hidden,
    /// Gun drawn; maybe a flash to blit.
    Drawn(Option<(FlashRequest, Window, Arc<PaletteTables>)>),
}

/// `R_DrawPlayerSprites`, start (sheet 11 section 6.2). `true` = skip the vanilla psprites.
pub(crate) fn draw_player_weapon() -> bool {
    if !mode::weapons_active() {
        return false;
    }
    // SAFETY: called by R_DrawPlayerSprites while a level is rendered: `viewplayer` is the
    // live view player; plain reads on the engine thread.
    let (playerstate, light_in) = unsafe {
        let vp = crate::doom::r_main::viewplayer;
        if vp.is_null() || (*vp).mo.is_null() {
            return false;
        }
        ((*vp).playerstate, read_light_inputs())
    };
    // SAFETY: plain read of `fuzzpos` on the engine thread.
    let mut fuzzpos = usize::try_from(unsafe { crate::doom::r_draw::fuzzpos }).unwrap_or(0);

    let decision = with_state(|s| {
        if !weapons::mw2_owns_weapon(s) {
            return Decision::Vanilla;
        }
        // Dead: no weapon, like MW2; also hides the vanilla death lowering (08 section 11 #9).
        if playerstate != PST_LIVE {
            return Decision::Hidden;
        }
        let Some(view) = s.sim.as_ref().map(dmw_weapons::WeaponSim::view_state) else {
            return Decision::Hidden;
        };
        if view.overlay_active {
            return Decision::Hidden;
        }
        let light = choose_light(light_in, s.config.render.directional_shading);
        s.viewmodels.last_light = Some(light);
        let win = read_window(&s.config.render);
        // SAFETY: the video buffer is live during rendering; no other reference to it exists
        // while this slice is used (Doom draws nothing during this call).
        let Some(pixels) = (unsafe { video_buffer() }) else {
            return Decision::Hidden;
        };
        match render_gun(s, &view, win, light, pixels, &mut fuzzpos) {
            Some((_, flash)) => {
                Decision::Drawn(flash.and_then(|f| s.palette.clone().map(|p| (f, win, p))))
            }
            None => Decision::Hidden,
        }
    });

    match decision {
        None | Some(Decision::Vanilla) => false,
        Some(Decision::Hidden) => true,
        Some(Decision::Drawn(flash)) => {
            // SAFETY: plain write of `fuzzpos` on the engine thread (wrapped by the rasterizer).
            unsafe { crate::doom::r_draw::fuzzpos = fuzzpos as i32 };
            if let Some((req, win, pal)) = flash {
                // SAFETY: as in the borrow above; the previous slice is gone.
                if let Some(pixels) = unsafe { video_buffer() } {
                    draw_flash(req, win, &pal, pixels);
                }
            }
            true
        }
    }
}

/// `test-hooks`: renders the held weapon through the same path as the hook into a fresh
/// 320x200 buffer filled with `background`, with the current view window and `light`
/// (flash patch included). `None` when MW2 does not own the weapon or nothing is prepared.
#[cfg(feature = "test-hooks")]
pub fn render_isolated(background: u8, light: Light) -> Option<(Vec<u8>, DrawStats)> {
    let mut pixels = vec![background; DOOM_SCREEN_W * DOOM_SCREEN_H];
    let mut fuzzpos = 0usize;
    let (stats, flash, win, pal) = with_state(|s| {
        if !weapons::mw2_owns_weapon(s) {
            return None;
        }
        let view = s.sim.as_ref()?.view_state();
        let win = read_window(&s.config.render);
        let (stats, flash) = render_gun(s, &view, win, light, &mut pixels, &mut fuzzpos)?;
        Some((stats, flash, win, s.palette.clone()?))
    })??;
    if let Some(req) = flash {
        draw_flash(req, win, &pal, &mut pixels);
    }
    Some((pixels, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_follows_r_draw_psprite_order() {
        let base = LightInputs {
            invisibility: 0,
            fixed: None,
            sprite_level: 9,
        };
        assert_eq!(
            choose_light(base, true),
            Light::Sector {
                level: 9,
                directional: true
            }
        );
        let fixed = LightInputs {
            fixed: Some(32),
            ..base
        };
        assert_eq!(choose_light(fixed, true), Light::Fixed(32));
        // Invisibility wins over fixedcolormap, as in R_DrawPSprite.
        let invis = LightInputs {
            invisibility: 4 * 32 + 1,
            ..fixed
        };
        assert_eq!(choose_light(invis, true), Light::Fuzz);
        // Blinking at the end of the power: fuzz on the tics with bit 3 set.
        assert_eq!(
            choose_light(
                LightInputs {
                    invisibility: 8,
                    ..base
                },
                false
            ),
            Light::Fuzz
        );
        assert_eq!(
            choose_light(
                LightInputs {
                    invisibility: 7,
                    ..base
                },
                false
            ),
            Light::Sector {
                level: 9,
                directional: false
            }
        );
        let out_of_range = LightInputs {
            sprite_level: 40,
            ..base
        };
        assert_eq!(
            choose_light(out_of_range, false),
            Light::Sector {
                level: 31,
                directional: false
            }
        );
    }

    #[test]
    fn flash_patch_by_weapon_class() {
        let auto = weapon_iw4::FireType::FullAuto as i32;
        let single = 1;
        assert_eq!(
            flash_patch(WEAPCLASS_PISTOL, WEAPTYPE_BULLET, single, false, true),
            c"PISFA0"
        );
        assert_eq!(
            flash_patch(WEAPCLASS_SPREAD, WEAPTYPE_BULLET, single, false, false),
            c"SHTFA0"
        );
        assert_eq!(
            flash_patch(WEAPCLASS_SPREAD, WEAPTYPE_BULLET, single, false, true),
            c"SHTFB0"
        );
        assert_eq!(
            flash_patch(0, WEAPTYPE_BULLET, auto, false, false),
            c"CHGFA0"
        );
        assert_eq!(
            flash_patch(0, WEAPTYPE_BULLET, auto, false, true),
            c"CHGFB0"
        );
        assert_eq!(
            flash_patch(0, WEAPTYPE_PROJECTILE, single, true, false),
            c"MISFA0"
        );
        assert_eq!(
            flash_patch(0, WEAPTYPE_BULLET, single, false, false),
            c"SHTFB0"
        );
    }

    #[test]
    fn flash_scale_grows_when_the_muzzle_is_closer() {
        assert!((flash_scale(1.0, 24.0) - 1.0).abs() < 1e-6);
        assert!((flash_scale(1.0, 32.0) - 0.75).abs() < 1e-6);
        assert!((flash_scale(1.0, 6.0) - 2.0).abs() < 1e-6);
        assert!((flash_scale(1.0, 1000.0) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn projection_uses_the_unsheared_centre() {
        let win = Window {
            x: 0,
            y: 0,
            w: 320,
            h: 168,
            pixel_aspect: 5.0 / 6.0,
        };
        let (cx, cy) = win
            .project(Vec3::new(10.0, 0.0, 0.0), 90.0)
            .expect("in front");
        assert!((cx - 160.0).abs() < 1e-4 && (cy - 84.0).abs() < 1e-4);
        // +y is left, +z is up.
        let (lx, uy) = win
            .project(Vec3::new(10.0, 1.0, 1.0), 90.0)
            .expect("in front");
        assert!(lx < 160.0 && uy < 84.0);
        assert!(win.project(Vec3::new(0.1, 0.0, 0.0), 90.0).is_none());
    }

    #[test]
    fn pixel_aspect_is_5_6_with_aspect_correct() {
        let mut cfg = RenderCfg::default();
        assert!((pixel_aspect(&cfg) - 5.0 / 6.0).abs() < 1e-6);
        cfg.aspect_correct = false;
        set_window_size(640, 400);
        assert!((pixel_aspect(&cfg) - 1.0).abs() < 1e-6);
        set_window_size(1280, 400);
        assert!((pixel_aspect(&cfg) - 2.0).abs() < 1e-6);
    }
}
