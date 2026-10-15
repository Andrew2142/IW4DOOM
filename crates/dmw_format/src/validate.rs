// SPDX-License-Identifier: GPL-3.0-or-later
//! Structural validation (sheet 03 section 5.8).
//!
//! Every failure is a [`PackError::Invalid`] whose message names the weapon, the file and the
//! field. The exporter runs these on its result before writing; [`crate::Pack::open`] runs
//! [`manifest`] (which includes the per-weapon facts checks) and [`crate::Pack::load_viewmodel`]
//! runs [`viewmodel`].

use std::collections::BTreeSet;

use crate::anim::{
    ClipData, ClipSet, FrameIndicesData, KeyedQuat, KeyedVec3, RotationData, TranslationData,
    WEAPON_ANIM_SLOTS,
};
use crate::error::PackError;
use crate::facts::WeaponFacts;
use crate::manifest::PackManifest;
use crate::model::{Skeleton, ViewmodelFile, ViewmodelMesh};

/// Maximum bones in a skeleton (`anim_iw4::PartBits::CAPACITY`).
pub const MAX_BONES: usize = 192;

/// Tolerance for "unit length" quaternion checks.
const QUAT_TOLERANCE: f32 = 1e-3;
/// Tolerance for "weights sum to one" checks.
const WEIGHT_TOLERANCE: f32 = 1e-3;

fn invalid(msg: String) -> PackError {
    PackError::Invalid(msg)
}

/// `true` for a relative path with `/` separators and no empty, `.` or `..` segment.
///
/// Manifest paths are joined onto the pack directory, so anything else could escape it.
pub fn is_safe_rel_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Validates the manifest and every weapon's facts together.
///
/// `facts[i]` must be the facts of `manifest.weapons[i]`.
pub fn manifest(manifest: &PackManifest, facts: &[WeaponFacts]) -> Result<(), PackError> {
    if manifest.weapons.len() != facts.len() {
        return Err(invalid(format!(
            "manifest.json: {} weapons but {} facts",
            manifest.weapons.len(),
            facts.len()
        )));
    }
    if manifest.weapons.len() >= usize::from(u16::MAX) {
        return Err(invalid(
            "manifest.json: too many weapons for a 16-bit id".to_owned(),
        ));
    }
    let mut names = BTreeSet::new();
    for (entry, f) in manifest.weapons.iter().zip(facts) {
        let w = &entry.name;
        if !is_safe_rel_path(w) || w.contains('/') {
            return Err(invalid(format!(
                "manifest.json: weapon name '{w}' is not a plain file name"
            )));
        }
        if !names.insert(w.as_str()) {
            return Err(invalid(format!("manifest.json: duplicate weapon '{w}'")));
        }
        if !is_safe_rel_path(&entry.dir) {
            return Err(invalid(format!(
                "weapon '{w}' manifest.json: dir '{}' is not a safe relative path",
                entry.dir
            )));
        }
        if f.name != *w {
            return Err(invalid(format!(
                "weapon '{w}' facts.json: name is '{}', manifest says '{w}'",
                f.name
            )));
        }
        if f.has_overlay != entry.has_overlay {
            return Err(invalid(format!(
                "weapon '{w}': facts.has_overlay ({}) disagrees with manifest has_overlay ({})",
                f.has_overlay, entry.has_overlay
            )));
        }
        weapon_facts(w, f)?;
    }

    let mut aliases = BTreeSet::new();
    for s in &manifest.sounds {
        if s.alias.is_empty() || !aliases.insert(s.alias.as_str()) {
            return Err(invalid(format!(
                "manifest.json: sound alias '{}' is empty or duplicated",
                s.alias
            )));
        }
        if s.variants.is_empty() {
            return Err(invalid(format!(
                "manifest.json: sound '{}' has no variants",
                s.alias
            )));
        }
        for v in &s.variants {
            if !is_safe_rel_path(&v.file) {
                return Err(invalid(format!(
                    "manifest.json: sound '{}' file '{}' is not a safe relative path",
                    s.alias, v.file
                )));
            }
            let finite = [v.volume, v.pitch, v.distance_min, v.distance_max]
                .iter()
                .all(|x| x.is_finite() && *x >= 0.0);
            if !finite || v.pitch <= 0.0 {
                return Err(invalid(format!(
                    "manifest.json: sound '{}' has a negative, non-finite or zero-pitch parameter",
                    s.alias
                )));
            }
        }
    }
    for (requested, target) in &manifest.aliases {
        if requested.is_empty() || !names.contains(target.as_str()) {
            return Err(invalid(format!(
                "manifest.json: alias '{requested}' points at unknown weapon '{target}'"
            )));
        }
    }
    Ok(())
}

/// Sound aliases referenced by a weapon's facts that the manifest does not provide, as
/// `(weapon, alias)`. Missing sounds are not fatal: the caller logs a warning.
pub fn missing_sounds(manifest: &PackManifest, facts: &[WeaponFacts]) -> Vec<(String, String)> {
    let have: BTreeSet<&str> = manifest.sounds.iter().map(|s| s.alias.as_str()).collect();
    let mut out = Vec::new();
    for f in facts {
        for alias in f.sounds.referenced_aliases() {
            if !have.contains(alias) {
                out.push((f.name.clone(), alias.to_owned()));
            }
        }
    }
    out
}

/// Validates one weapon's `facts.json`.
pub fn weapon_facts(weapon: &str, f: &WeaponFacts) -> Result<(), PackError> {
    combat(weapon, &f.combat)?;
    if let Some(p) = &f.projectile {
        if p.projectile_speed <= 0 {
            return Err(invalid(format!(
                "weapon '{weapon}' facts.json: projectile present but projectile_speed is {}",
                p.projectile_speed
            )));
        }
        if p.explosion_radius < 0 || p.explosion_radius_min < 0 {
            return Err(invalid(format!(
                "weapon '{weapon}' facts.json: projectile.explosion_radius is negative"
            )));
        }
    }
    Ok(())
}

/// The combat sanity rules of section 5.8.
pub fn combat(weapon: &str, c: &weapon_iw4::WeaponCombatFacts) -> Result<(), PackError> {
    let bad = |field: &str, why: String| {
        invalid(format!(
            "weapon '{weapon}' facts.json: combat.{field} {why}"
        ))
    };
    if !(0..=5).contains(&c.fire_type) {
        return Err(bad(
            "fire_type",
            format!("is {}, expected 0..=5", c.fire_type),
        ));
    }
    let shoots = c.weap_type == weapon_iw4::WEAPTYPE_BULLET
        || c.weap_type == weapon_iw4::WEAPTYPE_PROJECTILE;
    if shoots && c.fire_time_ms <= 0 {
        return Err(bad(
            "fire_time_ms",
            format!("is {}, must be > 0", c.fire_time_ms),
        ));
    }
    if c.clip_size < 1 {
        return Err(bad(
            "clip_size",
            format!("is {}, must be >= 1", c.clip_size),
        ));
    }
    if c.damage < 0 {
        return Err(bad("damage", format!("is {}, must be >= 0", c.damage)));
    }
    if c.min_damage_range > 0.0
        && c.max_damage_range > 0.0
        && c.min_damage_range < c.max_damage_range
    {
        return Err(bad(
            "min_damage_range",
            format!(
                "({}) is below max_damage_range ({}); falloff must end after it starts",
                c.min_damage_range, c.max_damage_range
            ),
        ));
    }
    if !c.max_damage_range.is_finite() || !c.min_damage_range.is_finite() {
        return Err(bad("damage_range", "is not finite".to_owned()));
    }
    if !weapon_iw4::location_damage_is_valid(&c.location_damage) {
        return Err(bad(
            "location_damage",
            "has a non-finite or negative entry".to_owned(),
        ));
    }
    // The two rules iw4L's own `WeaponCombatFacts::try_from_captured` adds.
    if (2..=4).contains(&c.fire_type) && c.burst_cooldown_ms <= 0 {
        return Err(bad(
            "burst_cooldown_ms",
            "must be > 0 for burst fire".to_owned(),
        ));
    }
    if c.segmented_reload && c.reload_ammo_add <= 0 {
        return Err(bad(
            "reload_ammo_add",
            "must be > 0 for a segmented reload".to_owned(),
        ));
    }
    Ok(())
}

/// Validates a decoded `viewmodel.bin` against the weapon's `texture_count`.
pub fn viewmodel(weapon: &str, vm: &ViewmodelFile, texture_count: u32) -> Result<(), PackError> {
    skeleton(weapon, &vm.skeleton)?;
    mesh(weapon, &vm.mesh, &vm.skeleton, texture_count)?;
    clips(weapon, &vm.clips, &vm.skeleton)
}

/// Skeleton rules: at most 192 bones, parents precede children, `view_bone` in range,
/// quaternions finite and unit length.
pub fn skeleton(weapon: &str, s: &Skeleton) -> Result<(), PackError> {
    let bad = |what: String| invalid(format!("weapon '{weapon}' viewmodel.bin: skeleton.{what}"));
    let n = s.bones.len();
    if n == 0 || n > MAX_BONES {
        return Err(bad(format!(
            "bones has {n} entries, expected 1..={MAX_BONES}"
        )));
    }
    for (i, b) in s.bones.iter().enumerate() {
        if let Some(p) = b.parent
            && usize::from(p) >= i
        {
            return Err(bad(format!(
                "bones[{i}] '{}' has parent {p} which is not before it",
                b.name
            )));
        }
        if usize::from(b.model) >= s.models.len() {
            return Err(bad(format!(
                "bones[{i}] '{}' model {} is out of range",
                b.name, b.model
            )));
        }
        let q = b.bind_rotation;
        let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
        if !q.iter().all(|c| c.is_finite()) || (len - 1.0).abs() > QUAT_TOLERANCE {
            return Err(bad(format!(
                "bones[{i}] '{}' bind_rotation {q:?} is not a unit quaternion",
                b.name
            )));
        }
        if !b.bind_translation.iter().all(|c| c.is_finite())
            || !b.bind_world.iter().all(|c| c.is_finite())
        {
            return Err(bad(format!(
                "bones[{i}] '{}' has a non-finite bind transform",
                b.name
            )));
        }
    }
    let in_range = |idx: u16| usize::from(idx) < n;
    if !in_range(s.view_bone) {
        return Err(bad(format!(
            "view_bone {} is out of range ({n} bones)",
            s.view_bone
        )));
    }
    for (name, bone) in [
        ("camera_bone", s.camera_bone),
        ("flash_bone", s.flash_bone),
        ("brass_bone", s.brass_bone),
    ] {
        if let Some(b) = bone
            && !in_range(b)
        {
            return Err(bad(format!("{name} {b} is out of range ({n} bones)")));
        }
    }
    if let Some(b) = s.collapsed_bones.iter().find(|&&b| !in_range(b)) {
        return Err(bad(format!("collapsed_bones entry {b} is out of range")));
    }
    if let Some(p) = s
        .duplicates
        .iter()
        .find(|&&(a, b)| !in_range(a) || !in_range(b))
    {
        return Err(bad(format!("duplicates entry {p:?} is out of range")));
    }
    for (i, m) in s.models.iter().enumerate() {
        if usize::from(m.base) + usize::from(m.bone_count) > n || !m.scale.is_finite() {
            return Err(bad(format!(
                "models[{i}] '{}' covers bones outside the skeleton",
                m.name
            )));
        }
    }
    Ok(())
}

/// Mesh rules: equal attribute lengths, indices in range, surface ranges inside `indices` and
/// non-overlapping, skin bones in range, weights summing to 1.
pub fn mesh(
    weapon: &str,
    m: &ViewmodelMesh,
    s: &Skeleton,
    texture_count: u32,
) -> Result<(), PackError> {
    let bad = |what: String| invalid(format!("weapon '{weapon}' viewmodel.bin: mesh.{what}"));
    let n = m.positions.len();
    if m.normals.len() != n || m.uvs.len() != n || m.skin.len() != n {
        return Err(bad(format!(
            "attribute lengths differ: positions {n}, normals {}, uvs {}, skin {}",
            m.normals.len(),
            m.uvs.len(),
            m.skin.len()
        )));
    }
    if !m.indices.len().is_multiple_of(3) {
        return Err(bad(format!(
            "indices has {} entries, not a multiple of 3",
            m.indices.len()
        )));
    }
    if let Some(i) = m.indices.iter().find(|&&i| i as usize >= n) {
        return Err(bad(format!("index {i} is out of range ({n} vertices)")));
    }
    let finite = m.positions.iter().flatten().all(|c| c.is_finite())
        && m.normals.iter().flatten().all(|c| c.is_finite())
        && m.uvs.iter().flatten().all(|c| c.is_finite());
    if !finite {
        return Err(bad(
            "positions, normals or uvs contain a non-finite value".to_owned()
        ));
    }
    for (v, skin) in m.skin.iter().enumerate() {
        if let Some(b) = skin
            .bones
            .iter()
            .find(|&&b| usize::from(b) >= s.bones.len())
        {
            return Err(bad(format!(
                "skin[{v}] bone {b} is out of range ({} bones)",
                s.bones.len()
            )));
        }
        let sum: f32 = skin.weights.iter().sum();
        let ok = skin.weights.iter().all(|w| w.is_finite() && *w >= 0.0)
            && (sum - 1.0).abs() <= WEIGHT_TOLERANCE;
        if !ok {
            return Err(bad(format!(
                "skin[{v}] weights {:?} are negative, non-finite or do not sum to 1",
                skin.weights
            )));
        }
    }
    let mut ranges: Vec<(u64, u64, usize)> = Vec::with_capacity(m.surfaces.len());
    for (i, surf) in m.surfaces.iter().enumerate() {
        let start = u64::from(surf.index_start);
        let end = start + u64::from(surf.index_count);
        if surf.index_start % 3 != 0 || surf.index_count % 3 != 0 {
            return Err(bad(format!(
                "surfaces[{i}] '{}' range is not triangle aligned",
                surf.name
            )));
        }
        if end > m.indices.len() as u64 {
            return Err(bad(format!(
                "surfaces[{i}] '{}' range {start}..{end} exceeds indices ({})",
                surf.name,
                m.indices.len()
            )));
        }
        if let Some(t) = surf.texture
            && u32::from(t) >= texture_count
        {
            return Err(bad(format!(
                "surfaces[{i}] '{}' texture {t} but the weapon has {texture_count} textures",
                surf.name
            )));
        }
        if usize::from(surf.model) >= s.models.len() {
            return Err(bad(format!(
                "surfaces[{i}] '{}' model {} is out of range",
                surf.name, surf.model
            )));
        }
        if let crate::model::AlphaMode::Test { cutoff } = surf.alpha
            && !(cutoff.is_finite() && (0.0..=1.0).contains(&cutoff))
        {
            return Err(bad(format!(
                "surfaces[{i}] '{}' alpha cutoff {cutoff} is not in 0..=1",
                surf.name
            )));
        }
        ranges.push((start, end, i));
    }
    ranges.sort_unstable();
    for pair in ranges.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(bad(format!(
                "surfaces[{}] and surfaces[{}] overlap",
                pair[0].2, pair[1].2
            )));
        }
    }
    Ok(())
}

/// Clip rules: 39 slots, valid slot and track-map references, sane frame rate, key counts that
/// match their frame indices.
pub fn clips(weapon: &str, c: &ClipSet, s: &Skeleton) -> Result<(), PackError> {
    let bad = |what: String| invalid(format!("weapon '{weapon}' viewmodel.bin: clips.{what}"));
    if c.slots.len() != WEAPON_ANIM_SLOTS {
        return Err(bad(format!(
            "slots has {} entries, expected {WEAPON_ANIM_SLOTS}",
            c.slots.len()
        )));
    }
    for (slot, clip) in c.slots.iter().enumerate() {
        if let Some(i) = clip
            && usize::from(*i) >= c.clips.len()
        {
            return Err(bad(format!(
                "slots[{slot}] points at clip {i} but there are {}",
                c.clips.len()
            )));
        }
    }
    if c.track_maps.len() != c.clips.len() {
        return Err(bad(format!(
            "track_maps has {} entries for {} clips",
            c.track_maps.len(),
            c.clips.len()
        )));
    }
    for (i, (clip, map)) in c.clips.iter().zip(&c.track_maps).enumerate() {
        let name = &clip.name;
        if map.len() != clip.tracks.len() {
            return Err(bad(format!(
                "track_maps[{i}] has {} entries for clip '{name}' with {} tracks",
                map.len(),
                clip.tracks.len()
            )));
        }
        if let Some(b) = map
            .iter()
            .flatten()
            .find(|&&b| usize::from(b) >= s.bones.len())
        {
            return Err(bad(format!(
                "track_maps[{i}] (clip '{name}') maps to missing bone {b}"
            )));
        }
        clip_data(weapon, i, clip)?;
    }
    Ok(())
}

fn clip_data(weapon: &str, index: usize, clip: &ClipData) -> Result<(), PackError> {
    let bad = |what: String| {
        invalid(format!(
            "weapon '{weapon}' viewmodel.bin: clips[{index}] '{}' {what}",
            clip.name
        ))
    };
    if !clip.framerate.is_finite() || clip.framerate < 0.0 {
        return Err(bad(format!(
            "framerate {} is negative or non-finite",
            clip.framerate
        )));
    }
    if clip.framerate <= 0.0 && clip.numframes > 1 {
        return Err(bad(format!(
            "framerate is 0 with {} frames",
            clip.numframes
        )));
    }
    if let Some(n) = clip.notifies.iter().find(|n| !n.time.is_finite()) {
        return Err(bad(format!("notify '{}' has a non-finite time", n.name)));
    }
    for t in &clip.tracks {
        match &t.rotation {
            RotationData::Default => {}
            RotationData::HalfKeyed(k) | RotationData::FullKeyed(k) => {
                keyed_quat(k, clip.numframes)
                    .map_err(|e| bad(format!("track '{}' rotation: {e}", t.name)))?;
            }
            RotationData::HalfConstant(q) | RotationData::FullConstant(q) => {
                if !q.iter().all(|c| c.is_finite()) {
                    return Err(bad(format!(
                        "track '{}' has a non-finite constant rotation",
                        t.name
                    )));
                }
            }
        }
        translation(&t.translation, clip.numframes)
            .map_err(|e| bad(format!("track '{}' translation: {e}", t.name)))?;
    }
    translation(&clip.delta_translation, clip.numframes)
        .map_err(|e| bad(format!("delta_translation: {e}")))
}

fn translation(t: &TranslationData, numframes: u16) -> Result<(), String> {
    match t {
        TranslationData::Default => Ok(()),
        TranslationData::SmallKeyed(k) | TranslationData::FullKeyed(k) => keyed_vec3(k, numframes),
        TranslationData::Constant(v) => {
            if v.iter().all(|c| c.is_finite()) {
                Ok(())
            } else {
                Err("constant is not finite".to_owned())
            }
        }
    }
}

fn keyed_quat(k: &KeyedQuat, numframes: u16) -> Result<(), String> {
    if !k.values.iter().flatten().all(|c| c.is_finite()) {
        return Err("non-finite key".to_owned());
    }
    key_frames(k.values.len(), &k.frames, numframes)
}

fn keyed_vec3(k: &KeyedVec3, numframes: u16) -> Result<(), String> {
    if !k.values.iter().flatten().all(|c| c.is_finite()) {
        return Err("non-finite key".to_owned());
    }
    key_frames(k.values.len(), &k.frames, numframes)
}

/// Dense keys: key `i` sits at frame `i`, so there are between 1 and `numframes + 1` of them
/// (sampling clamps to the last key, as `anim_iw4::sample::span` does). Sparse keys: one frame
/// number per key, non-decreasing, not past `numframes`.
fn key_frames(values: usize, frames: &FrameIndicesData, numframes: u16) -> Result<(), String> {
    if values == 0 {
        return Err("no keys".to_owned());
    }
    match frames {
        FrameIndicesData::Dense => {
            if values > usize::from(numframes) + 1 {
                return Err(format!("{values} dense keys for {numframes} frames"));
            }
        }
        FrameIndicesData::Sparse(f) => {
            if f.len() != values {
                return Err(format!("{values} keys but {} frame indices", f.len()));
            }
            if f.windows(2).any(|w| w[1] < w[0]) {
                return Err("frame indices are not non-decreasing".to_owned());
            }
            if f.last().is_some_and(|&last| last > numframes) {
                return Err(format!("last frame index is past numframes ({numframes})"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_paths() {
        for ok in ["a", "weapons/m4_mp", "sounds/x/0.wav", "a.b/c"] {
            assert!(is_safe_rel_path(ok), "{ok}");
        }
        for bad in [
            "", "/abs", "a//b", "../x", "a/../b", "a/.", "a\\b", "c:/x", "a/", "a\0b",
        ] {
            assert!(!is_safe_rel_path(bad), "{bad:?}");
        }
    }
}
