// SPDX-License-Identifier: GPL-3.0-or-later
//! Animation clips (sheet 03 section 5.6).
//!
//! Values are stored exactly as `xmodel_runtime` holds them after decode (already dequantised
//! `f32`), so sheet 10 can rebuild an `AnimClip` field for field and get identical sampling.
//! `Half*` and `Full*` variants differ only in how MW2 quantised them on disk; here they are
//! the same data and the distinction is kept for a lossless mirror.

use serde::{Deserialize, Serialize};

/// Number of weapon-anim slots (`weapon_iw4::WEAPON_ANIM_SLOTS`, 39).
pub const WEAPON_ANIM_SLOTS: usize = 39;

/// Slot indices into [`ClipSet::slots`], equal to `asset_game::WeaponAnimSlot::index()`.
///
/// Slot 0 (`weap_anim::ROOT`) never holds a clip. `RELOAD_QUICK` and `RELOAD_QUICK_EMPTY` are
/// `weapon_iw4::weap_anim_extra`. A unit test pins these to the iw4L values.
pub mod slot {
    pub const IDLE: usize = 0x1;
    pub const EMPTY_IDLE: usize = 0x2;
    pub const FIRE: usize = 0x3;
    pub const HOLD_FIRE: usize = 0x4;
    pub const LAST_SHOT: usize = 0x5;
    pub const RECHAMBER: usize = 0x6;
    pub const MELEE: usize = 0x7;
    pub const MELEE_CHARGE: usize = 0x8;
    pub const RELOAD: usize = 0x9;
    pub const RELOAD_EMPTY: usize = 0xA;
    pub const RELOAD_START: usize = 0xB;
    pub const RELOAD_END: usize = 0xC;
    pub const RAISE: usize = 0xD;
    pub const FIRST_RAISE: usize = 0xE;
    pub const BREACH_RAISE: usize = 0xF;
    pub const DROP: usize = 0x10;
    pub const ALT_RAISE: usize = 0x11;
    pub const ALT_DROP: usize = 0x12;
    pub const QUICK_RAISE: usize = 0x13;
    pub const QUICK_DROP: usize = 0x14;
    pub const EMPTY_RAISE: usize = 0x15;
    pub const EMPTY_DROP: usize = 0x16;
    pub const SPRINT_IN: usize = 0x17;
    pub const SPRINT_LOOP: usize = 0x18;
    pub const SPRINT_OUT: usize = 0x19;
    pub const STUNNED_START: usize = 0x1A;
    pub const STUNNED_LOOP: usize = 0x1B;
    pub const STUNNED_END: usize = 0x1C;
    pub const DETONATE: usize = 0x1D;
    pub const NIGHTVISION_WEAR: usize = 0x1E;
    pub const NIGHTVISION_REMOVE: usize = 0x1F;
    pub const ADS_FIRE: usize = 0x20;
    pub const ADS_LAST_SHOT: usize = 0x21;
    pub const ADS_RECHAMBER: usize = 0x22;
    pub const ADS_UP: usize = 0x23;
    pub const ADS_DOWN: usize = 0x24;
    pub const RELOAD_QUICK: usize = 0x25;
    pub const RELOAD_QUICK_EMPTY: usize = 0x26;
}

/// The clips of one weapon and which weapon-anim slot plays which.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipSet {
    /// Unique clips; several slots may share one.
    pub clips: Vec<ClipData>,
    /// Length [`WEAPON_ANIM_SLOTS`]; index = weapon-anim slot; value = index into `clips`.
    pub slots: Vec<Option<u16>>,
    /// Per clip: track index -> DObj bone index (`FpvAssembly::compose_tracks`).
    pub track_maps: Vec<Vec<Option<u16>>>,
}

/// Exact mirror of `xmodel_runtime::AnimClip`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipData {
    /// Clip name.
    pub name: String,
    /// Frames per second.
    pub framerate: f32,
    /// Number of frames; duration = `numframes / framerate`.
    pub numframes: u16,
    /// Wraps around instead of clamping.
    pub looping: bool,
    /// One track per animated bone.
    pub tracks: Vec<TrackData>,
    /// Notetracks, sorted by time.
    pub notifies: Vec<NotifyData>,
    /// Root motion.
    pub delta_translation: TranslationData,
}

/// One bone's rotation and translation channels.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackData {
    /// Bone name the track was authored against.
    pub name: String,
    /// Local rotation channel.
    pub rotation: RotationData,
    /// Local translation channel.
    pub translation: TranslationData,
}

/// Rotation channel (quaternions are x, y, z, w).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum RotationData {
    /// Keep the bind rotation.
    Default,
    /// Keyed, MW2 stored two components.
    HalfKeyed(KeyedQuat),
    /// Keyed, MW2 stored four components.
    FullKeyed(KeyedQuat),
    /// One constant, MW2 stored two components.
    HalfConstant([f32; 4]),
    /// One constant, MW2 stored four components.
    FullConstant([f32; 4]),
}

/// Translation channel.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TranslationData {
    /// Keep the bind translation.
    Default,
    /// Keyed, MW2 stored 8-bit components.
    SmallKeyed(KeyedVec3),
    /// Keyed, MW2 stored 16-bit components.
    FullKeyed(KeyedVec3),
    /// One constant.
    Constant([f32; 3]),
}

/// Keyed quaternions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeyedQuat {
    /// One value per key.
    pub values: Vec<[f32; 4]>,
    /// Which frame each key sits at.
    pub frames: FrameIndicesData,
}

/// Keyed vectors.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeyedVec3 {
    /// One value per key.
    pub values: Vec<[f32; 3]>,
    /// Which frame each key sits at.
    pub frames: FrameIndicesData,
}

/// Frame index of each key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum FrameIndicesData {
    /// Key `i` sits at frame `i`.
    Dense,
    /// Non-decreasing frame numbers, one per key.
    Sparse(Vec<u16>),
}

/// A notetrack (`xmodel_runtime::ClipNotify`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NotifyData {
    /// Notetrack name.
    pub name: String,
    /// Position in the clip as a fraction of its duration (0..1), as stored by `AnimClip`
    /// (iw4L's `ClipScheduler` clamps it to that range).
    pub time: f32,
}

impl ClipData {
    /// Clip duration in seconds (`0` when `framerate <= 0`), as `AnimClip::duration`.
    pub fn duration(&self) -> f32 {
        if self.framerate > 0.0 {
            f32::from(self.numframes) / self.framerate
        } else {
            0.0
        }
    }

    /// Index of the track named `name`.
    pub fn track_index(&self, name: &str) -> Option<usize> {
        self.tracks.iter().position(|t| t.name == name)
    }
}

impl ClipSet {
    /// The clip playing in weapon-anim slot `slot`, if any.
    pub fn clip_for_slot(&self, slot: usize) -> Option<&ClipData> {
        let index = usize::from((*self.slots.get(slot)?)?);
        self.clips.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot constants equal the iw4L values (`asset_iw4::size::weap_anim` and
    /// `weapon_iw4::weap_anim_extra`), which `asset_game::WeaponAnimSlot` is built from.
    #[test]
    fn slot_constants_match_iw4l() {
        assert_eq!(WEAPON_ANIM_SLOTS, weapon_iw4::WEAPON_ANIM_SLOTS);
        assert_eq!(
            slot::RELOAD_QUICK,
            weapon_iw4::weap_anim_extra::RELOAD_QUICK
        );
        assert_eq!(
            slot::RELOAD_QUICK_EMPTY,
            weapon_iw4::weap_anim_extra::RELOAD_QUICK_EMPTY
        );
        // weap_anim::ADS_DOWN is the last non-extra slot.
        assert_eq!(slot::ADS_DOWN + 1, slot::RELOAD_QUICK);
        assert_eq!(slot::RELOAD_QUICK_EMPTY + 1, WEAPON_ANIM_SLOTS);
        // Spot-check against the event-to-slot table in weapon_iw4.
        use weapon_iw4::{slot_for_weap_anim_event, weap_anim_event as ev};
        assert_eq!(slot_for_weap_anim_event(ev::FIRE), Some(slot::FIRE));
        assert_eq!(slot_for_weap_anim_event(ev::RELOAD), Some(slot::RELOAD));
        assert_eq!(
            slot_for_weap_anim_event(ev::RELOAD_EMPTY),
            Some(slot::RELOAD_EMPTY)
        );
        assert_eq!(slot_for_weap_anim_event(ev::RAISE), Some(slot::RAISE));
        assert_eq!(slot_for_weap_anim_event(ev::DROP), Some(slot::DROP));
        assert_eq!(
            slot_for_weap_anim_event(ev::SPRINT_LOOP),
            Some(slot::SPRINT_LOOP)
        );
        assert_eq!(slot_for_weap_anim_event(ev::MELEE), Some(slot::MELEE));
    }

    #[test]
    fn clip_duration_matches_anim_clip_rule() {
        let mut clip = ClipData {
            name: "c".into(),
            framerate: 30.0,
            numframes: 60,
            looping: false,
            tracks: vec![],
            notifies: vec![],
            delta_translation: TranslationData::Default,
        };
        assert!((clip.duration() - 2.0).abs() < 1e-6);
        clip.framerate = 0.0;
        assert!(clip.duration().abs() < f32::EPSILON);
    }
}
