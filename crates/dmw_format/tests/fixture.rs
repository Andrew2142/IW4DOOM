// SPDX-License-Identifier: GPL-3.0-or-later
//! The synthetic fixture pack (T3.5, T3.6, T3.8, T3.9 and content checks).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;

use dmw_format::fixture::{self, facts as fixture_facts};
use dmw_format::validate;
use dmw_format::{DmwWeaponId, Pack, PartRole};

use common::{TempDir, all_ids, tree};

fn write_fixture(dir: &Path) {
    fixture::write_to_dir(dir).unwrap();
}

/// T3.5: the on-disk fixture opens to exactly what `Pack::fixture()` holds.
#[test]
fn disk_fixture_equals_memory_fixture() {
    let dir = TempDir::new("t35");
    write_fixture(dir.path());
    let disk = Pack::open(dir.path()).unwrap();
    let mem = Pack::fixture();

    assert_eq!(disk.manifest, mem.manifest);
    assert_eq!(disk.weapon_count(), 5);
    for id in all_ids(&mem) {
        assert_eq!(
            disk.facts(id),
            mem.facts(id),
            "facts {}",
            mem.facts(id).name
        );
        assert_eq!(
            *disk.load_viewmodel(id).unwrap(),
            *mem.load_viewmodel(id).unwrap(),
            "viewmodel {}",
            mem.facts(id).name
        );
        assert_eq!(
            disk.load_overlay(id).unwrap().as_deref(),
            mem.load_overlay(id).unwrap().as_deref(),
            "overlay {}",
            mem.facts(id).name
        );
    }
    for s in &mem.manifest.sounds {
        assert_eq!(
            *disk.load_sound(&s.alias).unwrap(),
            *mem.load_sound(&s.alias).unwrap(),
            "sound {}",
            s.alias
        );
    }
}

/// T3.6: two runs produce byte-identical trees.
#[test]
fn fixture_output_is_byte_identical() {
    let a = TempDir::new("t36a");
    let b = TempDir::new("t36b");
    write_fixture(a.path());
    write_fixture(b.path());
    let (ta, tb) = (tree(a.path()), tree(b.path()));
    assert_eq!(ta.len(), tb.len());
    for ((pa, ba), (pb, bb)) in ta.iter().zip(&tb) {
        assert_eq!(pa, pb);
        assert!(ba == bb, "{pa} differs between runs");
    }
    let paths: Vec<&str> = ta.iter().map(|(p, _)| p.as_str()).collect();
    for expected in [
        "manifest.json",
        "weapons/fixture_pistol/facts.json",
        "weapons/fixture_pistol/viewmodel.bin",
        "weapons/fixture_pistol/textures/0.png",
        "weapons/fixture_pistol/textures/3.png",
        "weapons/fixture_sniper/overlay.png",
        "sounds/fixture_fire_rifle/0.wav",
        "sounds/fixture_explode/0.wav",
    ] {
        assert!(paths.contains(&expected), "missing {expected}");
    }
    assert!(!paths.contains(&"weapons/fixture_pistol/overlay.png"));
    // 1 manifest + 5 x (facts + viewmodel + 4 textures) + 1 overlay + 11 sounds.
    assert_eq!(paths.len(), 1 + 5 * 6 + 1 + 11);
}

/// The in-memory builder is deterministic too.
#[test]
fn memory_fixture_is_deterministic() {
    assert_eq!(fixture::build(), fixture::build());
}

/// T3.8: names and aliases resolve; unknown names do not.
#[test]
fn weapon_id_resolves_names_and_aliases() {
    let pack = Pack::fixture();
    assert_eq!(pack.weapon_id("fixture_pistol"), Some(DmwWeaponId(1)));
    assert_eq!(pack.weapon_id("fixture_launcher"), Some(DmwWeaponId(5)));
    for (alias, target) in fixture_facts::ALIASES {
        assert_eq!(pack.weapon_id(alias), pack.weapon_id(target), "{alias}");
        assert!(pack.weapon_id(alias).is_some(), "{alias}");
    }
    // Every default roster name of 00 section 9.1 plus the exporter extras maps somewhere.
    for name in [
        "usp_mp",
        "spas12_mp",
        "ranger_mp",
        "m4_mp",
        "rpg_mp",
        "mg4_mp",
        "barrett_mp",
        "cheytac_mp",
        "aa12_mp",
        "kriss_mp",
        "deserteagle_mp",
    ] {
        assert!(pack.weapon_id(name).is_some(), "{name}");
    }
    assert_eq!(pack.weapon_id("nope_mp"), None);
    assert_eq!(pack.weapon_id(""), None);
    assert_eq!(pack.weapon_id("vanilla"), None);
    assert!(pack.try_facts(DmwWeaponId(0)).is_none());
    assert!(pack.try_facts(DmwWeaponId(6)).is_none());
    assert!(pack.load_viewmodel(DmwWeaponId(0)).is_err());
}

/// T3.9: golden manifest (set `BLESS=1` to rewrite it).
#[test]
fn manifest_matches_golden() {
    let mut json = serde_json::to_string_pretty(&Pack::fixture().manifest).unwrap();
    json.push('\n');
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fixture_manifest.json");
    if std::env::var_os("BLESS").is_some_and(|v| v == "1") {
        std::fs::write(&golden, &json).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("{}: {e}; run with BLESS=1 to create it", golden.display()));
    assert!(
        expected == json,
        "tests/golden/fixture_manifest.json is stale; review the change and rerun with BLESS=1"
    );
}

/// The fixture passes every validation rule, loads fully and has the content sheet 03 lists.
#[test]
fn fixture_content_is_complete_and_valid() {
    let pack = Pack::fixture();
    let facts: Vec<_> = pack.weapon_ids().map(|id| pack.facts(id).clone()).collect();
    validate::manifest(&pack.manifest, &facts).unwrap();
    assert!(validate::missing_sounds(&pack.manifest, &facts).is_empty());
    assert!(pack.manifest.fixture);
    assert_eq!(pack.manifest.created_unix, 0);

    let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "fixture_pistol",
            "fixture_rifle",
            "fixture_shotgun",
            "fixture_sniper",
            "fixture_launcher"
        ]
    );

    for id in pack.weapon_ids() {
        let f = pack.facts(id);
        let vm = pack.load_viewmodel(id).unwrap();
        let file = dmw_format::ViewmodelFile {
            skeleton: vm.skeleton.clone(),
            mesh: vm.mesh.clone(),
            clips: vm.clips.clone(),
        };
        validate::viewmodel(&f.name, &file, vm.textures.len() as u32).unwrap();
        assert_eq!(vm.textures.len(), 4);
        assert!(vm.textures.iter().all(|t| (t.width, t.height) == (64, 64)));
        assert_eq!(vm.skeleton.bones.len(), 8);
        assert_eq!(
            vm.skeleton.bones[usize::from(vm.skeleton.view_bone)].name,
            "tag_view"
        );
        assert_eq!(
            vm.skeleton.bones[usize::from(vm.skeleton.flash_bone.unwrap())].name,
            "tag_flash"
        );
        let tris = vm.mesh.indices.len() / 3;
        assert!((150..=300).contains(&tris), "{tris} triangles");
        assert!(vm.mesh.surfaces.iter().any(|s| s.role == PartRole::Hands));
        for alias in f.sounds.referenced_aliases() {
            pack.load_sound(alias).unwrap();
        }
        // Repeated loads return the cached Arc.
        assert!(std::sync::Arc::ptr_eq(
            &vm,
            &pack.load_viewmodel(id).unwrap()
        ));
    }

    // Overlay: sniper only.
    let sniper = pack.weapon_id("barrett_mp").unwrap();
    let overlay = pack.load_overlay(sniper).unwrap().unwrap();
    assert_eq!((overlay.width, overlay.height), (256, 192));
    assert!(pack.facts(sniper).has_overlay);
    assert!(
        pack.load_overlay(pack.weapon_id("m4_mp").unwrap())
            .unwrap()
            .is_none()
    );

    // Sounds: 22 050 Hz mono; the 11 aliases of section 5.9.
    assert_eq!(pack.manifest.sounds.len(), 11);
    for s in &pack.manifest.sounds {
        let a = pack.load_sound(&s.alias).unwrap();
        assert_eq!((a.rate, a.channels), (22050, 1));
        assert!(a.samples.iter().all(|x| (-1.0..=1.0).contains(x)));
    }
    assert!(pack.load_sound("no_such_alias").is_err());
    assert!(pack.load_sound_variant("fixture_empty", 1).is_err());
}

/// Per-weapon behaviours the sim/anim sheets rely on.
#[test]
fn fixture_weapons_exercise_their_paths() {
    use dmw_format::anim::slot;
    let pack = Pack::fixture();
    let by = |n: &str| pack.weapon_id(n).unwrap();

    let rifle = pack.load_viewmodel(by("m4_mp")).unwrap();
    assert_eq!(
        rifle.clips.slots[slot::FIRE],
        rifle.clips.slots[slot::ADS_FIRE]
    );
    assert!(
        rifle.clips.slots[slot::QUICK_RAISE].is_none(),
        "missing clips stay None"
    );

    let sniper = pack.load_viewmodel(by("barrett_mp")).unwrap();
    let rechamber = sniper.clips.clip_for_slot(slot::RECHAMBER).unwrap();
    let notes: Vec<&str> = rechamber.notifies.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(notes, ["bolt_back", "bolt_forward"]);
    assert_eq!(
        pack.facts(by("barrett_mp")).sounds.notetracks["bolt_back"],
        "fixture_bolt_back"
    );

    let shotgun = pack.load_viewmodel(by("spas12_mp")).unwrap();
    for s in [slot::RELOAD_START, slot::RELOAD, slot::RELOAD_END] {
        assert!(shotgun.clips.clip_for_slot(s).is_some());
    }

    let reload = rifle.clips.clip_for_slot(slot::RELOAD).unwrap();
    let times: Vec<f32> = reload
        .notifies
        .iter()
        .map(|n| n.time * reload.duration())
        .collect();
    assert!(
        (times[0] - 0.3).abs() < 1e-4 && (times[1] - 0.7).abs() < 1e-4,
        "{times:?}"
    );

    let launcher = pack.facts(by("rpg_mp"));
    assert!(launcher.projectile.is_some());
    assert_eq!(launcher.combat.weap_type, weapon_iw4::WEAPTYPE_PROJECTILE);
    // Aliased names resolve to the same data.
    assert_eq!(pack.facts(by("usp_mp")).name, "fixture_pistol");
}
