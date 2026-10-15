// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/render_anim/src/anim/fpv_rig.rs (`PreparedFpvRig::pose_clips`), Copyright 2026 vladtrc,
// licensed under the Apache License 2.0.
// Modifications for DOOMMW2: hand 0 only, no left-hand blending, MW2 view axes (no Bevy fold),
// gun placement folded into the skin matrices.

//! `ViewmodelAnimator::pose` (sheet 10 §5.6): DObj pose → eye-space skin matrices.

use glam::{Mat4, Vec4};
use xmodel_runtime::AnimInstance;

use crate::ViewState;
use crate::anim::{ViewmodelAnimator, ViewmodelPose};
use crate::placement::placement_matrix;

impl ViewmodelAnimator {
    /// Eye-space pose for `view` (its `placement` positions the gun). With no clip weighted
    /// the bones take their bind pose.
    pub fn pose(&self, view: &ViewState) -> ViewmodelPose {
        let clips = self.controller.clips();
        // 1. Every weighted node with its clip and track map.
        let instances: Vec<AnimInstance<'_>> = self
            .controller
            .active_anims()
            .filter_map(|a| {
                let entry = clips.clip_at(a.node)?;
                Some(AnimInstance {
                    clip: a.clip,
                    tracks: &entry.tracks,
                    time: a.time,
                    weight: a.weight,
                    parts: None,
                })
            })
            .collect();
        // 2.
        let world = self.dobj.pose(&instances, &self.parts, Mat4::IDENTITY);
        // 3. Collapsed bones keep only their position (fpv_rig.rs:663-672).
        let mut skin = self.dobj.skin_matrices(&world);
        for &bone in &self.collapsed {
            if let (Some(s), Some(w)) = (skin.get_mut(bone), world.get(bone)) {
                *s = Mat4::from_cols(Vec4::ZERO, Vec4::ZERO, Vec4::ZERO, w.w_axis);
            }
        }
        // 4. eye_from_model = placement · world[tag_view]⁻¹ (iw4L folds into Bevy axes here).
        let view_world = world.get(self.view_bone).copied().unwrap_or(Mat4::IDENTITY);
        let eye_from_model = placement_matrix(&view.placement) * view_world.inverse();
        // 5.
        for s in &mut skin {
            *s = eye_from_model * *s;
        }
        let tag_flash = self
            .flash_bone
            .and_then(|f| world.get(f))
            .map(|w| eye_from_model * *w);

        let finite = skin.iter().all(|m| m.is_finite()) && tag_flash.is_none_or(|m| m.is_finite());
        debug_assert!(finite, "non-finite viewmodel pose");
        if finite {
            let mut last = self.last_good.borrow_mut();
            match last.as_mut() {
                Some(v) => v.clone_from(&skin),
                None => *last = Some(skin.clone()),
            }
            drop(last);
            ViewmodelPose { skin, tag_flash }
        } else {
            log::debug!(target: "dmw::anim", "non-finite pose; reusing the previous one");
            let previous = self.last_good.borrow().clone();
            ViewmodelPose {
                skin: previous.unwrap_or_else(|| vec![Mat4::IDENTITY; skin.len()]),
                tag_flash: None,
            }
        }
    }
}
