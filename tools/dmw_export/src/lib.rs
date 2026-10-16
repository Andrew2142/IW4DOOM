// SPDX-License-Identifier: GPL-3.0-or-later
//! `dmw_export` library half: everything the binary needs, split so the pure conversion
//! helpers (skin weights, texture downscale, PCM to WAV, atomic write, name suggestions) can be
//! tested without an MW2 install (sheet 04 §9).
//!
//! Modules that read MW2 data (`load`, `facts`, `viewmodel`, `clips`, `textures`, `sounds`)
//! take iw4L's `PreparedMatch` pieces and return pack types; only `main.rs` and `report.rs`
//! print.

pub mod clips;
pub mod export;
pub mod facts;
pub mod load;
pub mod report;
pub mod roster;
pub mod sounds;
pub mod textures;
pub mod viewmodel;
pub mod write;

/// The iw4L revision this exporter is built against (`IW4L_REV` at the repository root).
pub const IW4L_REV: &str = include_str!("../../../IW4L_REV");

/// `generator` string written into the manifest and report.
pub fn generator() -> String {
    format!("dmw_export {}", env!("CARGO_PKG_VERSION"))
}

/// Short iw4L revision (`fc3d8ea`).
pub fn iw4l_rev() -> String {
    IW4L_REV.trim().chars().take(7).collect()
}
