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
        println!("{name}\t{display}\t{}\t{key}", if fpv { "fpv" } else { "no-fpv" });
        n += 1;
    }
    println!("{n} published weapons; {} localized strings", loaded.strings.len());
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 1 } else { 0 });
        }
    };
    println!("dmw_export {} (iw4L {})", env!("CARGO_PKG_VERSION"), dmw_export::iw4l_rev());
    let started_at = Instant::now();
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let games = match load::games_root() {
        Ok(g) => g,
        Err(e) => {
            println!("{e}");
            return ExitCode::from(EXIT_NOT_FOUND);
        }
    };
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
    let Some(out) = args.out.clone() else {
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
        if outputs.iter().any(|o| o.facts.name == registry.script_name_of(index)) {
            println!("note: {name} listed twice; exported once");
            continue;
        }
        match export::export_weapon(&loaded, name, index, args.axis_hands) {
            Ok(o) => outputs.push(o),
            Err((e, report)) => {
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
