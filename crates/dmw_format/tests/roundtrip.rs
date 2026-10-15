// SPDX-License-Identifier: GPL-3.0-or-later
//! Serialization round trips (T3.2, T3.3 at the facts level, T3.4).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use dmw_format::anim::{
    ClipData, FrameIndicesData, KeyedQuat, KeyedVec3, NotifyData, RotationData, TrackData,
    TranslationData,
};
use dmw_format::remote::combat_facts_all_distinct;
use dmw_format::{Pack, ViewmodelFile, WeaponFacts};

use common::fixture_file;

/// T3.4: `ViewmodelFile` postcard round trip equality on the fixture.
#[test]
fn viewmodel_file_round_trips_for_every_fixture_weapon() {
    let pack = Pack::fixture();
    for id in pack.weapon_ids() {
        let name = pack.facts(id).name.clone();
        let file = fixture_file(&name);
        let bytes = file.to_bytes().unwrap();
        assert_eq!(&bytes[..4], b"DMWV");
        let back = ViewmodelFile::from_bytes(&bytes, &name).unwrap();
        assert_eq!(back, file, "{name}");
        // Encoding is deterministic.
        assert_eq!(back.to_bytes().unwrap(), bytes);
    }
}

/// T3.2 at the `WeaponFacts` level: every fixture weapon, and a weapon whose combat block has
/// every field set to a distinct value, survive `facts.json`.
#[test]
fn weapon_facts_round_trip_through_json() {
    let pack = Pack::fixture();
    let mut all: Vec<WeaponFacts> = pack.weapon_ids().map(|id| pack.facts(id).clone()).collect();
    let mut distinct = all[0].clone();
    distinct.combat = combat_facts_all_distinct();
    distinct.fpv.scope_zoom = distinct.combat.scope_zoom;
    all.push(distinct);
    for f in all {
        let json = serde_json::to_vec_pretty(&f).unwrap();
        let back: WeaponFacts = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, f, "{}", f.name);
    }
}

/// A clip that uses every channel variant, dense and sparse keys, notifies and a delta.
fn every_variant_clip() -> ClipData {
    let q = |x: f32| [x, 0.0, 0.0, (1.0 - x * x).sqrt()];
    let kq = |frames| KeyedQuat {
        values: vec![q(0.0), q(0.1), q(0.2)],
        frames,
    };
    let kv = |frames| KeyedVec3 {
        values: vec![[0.0; 3], [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        frames,
    };
    let sparse = FrameIndicesData::Sparse(vec![0, 3, 9]);
    ClipData {
        name: "every".into(),
        framerate: 24.0,
        numframes: 9,
        looping: true,
        tracks: vec![
            TrackData {
                name: "a".into(),
                rotation: RotationData::Default,
                translation: TranslationData::Default,
            },
            TrackData {
                name: "b".into(),
                rotation: RotationData::HalfKeyed(kq(FrameIndicesData::Dense)),
                translation: TranslationData::SmallKeyed(kv(sparse.clone())),
            },
            TrackData {
                name: "c".into(),
                rotation: RotationData::FullKeyed(kq(sparse)),
                translation: TranslationData::FullKeyed(kv(FrameIndicesData::Dense)),
            },
            TrackData {
                name: "d".into(),
                rotation: RotationData::HalfConstant(q(0.3)),
                translation: TranslationData::Constant([7.0, 8.0, 9.0]),
            },
            TrackData {
                name: "e".into(),
                rotation: RotationData::FullConstant(q(0.4)),
                translation: TranslationData::Default,
            },
        ],
        notifies: vec![
            NotifyData {
                name: "first".into(),
                time: 0.25,
            },
            NotifyData {
                name: "second".into(),
                time: 0.75,
            },
        ],
        delta_translation: TranslationData::SmallKeyed(kv(FrameIndicesData::Dense)),
    }
}

#[test]
fn clip_data_round_trips_every_variant() {
    let clip = every_variant_clip();
    let bytes = postcard::to_allocvec(&clip).unwrap();
    assert_eq!(postcard::from_bytes::<ClipData>(&bytes).unwrap(), clip);
}
