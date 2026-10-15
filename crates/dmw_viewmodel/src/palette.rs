// SPDX-License-Identifier: GPL-3.0-or-later
//! PLAYPAL + COLORMAP tables and the nearest-colour LUT (sheet 11 section 5.1).

use std::fmt;
use std::time::Instant;

use crate::RasterError;

/// Number of maps in a Doom COLORMAP lump: 32 light levels, invulnerability, all-black.
pub const COLORMAP_COUNT: usize = 34;
const LUT_SIZE: usize = 64 * 64 * 64;
/// sqrt of the largest scaled redmean distance between a cell centre (`4c + 2`) and a colour
/// of its cell (offsets -2..=+1 per channel): sqrt((1534 + 2048 + 1534) * 2²).
const CELL_RADIUS: f64 = 143.06;
/// The same for a block of 4x4x4 cells (centre `16k + 8`, offsets -8..=+7): sqrt(5116 * 8²).
const BLOCK_RADIUS: f64 = 572.2;

/// Largest palette distance (from a region's centre) of an entry that can still be the nearest
/// for some colour of the region, given the best distance `best` at the centre.
///
/// In the metric sqrt(distance), moving within the region changes any distance by at most
/// `radius`, plus a relative 0.05 % because the redmean weights depend on the red mean (each
/// weight moves by at most 1/1024 inside a region); 1 % and +1 cover both with room to spare.
/// So an entry can only win somewhere in the region if its centre distance is within
/// `2 * radius` of the best one.
fn candidate_limit(best: u32, radius: f64) -> u32 {
    let limit = f64::from(best).sqrt() * 1.01 + 2.0 * radius + 1.0;
    (limit * limit).min(f64::from(u32::MAX)) as u32
}

/// Doom's palette and light tables plus a 6-bit RGB nearest-colour lookup table.
pub struct PaletteTables {
    /// PLAYPAL palette 0, raw lump values (no gamma).
    pub playpal: [[u8; 3]; 256],
    /// COLORMAP: 34 maps (0..31 light, 32 invulnerability, 33 black).
    pub colormaps: Vec<[u8; 256]>,
    /// Nearest palette index for the centre of each 6-bit RGB cell.
    lut6: Box<[u8; LUT_SIZE]>,
    /// Per cell: 0 when every colour of the cell provably maps to `lut6[cell]`; otherwise
    /// `1 + offset` into `candidates`, where a little-endian u16 length is followed by the
    /// palette indices that can be nearest somewhere in the cell (searched exactly by
    /// `nearest_fast`).
    ambiguous: Vec<u32>,
    candidates: Vec<u8>,
}

impl fmt::Debug for PaletteTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaletteTables")
            .field("colormaps", &self.colormaps.len())
            .finish_non_exhaustive()
    }
}

/// "Redmean" weighted squared RGB distance, scaled by 512 so it stays an exact integer:
/// `512 * ((2 + r̄/256)Δr² + 4Δg² + (2 + (255 − r̄)/256)Δb²)` with `r̄ = (r1 + r2) / 2`.
pub fn colour_distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let rsum = u32::from(a[0]) + u32::from(b[0]);
    let dr = i32::from(a[0]) - i32::from(b[0]);
    let dg = i32::from(a[1]) - i32::from(b[1]);
    let db = i32::from(a[2]) - i32::from(b[2]);
    let (dr2, dg2, db2) = ((dr * dr) as u32, (dg * dg) as u32, (db * db) as u32);
    (1024 + rsum) * dr2 + 2048 * dg2 + (1024 + 510 - rsum) * db2
}

fn nearest_in(playpal: &[[u8; 3]; 256], rgb: [u8; 3]) -> u8 {
    let mut best = 0usize;
    let mut best_d = u32::MAX;
    for (i, &c) in playpal.iter().enumerate() {
        let d = colour_distance(rgb, c);
        if d < best_d {
            best_d = d;
            best = i;
            if d == 0 {
                break;
            }
        }
    }
    best as u8
}

impl PaletteTables {
    /// Builds the tables from raw PLAYPAL (≥ 768 bytes, palette 0 is used) and COLORMAP
    /// (≥ 34·256 bytes) lumps, and the 6-bit LUT (logged at `info!` on `dmw::raster`).
    pub fn from_wad_lumps(playpal: &[u8], colormap: &[u8]) -> Result<Self, RasterError> {
        if playpal.len() < 768 {
            return Err(RasterError::Palette(format!(
                "PLAYPAL is {} bytes, need at least 768",
                playpal.len()
            )));
        }
        if colormap.len() < COLORMAP_COUNT * 256 {
            return Err(RasterError::Palette(format!(
                "COLORMAP is {} bytes, need at least {}",
                colormap.len(),
                COLORMAP_COUNT * 256
            )));
        }
        let mut pal = [[0u8; 3]; 256];
        for (dst, src) in pal.iter_mut().zip(playpal.chunks_exact(3)) {
            *dst = [src[0], src[1], src[2]];
        }
        let colormaps = colormap
            .chunks_exact(256)
            .take(COLORMAP_COUNT)
            .map(|c| {
                let mut m = [0u8; 256];
                m.copy_from_slice(c);
                m
            })
            .collect();

        let started = Instant::now();
        let mut lut = vec![0u8; LUT_SIZE];
        let mut ambiguous = vec![0u32; LUT_SIZE];
        let mut candidates = Vec::new();
        let mut block = Vec::with_capacity(256);
        let mut dist = [0u32; 256];
        // Blocks of 4x4x4 cells (16 colour values per channel). Only palette entries that can
        // be nearest somewhere in the block are searched for its cells; both lists stay in
        // ascending index order so ties still resolve to the lowest index.
        for block_id in 0..16 * 16 * 16usize {
            let (br, bg, bb) = (block_id >> 8, (block_id >> 4) & 15, block_id & 15);
            let centre = [
                (br * 16 + 8) as u8,
                (bg * 16 + 8) as u8,
                (bb * 16 + 8) as u8,
            ];
            let mut best_d = u32::MAX;
            for (i, &c) in pal.iter().enumerate() {
                dist[i] = colour_distance(centre, c);
                best_d = best_d.min(dist[i]);
            }
            let limit = candidate_limit(best_d, BLOCK_RADIUS);
            block.clear();
            block.extend((0..256).filter(|&i| dist[i] <= limit).map(|i| i as u8));

            for cr in br * 4..br * 4 + 4 {
                for cg in bg * 4..bg * 4 + 4 {
                    for cb in bb * 4..bb * 4 + 4 {
                        let cell = cr << 12 | cg << 6 | cb;
                        let centre = [
                            (cr << 2 | 2) as u8,
                            (cg << 2 | 2) as u8,
                            (cb << 2 | 2) as u8,
                        ];
                        let mut best = block[0];
                        let mut best_d = u32::MAX;
                        for &i in &block {
                            let d = colour_distance(centre, pal[usize::from(i)]);
                            dist[usize::from(i)] = d;
                            if d < best_d {
                                best_d = d;
                                best = i;
                            }
                        }
                        lut[cell] = best;
                        let limit = candidate_limit(best_d, CELL_RADIUS);
                        let start = candidates.len();
                        candidates.extend_from_slice(&[0, 0]);
                        candidates.extend(
                            block
                                .iter()
                                .copied()
                                .filter(|&i| dist[usize::from(i)] <= limit),
                        );
                        let n = candidates.len() - start - 2;
                        if n <= 1 {
                            candidates.truncate(start);
                        } else {
                            candidates[start..start + 2].copy_from_slice(&(n as u16).to_le_bytes());
                            ambiguous[cell] = start as u32 + 1;
                        }
                    }
                }
            }
        }
        let lut6: Box<[u8; LUT_SIZE]> = lut
            .into_boxed_slice()
            .try_into()
            .map_err(|_| RasterError::Palette("LUT size mismatch".into()))?;
        let ambiguous_cells = ambiguous.iter().filter(|&&a| a != 0).count();
        log::info!(
            target: "dmw::raster",
            "palette LUT built in {:.1} ms ({ambiguous_cells} of {LUT_SIZE} cells need an exact check)",
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(Self {
            playpal: pal,
            colormaps,
            lut6,
            ambiguous,
            candidates,
        })
    }

    /// Exact nearest palette index (redmean distance, ties to the lowest index).
    pub fn nearest(&self, rgb: [u8; 3]) -> u8 {
        nearest_in(&self.playpal, rgb)
    }

    /// LUT lookup: the nearest index of the 6-bit cell containing `rgb`. Cells whose colours
    /// do not all share one nearest entry are resolved exactly among their few candidates, so
    /// the result equals [`PaletteTables::nearest`] (sheet 11 section 11).
    #[inline]
    pub fn nearest_fast(&self, rgb: [u8; 3]) -> u8 {
        let cell = (usize::from(rgb[0] >> 2) << 12)
            | (usize::from(rgb[1] >> 2) << 6)
            | usize::from(rgb[2] >> 2);
        match self.ambiguous[cell] {
            0 => self.lut6[cell],
            at => {
                let at = at as usize - 1;
                let n = usize::from(u16::from_le_bytes([
                    self.candidates[at],
                    self.candidates[at + 1],
                ]));
                let list = &self.candidates[at + 2..at + 2 + n];
                let mut best = list[0];
                let mut best_d = u32::MAX;
                for &i in list {
                    let d = colour_distance(rgb, self.playpal[usize::from(i)]);
                    if d < best_d {
                        best_d = d;
                        best = i;
                    }
                }
                best
            }
        }
    }

    /// The plain 6-bit LUT entry for `rgb`'s cell (nearest colour to the cell centre), without
    /// the exact check of [`PaletteTables::nearest_fast`].
    #[inline]
    pub fn nearest_cell(&self, rgb: [u8; 3]) -> u8 {
        let cell = (usize::from(rgb[0] >> 2) << 12)
            | (usize::from(rgb[1] >> 2) << 6)
            | usize::from(rgb[2] >> 2);
        self.lut6[cell]
    }

    /// Colormap `index`, clamped to the last map present.
    #[inline]
    pub(crate) fn colormap(&self, index: usize) -> &[u8; 256] {
        let last = self.colormaps.len().saturating_sub(1);
        &self.colormaps[index.min(last)]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_lumps() {
        assert!(PaletteTables::from_wad_lumps(&[0; 767], &[0; 34 * 256]).is_err());
        assert!(PaletteTables::from_wad_lumps(&[0; 768], &[0; 33 * 256]).is_err());
    }

    #[test]
    fn distance_is_symmetric_and_zero_on_equal() {
        let a = [10, 200, 30];
        let b = [250, 5, 99];
        assert_eq!(colour_distance(a, b), colour_distance(b, a));
        assert_eq!(colour_distance(a, a), 0);
        // Green is weighted more than red or blue for the same step.
        assert!(colour_distance([0, 10, 0], [0, 0, 0]) > colour_distance([10, 0, 0], [0, 0, 0]));
    }

    #[test]
    fn ties_go_to_lowest_index() {
        let mut playpal = vec![0u8; 768];
        playpal[3..6].copy_from_slice(&[0, 0, 0]); // index 1 duplicates index 0
        let t = PaletteTables::from_wad_lumps(&playpal, &[0; 34 * 256]).unwrap();
        assert_eq!(t.nearest([0, 0, 0]), 0);
    }
}
