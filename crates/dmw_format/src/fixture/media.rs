// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixture textures, scope overlay and sounds, all generated procedurally.
//!
//! Nothing here uses randomness from the environment, time or floating point that could differ
//! between runs on one machine: noise comes from a fixed-seed xorshift, envelopes from integer
//! arithmetic with a decay factor computed once per sound.

use crate::fixture::facts as names;
use crate::fixture::geometry::tex;
use crate::sound::{SoundAsset, i16_to_f32};
use crate::texture::RgbaImage;

/// Texture edge length in pixels.
pub const TEXTURE_SIZE: u32 = 64;
/// Overlay size in pixels.
pub const OVERLAY_SIZE: (u32, u32) = (256, 192);
/// Sound sample rate.
pub const SOUND_RATE: u32 = 22050;

/// Per-weapon colours: (gun base, gun stripe).
const GUN_COLOURS: [([u8; 3], [u8; 3]); 5] = [
    ([70, 74, 84], [130, 136, 150]),
    ([58, 58, 58], [112, 112, 112]),
    ([84, 62, 44], [140, 110, 80]),
    ([60, 72, 60], [110, 130, 106]),
    ([92, 98, 70], [150, 156, 120]),
];

fn opaque(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

/// The four textures of fixture weapon `index`, in `tex::*` order.
pub fn textures(index: usize) -> Vec<RgbaImage> {
    let (base, stripe) = GUN_COLOURS[index];
    let n = TEXTURE_SIZE;
    let mut out = Vec::with_capacity(tex::COUNT as usize);

    // GUN: dark with a light stripe every 8 rows and a single red texel in the top-right corner,
    // which shows at u = 1, v = 0 of every face and so reveals a flipped or mirrored image.
    let mut gun = RgbaImage::new(n, n, opaque(base));
    for y in (0..n).step_by(8) {
        for x in 0..n {
            gun.set(x, y, opaque(stripe));
        }
    }
    gun.set(n - 1, 0, [255, 0, 0, 255]);
    out.push(gun);

    // HANDS: flat skin tone, darker knuckle band.
    let mut hands = RgbaImage::new(n, n, opaque([224, 172, 140]));
    for y in 20..28 {
        for x in 0..n {
            hands.set(x, y, opaque([190, 140, 112]));
        }
    }
    out.push(hands);

    // MAG: olive with a darker 1-pixel border.
    let mut mag = RgbaImage::new(n, n, opaque([85, 107, 47]));
    for i in 0..n {
        for (x, y) in [(i, 0), (i, n - 1), (0, i), (n - 1, i)] {
            mag.set(x, y, opaque([60, 76, 30]));
        }
    }
    out.push(mag);

    // FLAP: grey plate with 8x8 holes (alpha 0) on a 16-pixel grid, for alpha testing.
    let mut flap = RgbaImage::new(n, n, opaque([150, 150, 160]));
    for y in 0..n {
        for x in 0..n {
            if (4..12).contains(&(x % 16)) && (4..12).contains(&(y % 16)) {
                flap.set(x, y, [150, 150, 160, 0]);
            }
        }
    }
    out.push(flap);
    out
}

/// The fixture sniper scope overlay: opaque black outside a 96-pixel radius circle, a dark ring
/// just inside it, one-pixel crosshair lines, transparent everywhere else.
pub fn overlay() -> RgbaImage {
    let (w, h) = OVERLAY_SIZE;
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let mut img = RgbaImage::new(w, h, [0, 0, 0, 0]);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as i32 - cx, y as i32 - cy);
            let r2 = dx * dx + dy * dy;
            let px = if r2 >= 96 * 96 {
                [0, 0, 0, 255]
            } else if r2 >= 90 * 90 {
                [20, 20, 20, 255]
            } else if (dx == 0 || dy == 0) && r2 >= 8 * 8 {
                [0, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            };
            img.set(x, y, px);
        }
    }
    img
}

/// xorshift32, fixed seeds only.
struct Xorshift(u32);

impl Xorshift {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// How to synthesise one sound.
struct Recipe {
    alias: &'static str,
    /// Length in milliseconds.
    ms: u32,
    /// One-pole low-pass coefficient in 1..=255 (higher = brighter).
    brightness: i32,
    /// Exponential decay time constant in milliseconds.
    tau_ms: f64,
    /// Starting amplitude, 0..=65535 (Q16 of full scale).
    amp: u32,
    /// Noise seed (non-zero).
    seed: u32,
    /// Playback parameters for the manifest.
    distance: (f32, f32),
}

const RECIPES: [Recipe; 11] = [
    Recipe {
        alias: names::FIRE_SOUNDS[0],
        ms: 120,
        brightness: 200,
        tau_ms: 25.0,
        amp: 50_000,
        seed: 0x1234_5671,
        distance: (100.0, 3000.0),
    },
    Recipe {
        alias: names::FIRE_SOUNDS[1],
        ms: 120,
        brightness: 140,
        tau_ms: 28.0,
        amp: 52_000,
        seed: 0x1234_5672,
        distance: (100.0, 3000.0),
    },
    Recipe {
        alias: names::FIRE_SOUNDS[2],
        ms: 120,
        brightness: 70,
        tau_ms: 35.0,
        amp: 56_000,
        seed: 0x1234_5673,
        distance: (100.0, 3000.0),
    },
    Recipe {
        alias: names::FIRE_SOUNDS[3],
        ms: 120,
        brightness: 40,
        tau_ms: 40.0,
        amp: 58_000,
        seed: 0x1234_5674,
        distance: (150.0, 4000.0),
    },
    Recipe {
        alias: names::FIRE_SOUNDS[4],
        ms: 120,
        brightness: 100,
        tau_ms: 30.0,
        amp: 54_000,
        seed: 0x1234_5675,
        distance: (150.0, 4000.0),
    },
    Recipe {
        alias: names::SOUND_EMPTY,
        ms: 10,
        brightness: 255,
        tau_ms: 2.0,
        amp: 30_000,
        seed: 0x2345_6781,
        distance: (0.0, 0.0),
    },
    Recipe {
        alias: names::SOUND_MAG_OUT,
        ms: 40,
        brightness: 180,
        tau_ms: 8.0,
        amp: 34_000,
        seed: 0x2345_6782,
        distance: (0.0, 0.0),
    },
    Recipe {
        alias: names::SOUND_MAG_IN,
        ms: 55,
        brightness: 150,
        tau_ms: 10.0,
        amp: 34_000,
        seed: 0x2345_6783,
        distance: (0.0, 0.0),
    },
    Recipe {
        alias: names::SOUND_BOLT_BACK,
        ms: 90,
        brightness: 120,
        tau_ms: 15.0,
        amp: 36_000,
        seed: 0x2345_6784,
        distance: (0.0, 0.0),
    },
    Recipe {
        alias: names::SOUND_BOLT_FORWARD,
        ms: 110,
        brightness: 100,
        tau_ms: 18.0,
        amp: 36_000,
        seed: 0x2345_6785,
        distance: (0.0, 0.0),
    },
    Recipe {
        alias: names::SOUND_EXPLODE,
        ms: 300,
        brightness: 24,
        tau_ms: 90.0,
        amp: 62_000,
        seed: 0x3456_7891,
        distance: (200.0, 4000.0),
    },
];

/// One synthesised sound: 22 050 Hz mono 16-bit PCM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureSound {
    /// Alias name (also the manifest alias).
    pub alias: String,
    /// Sample rate.
    pub rate: u32,
    /// Signed 16-bit samples.
    pub pcm: Vec<i16>,
    /// `(distance_min, distance_max)` in inches as raw `f32` bits, so the struct stays `Eq`.
    pub distance_bits: (u32, u32),
}

impl FixtureSound {
    /// The decoded form `Pack::load_sound` returns (identical to decoding the written WAV).
    pub fn to_asset(&self) -> SoundAsset {
        SoundAsset {
            rate: self.rate,
            channels: 1,
            samples: self.pcm.iter().map(|&s| i16_to_f32(s)).collect(),
        }
    }

    /// `(distance_min, distance_max)` in inches.
    pub fn distance(&self) -> (f32, f32) {
        (
            f32::from_bits(self.distance_bits.0),
            f32::from_bits(self.distance_bits.1),
        )
    }
}

fn synthesise(r: &Recipe) -> FixtureSound {
    let n = (SOUND_RATE * r.ms / 1000) as usize;
    let decay =
        (65536.0 * (-1.0 / (r.tau_ms * 0.001 * f64::from(SOUND_RATE))).exp()).round() as u64;
    let mut rng = Xorshift(r.seed);
    let mut lowpass = 0i64;
    let mut envelope = u64::from(r.amp);
    let mut pcm = Vec::with_capacity(n);
    for _ in 0..n {
        let white = i64::from((rng.next() >> 16) as u16 as i16);
        lowpass += ((white - lowpass) * i64::from(r.brightness)) >> 8;
        let sample = (lowpass * envelope as i64) >> 16;
        pcm.push(sample.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16);
        envelope = (envelope * decay) >> 16;
    }
    FixtureSound {
        alias: r.alias.to_owned(),
        rate: SOUND_RATE,
        pcm,
        distance_bits: (r.distance.0.to_bits(), r.distance.1.to_bits()),
    }
}

/// All fixture sounds, in manifest order.
pub fn sounds() -> Vec<FixtureSound> {
    RECIPES.iter().map(synthesise).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_have_the_marks() {
        for w in 0..5 {
            let t = textures(w);
            assert_eq!(t.len(), tex::COUNT as usize);
            assert!(t.iter().all(|i| i.width == 64 && i.height == 64));
            assert_eq!(t[usize::from(tex::GUN)].get(63, 0), [255, 0, 0, 255]);
            assert_ne!(t[usize::from(tex::GUN)].get(62, 0), [255, 0, 0, 255]);
            let flap = &t[usize::from(tex::FLAP)];
            assert_eq!(flap.get(8, 8)[3], 0);
            assert_eq!(flap.get(0, 0)[3], 255);
        }
        // Weapons are recoloured.
        assert_ne!(textures(0)[0], textures(1)[0]);
        assert_eq!(textures(0)[1], textures(1)[1]);
    }

    #[test]
    fn overlay_is_a_ring_with_a_clear_centre() {
        let o = overlay();
        assert_eq!((o.width, o.height), (256, 192));
        assert_eq!(o.get(128, 96)[3], 0, "centre is transparent");
        assert_eq!(o.get(0, 0), [0, 0, 0, 255], "corner is black");
        assert_eq!(o.get(128 + 50, 96)[3], 255, "horizontal crosshair line");
        assert_eq!(o.get(128 + 50, 96 + 3)[3], 0, "between the lines");
        assert_eq!(o.get(128, 96 + 93)[3], 255, "ring");
    }

    #[test]
    fn sounds_are_deterministic_and_distinct() {
        let a = sounds();
        assert_eq!(a, sounds());
        assert_eq!(a.len(), 11);
        for s in &a {
            assert!(!s.pcm.is_empty());
            assert!(s.pcm.iter().any(|&x| x != 0), "{} is silent", s.alias);
        }
        assert_eq!(a[0].pcm.len(), 2646); // 120 ms at 22 050 Hz
        let lens: std::collections::BTreeSet<usize> =
            a[5..10].iter().map(|s| s.pcm.len()).collect();
        assert_eq!(lens.len(), 5, "click lengths differ");
        // Different weapons fire different noise.
        assert_ne!(a[0].pcm, a[1].pcm);
        // The burst decays.
        let head: i64 = a[1].pcm[..200].iter().map(|&x| i64::from(x).abs()).sum();
        let tail: i64 = a[1].pcm[2400..2600]
            .iter()
            .map(|&x| i64::from(x).abs())
            .sum();
        assert!(head > tail * 4, "{head} vs {tail}");
    }
}
