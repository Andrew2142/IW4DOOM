// SPDX-License-Identifier: GPL-3.0-or-later
//! `manifest.json` (sheet 03 section 5.2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

fn one() -> f32 {
    1.0
}

/// Top level of a pack: which weapons and sounds it holds and where.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackManifest {
    /// Must equal [`crate::PACK_FORMAT_VERSION`].
    pub format_version: u32,
    /// `"dmw_export 0.1.0"` or `"dmw-fixture 0.1.0"`.
    pub generator: String,
    /// iw4L revision the data was read with, or `"fixture"`.
    pub iw4l_rev: String,
    /// Where the data came from.
    pub source: PackSource,
    /// Creation time (Unix seconds); `0` for the fixture so output is byte-identical.
    pub created_unix: u64,
    /// `true` only for the synthetic fixture pack.
    pub fixture: bool,
    /// Order defines `DmwWeaponId` = index + 1.
    pub weapons: Vec<WeaponEntry>,
    /// Sound aliases with their variants.
    pub sounds: Vec<SoundEntry>,
    /// Requested name -> weapon name (the fixture maps `usp_mp` etc. onto its own weapons).
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

/// Provenance of the pack contents.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackSource {
    /// `"iw4"` or `"fixture"`.
    pub game: String,
    /// Fastfile zone the weapons were read from.
    pub zone: String,
    /// Common zone, when one was loaded alongside.
    #[serde(default)]
    pub common_zone: Option<String>,
}

/// One weapon in the pack.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WeaponEntry {
    /// MW2 weapon asset name, for example `m4_mp`.
    pub name: String,
    /// Directory relative to the pack root, for example `weapons/m4_mp`.
    pub dir: String,
    /// `false` when the weapon has no FPV assembly (no `viewmodel.bin`).
    pub has_viewmodel: bool,
    /// `true` when `overlay.png` exists.
    pub has_overlay: bool,
    /// Number of `textures/<n>.png` files.
    pub texture_count: u32,
}

/// One sound alias.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SoundEntry {
    /// The original MW2 alias name (the on-disk folder uses a sanitised form).
    pub alias: String,
    /// At least one variant.
    pub variants: Vec<SoundVariant>,
}

/// One playable variant of a sound alias.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SoundVariant {
    /// WAV path relative to the pack root.
    pub file: String,
    /// Linear 0..1 (mean of the alias volume min and max when known).
    #[serde(default = "one")]
    pub volume: f32,
    /// Playback rate multiplier (mean of the alias pitch min and max).
    #[serde(default = "one")]
    pub pitch: f32,
    /// Loops until stopped.
    #[serde(default)]
    pub looping: bool,
    /// MW2 inches; `0` = unknown.
    #[serde(default)]
    pub distance_min: f32,
    /// MW2 inches; `0` = unknown.
    #[serde(default)]
    pub distance_max: f32,
}

/// Replaces every character outside `[A-Za-z0-9_.-]` with `_`: the on-disk folder name of an alias.
///
/// A name that would be empty or consist only of dots becomes `_` so it cannot name `.` or `..`.
pub fn sanitize_alias(alias: &str) -> String {
    let s: String = alias
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.chars().all(|c| c == '.') {
        "_".to_owned()
    } else {
        s
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_unsafe_characters() {
        assert_eq!(sanitize_alias("wpn_m4_fire.plr"), "wpn_m4_fire.plr");
        assert_eq!(sanitize_alias("a/b\\c d:e"), "a_b_c_d_e");
        assert_eq!(sanitize_alias(".."), "_");
        assert_eq!(sanitize_alias(""), "_");
    }

    #[test]
    fn variant_defaults() {
        let v: SoundVariant = serde_json::from_str(r#"{"file":"sounds/x/0.wav"}"#).unwrap();
        assert_eq!(v.volume.to_bits(), 1.0f32.to_bits());
        assert_eq!(v.pitch.to_bits(), 1.0f32.to_bits());
        assert!(!v.looping);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let m = r#"{"format_version":1,"generator":"g","iw4l_rev":"r",
            "source":{"game":"iw4","zone":"z","future":1},"created_unix":1,"fixture":false,
            "weapons":[],"sounds":[],"future_key":[1,2]}"#;
        let m: PackManifest = serde_json::from_str(m).unwrap();
        assert!(m.aliases.is_empty());
        assert_eq!(m.source.common_zone, None);
    }
}
