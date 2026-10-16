// SPDX-License-Identifier: GPL-3.0-or-later
//! `WeaponFacts` extraction (sheet 04 §5.4): every number is copied from what iw4L publishes
//! to its own simulation (`BoundWeapon`, `WeaponFpvFacts`, `WeaponAnimations`, ...).

use std::collections::BTreeMap;

use asset_audio::SoundCatalog;
use asset_core::AssetNamespace;
use asset_game::{
    WeaponAnimations, WeaponFpvFacts, WeaponKickFacts, WeaponRegistry, WeaponSoundSlot,
};
use dmw_format::{
    AnimTimes, FpvFacts, HudFacts, KickFacts, MovementScales, ProjectileFacts, SwayFacts,
    WeaponFacts, WeaponSounds,
};

/// A sound alias a weapon references, with the namespace it lives in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AliasRef {
    pub ns: AssetNamespace,
    pub alias: String,
}

/// Facts plus what the rest of the export needs to know about them.
#[derive(Debug)]
pub struct ExtractedFacts {
    pub facts: WeaponFacts,
    /// `false` when `fpv_facts()` is `None` (the weapon is exported without a viewmodel).
    pub has_fpv: bool,
    /// Every alias referenced by `facts.sounds`, with its namespace.
    pub aliases: Vec<AliasRef>,
    pub warnings: Vec<String>,
}

macro_rules! copy_kick {
    ($k:expr, $($f:ident),+ $(,)?) => {
        KickFacts { $($f: $k.$f),+ }
    };
}

fn kick(k: &WeaponKickFacts) -> KickFacts {
    copy_kick!(
        k,
        f_ads_view_kick_center_speed,
        f_hip_view_kick_center_speed,
        gun_max_pitch,
        gun_max_yaw,
        ads_gun_kick_reduced_kick_bullets,
        ads_gun_kick_reduced_kick_percent,
        ads_gun_kick_pitch_min,
        ads_gun_kick_pitch_max,
        ads_gun_kick_yaw_min,
        ads_gun_kick_yaw_max,
        ads_gun_kick_accel,
        ads_gun_kick_speed_max,
        ads_gun_kick_speed_decay,
        ads_gun_kick_static_decay,
        ads_view_kick_pitch_min,
        ads_view_kick_pitch_max,
        ads_view_kick_yaw_min,
        ads_view_kick_yaw_max,
        hip_gun_kick_reduced_kick_bullets,
        hip_gun_kick_reduced_kick_percent,
        hip_gun_kick_pitch_min,
        hip_gun_kick_pitch_max,
        hip_gun_kick_yaw_min,
        hip_gun_kick_yaw_max,
        hip_gun_kick_accel,
        hip_gun_kick_speed_max,
        hip_gun_kick_speed_decay,
        hip_gun_kick_static_decay,
        hip_view_kick_pitch_min,
        hip_view_kick_pitch_max,
        hip_view_kick_yaw_min,
        hip_view_kick_yaw_max,
    )
}

fn fpv(f: &WeaponFpvFacts) -> FpvFacts {
    FpvFacts {
        ads_aim_pitch: f.ads_aim_pitch,
        ads_bob_factor: f.ads_bob_factor,
        ads_view_bob_mult: f.ads_view_bob_mult,
        ads_zoom_fov: f.ads_zoom_fov,
        ads_zoom_in_frac: f.ads_zoom_in_frac,
        ads_zoom_out_frac: f.ads_zoom_out_frac,
        ads_overlay_width: f.ads_overlay_width,
        ads_overlay_height: f.ads_overlay_height,
        aim_down_sight: f.aim_down_sight,
        can_hold_breath: f.can_hold_breath,
        ducked_ofs: f.ducked_ofs,
        prone_ofs: f.prone_ofs,
        idle: f.idle,
        movement: f.movement,
        overlay_reticle: f.overlay_reticle,
        reload_time_ms: f.reload_time_ms,
        reload_show_rocket_time_ms: f.reload_show_rocket_time_ms,
        scope_zoom: f.scope_zoom,
        ads_overlay_convention: format!("{:?}", f.ads_overlay_convention()),
    }
}

fn anim_times(a: &WeaponAnimations) -> AnimTimes {
    AnimTimes {
        fire_time_ms: a.fire_time_ms,
        detonate_time_ms: a.detonate_time_ms,
        melee_time_ms: a.melee_time_ms,
        melee_charge_time_ms: a.melee_charge_time_ms,
        raise_time_ms: a.raise_time_ms,
        alternate_raise_time_ms: a.alternate_raise_time_ms,
        alternate_drop_time_ms: a.alternate_drop_time_ms,
        drop_time_ms: a.drop_time_ms,
        quick_drop_time_ms: a.quick_drop_time_ms,
        quick_raise_time_ms: a.quick_raise_time_ms,
        sprint_raise_time_ms: a.sprint_raise_time_ms,
        sprint_loop_time_ms: a.sprint_loop_time_ms,
        sprint_drop_time_ms: a.sprint_drop_time_ms,
        reload_time_ms: a.reload_time_ms,
        reload_empty_time_ms: a.reload_empty_time_ms,
        reload_start_time_ms: a.reload_start_time_ms,
        reload_end_time_ms: a.reload_end_time_ms,
        reload_quick_time_ms: a.reload_quick_time_ms,
        reload_quick_empty_time_ms: a.reload_quick_empty_time_ms,
    }
}

/// English display name: the localize key looked up in `strings` (in the weapon's identity
/// namespace, as iw4L `hud/src/weapon_name.rs`), else the `*_mp` asset name.
pub fn display_name(
    registry: &WeaponRegistry,
    index: u32,
    strings: &asset_game::LocalizeCatalog,
) -> String {
    let fallback = registry.script_name_of(index);
    let Some(key) = registry
        .display_name_key_of(index)
        .filter(|k| !k.is_empty())
    else {
        return fallback;
    };
    let text = match registry.identity_namespace_of(index) {
        Some(ns) => strings.text_in(ns, key).or_else(|| strings.text(key)),
        None => strings.text(key),
    };
    text.filter(|t| !t.is_empty())
        .map_or(fallback, str::to_owned)
}

/// Resolves one authored alias against the sound bank. Returns the bank's alias name.
fn bank_alias(
    registry: &WeaponRegistry,
    index: u32,
    authored: &str,
    sounds: Option<&SoundCatalog>,
) -> Option<AliasRef> {
    let ns = registry
        .sound_namespace_for(index, authored)
        .unwrap_or(AssetNamespace::Iw4);
    let sound = sounds?.sound_in(ns, authored)?;
    Some(AliasRef {
        ns,
        alias: sound.name.clone(),
    })
}

/// `WeaponSounds` (slot aliases plus notetracks) and the alias set it references.
fn weapon_sounds(
    registry: &WeaponRegistry,
    index: u32,
    sounds: Option<&SoundCatalog>,
    warnings: &mut Vec<String>,
) -> (WeaponSounds, Vec<AliasRef>) {
    let mut map = serde_json::Map::new();
    let mut aliases = Vec::new();
    // WeaponSoundSlot::ALL is in the same order as the first 39 WeaponSounds fields.
    for (slot, field) in WeaponSoundSlot::ALL.iter().zip(WeaponSounds::SLOT_NAMES) {
        let resolved = sounds.and_then(|catalog| {
            registry
                .weapon_sound_key(index, *slot, catalog)
                .map(|(ns, alias)| AliasRef {
                    ns,
                    alias: alias.to_owned(),
                })
        });
        let entry = match (resolved, registry.authored_weapon_sound(index, *slot)) {
            (Some(r), _) => Some(r),
            (None, Some(authored)) => {
                warnings.push(format!(
                    "sound slot {field}: alias '{authored}' not in the sound bank"
                ));
                bank_alias(registry, index, authored, sounds).or_else(|| {
                    // Keep the authored name so the game can log the miss (03 §5.8).
                    map.insert((*field).to_owned(), authored.into());
                    None
                })
            }
            (None, None) => None,
        };
        if let Some(r) = entry {
            map.insert((*field).to_owned(), r.alias.clone().into());
            aliases.push(r);
        }
    }
    // Loop/stop slots are not WeaponSoundSlots; read them from the alias table directly.
    if let Some(table) = registry.sounds_of(index) {
        for (field, authored) in [
            ("fire_loop", &table.fire_loop),
            ("fire_loop_player", &table.fire_loop_player),
            ("fire_stop", &table.fire_stop),
            ("fire_stop_player", &table.fire_stop_player),
        ] {
            let Some(authored) = authored.as_deref().filter(|a| !a.is_empty()) else {
                continue;
            };
            match bank_alias(registry, index, authored, sounds) {
                Some(r) => {
                    map.insert(field.to_owned(), r.alias.clone().into());
                    aliases.push(r);
                }
                None => {
                    warnings.push(format!(
                        "sound slot {field}: alias '{authored}' not in the sound bank"
                    ));
                    map.insert(field.to_owned(), authored.into());
                }
            }
        }
    }
    let mut notetracks = BTreeMap::new();
    for (note, action) in registry.notetrack_actions_of(index) {
        let Some(authored) = action.sound_alias.as_deref().filter(|a| !a.is_empty()) else {
            continue;
        };
        match bank_alias(registry, index, authored, sounds) {
            Some(r) => {
                notetracks.insert(note.to_owned(), r.alias.clone());
                aliases.push(r);
            }
            None => {
                warnings.push(format!(
                    "notetrack {note}: alias '{authored}' not in the sound bank"
                ));
                notetracks.insert(note.to_owned(), authored.to_owned());
            }
        }
    }
    let mut ws: WeaponSounds =
        serde_json::from_value(serde_json::Value::Object(map)).unwrap_or_default();
    ws.notetracks = notetracks;
    aliases.sort();
    aliases.dedup();
    (ws, aliases)
}

/// Extracts every fact of weapon row `index`. A `combat_facts` refusal is a weapon error.
pub fn extract(
    registry: &WeaponRegistry,
    index: u32,
    prepared: &assets::PreparedMatch,
    strings: &asset_game::LocalizeCatalog,
    sound_catalog: Option<&SoundCatalog>,
) -> Result<ExtractedFacts, String> {
    let mut warnings = Vec::new();
    let bound = registry
        .bind_published_row(index)
        .map_err(|e| format!("bind_published_row: {e:?}"))?;
    let combat = bound
        .combat_facts(
            weapon_iw4::WeaponHostRules::default(),
            prepared.lochit_table,
        )
        .map_err(|e| format!("combat facts refused: {e:?}"))?;
    let fpv_facts = bound.fpv_facts();
    let has_fpv = fpv_facts.is_some();
    if !has_fpv {
        warnings.push("no FPV facts: default kick/sway/placement, no viewmodel".to_owned());
    }
    let f = fpv_facts.unwrap_or_default();
    let hud = match bound.hud_facts() {
        Some(h) => HudFacts {
            ads_crosshair_in_frac: h.ads_crosshair_in_frac,
            ads_crosshair_out_frac: h.ads_crosshair_out_frac,
            hip_reticle_side_pos: h.hip_reticle_side_pos,
            i_reticle_min_ofs: h.i_reticle_min_ofs,
            i_reticle_side_size: h.i_reticle_side_size,
            clip_only: h.clip_only,
            clip_size: h.clip_size,
            ammo_counter_clip: h.ammo_counter_clip,
        },
        None => {
            warnings.push("no HUD facts: zeros".to_owned());
            HudFacts {
                ads_crosshair_in_frac: 0.0,
                ads_crosshair_out_frac: 0.0,
                hip_reticle_side_pos: 0.0,
                i_reticle_min_ofs: 0,
                i_reticle_side_size: 0,
                clip_only: false,
                clip_size: combat.clip_size,
                ammo_counter_clip: 0,
            }
        }
    };
    let (move_speed_scale, ads_move_speed_scale, sprint_duration_scale) = bound.movement_scales();
    let anims = WeaponAnimations::from_registry(registry, index, &prepared.xanims);
    let projectile = registry
        .equipment_facts_of(index)
        .filter(|e| e.projectile_speed > 0)
        .map(|e| {
            let ev = bound.event_facts();
            ProjectileFacts {
                projectile_speed: e.projectile_speed,
                projectile_speed_up: e.projectile_speed_up,
                projectile_speed_forward: e.projectile_speed_forward,
                explosion_radius: ev.map_or(e.explosion_radius, |v| v.explosion_radius),
                explosion_radius_min: ev.map_or(e.explosion_radius_min, |v| v.explosion_radius_min),
                explosion_inner_damage: ev
                    .map_or(e.explosion_inner_damage, |v| v.explosion_inner_damage),
                explosion_outer_damage: ev
                    .map_or(e.explosion_outer_damage, |v| v.explosion_outer_damage),
                impact_damage: e.impact_damage,
                proj_impact_explode: e.proj_impact_explode,
                missile_guidance: e.missile_guidance,
                ignition_delay_ms: ev.map_or(e.ignition_delay_ms, |v| v.ignition_delay_ms),
            }
        });
    let (sounds, aliases) = weapon_sounds(registry, index, sound_catalog, &mut warnings);
    let facts = WeaponFacts {
        name: registry.script_name_of(index),
        display_name: display_name(registry, index, strings),
        combat,
        kick: kick(&f.kick),
        sway: SwayFacts {
            hip: f.sway.hip_params(),
            ads: f.sway.ads_params(),
        },
        fpv: fpv(&f),
        hud,
        movement: MovementScales {
            move_speed_scale,
            ads_move_speed_scale,
            sprint_duration_scale,
        },
        anim_times: anim_times(&anims),
        projectile,
        sounds,
        has_overlay: false,
    };
    Ok(ExtractedFacts {
        facts,
        has_fpv,
        aliases,
        warnings,
    })
}
