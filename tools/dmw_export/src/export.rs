// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-weapon export and pack assembly (sheet 04 §5.4–5.10).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use asset_core::AssetNamespace;
use asset_transport::{NamespaceSoundIwd, NamespaceTrees};
use dmw_format::{
    PACK_FORMAT_VERSION, Pack, PackManifest, PackSource, RgbaImage, SoundEntry, ViewmodelFile,
    WeaponEntry, WeaponFacts,
};

use crate::facts::AliasRef;
use crate::load::Loaded;
use crate::report::{ClipReport, Report, SurfaceReport, TextureReport, WeaponReport};
use crate::sounds::{self, Skipped};
use crate::textures::{self, ExportedTexture, TextureSet};
use crate::write::{self, WriteError};
use crate::{clips, facts, viewmodel};

/// Sounds exported even though no weapon references them (sheet 12 hitmarker, 00 Revision 7).
pub const EXTRA_SOUNDS: [&str; 1] = ["mp_hit_alert"];

/// Everything one weapon contributes to the pack.
#[derive(Debug)]
pub struct WeaponOutput {
    pub facts: WeaponFacts,
    pub viewmodel: Option<ViewmodelFile>,
    pub textures: Vec<ExportedTexture>,
    pub overlay: Option<RgbaImage>,
    pub aliases: Vec<AliasRef>,
    pub report: WeaponReport,
}

/// Resolves and decodes the ADS overlay image, the way iw4L `hud/src/iris.rs` finds it
/// (`overlay_image_of` → UI image publication / IWD archives).
fn overlay_image(loaded: &Loaded, index: u32, warnings: &mut Vec<String>) -> Option<RgbaImage> {
    let registry = &loaded.prepared.weapons;
    let material = registry.overlay_material_of(index)?;
    if registry.overlay_is_hud_iris(index) {
        return None;
    }
    let Some(image_name) = registry.overlay_image_of(index) else {
        warnings.push(format!("overlay material '{material}' names no image"));
        return None;
    };
    let ns = registry
        .component_namespace_of(index, asset_game::WeaponComponent::OverlayImage)
        .unwrap_or(AssetNamespace::Iw4);
    let key = format!("{}:material/{image_name}", ns.as_str());
    let decoded = match asset_material::decode_ui_image(&loaded.prepared.ui_images, &loaded.games.0, &key) {
        Ok(Some(img)) => Some(img),
        Ok(None) => asset_material::decode_ui_archive_image(&loaded.games.0, image_name)
            .ok()
            .flatten(),
        Err(e) => {
            warnings.push(format!("overlay '{key}': {e}"));
            None
        }
    };
    let Some((w, h, rgba)) = decoded else {
        warnings.push(format!("overlay image '{image_name}' not found"));
        return None;
    };
    let image = textures::image_from_rgba(w, h, &rgba)?;
    Some(textures::downscale_to(image, textures::MAX_TEXTURE_EDGE))
}

/// Exports weapon row `index` (requested as `name`).
pub fn export_weapon(loaded: &Loaded, name: &str, index: u32, axis_hands: bool) -> Result<WeaponOutput, (String, WeaponReport)> {
    let p = &loaded.prepared;
    let registry = &p.weapons;
    let mut report = WeaponReport::new(name);
    let extracted = match facts::extract(registry, index, p) {
        Ok(e) => e,
        Err(e) => return Err((e, report)),
    };
    let mut facts = extracted.facts;
    report.name = facts.name.clone();
    report.display_name = facts.display_name.clone();
    report.warnings.extend(extracted.warnings);

    let mut texture_set = TextureSet::default();
    let mut viewmodel_file = None;
    let sides = registry.fpv_assemblies_of(index, axis_hands);
    match (extracted.has_fpv, sides) {
        (true, Some(sides)) => {
            let (which, assembly) = viewmodel::choose_assembly(sides);
            report.assembly = which.to_owned();
            let (skeleton, sk_warn) = viewmodel::skeleton(assembly).map_err(|e| (e, report.clone()))?;
            report.warnings.extend(sk_warn);
            let (mesh, counts) = viewmodel::flatten(
                assembly,
                &p.fpv_meshes,
                &p.materials.population,
                &mut texture_set,
                &mut report.warnings,
            )
            .map_err(|e| (e, report.clone()))?;
            report.surfaces = SurfaceReport {
                exported: counts.exported,
                hidden: counts.hidden,
                untextured: counts.untextured,
                no_material: counts.no_material,
            };
            let exported = clips::export(registry, index, &p.xanims, assembly).map_err(|e| (e, report.clone()))?;
            report.clips = ClipReport {
                slots_bound: exported.slots_bound,
                unique: u32::try_from(exported.set.clips.len()).unwrap_or(u32::MAX),
                dropped: exported.dropped,
            };
            viewmodel_file = Some(ViewmodelFile {
                skeleton,
                mesh,
                clips: exported.set,
            });
        }
        (true, None) => {
            report.warnings.push(format!(
                "no FPV assembly: {}",
                registry.fpv_assembly_gap_of(index, axis_hands)
            ));
        }
        (false, _) => {}
    }
    report.warnings.append(&mut texture_set.warnings);
    report.textures = texture_set
        .textures
        .iter()
        .enumerate()
        .map(|(n, t)| TextureReport {
            n: u32::try_from(n).unwrap_or(u32::MAX),
            w: t.image.width,
            h: t.image.height,
            orig_w: t.orig_w,
            orig_h: t.orig_h,
            source: t.source.clone(),
        })
        .collect();
    let overlay = overlay_image(loaded, index, &mut report.warnings);
    if facts.fpv.overlay_reticle != 0 && overlay.is_none() {
        report.warnings.push("overlay_reticle set but no overlay image exported".to_owned());
    }
    facts.has_overlay = overlay.is_some();
    report.has_overlay = facts.has_overlay;
    if let Some(vm) = &viewmodel_file {
        let count = u32::try_from(texture_set.textures.len()).unwrap_or(u32::MAX);
        if let Err(e) = dmw_format::validate::viewmodel(&facts.name, vm, count) {
            return Err((format!("validation: {e}"), report));
        }
    }
    if let Err(e) = dmw_format::validate::weapon_facts(&facts.name, &facts) {
        return Err((format!("validation: {e}"), report));
    }
    report.ok = true;
    Ok(WeaponOutput {
        facts,
        viewmodel: viewmodel_file,
        textures: texture_set.textures,
        overlay,
        aliases: extracted.aliases,
        report,
    })
}

/// Exported sounds for the whole pack.
#[derive(Debug, Default)]
pub struct PackSounds {
    pub entries: Vec<SoundEntry>,
    pub files: Vec<(String, Vec<u8>)>,
    pub skipped: Vec<Skipped>,
    pub extras: Vec<String>,
}

/// Exports the union of every weapon's aliases plus [`EXTRA_SOUNDS`].
pub fn export_sounds(loaded: &Loaded, weapons: &[WeaponOutput]) -> PackSounds {
    let mut out = PackSounds::default();
    let Some(Ok(catalog)) = loaded.prepared.sound.as_ref() else {
        out.skipped.push(Skipped {
            alias: "*".to_owned(),
            variant: 0,
            reason: "the load produced no sound catalog".to_owned(),
        });
        return out;
    };
    let (iwd, _lines) = NamespaceSoundIwd::open(&NamespaceTrees::discover(&loaded.games));
    let mut wanted: BTreeSet<AliasRef> = weapons.iter().flat_map(|w| w.aliases.iter().cloned()).collect();
    for extra in EXTRA_SOUNDS {
        if let Some(s) = catalog.sound_in(AssetNamespace::Iw4, extra) {
            out.extras.push(s.name.clone());
            wanted.insert(AliasRef {
                ns: AssetNamespace::Iw4,
                alias: s.name.clone(),
            });
        }
    }
    let mut used = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for r in wanted {
        if !seen.insert(r.alias.clone()) {
            continue;
        }
        let folder = sounds::folder_for(&r.alias, &mut used);
        if let Some(s) = sounds::export_alias(catalog, &iwd, r.ns, &r.alias, &folder, &mut out.skipped) {
            out.entries.push(s.entry);
            out.files.extend(s.files);
        }
    }
    out
}

/// Fills the per-weapon sound counters from the pack-wide sound result.
pub fn attach_sound_reports(weapons: &mut [WeaponOutput], sounds: &PackSounds) {
    let present: BTreeSet<&str> = sounds.entries.iter().map(|e| e.alias.as_str()).collect();
    for w in weapons {
        let mine: BTreeSet<&str> = w.aliases.iter().map(|a| a.alias.as_str()).collect();
        w.report.sounds.exported =
            u32::try_from(mine.iter().filter(|a| present.contains(*a)).count()).unwrap_or(u32::MAX);
        w.report.sounds.skipped = sounds
            .skipped
            .iter()
            .filter(|s| mine.contains(s.alias.as_str()))
            .cloned()
            .collect();
    }
}

/// Builds the manifest for `weapons` in roster order.
pub fn manifest(loaded: &Loaded, weapons: &[WeaponOutput], sounds: &PackSounds, created_unix: u64) -> PackManifest {
    PackManifest {
        format_version: PACK_FORMAT_VERSION,
        generator: crate::generator(),
        iw4l_rev: crate::iw4l_rev(),
        source: PackSource {
            game: "iw4".to_owned(),
            zone: loaded.zone.clone(),
            common_zone: loaded.common_zone.clone(),
        },
        created_unix,
        fixture: false,
        weapons: weapons
            .iter()
            .map(|w| WeaponEntry {
                name: w.facts.name.clone(),
                dir: format!("weapons/{}", w.facts.name),
                has_viewmodel: w.viewmodel.is_some(),
                has_overlay: w.overlay.is_some(),
                texture_count: u32::try_from(w.textures.len()).unwrap_or(u32::MAX),
            })
            .collect(),
        sounds: sounds.entries.clone(),
        aliases: BTreeMap::new(),
    }
}

fn json<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, String> {
    let mut b = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    b.push(b'\n');
    Ok(b)
}

/// Writes every pack file into `root` (the temporary directory of [`write::write_atomic`]).
pub fn fill(root: &Path, manifest: &PackManifest, weapons: &[WeaponOutput], sounds: &PackSounds, report: &Report) -> Result<(), String> {
    let w = |rel: &str, bytes: &[u8]| write::write_file(root, rel, bytes).map_err(|e| e.to_string());
    w("manifest.json", &json(manifest)?)?;
    for (entry, out) in manifest.weapons.iter().zip(weapons) {
        w(&format!("{}/facts.json", entry.dir), &json(&out.facts)?)?;
        if let Some(vm) = &out.viewmodel {
            w(&format!("{}/viewmodel.bin", entry.dir), &vm.to_bytes().map_err(|e| e.to_string())?)?;
        }
        for (n, t) in out.textures.iter().enumerate() {
            let png = dmw_format::texture::encode_png(&t.image).map_err(|e| e.to_string())?;
            w(&format!("{}/textures/{n}.png", entry.dir), &png)?;
        }
        if let Some(o) = &out.overlay {
            let png = dmw_format::texture::encode_png(o).map_err(|e| e.to_string())?;
            w(&format!("{}/overlay.png", entry.dir), &png)?;
        }
    }
    for (rel, bytes) in &sounds.files {
        w(rel, bytes)?;
    }
    w("report.json", &report.to_json())
}

/// Opens the written pack and performs every lazy load (§5.9).
pub fn verify(root: &Path) -> Result<(), String> {
    let pack = Pack::open(root).map_err(|e| e.to_string())?;
    for id in pack.weapon_ids() {
        let entry = pack.entry(id).ok_or("weapon id without entry")?;
        if entry.has_viewmodel {
            pack.load_viewmodel(id).map_err(|e| e.to_string())?;
        }
        pack.load_overlay(id).map_err(|e| e.to_string())?;
    }
    for s in &pack.manifest.sounds {
        for v in 0..s.variants.len() {
            pack.load_sound_variant(&s.alias, v).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Writes the pack atomically to `out`.
pub fn write_pack(out: &Path, manifest: &PackManifest, weapons: &[WeaponOutput], sounds: &PackSounds, report: &Report) -> Result<(), WriteError> {
    write::write_atomic(out, |root| fill(root, manifest, weapons, sounds, report), verify)
}
