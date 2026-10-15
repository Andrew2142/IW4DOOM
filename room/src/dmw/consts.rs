// SPDX-License-Identifier: GPL-3.0-or-later
//! Re-export of the contract constants so engine code uses `crate::dmw::consts::*`.
//!
//! The values live in `dmw_format::consts` (sheet 00 section 5). Do not redefine them here.

pub use dmw_format::consts::*;
