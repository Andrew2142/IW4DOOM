// SPDX-License-Identifier: GPL-3.0-or-later
//! Software viewmodel rasterizer and retro pipeline (sheet 11, crate half).
//!
//! Draws skinned MW2 viewmodels into Doom's 8-bit framebuffer using the WAD palette
//! and light tables. Receives poses in its own [`Pose`] type, so it never depends on the
//! simulation, the engine or iw4L.
//!
//! Pipeline:
//!
//! 1. [`PaletteTables::from_wad_lumps`] (once per IWAD) builds the nearest-colour LUT.
//! 2. [`PreparedViewmodel::new`] (once per weapon at level start) quantizes the textures with
//!    [`quantize`] and detects the triangle winding.
//! 3. [`Rasterizer::draw`] (every frame) skins, projects, clips and rasterizes into a
//!    [`DrawTarget`], shading through the COLORMAP selected by [`LightParams`].
//!
//! The 2D blitters in [`overlay`] draw Doom patches (muzzle flash) and indexed images (scope
//! overlay, sheet 12).
#![forbid(unsafe_code)]

mod palette;
mod prepared;
mod quantize;
mod raster;
mod shade;
mod skin;

pub mod overlay;

pub use dmw_format::{AlphaMode, RgbaImage};
pub use palette::{PaletteTables, colour_distance};
pub use prepared::{PreparedViewmodel, Winding};
pub use quantize::{Dither, IndexedMip, IndexedTexture, quantize};
pub use raster::{NEAR_PLANE, Rasterizer};
pub use shade::{FUZZ_COLORMAP, LIGHT_DIR};

/// Errors from building palette tables or preparing a viewmodel. Drawing never fails.
#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    /// PLAYPAL or COLORMAP lump too short or otherwise unusable.
    #[error("palette: {0}")]
    Palette(String),
    /// The viewmodel mesh is structurally broken (array lengths, index ranges).
    #[error("mesh: {0}")]
    Mesh(String),
}

/// A view of Doom's screen buffer plus the view window the gun is drawn into.
#[derive(Debug)]
pub struct DrawTarget<'a> {
    /// Doom screen buffer (normally 320*200), row-major palette indices.
    pub pixels: &'a mut [u8],
    /// Bytes per row (320).
    pub pitch: usize,
    /// View window left edge in screen pixels.
    pub view_x: i32,
    /// View window top edge in screen pixels.
    pub view_y: i32,
    /// View window width in pixels.
    pub view_w: i32,
    /// View window height in pixels.
    pub view_h: i32,
    /// Principal point inside the view window, **unsheared**: (view_w/2, view_h/2). Under
    /// y-shearing the aim direction always projects to the unsheared centre and the viewmodel
    /// is camera-attached, so it must not move with the world's shear (sheet 06 section 5.2).
    pub center_x: f32,
    /// See [`DrawTarget::center_x`].
    pub center_y: f32,
    /// Displayed pixel width / height (5/6 for 320x200 shown at 4:3).
    pub pixel_aspect: f32,
}

/// Eye-space skinning matrices, one per DObj bone (MW2 view axes: x forward, y left, z up;
/// inches), already multiplied by the inverse bind matrix and the gun placement. Same shape as
/// `dmw_weapons::ViewmodelPose`; the engine converts (01 section 5.3).
#[derive(Debug, Clone, Default)]
pub struct Pose {
    /// One matrix per bone. Bones missing from the vector are treated as identity.
    pub skin: Vec<glam::Mat4>,
    /// Eye-space transform of `tag_flash`, when the weapon has one.
    pub tag_flash: Option<glam::Mat4>,
}

/// Which optional parts are drawn (converted from `dmw_weapons::SurfaceMask`).
/// `PartRole::ScopeLens` surfaces are never drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Visibility {
    /// Draw `PartRole::Rocket` surfaces.
    pub rocket: bool,
    /// Draw `PartRole::Knife` surfaces.
    pub knife: bool,
}

/// How the gun is lit.
#[derive(Debug)]
pub enum LightParams<'a> {
    /// Sector lighting: COLORMAP level (0 bright .. 31 dark) plus the optional directional term.
    Sector {
        /// Base COLORMAP level, 0 bright .. 31 dark.
        colormap_level: u8,
        /// Add the per-vertex directional term (sheet 11 section 5.6).
        directional: bool,
    },
    /// One fixed colormap for every pixel (Doom's `fixedcolormap`).
    Fixed {
        /// Index into `colormaps`: 0 fullbright, 32 invulnerability.
        colormap: u8,
    },
    /// Partial invisibility: Doom's fuzz effect over the gun's silhouette.
    Fuzz {
        /// Doom's `fuzzoffset` table (offsets in pixels, normally +-320).
        fuzzoffset: &'a [i32],
        /// Doom's `fuzzpos`; advanced per fuzzed pixel and wrapped at `fuzzoffset.len()`.
        fuzzpos: &'a mut usize,
    },
}

/// Everything [`Rasterizer::draw`] needs besides the target and the model.
#[derive(Debug)]
pub struct ViewmodelDrawParams<'a> {
    /// Skinning matrices.
    pub pose: &'a Pose,
    /// Horizontal FOV across the view window (`ViewState::viewmodel_fov_deg`).
    pub hfov_deg: f32,
    /// Lighting mode.
    pub light: LightParams<'a>,
    /// Palette and light tables.
    pub pal: &'a PaletteTables,
    /// Optional part visibility.
    pub visibility: Visibility,
}

/// Counters from one [`Rasterizer::draw`] call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Triangles in visible surfaces (after role filtering).
    pub triangles_in: u32,
    /// Triangles (after clipping) that reached the scan converter.
    pub triangles_drawn: u32,
    /// Pixels written.
    pub pixels: u32,
    /// Wall time of the call in microseconds.
    pub micros: u32,
}
