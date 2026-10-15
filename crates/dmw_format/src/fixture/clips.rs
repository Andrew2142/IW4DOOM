// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixture animation clips: real keyed `ClipData` so that sampling and interpolation are
//! exercised, with notetracks on the reload and bolt clips.
//!
//! Conventions, matching how `xmodel_runtime` uses clips:
//! - A track value replaces the bone's local bind value (bind rotations are identity), so
//!   translation keys are absolute local positions (the bind position plus an offset).
//! - Rotation about +y by a positive angle pitches the muzzle down (MW2 pitch is positive
//!   down); recoil therefore uses negative angles.
//! - `NotifyData::time` is the fraction of the clip duration, as `AnimClip` stores it.
//!
//! The `Half*`/`Small*` variants are used in some clips and the `Full*` ones in others purely
//! to cover every variant; the stored data has no quantisation either way.

use crate::anim::{
    ClipData, ClipSet, FrameIndicesData, KeyedQuat, KeyedVec3, NotifyData, RotationData, TrackData,
    TranslationData, WEAPON_ANIM_SLOTS, slot,
};
use crate::fixture::geometry::{bind_translation, bone_index};
use crate::model::Skeleton;

/// Which clips a fixture weapon has beyond the common set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pistol,
    Rifle,
    Shotgun,
    Sniper,
    Launcher,
}

const GUN: &str = "j_gun";
const MAG: &str = "j_mag";

fn quat_y(deg: f32) -> [f32; 4] {
    let h = deg.to_radians() * 0.5;
    [0.0, h.sin(), 0.0, h.cos()]
}

fn quat_z(deg: f32) -> [f32; 4] {
    let h = deg.to_radians() * 0.5;
    [0.0, 0.0, h.sin(), h.cos()]
}

/// Hip position of the gun bone, local to `tag_view`.
fn gun_bind() -> [f32; 3] {
    bind_translation(GUN).unwrap_or([0.0; 3])
}

fn mag_bind() -> [f32; 3] {
    bind_translation(MAG).unwrap_or([0.0; 3])
}

fn add(a: [f32; 3], d: [f32; 3]) -> [f32; 3] {
    [a[0] + d[0], a[1] + d[1], a[2] + d[2]]
}

/// Keys at the given frames. Consecutive frames starting at 0 are stored as `Dense`, like
/// iw4L's `Keyed::new` does.
fn frames_of(frames: &[u16]) -> FrameIndicesData {
    if frames.iter().enumerate().all(|(i, &f)| usize::from(f) == i) {
        FrameIndicesData::Dense
    } else {
        FrameIndicesData::Sparse(frames.to_vec())
    }
}

fn rot_keys(keys: &[(u16, [f32; 4])], full: bool) -> RotationData {
    let k = KeyedQuat {
        values: keys.iter().map(|k| k.1).collect(),
        frames: frames_of(&keys.iter().map(|k| k.0).collect::<Vec<_>>()),
    };
    if full {
        RotationData::FullKeyed(k)
    } else {
        RotationData::HalfKeyed(k)
    }
}

fn trans_keys(keys: &[(u16, [f32; 3])], small: bool) -> TranslationData {
    let k = KeyedVec3 {
        values: keys.iter().map(|k| k.1).collect(),
        frames: frames_of(&keys.iter().map(|k| k.0).collect::<Vec<_>>()),
    };
    if small {
        TranslationData::SmallKeyed(k)
    } else {
        TranslationData::FullKeyed(k)
    }
}

struct Clip(ClipData);

impl Clip {
    fn new(name: &str, framerate: f32, numframes: u16, looping: bool) -> Self {
        Clip(ClipData {
            name: format!("fixture_{name}"),
            framerate,
            numframes,
            looping,
            tracks: Vec::new(),
            notifies: Vec::new(),
            delta_translation: TranslationData::Default,
        })
    }

    fn track(mut self, bone: &str, rotation: RotationData, translation: TranslationData) -> Self {
        self.0.tracks.push(TrackData {
            name: bone.to_owned(),
            rotation,
            translation,
        });
        self
    }

    fn rot(self, bone: &str, rotation: RotationData) -> Self {
        self.track(bone, rotation, TranslationData::Default)
    }

    fn trans(self, bone: &str, translation: TranslationData) -> Self {
        self.track(bone, RotationData::Default, translation)
    }

    /// Notetrack at `frame` (stored as a fraction of the clip).
    fn notify(mut self, name: &str, frame: u16) -> Self {
        let time = f32::from(frame) / f32::from(self.0.numframes);
        self.0.notifies.push(NotifyData {
            name: name.to_owned(),
            time,
        });
        self
    }
}

/// Breathing idle: 2 s loop, 1 degree of pitch on the gun, with constant wrist rotations and a
/// constant `tag_view` translation (covers the `*Constant` variants).
fn idle() -> Clip {
    let keys: Vec<(u16, [f32; 4])> = (0..=60u16)
        .map(|i| {
            (
                i,
                quat_y((std::f32::consts::TAU * f32::from(i) / 60.0).sin()),
            )
        })
        .collect();
    Clip::new("idle", 30.0, 60, true)
        .rot(GUN, rot_keys(&keys, false))
        .rot("j_wrist_ri", RotationData::HalfConstant(quat_z(2.0)))
        .rot("j_wrist_le", RotationData::FullConstant(quat_z(-2.0)))
        .trans("tag_view", TranslationData::Constant([0.0; 3]))
}

/// 0.15 s fire kick: gun pitches up 3 degrees and slides back 1.5 in, then returns.
fn fire() -> Clip {
    let g = gun_bind();
    Clip::new("fire", 40.0, 6, false)
        .track(
            GUN,
            rot_keys(
                &[(0, quat_y(0.0)), (1, quat_y(-3.0)), (6, quat_y(0.0))],
                true,
            ),
            trans_keys(&[(0, g), (1, add(g, [-1.5, 0.0, 0.0])), (6, g)], false),
        )
        // A track that stays at bind exercises `Default`/`Default`.
        .track(
            "tag_origin",
            RotationData::Default,
            TranslationData::Default,
        )
}

/// Fire that ends with the gun held back (slide lock / last round).
fn last_shot() -> Clip {
    let g = gun_bind();
    Clip::new("last_shot", 40.0, 8, false).track(
        GUN,
        rot_keys(
            &[(0, quat_y(0.0)), (1, quat_y(-3.0)), (8, quat_y(-1.5))],
            false,
        ),
        trans_keys(
            &[
                (0, g),
                (1, add(g, [-1.5, 0.0, 0.0])),
                (8, add(g, [-0.75, 0.0, 0.0])),
            ],
            false,
        ),
    )
}

/// Bolt cycle (sniper): the gun slides back 3 in and returns. Notetracks `bolt_back` and
/// `bolt_forward`.
fn rechamber() -> Clip {
    let g = gun_bind();
    Clip::new("rechamber", 30.0, 33, false)
        .trans(
            GUN,
            trans_keys(
                &[
                    (0, g),
                    (10, add(g, [-3.0, 0.0, 0.0])),
                    (20, add(g, [-3.0, 0.0, 0.0])),
                    (33, g),
                ],
                false,
            ),
        )
        .notify("bolt_back", 10)
        .notify("bolt_forward", 20)
}

/// Magazine change: the magazine drops 8 in (frame 9, 0.3 s) and returns, inserted from frame
/// 21 (0.7 s) to frame 33. Notetracks `mag_out` at 0.3 s and `mag_in` at 0.7 s of the 2.0 s clip.
fn reload() -> Clip {
    let m = mag_bind();
    let low = add(m, [0.0, 0.0, -8.0]);
    Clip::new("reload", 30.0, 60, false)
        .trans(
            MAG,
            trans_keys(&[(0, m), (9, low), (21, low), (33, m), (60, m)], false),
        )
        .rot(
            GUN,
            rot_keys(
                &[
                    (0, quat_y(0.0)),
                    (15, quat_y(10.0)),
                    (45, quat_y(10.0)),
                    (60, quat_y(0.0)),
                ],
                true,
            ),
        )
        .notify("mag_out", 9)
        .notify("mag_in", 21)
}

/// Empty reload: as [`reload`] over 2.4 s, then the charging handle (`bolt_back`,
/// `bolt_forward`).
fn reload_empty() -> Clip {
    let m = mag_bind();
    let g = gun_bind();
    let low = add(m, [0.0, 0.0, -8.0]);
    Clip::new("reload_empty", 30.0, 72, false)
        .trans(
            MAG,
            trans_keys(&[(0, m), (9, low), (21, low), (33, m), (72, m)], false),
        )
        .trans(
            GUN,
            trans_keys(
                &[
                    (0, g),
                    (48, g),
                    (54, add(g, [-2.0, 0.0, 0.0])),
                    (60, g),
                    (72, g),
                ],
                true,
            ),
        )
        .notify("mag_out", 9)
        .notify("mag_in", 21)
        .notify("bolt_back", 54)
        .notify("bolt_forward", 60)
}

/// Shotgun reload start (0.5 s): the gun tips toward the loading position.
fn reload_start() -> Clip {
    Clip::new("reload_start", 30.0, 15, false).rot(
        GUN,
        rot_keys(&[(0, quat_y(0.0)), (15, quat_y(15.0))], false),
    )
}

/// Shotgun shell insert (0.6 s), played once per shell.
fn reload_insert() -> Clip {
    let m = mag_bind();
    Clip::new("reload_insert", 30.0, 18, false)
        .rot(GUN, RotationData::FullConstant(quat_y(15.0)))
        .trans(
            MAG,
            trans_keys(&[(0, m), (9, add(m, [0.0, 0.0, -1.5])), (18, m)], true),
        )
        .notify("mag_in", 9)
}

/// Shotgun reload end (0.7 s).
fn reload_end() -> Clip {
    Clip::new("reload_end", 30.0, 21, false).rot(
        GUN,
        rot_keys(&[(0, quat_y(15.0)), (21, quat_y(0.0))], false),
    )
}

fn raise_like(name: &str, frames: u16, from_deg: f32, to_deg: f32) -> Clip {
    let g = gun_bind();
    let down = add(g, [0.0, 0.0, -8.0]);
    let (a, b) = if from_deg > to_deg {
        (down, g)
    } else {
        (g, down)
    };
    Clip::new(name, 30.0, frames, false).track(
        GUN,
        rot_keys(&[(0, quat_y(from_deg)), (frames, quat_y(to_deg))], false),
        trans_keys(&[(0, a), (frames, b)], true),
    )
}

fn sprint_in() -> Clip {
    Clip::new("sprint_in", 30.0, 8, false)
        .rot(GUN, rot_keys(&[(0, quat_z(0.0)), (8, quat_z(30.0))], false))
}

fn sprint_loop() -> Clip {
    let keys: Vec<(u16, [f32; 4])> = (0..=24u16)
        .map(|i| {
            (
                i,
                quat_z(30.0 + 2.0 * (std::f32::consts::TAU * f32::from(i) / 24.0).sin()),
            )
        })
        .collect();
    Clip::new("sprint_loop", 30.0, 24, true).rot(GUN, rot_keys(&keys, true))
}

fn sprint_out() -> Clip {
    Clip::new("sprint_out", 30.0, 8, false)
        .rot(GUN, rot_keys(&[(0, quat_z(30.0)), (8, quat_z(0.0))], false))
}

/// 0.25 s: the gun moves from the hip pose to the centred ADS pose (10, 0, -3) in `tag_view`
/// space.
fn ads_up() -> Clip {
    Clip::new("ads_up", 20.0, 5, false).trans(
        GUN,
        trans_keys(&[(0, gun_bind()), (5, [10.0, 0.0, -3.0])], false),
    )
}

fn ads_down() -> Clip {
    Clip::new("ads_down", 20.0, 5, false).trans(
        GUN,
        trans_keys(&[(0, [10.0, 0.0, -3.0]), (5, gun_bind())], false),
    )
}

/// 0.6 s lunge: the gun thrusts 6 in forward and comes back.
fn melee() -> Clip {
    let g = gun_bind();
    Clip::new("melee", 30.0, 18, false)
        .rot(
            GUN,
            rot_keys(
                &[(0, quat_y(0.0)), (5, quat_y(-8.0)), (18, quat_y(0.0))],
                false,
            ),
        )
        .trans(
            GUN,
            trans_keys(
                &[
                    (0, g),
                    (5, add(g, [6.0, 0.0, 0.0])),
                    (9, add(g, [6.0, 0.0, 0.0])),
                    (18, g),
                ],
                true,
            ),
        )
}

/// Builds the clip set of one fixture weapon and its slot table.
pub fn clip_set(kind: Kind, skeleton: &Skeleton) -> ClipSet {
    let mut clips: Vec<ClipData> = Vec::new();
    let mut slots: Vec<Option<u16>> = vec![None; WEAPON_ANIM_SLOTS];
    let mut put = |slot_index: usize, clip: Clip| {
        let index = clips.len() as u16;
        clips.push(clip.0);
        slots[slot_index] = Some(index);
        index
    };

    put(slot::IDLE, idle());
    let fire_index = put(slot::FIRE, fire());
    put(slot::RAISE, raise_like("raise", 15, 60.0, 0.0));
    put(slot::DROP, raise_like("drop", 12, 0.0, 60.0));
    put(slot::FIRST_RAISE, raise_like("first_raise", 30, 60.0, 0.0));
    put(slot::SPRINT_IN, sprint_in());
    put(slot::SPRINT_LOOP, sprint_loop());
    put(slot::SPRINT_OUT, sprint_out());
    put(slot::ADS_UP, ads_up());
    put(slot::ADS_DOWN, ads_down());
    put(slot::MELEE, melee());

    match kind {
        Kind::Pistol => {
            put(slot::RELOAD, reload());
            put(slot::RELOAD_EMPTY, reload_empty());
            put(slot::LAST_SHOT, last_shot());
        }
        Kind::Rifle => {
            put(slot::RELOAD, reload());
            put(slot::RELOAD_EMPTY, reload_empty());
            put(slot::LAST_SHOT, last_shot());
        }
        Kind::Shotgun => {
            put(slot::RELOAD, reload_insert());
            put(slot::RELOAD_START, reload_start());
            put(slot::RELOAD_END, reload_end());
        }
        Kind::Sniper => {
            put(slot::RELOAD, reload());
            put(slot::RELOAD_EMPTY, reload_empty());
            let r = put(slot::RECHAMBER, rechamber());
            slots[slot::ADS_RECHAMBER] = Some(r);
        }
        Kind::Launcher => {
            put(slot::RELOAD, reload());
        }
    }
    // The rifle shoots the same clip from the hip and from the sights (two slots, one clip).
    if kind == Kind::Rifle {
        slots[slot::ADS_FIRE] = Some(fire_index);
    }

    let track_maps = clips
        .iter()
        .map(|c| {
            c.tracks
                .iter()
                .map(|t| bone_index(&t.name).filter(|&b| usize::from(b) < skeleton.bones.len()))
                .collect()
        })
        .collect();
    ClipSet {
        clips,
        slots,
        track_maps,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::fixture::geometry;

    #[test]
    fn every_track_maps_to_a_bone() {
        let sk = geometry::skeleton();
        for kind in [
            Kind::Pistol,
            Kind::Rifle,
            Kind::Shotgun,
            Kind::Sniper,
            Kind::Launcher,
        ] {
            let set = clip_set(kind, &sk);
            assert_eq!(set.slots.len(), WEAPON_ANIM_SLOTS);
            assert_eq!(set.track_maps.len(), set.clips.len());
            for map in &set.track_maps {
                assert!(map.iter().all(Option::is_some), "{kind:?}: unmapped track");
            }
            assert!(set.slots[0].is_none());
        }
    }

    #[test]
    fn reload_notetracks_land_at_0_3_and_0_7_seconds() {
        let sk = geometry::skeleton();
        let set = clip_set(Kind::Rifle, &sk);
        let clip = set.clip_for_slot(slot::RELOAD).expect("reload clip");
        assert!((clip.duration() - 2.0).abs() < 1e-6);
        let at = |name: &str| {
            clip.notifies
                .iter()
                .find(|n| n.name == name)
                .map(|n| n.time * clip.duration())
        };
        assert!((at("mag_out").unwrap_or(-1.0) - 0.3).abs() < 1e-5);
        assert!((at("mag_in").unwrap_or(-1.0) - 0.7).abs() < 1e-5);
    }

    #[test]
    fn slot_layout_per_weapon() {
        let sk = geometry::skeleton();
        let rifle = clip_set(Kind::Rifle, &sk);
        assert_eq!(rifle.slots[slot::ADS_FIRE], rifle.slots[slot::FIRE]);
        assert!(rifle.slots[slot::QUICK_RAISE].is_none());
        let shotgun = clip_set(Kind::Shotgun, &sk);
        assert!(
            shotgun.slots[slot::RELOAD_START].is_some()
                && shotgun.slots[slot::RELOAD_END].is_some()
        );
        assert!(shotgun.slots[slot::RELOAD_EMPTY].is_none());
        let sniper = clip_set(Kind::Sniper, &sk);
        assert_eq!(
            sniper.slots[slot::RECHAMBER],
            sniper.slots[slot::ADS_RECHAMBER]
        );
        assert!(clip_set(Kind::Pistol, &sk).slots[slot::RECHAMBER].is_none());
    }

    #[test]
    fn every_enum_variant_is_used_somewhere() {
        let sk = geometry::skeleton();
        let mut seen = std::collections::BTreeSet::new();
        for kind in [
            Kind::Pistol,
            Kind::Rifle,
            Kind::Shotgun,
            Kind::Sniper,
            Kind::Launcher,
        ] {
            for c in clip_set(kind, &sk).clips {
                for t in c.tracks {
                    seen.insert(match t.rotation {
                        RotationData::Default => "r.default",
                        RotationData::HalfKeyed(_) => "r.half_keyed",
                        RotationData::FullKeyed(_) => "r.full_keyed",
                        RotationData::HalfConstant(_) => "r.half_const",
                        RotationData::FullConstant(_) => "r.full_const",
                    });
                    seen.insert(match t.translation {
                        TranslationData::Default => "t.default",
                        TranslationData::SmallKeyed(_) => "t.small_keyed",
                        TranslationData::FullKeyed(_) => "t.full_keyed",
                        TranslationData::Constant(_) => "t.const",
                    });
                }
            }
        }
        assert_eq!(seen.len(), 9, "{seen:?}");
    }
}
