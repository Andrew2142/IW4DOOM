// SPDX-License-Identifier: GPL-3.0-or-later
//! Extended per-tic command and per-player state (`ExtTiccmd`, `EXT_TICCMDS`, `PlayerExt`).
//!
//! Contract: `docs/specs/00-overview-and-contracts.md` section 7.4. Doom's `#[repr(C)]`
//! structs (`TiccmdT`, `PlayerT`, `mobj_t`) are never changed; the extra state travels in the
//! parallel arrays defined here. [`EXT_TICCMDS`] is a ring indexed by `maketic % BACKUPTICS`
//! that mirrors Doom's own ticcmd ring, and [`PLAYER_EXT`] is indexed like `players[]`.

use log::trace;

use crate::dmw::consts::{PITCH_LIMIT_DEG, PITCH_UNITS_PER_DEGREE};
use crate::doom::c_ffi::BACKUPTICS;
use crate::doom::d_player::MAXPLAYERS;

const TARGET: &str = "dmw::input";

/// Hold to aim down sights.
pub const EXT_BT_ADS: u16 = 1 << 0;
/// Reload.
pub const EXT_BT_RELOAD: u16 = 1 << 1;
/// Sprint (hold).
pub const EXT_BT_SPRINT: u16 = 1 << 2;
/// Melee.
pub const EXT_BT_MELEE: u16 = 1 << 3;
/// Sniper breath hold (shares the sprint key while ADS, sheet 05 section 5.4).
pub const EXT_BT_HOLD_BREATH: u16 = 1 << 4;
/// Switch to the last held weapon.
pub const EXT_BT_SWAP_LAST: u16 = 1 << 5;
/// Mirror of `BT_ATTACK`, so DOOMMW2 code never decodes Doom's button bits.
pub const EXT_BT_ATTACK: u16 = 1 << 6;
/// The slot was written by `on_build_ticcmd`.
pub const EXT_BT_VALID: u16 = 1 << 15;

/// Pitch limit in `pitch_q` units (1/128 degree).
pub const PITCH_LIMIT_Q: i32 = (PITCH_LIMIT_DEG * PITCH_UNITS_PER_DEGREE) as i32;

/// `pitch_q` units removed per tic while the player is dead (1 degree).
pub const DEAD_RECENTRE_Q: i32 = PITCH_UNITS_PER_DEGREE as i32;

/// Extension of Doom's `ticcmd_t`, carried beside it through the same ring slot index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtTiccmd {
    /// `maketic` it was built for; `-1` = empty slot.
    pub tic: i32,
    /// Pitch change, 1/128 degree, MW2 convention (positive = down).
    pub pitch_delta: i16,
    /// `EXT_BT_*` bits.
    pub buttons: u16,
}

impl ExtTiccmd {
    /// An empty slot: no pitch, no buttons, `tic == -1`.
    pub const NEUTRAL: ExtTiccmd = ExtTiccmd {
        tic: -1,
        pitch_delta: 0,
        buttons: 0,
    };
}

/// Ring of extended commands, indexed by `maketic % BACKUPTICS`.
pub static mut EXT_TICCMDS: [ExtTiccmd; BACKUPTICS] = [ExtTiccmd::NEUTRAL; BACKUPTICS];

/// Per-player DOOMMW2 state kept beside Doom's `players[]`.
#[derive(Clone, Debug)]
pub struct PlayerExt {
    /// Accumulated pitch, 1/128 degree, clamped to +-`PITCH_LIMIT_DEG`.
    pub pitch_q: i32,
    /// Copied in `G_Ticker` next to `players[i].cmd`.
    pub cmd: ExtTiccmd,
    /// `mo->angle` (BAM) before this tic's turn is applied, i.e. at the previous tic.
    pub prev_yaw: u32,
    /// `pitch_q` before this tic's pitch change.
    pub prev_pitch_q: i32,
    /// Written by sheet 08 from `TicOutput`, read by sheet 06.
    pub view_kick_deg: [f32; 3],
    /// Written by sheet 08, read by sheet 06.
    pub zoom_hfov_deg: Option<f32>,
}

impl PlayerExt {
    /// All-zero state with a neutral command.
    pub const NEW: PlayerExt = PlayerExt {
        pitch_q: 0,
        cmd: ExtTiccmd::NEUTRAL,
        prev_yaw: 0,
        prev_pitch_q: 0,
        view_kick_deg: [0.0; 3],
        zoom_hfov_deg: None,
    };
}

impl Default for PlayerExt {
    fn default() -> Self {
        Self::NEW
    }
}

/// Extension state for each player, indexed like Doom's `players[]`.
pub static mut PLAYER_EXT: [PlayerExt; MAXPLAYERS] = [PlayerExt::NEW; MAXPLAYERS];

/// Looks `gametic / ticdup` up in `ring`; a slot whose `tic` does not match is stale and a
/// neutral command is returned instead.
pub fn ext_lookup(ring: &[ExtTiccmd], gametic: i32, ticdup: i32) -> ExtTiccmd {
    if ring.is_empty() || ticdup <= 0 || gametic < 0 {
        return ExtTiccmd::NEUTRAL;
    }
    let tic = gametic / ticdup;
    let slot = ring[tic as usize % ring.len()];
    if slot.tic == tic {
        slot
    } else {
        ExtTiccmd::NEUTRAL
    }
}

/// The extended command for the tic Doom is about to run (`gametic`), or a neutral one when
/// the ring slot is stale.
pub fn ext_for_gametic(gametic: i32, ticdup: i32) -> ExtTiccmd {
    // SAFETY: the engine is single-threaded; the array is only read here and the shared
    // reference does not outlive this expression.
    let ring: &[ExtTiccmd] = unsafe { &*std::ptr::addr_of!(EXT_TICCMDS) };
    let ext = ext_lookup(ring, gametic, ticdup);
    if ext.tic == -1 {
        log_once!(
            TARGET,
            log::Level::Debug,
            "ext-stale",
            "no extended command for gametic {gametic}; using a neutral one"
        );
    }
    ext
}

/// Stores `ext` in the ring slot for `maketic`.
pub fn store_ext(maketic: i32, ext: ExtTiccmd) {
    if maketic < 0 {
        return;
    }
    // SAFETY: the engine is single-threaded and the index is reduced modulo the array length.
    unsafe {
        EXT_TICCMDS[maketic as usize % BACKUPTICS] = ext;
    }
}

/// Applies one tic's pitch change to `ext`.
///
/// Alive: `pitch_q += delta`, clamped to the pitch limit. Dead: input is ignored and the pitch
/// moves toward zero by [`DEAD_RECENTRE_Q`] per tic. `prev_pitch_q` always receives the value
/// from before this tic.
pub fn apply_pitch_to(ext: &mut PlayerExt, delta: i16, dead: bool) {
    ext.prev_pitch_q = ext.pitch_q;
    if dead {
        let step = DEAD_RECENTRE_Q.min(ext.pitch_q.abs());
        ext.pitch_q -= step * ext.pitch_q.signum();
    } else {
        ext.pitch_q = (ext.pitch_q + i32::from(delta)).clamp(-PITCH_LIMIT_Q, PITCH_LIMIT_Q);
    }
    trace!(target: TARGET, "pitch_q {} -> {}", ext.prev_pitch_q, ext.pitch_q);
}

/// Level start: pitch 0, ring cleared. Sheet 08's `on_level_start` calls this; the input
/// accumulators are cleared with it.
pub fn reset_level() {
    // SAFETY: the engine is single-threaded; no reference into these statics is live here.
    unsafe {
        for slot in 0..BACKUPTICS {
            EXT_TICCMDS[slot] = ExtTiccmd::NEUTRAL;
        }
        for i in 0..MAXPLAYERS {
            PLAYER_EXT[i].pitch_q = 0;
            PLAYER_EXT[i].prev_pitch_q = 0;
            PLAYER_EXT[i].cmd = ExtTiccmd::NEUTRAL;
        }
    }
    crate::dmw::input::clear_accumulators();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_with(tic: i32, ext: ExtTiccmd) -> Vec<ExtTiccmd> {
        let mut ring = vec![ExtTiccmd::NEUTRAL; BACKUPTICS];
        ring[tic as usize % BACKUPTICS] = ext;
        ring
    }

    /// T5.2: the pitch never leaves +-32 degrees.
    #[test]
    fn pitch_is_clamped() {
        let mut p = PlayerExt::NEW;
        for _ in 0..100 {
            apply_pitch_to(&mut p, i16::MAX, false);
        }
        assert_eq!(p.pitch_q, 32 * 128);
        for _ in 0..100 {
            apply_pitch_to(&mut p, i16::MIN, false);
        }
        assert_eq!(p.pitch_q, -32 * 128);
    }

    /// T5.2: a dead player ignores input and recentres at 1 degree per tic.
    #[test]
    fn dead_player_recentres_one_degree_per_tic() {
        let mut p = PlayerExt::NEW;
        p.pitch_q = 10 * 128 + 50;
        apply_pitch_to(&mut p, 1000, true);
        assert_eq!(p.pitch_q, 9 * 128 + 50);
        assert_eq!(p.prev_pitch_q, 10 * 128 + 50);
        for _ in 0..9 {
            apply_pitch_to(&mut p, 1000, true);
        }
        assert_eq!(p.pitch_q, 50);
        apply_pitch_to(&mut p, 1000, true);
        assert_eq!(p.pitch_q, 0);
        apply_pitch_to(&mut p, 1000, true);
        assert_eq!(p.pitch_q, 0);

        p.pitch_q = -3 * 128;
        apply_pitch_to(&mut p, -1000, true);
        assert_eq!(p.pitch_q, -2 * 128);
    }

    /// T5.3: the right slot is returned and a stale slot gives a neutral command.
    #[test]
    fn lookup_returns_matching_slot_and_neutral_when_stale() {
        let want = ExtTiccmd {
            tic: 300,
            pitch_delta: -12,
            buttons: EXT_BT_VALID | EXT_BT_ADS,
        };
        let ring = ring_with(300, want);
        assert_eq!(ext_lookup(&ring, 300, 1), want);
        // Same slot, different tic (one lap of the ring later): stale.
        assert_eq!(
            ext_lookup(&ring, 300 + BACKUPTICS as i32, 1),
            ExtTiccmd::NEUTRAL
        );
        // Other slots are empty.
        assert_eq!(ext_lookup(&ring, 301, 1), ExtTiccmd::NEUTRAL);
        assert_eq!(ExtTiccmd::NEUTRAL.tic, -1);
    }

    /// T5.3: `ticdup` maps several gametics onto one slot.
    #[test]
    fn lookup_divides_gametic_by_ticdup() {
        let want = ExtTiccmd {
            tic: 50,
            pitch_delta: 7,
            buttons: EXT_BT_VALID,
        };
        let ring = ring_with(50, want);
        assert_eq!(ext_lookup(&ring, 100, 2), want);
        assert_eq!(ext_lookup(&ring, 101, 2), want);
        assert_eq!(ext_lookup(&ring, 102, 2), ExtTiccmd::NEUTRAL);
        assert_eq!(ext_lookup(&ring, 5, 0), ExtTiccmd::NEUTRAL);
        assert_eq!(ext_lookup(&ring, -1, 1), ExtTiccmd::NEUTRAL);
    }

    #[test]
    fn button_bits_match_the_contract() {
        assert_eq!(EXT_BT_ADS, 1);
        assert_eq!(EXT_BT_RELOAD, 2);
        assert_eq!(EXT_BT_SPRINT, 4);
        assert_eq!(EXT_BT_MELEE, 8);
        assert_eq!(EXT_BT_HOLD_BREATH, 16);
        assert_eq!(EXT_BT_SWAP_LAST, 32);
        assert_eq!(EXT_BT_ATTACK, 64);
        assert_eq!(EXT_BT_VALID, 0x8000);
    }
}
