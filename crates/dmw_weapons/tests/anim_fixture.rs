// SPDX-License-Identifier: GPL-3.0-or-later
//! Sheet 10 T10.1–T10.8 on the fixture pack: conversion, controller dispatch, notetracks,
//! placement conventions, skinning identity, ADS, sprint and determinism.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::*;
use dmw_format::anim::{ClipSet, WEAPON_ANIM_SLOTS, slot};
use dmw_format::consts::tic_msec;
use dmw_format::{DmwWeaponId, ViewmodelAsset};
use dmw_weapons::controller::{ControllerState, WeaponAnimSlot};
use dmw_weapons::convert::clip_to_anim;
use dmw_weapons::placement::placement_matrix;
use dmw_weapons::{
    GunPlacement, SimEvent, SoundCue, TicButtons, TicInput, TicOutput, ViewState,
    ViewmodelAnimator, ViewmodelPose,
};
use glam::{Mat4, Vec3};
use weapon_iw4::WeaponState;
use xmodel_runtime::{AnimClip, ClipNotify, FrameIndices, Keyed, Rotation, Track, Translation};

/// A sim plus the animator of one weapon, advanced together each tic as sheet 08/11 will.
struct Driver {
    rig: Rig,
    anim: ViewmodelAnimator,
}

impl Driver {
    fn new(weapon: DmwWeaponId) -> Self {
        let rig = Rig::new();
        let asset = rig.pack.load_viewmodel(weapon).unwrap();
        let anim = ViewmodelAnimator::new(asset, rig.facts(weapon)).unwrap();
        Self { rig, anim }
    }

    fn run(&mut self, input: TicInput) -> (TicOutput, ViewState, Vec<SoundCue>) {
        let tic = self.rig.tic;
        let out = self.rig.run(input);
        let view = self.rig.sim.view_state();
        let cues = self.anim.advance(&view, tic_msec(tic));
        (out, view, cues)
    }

    fn step(
        &mut self,
        weapon: DmwWeaponId,
        b: TicButtons,
    ) -> (TicOutput, ViewState, Vec<SoundCue>) {
        let input = self.rig.input(Some(weapon), b);
        self.run(input)
    }

    /// Raises `weapon` until `Ready`.
    fn raise(&mut self, weapon: DmwWeaponId) {
        for _ in 0..200 {
            let (_, view, _) = self.step(weapon, TicButtons::default());
            if view.weapon == Some(weapon) && view.weaponstate == WeaponState::Ready as i32 {
                return;
            }
        }
        panic!("never ready");
    }

    fn state(&self) -> (ControllerState, Option<WeaponAnimSlot>) {
        (
            self.anim.controller().state(),
            self.anim.controller().action(),
        )
    }
}

fn translation(m: &Mat4) -> Vec3 {
    m.w_axis.truncate()
}

fn tag_flash(pose: &ViewmodelPose) -> Vec3 {
    translation(&pose.tag_flash.expect("fixture has tag_flash"))
}

// ---------------------------------------------------------------------------------------------

/// T10.1: the converted fixture reload clip samples exactly like a hand-built `AnimClip`.
#[test]
fn t10_1_converted_clip_samples_like_hand_built() {
    let pack = dmw_format::Pack::fixture();
    let asset = pack.load_viewmodel(RIFLE).unwrap();
    let data = asset.clips.clip_for_slot(slot::RELOAD).unwrap();
    let converted = clip_to_anim(data);

    // The fixture reload, written out by hand (dmw_format fixture/clips.rs `reload`).
    let qy = |deg: f32| {
        let h = deg.to_radians() * 0.5;
        [0.0, h.sin(), 0.0, h.cos()]
    };
    let hand = AnimClip {
        name: "fixture_reload".into(),
        framerate: 30.0,
        numframes: 60,
        looping: false,
        tracks: vec![
            Track {
                name: "j_mag".into(),
                rotation: Rotation::Default,
                translation: Translation::FullKeyed(Keyed {
                    values: vec![
                        [0.0; 3],
                        [0.0, 0.0, -8.0],
                        [0.0, 0.0, -8.0],
                        [0.0; 3],
                        [0.0; 3],
                    ],
                    frames: FrameIndices::Sparse(vec![0, 9, 21, 33, 60]),
                }),
            },
            Track {
                name: "j_gun".into(),
                rotation: Rotation::FullKeyed(Keyed {
                    values: vec![qy(0.0), qy(10.0), qy(10.0), qy(0.0)],
                    frames: FrameIndices::Sparse(vec![0, 15, 45, 60]),
                }),
                translation: Translation::Default,
            },
        ],
        notifies: vec![
            ClipNotify {
                name: "mag_out".into(),
                time: 9.0 / 60.0,
            },
            ClipNotify {
                name: "mag_in".into(),
                time: 21.0 / 60.0,
            },
        ],
        delta_translation: Translation::Default,
    };

    assert_eq!(converted.name, hand.name);
    assert_eq!(converted.duration(), hand.duration());
    assert_eq!(converted.tracks.len(), hand.tracks.len());
    for i in 0..20 {
        let t = hand.duration() * i as f32 / 19.0;
        for track in 0..hand.tracks.len() {
            let a = converted.sample_track(track, t).unwrap();
            let b = hand.sample_track(track, t).unwrap();
            assert_eq!(a.rotation, b.rotation, "rotation track {track} t {t}");
            assert_eq!(
                a.translation, b.translation,
                "translation track {track} t {t}"
            );
        }
    }
    for (a, b) in converted.notifies.iter().zip(&hand.notifies) {
        assert_eq!((a.name.as_str(), a.time), (b.name.as_str(), b.time));
    }
}

/// T10.2: raise → idle → fire ×3 → reload → idle drive the expected slots; golden
/// `anim_sequence.json` holds every controller change.
#[test]
fn t10_2_controller_sequence() {
    let mut d = Driver::new(RIFLE);
    let mut changes: Vec<serde_json::Value> = Vec::new();
    let mut last = None;
    let mut record = |d: &Driver, label: &str| {
        let now = d.state();
        if last != Some(now) {
            last = Some(now);
            changes.push(serde_json::json!({
                "tic": d.rig.tic - 1,
                "phase": label,
                "state": format!("{:?}", now.0),
                "action": format!("{:?}", now.1),
            }));
        }
    };
    d.rig.sim.inventory_mut().set_magazine(RIFLE, 20);
    // Raise from an empty hand (first raise), then idle.
    for _ in 0..60 {
        d.step(RIFLE, TicButtons::default());
        record(&d, "raise");
    }
    // Three taps of the trigger, a second apart.
    for _ in 0..3 {
        d.step(RIFLE, attack());
        record(&d, "fire");
        for _ in 0..34 {
            d.step(RIFLE, TicButtons::default());
            record(&d, "fire");
        }
    }
    // Reload a partly full magazine, then idle.
    d.step(RIFLE, reload());
    record(&d, "reload");
    for _ in 0..120 {
        d.step(RIFLE, TicButtons::default());
        record(&d, "reload");
    }

    let actions: Vec<String> = changes
        .iter()
        .map(|c| c["action"].as_str().unwrap().to_owned())
        .collect();
    let mut dedup = actions.clone();
    dedup.dedup();
    let want_order = [
        "Some(FirstRaise)",
        "None",
        "Some(Fire)",
        "None",
        "Some(Fire)",
        "None",
        "Some(Fire)",
        "None",
        "Some(Reload)",
        "None",
    ];
    assert_eq!(dedup, want_order, "changes: {changes:#?}");
    assert_eq!(d.anim.controller().state(), ControllerState::Ready);
    check_golden("anim_sequence.json", &serde_json::Value::Array(changes));
}

/// T10.3: the rifle reload emits `fixture_mag_out` and `fixture_mag_in` at the authored clip
/// times (0.3 s and 0.7 s of the 2.0 s clip), played at `reload_time_ms / clip length` as iw4L
/// scales non-native slots, ± one tic.
#[test]
fn t10_3_reload_notetracks() {
    let mut d = Driver::new(RIFLE);
    d.raise(RIFLE);
    d.rig.sim.inventory_mut().set_magazine(RIFLE, 10);
    let reload_ms = d.rig.facts(RIFLE).anim_times.reload_time_ms as f32;
    let clip_ms = 2000.0;
    let scale = reload_ms / clip_ms;

    let mut elapsed = 0i32;
    let mut heard: Vec<(String, i32)> = Vec::new();
    let mut started = false;
    for i in 0..120 {
        let b = if i == 0 {
            reload()
        } else {
            TicButtons::default()
        };
        let tic = d.rig.tic;
        let (out, _, cues) = d.step(RIFLE, b);
        if out
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::ReloadStarted { .. }))
        {
            started = true;
        }
        if started {
            elapsed += tic_msec(tic);
        }
        for c in cues {
            assert!(c.local);
            heard.push((c.alias, elapsed));
        }
    }
    assert!(started);
    let at = |alias: &str| heard.iter().find(|h| h.0 == alias).map(|h| h.1).unwrap();
    let out_ms = at("fixture_mag_out");
    let in_ms = at("fixture_mag_in");
    let want_out = 300.0 * scale;
    let want_in = 700.0 * scale;
    assert!(
        (out_ms as f32 - want_out).abs() <= 29.0,
        "mag_out at {out_ms} ms, want {want_out}"
    );
    assert!(
        (in_ms as f32 - want_in).abs() <= 29.0,
        "mag_in at {in_ms} ms, want {want_in}"
    );
    assert_eq!(heard.len(), 2, "{heard:?}");
}

/// T10.4: with a zero placement the fixture `tag_flash` lands forward, slightly right and below
/// the eye in MW2 view axes (x forward, y left, z up).
#[test]
fn t10_4_placement_axes() {
    let pack = dmw_format::Pack::fixture();
    let asset = pack.load_viewmodel(RIFLE).unwrap();
    let anim = ViewmodelAnimator::new(asset, pack.facts(RIFLE)).unwrap();
    let view = ViewState::default();
    let p = tag_flash(&anim.pose(&view));
    // Bind: j_gun (14, -5, -6) + tag_flash (18, 0, 1) below tag_view at the origin.
    assert!(p.x > 20.0, "forward: {p}");
    assert!(p.y < 0.0 && p.y > -10.0, "slightly right: {p}");
    assert!(p.z < 0.0, "below: {p}");
    assert!(p.abs_diff_eq(Vec3::new(32.0, -5.0, -5.0), 1.0), "{p}");

    // A placement offset [forward, right, up] moves it the same way in view axes.
    let shifted = ViewState {
        placement: GunPlacement {
            origin: [1.0, 2.0, 3.0],
            angles: [0.0; 3],
        },
        ..ViewState::default()
    };
    let q = tag_flash(&anim.pose(&shifted));
    assert!(
        (q - p).abs_diff_eq(Vec3::new(1.0, -2.0, 3.0), 1e-3),
        "{p} -> {q}"
    );
}

/// The fixture rifle with no clips at all: every bone stays at its bind pose.
fn bind_only_asset() -> Arc<ViewmodelAsset> {
    let pack = dmw_format::Pack::fixture();
    let asset = pack.load_viewmodel(RIFLE).unwrap();
    Arc::new(ViewmodelAsset {
        mesh: asset.mesh.clone(),
        skeleton: asset.skeleton.clone(),
        clips: ClipSet {
            clips: vec![],
            slots: vec![None; WEAPON_ANIM_SLOTS],
            track_maps: vec![],
        },
        textures: vec![],
    })
}

/// T10.5: at bind pose, skin matrices × bind positions = eye-space bind mesh positions.
#[test]
fn t10_5_bind_pose_skinning_identity() {
    let asset = bind_only_asset();
    let pack = dmw_format::Pack::fixture();
    let anim = ViewmodelAnimator::new(Arc::clone(&asset), pack.facts(RIFLE)).unwrap();
    for placement in [
        GunPlacement::default(),
        GunPlacement {
            origin: [2.0, -1.0, 0.5],
            angles: [3.0, -4.0, 5.0],
        },
    ] {
        let view = ViewState {
            placement,
            ..ViewState::default()
        };
        let pose = anim.pose(&view);
        assert_eq!(pose.skin.len(), asset.skeleton.bones.len());
        // tag_view is at the model origin, so eye space = placement · model space.
        let eye_from_model = placement_matrix(&placement);
        for (p, s) in asset.mesh.positions.iter().zip(&asset.mesh.skin) {
            let p = Vec3::from_array(*p);
            let mut skinned = Vec3::ZERO;
            for (b, w) in s.bones.iter().zip(s.weights) {
                skinned += pose.skin[usize::from(*b)].transform_point3(p) * w;
            }
            let want = eye_from_model.transform_point3(p);
            assert!(skinned.abs_diff_eq(want, 1e-4), "{skinned} vs {want}");
        }
    }
}

/// T10.6: ADS 0 → 1 moves the fixture gun from the hip toward the centre: `tag_flash` y → ~0.
#[test]
fn t10_6_ads_centres_the_gun() {
    let mut d = Driver::new(RIFLE);
    d.raise(RIFLE);
    let (_, hip_view, _) = d.step(RIFLE, TicButtons::default());
    let hip = tag_flash(&d.anim.pose(&hip_view));
    let mut ads_view = hip_view.clone();
    for _ in 0..30 {
        let (_, view, _) = d.step(RIFLE, ads());
        ads_view = view;
    }
    assert!(ads_view.ads_frac >= 1.0);
    let aimed = tag_flash(&d.anim.pose(&ads_view));
    assert!(hip.y < -3.0, "hip {hip}");
    assert!(aimed.y.abs() < 0.5, "aimed {aimed} (hip {hip})");
    assert!(aimed.z > hip.z, "the gun comes up: {hip} -> {aimed}");
}

/// T10.7: sprint_in → sprint_loop → sprint_out follow weaponstate 0x17–0x19.
#[test]
fn t10_7_sprint_dispatches_follow_weaponstate() {
    let mut d = Driver::new(RIFLE);
    d.raise(RIFLE);
    let mut seen: Vec<(i32, Option<WeaponAnimSlot>)> = Vec::new();
    for i in 0..120 {
        let mut input = d.rig.input(Some(RIFLE), TicButtons::default());
        if i < 70 {
            input.buttons.sprint = true;
            input.forwardmove = 50;
            input.velocity_mw2 = [190.0, 0.0, 0.0];
        }
        let (_, view, _) = d.run(input);
        let entry = (view.weaponstate, d.anim.controller().action());
        if seen.last() != Some(&entry) {
            seen.push(entry);
        }
    }
    let states: Vec<i32> = seen.iter().map(|s| s.0).collect();
    for (ws, slot) in [
        (WeaponState::SprintIn, WeaponAnimSlot::SprintIn),
        (WeaponState::SprintLoop, WeaponAnimSlot::SprintLoop),
        (WeaponState::SprintOut, WeaponAnimSlot::SprintOut),
    ] {
        let ws = ws as i32;
        assert!(states.contains(&ws), "never in {ws:#x}: {seen:?}");
        assert!(
            seen.iter().any(|s| s.0 == ws && s.1 == Some(slot)),
            "weaponstate {ws:#x} without {slot:?}: {seen:?}"
        );
    }
    let first = |slot| seen.iter().position(|s| s.1 == Some(slot)).unwrap();
    assert!(first(WeaponAnimSlot::SprintIn) < first(WeaponAnimSlot::SprintLoop));
    assert!(first(WeaponAnimSlot::SprintLoop) < first(WeaponAnimSlot::SprintOut));
}

fn scripted_poses(tics: u32) -> (u64, usize) {
    use std::hash::{Hash, Hasher};
    let mut d = Driver::new(RIFLE);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut cues = 0;
    for i in 0..tics {
        let phase = i % 400;
        let b = TicButtons {
            attack: (60..120).contains(&phase),
            ads: (100..200).contains(&phase),
            reload: phase == 250,
            sprint: (300..340).contains(&phase),
            ..TicButtons::default()
        };
        d.rig.pools[0] = 200;
        let mut input = d.rig.input(Some(RIFLE), b);
        if b.sprint {
            input.forwardmove = 50;
        }
        input.yaw_deg = 90.0 + (i as f32 * 0.1).sin() * 5.0;
        let (_, view, c) = d.run(input);
        cues += c.len();
        let pose = d.anim.pose(&view);
        for m in &pose.skin {
            for v in m.to_cols_array() {
                v.to_bits().hash(&mut hasher);
            }
        }
        let _ = d.anim.visibility(&view);
    }
    (hasher.finish(), cues)
}

/// T10.8 (determinism half; timing is `tests/anim_perf.rs`): 10 000 advance + pose calls on the
/// same script give bit-identical poses.
#[test]
fn t10_8_deterministic() {
    let a = scripted_poses(10_000);
    let b = scripted_poses(10_000);
    assert_eq!(a, b);
    assert!(a.1 > 0, "the script reloads, so notetracks play");
}

/// Sheet 10 §5.8: rocket hidden with an empty launcher until the reload is under way; knife
/// only during melee.
#[test]
fn surface_visibility() {
    let mut d = Driver::new(LAUNCHER);
    d.raise(LAUNCHER);
    let (_, view, _) = d.step(LAUNCHER, TicButtons::default());
    assert!(d.anim.visibility(&view).rocket, "loaded");
    assert!(!d.anim.visibility(&view).knife);
    d.rig.sim.inventory_mut().set_magazine(LAUNCHER, 0);
    let (_, view, _) = d.step(LAUNCHER, TicButtons::default());
    assert!(!d.anim.visibility(&view).rocket, "empty, not reloading");
    let (_, view, _) = d.step(LAUNCHER, reload());
    assert!(view.weaponstate == WeaponState::Reloading as i32);
    let (_, view, _) = d.step(LAUNCHER, TicButtons::default());
    assert!(d.anim.visibility(&view).rocket, "reloading");

    let mut knife = false;
    let mut d = Driver::new(RIFLE);
    d.raise(RIFLE);
    for i in 0..40 {
        let b = TicButtons {
            melee: i == 0,
            ..TicButtons::default()
        };
        let (_, view, _) = d.step(RIFLE, b);
        knife |= d.anim.visibility(&view).knife;
    }
    assert!(knife, "melee shows the knife");
}
