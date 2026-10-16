// SPDX-License-Identifier: GPL-3.0-or-later
//! Weapon animation clips and track maps (sheet 04 §5.6).

use std::collections::BTreeMap;

use asset_game::{FpvAssembly, WeaponAnimSlot, WeaponAnimations, WeaponRegistry};
use dmw_format::anim::{
    ClipData, ClipSet, FrameIndicesData, KeyedQuat, KeyedVec3, NotifyData, RotationData, TrackData,
    TranslationData, WEAPON_ANIM_SLOTS,
};
use xmodel_runtime::{AnimClip, FrameIndices, Rotation, Translation};

fn frames(f: &FrameIndices) -> FrameIndicesData {
    match f {
        FrameIndices::Dense => FrameIndicesData::Dense,
        FrameIndices::Sparse(v) => FrameIndicesData::Sparse(v.clone()),
    }
}

fn rotation(r: &Rotation) -> RotationData {
    match r {
        Rotation::Default => RotationData::Default,
        Rotation::HalfKeyed(k) => RotationData::HalfKeyed(KeyedQuat {
            values: k.values.clone(),
            frames: frames(&k.frames),
        }),
        Rotation::FullKeyed(k) => RotationData::FullKeyed(KeyedQuat {
            values: k.values.clone(),
            frames: frames(&k.frames),
        }),
        Rotation::HalfConstant(q) => RotationData::HalfConstant(*q),
        Rotation::FullConstant(q) => RotationData::FullConstant(*q),
    }
}

fn translation(t: &Translation) -> TranslationData {
    match t {
        Translation::Default => TranslationData::Default,
        Translation::SmallKeyed(k) => TranslationData::SmallKeyed(KeyedVec3 {
            values: k.values.clone(),
            frames: frames(&k.frames),
        }),
        Translation::FullKeyed(k) => TranslationData::FullKeyed(KeyedVec3 {
            values: k.values.clone(),
            frames: frames(&k.frames),
        }),
        Translation::Constant(v) => TranslationData::Constant(*v),
    }
}

/// `xmodel_runtime::AnimClip` → `ClipData`, field for field (03 §5.6).
pub fn clip_data(c: &AnimClip) -> ClipData {
    ClipData {
        name: c.name.clone(),
        framerate: c.framerate,
        numframes: c.numframes,
        looping: c.looping,
        tracks: c
            .tracks
            .iter()
            .map(|t| TrackData {
                name: t.name.clone(),
                rotation: rotation(&t.rotation),
                translation: translation(&t.translation),
            })
            .collect(),
        notifies: c
            .notifies
            .iter()
            .map(|n| NotifyData {
                name: n.name.clone(),
                time: n.time,
            })
            .collect(),
        delta_translation: translation(&c.delta_translation),
    }
}

/// Clip export result.
#[derive(Debug)]
pub struct ExportedClips {
    pub set: ClipSet,
    pub slots_bound: u32,
    pub dropped: Vec<String>,
}

/// Every slot clip of weapon `index`, deduplicated by xanim catalog order, with its composed
/// track → DObj bone map. A clip whose map binds no bone is dropped from its slot.
pub fn export(
    registry: &WeaponRegistry,
    index: u32,
    xanims: &asset_anim::XAnimCatalog,
    assembly: &FpvAssembly,
) -> Result<ExportedClips, String> {
    let anims = WeaponAnimations::from_registry(registry, index, xanims);
    let orders = anims.clip_orders();
    let mut set = ClipSet {
        clips: Vec::new(),
        slots: vec![None; WEAPON_ANIM_SLOTS],
        track_maps: Vec::new(),
    };
    let mut by_order: BTreeMap<usize, Option<u16>> = BTreeMap::new();
    let mut dropped = Vec::new();
    let mut slots_bound = 0;
    for (k, order) in orders.iter().enumerate() {
        let Some(slot) = WeaponAnimSlot::from_index(k) else {
            continue;
        };
        let (Some(clip), Some(order)) = (anims.clip(slot), *order) else {
            continue;
        };
        let clip_index = match by_order.get(&order) {
            Some(done) => *done,
            None => {
                let map = assembly.compose_tracks(order, clip, registry.fpv_clip_tracks());
                let result = match map {
                    Some(map) if map.iter().any(Option::is_some) => {
                        let map = map
                            .into_iter()
                            .map(|b| b.and_then(|b| u16::try_from(b).ok()))
                            .collect();
                        let i = u16::try_from(set.clips.len())
                            .map_err(|_| "more than 65535 clips".to_owned())?;
                        set.clips.push(clip_data(clip));
                        set.track_maps.push(map);
                        Some(i)
                    }
                    Some(_) => {
                        dropped.push(format!("{slot:?} '{}': track map binds no bone", clip.name));
                        None
                    }
                    None => {
                        dropped.push(format!(
                            "{slot:?} '{}': clip not bound to this assembly's meshes",
                            clip.name
                        ));
                        None
                    }
                };
                by_order.insert(order, result);
                result
            }
        };
        if let Some(ci) = clip_index {
            set.slots[k] = Some(ci);
            slots_bound += 1;
        }
    }
    Ok(ExportedClips {
        set,
        slots_bound,
        dropped,
    })
}
