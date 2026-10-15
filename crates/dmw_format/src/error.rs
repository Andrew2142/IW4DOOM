// SPDX-License-Identifier: GPL-3.0-or-later
//! The pack error type (00 section 7.1, 01 section 5.5).

use std::path::PathBuf;

/// Every failure to open, validate or load a pack.
///
/// Each variant carries enough context (path, weapon, field) for a one-line log message.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// An I/O failure other than "file not found".
    #[error("{}: {source}", .path.display())]
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A file exists but cannot be decoded (JSON, postcard, PNG, WAV, bad magic).
    #[error("format error: {0}")]
    Format(String),
    /// `manifest.json` declares a format version this reader does not support.
    #[error("unsupported pack format version {found} (this build reads {expected})")]
    Version {
        /// Version found in the manifest.
        found: u32,
        /// Version this crate reads (`PACK_FORMAT_VERSION`).
        expected: u32,
    },
    /// A file or entry the manifest promises does not exist.
    #[error("missing: {0}")]
    Missing(String),
    /// The data decoded but breaks a structural rule (sheet 03 section 5.8).
    #[error("invalid: {0}")]
    Invalid(String),
}

impl PackError {
    /// Wraps an I/O error, mapping "not found" to [`PackError::Missing`] with `rel` (the path
    /// relative to the pack root) as the name.
    pub(crate) fn from_io(path: &std::path::Path, rel: &str, source: std::io::Error) -> Self {
        if source.kind() == std::io::ErrorKind::NotFound {
            PackError::Missing(rel.to_owned())
        } else {
            PackError::Io {
                path: path.to_owned(),
                source,
            }
        }
    }

    /// Shorthand for [`PackError::Invalid`].
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        PackError::Invalid(msg.into())
    }

    /// Shorthand for [`PackError::Format`].
    pub(crate) fn format(msg: impl Into<String>) -> Self {
        PackError::Format(msg.into())
    }
}
