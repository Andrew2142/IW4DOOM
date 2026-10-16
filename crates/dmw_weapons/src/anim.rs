// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/render_anim/src/anim/fpv.rs (`tick_equipped_fpv_with_extra_events`,
// `observe_weap_anim_edge_on`), Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: one hand, no authority/prediction split, driven by ViewState.

//! `ViewmodelAnimator` (00 §7.2, sheet 10): clips, controller and pose for one weapon.
//!
//! Per tic, sheet 08/11 call [`ViewmodelAnimator::advance`] with the sim's `ViewState` and the
//! tic's `msec`; per frame, [`ViewmodelAnimator::pose`] (in `pose.rs`) and
//! [`ViewmodelAnimator::visibility`].

use std::cell::RefCell;
use std::sync::Arc;

use anim_iw4::PartBits;
use dmw_format::{ViewmodelAsset, WeaponFacts};
use glam::Mat4;
use weapon_iw4::WeaponState;
use xmodel_runtime::DObj;

use crate::ViewState;
use crate::controller::{
    AdsOverlayConvention, ControllerClips, EventResult, ViewmodelController, WeaponAnimSlot,
};
use crate::convert::{clip_set, skeleton_to_dobj};
use crate::events::SoundCue;
use crate::notetracks::NotetrackSounds;

const LOG: &str = "dmw::anim";

/// Slots whose absence is worth a warning (sheet 10 §7).
const COMMON_SLOTS: [WeaponAnimSlot; 5] = [
    WeaponAnimSlot::Idle,
    WeaponAnimSlot::Fire,
    WeaponAnimSlot::Reload,
    WeaponAnimSlot::Raise,
    WeaponAnimSlot::Drop,
];

/// Why a viewmodel could not be prepared. 08 demotes the slot to vanilla.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnimError {
    /// More bones than `anim_iw4::PartBits::CAPACITY` (192).
    #[error("viewmodel skeleton has {count} bones, at most 192 are supported")]
    TooManyBones {
        /// Bone count.
        count: usize,
    },
    /// The skeleton is structurally unusable.
    #[error("viewmodel skeleton: {0}")]
    Skeleton(String),
}

/// Eye-space skinning matrices (MW2 view axes: x forward, y left, z up; inches), one per DObj
/// bone, already multiplied by the inverse bind matrix and by the gun placement transform.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewmodelPose {
    /// Skin matrix per bone: bind-space position → eye space.
    pub skin: Vec<Mat4>,
    /// `tag_flash` in eye space, when the weapon has one.
    pub tag_flash: Option<Mat4>,
}

/// Which optional surface groups to draw (sheet 10 §5.8, 00 Revision 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceMask {
    /// `PartRole::Rocket` surfaces.
    pub rocket: bool,
    /// `PartRole::Knife` surfaces.
    pub knife: bool,
}

/// What a `weap_anim` change means (iw4L `WeapAnimEdge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeapAnimEdge {
    Unchanged,
    Idle,
    ForceIdle,
    Dispatch(usize),
}

/// fpv.rs:483-497.
fn observe_weap_anim_edge(previous: &mut Option<i32>, raw: i32) -> WeapAnimEdge {
    let last = previous.replace(raw);
    if last == Some(raw) {
        return WeapAnimEdge::Unchanged;
    }
    let masked = raw as u32 & weapon_iw4::WEAP_ANIM_EVENT_MASK;
    match masked {
        0 => WeapAnimEdge::Idle,
        1 => WeapAnimEdge::ForceIdle,
        _ => match weapon_iw4::slot_for_weap_anim_event(masked) {
            Some(slot) => WeapAnimEdge::Dispatch(slot),
            None => WeapAnimEdge::Idle,
        },
    }
}

/// Animation, pose and visibility of one weapon's viewmodel (00 §7.2).
#[derive(Debug)]
pub struct ViewmodelAnimator {
    clips: ControllerClips,
    pub(crate) controller: ViewmodelController,
    pub(crate) dobj: DObj,
    pub(crate) parts: PartBits,
    pub(crate) view_bone: usize,
    pub(crate) flash_bone: Option<usize>,
    pub(crate) collapsed: Vec<usize>,
    notetracks: NotetrackSounds,
    reload_time_ms: i32,
    /// `FpvFacts::reload_show_rocket_time_ms`; 0 (fixture, older packs) shows the rocket as soon
    /// as a reload has started.
    reload_show_rocket_time_ms: i32,
    last_raw: Option<i32>,
    /// Time spent in the current reload, ms (the sim does not expose `weapon_time`).
    reload_elapsed_ms: i32,
    in_reload: bool,
    /// Last finite skin, used if a pose comes out non-finite in a release build (sheet 10 §6).
    pub(crate) last_good: RefCell<Option<Vec<Mat4>>>,
}

impl ViewmodelAnimator {
    /// Converts the asset's skeleton and clips. Fails only on a skeleton with more than 192
    /// bones or a broken bone table.
    pub fn new(asset: Arc<ViewmodelAsset>, facts: &WeaponFacts) -> Result<Self, AnimError> {
        let sk = &asset.skeleton;
        let dobj = skeleton_to_dobj(sk)?;
        let count = dobj.bone_count();
        let view_bone = usize::from(sk.view_bone);
        if view_bone >= count {
            return Err(AnimError::Skeleton(format!(
                "view bone {view_bone} outside {count} bones"
            )));
        }
        let flash_bone = sk.flash_bone.map(usize::from).filter(|&b| b < count);
        let collapsed = sk
            .collapsed_bones
            .iter()
            .map(|&b| usize::from(b))
            .filter(|&b| b < count)
            .collect();
        let clips = ControllerClips {
            clips: clip_set(&asset.clips, count),
            times: facts.anim_times.clone(),
            ads_overlay: AdsOverlayConvention::from_name(&facts.fpv.ads_overlay_convention),
        };
        for slot in COMMON_SLOTS {
            if clips.clip(slot).is_none() {
                log::warn!(target: LOG, "{}: no clip for {slot:?}", facts.name);
            }
        }
        Ok(Self {
            controller: ViewmodelController::new(clips.clone()),
            clips,
            parts: dobj.all_parts(),
            dobj,
            view_bone,
            flash_bone,
            collapsed,
            notetracks: NotetrackSounds::new(&facts.sounds.notetracks),
            reload_time_ms: facts.fpv.reload_time_ms,
            reload_show_rocket_time_ms: facts.fpv.reload_show_rocket_time_ms,
            last_raw: None,
            reload_elapsed_ms: 0,
            in_reload: false,
            last_good: RefCell::new(None),
        })
    }

    /// Back to the state right after [`ViewmodelAnimator::new`] (cached animators are reset on
    /// a new raise rather than rebuilt).
    pub fn reset(&mut self) {
        self.controller = ViewmodelController::new(self.clips.clone());
        self.last_raw = None;
        self.reload_elapsed_ms = 0;
        self.in_reload = false;
        self.last_good.replace(None);
    }

    /// The controller (tests and diagnostics).
    pub fn controller(&self) -> &ViewmodelController {
        &self.controller
    }

    /// Advances one tic: dispatches the `weap_anim` edge, steps the clips by `msec`, drives
    /// the ADS clips and returns the notetrack sounds crossed, in clip-time order
    /// (fpv.rs:384-481 for one hand).
    pub fn advance(&mut self, view: &ViewState, msec: i32) -> Vec<SoundCue> {
        match observe_weap_anim_edge(&mut self.last_raw, view.weap_anim) {
            WeapAnimEdge::Unchanged => {}
            WeapAnimEdge::Idle => {
                if !self.controller.apply_idle_weap_anim(view.clip == 0) {
                    self.last_raw = None;
                }
            }
            WeapAnimEdge::ForceIdle => {
                self.controller.apply_force_idle_weap_anim(view.clip == 0);
            }
            WeapAnimEdge::Dispatch(slot) => {
                let result = self.controller.dispatch_sz_xanim_index(slot);
                log::debug!(target: LOG, "dispatch slot {slot:#x}: {result:?}");
                if let EventResult::IgnoredMissingClip(missing) = result {
                    log::debug!(target: LOG, "no clip for {missing:?}; previous animation continues");
                }
            }
        }
        let notifications = self.controller.advance(msec.max(0) as f32 / 1000.0);
        self.controller.apply_ads_overlay_frame(view.ads_frac);

        let reloading = WeaponState::from_i32(view.weaponstate).is_ok_and(|s| s.is_reload_family());
        self.reload_elapsed_ms = match (reloading, self.in_reload) {
            (true, true) => self.reload_elapsed_ms.saturating_add(msec),
            _ => 0,
        };
        self.in_reload = reloading;

        if log::log_enabled!(target: LOG, log::Level::Trace) {
            for a in self.controller.active_anims() {
                log::trace!(
                    target: LOG,
                    "node {:#x} {} weight {:.2} time {:.3}",
                    a.node,
                    a.clip.name,
                    a.weight,
                    a.time
                );
            }
        }
        self.notetracks.to_cues(&notifications)
    }

    /// Rocket surfaces while a rocket is loaded or late enough into a reload
    /// (`weapon_iw4::viewmodel_rocket_should_be_attached`); knife surfaces during melee.
    pub fn visibility(&self, view: &ViewState) -> SurfaceMask {
        let weapon_time = self.reload_time_ms.saturating_sub(self.reload_elapsed_ms);
        let rocket = weapon_iw4::viewmodel_rocket_should_be_attached(
            view.clip,
            view.weaponstate,
            weapon_time,
            self.reload_time_ms,
            self.reload_show_rocket_time_ms,
        );
        let knife = (WeaponState::MeleeInit as i32..=WeaponState::MeleeEnd as i32)
            .contains(&view.weaponstate);
        SurfaceMask { rocket, knife }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        let mut last = None;
        assert_eq!(observe_weap_anim_edge(&mut last, 0), WeapAnimEdge::Idle);
        assert_eq!(
            observe_weap_anim_edge(&mut last, 0),
            WeapAnimEdge::Unchanged
        );
        let fire = weapon_iw4::weap_anim_event::FIRE as i32;
        assert_eq!(
            observe_weap_anim_edge(&mut last, fire),
            WeapAnimEdge::Dispatch(dmw_format::anim::slot::FIRE)
        );
        assert_eq!(
            observe_weap_anim_edge(&mut last, 1),
            WeapAnimEdge::ForceIdle
        );
    }
}
