// SPDX-License-Identifier: GPL-3.0-or-later
//! Door and switch prompt (sheet 12 section 5.6): "Press F to open", "Press F to use",
//! "Needs blue key".
//!
//! Each frame in MW2 mode this module repeats the geometry of `P_UseLines` /
//! `PTR_UseTraverse` (a `USERANGE` trace from the player along `mo->angle`, first line with a
//! special wins, a special-less line only lets the trace through when it has an opening) but
//! activates nothing. The pure parts ([`classify`], [`key_label`], [`prompt_text`]) are unit
//! tested; [`current`] is the engine-facing probe.

use std::cell::Cell;
use std::ffi::{c_int, c_uint};

use crate::doom::c_ffi::{intercept_t, line_t};
use crate::doom::d_player::PlayerT;
use crate::doom::m_fixed::{FRACBITS, FRACUNIT};
use crate::doom::p_maputl::{
    openrange, P_LineOpening, P_PathTraverse, P_PointOnLineSide, PT_ADDLINES,
};
use crate::doom::tables::{finecosine, finesine, ANGLETOFINESHIFT};

use super::config::KEY_NAMES;

/// `USERANGE` of `p_map.rs`: 64 map units.
const USERANGE: c_int = 64 * FRACUNIT;

/// Key colour a locked special asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyColour {
    /// Blue card or skull.
    Blue,
    /// Yellow card or skull.
    Yellow,
    /// Red card or skull.
    Red,
}

impl KeyColour {
    fn name(self) -> &'static str {
        match self {
            KeyColour::Blue => "blue",
            KeyColour::Yellow => "yellow",
            KeyColour::Red => "red",
        }
    }
}

/// What kind of use-activated special a line carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseKind {
    /// Manual door (`DR`/`D1`): "open".
    Door,
    /// Switch, button or lift: "use".
    Switch,
}

/// A use-activated special: its kind and the key it needs, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UseSpecial {
    /// Door or switch wording.
    pub kind: UseKind,
    /// Key colour required, if locked.
    pub key: Option<KeyColour>,
}

/// Classifies a line special exactly as `P_UseSpecialLine` handles it; walk, shoot and
/// other-thing-only specials return `None`.
pub fn classify(special: i16) -> Option<UseSpecial> {
    let door = |key| {
        Some(UseSpecial {
            kind: UseKind::Door,
            key,
        })
    };
    let switch = |key| {
        Some(UseSpecial {
            kind: UseKind::Switch,
            key,
        })
    };
    match special {
        1 | 31 | 117 | 118 => door(None),
        26 | 32 => door(Some(KeyColour::Blue)),
        27 | 34 => door(Some(KeyColour::Yellow)),
        28 | 33 => door(Some(KeyColour::Red)),
        99 | 133 => switch(Some(KeyColour::Blue)),
        134 | 135 => switch(Some(KeyColour::Red)),
        136 | 137 => switch(Some(KeyColour::Yellow)),
        // S1/SR switches, buttons and lifts.
        7 | 9 | 11 | 14 | 15 | 18 | 20 | 21 | 23 | 29 | 41 | 49 | 50 | 51 | 55 | 71 | 101 | 102
        | 103 | 111 | 112 | 113 | 122 | 127 | 131 | 140 | 42 | 43 | 45 | 60 | 61 | 62 | 63 | 64
        | 65 | 66 | 67 | 68 | 69 | 70 | 114 | 115 | 116 | 123 | 132 | 138 | 139 => switch(None),
        _ => None,
    }
}

/// Label of a Doom key code for the prompt: a letter upper-cased, a name from
/// [`KEY_NAMES`] upper-cased (`SPACE`, `ENTER`), else `USE`.
pub fn key_label(code: i32) -> String {
    if code <= 0 {
        return "USE".to_owned();
    }
    if let Some(&(name, _)) = KEY_NAMES.iter().find(|(_, c)| i32::from(*c) == code) {
        return name.to_ascii_uppercase();
    }
    match u8::try_from(code) {
        Ok(b) if b.is_ascii_graphic() => char::from(b).to_ascii_uppercase().to_string(),
        _ => "USE".to_owned(),
    }
}

/// The prompt text for a special given the use key and whether the key is held.
pub fn prompt_text(special: UseSpecial, use_key: i32, has_key: bool) -> String {
    match special.key {
        Some(colour) if !has_key => format!("Needs {} key", colour.name()),
        _ => {
            let verb = match special.kind {
                UseKind::Door => "open",
                UseKind::Switch => "use",
            };
            format!("Press {} to {verb}", key_label(use_key))
        }
    }
}

thread_local! {
    /// The line the probe's `PTR_UseTraverse` copy stopped at, and the side the player is on.
    static HIT: Cell<Option<(*mut line_t, c_int)>> = const { Cell::new(None) };
    /// The player position the probe tests the side against.
    static USE_AT: Cell<(c_int, c_int)> = const { Cell::new((0, 0)) };
}

/// Copy of `PTR_UseTraverse` without the activation and the "oof" sound.
unsafe extern "C" fn probe_traverse(in_: *mut intercept_t) -> c_uint {
    // SAFETY: `P_PathTraverse` passes a valid intercept with `d.line` set (`PT_ADDLINES`).
    unsafe {
        let line = (*in_).d.line;
        if (*line).special == 0 {
            P_LineOpening(line);
            return u32::from(openrange > 0);
        }
        let (x, y) = USE_AT.get();
        let side = P_PointOnLineSide(x, y, line);
        HIT.set(Some((line, side)));
        0
    }
}

/// `true` when every sector tagged by `line` already runs a thinker (the action would fail).
///
/// # Safety
/// `line` must be a live map line.
unsafe fn tagged_sectors_busy(line: *mut line_t) -> bool {
    use crate::doom::p_setup::sectors;
    use crate::doom::p_spec::P_FindSectorFromLineTag;
    let mut any = false;
    let mut n = -1;
    loop {
        // SAFETY: caller contract; `P_FindSectorFromLineTag` only reads the sector array.
        n = unsafe { P_FindSectorFromLineTag(line, n) };
        if n < 0 {
            break;
        }
        any = true;
        // SAFETY: `n` indexes `sectors`.
        if unsafe { (*sectors.add(n as usize)).specialdata.is_null() } {
            return false;
        }
    }
    any
}

/// `true` while a repeatable switch's button texture is still counting down.
///
/// # Safety
/// Engine globals on the engine thread.
unsafe fn button_active(line: *mut line_t) -> bool {
    use crate::doom::p_switch::buttonlist;
    // SAFETY: caller contract.
    unsafe {
        let first = (&raw const buttonlist).cast::<crate::doom::p_switch::button_t>();
        (0..crate::doom::p_switch::MAXBUTTONS).any(|i| {
            let b = first.add(i);
            (*b).btimer > 0 && std::ptr::eq((*b).line.cast::<line_t>(), line)
        })
    }
}

fn has_key(player: &PlayerT, colour: KeyColour) -> bool {
    let (card, skull) = match colour {
        KeyColour::Blue => (0, 3),
        KeyColour::Yellow => (1, 4),
        KeyColour::Red => (2, 5),
    };
    player.cards[card] != 0 || player.cards[skull] != 0
}

/// What pressing Use would do right now, as prompt text, or `None` when nothing (or nothing
/// useful) is in reach.
///
/// # Safety
/// `player` must be a live player with a map object, on the engine thread with a level loaded.
pub unsafe fn current(player: &PlayerT, use_key: i32) -> Option<String> {
    use crate::doom::p_setup::sides;
    // SAFETY: caller contract; all reads of engine globals on the engine thread.
    unsafe {
        let mo = player.mo.cast::<crate::doom::c_ffi::mobj_t>();
        if mo.is_null() {
            return None;
        }
        let angle = ((*mo).angle >> ANGLETOFINESHIFT) as usize;
        let (x1, y1) = ((*mo).x, (*mo).y);
        let range = USERANGE >> FRACBITS;
        let x2 = x1 + range * *finecosine.0.add(angle);
        let y2 = y1 + range * finesine[angle];
        HIT.set(None);
        USE_AT.set((x1, y1));
        P_PathTraverse(x1, y1, x2, y2, PT_ADDLINES, Some(probe_traverse));
        let (line, side) = HIT.take()?;
        // `P_UseSpecialLine` ignores the back of every handled special.
        if side != 0 {
            return None;
        }
        let special = classify((*line).special)?;
        let missing = special.key.is_some_and(|c| !has_key(player, c));
        if !missing {
            match special.kind {
                UseKind::Door => {
                    // `EV_VerticalDoor` works on the sector behind the line; a moving or open
                    // door has a thinker there and use would only reverse it.
                    let back = (*line).sidenum[1];
                    if back < 0 {
                        return None;
                    }
                    let sec = (*sides.offset(back as isize)).sector;
                    if sec.is_null() || !(*sec).specialdata.is_null() {
                        return None;
                    }
                }
                UseKind::Switch => {
                    if button_active(line) || tagged_sectors_busy(line) {
                        return None;
                    }
                }
            }
        }
        Some(prompt_text(special, use_key, !missing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_and_shoot_specials_never_prompt() {
        // W1 door open, WR door, G1/GR door, W1 exit, teleport.
        for s in [2, 3, 4, 10, 12, 39, 46, 24, 52, 97, 86, 88, 0, 124] {
            assert_eq!(classify(s), None, "special {s}");
        }
    }

    #[test]
    fn doors_switches_and_locks_classify() {
        let door = classify(1).unwrap();
        assert_eq!((door.kind, door.key), (UseKind::Door, None));
        assert_eq!(classify(26).unwrap().key, Some(KeyColour::Blue));
        assert_eq!(classify(27).unwrap().key, Some(KeyColour::Yellow));
        assert_eq!(classify(28).unwrap().key, Some(KeyColour::Red));
        assert_eq!(classify(33).unwrap().key, Some(KeyColour::Red));
        assert_eq!(classify(34).unwrap().key, Some(KeyColour::Yellow));
        assert_eq!(classify(11).unwrap().kind, UseKind::Switch);
        assert_eq!(classify(62).unwrap().kind, UseKind::Switch);
        assert_eq!(classify(133).unwrap().key, Some(KeyColour::Blue));
        assert_eq!(classify(137).unwrap().key, Some(KeyColour::Yellow));
    }

    #[test]
    fn text_uses_the_bound_key() {
        let door = classify(1).unwrap();
        let sw = classify(11).unwrap();
        assert_eq!(prompt_text(door, i32::from(b'f'), true), "Press F to open");
        assert_eq!(prompt_text(sw, i32::from(b'e'), true), "Press E to use");
        assert_eq!(
            prompt_text(door, i32::from(b' '), true),
            "Press SPACE to open"
        );
        assert_eq!(prompt_text(door, 0, true), "Press USE to open");
        let locked = classify(27).unwrap();
        assert_eq!(
            prompt_text(locked, i32::from(b'f'), false),
            "Needs yellow key"
        );
        assert_eq!(
            prompt_text(locked, i32::from(b'f'), true),
            "Press F to open"
        );
    }
}
