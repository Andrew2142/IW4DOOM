// SPDX-License-Identifier: GPL-3.0-or-later
//! Pack mirrors → `xmodel_runtime` types (sheet 10 §5.2).
//!
//! Values are copied field for field; nothing is re-quantised, so a converted clip samples
//! exactly like the `AnimClip` the exporter mirrored.

use std::sync::Arc;

use anim_iw4::PartBits;
use dmw_format::Skeleton;
use dmw_format::anim::{
    ClipData, ClipSet, FrameIndicesData, KeyedQuat, KeyedVec3, RotationData, TranslationData,
    WEAPON_ANIM_SLOTS,
};
use glam::{Mat4, Quat, Vec3};
use xmodel_runtime::{
    AnimClip, ClipNotify, DObj, FrameIndices, Keyed, ModelPoseSrc, Rotation, Track, Translation,
};

use crate::anim::AnimError;

const LOG: &str = "dmw::anim";

/// One weapon-anim slot's clip and its track → DObj bone map.
#[derive(Debug, Clone)]
pub struct SlotClip {
    /// The clip (shared when several slots play the same clip).
    pub clip: Arc<AnimClip>,
    /// Track index → DObj bone index (`None` = the track drives no bone).
    pub tracks: Arc<[Option<usize>]>,
}

/// Clips indexed by weapon-anim slot (`WEAPON_ANIM_SLOTS` entries).
pub type SlotClips = Vec<Option<SlotClip>>;

fn frames(f: &FrameIndicesData) -> FrameIndices {
    match f {
        FrameIndicesData::Dense => FrameIndices::Dense,
        FrameIndicesData::Sparse(v) => FrameIndices::Sparse(v.clone()),
    }
}

fn keyed_quat(k: &KeyedQuat) -> Keyed<[f32; 4]> {
    Keyed {
        values: k.values.clone(),
        frames: frames(&k.frames),
    }
}

fn keyed_vec3(k: &KeyedVec3) -> Keyed<[f32; 3]> {
    Keyed {
        values: k.values.clone(),
        frames: frames(&k.frames),
    }
}

fn rotation(r: &RotationData) -> Rotation {
    match r {
        RotationData::Default => Rotation::Default,
        RotationData::HalfKeyed(k) => Rotation::HalfKeyed(keyed_quat(k)),
        RotationData::FullKeyed(k) => Rotation::FullKeyed(keyed_quat(k)),
        RotationData::HalfConstant(q) => Rotation::HalfConstant(*q),
        RotationData::FullConstant(q) => Rotation::FullConstant(*q),
    }
}

fn translation(t: &TranslationData) -> Translation {
    match t {
        TranslationData::Default => Translation::Default,
        TranslationData::SmallKeyed(k) => Translation::SmallKeyed(keyed_vec3(k)),
        TranslationData::FullKeyed(k) => Translation::FullKeyed(keyed_vec3(k)),
        TranslationData::Constant(v) => Translation::Constant(*v),
    }
}

/// `ClipData` → `AnimClip`, field for field.
pub fn clip_to_anim(c: &ClipData) -> AnimClip {
    AnimClip {
        name: c.name.clone(),
        framerate: c.framerate,
        numframes: c.numframes,
        looping: c.looping,
        tracks: c
            .tracks
            .iter()
            .map(|t| Track {
                name: t.name.clone(),
                rotation: rotation(&t.rotation),
                translation: translation(&t.translation),
            })
            .collect(),
        notifies: c
            .notifies
            .iter()
            .map(|n| ClipNotify {
                name: n.name.clone(),
                time: n.time,
            })
            .collect(),
        delta_translation: translation(&c.delta_translation),
    }
}

/// `Skeleton` → `DObj`.
///
/// `xmodel_runtime` does not export its `Bone`/`ModelSlot` types, only `DObj::build` from
/// XModel sources (which re-quantises bind rotations). The DObj is therefore built once with
/// the right bone count (every bone a root, identity binds) and each bone and slot is then
/// overwritten with the pack's exact values.
pub fn skeleton_to_dobj(sk: &Skeleton) -> Result<DObj, AnimError> {
    let count = sk.bones.len();
    if count > PartBits::CAPACITY {
        return Err(AnimError::TooManyBones { count });
    }
    if count == 0 || sk.models.is_empty() {
        return Err(AnimError::Skeleton(
            "skeleton has no bones or no models".into(),
        ));
    }
    let src = ModelPoseSrc {
        name: sk.models[0].name.clone(),
        num_bones: count,
        num_root_bones: count,
        scale: 1.0,
        no_scale_part_bits: [0; 6],
        bone_names: sk.bones.iter().map(|b| b.name.clone()).collect(),
        parent_list: Vec::new(),
        quats: Vec::new(),
        trans: Vec::new(),
        base_mat: vec![(Quat::IDENTITY, Vec3::ZERO); count],
        root_rest: None,
    };
    let mut dobj = DObj::build(&[(&src, None)]).map_err(|e| AnimError::Skeleton(e.to_string()))?;
    for (index, (bone, data)) in dobj.bones.iter_mut().zip(&sk.bones).enumerate() {
        let parent = data.parent.map(usize::from);
        if parent.is_some_and(|p| p >= index) || usize::from(data.model) >= sk.models.len() {
            return Err(AnimError::Skeleton(format!(
                "bone {index} ({}) has a bad parent or model",
                data.name
            )));
        }
        let [x, y, z, w] = data.bind_rotation;
        bone.name.clone_from(&data.name);
        bone.model = usize::from(data.model);
        bone.parent = parent;
        bone.bind_rotation = Quat::from_xyzw(x, y, z, w);
        bone.bind_translation = Vec3::from_array(data.bind_translation);
        bone.bind_world = Mat4::from_cols_array(&data.bind_world);
        bone.no_scale = data.no_scale;
    }
    let template = dobj.models[0].clone();
    dobj.models = sk
        .models
        .iter()
        .map(|m| {
            let mut slot = template.clone();
            slot.name.clone_from(&m.name);
            slot.base = usize::from(m.base);
            slot.bone_count = usize::from(m.bone_count);
            slot.scale = m.scale;
            slot
        })
        .collect();
    dobj.duplicates = sk
        .duplicates
        .iter()
        .map(|&(d, s)| (usize::from(d), usize::from(s)))
        .filter(|&(d, s)| d < count && s < count)
        .collect();
    Ok(dobj)
}

/// `ClipSet` → clips by weapon-anim slot. Track entries naming a bone outside the skeleton are
/// dropped (the exporter already drops unmapped tracks; this is defensive).
pub fn clip_set(set: &ClipSet, bone_count: usize) -> SlotClips {
    let converted: Vec<SlotClip> = set
        .clips
        .iter()
        .enumerate()
        .map(|(index, data)| {
            let map = set.track_maps.get(index);
            let tracks = (0..data.tracks.len())
                .map(|t| {
                    let bone = map
                        .and_then(|m| m.get(t).copied().flatten())
                        .map(usize::from);
                    match bone {
                        Some(b) if b >= bone_count => {
                            log::debug!(
                                target: LOG,
                                "clip {} track {t} maps to bone {b} of {bone_count}; dropped",
                                data.name
                            );
                            None
                        }
                        other => other,
                    }
                })
                .collect::<Vec<_>>();
            SlotClip {
                clip: Arc::new(clip_to_anim(data)),
                tracks: tracks.into(),
            }
        })
        .collect();
    (0..WEAPON_ANIM_SLOTS)
        .map(|slot| {
            let index = usize::from((*set.slots.get(slot)?)?);
            converted.get(index).cloned()
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use dmw_format::{DmwWeaponId, Pack};

    #[test]
    fn fixture_skeleton_round_trips_into_dobj() {
        let pack = Pack::fixture();
        let asset = pack.load_viewmodel(DmwWeaponId(2)).unwrap();
        let dobj = skeleton_to_dobj(&asset.skeleton).unwrap();
        assert_eq!(dobj.bone_count(), asset.skeleton.bones.len());
        for (b, d) in dobj.bones.iter().zip(&asset.skeleton.bones) {
            assert_eq!(b.name, d.name);
            assert_eq!(b.parent, d.parent.map(usize::from));
            assert_eq!(b.bind_world.to_cols_array(), d.bind_world);
        }
        assert_eq!(dobj.models.len(), asset.skeleton.models.len());
    }

    #[test]
    fn too_many_bones_is_rejected() {
        let pack = Pack::fixture();
        let asset = pack.load_viewmodel(DmwWeaponId(2)).unwrap();
        let mut sk = asset.skeleton.clone();
        let extra = sk.bones[0].clone();
        sk.bones.resize(PartBits::CAPACITY + 1, extra);
        assert!(matches!(
            skeleton_to_dobj(&sk),
            Err(AnimError::TooManyBones { count: 193 })
        ));
    }
}
