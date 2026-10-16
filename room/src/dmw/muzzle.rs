// SPDX-License-Identifier: GPL-3.0-or-later
//! Procedural MW2-style muzzle flash (sheet 11 section 6.3, `[render] muzzle_flash = "mw2"`).
//!
//! A small star of hard-edged pixels at the projected `tag_flash`: a white core, a yellow body
//! and an orange rim, with a few uneven pointed rays. The shape is a pure function of the
//! weapon class, the view scale and a per-shot variant (the magazine count), so it needs no
//! random numbers and goldens stay deterministic. No Doom lumps are used; the engine maps the
//! four [`Tone`]s onto the palette.

use weapon_iw4::{WEAPCLASS_PISTOL, WEAPCLASS_SPREAD, WEAPTYPE_PROJECTILE};

/// `WEAPCLASS_SNIPER` of iw4L's `weapClass_t` (rifle 0, sniper 1, mg 2, smg 3, spread 4, pistol
/// 5, grenade 6, rocket launcher 7).
const WEAPCLASS_SNIPER: i32 = 1;

/// Smallest flash scale (relative to the class' base size at 320 pixels wide).
pub const SCALE_MIN: f32 = 0.6;
/// Largest flash scale.
pub const SCALE_MAX: f32 = 1.3;
/// Muzzle distance (inches) at which the flash has its base size.
pub const REF_DIST: f32 = 24.0;

/// Weapon classes with their own flash shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Rifles, SMGs and machine guns.
    Rifle,
    /// Pistols and machine pistols.
    Pistol,
    /// Shotguns: a wide forward fan.
    Shotgun,
    /// Rocket launchers: a modest burst.
    Rocket,
    /// Sniper rifles: tiny.
    Sniper,
}

/// Flash shape of a weapon from its facts.
pub fn classify(weap_class: i32, weap_type: i32, projectile: bool) -> Kind {
    if weap_class == WEAPCLASS_PISTOL {
        Kind::Pistol
    } else if weap_class == WEAPCLASS_SPREAD {
        Kind::Shotgun
    } else if projectile || weap_type == WEAPTYPE_PROJECTILE {
        Kind::Rocket
    } else if weap_class == WEAPCLASS_SNIPER {
        Kind::Sniper
    } else {
        Kind::Rifle
    }
}

/// Nominal size in pixels of the flash at scale 1.0 on a 320-pixel-wide view (pixelised
/// flashes come out a few pixels smaller; the usual scale is about 0.75).
pub fn base_diameter(kind: Kind) -> f32 {
    match kind {
        Kind::Rifle => 16.0,
        Kind::Pistol => 11.5,
        Kind::Shotgun => 26.0,
        Kind::Rocket => 12.0,
        Kind::Sniper => 8.0,
    }
}

/// Flash scale for a muzzle `x_eye` inches in front of the eye; `view_scale` is
/// `scaledviewwidth / 320`. Clamped to [`SCALE_MIN`]..[`SCALE_MAX`] so the flash never grows
/// to cover the end of the gun.
pub fn scale(view_scale: f32, x_eye: f32) -> f32 {
    (view_scale * REF_DIST / x_eye).clamp(SCALE_MIN, SCALE_MAX)
}

/// Colours of the flash, hottest first; the engine picks the nearest palette entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Core.
    White,
    /// Inner body.
    Pale,
    /// Outer body.
    Yellow,
    /// Rim.
    Orange,
}

impl Tone {
    /// RGB the palette lookup starts from.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Tone::White => [255, 255, 235],
            Tone::Pale => [255, 240, 140],
            Tone::Yellow => [255, 200, 40],
            Tone::Orange => [240, 110, 16],
        }
    }
}

/// One pointed ray.
#[derive(Clone, Copy, Debug)]
struct Ray {
    /// Angle from the forward direction, radians.
    angle: f32,
    /// Length as a fraction of the flash radius.
    len: f32,
}

/// Where and how big to draw one flash.
#[derive(Clone, Copy, Debug)]
pub struct Burst {
    /// Weapon class shape.
    pub kind: Kind,
    /// Per-shot variant (the magazine count).
    pub variant: u32,
    /// [`scale`] result.
    pub scale: f32,
    /// Screen x of `tag_flash`.
    pub cx: f32,
    /// Screen y of `tag_flash`.
    pub cy: f32,
    /// Unit screen direction the barrel points in (towards the vanishing point).
    pub fwd: (f32, f32),
    /// Displayed pixel width / height.
    pub pixel_aspect: f32,
}

fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// Deterministic value in 0..=1 for stream index `i` of a variant.
fn unit(variant: u32, i: u32) -> f32 {
    let h = mix(variant
        .wrapping_mul(0x9e37_79b1)
        .wrapping_add(i.wrapping_mul(0x85eb_ca6b)));
    (h & 0xffff) as f32 / 65535.0
}

/// The rays of one flash. Consecutive shots differ: the parity of `variant` turns the rays
/// and the rest is hashed from the whole value.
fn rays(kind: Kind, variant: u32) -> Vec<Ray> {
    let odd = variant & 1 == 1;
    if kind == Kind::Shotgun {
        // Seven rays in a +-70 degree fan, the middle ones longest.
        let n = 7u32;
        return (0..n)
            .map(|k| {
                let t = k as f32 / (n - 1) as f32 * 2.0 - 1.0;
                let jitter = (unit(variant, k) - 0.5) * 0.18;
                let shorten = (unit(variant, 16 + k) - 0.5) * 0.25;
                let skew = if odd { 0.07 } else { -0.07 };
                Ray {
                    angle: t * 70.0f32.to_radians() + jitter + skew,
                    len: (1.0 - 0.25 * t.abs() + shorten).clamp(0.55, 1.0),
                }
            })
            .collect();
    }
    let n: u32 = match kind {
        Kind::Pistol | Kind::Sniper => 4,
        Kind::Rocket => 5,
        _ => 6,
    };
    let step = std::f32::consts::TAU / n as f32;
    (0..n)
        .map(|k| {
            let jitter = (unit(variant, k) - 0.5) * 0.55 * step;
            if k == 0 {
                // The forward ray is always the longest.
                return Ray {
                    angle: jitter * 0.4,
                    len: 1.0,
                };
            }
            let turn = if odd {
                0.5 * step * k as f32 / n as f32
            } else {
                0.0
            };
            Ray {
                angle: k as f32 * step + turn + jitter,
                len: if k == n / 2 {
                    0.85 + 0.15 * unit(variant, 16 + k)
                } else {
                    0.65 + 0.35 * unit(variant, 16 + k)
                },
            }
        })
        .collect()
}

/// Pixels of `burst` as `(x, y, tone)` in screen coordinates, unclipped, in raster order.
pub fn pixels(burst: &Burst) -> Vec<(i32, i32, Tone)> {
    let radius = base_diameter(burst.kind) * burst.scale * 0.5;
    if !(radius.is_finite() && radius > 0.0 && burst.cx.is_finite() && burst.cy.is_finite()) {
        return Vec::new();
    }
    let aspect = if burst.pixel_aspect.is_finite() && burst.pixel_aspect > 0.0 {
        burst.pixel_aspect
    } else {
        1.0
    };
    let (fx, fy) = burst.fwd;
    let core = radius * 0.3;
    let base_w = (radius * 0.2).max(0.9);

    // Rays in display space (x right, y down, one unit = one pixel width):
    // (direction x, direction y, length, base half-width).
    let dirs: Vec<(f32, f32, f32, f32)> = rays(burst.kind, burst.variant)
        .iter()
        .map(|r| {
            let (s, c) = r.angle.sin_cos();
            (fx * c - fy * s, fx * s + fy * c, r.len * radius, base_w)
        })
        .collect();

    let sample = |px: i32, py: i32| -> Option<f32> {
        let ox = px as f32 + 0.5 - burst.cx;
        let oy = (py as f32 + 0.5 - burst.cy) / aspect;
        let d = (ox * ox + oy * oy).sqrt();
        if d <= core {
            return Some(d);
        }
        dirs.iter()
            .any(|&(dx, dy, len, w)| {
                let along = ox * dx + oy * dy;
                if along <= 0.0 || along > len + 0.5 {
                    return false;
                }
                let perp = (ox * dy - oy * dx).abs();
                perp <= w * (1.0 - (along / len).min(1.0)) + 0.35
            })
            .then_some(d)
    };

    let reach = radius.ceil() as i32 + 1;
    let (x0, y0) = (burst.cx.floor() as i32, burst.cy.floor() as i32);
    let mut out = Vec::new();
    for py in y0 - reach..=y0 + reach {
        for px in x0 - reach..=x0 + reach {
            let Some(d) = sample(px, py) else { continue };
            let t = d / radius;
            let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|&(ax, ay)| sample(px + ax, py + ay).is_none());
            let tone = if t < 0.32 {
                Tone::White
            } else if edge && t > 0.4 {
                Tone::Orange
            } else if t < 0.6 {
                Tone::Pale
            } else {
                Tone::Yellow
            };
            out.push((px, py, tone));
        }
    }
    out
}

/// Unit screen direction from `from` towards `to`, `(0, -1)` (up) when they coincide.
pub fn direction(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len.is_finite() && len > 1e-3 {
        (dx / len, dy / len)
    } else {
        (0.0, -1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(kind: Kind, variant: u32, scale: f32) -> Burst {
        Burst {
            kind,
            variant,
            scale,
            cx: 100.0,
            cy: 80.0,
            fwd: (0.0, -1.0),
            pixel_aspect: 5.0 / 6.0,
        }
    }

    /// Bounding box (width, height) in pixels.
    fn extent(b: &Burst) -> (i32, i32) {
        let p = pixels(b);
        assert!(!p.is_empty());
        let span = |f: fn(&(i32, i32, Tone)) -> i32| {
            let lo = p.iter().map(f).min().unwrap_or(0);
            let hi = p.iter().map(f).max().unwrap_or(0);
            hi - lo + 1
        };
        (span(|p| p.0), span(|p| p.1))
    }

    #[test]
    fn classes_pick_their_shape() {
        assert_eq!(classify(WEAPCLASS_PISTOL, 0, false), Kind::Pistol);
        assert_eq!(classify(WEAPCLASS_SPREAD, 0, false), Kind::Shotgun);
        assert_eq!(classify(7, WEAPTYPE_PROJECTILE, true), Kind::Rocket);
        assert_eq!(classify(1, 0, false), Kind::Sniper);
        assert_eq!(classify(0, 0, false), Kind::Rifle);
        assert_eq!(classify(2, 0, false), Kind::Rifle);
        assert_eq!(classify(3, 0, false), Kind::Rifle);
    }

    /// Scale 0.75 is a muzzle 32 inches out on a 320-pixel view, where the fixture guns sit.
    #[test]
    fn sizes_per_class_at_the_usual_muzzle_distance() {
        for variant in 0..8 {
            let (w, h) = extent(&burst(Kind::Rifle, variant, 0.75));
            assert!(w.max(h) >= 8 && w <= 14 && h <= 14, "rifle {w}x{h}");
            let (w, h) = extent(&burst(Kind::Pistol, variant, 0.75));
            assert!(w.max(h) >= 6 && w <= 9 && h <= 9, "pistol {w}x{h}");
            let (w, h) = extent(&burst(Kind::Shotgun, variant, 0.75));
            assert!(w.max(h) >= 12 && w <= 18 && h <= 18, "shotgun {w}x{h}");
            let (w, h) = extent(&burst(Kind::Rocket, variant, 0.75));
            assert!(w.max(h) >= 6 && w <= 12 && h <= 12, "rocket {w}x{h}");
            let (w, h) = extent(&burst(Kind::Sniper, variant, 0.75));
            assert!(w <= 7 && h <= 7, "sniper {w}x{h}");
        }
    }

    #[test]
    fn classes_are_ordered_by_size() {
        let n = |k| pixels(&burst(k, 0, 1.0)).len();
        assert!(n(Kind::Sniper) < n(Kind::Pistol));
        assert!(n(Kind::Pistol) < n(Kind::Rifle));
        assert!(n(Kind::Rocket) < n(Kind::Shotgun));
    }

    #[test]
    fn scale_is_clamped_low() {
        assert!((scale(1.0, 24.0) - 1.0).abs() < 1e-6);
        assert!((scale(1.0, 32.0) - 0.75).abs() < 1e-6);
        assert!((scale(1.0, 2.0) - SCALE_MAX).abs() < 1e-6);
        assert!((scale(1.0, 1000.0) - SCALE_MIN).abs() < 1e-6);
    }

    #[test]
    fn closest_flash_stays_small() {
        let (w, h) = extent(&burst(Kind::Rifle, 3, SCALE_MAX));
        assert!(w <= 18 && h <= 18, "{w}x{h}");
        let (w, h) = extent(&burst(Kind::Shotgun, 3, SCALE_MAX));
        assert!(w <= 24 && h <= 24, "{w}x{h}");
    }

    #[test]
    fn consecutive_shots_differ_and_repeat() {
        for kind in [
            Kind::Rifle,
            Kind::Pistol,
            Kind::Shotgun,
            Kind::Rocket,
            Kind::Sniper,
        ] {
            let a = pixels(&burst(kind, 30, 0.8));
            let b = pixels(&burst(kind, 29, 0.8));
            assert_ne!(a, b, "{kind:?}");
            assert_eq!(a, pixels(&burst(kind, 30, 0.8)), "{kind:?}");
        }
    }

    #[test]
    fn core_is_white_and_rim_is_orange() {
        let p = pixels(&burst(Kind::Rifle, 0, 1.0));
        assert!(p.iter().any(|&(_, _, t)| t == Tone::Orange));
        let centre = p
            .iter()
            .find(|&&(x, y, _)| x == 100 && y == 80)
            .map(|p| p.2);
        assert_eq!(centre, Some(Tone::White));
    }

    #[test]
    fn degenerate_inputs_draw_nothing() {
        let mut b = burst(Kind::Rifle, 0, 1.0);
        b.cx = f32::NAN;
        assert!(pixels(&b).is_empty());
        assert_eq!(direction((1.0, 1.0), (1.0, 1.0)), (0.0, -1.0));
    }
}
