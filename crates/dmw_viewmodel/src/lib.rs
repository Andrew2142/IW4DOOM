// SPDX-License-Identifier: GPL-3.0-or-later
//! Software viewmodel rasterizer and retro pipeline (sheet 11).
//!
//! Draws skinned MW2 viewmodels into Doom's 8-bit framebuffer using the WAD palette
//! and light tables. Receives poses in its own `Pose` type, so it never depends on the
//! simulation, the engine or iw4L.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    /// Touches the shared third-party dependencies so a broken manifest fails `cargo test`.
    #[test]
    fn links() {
        assert_eq!(glam::Mat4::IDENTITY.x_axis.x.to_bits(), 1.0f32.to_bits());
        assert_eq!(dmw_format::consts::DOOM_SCREEN_W, 320);
    }
}
