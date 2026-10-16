// SPDX-License-Identifier: GPL-3.0-or-later
//! Freelook y-shearing, visual view kick, FOV zoom and the view tables they depend on
//! (sheet 06).
//!
//! Doom's renderer has no pitch. Looking up or down moves the horizon row (`centery`) and
//! rebuilds `yslope` around it (Heretic-style y-shearing). ADS zoom narrows the horizontal field
//! of view: `R_InitTextureMapping` reads [`fine_fov`] instead of the constant 2048 and
//! `R_ExecuteSetViewSize` reads [`focal_fixed`] / [`focal_y_fixed`] instead of the 90-degree
//! focal lengths. Every value here reduces to the exact vanilla value at pitch 0 and 90 degrees,
//! so an unsheared, unzoomed MW2-mode frame is byte-identical to a vanilla one, and vanilla
//! mode (MW2 mode off) never changes anything.
//!
//! Aim is never touched: `mo->angle` and `PlayerExt::pitch_q` are only read. The aim direction
//! projects to the unsheared centre `(centerx, viewheight / 2)`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

use log::{debug, trace};

use crate::dmw::consts::{BAM_PER_DEGREE, DOOM_HFOV, PITCH_LIMIT_DEG, PITCH_UNITS_PER_DEGREE};
use crate::dmw::ext::PLAYER_EXT;
use crate::dmw::mode;
use crate::doom::d_player::{PlayerT, MAXPLAYERS};
use crate::doom::g_game::{displayplayer, players};
use crate::doom::m_fixed::{fixed_t, FixedDiv, FixedMul, FRACBITS, FRACUNIT};
use crate::doom::r_draw::{viewheight, viewwidth};
use crate::doom::r_main::{
    centerxfrac, centery, centeryfrac, detailshift, setsizeneeded, viewangle, viewcos, viewsin,
    R_ExecuteSetViewSize,
};
use crate::doom::r_plane::yslope;
use crate::doom::tables::{self, ANGLETOFINESHIFT, FINEANGLES};
use crate::types::Boolean;

const TARGET: &str = "dmw::freelook";

/// Doom's fixed field of view in fine angles (90 degrees, `FIELDOFVIEW` in r_main.rs).
pub const VANILLA_FINE_FOV: i32 = 2048;

/// Narrowest accepted horizontal field of view (degrees).
pub const MIN_HFOV_DEG: f32 = 5.0;

/// How far the view kick may push the rendered pitch past the input clamp (degrees).
pub const KICK_PITCH_MARGIN_DEG: f32 = 5.0;

/// Number of sky texture rows Doom draws above the horizon (`skytexturemid` = 100 rows).
const SKY_ROWS_ABOVE_HORIZON: i32 = 100;

/// Cached freelook view state (sheet 06 section 5.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewParams {
    /// Rows the horizon moves; positive = horizon moves down (looking up).
    pub shear_px: i32,
    /// Horizontal field of view in degrees (90 unless zoomed).
    pub hfov_deg: f32,
    /// `viewheight / 2`: the unsheared horizon row.
    pub base_centery: i32,
    /// Rendered pitch in degrees (MW2 sign, positive = down), input pitch plus kick.
    pub pitch_deg: f32,
    /// Fine-angle field of view the texture mapping is built for ([`fine_fov`]).
    fov_key: i32,
    /// `(centery, fine fov)` the `yslope` table was built for.
    yslope_key: (i32, i32),
    /// Sky mapping used by the last sky plane: `(dc_iscale, centery)`.
    last_sky: Option<(fixed_t, i32)>,
}

impl ViewParams {
    /// Vanilla state: no shear, 90 degrees.
    pub const VANILLA: ViewParams = ViewParams {
        shear_px: 0,
        hfov_deg: DOOM_HFOV,
        base_centery: 0,
        pitch_deg: 0.0,
        fov_key: VANILLA_FINE_FOV,
        yslope_key: (0, VANILLA_FINE_FOV),
        last_sky: None,
    };

    /// `true` when the tables hold exactly what vanilla `R_ExecuteSetViewSize` builds.
    fn is_vanilla(&self) -> bool {
        self.shear_px == 0
            && self.fov_key == VANILLA_FINE_FOV
            && self.yslope_key == (self.base_centery, VANILLA_FINE_FOV)
    }

    /// Fine-angle field of view the tables are built for.
    pub fn fine_fov(&self) -> i32 {
        self.fov_key
    }

    /// `(dc_iscale, centery)` used for the last sky plane drawn in MW2 mode.
    pub fn last_sky(&self) -> Option<(fixed_t, i32)> {
        self.last_sky
    }
}

thread_local! {
    static VIEW: RefCell<ViewParams> = const { RefCell::new(ViewParams::VANILLA) };
}

/// `tan(hfov / 2)` in fixed point for the light correction; `FRACUNIT` when not zoomed.
/// Atomic because [`light_scale`] runs once per wall column and sprite.
static LIGHT_TAN: AtomicI32 = AtomicI32::new(FRACUNIT);

fn view() -> ViewParams {
    VIEW.with(|v| *v.borrow())
}

fn update_view(f: impl FnOnce(&mut ViewParams)) {
    VIEW.with(|v| f(&mut v.borrow_mut()));
}

/// Snapshot of the freelook state (for tests and diagnostics).
pub fn view_params() -> ViewParams {
    view()
}

// ---------------------------------------------------------------------------
// Pure math
// ---------------------------------------------------------------------------

/// Clamps a zoom FOV to `MIN_HFOV_DEG..=90`. `None` is 90. Returns the value and whether the
/// input was invalid (NaN or out of range).
pub fn sanitize_hfov(zoom: Option<f32>) -> (f32, bool) {
    match zoom {
        None => (DOOM_HFOV, false),
        Some(z) if z.is_nan() => (DOOM_HFOV, true),
        Some(z) => {
            let c = z.clamp(MIN_HFOV_DEG, DOOM_HFOV);
            (c, c != z)
        }
    }
}

/// Field of view in fine angles: `round(hfov / 360 * FINEANGLES)`, rounded to an even value so
/// the half angle is a whole fine angle. 2048 at 90 degrees.
pub fn fine_fov_for(hfov_deg: f32) -> i32 {
    let half = (f64::from(hfov_deg) / 360.0 * FINEANGLES as f64 / 2.0).round() as i32;
    (half * 2).clamp(2, VANILLA_FINE_FOV)
}

/// `tan(fov / 2)` in fixed point from Doom's `finetangent` table (the same entry
/// `R_InitTextureMapping` divides by). Exactly `FRACUNIT` at 90 degrees, so vanilla focal
/// lengths are reproduced bit for bit.
pub fn half_fov_tan_fixed(fine_fov: i32) -> fixed_t {
    if fine_fov == VANILLA_FINE_FOV {
        return FRACUNIT;
    }
    let fov = fine_fov.clamp(2, VANILLA_FINE_FOV) as usize;
    tables::finetangent[FINEANGLES / 4 + fov / 2]
}

/// Horizontal focal length (`projection`) for `centerxfrac` at `fine_fov`.
pub fn focal_fixed_for(center_x_frac: fixed_t, fine_fov: i32) -> fixed_t {
    if fine_fov == VANILLA_FINE_FOV {
        center_x_frac
    } else {
        FixedDiv(center_x_frac, half_fov_tan_fixed(fine_fov))
    }
}

/// Vertical focal length used by `yslope` and the shear, in fixed point: vanilla
/// `(viewwidth << detailshift) / 2 * FRACUNIT` divided by `tan(fov / 2)`.
pub fn focal_y_fixed_for(view_width: i32, detail_shift: i32, fine_fov: i32) -> fixed_t {
    let base = (view_width << detail_shift) / 2 * FRACUNIT;
    if fine_fov == VANILLA_FINE_FOV {
        base
    } else {
        FixedDiv(base, half_fov_tan_fixed(fine_fov))
    }
}

/// Rows the horizon moves for `pitch_deg` (MW2 sign, positive = down): looking down gives a
/// negative shear (horizon moves up), looking up a positive one.
pub fn shear_px_for(pitch_deg: f32, focal_y_px: f64) -> i32 {
    let s = -f64::from(pitch_deg).to_radians().tan() * focal_y_px;
    if s == 0.0 {
        0
    } else {
        s.round() as i32
    }
}

/// Rendered pitch: input pitch plus kick pitch, clamped to the limit plus the kick margin.
pub fn rendered_pitch_deg(pitch_q: i32, kick_pitch_deg: f32) -> f32 {
    let limit = PITCH_LIMIT_DEG + KICK_PITCH_MARGIN_DEG;
    let kick = if kick_pitch_deg.is_finite() {
        kick_pitch_deg
    } else {
        0.0
    };
    (pitch_q as f32 / PITCH_UNITS_PER_DEGREE + kick).clamp(-limit, limit)
}

/// Fills `out[0..view_height]` with `FixedDiv(focal_y, |((i - centery) << FRACBITS) + ½|)`.
/// With `center_y == view_height / 2` and the vanilla focal length this is exactly the table
/// `R_ExecuteSetViewSize` builds.
pub fn build_yslope(out: &mut [fixed_t], view_height: i32, center_y: i32, focal_y: fixed_t) {
    let rows = (view_height.max(0) as usize).min(out.len());
    for (i, slot) in out.iter_mut().enumerate().take(rows) {
        let dy = (((i as i32) - center_y) << FRACBITS) + FRACUNIT / 2;
        *slot = FixedDiv(focal_y, dy.wrapping_abs());
    }
}

/// Sky `dc_iscale` for a sheared, zoomed view.
///
/// The sky zooms with the world (`iscale * tan(fov / 2)`). Doom anchors sky texture row 100 at
/// the horizon and wraps rows with `& 127`, so when the horizon drops far enough that row 0 of
/// the texture would leave the top of the view, the step is reduced until texture row 0 lands
/// exactly on screen row 0 (the sky stretches instead of wrapping). Whenever vanilla would not
/// wrap, the value is the vanilla one, so the mapping is continuous in pitch and an unsheared,
/// unzoomed view keeps the vanilla sky.
pub fn sky_iscale_for(vanilla_iscale: fixed_t, center_y: i32, fov_tan: fixed_t) -> fixed_t {
    let zoomed = if fov_tan == FRACUNIT {
        vanilla_iscale
    } else {
        FixedMul(vanilla_iscale, fov_tan)
    };
    if center_y <= 0 {
        return zoomed;
    }
    let max_iscale = (SKY_ROWS_ABOVE_HORIZON * FRACUNIT) / center_y;
    zoomed.min(max_iscale)
}

/// MW2 yaw degrees (counter-clockwise, like BAM) to a BAM offset.
fn yaw_deg_to_bam(deg: f32) -> u32 {
    if !deg.is_finite() {
        return 0;
    }
    (f64::from(deg) * BAM_PER_DEGREE).round() as i64 as u32
}

// ---------------------------------------------------------------------------
// Calls from the Doom modules
// ---------------------------------------------------------------------------

/// Field of view in fine angles for `R_InitTextureMapping` (2048 unless zoomed).
pub(crate) fn fine_fov() -> i32 {
    view().fov_key
}

/// `projection` for `R_ExecuteSetViewSize`: `centerxfrac` at 90 degrees.
///
/// # Safety
/// Reads `centerxfrac`; call on the engine thread after it is set.
pub(crate) unsafe fn focal_fixed() -> fixed_t {
    // SAFETY: plain read of a renderer global on the single engine thread.
    let cx = unsafe { centerxfrac };
    focal_fixed_for(cx, fine_fov())
}

/// Numerator of the `yslope` table for `R_ExecuteSetViewSize`.
///
/// # Safety
/// Reads `viewwidth` and `detailshift`; call on the engine thread after they are set.
pub(crate) unsafe fn focal_y_fixed() -> fixed_t {
    // SAFETY: plain reads of renderer globals on the single engine thread.
    let (w, d) = unsafe { (viewwidth, detailshift) };
    focal_y_fixed_for(w, d, fine_fov())
}

/// Light-diminishing scale with the zoom taken out: `scale * tan(fov / 2)`, the identity when
/// not zoomed. Called where walls, masked midtextures and sprites turn a scale into a
/// `scalelight` index, so zooming does not brighten distant surfaces.
#[inline]
pub(crate) fn light_scale(scale: fixed_t) -> fixed_t {
    let t = LIGHT_TAN.load(Ordering::Relaxed);
    if t == FRACUNIT {
        scale
    } else {
        FixedMul(scale, t)
    }
}

/// Sky column `dc_iscale` (`R_DrawPlanes`): the vanilla value unless MW2 mode shears or zooms.
pub(crate) fn sky_iscale(vanilla_iscale: fixed_t) -> fixed_t {
    if !mode::level_active() {
        return vanilla_iscale;
    }
    // SAFETY: plain read of a renderer global on the single engine thread.
    let cy = unsafe { centery };
    let iscale = sky_iscale_for(vanilla_iscale, cy, LIGHT_TAN.load(Ordering::Relaxed));
    update_view(|v| v.last_sky = Some((iscale, cy)));
    iscale
}

/// Runs `f` with the view centre unsheared, so vanilla psprites (fist, chainsaw) do not move
/// with pitch. Restores the sheared centre afterwards.
pub(crate) fn with_unsheared_center<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: renderer globals on the single engine thread; no references are held.
    let saved = unsafe { (centery, centeryfrac) };
    // SAFETY: as above.
    let base = unsafe { viewheight / 2 };
    if saved.0 == base {
        return f();
    }
    // SAFETY: as above.
    unsafe {
        centery = base;
        centeryfrac = base << FRACBITS;
    }
    let r = f();
    // SAFETY: as above.
    unsafe {
        centery = saved.0;
        centeryfrac = saved.1;
    }
    r
}

/// Level start (called by sheet 08's `on_level_start`): vanilla view state, and a full table
/// rebuild before the first frame of the level.
pub fn reset() {
    update_view(|v| {
        let base = v.base_centery;
        *v = ViewParams::VANILLA;
        v.base_centery = base;
        v.yslope_key = (-1, -1); // forces the shear pass to rebuild after the size change
    });
    LIGHT_TAN.store(FRACUNIT, Ordering::Relaxed);
    // SAFETY: single engine thread; `D_Display` runs `R_ExecuteSetViewSize` before rendering.
    unsafe { setsizeneeded = Boolean::TRUE };
}

/// End of `R_ExecuteSetViewSize`: the tables were just rebuilt for the current FOV around
/// `viewheight / 2`; apply the current shear on top.
pub(crate) fn on_view_size_changed() {
    // SAFETY: renderer globals on the single engine thread.
    let base = unsafe { viewheight / 2 };
    update_view(|v| {
        v.base_centery = base;
        v.yslope_key = (base, v.fov_key);
    });
    let active = mode::level_active();
    // SAFETY: renderer globals on the single engine thread.
    unsafe { apply_shear(active) };
}

/// End of `R_SetupFrame`: shear, view kick and FOV zoom for `player` (the display player).
///
/// # Safety
/// `player` is the pointer `R_SetupFrame` received (an element of `players`), or null.
pub(crate) unsafe fn on_setup_frame(player: *mut PlayerT) {
    if !mode::level_active() {
        if !view().is_vanilla() {
            // SAFETY: single engine thread, between frames' table use.
            unsafe { restore_vanilla() };
        }
        return;
    }

    // SAFETY: `players` and `PLAYER_EXT` are static arrays indexed alike; the index is bounded.
    let (pitch_q, kick, zoom) = unsafe {
        let idx = player_index(player);
        let ext = &*(&raw const PLAYER_EXT)
            .cast::<crate::dmw::ext::PlayerExt>()
            .add(idx);
        (ext.pitch_q, ext.view_kick_deg, ext.zoom_hfov_deg)
    };

    let (hfov, invalid) = sanitize_hfov(zoom);
    if invalid {
        crate::dmw::log_once!(
            TARGET,
            log::Level::Warn,
            "freelook-bad-zoom",
            "zoom_hfov_deg {:?} is out of range; clamped to {hfov}",
            zoom
        );
    }
    let fine = fine_fov_for(hfov);
    // Sheet 16: mouse pitch not yet applied by a tic is shown this frame.
    let pitch_q = (pitch_q + crate::dmw::interp::frame_pitch_q()).clamp(
        -crate::dmw::ext::PITCH_LIMIT_Q,
        crate::dmw::ext::PITCH_LIMIT_Q,
    );
    let pitch = rendered_pitch_deg(pitch_q, kick[0]);
    let old_fov = view().fov_key;
    update_view(|v| {
        v.hfov_deg = hfov;
        v.pitch_deg = pitch;
        v.fov_key = fine;
    });

    if fine != old_fov {
        LIGHT_TAN.store(half_fov_tan_fixed(fine), Ordering::Relaxed);
        let t0 = Instant::now();
        // SAFETY: single engine thread; R_SetupFrame runs before any table is used this frame.
        // R_ExecuteSetViewSize rebuilds every table for `fine` and calls on_view_size_changed,
        // which applies the shear.
        unsafe { R_ExecuteSetViewSize() };
        debug!(
            target: TARGET,
            "FOV tables rebuilt for {hfov:.2} deg ({fine} fine angles) in {:.3} ms",
            t0.elapsed().as_secs_f64() * 1000.0
        );
    } else {
        // SAFETY: single engine thread.
        unsafe { apply_shear(true) };
    }

    if kick[1] != 0.0 {
        // SAFETY: renderer globals on the single engine thread; R_SetupFrame derived viewsin
        // and viewcos from viewangle just before this hook, so both are recomputed.
        unsafe {
            viewangle = viewangle.wrapping_add(yaw_deg_to_bam(kick[1]));
            let fine_angle = (viewangle >> ANGLETOFINESHIFT) as usize;
            viewsin = tables::finesine[fine_angle];
            viewcos = *tables::finecosine.0.add(fine_angle);
        }
    }

    let v = view();
    trace!(
        target: TARGET,
        "pitch {:.2} deg shear {} px hfov {:.2}",
        v.pitch_deg,
        v.shear_px,
        v.hfov_deg
    );
}

/// Index of `player` in `players`, or `displayplayer` when it is not an element.
unsafe fn player_index(player: *mut PlayerT) -> usize {
    let base = (&raw const players).cast::<PlayerT>();
    // SAFETY: only the address difference is computed; both point into or at the static.
    let off = (player as usize).wrapping_sub(base as usize) / std::mem::size_of::<PlayerT>();
    if !player.is_null() && off < MAXPLAYERS {
        off
    } else {
        // SAFETY: plain int read on the engine thread.
        (unsafe { displayplayer } as usize).min(MAXPLAYERS - 1)
    }
}

/// Moves the horizon for the current pitch and rebuilds `yslope` when needed.
///
/// # Safety
/// Writes renderer globals; engine thread only, outside a frame's table use.
unsafe fn apply_shear(active: bool) {
    let v = view();
    // SAFETY: renderer globals on the single engine thread.
    let (vw, vh, ds) = unsafe { (viewwidth, viewheight, detailshift) };
    let focal_y = focal_y_fixed_for(vw, ds, v.fov_key);
    let shear = if active {
        shear_px_for(v.pitch_deg, f64::from(focal_y) / f64::from(FRACUNIT))
    } else {
        0
    };
    let base = vh / 2;
    let cy = base + shear;
    // SAFETY: as above.
    unsafe {
        centery = cy;
        centeryfrac = cy << FRACBITS;
    }
    let key = (cy, v.fov_key);
    if key != v.yslope_key {
        // SAFETY: `yslope` is a static array only touched on the engine thread; the mutable
        // borrow ends with this call.
        unsafe {
            let table = &raw mut yslope;
            build_yslope((*table).as_mut_slice(), vh, cy, focal_y);
        }
    }
    update_view(|p| {
        p.shear_px = shear;
        p.base_centery = base;
        p.yslope_key = key;
    });
}

/// Rebuilds every view table for the vanilla 90-degree, unsheared view.
///
/// # Safety
/// Engine thread only, outside a frame's table use.
unsafe fn restore_vanilla() {
    update_view(|v| {
        v.hfov_deg = DOOM_HFOV;
        v.pitch_deg = 0.0;
        v.fov_key = VANILLA_FINE_FOV;
        v.last_sky = None;
    });
    LIGHT_TAN.store(FRACUNIT, Ordering::Relaxed);
    // SAFETY: caller contract. The rebuild runs the vanilla code path; on_view_size_changed then
    // sees MW2 mode off and leaves the horizon at viewheight / 2.
    unsafe { R_ExecuteSetViewSize() };
    debug!(target: TARGET, "view tables restored to vanilla");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T6.1: shear sign and magnitude at full width (focal 160 px).
    #[test]
    fn shear_sign_and_magnitude() {
        let focal = f64::from(focal_y_fixed_for(320, 0, VANILLA_FINE_FOV)) / 65536.0;
        assert!((focal - 160.0).abs() < 1e-9);
        let up = shear_px_for(-32.0, focal);
        let down = shear_px_for(32.0, focal);
        assert!((up - 100).abs() <= 1, "up {up}");
        assert!((down + 100).abs() <= 1, "down {down}");
        assert_eq!(shear_px_for(0.0, focal), 0);
        assert_eq!(shear_px_for(-0.0, focal), 0);
    }

    /// Low detail: the vertical focal length uses `viewwidth << detailshift`.
    #[test]
    fn low_detail_focal_matches_full_detail() {
        assert_eq!(
            focal_y_fixed_for(160, 1, VANILLA_FINE_FOV),
            focal_y_fixed_for(320, 0, VANILLA_FINE_FOV)
        );
        assert_eq!(
            focal_y_fixed_for(160, 1, 1024),
            focal_y_fixed_for(320, 0, 1024)
        );
    }

    /// T6.2: 2048 fine angles at 90 degrees; projection equals vanilla `centerxfrac` exactly.
    #[test]
    fn vanilla_fov_and_projection_are_exact() {
        assert_eq!(fine_fov_for(90.0), 2048);
        assert_eq!(fine_fov_for(45.0), 1024);
        assert_eq!(fine_fov_for(5.0) % 2, 0);
        for cx in [80, 160, 48, 144] {
            let frac = cx << FRACBITS;
            assert_eq!(focal_fixed_for(frac, fine_fov_for(90.0)), frac);
        }
        assert_eq!(half_fov_tan_fixed(2048), FRACUNIT);
    }

    #[test]
    fn zoom_scales_focal_lengths() {
        let fine = fine_fov_for(45.0);
        let p = focal_fixed_for(160 << FRACBITS, fine);
        let expect = 160.0 / (22.5f64.to_radians().tan());
        assert!((f64::from(p) / 65536.0 - expect).abs() < 0.5, "{p}");
        let tan = half_fov_tan_fixed(fine);
        assert!((f64::from(tan) / 65536.0 - 22.5f64.to_radians().tan()).abs() < 1e-3);
        // Light correction undoes the zoom on a scale value.
        LIGHT_TAN.store(tan, Ordering::Relaxed);
        let s = FixedDiv(p, 512 << FRACBITS);
        let vanilla = FixedDiv(160 << FRACBITS, 512 << FRACBITS);
        assert!(
            (light_scale(s) - vanilla).abs() <= 64,
            "{} vs {vanilla}",
            light_scale(s)
        );
        LIGHT_TAN.store(FRACUNIT, Ordering::Relaxed);
        assert_eq!(light_scale(12345), 12345);
    }

    #[test]
    fn hfov_is_sanitized() {
        assert_eq!(sanitize_hfov(None), (90.0, false));
        assert_eq!(sanitize_hfov(Some(45.0)), (45.0, false));
        assert_eq!(sanitize_hfov(Some(1.0)), (5.0, true));
        assert_eq!(sanitize_hfov(Some(120.0)), (90.0, true));
        assert_eq!(sanitize_hfov(Some(f32::NAN)), (90.0, true));
    }

    #[test]
    fn rendered_pitch_adds_kick_and_clamps() {
        assert_eq!(rendered_pitch_deg(10 * 128, 1.5), 11.5);
        assert_eq!(rendered_pitch_deg(32 * 128, 10.0), 37.0);
        assert_eq!(rendered_pitch_deg(-32 * 128, -10.0), -37.0);
        assert_eq!(rendered_pitch_deg(64, f32::NAN), 0.5);
    }

    /// The shear rebuild of `yslope` reproduces vanilla exactly when unsheared.
    #[test]
    fn yslope_matches_vanilla_formula_when_unsheared() {
        for (vw, vh, ds) in [(320, 168, 0), (320, 200, 0), (160, 168, 1), (224, 112, 0)] {
            let mut ours = [0; 200];
            build_yslope(
                &mut ours,
                vh,
                vh / 2,
                focal_y_fixed_for(vw, ds, VANILLA_FINE_FOV),
            );
            for i in 0..vh as usize {
                let mut dy = ((i as i32 - vh / 2) << FRACBITS) + FRACUNIT / 2;
                dy = dy.wrapping_abs();
                assert_eq!(ours[i], FixedDiv((vw << ds) / 2 * FRACUNIT, dy), "row {i}");
            }
        }
        // Horizon outside the view: no division by zero, values stay positive.
        let mut t = [0; 200];
        build_yslope(&mut t, 168, 250, 160 << FRACBITS);
        assert!(t[..168].iter().all(|&v| v > 0));
    }

    /// T6.6 (math): at the maximum up-pitch the sky never reaches past texture row 0.
    #[test]
    fn sky_never_wraps_at_max_up_pitch() {
        for (vw, vh, ds) in [(320, 200, 0), (320, 168, 0), (160, 168, 1), (288, 144, 0)] {
            let vanilla = (FRACUNIT * 320 / vw) >> ds;
            let focal = f64::from(focal_y_fixed_for(vw, ds, VANILLA_FINE_FOV)) / 65536.0;
            for pitch in [-37.0f32, -32.0, -20.0, 0.0, 20.0, 37.0] {
                let cy = vh / 2 + shear_px_for(pitch, focal);
                let iscale = sky_iscale_for(vanilla, cy, FRACUNIT);
                let top = 100 * FRACUNIT - cy * iscale;
                assert!(top >= 0, "{vw}x{vh} pitch {pitch}: top texel {top}");
                if pitch == 0.0 {
                    assert_eq!(iscale, vanilla, "unsheared sky is vanilla");
                }
            }
        }
    }

    #[test]
    fn yaw_kick_converts_with_sign() {
        assert_eq!(yaw_deg_to_bam(90.0), 0x4000_0000);
        assert_eq!(yaw_deg_to_bam(-90.0), 0xC000_0000);
        assert_eq!(yaw_deg_to_bam(f32::NAN), 0);
    }

    #[test]
    fn with_unsheared_center_returns_the_closure_value() {
        assert_eq!(with_unsheared_center(|| 7), 7);
    }
}
