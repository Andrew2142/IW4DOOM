// SPDX-License-Identifier: GPL-3.0-or-later
//! `dmw_export`: offline exporter from a Modern Warfare 2 install to a DOOMMW2 pack.
//!
//! Scaffold (sheet 02): parses `--out <dir>` and `--help`, prints the version line and exits.
//! Sheet 04 implements the export.
#![allow(clippy::print_stdout)] // command-line tool: the version line is its output

use std::path::PathBuf;

use clap::Parser;

/// The iw4L revision this exporter is built against (`IW4L_REV` at the repository root).
const IW4L_REV: &str = include_str!("../../../IW4L_REV");

/// Export MW2 weapons to a DOOMMW2 pack.
#[derive(Debug, Parser)]
#[command(name = "dmw_export", version, about)]
struct Args {
    /// Output pack directory (for example `packs/mw2`).
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
}

fn main() {
    let _args = Args::parse();
    println!(
        "dmw_export {} (iw4L {})",
        env!("CARGO_PKG_VERSION"),
        IW4L_REV.trim()
    );
}
