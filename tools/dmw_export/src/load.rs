// SPDX-License-Identifier: GPL-3.0-or-later
//! Headless MW2 load (sheet 04 §5.3), following iw4L `bootstrap/src/launch.rs`
//! `run_export_gltf` without the `bootstrap` dependency and keeping the whole `PreparedMatch`.

use std::path::PathBuf;
use std::time::Instant;

use asset_transport::{GamesRoot, LoadProgress};

/// Why the load did not produce a `PreparedMatch`.
#[derive(Debug)]
pub enum LoadError {
    /// No games root, zone not found, or the zone is not IW4 (exit 2).
    NotFound(String),
    /// iw4L refused or cancelled the load (exit 3).
    Refused(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) | Self::Refused(m) => f.write_str(m),
        }
    }
}

/// A loaded match and where it came from.
pub struct Loaded {
    pub games: GamesRoot,
    pub zone: String,
    pub common_zone: Option<String>,
    pub prepared: assets::PreparedMatch,
    /// English strings: `prepared.strings`, or (when the match load leaves it empty) the MP
    /// localized zones read with `asset_game::load_mp_localized_strings`.
    pub strings: asset_game::LocalizeCatalog,
    /// The match's full sound bank. `prepared.sound` holds only the map zone's aliases; the
    /// session composes it with the runtime zones (`common_mp`, `localized_common_mp`, ...) at
    /// match start. The exporter builds the same composition with
    /// `asset_audio::load_mp_sound_bank`.
    pub sounds: Result<asset_audio::SoundCatalog, String>,
    /// Sound zones that could not be read (`LoadedSoundBank::gap_lines`).
    pub sound_gaps: Vec<String>,
    pub load_ms: u128,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded")
            .field("games", &self.games.0)
            .field("zone", &self.zone)
            .field("common_zone", &self.common_zone)
            .field("load_ms", &self.load_ms)
            .finish_non_exhaustive()
    }
}

/// Games root from `DMW_MW2_GAMES`, else `IW4L_GAMES` (00 §4.2). The exporter does not fall back
/// to iw4L's portable default: with neither variable set it refuses, listing the roots iw4L
/// would search.
///
/// iw4L's `games_root_from_env` reads only `IW4L_GAMES`; instead of writing that variable into
/// the process environment (which is `unsafe` in edition 2024) the root is built directly. No
/// other iw4L code on the export path reads the variable.
pub fn games_root() -> Result<GamesRoot, LoadError> {
    let raw = std::env::var_os("DMW_MW2_GAMES").or_else(|| std::env::var_os("IW4L_GAMES"));
    let Some(raw) = raw else {
        let mut text = String::from(
            "MW2 Multiplayer was not found: DMW_MW2_GAMES is not set.\n\
             Set DMW_MW2_GAMES to a games root containing an `iw4` folder or symlink to the \
             Modern Warfare 2 installation (see iw4L docs/RUN.md).\n",
        );
        if let Ok(cwd) = std::env::current_dir() {
            text.push_str("Searched (relative to the current directory):\n");
            for root in asset_transport::search_roots(&cwd) {
                text.push_str(&format!("  {}\n", root.display()));
            }
        }
        return Err(LoadError::NotFound(text));
    };
    let path = std::path::absolute(PathBuf::from(raw)).map_err(|e| {
        LoadError::NotFound(format!("MW2 Multiplayer was not found: games root: {e}"))
    })?;
    if !path.is_dir() {
        return Err(LoadError::NotFound(format!(
            "MW2 Multiplayer was not found: games root {} is not a directory",
            path.display()
        )));
    }
    Ok(GamesRoot(path))
}

/// Loads `zone` (and `common_mp`) headlessly.
pub fn load(games: GamesRoot, zone: &str) -> Result<Loaded, LoadError> {
    let started = Instant::now();
    let found = asset_transport::find_zone_file(&games, zone).map_err(|e| {
        let mut text = format!("MW2 Multiplayer was not found: zone {zone}: {e}\nSearched:\n");
        for root in asset_transport::search_roots(&games.0) {
            text.push_str(&format!("  {}\n", root.display()));
        }
        LoadError::NotFound(text)
    })?;
    match asset_transport::zone_game_for_path(&found.path) {
        Some(asset_core::ZoneGame::Iw4) => {}
        other => {
            return Err(LoadError::NotFound(format!(
                "zone {} is {}, the exporter reads IW4 (MW2 2009) only",
                found.zone_name,
                other.map_or("not identifiable", |g| g.prefix())
            )));
        }
    }
    let common = asset_transport::find_runtime_common_mp(&games, &found.path);
    let common_zone = common.as_ref().ok().map(|z| z.zone_name.clone());
    let common_mp = common.map(|z| z.path);
    let families = frame::AssetFamilies::from_inventory(
        master_protocol::ContentFlags::IW4.0,
        &frame::UiMenuDvars::default(),
    )
    .map_err(LoadError::Refused)?;
    let outcome = bevy::tasks::futures_lite::future::block_on(assets::load_prepared_match(
        Ok(found.path.clone()),
        common_mp,
        LoadProgress::default(),
        families,
    ));
    let prepared = match outcome {
        assets::MatchLoadOutcome::Ready(prepared) => prepared,
        assets::MatchLoadOutcome::Canceled => {
            return Err(LoadError::Refused("map walk canceled".into()));
        }
        assets::MatchLoadOutcome::Refused(reason) => {
            return Err(LoadError::Refused(format!("map walk refused: {reason}")));
        }
    };
    let strings = if prepared.strings.is_empty() {
        asset_game::load_mp_localized_strings(&games, zone).unwrap_or_default()
    } else {
        prepared.strings.clone()
    };
    let (sounds, sound_gaps) = match asset_audio::load_mp_sound_bank(&games, zone) {
        Ok(bank) => {
            let gaps = bank.gap_lines();
            (Ok(bank.catalog), gaps)
        }
        Err(e) => (Err(e), Vec::new()),
    };
    Ok(Loaded {
        games,
        zone: found.zone_name,
        common_zone,
        prepared,
        strings,
        sounds,
        sound_gaps,
        load_ms: started.elapsed().as_millis(),
    })
}
