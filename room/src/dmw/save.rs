// SPDX-License-Identifier: GPL-3.0-or-later
//! Savegame extension block appended after `SAVEGAME_EOF` (sheet 08 section 5.11, D19).
//!
//! Layout: `b"DMW1"`, `u32` little-endian payload length, postcard-encoded [`SaveBlock`].
//! Vanilla readers stop at `SAVEGAME_EOF` (`P_ReadSaveGameEOF` reads one byte and
//! `G_DoLoadGame` closes the file), so the trailing block is ignored by room. The block is
//! only written while MW2 mode is active; otherwise save files are byte-identical to room's.

use std::ffi::c_void;

use dmw_weapons::SimSave;
use log::{debug, warn};
use serde::{Deserialize, Serialize};

const TARGET: &str = "dmw::save";

/// Block magic.
pub const MAGIC: [u8; 4] = *b"DMW1";
/// Current [`SaveBlock::version`].
pub const VERSION: u32 = 1;
/// Largest payload accepted on load (a block is a few hundred bytes).
pub const MAX_PAYLOAD: u32 = 1 << 20;

/// The DOOMMW2 part of a savegame. Kick, sway, spread, ADS and switch state are not saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveBlock {
    /// [`VERSION`].
    pub version: u32,
    /// `PLAYER_EXT[consoleplayer].pitch_q`.
    pub pitch_q: i32,
    /// MW2 mode was on when the game was saved.
    pub level_active: bool,
    /// Inventory, magazines and the held weapon, by name.
    pub sim: SimSave,
}

/// Magic + length + payload, or `None` if encoding fails.
pub fn encode(block: &SaveBlock) -> Option<Vec<u8>> {
    let payload = match postcard::to_allocvec(block) {
        Ok(p) => p,
        Err(e) => {
            warn!(target: TARGET, "cannot encode the DOOMMW2 save block: {e}");
            return None;
        }
    };
    let len = u32::try_from(payload.len()).ok()?;
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&payload);
    Some(out)
}

/// What followed `SAVEGAME_EOF`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trailer {
    /// Nothing, or bytes that are not a DOOMMW2 block (a vanilla save).
    Vanilla,
    /// A valid block.
    Block(SaveBlock),
    /// `DMW1` followed by something unreadable.
    Malformed(String),
}

/// Parses the bytes after `SAVEGAME_EOF`. `read(n)` returns up to `n` bytes from the file.
pub fn parse(mut read: impl FnMut(usize) -> Vec<u8>) -> Trailer {
    let magic = read(4);
    if magic.as_slice() != MAGIC {
        return Trailer::Vanilla;
    }
    let len = read(4);
    let Ok(len) = <[u8; 4]>::try_from(len.as_slice()) else {
        return Trailer::Malformed("truncated length".to_owned());
    };
    let len = u32::from_le_bytes(len);
    if len > MAX_PAYLOAD {
        return Trailer::Malformed(format!("payload length {len} is too large"));
    }
    let payload = read(len as usize);
    if payload.len() != len as usize {
        return Trailer::Malformed(format!(
            "payload truncated ({} of {len} bytes)",
            payload.len()
        ));
    }
    match postcard::from_bytes::<SaveBlock>(&payload) {
        Ok(b) if b.version == VERSION => Trailer::Block(b),
        Ok(b) => Trailer::Malformed(format!("unsupported block version {}", b.version)),
        Err(e) => Trailer::Malformed(format!("payload does not decode: {e}")),
    }
}

/// Appends `bytes` to the open savegame stream.
pub(crate) fn write_to_save_stream(bytes: &[u8]) -> bool {
    // SAFETY: called from `G_DoSaveGame` right after `P_WriteSaveGameEOF`, while `save_stream`
    // is the open, writable savegame file; `bytes` is a live slice.
    unsafe {
        let stream = crate::doom::p_saveg::save_stream;
        if stream.is_null() {
            return false;
        }
        libc::fwrite(bytes.as_ptr().cast::<c_void>(), 1, bytes.len(), stream) == bytes.len()
    }
}

/// Reads up to `n` bytes from the open savegame stream.
pub(crate) fn read_from_save_stream(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    // SAFETY: called from `G_DoLoadGame` right after `P_ReadSaveGameEOF` succeeded, while
    // `save_stream` is the open, readable savegame file; `buf` has room for `n` bytes.
    let got = unsafe {
        let stream = crate::doom::p_saveg::save_stream;
        if stream.is_null() {
            return Vec::new();
        }
        libc::fread(buf.as_mut_ptr().cast::<c_void>(), 1, n, stream)
    };
    buf.truncate(got);
    debug!(target: TARGET, "read {got} of {n} trailer bytes");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> SaveBlock {
        SaveBlock {
            version: VERSION,
            pitch_q: -640,
            level_active: true,
            sim: SimSave {
                owned: vec!["fixture_pistol".into(), "fixture_shotgun".into()],
                magazines: [
                    ("fixture_pistol".to_owned(), 7),
                    ("fixture_shotgun".to_owned(), 3),
                ]
                .into_iter()
                .collect(),
                last_held: Some("fixture_pistol".into()),
                held: Some("fixture_shotgun".into()),
            },
        }
    }

    fn reader(bytes: Vec<u8>) -> impl FnMut(usize) -> Vec<u8> {
        let mut pos = 0;
        move |n| {
            let end = (pos + n).min(bytes.len());
            let out = bytes[pos..end].to_vec();
            pos = end;
            out
        }
    }

    #[test]
    fn block_round_trips() {
        let bytes = encode(&block()).expect("encodes");
        assert_eq!(&bytes[..4], b"DMW1");
        let len = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
        assert_eq!(len as usize, bytes.len() - 8);
        assert_eq!(parse(reader(bytes)), Trailer::Block(block()));
    }

    #[test]
    fn vanilla_and_eof_trailers_are_vanilla() {
        assert_eq!(parse(reader(Vec::new())), Trailer::Vanilla);
        assert_eq!(parse(reader(b"XYZW1234".to_vec())), Trailer::Vanilla);
    }

    #[test]
    fn malformed_blocks_are_reported() {
        let mut bytes = encode(&block()).expect("encodes");
        bytes.truncate(bytes.len() - 3);
        assert!(matches!(parse(reader(bytes)), Trailer::Malformed(_)));
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&(MAX_PAYLOAD + 1).to_le_bytes());
        assert!(matches!(parse(reader(huge)), Trailer::Malformed(_)));
        let mut garbage = MAGIC.to_vec();
        garbage.extend_from_slice(&3u32.to_le_bytes());
        garbage.extend_from_slice(&[0xff, 0xff, 0xff]);
        assert!(matches!(parse(reader(garbage)), Trailer::Malformed(_)));
        let mut future = block();
        future.version = 2;
        let bytes = encode(&future).expect("encodes");
        assert!(matches!(parse(reader(bytes)), Trailer::Malformed(_)));
    }

    /// T8.7 parser half: room's loader reads exactly one byte after the archived state and
    /// checks it is `SAVEGAME_EOF`; a DOOMMW2 file is `vanilla bytes + 0x1d + block`, so the
    /// byte room checks is unchanged and everything after it is never read.
    #[test]
    fn vanilla_reader_sees_eof_before_the_block() {
        let mut file = vec![0x11, 0x22, 0x33, 0x1d];
        file.extend(encode(&block()).expect("encodes"));
        let eof_pos = 3;
        assert_eq!(file[eof_pos], 0x1d);
        let trailer = file[eof_pos + 1..].to_vec();
        assert_eq!(parse(reader(trailer)), Trailer::Block(block()));
    }
}
