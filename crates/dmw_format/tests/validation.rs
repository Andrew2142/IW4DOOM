// SPDX-License-Identifier: GPL-3.0-or-later
//! T3.7: every validation rule of sheet 03 section 5.8 rejects a hand-corrupted copy, and the
//! loader's error cases of section 6 return the documented errors.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use dmw_format::anim::{FrameIndicesData, RotationData, TranslationData};
use dmw_format::fixture;
use dmw_format::validate;
use dmw_format::{Pack, PackError, ViewmodelFile, WeaponFacts};

use common::{TempDir, fixture_file};

fn facts() -> Vec<WeaponFacts> {
    let pack = Pack::fixture();
    pack.weapon_ids().map(|id| pack.facts(id).clone()).collect()
}

fn manifest_with(f: impl FnOnce(&mut Vec<WeaponFacts>)) -> Result<(), PackError> {
    let pack = Pack::fixture();
    let mut all = facts();
    f(&mut all);
    validate::manifest(&pack.manifest, &all)
}

/// Asserts `Err(Invalid(msg))` with every `needle` in `msg`.
#[track_caller]
fn rejects<T: std::fmt::Debug>(r: Result<T, PackError>, needles: &[&str]) {
    match r {
        Err(PackError::Invalid(msg)) => {
            for n in needles {
                assert!(msg.contains(n), "{msg:?} does not mention {n:?}");
            }
        }
        other => panic!("expected Invalid mentioning {needles:?}, got {other:?}"),
    }
}

fn vm_check(vm: &ViewmodelFile) -> Result<(), PackError> {
    validate::viewmodel("fixture_rifle", vm, 4)
}

fn rifle() -> ViewmodelFile {
    fixture_file("fixture_rifle")
}

#[test]
fn pristine_fixture_passes() {
    assert!(manifest_with(|_| {}).is_ok());
    assert!(vm_check(&rifle()).is_ok());
}

// ---- combat sanity ---------------------------------------------------------

#[test]
fn combat_fire_time_must_be_positive_for_guns() {
    rejects(
        manifest_with(|f| f[1].combat.fire_time_ms = 0),
        &["fixture_rifle", "combat.fire_time_ms"],
    );
    // A projectile weapon needs it too.
    rejects(
        manifest_with(|f| f[4].combat.fire_time_ms = -5),
        &["fixture_launcher", "fire_time_ms"],
    );
}

#[test]
fn combat_clip_size_must_be_at_least_one() {
    rejects(manifest_with(|f| f[0].combat.clip_size = 0), &["clip_size"]);
}

#[test]
fn combat_damage_must_not_be_negative() {
    rejects(
        manifest_with(|f| f[0].combat.damage = -1),
        &["combat.damage"],
    );
}

#[test]
fn combat_damage_range_order() {
    rejects(
        manifest_with(|f| {
            f[1].combat.max_damage_range = 2000.0;
            f[1].combat.min_damage_range = 1000.0;
        }),
        &["min_damage_range"],
    );
    // Equal is allowed, and a zero (unknown) range disables the rule.
    assert!(
        manifest_with(|f| {
            f[1].combat.max_damage_range = 1500.0;
            f[1].combat.min_damage_range = 1500.0;
        })
        .is_ok()
    );
    assert!(manifest_with(|f| f[1].combat.min_damage_range = 0.0).is_ok());
}

#[test]
fn combat_fire_type_range() {
    rejects(manifest_with(|f| f[0].combat.fire_type = 6), &["fire_type"]);
    rejects(
        manifest_with(|f| f[0].combat.fire_type = -1),
        &["fire_type"],
    );
}

#[test]
fn combat_location_damage_finite_and_non_negative() {
    rejects(
        manifest_with(|f| f[0].combat.location_damage[2] = -0.5),
        &["location_damage"],
    );
    rejects(
        manifest_with(|f| f[0].combat.location_damage[2] = f32::NAN),
        &["location_damage"],
    );
    rejects(
        manifest_with(|f| f[0].combat.location_damage[19] = f32::INFINITY),
        &["location_damage"],
    );
}

#[test]
fn combat_iw4l_try_from_captured_rules() {
    rejects(
        manifest_with(|f| {
            f[1].combat.fire_type = 3;
            f[1].combat.burst_cooldown_ms = 0;
        }),
        &["burst_cooldown_ms"],
    );
    rejects(
        manifest_with(|f| f[2].combat.reload_ammo_add = 0),
        &["reload_ammo_add"],
    );
}

#[test]
fn manifest_cross_checks() {
    rejects(
        manifest_with(|f| f[0].name = "other".into()),
        &["name is 'other'"],
    );
    rejects(
        manifest_with(|f| f[3].has_overlay = false),
        &["has_overlay"],
    );
    rejects(
        manifest_with(|f| f[4].projectile.as_mut().unwrap().projectile_speed = 0),
        &["projectile_speed"],
    );
    rejects(manifest_with(|f| drop(f.pop())), &["5 weapons", "4 facts"]);
}

#[test]
fn manifest_paths_and_sounds_are_checked() {
    let base = Pack::fixture().manifest;
    let all = facts();

    let mut m = base.clone();
    m.weapons[0].dir = "../outside".into();
    rejects(validate::manifest(&m, &all), &["dir", "safe"]);

    let mut m = base.clone();
    m.weapons[1].name = m.weapons[0].name.clone();
    let mut dup = all.clone();
    dup[1].name = dup[0].name.clone();
    rejects(validate::manifest(&m, &dup), &["duplicate weapon"]);

    let mut m = base.clone();
    m.sounds[0].variants.clear();
    rejects(validate::manifest(&m, &all), &["no variants"]);

    let mut m = base.clone();
    m.sounds[1].variants[0].file = "/etc/passwd".into();
    rejects(validate::manifest(&m, &all), &["safe relative path"]);

    let mut m = base.clone();
    m.sounds[1].alias = m.sounds[0].alias.clone();
    rejects(validate::manifest(&m, &all), &["duplicated"]);

    let mut m = base.clone();
    m.sounds[2].variants[0].pitch = 0.0;
    rejects(validate::manifest(&m, &all), &["pitch"]);

    let mut m = base;
    m.aliases.insert("evil_mp".into(), "no_such_weapon".into());
    rejects(validate::manifest(&m, &all), &["evil_mp"]);
}

// ---- skeleton --------------------------------------------------------------

#[test]
fn skeleton_bone_limit_is_192() {
    let mut vm = rifle();
    let template = vm.skeleton.bones[7].clone();
    while vm.skeleton.bones.len() < 192 {
        vm.skeleton.bones.push(template.clone());
    }
    vm.skeleton.models[0].bone_count = 192;
    assert!(
        validate::skeleton("w", &vm.skeleton).is_ok(),
        "192 bones are allowed"
    );
    vm.skeleton.bones.push(template);
    vm.skeleton.models[0].bone_count = 193;
    rejects(validate::skeleton("w", &vm.skeleton), &["193", "1..=192"]);
}

#[test]
fn skeleton_parent_must_precede_child() {
    let mut vm = rifle();
    vm.skeleton.bones[2].parent = Some(2);
    rejects(vm_check(&vm), &["bones[2]", "parent"]);
    let mut vm = rifle();
    vm.skeleton.bones[1].parent = Some(5);
    rejects(vm_check(&vm), &["bones[1]", "parent"]);
}

#[test]
fn skeleton_view_bone_in_range() {
    let mut vm = rifle();
    vm.skeleton.view_bone = 8;
    rejects(vm_check(&vm), &["view_bone"]);
    let mut vm = rifle();
    vm.skeleton.flash_bone = Some(99);
    rejects(vm_check(&vm), &["flash_bone"]);
}

#[test]
fn skeleton_quaternions_are_unit_and_finite() {
    let mut vm = rifle();
    vm.skeleton.bones[3].bind_rotation = [0.0, 0.0, 0.0, 1.01];
    rejects(vm_check(&vm), &["bones[3]", "unit quaternion"]);
    let mut vm = rifle();
    vm.skeleton.bones[3].bind_rotation = [f32::NAN, 0.0, 0.0, 1.0];
    rejects(vm_check(&vm), &["unit quaternion"]);
    // Just inside the tolerance is fine.
    let mut vm = rifle();
    vm.skeleton.bones[3].bind_rotation = [0.0, 0.0, 0.0, 1.0005];
    assert!(vm_check(&vm).is_ok());
}

// ---- mesh ------------------------------------------------------------------

#[test]
fn mesh_attribute_lengths_must_match() {
    let mut vm = rifle();
    vm.mesh.uvs.pop();
    rejects(vm_check(&vm), &["attribute lengths"]);
    let mut vm = rifle();
    vm.mesh.skin.pop();
    rejects(vm_check(&vm), &["attribute lengths"]);
}

#[test]
fn mesh_indices_in_range_and_triangles() {
    let mut vm = rifle();
    let n = vm.mesh.positions.len() as u32;
    vm.mesh.indices[0] = n;
    rejects(vm_check(&vm), &["out of range"]);
    let mut vm = rifle();
    vm.mesh.indices.push(0);
    rejects(vm_check(&vm), &["multiple of 3"]);
}

#[test]
fn mesh_surface_ranges_inside_and_disjoint() {
    let mut vm = rifle();
    let total = vm.mesh.indices.len() as u32;
    vm.mesh.surfaces[0].index_count = total + 3;
    rejects(vm_check(&vm), &["surfaces[0]", "exceeds indices"]);

    let mut vm = rifle();
    vm.mesh.surfaces[1].index_start = vm.mesh.surfaces[0].index_start;
    rejects(vm_check(&vm), &["overlap"]);

    let mut vm = rifle();
    vm.mesh.surfaces[2].index_count += 1;
    rejects(vm_check(&vm), &["triangle aligned"]);
}

#[test]
fn mesh_surface_texture_and_model_references() {
    let mut vm = rifle();
    vm.mesh.surfaces[0].texture = Some(4);
    rejects(vm_check(&vm), &["texture 4", "4 textures"]);
    let mut vm = rifle();
    vm.mesh.surfaces[0].model = 3;
    rejects(vm_check(&vm), &["model 3"]);
}

#[test]
fn mesh_skin_bones_and_weights() {
    let mut vm = rifle();
    vm.mesh.skin[3].bones[0] = 8;
    rejects(vm_check(&vm), &["skin[3]", "out of range"]);
    let mut vm = rifle();
    vm.mesh.skin[3].weights = [0.5, 0.0, 0.0, 0.0];
    rejects(vm_check(&vm), &["skin[3]", "sum to 1"]);
    let mut vm = rifle();
    vm.mesh.skin[3].weights = [f32::NAN, 0.0, 0.0, 0.0];
    rejects(vm_check(&vm), &["skin[3]"]);
    let mut vm = rifle();
    vm.mesh.skin[3].weights = [1.5, -0.5, 0.0, 0.0];
    rejects(vm_check(&vm), &["skin[3]"]);
    // Weights within 1e-3 of 1 pass.
    let mut vm = rifle();
    vm.mesh.skin[3].weights = [0.9995, 0.0, 0.0, 0.0];
    assert!(vm_check(&vm).is_ok());
}

// ---- clips -----------------------------------------------------------------

#[test]
fn clips_need_39_slots() {
    let mut vm = rifle();
    vm.clips.slots.pop();
    rejects(vm_check(&vm), &["slots has 38", "39"]);
}

#[test]
fn clips_slot_references_must_exist() {
    let mut vm = rifle();
    let bad = vm.clips.clips.len() as u16;
    vm.clips.slots[3] = Some(bad);
    rejects(vm_check(&vm), &["slots[3]"]);
}

#[test]
fn clips_track_maps_match_tracks_and_bones() {
    let mut vm = rifle();
    vm.clips.track_maps[0].pop();
    rejects(vm_check(&vm), &["track_maps[0]"]);
    let mut vm = rifle();
    vm.clips.track_maps[0][0] = Some(8);
    rejects(vm_check(&vm), &["missing bone 8"]);
    let mut vm = rifle();
    vm.clips.track_maps.pop();
    rejects(vm_check(&vm), &["track_maps has"]);
}

#[test]
fn clips_framerate_rule() {
    let mut vm = rifle();
    vm.clips.clips[0].framerate = 0.0;
    vm.clips.clips[0].numframes = 30;
    rejects(vm_check(&vm), &["framerate"]);
    // A single-frame pose may have no frame rate.
    let mut vm = rifle();
    vm.clips.clips[0].framerate = 0.0;
    vm.clips.clips[0].numframes = 1;
    for t in &mut vm.clips.clips[0].tracks {
        t.rotation = RotationData::Default;
        t.translation = TranslationData::Default;
    }
    assert!(vm_check(&vm).is_ok());
    let mut vm = rifle();
    vm.clips.clips[0].framerate = f32::NAN;
    rejects(vm_check(&vm), &["framerate"]);
}

/// First keyed rotation track of the rifle's idle clip (clip 0, track 0: dense, 61 keys).
fn idle_keys(vm: &mut ViewmodelFile) -> &mut dmw_format::anim::KeyedQuat {
    match &mut vm.clips.clips[0].tracks[0].rotation {
        RotationData::HalfKeyed(k) | RotationData::FullKeyed(k) => k,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn clips_keyed_counts_match_frames() {
    // Dense with more keys than frames + 1.
    let mut vm = rifle();
    idle_keys(&mut vm).values.push([0.0, 0.0, 0.0, 1.0]);
    rejects(vm_check(&vm), &["dense keys"]);

    // Sparse with a frame list of the wrong length.
    let mut vm = rifle();
    idle_keys(&mut vm).frames = FrameIndicesData::Sparse(vec![0, 1, 2]);
    rejects(vm_check(&vm), &["frame indices"]);

    // Sparse that goes backwards.
    let mut vm = rifle();
    let k = idle_keys(&mut vm);
    k.values.truncate(3);
    k.frames = FrameIndicesData::Sparse(vec![0, 9, 4]);
    rejects(vm_check(&vm), &["non-decreasing"]);

    // Sparse that runs past the end of the clip.
    let mut vm = rifle();
    let k = idle_keys(&mut vm);
    k.values.truncate(2);
    k.frames = FrameIndicesData::Sparse(vec![0, 61]);
    rejects(vm_check(&vm), &["past numframes"]);

    // No keys at all.
    let mut vm = rifle();
    let k = idle_keys(&mut vm);
    k.values.clear();
    k.frames = FrameIndicesData::Sparse(vec![]);
    rejects(vm_check(&vm), &["no keys"]);
}

// ---- sounds ----------------------------------------------------------------

#[test]
fn missing_sound_aliases_are_reported_but_not_fatal() {
    let pack = Pack::fixture();
    let mut all = facts();
    all[0].sounds.reload = Some("fixture_does_not_exist".into());
    // Not a validation error...
    assert!(validate::manifest(&pack.manifest, &all).is_ok());
    // ...but listed so the loader can warn once per alias.
    let missing = validate::missing_sounds(&pack.manifest, &all);
    assert_eq!(
        missing,
        [(
            "fixture_pistol".to_owned(),
            "fixture_does_not_exist".to_owned()
        )]
    );

    // An on-disk pack with that dangling alias still opens.
    let dir = TempDir::new("missing_sound");
    fixture::write_to_dir(dir.path()).unwrap();
    let path = dir.path().join("weapons/fixture_pistol/facts.json");
    let mut f: WeaponFacts = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    f.sounds.reload = Some("fixture_does_not_exist".into());
    std::fs::write(&path, serde_json::to_vec_pretty(&f).unwrap()).unwrap();
    let opened = Pack::open(dir.path()).unwrap();
    assert!(opened.load_sound("fixture_does_not_exist").is_err());
}

// ---- loader error cases (section 6) ----------------------------------------

fn written(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    fixture::write_to_dir(dir.path()).unwrap();
    dir
}

#[test]
fn missing_manifest() {
    let dir = TempDir::new("no_manifest");
    match Pack::open(dir.path()) {
        Err(PackError::Missing(what)) => assert_eq!(what, "manifest.json"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn wrong_version() {
    let dir = written("version");
    let path = dir.path().join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"format_version\": 1", "\"format_version\": 2");
    std::fs::write(&path, text).unwrap();
    match Pack::open(dir.path()) {
        Err(PackError::Version {
            found: 2,
            expected: 1,
        }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn version_is_checked_before_the_rest_of_the_manifest() {
    let dir = written("version_shape");
    std::fs::write(
        dir.path().join("manifest.json"),
        r#"{"format_version": 7, "weird": true}"#,
    )
    .unwrap();
    assert!(matches!(
        Pack::open(dir.path()),
        Err(PackError::Version { found: 7, .. })
    ));
}

#[test]
fn garbage_manifest_is_a_format_error() {
    let dir = written("garbage");
    std::fs::write(dir.path().join("manifest.json"), "not json").unwrap();
    assert!(matches!(Pack::open(dir.path()), Err(PackError::Format(_))));
}

#[test]
fn one_bad_facts_file_fails_the_whole_pack_and_names_the_weapon() {
    let dir = written("bad_facts");
    std::fs::write(dir.path().join("weapons/fixture_shotgun/facts.json"), "{").unwrap();
    match Pack::open(dir.path()) {
        Err(PackError::Format(msg)) => assert!(msg.contains("fixture_shotgun"), "{msg}"),
        other => panic!("{other:?}"),
    }

    let dir = written("missing_facts");
    std::fs::remove_file(dir.path().join("weapons/fixture_rifle/facts.json")).unwrap();
    match Pack::open(dir.path()) {
        Err(PackError::Missing(msg)) => assert!(msg.contains("fixture_rifle"), "{msg}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn facts_name_must_match_the_manifest() {
    let dir = written("name_mismatch");
    let a = dir.path().join("weapons/fixture_pistol/facts.json");
    let b = dir.path().join("weapons/fixture_rifle/facts.json");
    std::fs::copy(&b, &a).unwrap();
    rejects(
        Pack::open(dir.path()),
        &["fixture_pistol", "name is 'fixture_rifle'"],
    );
}

#[test]
fn viewmodel_file_errors() {
    // Missing while has_viewmodel = true.
    let dir = written("no_vm");
    std::fs::remove_file(dir.path().join("weapons/fixture_pistol/viewmodel.bin")).unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    let id = pack.weapon_id("fixture_pistol").unwrap();
    assert!(matches!(
        pack.load_viewmodel(id),
        Err(PackError::Missing(_))
    ));

    // Bad magic.
    let dir = written("bad_magic");
    let path = dir.path().join("weapons/fixture_pistol/viewmodel.bin");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] = b'X';
    std::fs::write(&path, bytes).unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(matches!(
        pack.load_viewmodel(pack.weapon_id("fixture_pistol").unwrap()),
        Err(PackError::Format(_))
    ));

    // Truncated postcard body.
    let dir = written("truncated");
    let path = dir.path().join("weapons/fixture_pistol/viewmodel.bin");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(matches!(
        pack.load_viewmodel(pack.weapon_id("fixture_pistol").unwrap()),
        Err(PackError::Format(_))
    ));

    // A structurally invalid but decodable file is Invalid at load time.
    let dir = written("invalid_vm");
    let mut vm = fixture_file("fixture_pistol");
    vm.skeleton.bones[2].parent = Some(7);
    std::fs::write(
        dir.path().join("weapons/fixture_pistol/viewmodel.bin"),
        vm.to_bytes().unwrap(),
    )
    .unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    rejects(
        pack.load_viewmodel(pack.weapon_id("fixture_pistol").unwrap()),
        &["fixture_pistol", "parent"],
    );
}

#[test]
fn texture_and_sound_file_errors() {
    // Missing texture.
    let dir = written("no_tex");
    std::fs::remove_file(dir.path().join("weapons/fixture_pistol/textures/2.png")).unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(
        matches!(pack.load_viewmodel(pack.weapon_id("fixture_pistol").unwrap()), Err(PackError::Missing(m)) if m.contains("textures/2.png"))
    );

    // Undecodable texture.
    let dir = written("bad_tex");
    std::fs::write(
        dir.path().join("weapons/fixture_pistol/textures/0.png"),
        b"nope",
    )
    .unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(matches!(
        pack.load_viewmodel(pack.weapon_id("fixture_pistol").unwrap()),
        Err(PackError::Invalid(_))
    ));

    // Oversize texture (4097 wide).
    let dir = written("big_tex");
    let big =
        dmw_format::texture::encode_png(&dmw_format::RgbaImage::new(4097, 1, [0; 4])).unwrap();
    std::fs::write(
        dir.path().join("weapons/fixture_pistol/textures/0.png"),
        big,
    )
    .unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(matches!(
        pack.load_viewmodel(pack.weapon_id("fixture_pistol").unwrap()),
        Err(PackError::Invalid(_))
    ));

    // Missing overlay.
    let dir = written("no_overlay");
    std::fs::remove_file(dir.path().join("weapons/fixture_sniper/overlay.png")).unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(matches!(
        pack.load_overlay(pack.weapon_id("fixture_sniper").unwrap()),
        Err(PackError::Missing(_))
    ));

    // Missing and 3-channel WAV.
    let dir = written("sounds");
    std::fs::remove_file(dir.path().join("sounds/fixture_empty/0.wav")).unwrap();
    let wav3 = dmw_format::sound::encode_wav_i16(22050, 3, &[0, 0, 0]).unwrap();
    std::fs::write(dir.path().join("sounds/fixture_mag_in/0.wav"), wav3).unwrap();
    let pack = Pack::open(dir.path()).unwrap();
    assert!(matches!(
        pack.load_sound("fixture_empty"),
        Err(PackError::Missing(_))
    ));
    assert!(matches!(
        pack.load_sound("fixture_mag_in"),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn manifest_cannot_point_outside_the_pack() {
    let dir = written("escape");
    let path = dir.path().join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("weapons/fixture_pistol", "../fixture_pistol");
    std::fs::write(&path, text).unwrap();
    rejects(Pack::open(dir.path()), &["safe relative path"]);
}
