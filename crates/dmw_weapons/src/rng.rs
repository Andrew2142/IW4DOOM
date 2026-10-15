// SPDX-License-Identifier: GPL-3.0-or-later
//! `DmwRng`: the single deterministic random stream of the weapon simulation (sheet 07 §5.10).
//!
//! Consumption order inside one tic is fixed: four recoil samples per accepted shot
//! ([`crate::view_kick::ViewKickState::seed_fire`]), then two draws per pellet
//! ([`crate::spread::spread_direction_on_plane`]). Nothing else reads this stream.

/// xorshift32 stream, seeded per level from `(gameskill, gameepisode, gamemap)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmwRng(u32);

impl Default for DmwRng {
    fn default() -> Self {
        Self::seeded(0, 0, 0)
    }
}

/// splitmix32 finaliser, used only to spread the level key over 32 bits.
const fn splitmix32(mut z: u32) -> u32 {
    z = z.wrapping_add(0x9E37_79B9);
    z = (z ^ (z >> 16)).wrapping_mul(0x85EB_CA6B);
    z = (z ^ (z >> 13)).wrapping_mul(0xC2B2_AE35);
    z ^ (z >> 16)
}

impl DmwRng {
    /// Seeds from Doom's level identity: splitmix32 of `skill << 16 ^ episode << 8 ^ map`.
    /// The state is never 0 (xorshift would stick there).
    pub fn seeded(skill: i32, episode: i32, map: i32) -> Self {
        Self::from_seed(Self::level_seed(skill, episode, map))
    }

    /// The `u32` seed [`DmwRng::seeded`] uses; sheet 08 passes it as `LevelStart::seed`.
    pub fn level_seed(skill: i32, episode: i32, map: i32) -> u32 {
        let key = ((skill as u32) << 16) ^ ((episode as u32) << 8) ^ (map as u32);
        splitmix32(key)
    }

    /// Uses `seed` directly as the state (0 becomes 1). `LevelStart::seed` goes through this.
    pub fn from_seed(seed: u32) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    /// Next value: xorshift32 with shifts 13/17/5, the generator of iw4L's `ViewKickState`
    /// (render_anim/src/anim/view_kick_state.rs:117-124), including its never-zero guard.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = if x == 0 { 1 } else { x };
        x
    }

    /// `next_u32() as f32 / u32::MAX as f32`, in `[0, 1]`.
    pub fn unit01(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }

    /// Current state (for tests and debugging).
    pub fn state(&self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_is_deterministic_and_nonzero() {
        let a = DmwRng::seeded(2, 1, 1);
        let b = DmwRng::seeded(2, 1, 1);
        assert_eq!(a, b);
        assert_ne!(a.state(), 0);
        assert_ne!(DmwRng::seeded(2, 1, 2), a);
        assert_eq!(DmwRng::from_seed(0).state(), 1);
        assert_eq!(DmwRng::from_seed(DmwRng::level_seed(2, 1, 1)), a);
    }

    #[test]
    fn xorshift_matches_reference_values() {
        // Marsaglia xorshift32 (13/17/5) from state 1: 270369, 67634689, ...
        let mut r = DmwRng::from_seed(1);
        assert_eq!(r.next_u32(), 270_369);
        assert_eq!(r.next_u32(), 67_634_689);
        let u = r.unit01();
        assert!((0.0..=1.0).contains(&u));
    }
}
