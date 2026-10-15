// SPDX-License-Identifier: GPL-3.0-or-later
//! `dmw-fixture <dir>`: writes the synthetic fixture pack (sheet 03 section 5.9) to `<dir>`
//! in the v1 on-disk layout. `task fixture-pack` runs it with `packs/fixture`.
#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        let _ = writeln!(std::io::stderr(), "usage: dmw-fixture <output-dir>");
        return ExitCode::from(2);
    };
    let dir = PathBuf::from(dir);
    match dmw_format::fixture::write_to_dir(&dir) {
        Ok(()) => {
            let _ = writeln!(std::io::stdout(), "dmw-fixture: wrote {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "dmw-fixture: {e}");
            ExitCode::FAILURE
        }
    }
}
