// SPDX-License-Identifier: GPL-3.0-or-later
//! `facts.json`: everything the simulation, animation and HUD need to know about one weapon
//! (sheet 03 section 5.3). Field names are snake_case copies of the iw4L names so that the
//! exporter is a straight copy.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::remote;

/// The contents of one weapon's `facts.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WeaponFacts {
    /// MW2 weapon asset name, for example `m4_mp`.
    pub name: String,
    /// Localised display name, for example `M4A1`.
    pub display_name: String,
    /// The numbers `weapon_iw4` runs on.
    #[serde(with = "remote::WeaponCombatFactsDef")]
    pub combat: weapon_iw4::WeaponCombatFacts,
    /// Gun and view kick.
    pub kick: KickFacts,
    /// Weapon sway, hip and ADS.
    pub sway: SwayFacts,
    /// Placement inputs.
    pub fpv: FpvFacts,
    /// HUD numbers (sheet 12).
    pub hud: HudFacts,
    /// Movement speed scales.
    pub movement: MovementScales,
    /// Animation durations the viewmodel controller needs.
    pub anim_times: AnimTimes,
    /// Present only when the weapon launches a projectile.
    pub projectile: Option<ProjectileFacts>,
    /// Sound aliases per slot.
    pub sounds: WeaponSounds,
    /// `true` when the weapon has a scope overlay image.
    pub has_overlay: bool,
}

/// Every field of `asset_game::WeaponKickFacts` (`weapon_catalog.rs:364` at fc3d8ea), same names
/// and types.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct KickFacts {
    pub f_ads_view_kick_center_speed: f32,
    pub f_hip_view_kick_center_speed: f32,
    pub gun_max_pitch: f32,
    pub gun_max_yaw: f32,
    pub ads_gun_kick_reduced_kick_bullets: i32,
    pub ads_gun_kick_reduced_kick_percent: f32,
    pub ads_gun_kick_pitch_min: f32,
    pub ads_gun_kick_pitch_max: f32,
    pub ads_gun_kick_yaw_min: f32,
    pub ads_gun_kick_yaw_max: f32,
    pub ads_gun_kick_accel: f32,
    pub ads_gun_kick_speed_max: f32,
    pub ads_gun_kick_speed_decay: f32,
    pub ads_gun_kick_static_decay: f32,
    pub ads_view_kick_pitch_min: f32,
    pub ads_view_kick_pitch_max: f32,
    pub ads_view_kick_yaw_min: f32,
    pub ads_view_kick_yaw_max: f32,
    pub hip_gun_kick_reduced_kick_bullets: i32,
    pub hip_gun_kick_reduced_kick_percent: f32,
    pub hip_gun_kick_pitch_min: f32,
    pub hip_gun_kick_pitch_max: f32,
    pub hip_gun_kick_yaw_min: f32,
    pub hip_gun_kick_yaw_max: f32,
    pub hip_gun_kick_accel: f32,
    pub hip_gun_kick_speed_max: f32,
    pub hip_gun_kick_speed_decay: f32,
    pub hip_gun_kick_static_decay: f32,
    pub hip_view_kick_pitch_min: f32,
    pub hip_view_kick_pitch_max: f32,
    pub hip_view_kick_yaw_min: f32,
    pub hip_view_kick_yaw_max: f32,
}

/// Hip and ADS sway parameters.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SwayFacts {
    /// Hip-fire sway.
    #[serde(with = "remote::WeaponSwayParamsDef")]
    pub hip: weapon_iw4::WeaponSwayParams,
    /// Aim-down-sights sway.
    #[serde(with = "remote::WeaponSwayParamsDef")]
    pub ads: weapon_iw4::WeaponSwayParams,
}

/// Inputs to `weapon_iw4::weapon_placement_assemble` that come from the weapon
/// (`WeaponFpvFacts`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FpvFacts {
    pub ads_aim_pitch: f32,
    pub ads_bob_factor: f32,
    pub ads_view_bob_mult: f32,
    pub ads_zoom_fov: f32,
    pub ads_zoom_in_frac: f32,
    pub ads_zoom_out_frac: f32,
    pub ads_overlay_width: f32,
    pub ads_overlay_height: f32,
    pub aim_down_sight: bool,
    pub can_hold_breath: bool,
    pub ducked_ofs: [f32; 3],
    pub prone_ofs: [f32; 3],
    #[serde(with = "remote::WeaponIdleInputsDef")]
    pub idle: weapon_iw4::WeaponIdleInputs,
    #[serde(with = "remote::WeaponMovementOfsInputsDef")]
    pub movement: weapon_iw4::WeaponMovementOfsInputs,
    pub overlay_reticle: i32,
    pub reload_time_ms: i32,
    #[serde(with = "remote::scope_zoom")]
    pub scope_zoom: weapon_iw4::ScopeZoom,
    /// Debug name of `asset_game::AdsOverlayConvention`: `"WeightIsFrac"` or `"PlayAdsAnim"`.
    pub ads_overlay_convention: String,
}

/// The subset of `WeaponHudFacts` used by sheet 12.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HudFacts {
    pub ads_crosshair_in_frac: f32,
    pub ads_crosshair_out_frac: f32,
    pub hip_reticle_side_pos: f32,
    pub i_reticle_min_ofs: i32,
    pub i_reticle_side_size: i32,
    pub clip_only: bool,
    pub clip_size: i32,
    pub ammo_counter_clip: i32,
}

/// Movement scales from the weapon definition.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MovementScales {
    pub move_speed_scale: f32,
    pub ads_move_speed_scale: f32,
    pub sprint_duration_scale: f32,
}

/// Animation times from `asset_game::WeaponAnimations` (`weapon_animations.rs`), all in ms.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AnimTimes {
    pub fire_time_ms: i32,
    pub detonate_time_ms: i32,
    pub melee_time_ms: i32,
    pub melee_charge_time_ms: i32,
    pub raise_time_ms: i32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub drop_time_ms: i32,
    pub quick_drop_time_ms: i32,
    pub quick_raise_time_ms: i32,
    pub sprint_raise_time_ms: i32,
    pub sprint_loop_time_ms: i32,
    pub sprint_drop_time_ms: i32,
    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    pub reload_start_time_ms: i32,
    pub reload_end_time_ms: i32,
    pub reload_quick_time_ms: i32,
    pub reload_quick_empty_time_ms: i32,
}

/// From `weapon_iw4::EquipmentRuntimeFacts` and `WeaponEventFacts`; present only when
/// `projectile_speed > 0`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectileFacts {
    pub projectile_speed: i32,
    pub projectile_speed_up: i32,
    pub projectile_speed_forward: i32,
    pub explosion_radius: i32,
    pub explosion_radius_min: i32,
    pub explosion_inner_damage: i32,
    pub explosion_outer_damage: i32,
    pub impact_damage: i32,
    pub proj_impact_explode: bool,
    pub missile_guidance: i32,
    pub ignition_delay_ms: i32,
}

macro_rules! weapon_sounds {
    ($($field:ident),+ $(,)?) => {
        /// One `Option<String>` (sound alias) per `asset_game::WeaponSoundSlot`, same snake_case
        /// names, plus the loop and stop slots of the sound alias table, plus `notetracks`
        /// (notetrack name -> sound alias, from `WeaponRegistry::notetrack_sound_aliases_of`).
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
        #[serde(default)]
        pub struct WeaponSounds {
            $(pub $field: Option<String>,)+
            /// Notetrack name (compared case-insensitively by sheet 10) -> sound alias.
            pub notetracks: BTreeMap<String, String>,
        }

        impl WeaponSounds {
            /// Names of the per-slot fields, in declaration order.
            pub const SLOT_NAMES: &'static [&'static str] = &[$(stringify!($field)),+];

            /// Every slot as `(name, alias)`; `None` when the weapon has no sound for the slot.
            pub fn slots(&self) -> Vec<(&'static str, Option<&str>)> {
                vec![$((stringify!($field), self.$field.as_deref())),+]
            }
        }
    };
}

weapon_sounds! {
    fire, fire_player, empty_fire, empty_fire_player,
    melee_swipe, melee_swipe_player, melee_hit, melee_miss,
    pickup, pickup_player, ammo_pickup, ammo_pickup_player,
    detonate, detonate_player, pullback, pullback_player,
    reload, reload_player, reload_empty, reload_empty_player,
    reload_start, reload_start_player, reload_end, reload_end_player,
    rechamber, rechamber_player, alt_switch, alt_switch_player,
    raise, raise_player, first_raise, first_raise_player,
    putaway, putaway_player,
    proj_explosion, projectile, proj_ignition_sound,
    fire_last, fire_last_player,
    fire_loop, fire_loop_player, fire_stop, fire_stop_player,
}

impl WeaponSounds {
    /// Every alias this weapon can play (slots and notetracks), sorted and deduplicated.
    /// Empty strings are ignored.
    pub fn referenced_aliases(&self) -> Vec<&str> {
        let mut set: BTreeSet<&str> = self
            .slots()
            .into_iter()
            .filter_map(|(_, alias)| alias)
            .collect();
        set.extend(self.notetracks.values().map(String::as_str));
        set.remove("");
        set.into_iter().collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn weapon_sounds_default_and_aliases() {
        let mut s = WeaponSounds::default();
        assert!(s.referenced_aliases().is_empty());
        assert_eq!(WeaponSounds::SLOT_NAMES.len(), 43);
        s.fire = Some("b".into());
        s.fire_player = Some("b".into());
        s.reload = Some("a".into());
        s.raise = Some(String::new());
        s.notetracks.insert("mag_out".into(), "c".into());
        assert_eq!(s.referenced_aliases(), ["a", "b", "c"]);
    }

    #[test]
    fn weapon_sounds_json_accepts_missing_keys() {
        let s: WeaponSounds = serde_json::from_str(r#"{"fire":"x"}"#).unwrap();
        assert_eq!(s.fire.as_deref(), Some("x"));
        assert!(s.fire_last.is_none());
    }

    #[test]
    fn slot_names_are_unique() {
        let set: BTreeSet<_> = WeaponSounds::SLOT_NAMES.iter().collect();
        assert_eq!(set.len(), WeaponSounds::SLOT_NAMES.len());
    }
}
