// SPDX-License-Identifier: GPL-3.0-or-later
//! Sound aliases → 16-bit PCM WAV (sheet 04 §5.8): loaded MSS PCM from the zone, streamed
//! RIFF WAVE PCM from the IWD archives (read like iw4L `audio/src/clip_store.rs`
//! `prepare_streamed`).

use asset_audio::{MSS_PCM, SoundCatalog};
use asset_core::AssetNamespace;
use asset_transport::NamespaceSoundIwd;
use dmw_format::manifest::sanitize_alias;
use dmw_format::{SoundEntry, SoundVariant};

/// Raw little-endian PCM → interleaved signed 16-bit samples. 8-bit data is unsigned
/// (`(b - 128) << 8`, as iw4L `audio/src/pcm.rs`), 16-bit is signed. A trailing partial frame is
/// dropped.
pub fn pcm_to_i16(bytes: &[u8], bits: u16, channels: u16) -> Result<Vec<i16>, String> {
    if !matches!(channels, 1 | 2) {
        return Err(format!("{channels} channels (1 or 2 supported)"));
    }
    let frame = usize::from(bits / 8) * usize::from(channels);
    let usable = match bits {
        8 | 16 => bytes.len() - bytes.len() % frame,
        _ => return Err(format!("{bits}-bit PCM (8 or 16 supported)")),
    };
    let bytes = &bytes[..usable];
    Ok(match bits {
        8 => bytes.iter().map(|&b| (i16::from(b) - 128) << 8).collect(),
        _ => bytes
            .chunks_exact(2)
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect(),
    })
}

/// A decoded RIFF WAVE PCM file.
#[derive(Debug, Clone, PartialEq)]
pub struct RiffPcm {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub data: Vec<u8>,
}

/// Parses a RIFF WAVE file with an integer PCM (`format tag 1`) `fmt ` chunk. `Ok(None)` when the
/// bytes are not RIFF WAVE at all; `Err` for a WAVE with another codec or a broken layout.
pub fn parse_riff(bytes: &[u8]) -> Result<Option<RiffPcm>, String> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Ok(None);
    }
    let mut fmt: Option<&[u8]> = None;
    let mut data: Option<&[u8]> = None;
    let mut at = 12usize;
    while let Some(header) = bytes.get(at..at + 8) {
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let body = at + 8;
        let end = body.checked_add(size).ok_or("chunk size overflow")?;
        // Some encoders write a data size past the end of the file; take what exists.
        let chunk = bytes.get(body..end.min(bytes.len())).ok_or("truncated chunk")?;
        match &header[..4] {
            b"fmt " => fmt = Some(chunk),
            b"data" => data = Some(chunk),
            _ => {}
        }
        at = end + (size & 1);
    }
    let fmt = fmt.filter(|f| f.len() >= 16).ok_or("no fmt chunk")?;
    let field = |o: usize| u16::from_le_bytes([fmt[o], fmt[o + 1]]);
    let tag = field(0);
    if tag != 1 {
        return Err(format!("WAVE format tag {tag:#x} (only PCM is exported)"));
    }
    Ok(Some(RiffPcm {
        channels: field(2),
        rate: u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]),
        bits: field(14),
        data: data.ok_or("no data chunk")?.to_vec(),
    }))
}

/// One skipped variant.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Skipped {
    pub alias: String,
    pub variant: usize,
    pub reason: String,
}

/// One exported alias: its manifest entry and the WAV files to write (pack-relative path, bytes).
#[derive(Debug)]
pub struct ExportedSound {
    pub entry: SoundEntry,
    pub files: Vec<(String, Vec<u8>)>,
    /// `true` when volume/pitch/distance came from the alias row (always, for IW4 aliases).
    pub alias_fields: bool,
}

/// Exports every variant of `alias`. `folder` is the on-disk folder name (sanitised, unique).
pub fn export_alias(
    catalog: &SoundCatalog,
    iwd: &NamespaceSoundIwd,
    ns: AssetNamespace,
    alias: &str,
    folder: &str,
    skipped: &mut Vec<Skipped>,
) -> Option<ExportedSound> {
    let Some(sound) = catalog.sound_in(ns, alias) else {
        skipped.push(Skipped {
            alias: alias.to_owned(),
            variant: 0,
            reason: "alias not in the sound bank".to_owned(),
        });
        return None;
    };
    let mut variants = Vec::new();
    let mut files = Vec::new();
    for (v, row) in sound.aliases.iter().enumerate() {
        let skip = |reason: String, skipped: &mut Vec<Skipped>| {
            skipped.push(Skipped {
                alias: alias.to_owned(),
                variant: v,
                reason,
            });
        };
        let pcm: Result<(u32, u16, Vec<i16>), String> =
            if let Some(loaded) = catalog.pcm_for_variant(ns, alias, v) {
                if loaded.format() != MSS_PCM {
                    Err(format!("UnsupportedLoadedFormat({})", loaded.format()))
                } else {
                    let bits = u16::try_from(loaded.bits()).unwrap_or(0);
                    let channels = u16::try_from(loaded.channels()).unwrap_or(0);
                    pcm_to_i16(loaded.encoded_bytes(), bits, channels)
                        .map(|s| (loaded.rate, channels, s))
                }
            } else if let Some((sns, dir, name)) = catalog.streamed_for_variant(ns, alias, v) {
                let rel = format!("{dir}/{name}");
                match iwd.read_sound(sns, &rel) {
                    None => Err(format!("streamed '{rel}' not found in the IWD archives")),
                    Some(Err(e)) => Err(format!("streamed '{rel}': {e}")),
                    Some(Ok(bytes)) => match parse_riff(&bytes) {
                        Ok(Some(w)) => pcm_to_i16(&w.data, w.bits, w.channels)
                            .map(|s| (w.rate, w.channels, s)),
                        Ok(None) => Err(format!("streamed '{rel}' is not RIFF WAVE")),
                        Err(e) => Err(format!("streamed '{rel}': {e}")),
                    },
                }
            } else if row.is_null_file() {
                Err("null file".to_owned())
            } else {
                Err("variant has neither loaded nor streamed media".to_owned())
            };
        let (rate, channels, samples) = match pcm {
            Ok(p) if !p.2.is_empty() && p.0 > 0 => p,
            Ok(_) => {
                skip("empty PCM".to_owned(), skipped);
                continue;
            }
            Err(reason) => {
                skip(reason, skipped);
                continue;
            }
        };
        let bytes = match dmw_format::sound::encode_wav_i16(rate, channels, &samples) {
            Ok(b) => b,
            Err(e) => {
                skip(format!("WAV encode: {e}"), skipped);
                continue;
            }
        };
        let file = format!("sounds/{folder}/{v}.wav");
        let mean = |a: f32, b: f32| if a.is_finite() && b.is_finite() { (a + b) * 0.5 } else { 1.0 };
        variants.push(SoundVariant {
            file: file.clone(),
            volume: mean(row.vol_min, row.vol_max),
            pitch: mean(row.pitch_min, row.pitch_max),
            looping: row.is_looping().unwrap_or(false),
            distance_min: row.dist_min.max(0.0),
            distance_max: row.dist_max.max(0.0),
        });
        files.push((file, bytes));
    }
    if variants.is_empty() {
        skipped.push(Skipped {
            alias: alias.to_owned(),
            variant: 0,
            reason: "no exportable variant; alias omitted".to_owned(),
        });
        return None;
    }
    Some(ExportedSound {
        entry: SoundEntry {
            alias: alias.to_owned(),
            variants,
        },
        files,
        alias_fields: true,
    })
}

/// Unique on-disk folder for `alias` given folders already used.
pub fn folder_for(alias: &str, used: &mut std::collections::BTreeSet<String>) -> String {
    let base = sanitize_alias(alias);
    let mut name = base.clone();
    let mut n = 1;
    while !used.insert(name.clone()) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}
