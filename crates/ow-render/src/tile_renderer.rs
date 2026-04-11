//! # Tile Map Renderer — Isometric tile grid rendering
//!
//! Renders a parsed [`GameMap`] using tile sprites from a `.TIL` sprite sheet.
//!
//! ## Rendering pipeline
//!
//! 1. **Load tileset** — RLE-decode each tile sprite from the TIL container,
//!    apply the 256-color palette, upload as SDL2 textures. This is done once
//!    at startup via [`TileMapRenderer::load_tileset`].
//!
//! 2. **Render frame** — Each frame, iterate visible tiles in painter's
//!    algorithm order and blit the corresponding tile texture to the canvas.
//!
//! ## Painter's algorithm and draw order
//!
//! Isometric rendering requires drawing tiles from back to front so that
//! closer tiles correctly overlap farther ones. In a standard diamond-grid
//! isometric projection:
//!
//! - Tiles with smaller Y come first (they are "further back" on screen).
//! - Within the same Y row, tiles with smaller X come first.
//!
//! This back-to-front order ensures that tiles closer to the camera (higher Y,
//! higher X) are drawn on top of tiles further away, producing correct
//! occlusion without a depth buffer.
//!
//! ## Frustum culling
//!
//! Only tiles within the camera's visible bounds are drawn. The camera
//! computes the tile coordinate range that maps to the screen viewport,
//! and we skip everything outside that range. For a 200x252 map this
//! avoids drawing ~50,000 tiles per frame when only ~500 are visible.

use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use tracing::{debug, trace, warn};

use crate::camera::Camera;
use crate::iso_math::{IsoConfig, TilePos};
use crate::palette::{apply_palette_with_brightness, Palette256};
use ow_data::map_loader::GameMap;
use ow_data::sprite::SpriteSheet;

/// Manages tile textures and renders the isometric tile map.
///
/// Tile textures are decoded from a `.TIL` sprite sheet and cached as SDL2
/// textures. Each texture is keyed by its sprite index in the TIL file.
pub struct TileMapRenderer<'tc> {
    /// SDL2 texture creator, needed to build textures from pixel data.
    texture_creator: &'tc TextureCreator<WindowContext>,
    /// Cached tile textures, indexed by tile sprite index.
    /// A `None` entry means the tile had zero dimensions and was skipped.
    tile_textures: Vec<Option<Texture<'tc>>>,
    /// Width of each tile texture in pixels (from the TIL sprite dimensions).
    /// Used to compute destination rectangles during rendering.
    tile_pixel_width: u32,
    /// Height of each tile texture in pixels.
    tile_pixel_height: u32,
}

impl<'tc> TileMapRenderer<'tc> {
    /// Create a new tile map renderer.
    ///
    /// No textures are loaded yet — call [`load_tileset`] to decode and
    /// upload tile sprites before rendering.
    pub fn new(texture_creator: &'tc TextureCreator<WindowContext>) -> Self {
        Self {
            texture_creator,
            tile_textures: Vec::new(),
            tile_pixel_width: 0,
            tile_pixel_height: 0,
        }
    }

    /// Decode all tile sprites from a TIL sprite sheet and create SDL2 textures.
    ///
    /// Each tile frame is RLE-decoded, palette-mapped to RGBA, and uploaded
    /// to an SDL2 static texture with alpha blending enabled.
    ///
    /// The tile dimensions are read from the first non-zero sprite in the
    /// sheet. All TIL tiles in a given scenario share the same dimensions
    /// (typically 128x63 for Wages of War isometric tiles).
    pub fn load_tileset(
        &mut self,
        tileset: &SpriteSheet,
        palette: &Palette256,
    ) -> Result<(), String> {
        let frame_count = tileset.frames.len();
        debug!(frame_count, "loading tileset into GPU textures");

        self.tile_textures.clear();
        self.tile_textures.reserve(frame_count);

        // Determine tile pixel dimensions from the first valid frame.
        // All tiles in a TIL file share the same dimensions.
        let mut found_dims = false;
        for frame in &tileset.frames {
            if frame.header.width > 0 && frame.header.height > 0 {
                self.tile_pixel_width = frame.header.width as u32;
                self.tile_pixel_height = frame.header.height as u32;
                found_dims = true;
                debug!(
                    width = self.tile_pixel_width,
                    height = self.tile_pixel_height,
                    "tile dimensions detected from TIL sprite sheet"
                );
                break;
            }
        }

        if !found_dims {
            return Err("tileset contains no valid (non-zero) sprite frames".into());
        }

        for (i, frame) in tileset.frames.iter().enumerate() {
            let w = frame.header.width as u32;
            let h = frame.header.height as u32;

            if w == 0 || h == 0 {
                trace!(index = i, "skipping zero-dimension tile");
                self.tile_textures.push(None);
                continue;
            }

            // Step 1: RLE decode compressed pixel data to palette indices.
            let pixels = match ow_data::sprite::decode_rle(
                &frame.compressed_data,
                frame.header.width,
                frame.header.height,
                i,
            ) {
                Ok(p) => p,
                Err(e) => {
                    warn!(index = i, error = %e, "RLE decode failed for tile, inserting blank");
                    self.tile_textures.push(None);
                    continue;
                }
            };

            // Step 2: Apply palette to convert indexed pixels to RGBA.
            // Boost brightness by 1.5x to compensate for CRT-to-LCD gamma difference.
            // The original game was designed for CRT displays with higher inherent gamma,
            // making the dark jungle tiles look much brighter than on modern LCDs.
            let rgba = apply_palette_with_brightness(&pixels, palette, 1.5);

            // Step 3: Create SDL2 texture and upload RGBA pixel data.
            let mut texture = self
                .texture_creator
                .create_texture_static(PixelFormatEnum::RGBA32, w, h)
                .map_err(|e| format!("failed to create tile texture {i}: {e}"))?;

            texture
                .update(None, &rgba, (w * 4) as usize)
                .map_err(|e| format!("failed to upload tile pixels {i}: {e}"))?;

            // Step 4: Enable alpha blending so transparent pixels (index 0) are invisible.
            texture.set_blend_mode(sdl2::render::BlendMode::Blend);

            trace!(index = i, w, h, "tile texture created");
            self.tile_textures.push(Some(texture));
        }

        debug!(
            loaded = self.tile_textures.iter().filter(|t| t.is_some()).count(),
            skipped = self.tile_textures.iter().filter(|t| t.is_none()).count(),
            "tileset loading complete"
        );

        Ok(())
    }

    /// Render the isometric tile map to the canvas.
    ///
    /// Iterates all tiles within the camera's visible bounds in painter's
    /// algorithm order (back to front), converting each tile position to
    /// screen coordinates and blitting the corresponding tile texture.
    ///
    /// ## Draw order
    ///
    /// The painter's algorithm for isometric grids draws in row-major order
    /// with increasing Y first (further rows are "behind"), then increasing X
    /// within each row. This ensures tiles in the foreground correctly
    /// overdraw tiles in the background.
    ///
    /// ## Frustum culling
    ///
    /// Only tiles within `camera.visible_tile_bounds()` are considered.
    /// Additionally, border tiles (marked `is_border == true` in the map data)
    /// are skipped — these are padding cells that contain no meaningful graphics.
    pub fn render_map(
        &self,
        canvas: &mut Canvas<Window>,
        map: &GameMap,
        camera: &Camera,
        iso: &IsoConfig,
    ) {
        let (min_x, min_y, max_x, max_y) = camera.visible_tile_bounds(iso);

        // Clamp bounds to the actual map dimensions to avoid out-of-bounds lookups.
        let min_x = min_x.max(0) as usize;
        let min_y = min_y.max(0) as usize;
        let max_x = (max_x as usize).min(map.width().saturating_sub(1));
        let max_y = (max_y as usize).min(map.active_rows().saturating_sub(1));

        let mut tiles_drawn: u32 = 0;
        let mut tiles_skipped: u32 = 0;

        // Painter's algorithm: iterate rows from back (low Y) to front (high Y),
        // and within each row from left (low X) to right (high X).
        // This ensures correct occlusion in the isometric diamond projection.
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                let tile = match map.get_tile(tx, ty) {
                    Some(t) => t,
                    None => continue,
                };

                // Skip border/padding cells — they have no valid tile graphics.
                if tile.is_border {
                    tiles_skipped += 1;
                    continue;
                }

                // Look up the tile texture by the primary terrain layer.
                // layer0 is a 9-bit index (0-511) into the TIL sprite sheet,
                // already unpacked by the MAP parser from the packed u32 cell word.
                let tex_idx = tile.layer0 as usize;
                let texture = match self.tile_textures.get(tex_idx) {
                    Some(Some(tex)) => tex,
                    _ => {
                        tiles_skipped += 1;
                        continue;
                    }
                };

                // Convert tile grid position to world-space screen coordinates
                // using the isometric projection.
                let world_pos = iso.tile_to_screen(TilePos {
                    x: tx as i32,
                    y: ty as i32,
                });

                // Apply camera transform to get final screen position.
                let screen_pos = camera.world_to_screen(world_pos);

                // The tile texture's anchor point is its top-left corner.
                // For diamond isometric tiles, the tile_to_screen gives us the
                // top center of the diamond. We need to offset left by half
                // the tile pixel width so the diamond is centered on the grid point.
                let draw_x = screen_pos.x - (self.tile_pixel_width as f32 * camera.zoom) / 2.0;
                let draw_y = screen_pos.y;

                let dst_w = (self.tile_pixel_width as f32 * camera.zoom) as u32;
                let dst_h = (self.tile_pixel_height as f32 * camera.zoom) as u32;

                let dst = Rect::new(draw_x as i32, draw_y as i32, dst_w, dst_h);
                if let Err(e) = canvas.copy(texture, None, dst) {
                    warn!(tx, ty, error = %e, "failed to draw tile");
                }

                // Render overlay layers (layer1 and layer2) on top of the base tile.
                // These contain buildings, paths, vegetation, and detail sprites
                // composited over the base terrain.
                // Skip indices 500+ — these are marker/debug sprites in the TIL file
                // (tiny sprites with ~213 bytes, not real terrain graphics).
                for overlay_layer in [tile.layer1, tile.layer2] {
                    if overlay_layer > 0 && overlay_layer < 380 {
                        if let Some(Some(overlay_tex)) = self.tile_textures.get(overlay_layer as usize) {
                            if let Err(e) = canvas.copy(overlay_tex, None, dst) {
                                trace!(tx, ty, layer = overlay_layer, error = %e, "overlay draw failed");
                            }
                        }
                    }
                }

                tiles_drawn += 1;
            }
        }

        trace!(
            tiles_drawn,
            tiles_skipped,
            bounds = ?(min_x, min_y, max_x, max_y),
            "map frame rendered"
        );
    }

    /// Returns the pixel width of tile sprites in this tileset.
    pub fn tile_pixel_width(&self) -> u32 {
        self.tile_pixel_width
    }

    /// Returns the pixel height of tile sprites in this tileset.
    pub fn tile_pixel_height(&self) -> u32 {
        self.tile_pixel_height
    }

    /// Returns the number of tile textures loaded (including blank slots).
    pub fn tile_count(&self) -> usize {
        self.tile_textures.len()
    }
}
