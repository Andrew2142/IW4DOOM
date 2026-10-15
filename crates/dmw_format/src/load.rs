// SPDX-License-Identifier: GPL-3.0-or-later
//! [`Pack`]: opening a pack directory (or the in-memory fixture) and lazily loading assets
//! (sheet 03 section 5.8, 00 section 7.1).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::de::DeserializeOwned;

use crate::PACK_FORMAT_VERSION;
use crate::anim::ClipSet;
use crate::consts::DmwWeaponId;
use crate::error::PackError;
use crate::facts::WeaponFacts;
use crate::manifest::{PackManifest, SoundEntry, WeaponEntry};
use crate::model::{Skeleton, ViewmodelFile, ViewmodelMesh};
use crate::sound::SoundAsset;
use crate::texture::RgbaImage;
use crate::validate;

const LOG: &str = "dmw::pack";

/// A weapon's FPV model, skeleton, clips and textures, ready for sheets 10 and 11.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewmodelAsset {
    /// Skinned triangle mesh.
    pub mesh: ViewmodelMesh,
    /// Bones of the composed assembly.
    pub skeleton: Skeleton,
    /// Animation clips and the slot table.
    pub clips: ClipSet,
    /// `textures/<n>.png` in order; surfaces refer to them by index.
    pub textures: Vec<RgbaImage>,
}

/// Where a [`Pack`] reads its lazily loaded assets from.
#[derive(Debug)]
enum Store {
    /// Files below `Pack::dir`.
    Disk,
    /// Built in memory by the fixture generator; nothing touches the filesystem.
    Memory(MemoryAssets),
}

/// The assets of an in-memory pack.
#[derive(Debug, Default)]
pub(crate) struct MemoryAssets {
    /// Per weapon (same order as the manifest).
    pub viewmodels: Vec<Option<Arc<ViewmodelAsset>>>,
    /// Per weapon.
    pub overlays: Vec<Option<Arc<RgbaImage>>>,
    /// Per manifest sound entry, per variant.
    pub sounds: Vec<Vec<Arc<SoundAsset>>>,
}

#[derive(Debug, Default)]
struct Caches {
    viewmodels: RefCell<BTreeMap<u16, Arc<ViewmodelAsset>>>,
    overlays: RefCell<BTreeMap<u16, Option<Arc<RgbaImage>>>>,
    sounds: RefCell<BTreeMap<(usize, usize), Arc<SoundAsset>>>,
}

/// An opened pack.
///
/// Weapon facts are loaded eagerly; viewmodels, sounds and overlays are read on first use and
/// cached. The engine is single-threaded, so the caches are `RefCell`s and `Pack` is `!Sync`.
#[derive(Debug)]
pub struct Pack {
    /// The pack directory (`<fixture>` for [`Pack::fixture`]).
    pub dir: PathBuf,
    /// The parsed `manifest.json`.
    pub manifest: PackManifest,
    facts: Vec<WeaponFacts>,
    store: Store,
    cache: Caches,
}

/// Reads a file below the pack root, mapping "not found" to [`PackError::Missing`].
fn read_rel(dir: &Path, rel: &str) -> Result<Vec<u8>, PackError> {
    let path = dir.join(rel);
    std::fs::read(&path).map_err(|e| PackError::from_io(&path, rel, e))
}

fn parse_json<T: DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T, PackError> {
    serde_json::from_slice(bytes).map_err(|e| PackError::format(format!("{what}: {e}")))
}

impl Pack {
    /// Opens the pack at `dir`: parses `manifest.json`, checks its version, loads every
    /// weapon's `facts.json` and validates them (sheet 03 section 5.8).
    ///
    /// One invalid `facts.json` fails the whole pack; partial packs would hide exporter bugs.
    pub fn open(dir: &Path) -> Result<Pack, PackError> {
        #[derive(serde::Deserialize)]
        struct VersionProbe {
            format_version: u32,
        }

        let manifest_bytes = read_rel(dir, "manifest.json")?;
        let probe: VersionProbe = parse_json(&manifest_bytes, "manifest.json")?;
        if probe.format_version != PACK_FORMAT_VERSION {
            return Err(PackError::Version {
                found: probe.format_version,
                expected: PACK_FORMAT_VERSION,
            });
        }
        let manifest: PackManifest = parse_json(&manifest_bytes, "manifest.json")?;

        let mut facts = Vec::with_capacity(manifest.weapons.len());
        for entry in &manifest.weapons {
            if !validate::is_safe_rel_path(&entry.dir) {
                return Err(PackError::invalid(format!(
                    "weapon '{}' manifest.json: dir '{}' is not a safe relative path",
                    entry.name, entry.dir
                )));
            }
            let rel = format!("{}/facts.json", entry.dir);
            let bytes = read_rel(dir, &rel).map_err(|e| with_weapon(&entry.name, e))?;
            let f: WeaponFacts = parse_json(&bytes, &format!("weapon '{}' {rel}", entry.name))?;
            facts.push(f);
        }
        validate::manifest(&manifest, &facts)?;
        Ok(Self::finish(dir.to_owned(), manifest, facts, Store::Disk))
    }

    /// Builds the in-memory synthetic fixture pack (sheet 03 section 5.9). Never touches the
    /// filesystem and cannot fail: the generator's output is covered by tests.
    pub fn fixture() -> Pack {
        let built = crate::fixture::build();
        let mut facts = Vec::with_capacity(built.weapons.len());
        let mut memory = MemoryAssets::default();
        for w in built.weapons {
            facts.push(w.facts);
            memory.viewmodels.push(Some(Arc::new(w.viewmodel)));
            memory.overlays.push(w.overlay.map(Arc::new));
        }
        for s in built.sounds {
            memory.sounds.push(vec![Arc::new(s.to_asset())]);
        }
        Self::finish(
            PathBuf::from("<fixture>"),
            built.manifest,
            facts,
            Store::Memory(memory),
        )
    }

    fn finish(dir: PathBuf, manifest: PackManifest, facts: Vec<WeaponFacts>, store: Store) -> Pack {
        let pack = Pack {
            dir,
            manifest,
            facts,
            store,
            cache: Caches::default(),
        };
        let mut warned = BTreeSet::new();
        for (weapon, alias) in validate::missing_sounds(&pack.manifest, &pack.facts) {
            if warned.insert(alias.clone()) {
                log::warn!(target: LOG, "weapon {weapon} references sound alias {alias} which the pack does not contain");
            }
        }
        log::info!(
            target: LOG,
            "pack {}: {} weapons, {} sound aliases, generator {}, iw4l {}{}",
            pack.dir.display(),
            pack.manifest.weapons.len(),
            pack.manifest.sounds.len(),
            pack.manifest.generator,
            pack.manifest.iw4l_rev,
            if pack.manifest.fixture { " (fixture)" } else { "" }
        );
        pack
    }

    /// Resolves a weapon name, first against `weapons[].name`, then against `aliases`.
    pub fn weapon_id(&self, name: &str) -> Option<DmwWeaponId> {
        let position = |n: &str| self.manifest.weapons.iter().position(|w| w.name == n);
        let index = position(name).or_else(|| position(self.manifest.aliases.get(name)?))?;
        u16::try_from(index + 1).ok().map(DmwWeaponId)
    }

    /// Number of weapons in the pack.
    pub fn weapon_count(&self) -> usize {
        self.manifest.weapons.len()
    }

    /// Every weapon id in manifest order.
    pub fn weapon_ids(&self) -> impl Iterator<Item = DmwWeaponId> + use<> {
        let count = self.manifest.weapons.len();
        (1..=count).filter_map(|i| u16::try_from(i).ok().map(DmwWeaponId))
    }

    fn index(&self, id: DmwWeaponId) -> Option<usize> {
        usize::from(id.0)
            .checked_sub(1)
            .filter(|&i| i < self.manifest.weapons.len())
    }

    /// The manifest entry of `id`.
    pub fn entry(&self, id: DmwWeaponId) -> Option<&WeaponEntry> {
        self.manifest.weapons.get(self.index(id)?)
    }

    /// The facts of `id`, or `None` when `id` does not belong to this pack.
    pub fn try_facts(&self, id: DmwWeaponId) -> Option<&WeaponFacts> {
        self.facts.get(self.index(id)?)
    }

    /// The facts of `id` (loaded eagerly at open).
    ///
    /// # Panics
    ///
    /// Panics when `id` was not produced by this pack (0 or past the last weapon). Ids only
    /// come from [`Pack::weapon_id`] and [`Pack::weapon_ids`]; use [`Pack::try_facts`] for
    /// untrusted values.
    #[allow(clippy::panic)]
    pub fn facts(&self, id: DmwWeaponId) -> &WeaponFacts {
        match self.try_facts(id) {
            Some(f) => f,
            None => panic!(
                "weapon id {} is not in this pack ({} weapons)",
                id.0,
                self.facts.len()
            ),
        }
    }

    /// The manifest entry of the sound alias `alias`.
    pub fn sound_entry(&self, alias: &str) -> Option<&SoundEntry> {
        self.manifest.sounds.iter().find(|s| s.alias == alias)
    }

    /// Loads (once) the viewmodel of `id`: mesh, skeleton, clips and textures.
    ///
    /// Returns [`PackError::Missing`] when the weapon has no viewmodel or `viewmodel.bin` is
    /// absent; the engine then treats the weapon as unusable (sheet 08 section 6).
    pub fn load_viewmodel(&self, id: DmwWeaponId) -> Result<Arc<ViewmodelAsset>, PackError> {
        let (index, entry) = self.index_and_entry(id)?;
        if let Some(hit) = self.cache.viewmodels.borrow().get(&id.0) {
            return Ok(Arc::clone(hit));
        }
        let started = Instant::now();
        let asset = match &self.store {
            Store::Memory(mem) => mem
                .viewmodels
                .get(index)
                .and_then(Clone::clone)
                .ok_or_else(|| {
                    PackError::Missing(format!("weapon '{}' has no viewmodel", entry.name))
                })?,
            Store::Disk => {
                if !entry.has_viewmodel {
                    return Err(PackError::Missing(format!(
                        "weapon '{}' has no viewmodel",
                        entry.name
                    )));
                }
                Arc::new(self.read_viewmodel(entry)?)
            }
        };
        log::debug!(
            target: LOG,
            "loaded viewmodel {} ({} vertices, {} textures) in {:?}",
            entry.name,
            asset.mesh.positions.len(),
            asset.textures.len(),
            started.elapsed()
        );
        self.cache
            .viewmodels
            .borrow_mut()
            .insert(id.0, Arc::clone(&asset));
        Ok(asset)
    }

    fn read_viewmodel(&self, entry: &WeaponEntry) -> Result<ViewmodelAsset, PackError> {
        let rel = format!("{}/viewmodel.bin", entry.dir);
        let bytes = read_rel(&self.dir, &rel)?;
        let file = ViewmodelFile::from_bytes(&bytes, &format!("weapon '{}' {rel}", entry.name))?;
        validate::viewmodel(&entry.name, &file, entry.texture_count)?;
        let mut textures = Vec::with_capacity(entry.texture_count as usize);
        for n in 0..entry.texture_count {
            let rel = format!("{}/textures/{n}.png", entry.dir);
            let path = self.dir.join(&rel);
            let png = std::fs::read(&path).map_err(|e| PackError::from_io(&path, &rel, e))?;
            textures.push(crate::texture::decode_png(&png, &rel)?);
        }
        let ViewmodelFile {
            skeleton,
            mesh,
            clips,
        } = file;
        Ok(ViewmodelAsset {
            mesh,
            skeleton,
            clips,
            textures,
        })
    }

    /// Loads (once) the first variant of the sound alias `alias`.
    ///
    /// Use [`Pack::sound_entry`] for the variant list and their volume, pitch and distance
    /// parameters, and [`Pack::load_sound_variant`] for the others.
    pub fn load_sound(&self, alias: &str) -> Result<Arc<SoundAsset>, PackError> {
        self.load_sound_variant(alias, 0)
    }

    /// Loads (once) variant `variant` of the sound alias `alias`.
    pub fn load_sound_variant(
        &self,
        alias: &str,
        variant: usize,
    ) -> Result<Arc<SoundAsset>, PackError> {
        let Some(entry_index) = self.manifest.sounds.iter().position(|s| s.alias == alias) else {
            return Err(PackError::Missing(format!("sound alias '{alias}'")));
        };
        let entry = &self.manifest.sounds[entry_index];
        let Some(v) = entry.variants.get(variant) else {
            return Err(PackError::Missing(format!(
                "sound alias '{alias}' variant {variant} (it has {})",
                entry.variants.len()
            )));
        };
        if let Some(hit) = self.cache.sounds.borrow().get(&(entry_index, variant)) {
            return Ok(Arc::clone(hit));
        }
        let started = Instant::now();
        let asset = match &self.store {
            Store::Memory(mem) => mem
                .sounds
                .get(entry_index)
                .and_then(|vs| vs.get(variant))
                .cloned()
                .ok_or_else(|| PackError::Missing(format!("sound alias '{alias}'")))?,
            Store::Disk => {
                let path = self.dir.join(&v.file);
                let bytes =
                    std::fs::read(&path).map_err(|e| PackError::from_io(&path, &v.file, e))?;
                Arc::new(crate::sound::decode_wav(&bytes, &v.file)?)
            }
        };
        log::debug!(
            target: LOG,
            "loaded sound {alias}[{variant}] ({} samples) in {:?}",
            asset.samples.len(),
            started.elapsed()
        );
        self.cache
            .sounds
            .borrow_mut()
            .insert((entry_index, variant), Arc::clone(&asset));
        Ok(asset)
    }

    /// Loads (once) the scope overlay image of `id`; `Ok(None)` when the weapon has none.
    pub fn load_overlay(&self, id: DmwWeaponId) -> Result<Option<Arc<RgbaImage>>, PackError> {
        let (index, entry) = self.index_and_entry(id)?;
        if !entry.has_overlay {
            return Ok(None);
        }
        if let Some(hit) = self.cache.overlays.borrow().get(&id.0) {
            return Ok(hit.clone());
        }
        let image = match &self.store {
            Store::Memory(mem) => mem.overlays.get(index).and_then(Clone::clone),
            Store::Disk => {
                let rel = format!("{}/overlay.png", entry.dir);
                let path = self.dir.join(&rel);
                let png = std::fs::read(&path).map_err(|e| PackError::from_io(&path, &rel, e))?;
                Some(Arc::new(crate::texture::decode_png(&png, &rel)?))
            }
        };
        self.cache.overlays.borrow_mut().insert(id.0, image.clone());
        Ok(image)
    }

    fn index_and_entry(&self, id: DmwWeaponId) -> Result<(usize, &WeaponEntry), PackError> {
        self.index(id)
            .and_then(|i| self.manifest.weapons.get(i).map(|e| (i, e)))
            .ok_or_else(|| PackError::Missing(format!("weapon id {}", id.0)))
    }
}

/// Adds the weapon name to a `Missing`/`Format` error that does not already carry it.
fn with_weapon(weapon: &str, e: PackError) -> PackError {
    match e {
        PackError::Missing(what) => PackError::Missing(format!("weapon '{weapon}': {what}")),
        other => other,
    }
}
