// SPDX-License-Identifier: GPL-3.0-or-later
//! The synthetic fixture pack (sheet 03 section 5.9).
//!
//! Five original, code-generated weapons (pistol, rifle, shotgun, sniper, launcher) that
//! exercise every feature of the format with no MW2 data: semi/full auto, spread class,
//! segmented reload, bolt action, scope overlay, hold breath, projectile, keyed clips with
//! notetracks, skinned meshes with blended vertices, alpha-tested textures and generated
//! sounds. The output is byte-identical on every run: no timestamps, no random seeds that
//! vary, `created_unix = 0`.
//!
//! [`crate::Pack::fixture`] builds the pack in memory; [`write_to_dir`] (used by the
//! `dmw-fixture` binary) writes the same data in the on-disk v1 layout.

use std::collections::BTreeMap;
use std::path::Path;

use crate::PACK_FORMAT_VERSION;
use crate::error::PackError;
use crate::facts::WeaponFacts;
use crate::load::ViewmodelAsset;
use crate::manifest::{
    PackManifest, PackSource, SoundEntry, SoundVariant, WeaponEntry, sanitize_alias,
};
use crate::model::ViewmodelFile;
use crate::texture::RgbaImage;

pub mod clips;
pub mod facts;
pub mod geometry;
pub mod media;

pub use media::FixtureSound;

/// Generator string recorded in the manifest.
pub const GENERATOR: &str = "dmw-fixture 0.1.0";

/// One fixture weapon with everything the pack stores for it.
#[derive(Clone, Debug, PartialEq)]
pub struct FixtureWeapon {
    /// `facts.json`.
    pub facts: WeaponFacts,
    /// `viewmodel.bin` plus textures.
    pub viewmodel: ViewmodelAsset,
    /// `overlay.png`, sniper only.
    pub overlay: Option<RgbaImage>,
}

/// The complete fixture pack contents.
#[derive(Clone, Debug, PartialEq)]
pub struct FixturePack {
    /// `manifest.json`.
    pub manifest: PackManifest,
    /// Weapons in manifest order.
    pub weapons: Vec<FixtureWeapon>,
    /// Sounds in manifest order (one variant each).
    pub sounds: Vec<FixtureSound>,
}

const KINDS: [clips::Kind; 5] = [
    clips::Kind::Pistol,
    clips::Kind::Rifle,
    clips::Kind::Shotgun,
    clips::Kind::Sniper,
    clips::Kind::Launcher,
];

/// Relative path of a sound variant file.
fn sound_file(alias: &str, variant: usize) -> String {
    format!("sounds/{}/{variant}.wav", sanitize_alias(alias))
}

/// Generates the whole fixture pack.
pub fn build() -> FixturePack {
    let skeleton = geometry::skeleton();
    let mesh = geometry::mesh();

    let mut weapons = Vec::new();
    let mut entries = Vec::new();
    for (i, kind) in KINDS.into_iter().enumerate() {
        let facts = facts::weapon(i);
        let overlay = facts.has_overlay.then(media::overlay);
        let textures = media::textures(i);
        entries.push(WeaponEntry {
            name: facts.name.clone(),
            dir: format!("weapons/{}", facts.name),
            has_viewmodel: true,
            has_overlay: overlay.is_some(),
            texture_count: textures.len() as u32,
        });
        weapons.push(FixtureWeapon {
            facts,
            viewmodel: ViewmodelAsset {
                mesh: mesh.clone(),
                skeleton: skeleton.clone(),
                clips: clips::clip_set(kind, &skeleton),
                textures,
            },
            overlay,
        });
    }

    let sounds = media::sounds();
    let sound_entries = sounds
        .iter()
        .map(|s| {
            let (distance_min, distance_max) = s.distance();
            SoundEntry {
                alias: s.alias.clone(),
                variants: vec![SoundVariant {
                    file: sound_file(&s.alias, 0),
                    volume: 1.0,
                    pitch: 1.0,
                    looping: false,
                    distance_min,
                    distance_max,
                }],
            }
        })
        .collect();

    let aliases: BTreeMap<String, String> = facts::ALIASES
        .iter()
        .map(|&(from, to)| (from.to_owned(), to.to_owned()))
        .collect();

    FixturePack {
        manifest: PackManifest {
            format_version: PACK_FORMAT_VERSION,
            generator: GENERATOR.to_owned(),
            iw4l_rev: "fixture".to_owned(),
            source: PackSource {
                game: "fixture".to_owned(),
                zone: "fixture".to_owned(),
                common_zone: None,
            },
            created_unix: 0,
            fixture: true,
            weapons: entries,
            sounds: sound_entries,
            aliases,
        },
        weapons,
        sounds,
    }
}

fn io_err(path: &Path, source: std::io::Error) -> PackError {
    PackError::Io {
        path: path.to_owned(),
        source,
    }
}

fn write_file(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), PackError> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    std::fs::write(&path, bytes).map_err(|e| io_err(&path, e))
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, PackError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| PackError::format(format!("json encode: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Writes the fixture pack to `dir` in the v1 layout (sheet 03 section 5.1).
///
/// Existing files with the same names are overwritten; nothing else in `dir` is touched.
pub fn write_to_dir(dir: &Path) -> Result<(), PackError> {
    let pack = build();
    write_file(dir, "manifest.json", &json_bytes(&pack.manifest)?)?;
    for (entry, w) in pack.manifest.weapons.iter().zip(&pack.weapons) {
        let base = &entry.dir;
        write_file(dir, &format!("{base}/facts.json"), &json_bytes(&w.facts)?)?;
        let file = ViewmodelFile {
            skeleton: w.viewmodel.skeleton.clone(),
            mesh: w.viewmodel.mesh.clone(),
            clips: w.viewmodel.clips.clone(),
        };
        write_file(dir, &format!("{base}/viewmodel.bin"), &file.to_bytes()?)?;
        for (n, texture) in w.viewmodel.textures.iter().enumerate() {
            write_file(
                dir,
                &format!("{base}/textures/{n}.png"),
                &crate::texture::encode_png(texture)?,
            )?;
        }
        if let Some(overlay) = &w.overlay {
            write_file(
                dir,
                &format!("{base}/overlay.png"),
                &crate::texture::encode_png(overlay)?,
            )?;
        }
    }
    for (entry, s) in pack.manifest.sounds.iter().zip(&pack.sounds) {
        let wav = crate::sound::encode_wav_i16(s.rate, 1, &s.pcm)?;
        write_file(dir, &entry.variants[0].file, &wav)?;
    }
    Ok(())
}
