// SPDX-License-Identifier: GPL-3.0-or-later
//! `report.json` and the printed summary (sheet 04 §5.10).

use serde::Serialize;

use crate::sounds::Skipped;

/// Surface counters.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SurfaceReport {
    pub exported: u32,
    pub hidden: u32,
    pub untextured: u32,
    pub no_material: u32,
}

/// One exported surface (for diagnosing how the viewmodel renders).
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceDetail {
    pub material: String,
    pub model: String,
    pub role: String,
    pub alpha: String,
    pub cull: String,
    pub texture: Option<u16>,
    pub triangles: u32,
    /// Fraction of texels with alpha < 128 in the surface's texture (alpha-test holes).
    pub texture_low_alpha: Option<f32>,
}

/// Clip counters.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ClipReport {
    pub slots_bound: u32,
    pub unique: u32,
    pub dropped: Vec<String>,
}

/// One exported texture.
#[derive(Debug, Clone, Serialize)]
pub struct TextureReport {
    pub n: u32,
    pub w: u32,
    pub h: u32,
    pub orig_w: u32,
    pub orig_h: u32,
    pub source: String,
}

/// Sounds referenced by one weapon.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SoundReport {
    /// Referenced aliases present in the pack.
    pub exported: u32,
    /// Variants of this weapon's aliases that were skipped.
    pub skipped: Vec<Skipped>,
}

/// One weapon.
#[derive(Debug, Clone, Serialize)]
pub struct WeaponReport {
    pub name: String,
    pub display_name: String,
    pub ok: bool,
    pub assembly: String,
    pub surfaces: SurfaceReport,
    pub surface_detail: Vec<SurfaceDetail>,
    pub clips: ClipReport,
    pub textures: Vec<TextureReport>,
    pub sounds: SoundReport,
    pub has_overlay: bool,
    pub warnings: Vec<String>,
}

impl WeaponReport {
    /// An empty report for `name` (filled as the export proceeds).
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            display_name: String::new(),
            ok: false,
            assembly: "none".to_owned(),
            surfaces: SurfaceReport::default(),
            surface_detail: Vec::new(),
            clips: ClipReport::default(),
            textures: Vec::new(),
            sounds: SoundReport::default(),
            has_overlay: false,
            warnings: Vec::new(),
        }
    }
}

/// The whole report.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub generator: String,
    pub iw4l_rev: String,
    pub zone: String,
    pub common_zone: Option<String>,
    pub started: u64,
    pub load_ms: u64,
    pub duration_ms: u64,
    pub weapons: Vec<WeaponReport>,
    /// Sounds outside any weapon (`mp_hit_alert`).
    pub extra_sounds: Vec<String>,
    /// Every skipped sound variant across the pack.
    pub sounds_skipped: Vec<Skipped>,
    pub errors: Vec<String>,
}

impl Report {
    /// Pretty JSON.
    pub fn to_json(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    /// One line per weapon: `ok m4_mp: 7 surfaces, 24 clips, 5 textures, 11 sounds`.
    #[allow(clippy::print_stdout)] // the exporter is a CLI; the summary is its output (§7)
    pub fn print_summary(&self) {
        for w in &self.weapons {
            println!(
                "{} {}: {} surfaces, {} clips, {} textures, {} sounds{}{}",
                if w.ok { "ok" } else { "FAILED" },
                w.name,
                w.surfaces.exported,
                w.clips.slots_bound,
                w.textures.len(),
                w.sounds.exported,
                if w.warnings.is_empty() {
                    String::new()
                } else {
                    format!(", {} warnings", w.warnings.len())
                },
                if w.sounds.skipped.is_empty() {
                    String::new()
                } else {
                    format!(", {} sound variants skipped", w.sounds.skipped.len())
                },
            );
        }
        for e in &self.errors {
            println!("error: {e}");
        }
    }
}
