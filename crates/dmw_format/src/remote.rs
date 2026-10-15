// SPDX-License-Identifier: GPL-3.0-or-later
//! `#[serde(remote = ...)]` mirrors of `weapon_iw4` structs (sheet 03 section 5.4).
//!
//! Each mirror lists every field of the remote type, so a field added, renamed or retyped in
//! iw4L is a compile error here instead of silently dropped data. Struct literals in the
//! generated code have no `..` rest pattern, which is what enforces that.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use weapon_iw4::{
    AimAssistRanges, DualMagTimes, EmptyReloadPolicy, HITLOC_COUNT, ScopeZoom, WeaponCombatFacts,
    WeaponIdleInputs, WeaponMovementOfsInputs, WeaponSwayParams,
};

/// Mirror of [`weapon_iw4::AimAssistRanges`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "AimAssistRanges")]
pub struct AimAssistRangesDef {
    pub auto_aim: f32,
    pub hip: f32,
    pub ads: f32,
}

/// Mirror of [`weapon_iw4::EmptyReloadPolicy`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "EmptyReloadPolicy")]
pub enum EmptyReloadPolicyDef {
    Ordinary,
    Authored,
}

/// Mirror of [`weapon_iw4::DualMagTimes`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "DualMagTimes")]
pub struct DualMagTimesDef {
    pub reload_ms: i32,
    pub reload_empty_ms: i32,
    pub add_ms: i32,
    pub empty_add_ms: i32,
}

/// Mirror of [`weapon_iw4::WeaponSwayParams`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "WeaponSwayParams")]
pub struct WeaponSwayParamsDef {
    pub max_angle: f32,
    pub pitch_scale: f32,
    pub yaw_scale: f32,
    pub horiz_scale: f32,
    pub vert_scale: f32,
}

/// Mirror of [`weapon_iw4::WeaponIdleInputs`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "WeaponIdleInputs")]
pub struct WeaponIdleInputsDef {
    pub ads_idle_amount: f32,
    pub hip_idle_amount: f32,
    pub ads_idle_speed: f32,
    pub hip_idle_speed: f32,
    pub idle_crouch_factor: f32,
    pub idle_prone_factor: f32,
}

/// Mirror of [`weapon_iw4::WeaponMovementOfsInputs`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "WeaponMovementOfsInputs")]
pub struct WeaponMovementOfsInputsDef {
    pub stand_move: [f32; 3],
    pub stand_rot: [f32; 3],
    pub strafe_move: [f32; 3],
    pub strafe_rot: [f32; 3],
    pub ducked_move: [f32; 3],
    pub ducked_rot: [f32; 3],
    pub prone_move: [f32; 3],
    pub prone_rot: [f32; 3],
    pub pos_move_rate: f32,
    pub pos_prone_move_rate: f32,
    pub stand_move_min_speed: f32,
    pub ducked_move_min_speed: f32,
    pub prone_move_min_speed: f32,
    pub pos_rot_rate: f32,
    pub pos_prone_rot_rate: f32,
}

/// Mirror of [`weapon_iw4::WeaponCombatFacts`] (`tick.rs`), fields in declaration order.
#[derive(Serialize, Deserialize, Debug)]
#[serde(remote = "WeaponCombatFacts")]
pub struct WeaponCombatFactsDef {
    #[serde(with = "AimAssistRangesDef")]
    pub aim_assist: AimAssistRanges,
    pub dual_wield: bool,
    pub fire_melees: bool,
    pub fire_time_ms: i32,
    pub fire_delay_ms: i32,
    pub raise_time_ms: i32,
    pub drop_time_ms: i32,
    pub alternate_weapon: u32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub first_raise_time_ms: i32,
    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    #[serde(with = "EmptyReloadPolicyDef")]
    pub empty_reload: EmptyReloadPolicy,
    pub clip_size: i32,
    pub start_ammo: i32,
    pub max_ammo: i32,
    pub ammo_index: i32,
    pub clip_index: i32,
    pub fire_type: i32,
    pub weap_type: i32,
    pub weap_class: i32,
    pub player_anim_type: i32,
    pub inventory_type: i32,
    pub impact_type: i32,
    pub shots_per_fire: i32,
    pub burst_cooldown_ms: i32,
    pub bolt_action: bool,
    pub rechamber_time_ms: i32,
    pub rechamber_bolt_time_ms: i32,
    pub rechamber_bolt_delay_ms: i32,
    pub segmented_reload: bool,
    pub reload_start_time_ms: i32,
    pub reload_end_time_ms: i32,
    pub reload_ammo_add: i32,
    pub reload_add_time_ms: i32,
    pub reload_empty_add_time_ms: i32,
    pub reload_start_add_time_ms: i32,
    pub reload_start_add: i32,
    pub no_partial_reload: bool,
    pub inherits_perks: bool,
    pub sprint_raise_time_ms: i32,
    pub sprint_drop_time_ms: i32,
    pub stunned_start_time_ms: i32,
    pub stunned_end_time_ms: i32,
    pub damage: i32,
    pub min_damage: i32,
    pub max_damage_range: f32,
    pub min_damage_range: f32,
    pub hip_spread_stand_min: f32,
    pub hip_spread_ducked_min: f32,
    pub hip_spread_prone_min: f32,
    pub hip_spread_stand_max: f32,
    pub hip_spread_ducked_max: f32,
    pub hip_spread_prone_max: f32,
    pub hip_spread_decay_rate: f32,
    pub hip_spread_fire_add: f32,
    pub hip_spread_turn_add: f32,
    pub hip_spread_move_add: f32,
    pub hip_spread_ducked_decay: f32,
    pub hip_spread_prone_decay: f32,
    pub ads_spread: f32,
    pub aim_down_sight: bool,
    pub no_ads_when_mag_empty: bool,
    pub ads_reload_trans_time_ms: i32,
    pub ads_in_rate: f32,
    pub ads_out_rate: f32,
    pub rechamber_while_ads: bool,
    pub ads_fire_only: bool,
    pub melee_damage: i32,
    pub can_hold_breath: bool,
    #[serde(with = "scope_zoom")]
    pub scope_zoom: ScopeZoom,
    pub overlay_reticle: i32,
    pub melee_time_ms: i32,
    pub melee_delay_ms: i32,
    pub melee_charge_time_ms: i32,
    pub melee_charge_delay_ms: i32,
    pub melee_charge_anim: bool,
    pub knife_model: u32,
    pub quick_raise_time_ms: i32,
    pub quick_drop_time_ms: i32,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub ads_gun_kick_reduced_kick_bullets: i32,
    pub hip_gun_kick_reduced_kick_bullets: i32,
    pub location_damage: [f32; HITLOC_COUNT],
    #[serde(with = "opt_dual_mag")]
    pub dual_mag: Option<DualMagTimes>,
}

/// `ScopeZoom` has private fields, so it is stored as the list of its zoom FOVs.
///
/// An empty list is `ScopeZoom::NONE`. Loading rejects lists `ScopeZoom::from_fovs` would
/// silently trim (more than 3 levels, duplicates, FOV outside (1, 180)).
pub mod scope_zoom {
    use super::*;

    /// Serialises as `[fov0, fov1, ...]`.
    pub fn serialize<S: Serializer>(z: &ScopeZoom, s: S) -> Result<S::Ok, S::Error> {
        let fovs: Vec<f32> = (0..z.levels())
            .filter_map(|i| z.fov(u32::from(i)))
            .collect();
        fovs.serialize(s)
    }

    /// Deserialises with `ScopeZoom::from_fovs`, padding with 0.0 (which `from_fovs` skips).
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ScopeZoom, D::Error> {
        let fovs = Vec::<f32>::deserialize(d)?;
        if fovs.len() > 3 {
            return Err(D::Error::custom(format!(
                "scope_zoom has {} levels, at most 3 are supported",
                fovs.len()
            )));
        }
        let mut padded = [0.0f32; 3];
        padded[..fovs.len()].copy_from_slice(&fovs);
        let zoom = ScopeZoom::from_fovs(padded);
        if usize::from(zoom.levels()) != fovs.len() {
            return Err(D::Error::custom(format!(
                "scope_zoom {fovs:?} has an invalid or duplicate FOV"
            )));
        }
        Ok(zoom)
    }
}

/// `Option<DualMagTimes>` through [`DualMagTimesDef`].
pub mod opt_dual_mag {
    use super::*;

    /// Serialises `None` as `null`.
    pub fn serialize<S: Serializer>(v: &Option<DualMagTimes>, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wrap<'a>(#[serde(with = "DualMagTimesDef")] &'a DualMagTimes);
        match v {
            Some(x) => s.serialize_some(&Wrap(x)),
            None => s.serialize_none(),
        }
    }

    /// Deserialises `null` as `None`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DualMagTimes>, D::Error> {
        #[derive(Deserialize)]
        struct Wrap(#[serde(with = "DualMagTimesDef")] DualMagTimes);
        Ok(Option::<Wrap>::deserialize(d)?.map(|w| w.0))
    }
}

/// Hands out distinct numbers so that no two fields of a test value coincide.
struct Distinct(u32);

impl Distinct {
    fn i(&mut self) -> i32 {
        self.0 += 1;
        self.0 as i32 * 3 + 1
    }
    fn u(&mut self) -> u32 {
        self.0 += 1;
        self.0 * 3 + 1_000_001
    }
    fn f(&mut self) -> f32 {
        self.0 += 1;
        self.0 as f32 * 0.25 + 0.0625
    }
    fn v3(&mut self) -> [f32; 3] {
        [self.f(), self.f(), self.f()]
    }
}

/// A [`WeaponCombatFacts`] with every field set to a distinct non-default value (T3.2).
///
/// Booleans are all `true` and options are `Some`, the non-default choice for each. The struct
/// literal names every field, so adding a field upstream breaks this function at compile time.
pub fn combat_facts_all_distinct() -> WeaponCombatFacts {
    let mut d = Distinct(0);
    let mut location_damage = [0.0f32; HITLOC_COUNT];
    for slot in &mut location_damage {
        *slot = d.f();
    }
    WeaponCombatFacts {
        aim_assist: AimAssistRanges {
            auto_aim: d.f(),
            hip: d.f(),
            ads: d.f(),
        },
        dual_wield: true,
        fire_melees: true,
        fire_time_ms: d.i(),
        fire_delay_ms: d.i(),
        raise_time_ms: d.i(),
        drop_time_ms: d.i(),
        alternate_weapon: d.u(),
        alternate_raise_time_ms: d.i(),
        alternate_drop_time_ms: d.i(),
        first_raise_time_ms: d.i(),
        reload_time_ms: d.i(),
        reload_empty_time_ms: d.i(),
        empty_reload: EmptyReloadPolicy::Ordinary,
        clip_size: d.i(),
        start_ammo: d.i(),
        max_ammo: d.i(),
        ammo_index: d.i(),
        clip_index: d.i(),
        fire_type: d.i(),
        weap_type: d.i(),
        weap_class: d.i(),
        player_anim_type: d.i(),
        inventory_type: d.i(),
        impact_type: d.i(),
        shots_per_fire: d.i(),
        burst_cooldown_ms: d.i(),
        bolt_action: true,
        rechamber_time_ms: d.i(),
        rechamber_bolt_time_ms: d.i(),
        rechamber_bolt_delay_ms: d.i(),
        segmented_reload: true,
        reload_start_time_ms: d.i(),
        reload_end_time_ms: d.i(),
        reload_ammo_add: d.i(),
        reload_add_time_ms: d.i(),
        reload_empty_add_time_ms: d.i(),
        reload_start_add_time_ms: d.i(),
        reload_start_add: d.i(),
        no_partial_reload: true,
        inherits_perks: true,
        sprint_raise_time_ms: d.i(),
        sprint_drop_time_ms: d.i(),
        stunned_start_time_ms: d.i(),
        stunned_end_time_ms: d.i(),
        damage: d.i(),
        min_damage: d.i(),
        max_damage_range: d.f(),
        min_damage_range: d.f(),
        hip_spread_stand_min: d.f(),
        hip_spread_ducked_min: d.f(),
        hip_spread_prone_min: d.f(),
        hip_spread_stand_max: d.f(),
        hip_spread_ducked_max: d.f(),
        hip_spread_prone_max: d.f(),
        hip_spread_decay_rate: d.f(),
        hip_spread_fire_add: d.f(),
        hip_spread_turn_add: d.f(),
        hip_spread_move_add: d.f(),
        hip_spread_ducked_decay: d.f(),
        hip_spread_prone_decay: d.f(),
        ads_spread: d.f(),
        aim_down_sight: true,
        no_ads_when_mag_empty: true,
        ads_reload_trans_time_ms: d.i(),
        ads_in_rate: d.f(),
        ads_out_rate: d.f(),
        rechamber_while_ads: true,
        ads_fire_only: true,
        melee_damage: d.i(),
        can_hold_breath: true,
        scope_zoom: ScopeZoom::from_fovs([30.0, 15.5, 7.25]),
        overlay_reticle: d.i(),
        melee_time_ms: d.i(),
        melee_delay_ms: d.i(),
        melee_charge_time_ms: d.i(),
        melee_charge_delay_ms: d.i(),
        melee_charge_anim: true,
        knife_model: d.u(),
        quick_raise_time_ms: d.i(),
        quick_drop_time_ms: d.i(),
        select_requires_ammo: Some(true),
        offhand_hold_is_cancelable: Some(true),
        ads_gun_kick_reduced_kick_bullets: d.i(),
        hip_gun_kick_reduced_kick_bullets: d.i(),
        location_damage,
        dual_mag: Some(DualMagTimes {
            reload_ms: d.i(),
            reload_empty_ms: d.i(),
            add_ms: d.i(),
            empty_add_ms: d.i(),
        }),
    }
}

/// A [`WeaponSwayParams`] with distinct values, for tests of the sway mirror.
pub fn sway_params_distinct(seed: u32) -> WeaponSwayParams {
    let mut d = Distinct(seed);
    WeaponSwayParams {
        max_angle: d.f(),
        pitch_scale: d.f(),
        yaw_scale: d.f(),
        horiz_scale: d.f(),
        vert_scale: d.f(),
    }
}

/// A [`WeaponIdleInputs`] with distinct values.
pub fn idle_inputs_distinct(seed: u32) -> WeaponIdleInputs {
    let mut d = Distinct(seed);
    WeaponIdleInputs {
        ads_idle_amount: d.f(),
        hip_idle_amount: d.f(),
        ads_idle_speed: d.f(),
        hip_idle_speed: d.f(),
        idle_crouch_factor: d.f(),
        idle_prone_factor: d.f(),
    }
}

/// A [`WeaponMovementOfsInputs`] with distinct values.
pub fn movement_ofs_distinct(seed: u32) -> WeaponMovementOfsInputs {
    let mut d = Distinct(seed);
    WeaponMovementOfsInputs {
        stand_move: d.v3(),
        stand_rot: d.v3(),
        strafe_move: d.v3(),
        strafe_rot: d.v3(),
        ducked_move: d.v3(),
        ducked_rot: d.v3(),
        prone_move: d.v3(),
        prone_rot: d.v3(),
        pos_move_rate: d.f(),
        pos_prone_move_rate: d.f(),
        stand_move_min_speed: d.f(),
        ducked_move_min_speed: d.f(),
        prone_move_min_speed: d.f(),
        pos_rot_rate: d.f(),
        pos_prone_rot_rate: d.f(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "WeaponCombatFactsDef")]
        combat: WeaponCombatFacts,
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct ZoomHolder {
        #[serde(with = "scope_zoom")]
        z: ScopeZoom,
    }

    /// T3.2: every field of `WeaponCombatFacts` survives a JSON round trip.
    #[test]
    fn combat_facts_round_trip() {
        let original = Holder {
            combat: combat_facts_all_distinct(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: Holder = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
        // A sample of the opposite choices proves the all-distinct value is not the default.
        assert_ne!(original.combat, WeaponCombatFacts::none());
        assert!(json.contains("\"empty_reload\":\"Ordinary\""), "{json}");
    }

    #[test]
    fn combat_facts_all_distinct_values_do_not_collide() {
        let json = serde_json::to_value(Holder {
            combat: combat_facts_all_distinct(),
        })
        .unwrap();
        let obj = json["combat"].as_object().unwrap();
        let mut seen: Vec<(String, String)> = Vec::new();
        for (k, v) in obj {
            // Skip booleans, enums, arrays and objects; only scalar numbers must differ.
            if v.is_number() {
                let text = v.to_string();
                if let Some((other, _)) = seen.iter().find(|(_, s)| *s == text) {
                    panic!("{k} and {other} share the value {v}");
                }
                seen.push((k.clone(), text));
            }
        }
        assert!(seen.len() > 60, "only {} numeric fields", seen.len());
    }

    #[test]
    fn combat_facts_none_round_trip() {
        let original = Holder {
            combat: WeaponCombatFacts::none(),
        };
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"dual_mag\":null"), "{json}");
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), original);
    }

    #[test]
    fn combat_facts_postcard_round_trip() {
        let original = Holder {
            combat: combat_facts_all_distinct(),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        assert_eq!(postcard::from_bytes::<Holder>(&bytes).unwrap(), original);
    }

    /// T3.3: scope zoom round trip for 0, 1, 2 and 3 levels.
    #[test]
    fn scope_zoom_round_trip() {
        for fovs in [
            &[][..],
            &[20.0][..],
            &[20.0, 10.0][..],
            &[40.0, 20.0, 5.5][..],
        ] {
            let mut padded = [0.0f32; 3];
            padded[..fovs.len()].copy_from_slice(fovs);
            let z = ScopeZoom::from_fovs(padded);
            assert_eq!(usize::from(z.levels()), fovs.len());
            let original = ZoomHolder { z };
            let json = serde_json::to_string(&original).unwrap();
            let back: ZoomHolder = serde_json::from_str(&json).unwrap();
            assert_eq!(back, original, "{json}");
            if fovs.is_empty() {
                assert_eq!(back.z, ScopeZoom::NONE);
                assert_eq!(json, r#"{"z":[]}"#);
            }
        }
    }

    #[test]
    fn scope_zoom_rejects_untrimmable_lists() {
        for bad in ["[1,2,3,4]", "[20,20]", "[0.5]", "[200]", "[20,0]", "[0,20]"] {
            let json = format!("{{\"z\":{bad}}}");
            assert!(serde_json::from_str::<ZoomHolder>(&json).is_err(), "{bad}");
        }
    }

    #[test]
    fn small_mirrors_round_trip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct All {
            #[serde(with = "WeaponSwayParamsDef")]
            sway: WeaponSwayParams,
            #[serde(with = "WeaponIdleInputsDef")]
            idle: WeaponIdleInputs,
            #[serde(with = "WeaponMovementOfsInputsDef")]
            movement: WeaponMovementOfsInputs,
        }
        let original = All {
            sway: sway_params_distinct(1),
            idle: idle_inputs_distinct(40),
            movement: movement_ofs_distinct(80),
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(serde_json::from_str::<All>(&json).unwrap(), original);
    }
}
