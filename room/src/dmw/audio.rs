// SPDX-License-Identifier: GPL-3.0-or-later
//! MW2 sound playback through the rodio mixer (sheet 12 section 5.6).
//!
//! Sheet 08's event dispatch and sheet 10's notetrack cues call [`play`]. Everything that
//! decides how a cue sounds (variant, volume, pan, pitch) is a pure function here, tested
//! without an audio device; only the last step touches [`crate::audio::AUDIO`], which is
//! `None` when no device exists (headless runs, `-nosound`), making playback a no-op.

use std::cell::Cell;

use dmw_weapons::SoundCue;
use log::debug;

use super::with_state;
use crate::doom::d_player::PlayerT;
use crate::doom::m_fixed::{FixedMul, FRACBITS, FRACUNIT};
use crate::doom::p_telept::mobj_t;
use crate::doom::tables::{finesine, ANGLETOFINESHIFT};

const TARGET: &str = "dmw::audio";

/// Doom `S_CLIPPING_DIST`: beyond this a positional sound is inaudible.
pub const CLIPPING_DIST: i32 = 1200 * FRACUNIT;
/// Doom `S_CLOSE_DIST`: within this a sound plays at full volume.
pub const CLOSE_DIST: i32 = 200 * FRACUNIT;
/// Doom `S_STEREO_SWING`.
pub const STEREO_SWING: i32 = 96 * FRACUNIT;
/// Doom `NORM_SEP`: centred.
pub const NORM_SEP: i32 = 128;
/// The pack alias of the hitmarker sound.
pub const HITMARKER_ALIAS: &str = "mp_hit_alert";

thread_local! {
    /// Audio-only xorshift32 state (sheet 01 section 5.6: never `DmwRng`, never Doom's RNG).
    static VARIANT_RNG: Cell<u32> = const { Cell::new(0x2545_F491) };
}

/// Next value of the audio-only xorshift32 generator.
fn next_random() -> u32 {
    VARIANT_RNG.with(|c| {
        let mut x = c.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        c.set(x);
        x
    })
}

/// Uniform variant index for an alias with `count` variants.
pub fn pick_variant(count: usize, random: u32) -> usize {
    if count <= 1 {
        0
    } else {
        random as usize % count
    }
}

/// Doom's `S_AdjustSoundParams`: `(volume 0..=15, separation 0..=255)` of a sound at `source`
/// as heard from `listener` (map position and facing in BAM), or `None` when inaudible.
/// `sfx_volume` is Doom's 0..=15 slider; `map8` keeps the boss-level floor of 15.
pub fn adjust_params(
    listener: (i32, i32, u32),
    source: (i32, i32),
    sfx_volume: i32,
    map8: bool,
) -> Option<(i32, i32)> {
    let adx = listener.0.wrapping_sub(source.0).wrapping_abs();
    let ady = listener.1.wrapping_sub(source.1).wrapping_abs();
    let approx = adx
        .wrapping_add(ady)
        .wrapping_sub(if adx < ady { adx } else { ady } >> 1);
    if !map8 && approx > CLIPPING_DIST {
        return None;
    }
    // SAFETY: `R_PointToAngle2` only reads the tangent tables and writes the renderer's
    // `viewx`/`viewy` scratch globals, on the engine thread.
    let mut angle =
        unsafe { crate::doom::r_main::R_PointToAngle2(listener.0, listener.1, source.0, source.1) };
    if angle > listener.2 {
        angle -= listener.2;
    } else {
        angle = angle.wrapping_add(0xffff_ffff_u32 - listener.2);
    }
    angle >>= ANGLETOFINESHIFT;
    let sine = finesine[angle as usize];
    let sep = 128 - (FixedMul(STEREO_SWING, sine) >> FRACBITS);
    let attenuator = (CLIPPING_DIST - CLOSE_DIST) >> FRACBITS;
    let vol = if approx < CLOSE_DIST {
        sfx_volume
    } else if map8 {
        let clipped = approx.min(CLIPPING_DIST);
        15 + ((sfx_volume - 15) * ((CLIPPING_DIST - clipped) >> FRACBITS)) / attenuator
    } else {
        (sfx_volume * ((CLIPPING_DIST - approx) >> FRACBITS)) / attenuator
    };
    (vol > 0).then_some((vol, sep))
}

/// Everything the mixer needs for one cue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixParams {
    /// Doom's 0..=127 volume scale.
    pub vol: i32,
    /// Doom's 0..=255 separation (128 = centred).
    pub sep: i32,
    /// Playback rate multiplier.
    pub speed: f32,
}

/// Final mix parameters: `variant_volume x mw2_volume x doom_volume / 15`, centred or panned.
/// `doom_volume` is the 0..=15 value of [`adjust_params`] (or the slider for local cues).
/// `None` when the result is silent.
pub fn mix_params(
    variant_volume: f32,
    pitch: f32,
    mw2_volume: f32,
    doom_volume: i32,
    sep: i32,
) -> Option<MixParams> {
    let gain =
        variant_volume.max(0.0) * mw2_volume.max(0.0) * (doom_volume.clamp(0, 15) as f32) / 15.0;
    let vol = (gain * 127.0).round().clamp(0.0, 127.0) as i32;
    (vol > 0).then_some(MixParams {
        vol,
        sep: sep.clamp(0, 255),
        speed: if pitch.is_finite() && pitch > 0.0 {
            pitch.clamp(0.25, 4.0)
        } else {
            1.0
        },
    })
}

/// `true` when the pack has a sound for `alias`.
pub fn has_alias(alias: &str) -> bool {
    with_state(|s| {
        s.pack
            .as_ref()
            .is_some_and(|p| p.sound_entry(alias).is_some())
    })
    .unwrap_or(false)
}

/// [`play`] for a cue with no emitter: centred, unattenuated.
pub fn play_local(cue: &SoundCue) -> bool {
    play(cue, std::ptr::null_mut())
}

/// Plays `cue`. `origin` is the emitting map object; local cues and cues from the console
/// player's own object are centred and unattenuated. Returns `true` when the pack knows the
/// alias (the sound was played, or would have been with an audio device), so callers can fall
/// back to a Doom sound otherwise.
pub(crate) fn play(cue: &SoundCue, origin: *mut mobj_t) -> bool {
    let Some((pack, mw2_volume, pool)) = with_state(|s| {
        s.pack
            .clone()
            .map(|p| (p, s.config.audio.mw2_volume, s.config.audio.channels))
    })
    .flatten() else {
        return false;
    };
    let Some(entry) = pack.sound_entry(&cue.alias) else {
        log_once!(
            TARGET,
            log::Level::Warn,
            &format!("missing-alias:{}", cue.alias),
            "sound alias '{}' is not in the pack",
            cue.alias
        );
        return false;
    };
    if entry.variants.is_empty() {
        return false;
    }
    let index = pick_variant(entry.variants.len(), next_random());
    let variant = &entry.variants[index];
    let asset = match pack.load_sound_variant(&cue.alias, index) {
        Ok(a) => a,
        Err(e) => {
            log_once!(
                TARGET,
                log::Level::Warn,
                &format!("load-failed:{}", cue.alias),
                "sound '{}' not loaded: {e}",
                cue.alias
            );
            return false;
        }
    };

    // SAFETY: engine globals read on the engine thread; the listener pointer is the console
    // player's live map object (or null before a level exists).
    let (sfx_volume, listener, map8) = unsafe {
        let cp = crate::doom::g_game::consoleplayer;
        let mo = usize::try_from(cp)
            .ok()
            .filter(|&i| i < crate::doom::d_player::MAXPLAYERS)
            .map(|i| {
                (*(&raw const crate::doom::g_game::players)
                    .cast::<PlayerT>()
                    .add(i))
                .mo
            })
            .unwrap_or(std::ptr::null_mut())
            .cast::<mobj_t>();
        (
            crate::doom::s_sound::snd_SfxVolume,
            mo,
            crate::doom::g_game::gamemap == 8,
        )
    };
    let positional = !cue.local && !origin.is_null() && origin != listener && !listener.is_null();
    let (doom_volume, sep) = if positional {
        // SAFETY: both pointers are live map objects (checked non-null above).
        let (l, o) = unsafe { (&*listener, &*origin) };
        match adjust_params((l.x, l.y, l.angle), (o.x, o.y), sfx_volume, map8) {
            Some(v) => v,
            None => return true,
        }
    } else {
        (sfx_volume, NORM_SEP)
    };
    let Some(params) = mix_params(variant.volume, variant.pitch, mw2_volume, doom_volume, sep)
    else {
        return true;
    };
    debug!(
        target: TARGET,
        "cue {} variant {index} vol {} sep {} speed {:.2}",
        cue.alias,
        params.vol,
        params.sep,
        params.speed
    );
    crate::audio::AUDIO.with_borrow_mut(|audio| {
        if let Some(a) = audio.as_mut() {
            a.start_dmw_sound(
                asset,
                &cue.alias,
                params.vol,
                params.sep,
                params.speed,
                pool,
            );
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_uniform_and_in_range() {
        assert_eq!(pick_variant(0, 5), 0);
        assert_eq!(pick_variant(1, 123), 0);
        let mut seen = [0usize; 3];
        for _ in 0..3000 {
            seen[pick_variant(3, next_random())] += 1;
        }
        assert!(seen.iter().all(|&n| n > 800), "{seen:?}");
    }

    #[test]
    fn mix_params_scales_and_mutes() {
        let p = mix_params(1.0, 1.0, 1.0, 15, NORM_SEP).unwrap();
        assert_eq!((p.vol, p.sep), (127, 128));
        let half = mix_params(0.5, 1.0, 1.0, 15, 0).unwrap();
        assert_eq!(half.vol, 64);
        assert!(
            mix_params(1.0, 1.0, 1.0, 0, 128).is_none(),
            "sfxVolume 0 mutes"
        );
        assert!(mix_params(1.0, 1.0, 0.0, 15, 128).is_none());
        assert_eq!(mix_params(1.0, 2.0, 1.0, 8, 128).unwrap().speed, 2.0);
        assert_eq!(mix_params(1.0, f32::NAN, 1.0, 8, 128).unwrap().speed, 1.0);
    }

    /// Sound straight ahead at `dist` map units, listener at the origin facing +x.
    fn ahead(dist: i32, sfx: i32) -> Option<(i32, i32)> {
        adjust_params((0, 0, 0), (dist * FRACUNIT, 0), sfx, false)
    }

    #[test]
    fn positional_params_follow_doom_s_adjust_sound_params() {
        // Close sounds play at the slider volume. Straight ahead is 129, not 128: the wrapped
        // angle lands on the last sine table entry, which is slightly negative.
        assert_eq!(ahead(100, 15), Some((15, 129)));
        assert_eq!(ahead(200, 15), Some((15, 129)));
        // Linear falloff: vol = sfx * (1200 - d) / 1000 (integer division).
        assert_eq!(ahead(700, 15), Some((7, 129)), "15 * 500 / 1000");
        assert_eq!(ahead(1000, 15), Some((3, 129)), "15 * 200 / 1000");
        assert_eq!(ahead(1199, 8), None, "8 * 1 / 1000 = 0 is inaudible");
        assert_eq!(ahead(1201, 15), None, "beyond the clipping distance");
        // Boss maps keep a floor of 15.
        assert_eq!(
            adjust_params((0, 0, 0), (5000 * FRACUNIT, 0), 8, true),
            Some((15, 129))
        );
        // Diagonal distance uses |dx| + |dy| - min / 2 = 600 + 800 - 300 = 1100.
        assert_eq!(
            adjust_params((0, 0, 0), (600 * FRACUNIT, 800 * FRACUNIT), 15, false).map(|(v, _)| v),
            Some(1),
            "15 * 100 / 1000"
        );
    }

    #[test]
    fn stereo_separation_swings_with_the_angle() {
        // Independent restatement: sep = 128 - 96 * sin(bearing), bearing measured from the
        // listener facing; the table arithmetic is within one step of it.
        for deg in [0.0f64, 30.0, 90.0, 135.0, 180.0, 225.0, 270.0, 330.0] {
            let (x, y) = (
                (deg.to_radians().cos() * 100.0 * f64::from(FRACUNIT)) as i32,
                (deg.to_radians().sin() * 100.0 * f64::from(FRACUNIT)) as i32,
            );
            let sep = adjust_params((0, 0, 0), (x, y), 15, false).unwrap().1;
            let expect = 128.0 - 96.0 * deg.to_radians().sin();
            assert!(
                (f64::from(sep) - expect).abs() <= 1.5,
                "{deg}: {sep} vs {expect}"
            );
        }
        // Hard left (+y) and hard right (-y) for a listener facing +x.
        assert_eq!(
            adjust_params((0, 0, 0), (0, 100 * FRACUNIT), 15, false)
                .unwrap()
                .1,
            33
        );
        assert_eq!(
            adjust_params((0, 0, 0), (0, -100 * FRACUNIT), 15, false)
                .unwrap()
                .1,
            224
        );
        // Listener facing 90 degrees: a source along +y is straight ahead.
        let facing = adjust_params((0, 0, 0x4000_0000), (0, 100 * FRACUNIT), 15, false)
            .unwrap()
            .1;
        assert!((127..=129).contains(&facing), "{facing}");
    }
}
