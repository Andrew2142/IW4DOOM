// SPDX-License-Identifier: GPL-3.0-or-later
//! Atomic pack writing (sheet 04 §5.9).
//!
//! The pack is built in `<out>.tmp-<pid>/`, verified, then swapped in: the previous `<out>` is
//! renamed to `<out>.old-<pid>`, the temporary directory takes its place and the old one is
//! deleted. Any failure before the swap removes the temporary directory and leaves the previous
//! pack untouched.

use std::fs;
use std::path::{Path, PathBuf};

/// Failure while writing or verifying a pack.
#[derive(Debug)]
pub enum WriteError {
    /// Filesystem error at a path.
    Io { path: PathBuf, error: std::io::Error },
    /// The fill step failed (export error or injected failure).
    Fill(String),
    /// The written pack did not pass `Pack::open` and the lazy loads.
    Verify(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Fill(m) => write!(f, "writing the pack failed: {m}"),
            Self::Verify(m) => write!(f, "the written pack failed verification: {m}"),
        }
    }
}

impl std::error::Error for WriteError {}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> WriteError + '_ {
    move |error| WriteError::Io {
        path: path.to_owned(),
        error,
    }
}

fn sibling(out: &Path, suffix: &str) -> PathBuf {
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pack".to_owned());
    out.with_file_name(format!("{name}.{suffix}-{}", std::process::id()))
}

/// Writes one file below `root`, creating parent directories.
pub fn write_file(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), WriteError> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    fs::write(&path, bytes).map_err(io(&path))
}

/// Builds the pack with `fill` in a temporary sibling of `out`, checks it with `verify`, then
/// atomically replaces `out`.
pub fn write_atomic(
    out: &Path,
    fill: impl FnOnce(&Path) -> Result<(), String>,
    verify: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), WriteError> {
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let tmp = sibling(out, "tmp");
    if tmp.exists() {
        fs::remove_dir_all(&tmp).map_err(io(&tmp))?;
    }
    fs::create_dir_all(&tmp).map_err(io(&tmp))?;
    let staged = fill(&tmp)
        .map_err(WriteError::Fill)
        .and_then(|()| verify(&tmp).map_err(WriteError::Verify));
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&tmp);
        return Err(error);
    }
    let old = sibling(out, "old");
    let had_old = out.exists();
    if had_old {
        if old.exists() {
            fs::remove_dir_all(&old).map_err(io(&old))?;
        }
        if let Err(error) = fs::rename(out, &old) {
            let _ = fs::remove_dir_all(&tmp);
            return Err(io(out)(error));
        }
    }
    if let Err(error) = fs::rename(&tmp, out) {
        if had_old {
            let _ = fs::rename(&old, out);
        }
        let _ = fs::remove_dir_all(&tmp);
        return Err(io(out)(error));
    }
    if had_old {
        fs::remove_dir_all(&old).map_err(io(&old))?;
    }
    Ok(())
}
