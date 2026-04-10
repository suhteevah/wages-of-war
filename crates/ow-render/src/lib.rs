//! # ow-render — Isometric Tile/Sprite Renderer
//!
//! ## Modules
//!
//! - [`iso_math`] — Isometric coordinate math (screen ↔ tile conversions)
//! - [`palette`] — Palette-indexed → RGBA conversion + PCX palette extraction
//! - [`sprite_renderer`] — SDL2 texture creation and drawing for decoded sprites
//! - [`viewer`] — Interactive sprite viewer (developer tool)

pub mod iso_math;
pub mod palette;
pub mod sprite_renderer;
pub mod viewer;
