// SPDX-License-Identifier: GPL-3.0-or-later
//! `dmw_export`: offline exporter from a Modern Warfare 2 (2009) multiplayer install to a
//! DOOMMW2 pack (sheet 04).
//!
//! Exit codes: 0 success; 1 usage; 2 MW2 not found / wrong game; 3 load refused; 4 a requested
//! weapon failed (without `--keep-going`); 5 write failure.
#![allow(clippy::print_stdout)] // command-line tool: progress and the summary are its output (§7)

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use dmw_export::export::{self, WeaponOutput};
use dmw_export::report::Report;
use dmw_export::{load, roster};

/// Export MW2 weapons to a DOOMMW2 pack.
#[derive(Debug, Parser)]
#[command(name = "dmw_export", version, about)]
struct Args {
    /// Output pack directory (for example `packs/mw2`); replaced atomically.
    #[arg(long, value_name = "DIR", required_unless_present = "list")]
    out: Option<PathBuf>,
    /// Map zone whose load brings in common_mp (any IW4 MP map).
    #[arg(long, default_value = "mp_rust")]
    zone: String,
    /// Comma-separated `*_mp` weapon names (default: the 00 §9.1 roster plus extras).
    #[arg(long, value_delimiter = ',')]
    weapons: Option<Vec<String>>,
    /// Print every published weapon and exit (no pack is written).
    #[arg(long)]
    list: bool,
    /// Report and skip a weapon that fails instead of aborting.
    #[arg(long)]
    keep_going: bool,
    /// Use the alternate faction's view hands.
    #[arg(long)]
    axis_hands: bool,
    /// Where iw4L keeps its decoded-image cache (MW2-derived data). Default: `.dmw_export-cache`
    /// next to `--out`; with `--list`, a temporary directory removed on exit.
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
}

/// iw4L writes its artifact cache (decoded image mips) to `$IW4L_ARTIFACTS_DIR`, else to
/// `./iw4l-artifacts`. Setting the variable needs `unsafe` in edition 2024, so the exporter
/// moves its working directory into a cache directory it controls instead: MW2-derived bytes
/// never land in the caller's directory (for example the repository, under `task export`).
/// Returns the directory to delete on exit, if any.
fn enter_cache_dir(args: &Args, out: Option<&std::path::Path>) -> Result<Option<PathBuf>, String> {
    if std::env::var_os("IW4L_ARTIFACTS_DIR").is_some() {
        return Ok(None);
    }
    let (dir, temporary) = match (&args.cache_dir, out) {
        (Some(d), _) => (std::path::absolute(d).map_err(|e| e.to_string())?, false),
        (None, Some(out)) => (
            out.parent()
                .unwrap_or_else(|| std::path::Path::new("/"))
                .join(".dmw_export-cache"),
            false,
        ),
        (None, None) => (
            std::env::temp_dir().join(format!("dmw_export-cache-{}", std::process::id())),
            true,
        ),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("cache dir {}: {e}", dir.display()))?;
    std::env::set_current_dir(&dir).map_err(|e| format!("cache dir {}: {e}", dir.display()))?;
    println!("iw4L cache: {}", dir.join("iw4l-artifacts").display());
    Ok(temporary.then_some(dir))
}

const EXIT_NOT_FOUND: u8 = 2;
const EXIT_REFUSED: u8 = 3;
const EXIT_WEAPON: u8 = 4;
const EXIT_WRITE: u8 = 5;

fn list(loaded: &load::Loaded) {
    let registry = &loaded.prepared.weapons;
    let mut n = 0;
    for bound in registry.published_weapons() {
        let i = bound.wire_id();
        if registry.name_of(i).is_empty() {
            continue;
        }
        let name = registry.script_name_of(i);
        let display = dmw_export::facts::display_name(registry, i, &loaded.strings);
        let fpv = registry.fpv_assemblies_of(i, false).is_some();
        let key = registry.display_name_key_of(i).unwrap_or("-");
        println!(
            "{name}\t{display}\t{}\t{key}",
            if fpv { "fpv" } else { "no-fpv" }
        );
        n += 1;
    }
    println!(
        "{n} published weapons; {} localized strings",
        loaded.strings.len()
    );
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 1 } else { 0 });
        }
    };
    println!(
        "dmw_export {} (iw4L {})",
        env!("CARGO_PKG_VERSION"),
        dmw_export::iw4l_rev()
    );
    let started_at = Instant::now();
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    // Relative paths are resolved before the working directory moves (enter_cache_dir).
    let out = match args.out.as_deref().map(std::path::absolute).transpose() {
        Ok(o) => o,
        Err(e) => {
            println!("--out: {e}");
            return ExitCode::from(1);
        }
    };
    let games = match load::games_root() {
        Ok(g) => g,
        Err(e) => {
            println!("{e}");
            return ExitCode::from(EXIT_NOT_FOUND);
        }
    };
    let temp_cache = match enter_cache_dir(&args, out.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            println!("error: {e}");
            return ExitCode::from(EXIT_WRITE);
        }
    };
    let code = run(&args, out, games, started_at, started_unix);
    if let Some(dir) = temp_cache {
        let _ = std::env::set_current_dir(std::env::temp_dir());
        let _ = std::fs::remove_dir_all(dir);
    }
    code
}

fn run(
    args: &Args,
    out: Option<PathBuf>,
    games: asset_transport::GamesRoot,
    started_at: Instant,
    started_unix: u64,
) -> ExitCode {
    println!("games root: {}", games.0.display());
    let loaded = match load::load(games, &args.zone) {
        Ok(l) => l,
        Err(e @ load::LoadError::NotFound(_)) => {
            println!("{e}");
            return ExitCode::from(EXIT_NOT_FOUND);
        }
        Err(e) => {
            println!("load refused: {e}");
            return ExitCode::from(EXIT_REFUSED);
        }
    };
    let registry = &loaded.prepared.weapons;
    println!(
        "loaded zone {} (common {}): {} published weapons in {} ms",
        loaded.zone,
        loaded.common_zone.as_deref().unwrap_or("none"),
        roster::published_names(registry).len(),
        loaded.load_ms
    );
    for gap in &loaded.sound_gaps {
        println!("note: {gap}");
    }
    if args.list {
        list(&loaded);
        return ExitCode::SUCCESS;
    }
    let Some(out) = out else {
        println!("--out is required");
        return ExitCode::from(1);
    };

    let names = args.weapons.clone().unwrap_or_else(roster::default_roster);
    let mut outputs: Vec<WeaponOutput> = Vec::new();
    let mut failed = Vec::new();
    let mut errors = Vec::new();
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        let index = match roster::resolve(registry, name) {
            Ok(i) => i,
            Err(e) => {
                println!("error: {e}");
                errors.push(e);
                if !args.keep_going {
                    return ExitCode::from(EXIT_WEAPON);
                }
                continue;
            }
        };
        if outputs
            .iter()
            .any(|o| o.facts.name == registry.script_name_of(index))
        {
            println!("note: {name} listed twice; exported once");
            continue;
        }
        match export::export_weapon(&loaded, name, index, args.axis_hands) {
            Ok(o) => outputs.push(o),
            Err(failure) => {
                let (e, report) = *failure;
                let line = format!("{name}: {e}");
                println!("error: {line}");
                errors.push(line);
                failed.push(report);
                if !args.keep_going {
                    return ExitCode::from(EXIT_WEAPON);
                }
            }
        }
    }

    let sounds = export::export_sounds(&loaded, &outputs);
    export::attach_sound_reports(&mut outputs, &sounds);
    let manifest = export::manifest(&loaded, &outputs, &sounds, started_unix);
    let mut weapons_report: Vec<_> = outputs.iter().map(|o| o.report.clone()).collect();
    weapons_report.extend(failed);
    let report = Report {
        generator: dmw_export::generator(),
        iw4l_rev: dmw_export::iw4l_rev(),
        zone: loaded.zone.clone(),
        common_zone: loaded.common_zone.clone(),
        started: started_unix,
        load_ms: u64::try_from(loaded.load_ms).unwrap_or(u64::MAX),
        duration_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        weapons: weapons_report,
        extra_sounds: sounds.extras.clone(),
        sounds_skipped: sounds.skipped.clone(),
        errors,
    };
    if let Err(e) = export::write_pack(&out, &manifest, &outputs, &sounds, &report) {
        println!("error: {e}");
        println!("the previous pack at {} is untouched", out.display());
        return ExitCode::from(EXIT_WRITE);
    }
    report.print_summary();
    println!(
        "wrote {} ({} weapons, {} sound aliases); report: {}",
        out.display(),
        manifest.weapons.len(),
        manifest.sounds.len(),
        out.join("report.json").display()
    );
    ExitCode::SUCCESS
}
