// SPDX-License-Identifier: GPL-3.0-or-later
//! Weapon switching and the vanilla handoff (sheet 07 §5.8).
//!
//! Doom decides *which* weapon; MW2 decides *how long* the switch takes. MW2-to-MW2 switches
//! are plain `weapon_iw4` switches (`WeaponCmd::cmd_weapon`). `weapon_iw4` has no "switch to
//! nothing" with the normal drop time (a `cmd_weapon` of 0 means "keep the current weapon",
//! weapon_iw4/src/tick.rs:905), so the vanilla handoff starts the ordinary drop itself with
//! the public [`weapon_iw4::begin_weapon_change`] and then holds it with
//! `WeaponCmd::mantle_weapon_inactive`, which makes `weapon_iw4` finish the drop holstered and
//! report `DropFinished` (weapon_change.rs:69, tick.rs:1066-1068).

use dmw_format::{DmwWeaponId, DoomWeapon};
use weapon_iw4::{
    PMF_CHANGE_BLOCK, WeaponCombatFacts, WeaponHandState, WeaponState, begin_weapon_change,
    check_for_change_admits,
};

/// Where the hand is going.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchState {
    /// Nothing pending; the hand holds its weapon (or nothing).
    #[default]
    Idle,
    /// `weapon_iw4` is dropping the current weapon and/or raising `target`.
    ToMw2 {
        /// Weapon being switched to.
        target: DmwWeaponId,
    },
    /// Dropping the MW2 weapon; then Doom raises `doom_weapon`'s psprite.
    ToVanilla {
        /// The Doom weapon that takes over.
        doom_weapon: DoomWeapon,
    },
    /// A vanilla weapon is up; `hands[0].weapon == 0`.
    VanillaHeld,
}

/// `true` when the hand is in one of the drop states.
pub fn is_dropping(hand: &WeaponHandState) -> bool {
    matches!(
        WeaponState::from_i32(hand.weaponstate),
        Ok(WeaponState::Dropping | WeaponState::DroppingQuick | WeaponState::DroppingAltswitch)
    )
}

/// Starts the ordinary (not quick) drop for the vanilla handoff once `weapon_iw4` would admit a
/// weapon change. Returns `true` when the hand is dropping afterwards.
///
/// Admission and the change block mirror `weapon_check_for_change`
/// (weapon_iw4/src/weapon_change.rs:60-106). `begin_weapon_change` forces a quick drop when its
/// target is 0 (weapon_change.rs:133), so the current weapon is passed as the target: the
/// argument only selects quick vs normal and is otherwise unused (weapon_change.rs:155).
pub fn begin_vanilla_drop(
    hand: &mut WeaponHandState,
    facts: &WeaponCombatFacts,
    pm_flags: u32,
) -> bool {
    if is_dropping(hand) {
        return true;
    }
    if hand.weapon == 0 {
        return false;
    }
    if !check_for_change_admits(hand.weaponstate, hand.weapon_time, hand.weapon_delay) {
        return false;
    }
    if pm_flags & PMF_CHANGE_BLOCK != 0 {
        return false;
    }
    let _ = begin_weapon_change(hand, facts, hand.weapon, false, pm_flags);
    is_dropping(hand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_drop_uses_normal_drop_time() {
        let pack = dmw_format::Pack::fixture();
        let facts = pack.facts(DmwWeaponId(2)).combat;
        let mut hand = weapon_iw4::spawn_weapon_hand(2, &facts, false);
        hand.weaponstate = WeaponState::Ready as i32;
        hand.weapon_time = 0;
        assert!(begin_vanilla_drop(&mut hand, &facts, 0));
        assert_eq!(hand.weaponstate, WeaponState::Dropping as i32);
        assert_eq!(hand.weapon_time, facts.drop_time_ms);
    }

    #[test]
    fn vanilla_drop_waits_while_firing() {
        let pack = dmw_format::Pack::fixture();
        let facts = pack.facts(DmwWeaponId(2)).combat;
        let mut hand = weapon_iw4::spawn_weapon_hand(2, &facts, false);
        hand.weaponstate = WeaponState::Firing as i32;
        hand.weapon_time = 50;
        hand.weapon_delay = 10;
        assert!(!begin_vanilla_drop(&mut hand, &facts, 0));
        assert_eq!(hand.weaponstate, WeaponState::Firing as i32);
    }
}
