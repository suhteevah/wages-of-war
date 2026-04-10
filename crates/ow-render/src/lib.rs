//! # ow-render — Isometric Tile/Sprite Renderer
//!
//! ## Modules
//!
//! - [`iso_math`] — Isometric coordinate math (screen ↔ tile conversions)
//! - [`palette`] — Palette-indexed → RGBA conversion + PCX palette extraction
//! - [`sprite_renderer`] — SDL2 texture creation and drawing for decoded sprites
//! - [`camera`] — Viewport camera with scroll and zoom
//! - [`tile_renderer`] — Isometric tile map renderer (TIL-based)
//! - [`viewer`] — Interactive sprite viewer (developer tool)

pub mod camera;
pub mod iso_math;
pub mod palette;
pub mod sprite_renderer;
pub mod tile_renderer;
pub mod viewer;
pub mod anim_controller;
pub mod unit_renderer;
pub mod hud;
pub mod ui;
pub mod text;
