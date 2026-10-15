// SPDX-License-Identifier: GPL-3.0-or-later
//! What the simulation reports each tic (00 §7.2).

use dmw_format::DmwWeaponId;

/// One thing that happened during a tic, in the order `weapon_iw4` produced it.
#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    /// A shot left the barrel (hitscan pellets or one projectile).
    Shot(ShotRequest),
    /// A melee swing reached its hit frame.
    Melee(MeleeRequest),
    /// A weapon sound to play now.
    Sound(SoundCue),
    /// Trigger pulled on an empty magazine.
    EmptyClick,
    /// A reload began.
    ReloadStarted {
        /// The magazine was empty when the reload started.
        empty: bool,
    },
    /// A segmented reload entered its end segment.
    ReloadFinished,
    /// A weapon started raising.
    RaiseStarted(DmwWeaponId),
    /// The weapon in hand finished dropping for the vanilla handoff (sheet 07 §5.8).
    DropFinished(DmwWeaponId),
    /// Muzzle flash for the viewmodel and Doom's light flash.
    MuzzleFlash {
        /// The firing weapon.
        weapon: DmwWeaponId,
        /// The shot emptied the magazine.
        last_shot: bool,
    },
}

/// A fired shot for sheet 09 to resolve.
#[derive(Clone, Debug, PartialEq)]
pub struct ShotRequest {
    /// The firing weapon.
    pub weapon: DmwWeaponId,
    /// Unit directions, MW2 world frame = Doom world axes (x, y, z-up).
    pub pellets: Vec<[f32; 3]>,
    /// `facts.combat.damage`.
    pub base_damage: i32,
    /// `facts.combat.bullet_range()`, inches.
    pub max_range_mw2: f32,
    /// `true`: spawn the weapon's `ProjectileFacts` missile instead of hitscan.
    pub projectile: bool,
}

/// A melee hit request for sheet 09.
#[derive(Clone, Debug, PartialEq)]
pub struct MeleeRequest {
    /// The weapon in hand.
    pub weapon: DmwWeaponId,
    /// `facts.combat.melee_damage`.
    pub damage: i32,
    /// Reach in inches (`WeaponCmd::player_melee_range`).
    pub range_mw2: f32,
    /// Unit view direction.
    pub dir: [f32; 3],
}

/// A sound alias to play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundCue {
    /// Pack sound alias.
    pub alias: String,
    /// Non-positional (the local player's own weapon).
    pub local: bool,
}
