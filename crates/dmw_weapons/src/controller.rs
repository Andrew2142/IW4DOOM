// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/render_anim/src/anim/viewmodel_controller.rs (`ViewmodelController`) and
// crates/asset_game/src/weapon_animations.rs (`WeaponAnimSlot`, `AdsOverlayConvention`),
// Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: one hand, perks fixed at 0, clips and timers from the pack.

//! Which clips play with which weights (port of iw4L `ViewmodelController`, sheet 10 §5.5).
//!
//! Reductions against iw4L: no left hand, perks always 0 (`perk_scaled_action_timer` returns its
//! input), and `asset_game::WeaponAnimations` is replaced by [`ControllerClips`] built from the
//! pack (`ClipSet`, `AnimTimes`, `FpvFacts::ads_overlay_convention`). The goal-weight and
//! rate constants come from `weapon_iw4` (asset_game re-exports the same items).

use std::sync::Arc;

use dmw_format::AnimTimes;
use dmw_format::anim::slot;
use weapon_iw4::{
    ACTION_GOAL_TIME_SECS, ACTIVE_GOAL_WEIGHT, IDLE_INTERRUPT_GOAL_TIME_SECS, INACTIVE_GOAL_WEIGHT,
    WEAPON_ANIM_SLOTS, playback_rate, slot_uses_native_rate,
};
use xmodel_runtime::AnimClip;

use crate::clip_scheduler::{ActiveAnim, ClipNotification, ClipScheduler};
use crate::convert::{SlotClip, SlotClips};

/// First dispatchable slot (viewmodel_controller.rs:10).
const DISPATCH_SLOT_START: usize = 1;
/// Last dispatchable non-ADS slot (viewmodel_controller.rs:11).
const DISPATCH_SLOT_END: usize = 0x22;
/// `asset_iw4::size::WEAPON_ANIM_COUNT`: first `weap_anim_extra` slot.
const WEAPON_ANIM_COUNT: usize = slot::RELOAD_QUICK;

fn dispatch_slots() -> impl Iterator<Item = usize> {
    (DISPATCH_SLOT_START..=DISPATCH_SLOT_END).chain(WEAPON_ANIM_COUNT..WEAPON_ANIM_SLOTS)
}

/// How the AdsUp/AdsDown clips are driven (asset_game `AdsOverlayConvention`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AdsOverlayConvention {
    /// AdsUp weight equals the ADS fraction (IW4 default).
    #[default]
    WeightIsFrac,
    /// AdsUp/AdsDown play at full weight while aiming / not aiming.
    PlayAdsAnim,
}

impl AdsOverlayConvention {
    /// From the pack's debug name (`"WeightIsFrac"` or `"PlayAdsAnim"`); unknown names give the
    /// IW4 default.
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("PlayAdsAnim") || name.eq_ignore_ascii_case("play_ads_anim") {
            Self::PlayAdsAnim
        } else {
            Self::WeightIsFrac
        }
    }
}

/// Weapon-anim slots that hold clips (asset_game `WeaponAnimSlot`), values = slot indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum WeaponAnimSlot {
    Idle,
    EmptyIdle,
    Fire,
    HoldFire,
    Detonate,
    LastShot,
    Rechamber,
    Melee,
    MeleeCharge,
    Reload,
    ReloadEmpty,
    ReloadStart,
    ReloadEnd,
    ReloadQuick,
    ReloadQuickEmpty,
    Raise,
    FirstRaise,
    Drop,
    AltRaise,
    AltDrop,
    QuickRaise,
    QuickDrop,
    EmptyRaise,
    EmptyDrop,
    SprintIn,
    SprintLoop,
    SprintOut,
    AdsFire,
    AdsLastShot,
    AdsRechamber,
    AdsUp,
    AdsDown,
}

impl WeaponAnimSlot {
    /// Slot index (`dmw_format::anim::slot`).
    pub const fn index(self) -> usize {
        match self {
            Self::Idle => slot::IDLE,
            Self::EmptyIdle => slot::EMPTY_IDLE,
            Self::Fire => slot::FIRE,
            Self::HoldFire => slot::HOLD_FIRE,
            Self::Detonate => slot::DETONATE,
            Self::LastShot => slot::LAST_SHOT,
            Self::Rechamber => slot::RECHAMBER,
            Self::Melee => slot::MELEE,
            Self::MeleeCharge => slot::MELEE_CHARGE,
            Self::Reload => slot::RELOAD,
            Self::ReloadEmpty => slot::RELOAD_EMPTY,
            Self::ReloadStart => slot::RELOAD_START,
            Self::ReloadEnd => slot::RELOAD_END,
            Self::ReloadQuick => slot::RELOAD_QUICK,
            Self::ReloadQuickEmpty => slot::RELOAD_QUICK_EMPTY,
            Self::Raise => slot::RAISE,
            Self::FirstRaise => slot::FIRST_RAISE,
            Self::Drop => slot::DROP,
            Self::AltRaise => slot::ALT_RAISE,
            Self::AltDrop => slot::ALT_DROP,
            Self::QuickRaise => slot::QUICK_RAISE,
            Self::QuickDrop => slot::QUICK_DROP,
            Self::EmptyRaise => slot::EMPTY_RAISE,
            Self::EmptyDrop => slot::EMPTY_DROP,
            Self::SprintIn => slot::SPRINT_IN,
            Self::SprintLoop => slot::SPRINT_LOOP,
            Self::SprintOut => slot::SPRINT_OUT,
            Self::AdsFire => slot::ADS_FIRE,
            Self::AdsLastShot => slot::ADS_LAST_SHOT,
            Self::AdsRechamber => slot::ADS_RECHAMBER,
            Self::AdsUp => slot::ADS_UP,
            Self::AdsDown => slot::ADS_DOWN,
        }
    }

    /// Every slot, in declaration order.
    pub const ALL: [Self; 32] = [
        Self::Idle,
        Self::EmptyIdle,
        Self::Fire,
        Self::HoldFire,
        Self::Detonate,
        Self::LastShot,
        Self::Rechamber,
        Self::Melee,
        Self::MeleeCharge,
        Self::Reload,
        Self::ReloadEmpty,
        Self::ReloadStart,
        Self::ReloadEnd,
        Self::ReloadQuick,
        Self::ReloadQuickEmpty,
        Self::Raise,
        Self::FirstRaise,
        Self::Drop,
        Self::AltRaise,
        Self::AltDrop,
        Self::QuickRaise,
        Self::QuickDrop,
        Self::EmptyRaise,
        Self::EmptyDrop,
        Self::SprintIn,
        Self::SprintLoop,
        Self::SprintOut,
        Self::AdsFire,
        Self::AdsLastShot,
        Self::AdsRechamber,
        Self::AdsUp,
        Self::AdsDown,
    ];

    /// The slot with index `index`; `None` for slot 0 (root) and the night-vision and stun
    /// slots, which hold no viewmodel clip.
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.index() == index)
    }
}

/// Controller state (viewmodel_controller.rs `WeaponState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ControllerState {
    Ready,
    Raising { first: bool, quick: bool },
    Firing,
    Reloading { empty: bool },
    ReloadStarting,
    ReloadEnding,
    Rechambering,
    Dropping,
    SprintIn,
    SprintLoop,
    SprintOut,
}

/// What a dispatch did (viewmodel_controller.rs `EventResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    /// The action started; the controller is now in this state.
    Started(ControllerState),
    /// Not allowed in the current state.
    IgnoredState(ControllerState),
    /// The weapon has no clip for the slot; the previous animation continues.
    IgnoredMissingClip(WeaponAnimSlot),
}

/// The pack data the controller runs on (replaces asset_game `WeaponAnimations`).
#[derive(Debug, Clone)]
pub struct ControllerClips {
    /// Clips by slot index.
    pub clips: SlotClips,
    /// Timers, ms.
    pub times: AnimTimes,
    /// ADS clip convention.
    pub ads_overlay: AdsOverlayConvention,
}

impl ControllerClips {
    /// The clip of `slot`.
    pub fn clip(&self, slot: WeaponAnimSlot) -> Option<&SlotClip> {
        self.clip_at(slot.index())
    }

    /// The clip at slot index `index`.
    pub fn clip_at(&self, index: usize) -> Option<&SlotClip> {
        self.clips.get(index).and_then(Option::as_ref)
    }
}

fn positive_ms(ms: i32) -> Option<i32> {
    (ms > 0).then_some(ms)
}

/// Reduced iw4L `ViewmodelController` for one hand.
#[derive(Debug, Clone)]
pub struct ViewmodelController {
    weapon: ControllerClips,
    tree: ClipScheduler,
    state: ControllerState,
    action: Option<WeaponAnimSlot>,
    action_remaining: Option<f32>,
    last_ads_frac: f32,
}

impl ViewmodelController {
    /// Idle at full weight; AdsDown parked at its end at rate 0 (bones animated only by ADS clips
    /// keep their lowered pose at the hip), else AdsUp at full weight for `PlayAdsAnim`
    /// (viewmodel_controller.rs:76-114).
    pub fn new(weapon: ControllerClips) -> Self {
        let mut tree = ClipScheduler::new(WEAPON_ANIM_SLOTS);
        for (node, entry) in weapon.clips.iter().enumerate() {
            if let Some(entry) = entry {
                let _ = tree.set_clip(node, Arc::clone(&entry.clip));
            }
        }
        let mut this = Self {
            weapon,
            tree,
            state: ControllerState::Ready,
            action: None,
            action_remaining: None,
            last_ads_frac: 0.0,
        };
        this.set_weight(
            WeaponAnimSlot::Idle,
            ACTIVE_GOAL_WEIGHT,
            ACTION_GOAL_TIME_SECS,
        );
        if let Some(duration) = this.duration_of(WeaponAnimSlot::AdsDown) {
            let node = WeaponAnimSlot::AdsDown.index();
            this.set_weight(WeaponAnimSlot::AdsDown, ACTIVE_GOAL_WEIGHT, 0.0);
            let _ = this.tree.set_time(node, duration);
            let _ = this.tree.set_rate(node, 0.0);
        } else if this.weapon.ads_overlay == AdsOverlayConvention::PlayAdsAnim
            && this.weapon.clip(WeaponAnimSlot::AdsUp).is_some()
        {
            this.set_weight(WeaponAnimSlot::AdsUp, ACTIVE_GOAL_WEIGHT, 0.0);
        }
        this
    }

    /// Current state.
    pub fn state(&self) -> ControllerState {
        self.state
    }

    /// The action slot playing, if any.
    pub fn action(&self) -> Option<WeaponAnimSlot> {
        self.action
    }

    /// The pack clips.
    pub fn clips(&self) -> &ControllerClips {
        &self.weapon
    }

    /// Nodes with weight > 0.
    pub fn active_anims(&self) -> impl Iterator<Item = ActiveAnim<'_>> {
        self.tree.active()
    }

    /// Weight of `slot`.
    pub fn animation_weight(&self, slot: WeaponAnimSlot) -> f32 {
        self.tree.weight(slot.index()).unwrap_or(0.0)
    }

    /// Clip time of `slot`, seconds.
    pub fn animation_time(&self, slot: WeaponAnimSlot) -> f32 {
        self.tree.time(slot.index()).unwrap_or(0.0)
    }

    fn duration_of(&self, slot: WeaponAnimSlot) -> Option<f32> {
        self.weapon.clip(slot).map(|c| c.clip.duration())
    }

    /// Starts the clip of weapon-anim slot `slot_index` (viewmodel_controller.rs:222-238).
    pub fn dispatch_sz_xanim_index(&mut self, slot_index: usize) -> EventResult {
        if slot_index == WeaponAnimSlot::AdsUp.index()
            || slot_index == WeaponAnimSlot::AdsDown.index()
        {
            return EventResult::IgnoredState(self.state);
        }
        let Some(slot) = WeaponAnimSlot::from_index(slot_index) else {
            return EventResult::IgnoredState(self.state);
        };
        if matches!(slot, WeaponAnimSlot::Idle | WeaponAnimSlot::EmptyIdle) {
            self.start_idle();
            return EventResult::Started(ControllerState::Ready);
        }
        let (state, timer) = self.state_and_timer_for_slot(slot);
        self.start_action(slot, state, timer)
    }

    /// viewmodel_controller.rs:240-382.
    fn state_and_timer_for_slot(&self, slot: WeaponAnimSlot) -> (ControllerState, Option<i32>) {
        use ControllerState as S;
        use WeaponAnimSlot as W;
        let t = &self.weapon.times;
        let raising = |first, quick| S::Raising { first, quick };
        match slot {
            W::Fire | W::AdsFire | W::LastShot | W::AdsLastShot | W::HoldFire => {
                (S::Firing, positive_ms(t.fire_time_ms))
            }
            W::Detonate => (S::Firing, positive_ms(t.detonate_time_ms)),
            W::FirstRaise => (raising(true, false), None),
            W::QuickRaise => (raising(false, true), positive_ms(t.quick_raise_time_ms)),
            W::AltRaise => (
                raising(false, false),
                positive_ms(t.alternate_raise_time_ms),
            ),
            W::EmptyRaise => (raising(false, false), None),
            W::Raise => (raising(false, false), Some(t.raise_time_ms)),
            W::Reload => (S::Reloading { empty: false }, positive_ms(t.reload_time_ms)),
            W::ReloadEmpty => (
                S::Reloading { empty: true },
                positive_ms(t.reload_empty_time_ms),
            ),
            W::ReloadQuick => (
                S::Reloading { empty: false },
                positive_ms(t.reload_quick_time_ms),
            ),
            W::ReloadQuickEmpty => (
                S::Reloading { empty: true },
                positive_ms(t.reload_quick_empty_time_ms),
            ),
            W::ReloadStart => (S::ReloadStarting, positive_ms(t.reload_start_time_ms)),
            W::ReloadEnd => (S::ReloadEnding, positive_ms(t.reload_end_time_ms)),
            W::Rechamber | W::AdsRechamber => (S::Rechambering, None),
            W::Drop => (S::Dropping, positive_ms(t.drop_time_ms)),
            W::QuickDrop => (S::Dropping, positive_ms(t.quick_drop_time_ms)),
            W::AltDrop => (S::Dropping, positive_ms(t.alternate_drop_time_ms)),
            W::EmptyDrop => (S::Dropping, None),
            W::SprintIn => (S::SprintIn, positive_ms(t.sprint_raise_time_ms)),
            W::SprintLoop => (S::SprintLoop, positive_ms(t.sprint_loop_time_ms)),
            W::SprintOut => (S::SprintOut, positive_ms(t.sprint_drop_time_ms)),
            W::Melee => (S::Ready, positive_ms(t.melee_time_ms)),
            W::MeleeCharge => (S::Ready, positive_ms(t.melee_charge_time_ms)),
            W::Idle | W::EmptyIdle | W::AdsUp | W::AdsDown => (S::Ready, None),
        }
    }

    /// Steps the scheduler by `dt_secs` and ends a finished fire cycle
    /// (viewmodel_controller.rs:384-405).
    pub fn advance(&mut self, dt_secs: f32) -> Vec<ClipNotification> {
        let dt_secs = dt_secs.max(0.0);
        let advance = self.tree.advance(dt_secs);
        if advance.discarded > 0 {
            log::debug!(target: "dmw::anim", "{} notetracks discarded", advance.discarded);
        }
        if let Some(remaining) = self.action_remaining.as_mut() {
            *remaining = (*remaining - dt_secs).max(0.0);
        }
        if matches!(self.state, ControllerState::Firing)
            && let Some(slot) = self.action
            && self.action_finished(slot)
        {
            self.finish_fire_cycle();
        }
        advance.notifications
    }

    /// `weap_anim` went to idle: start idle unless an action is still playing; returns `false`
    /// when it did not (viewmodel_controller.rs:407-413).
    pub fn apply_idle_weap_anim(&mut self, empty_mag: bool) -> bool {
        if self.dispatch_range_unfinished() {
            return false;
        }
        self.start_idle_family(empty_mag, false);
        true
    }

    /// `weap_anim` forced idle: start idle, blending out an unfinished action over
    /// `IDLE_INTERRUPT_GOAL_TIME_SECS` (viewmodel_controller.rs:415-418).
    pub fn apply_force_idle_weap_anim(&mut self, empty_mag: bool) {
        let interrupt = self.dispatch_range_unfinished();
        self.start_idle_family(empty_mag, interrupt);
    }

    /// Drives the AdsUp/AdsDown clips from the ADS fraction (viewmodel_controller.rs:460-528).
    pub fn apply_ads_overlay_frame(&mut self, f_weapon_pos_frac: f32) {
        let scrub = weapon_iw4::ads_overlay_scrub(f_weapon_pos_frac);
        self.last_ads_frac = scrub.ads_up_weight;
        if matches!(
            self.action,
            Some(WeaponAnimSlot::Melee | WeaponAnimSlot::MeleeCharge)
        ) {
            self.set_weight(WeaponAnimSlot::AdsUp, INACTIVE_GOAL_WEIGHT, 0.0);
            self.set_weight(WeaponAnimSlot::AdsDown, INACTIVE_GOAL_WEIGHT, 0.0);
            return;
        }
        const OVERLAY_SCRUB_RATE: f32 = 0.0;
        let aiming = f_weapon_pos_frac > 0.0;
        let has_ads_down = self.weapon.clip(WeaponAnimSlot::AdsDown).is_some();
        if let Some(duration) = self.duration_of(WeaponAnimSlot::AdsUp) {
            let node = WeaponAnimSlot::AdsUp.index();
            let weight = match self.weapon.ads_overlay {
                AdsOverlayConvention::WeightIsFrac => scrub.ads_up_weight,
                AdsOverlayConvention::PlayAdsAnim => {
                    if has_ads_down && !aiming {
                        INACTIVE_GOAL_WEIGHT
                    } else {
                        ACTIVE_GOAL_WEIGHT
                    }
                }
            };
            let _ = self.tree.set_rate(node, OVERLAY_SCRUB_RATE);
            let _ = self.tree.set_time(node, duration * scrub.ads_up_time_norm);
            let _ = self
                .tree
                .set_goal_weight(node, weight, ACTION_GOAL_TIME_SECS);
        }
        if let Some(duration) = self.duration_of(WeaponAnimSlot::AdsDown) {
            let node = WeaponAnimSlot::AdsDown.index();
            let weight = if self.weapon.ads_overlay == AdsOverlayConvention::PlayAdsAnim && aiming {
                INACTIVE_GOAL_WEIGHT
            } else {
                ACTIVE_GOAL_WEIGHT
            };
            let _ = self.tree.set_rate(node, OVERLAY_SCRUB_RATE);
            let _ = self
                .tree
                .set_time(node, duration * scrub.ads_down_time_norm);
            let _ = self
                .tree
                .set_goal_weight(node, weight, ACTION_GOAL_TIME_SECS);
        }
    }

    /// viewmodel_controller.rs:557-603 (perks are 0, so timers are not scaled).
    fn start_action(
        &mut self,
        slot: WeaponAnimSlot,
        state: ControllerState,
        timer_ms: Option<i32>,
    ) -> EventResult {
        let Some(clip) = self.weapon.clip(slot).map(|c| Arc::clone(&c.clip)) else {
            self.zero_dispatch_slots(ACTION_GOAL_TIME_SECS);
            self.action = None;
            self.action_remaining = None;
            self.state = state;
            return EventResult::IgnoredMissingClip(slot);
        };
        let node = slot.index();
        self.zero_dispatch_slots(ACTION_GOAL_TIME_SECS);
        let _ = self.tree.set_clip(node, Arc::clone(&clip));
        let rate = if slot_uses_native_rate(node) {
            1.0
        } else {
            timer_ms
                .filter(|time| *time > 0)
                .map(|time| playback_rate(clip_len_ms(&clip), time))
                .unwrap_or(1.0)
        };
        let _ = self.tree.set_rate(node, rate);
        let _ = self
            .tree
            .set_goal_weight(node, ACTIVE_GOAL_WEIGHT, ACTION_GOAL_TIME_SECS);
        self.action = Some(slot);
        self.action_remaining = if clip.looping {
            None
        } else {
            timer_ms
                .filter(|time| *time > 0)
                .map(|time| time as f32 / 1000.0)
        };
        self.state = state;
        EventResult::Started(state)
    }

    fn start_idle(&mut self) {
        self.start_idle_family(false, false);
    }

    /// viewmodel_controller.rs:609-626.
    fn start_idle_family(&mut self, empty: bool, interrupt: bool) {
        let slot = if empty && self.weapon.clip(WeaponAnimSlot::EmptyIdle).is_some() {
            WeaponAnimSlot::EmptyIdle
        } else {
            WeaponAnimSlot::Idle
        };
        let goal_time = if interrupt {
            IDLE_INTERRUPT_GOAL_TIME_SECS
        } else {
            ACTION_GOAL_TIME_SECS
        };
        self.action = None;
        self.action_remaining = None;
        self.state = ControllerState::Ready;
        self.zero_dispatch_slots(goal_time);
        self.set_weight(slot, ACTIVE_GOAL_WEIGHT, goal_time);
    }

    /// viewmodel_controller.rs:628-646.
    fn dispatch_range_unfinished(&self) -> bool {
        dispatch_slots().any(|node| {
            if self.tree.weight(node).unwrap_or(0.0) <= 0.0 {
                return false;
            }
            let Some(entry) = self.weapon.clip_at(node) else {
                return false;
            };
            entry.clip.looping || self.tree.time(node).unwrap_or(0.0) < entry.clip.duration()
        })
    }

    fn zero_dispatch_slots(&mut self, goal_time: f32) {
        for node in dispatch_slots() {
            let _ = self
                .tree
                .set_goal_weight(node, INACTIVE_GOAL_WEIGHT, goal_time);
            let _ = self.tree.set_rate(node, 1.0);
        }
    }

    fn finish_fire_cycle(&mut self) {
        self.action = None;
        self.action_remaining = None;
        self.state = ControllerState::Ready;
    }

    fn action_finished(&self, slot: WeaponAnimSlot) -> bool {
        if let Some(remaining) = self.action_remaining {
            return remaining <= 0.0;
        }
        let Some(entry) = self.weapon.clip(slot) else {
            return true;
        };
        !entry.clip.looping && self.animation_time(slot) >= entry.clip.duration()
    }

    fn set_weight(&mut self, slot: WeaponAnimSlot, weight: f32, goal_time: f32) {
        if self.weapon.clip(slot).is_some() {
            let _ = self.tree.set_goal_weight(slot.index(), weight, goal_time);
        }
    }
}

fn clip_len_ms(clip: &AnimClip) -> i32 {
    (clip.duration() * 1000.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_indices_round_trip_and_match_iw4l() {
        for s in WeaponAnimSlot::ALL {
            assert_eq!(WeaponAnimSlot::from_index(s.index()), Some(s));
            assert!(s.index() < WEAPON_ANIM_SLOTS);
        }
        assert_eq!(WeaponAnimSlot::from_index(0), None);
        assert_eq!(WEAPON_ANIM_COUNT, weapon_iw4::weap_anim_extra::RELOAD_QUICK);
        assert_eq!(
            AdsOverlayConvention::from_name("WeightIsFrac"),
            AdsOverlayConvention::WeightIsFrac
        );
        assert_eq!(
            AdsOverlayConvention::from_name("PlayAdsAnim"),
            AdsOverlayConvention::PlayAdsAnim
        );
    }
}
