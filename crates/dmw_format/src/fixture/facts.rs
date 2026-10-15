// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixture weapon facts.
//!
//! Every number here is SYNTHETIC: chosen once to look like plausible gun data and to make
//! each weapon exercise a different code path (sheet 03 section 5.9). None of it is read from
//! or derived from MW2 data. Times are milliseconds, distances inches, angles degrees.

use std::collections::BTreeMap;

use weapon_iw4::{
    AimAssistRanges, EmptyReloadPolicy, HITLOC_COUNT, ScopeZoom, WEAPCLASS_PISTOL,
    WEAPCLASS_SPREAD, WEAPTYPE_BULLET, WEAPTYPE_PROJECTILE, WeaponCombatFacts, WeaponIdleInputs,
    WeaponMovementOfsInputs, WeaponSwayParams,
};

use crate::facts::{
    AnimTimes, FpvFacts, HudFacts, KickFacts, MovementScales, ProjectileFacts, SwayFacts,
    WeaponFacts, WeaponSounds,
};

/// Fixture weapon names in `DmwWeaponId` order (id = index + 1).
pub const WEAPONS: [&str; 5] = [
    "fixture_pistol",
    "fixture_rifle",
    "fixture_shotgun",
    "fixture_sniper",
    "fixture_launcher",
];

/// `(requested MW2 name, fixture weapon)` pairs for `PackManifest::aliases` (00 section 9.1).
pub const ALIASES: [(&str, &str); 11] = [
    ("usp_mp", "fixture_pistol"),
    ("deserteagle_mp", "fixture_pistol"),
    ("m4_mp", "fixture_rifle"),
    ("mg4_mp", "fixture_rifle"),
    ("kriss_mp", "fixture_rifle"),
    ("spas12_mp", "fixture_shotgun"),
    ("ranger_mp", "fixture_shotgun"),
    ("aa12_mp", "fixture_shotgun"),
    ("barrett_mp", "fixture_sniper"),
    ("cheytac_mp", "fixture_sniper"),
    ("rpg_mp", "fixture_launcher"),
];

/// Sound aliases the fixture provides.
pub const FIRE_SOUNDS: [&str; 5] = [
    "fixture_fire_pistol",
    "fixture_fire_rifle",
    "fixture_fire_shotgun",
    "fixture_fire_sniper",
    "fixture_fire_launcher",
];
pub const SOUND_EMPTY: &str = "fixture_empty";
pub const SOUND_MAG_OUT: &str = "fixture_mag_out";
pub const SOUND_MAG_IN: &str = "fixture_mag_in";
pub const SOUND_BOLT_BACK: &str = "fixture_bolt_back";
pub const SOUND_BOLT_FORWARD: &str = "fixture_bolt_forward";
pub const SOUND_EXPLODE: &str = "fixture_explode";

/// Damage by hit location (index = `weapon_iw4::HITLOC_NAMES`), synthetic.
fn location_damage(head: f32) -> [f32; HITLOC_COUNT] {
    let mut t = [1.0f32; HITLOC_COUNT];
    t[1] = head; // helmet
    t[2] = head; // head
    t[3] = head * 0.8; // neck
    t[6..=11].fill(0.9); // arms and hands
    t[12..=15].fill(0.8); // legs
    t[16] = 0.7; // feet
    t[17] = 0.7;
    t
}

/// Values shared by all fixture guns before per-weapon changes.
fn base_combat(index: i32) -> WeaponCombatFacts {
    let mut c = WeaponCombatFacts::none();
    c.aim_assist = AimAssistRanges {
        auto_aim: 0.0,
        hip: 0.0,
        ads: 0.0,
    };
    c.raise_time_ms = 500;
    c.drop_time_ms = 400;
    c.first_raise_time_ms = 1000;
    c.quick_raise_time_ms = 300;
    c.quick_drop_time_ms = 250;
    c.sprint_raise_time_ms = 300;
    c.sprint_drop_time_ms = 300;
    c.empty_reload = EmptyReloadPolicy::Authored;
    c.ammo_index = index;
    c.clip_index = index;
    c.weap_type = WEAPTYPE_BULLET;
    c.inventory_type = 0;
    c.impact_type = 1;
    c.shots_per_fire = 1;
    c.burst_cooldown_ms = weapon_iw4::BURST_COOLDOWN_DEFAULT_MS;
    c.inherits_perks = true;
    c.hip_spread_stand_min = 4.0;
    c.hip_spread_ducked_min = 2.8;
    c.hip_spread_prone_min = 1.6;
    c.hip_spread_stand_max = 10.0;
    c.hip_spread_ducked_max = 7.0;
    c.hip_spread_prone_max = 4.5;
    c.hip_spread_decay_rate = 5.0;
    c.hip_spread_fire_add = 1.5;
    c.hip_spread_turn_add = 0.0;
    c.hip_spread_move_add = 3.0;
    c.hip_spread_ducked_decay = 6.0;
    c.hip_spread_prone_decay = 8.0;
    c.ads_spread = 0.4;
    c.aim_down_sight = true;
    c.ads_reload_trans_time_ms = 350;
    // ADS rates are per millisecond, as iw4L stores them (movement_iw4/src/ads_frac.rs adds
    // `rate * msec`; asset_game stores `1 / (seconds * 1000)`): 250 ms in and out.
    c.ads_in_rate = 1.0 / 250.0;
    c.ads_out_rate = 1.0 / 250.0;
    c.melee_damage = 135;
    c.melee_time_ms = 750;
    c.melee_delay_ms = 200;
    c.select_requires_ammo = Some(false);
    c.offhand_hold_is_cancelable = Some(false);
    c.location_damage = location_damage(2.5);
    c
}

fn pistol_combat() -> WeaponCombatFacts {
    let mut c = base_combat(1);
    c.fire_type = 1; // semi-auto
    c.fire_time_ms = 120;
    c.weap_class = WEAPCLASS_PISTOL;
    c.player_anim_type = 2;
    c.raise_time_ms = 350; // fast raise
    c.drop_time_ms = 300;
    c.first_raise_time_ms = 800;
    c.clip_size = 12;
    c.start_ammo = 60;
    c.max_ammo = 120;
    c.reload_time_ms = 1500;
    c.reload_empty_time_ms = 1800; // authored empty reload
    c.damage = 40;
    c.min_damage = 25;
    c.max_damage_range = 500.0;
    c.min_damage_range = 1200.0;
    c.ads_in_rate = 1.0 / 200.0;
    c.ads_out_rate = 1.0 / 200.0;
    c.hip_spread_stand_min = 3.0;
    c.melee_time_ms = 650;
    c
}

fn rifle_combat() -> WeaponCombatFacts {
    let mut c = base_combat(2);
    c.fire_type = 0; // full auto
    c.fire_time_ms = 86;
    c.weap_class = 0;
    c.player_anim_type = 4;
    c.clip_size = 30;
    c.start_ammo = 120;
    c.max_ammo = 240;
    c.reload_time_ms = 2200;
    c.reload_empty_time_ms = 2600;
    c.damage = 30;
    c.min_damage = 20;
    c.max_damage_range = 1000.0;
    c.min_damage_range = 2000.0;
    c.ads_in_rate = 1.0 / 250.0;
    c.ads_out_rate = 1.0 / 250.0;
    c.hip_gun_kick_reduced_kick_bullets = 5;
    c.ads_gun_kick_reduced_kick_bullets = 5;
    c
}

fn shotgun_combat() -> WeaponCombatFacts {
    let mut c = base_combat(3);
    c.fire_type = 1;
    c.fire_time_ms = 700;
    c.weap_class = WEAPCLASS_SPREAD;
    c.player_anim_type = 1;
    c.shots_per_fire = 8;
    c.clip_size = 8;
    c.start_ammo = 32;
    c.max_ammo = 64;
    c.empty_reload = EmptyReloadPolicy::Ordinary;
    c.segmented_reload = true;
    c.reload_start_time_ms = 500;
    c.reload_end_time_ms = 700;
    c.reload_ammo_add = 1;
    c.reload_add_time_ms = 600;
    c.reload_empty_add_time_ms = 600;
    c.reload_time_ms = 600;
    c.reload_empty_time_ms = 600;
    c.damage = 40;
    c.min_damage = 15;
    c.max_damage_range = 200.0;
    c.min_damage_range = 600.0; // also the pellet range for the spread class
    c.hip_spread_stand_min = 6.0;
    c.hip_spread_stand_max = 12.0;
    c.ads_spread = 3.0;
    c
}

fn sniper_combat() -> WeaponCombatFacts {
    let mut c = base_combat(4);
    c.fire_type = 1;
    c.fire_time_ms = 250;
    c.weap_class = 1;
    c.player_anim_type = 6;
    c.bolt_action = true;
    c.rechamber_time_ms = 1100;
    c.rechamber_bolt_time_ms = 800;
    c.rechamber_bolt_delay_ms = 300;
    c.clip_size = 5;
    c.start_ammo = 20;
    c.max_ammo = 40;
    c.reload_time_ms = 3000;
    c.reload_empty_time_ms = 3300;
    c.damage = 120;
    c.min_damage = 120;
    c.max_damage_range = 3000.0;
    c.min_damage_range = 6000.0;
    c.can_hold_breath = true;
    c.scope_zoom = ScopeZoom::from_fovs([20.0, 10.0, 0.0]);
    c.overlay_reticle = 1;
    c.ads_in_rate = 1.0 / 400.0;
    c.ads_out_rate = 1.0 / 350.0;
    c.ads_spread = 0.0;
    c.hip_spread_stand_min = 8.0;
    c.hip_spread_stand_max = 16.0;
    c.melee_time_ms = 900;
    c.location_damage = location_damage(5.0);
    c
}

fn launcher_combat() -> WeaponCombatFacts {
    let mut c = base_combat(5);
    c.fire_type = 1;
    c.fire_time_ms = 1000;
    c.weap_type = WEAPTYPE_PROJECTILE;
    c.weap_class = 7;
    c.player_anim_type = 7;
    c.clip_size = 1;
    c.start_ammo = 3;
    c.max_ammo = 6;
    c.reload_time_ms = 2500;
    c.reload_empty_time_ms = 2500;
    c.damage = 100; // direct impact
    c.min_damage = 100;
    c.melee_time_ms = 900;
    c.location_damage = [1.0; HITLOC_COUNT];
    c
}

/// Kick values scaled by `k` (a bigger gun kicks harder).
fn kick(k: f32) -> KickFacts {
    KickFacts {
        f_ads_view_kick_center_speed: 2000.0,
        f_hip_view_kick_center_speed: 1500.0,
        gun_max_pitch: 6.0 * k,
        gun_max_yaw: 3.0 * k,
        ads_gun_kick_reduced_kick_bullets: 0,
        ads_gun_kick_reduced_kick_percent: 50.0,
        ads_gun_kick_pitch_min: 0.2 * k,
        ads_gun_kick_pitch_max: 0.5 * k,
        ads_gun_kick_yaw_min: -0.2 * k,
        ads_gun_kick_yaw_max: 0.2 * k,
        ads_gun_kick_accel: 800.0,
        ads_gun_kick_speed_max: 900.0,
        ads_gun_kick_speed_decay: 60.0,
        ads_gun_kick_static_decay: 40.0,
        ads_view_kick_pitch_min: 0.5 * k,
        ads_view_kick_pitch_max: 1.0 * k,
        ads_view_kick_yaw_min: -0.4 * k,
        ads_view_kick_yaw_max: 0.4 * k,
        hip_gun_kick_reduced_kick_bullets: 0,
        hip_gun_kick_reduced_kick_percent: 100.0,
        hip_gun_kick_pitch_min: 0.4 * k,
        hip_gun_kick_pitch_max: 1.0 * k,
        hip_gun_kick_yaw_min: -0.4 * k,
        hip_gun_kick_yaw_max: 0.4 * k,
        hip_gun_kick_accel: 1000.0,
        hip_gun_kick_speed_max: 1200.0,
        hip_gun_kick_speed_decay: 70.0,
        hip_gun_kick_static_decay: 50.0,
        hip_view_kick_pitch_min: 1.0 * k,
        hip_view_kick_pitch_max: 2.0 * k,
        hip_view_kick_yaw_min: -0.8 * k,
        hip_view_kick_yaw_max: 0.8 * k,
    }
}

fn sway(k: f32) -> SwayFacts {
    SwayFacts {
        hip: WeaponSwayParams {
            max_angle: 3.0 * k,
            pitch_scale: 0.5,
            yaw_scale: 0.5,
            horiz_scale: 1.0,
            vert_scale: 0.8,
        },
        ads: WeaponSwayParams {
            max_angle: 1.0 * k,
            pitch_scale: 0.2,
            yaw_scale: 0.2,
            horiz_scale: 0.3,
            vert_scale: 0.3,
        },
    }
}

fn movement_ofs() -> WeaponMovementOfsInputs {
    WeaponMovementOfsInputs {
        stand_move: [0.4, -0.3, 0.2],
        stand_rot: [1.0, -0.5, 0.5],
        strafe_move: [0.2, -0.6, 0.1],
        strafe_rot: [0.5, -1.0, 2.0],
        ducked_move: [0.3, -0.2, 0.4],
        ducked_rot: [0.8, -0.4, 0.4],
        prone_move: [0.1, -0.1, 0.1],
        prone_rot: [0.3, -0.2, 0.2],
        pos_move_rate: 8.0,
        pos_prone_move_rate: 6.0,
        stand_move_min_speed: 120.0,
        ducked_move_min_speed: 80.0,
        prone_move_min_speed: 40.0,
        pos_rot_rate: 8.0,
        pos_prone_rot_rate: 6.0,
    }
}

fn fpv(c: &WeaponCombatFacts, zoom_fov: f32, overlay: (f32, f32)) -> FpvFacts {
    FpvFacts {
        ads_aim_pitch: 0.0,
        ads_bob_factor: 0.5,
        ads_view_bob_mult: 0.5,
        ads_zoom_fov: zoom_fov,
        ads_zoom_in_frac: 0.4,
        ads_zoom_out_frac: 0.3,
        ads_overlay_width: overlay.0,
        ads_overlay_height: overlay.1,
        aim_down_sight: c.aim_down_sight,
        can_hold_breath: c.can_hold_breath,
        ducked_ofs: [0.0, 0.0, -1.0],
        prone_ofs: [0.0, 0.0, -2.0],
        idle: WeaponIdleInputs {
            ads_idle_amount: 80.0,
            hip_idle_amount: 80.0,
            ads_idle_speed: 1.0,
            hip_idle_speed: 1.0,
            idle_crouch_factor: 0.5,
            idle_prone_factor: 0.3,
        },
        movement: movement_ofs(),
        overlay_reticle: c.overlay_reticle,
        reload_time_ms: c.reload_time_ms,
        scope_zoom: c.scope_zoom,
        ads_overlay_convention: "WeightIsFrac".to_owned(),
    }
}

fn hud(c: &WeaponCombatFacts, counter: i32) -> HudFacts {
    HudFacts {
        ads_crosshair_in_frac: 0.5,
        ads_crosshair_out_frac: 0.3,
        hip_reticle_side_pos: 2.0,
        i_reticle_min_ofs: 6,
        i_reticle_side_size: 6,
        clip_only: false,
        clip_size: c.clip_size,
        ammo_counter_clip: counter,
    }
}

fn anim_times(c: &WeaponCombatFacts) -> AnimTimes {
    AnimTimes {
        fire_time_ms: c.fire_time_ms,
        detonate_time_ms: 0,
        melee_time_ms: c.melee_time_ms,
        melee_charge_time_ms: 0,
        raise_time_ms: c.raise_time_ms,
        alternate_raise_time_ms: 0,
        alternate_drop_time_ms: 0,
        drop_time_ms: c.drop_time_ms,
        quick_drop_time_ms: c.quick_drop_time_ms,
        quick_raise_time_ms: c.quick_raise_time_ms,
        sprint_raise_time_ms: c.sprint_raise_time_ms,
        sprint_loop_time_ms: 800,
        sprint_drop_time_ms: c.sprint_drop_time_ms,
        reload_time_ms: c.reload_time_ms,
        reload_empty_time_ms: c.reload_empty_time_ms,
        reload_start_time_ms: c.reload_start_time_ms,
        reload_end_time_ms: c.reload_end_time_ms,
        reload_quick_time_ms: 0,
        reload_quick_empty_time_ms: 0,
    }
}

fn sounds(fire: &str, with_bolt: bool) -> WeaponSounds {
    let mut notetracks = BTreeMap::new();
    notetracks.insert("mag_out".to_owned(), SOUND_MAG_OUT.to_owned());
    notetracks.insert("mag_in".to_owned(), SOUND_MAG_IN.to_owned());
    if with_bolt {
        notetracks.insert("bolt_back".to_owned(), SOUND_BOLT_BACK.to_owned());
        notetracks.insert("bolt_forward".to_owned(), SOUND_BOLT_FORWARD.to_owned());
    }
    WeaponSounds {
        fire: Some(fire.to_owned()),
        fire_player: Some(fire.to_owned()),
        empty_fire: Some(SOUND_EMPTY.to_owned()),
        empty_fire_player: Some(SOUND_EMPTY.to_owned()),
        notetracks,
        ..WeaponSounds::default()
    }
}

fn movement(speed: f32) -> MovementScales {
    MovementScales {
        move_speed_scale: speed,
        ads_move_speed_scale: 0.5,
        sprint_duration_scale: 1.0,
    }
}

/// The facts of fixture weapon `index` (0-based, see [`WEAPONS`]).
pub fn weapon(index: usize) -> WeaponFacts {
    let name = WEAPONS[index];
    let (display, combat) = match index {
        0 => ("Fixture Pistol", pistol_combat()),
        1 => ("Fixture Rifle", rifle_combat()),
        2 => ("Fixture Shotgun", shotgun_combat()),
        3 => ("Fixture Sniper", sniper_combat()),
        _ => ("Fixture Launcher", launcher_combat()),
    };
    let (kick_scale, zoom_fov, overlay, counter, speed) = match index {
        0 => (0.6, 65.0, (0.0, 0.0), 1, 1.0),
        1 => (1.0, 50.0, (0.0, 0.0), 1, 0.95),
        2 => (1.8, 55.0, (0.0, 0.0), 3, 0.95),
        3 => (2.4, 20.0, (256.0, 192.0), 1, 0.85),
        _ => (1.5, 65.0, (0.0, 0.0), 4, 0.8),
    };
    let mut s = sounds(FIRE_SOUNDS[index], matches!(index, 1 | 3));
    if index == 4 {
        s.proj_explosion = Some(SOUND_EXPLODE.to_owned());
        s.detonate = Some(SOUND_EXPLODE.to_owned());
    }
    let projectile = (index == 4).then_some(ProjectileFacts {
        projectile_speed: 1800,
        projectile_speed_up: 0,
        projectile_speed_forward: 0,
        explosion_radius: 256,
        explosion_radius_min: 0,
        explosion_inner_damage: 200,
        explosion_outer_damage: 50,
        impact_damage: 100,
        proj_impact_explode: true,
        missile_guidance: 0,
        ignition_delay_ms: 0,
    });
    WeaponFacts {
        name: name.to_owned(),
        display_name: display.to_owned(),
        kick: kick(kick_scale),
        sway: sway(kick_scale.sqrt()),
        fpv: fpv(&combat, zoom_fov, overlay),
        hud: hud(&combat, counter),
        movement: movement(speed),
        anim_times: anim_times(&combat),
        projectile,
        sounds: s,
        has_overlay: index == 3,
        combat,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn spec_numbers() {
        let p = weapon(0).combat;
        assert_eq!((p.fire_type, p.fire_time_ms, p.clip_size), (1, 120, 12));
        assert_eq!((p.reload_time_ms, p.reload_empty_time_ms), (1500, 1800));
        assert_eq!((p.damage, p.min_damage), (40, 25));
        assert_eq!(
            (p.max_damage_range as i32, p.min_damage_range as i32),
            (500, 1200)
        );
        assert_eq!(p.weap_class, 5);

        let r = weapon(1);
        assert_eq!(
            (
                r.combat.fire_type,
                r.combat.fire_time_ms,
                r.combat.clip_size
            ),
            (0, 86, 30)
        );
        assert_eq!(
            (r.combat.reload_time_ms, r.combat.reload_empty_time_ms),
            (2200, 2600)
        );
        assert_eq!((r.combat.damage, r.combat.min_damage), (30, 20));
        assert_eq!(
            (
                r.combat.max_damage_range as i32,
                r.combat.min_damage_range as i32
            ),
            (1000, 2000)
        );
        assert!((r.fpv.ads_zoom_fov - 50.0).abs() < f32::EPSILON);
        assert!((r.combat.ads_in_rate - 1.0 / 250.0).abs() < 1e-9);

        let s = weapon(2).combat;
        assert_eq!((s.weap_class, s.shots_per_fire, s.clip_size), (4, 8, 8));
        assert!(s.segmented_reload);
        assert_eq!(
            (
                s.reload_start_time_ms,
                s.reload_add_time_ms,
                s.reload_end_time_ms
            ),
            (500, 600, 700)
        );
        assert_eq!((s.damage, s.min_damage), (40, 15));
        assert_eq!(s.min_damage_range as i32, 600);
        assert!((s.bullet_range() - 600.0).abs() < f32::EPSILON);

        let b = weapon(3);
        assert!(b.combat.bolt_action && b.combat.can_hold_breath && b.has_overlay);
        assert_eq!(
            (
                b.combat.rechamber_time_ms,
                b.combat.clip_size,
                b.combat.overlay_reticle
            ),
            (1100, 5, 1)
        );
        assert_eq!(b.combat.scope_zoom, ScopeZoom::from_fovs([20.0, 10.0, 0.0]));
        assert_eq!(b.combat.scope_zoom.levels(), 2);

        let l = weapon(4);
        assert_eq!(
            (
                l.combat.weap_type,
                l.combat.clip_size,
                l.combat.reload_time_ms
            ),
            (2, 1, 2500)
        );
        let pj = l.projectile.expect("launcher has projectile facts");
        assert_eq!((pj.projectile_speed, pj.explosion_radius), (1800, 256));
        assert_eq!(
            (pj.explosion_inner_damage, pj.explosion_outer_damage),
            (200, 50)
        );
        assert!(weapon(0).projectile.is_none());
    }

    #[test]
    fn melee_damage_is_135_everywhere() {
        for i in 0..WEAPONS.len() {
            assert_eq!(weapon(i).combat.melee_damage, 135);
        }
    }
}
