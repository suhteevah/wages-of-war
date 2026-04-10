//! # Sprite Renderer — SDL2 sprite display
//!
//! Rendering pipeline for palette-indexed sprites:
//!
//! 1. **Raw indexed pixels** — output of `decode_rle()`, a flat `Vec<u8>` where
//!    each byte is a palette index (0 = transparent).
//!
//! 2. **Palette lookup** — `palette::apply_palette()` converts each index to
//!    an (R, G, B, A) quad. Index 0 becomes fully transparent (A=0).
//!
//! 3. **RGBA texture** — the RGBA buffer is uploaded to an SDL2 `Texture` via
//!    `update()`. The texture uses `RGBA32` pixel format and
//!    `TextureAccess::Static` (immutable, GPU-resident).
//!
//! 4. **SDL2 draw** — `Canvas::copy()` blits the texture to a destination
//!    rectangle on screen. SDL2 handles alpha blending automatically when the
//!    texture blend mode is set to `Blend`.
//!
//! The `SpriteRenderer` caches textures by a caller-provided key so we don't
//! re-upload the same sprite every frame.

use std::collections::HashMap;

use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use tracing::{debug, trace};

use crate::palette::{apply_palette, Palette256};
use ow_data::sprite::SpriteFrame;

/// Manages SDL2 textures for decoded sprites.
///
/// Textures are created on demand and cached by a `u32` key (typically the
/// sprite index within the sheet). The cache is tied to the lifetime of the
/// SDL2 `TextureCreator`.
pub struct SpriteRenderer<'tc> {
    texture_creator: &'tc TextureCreator<WindowContext>,
    cache: HashMap<u32, Texture<'tc>>,
}

impl<'tc> SpriteRenderer<'tc> {
    /// Create a new renderer bound to the given texture creator.
    pub fn new(texture_creator: &'tc TextureCreator<WindowContext>) -> Self {
        Self {
            texture_creator,
            cache: HashMap::new(),
        }
    }

    /// Create an SDL2 texture from a decoded `SpriteFrame` and a palette.
    ///
    /// This performs the full pipeline:
    ///   1. RLE-decode the sprite's compressed data into indexed pixels.
    ///   2. Apply the palette to get an RGBA buffer.
    ///   3. Upload the RGBA buffer into a new SDL2 texture.
    ///   4. Set the texture blend mode to `Blend` so index-0 pixels are transparent.
    ///
    /// The texture is cached under `key` for future `draw()` calls.
    /// If a texture with this key already exists, it is replaced.
    pub fn create_texture(
        &mut self,
        key: u32,
        frame: &SpriteFrame,
        palette: &Palette256,
    ) -> Result<(), String> {
        let w = frame.header.width as u32;
        let h = frame.header.height as u32;

        if w == 0 || h == 0 {
            debug!(key, w, h, "skipping zero-dimension sprite");
            return Ok(());
        }

        // Step 1: RLE decode compressed pixel data → palette indices.
        let pixels = ow_data::sprite::decode_rle(
            &frame.compressed_data,
            frame.header.width,
            frame.header.height,
            key as usize,
        )
        .map_err(|e| format!("RLE decode failed for sprite {key}: {e}"))?;

        // Step 2: Palette lookup — indexed pixels → RGBA buffer.
        let rgba = apply_palette(&pixels, palette);

        // Step 3: Create an SDL2 texture and upload the RGBA pixel data.
        //
        // We use RGBA32 which is the platform-endian RGBA format — it matches
        // our byte layout of [R, G, B, A] regardless of endianness.
        let mut texture = self
            .texture_creator
            .create_texture_static(PixelFormatEnum::RGBA32, w, h)
            .map_err(|e| format!("failed to create texture for sprite {key}: {e}"))?;

        // Upload pixel data. Pitch = width * 4 bytes per pixel.
        texture
            .update(None, &rgba, (w * 4) as usize)
            .map_err(|e| format!("failed to upload pixels for sprite {key}: {e}"))?;

        // Step 4: Enable alpha blending so transparent pixels (A=0) are not drawn.
        texture.set_blend_mode(sdl2::render::BlendMode::Blend);

        trace!(key, w, h, rgba_bytes = rgba.len(), "texture created and cached");

        self.cache.insert(key, texture);
        Ok(())
    }

    /// Draw a cached sprite texture at the given screen position.
    ///
    /// `(x, y)` is the top-left corner of the destination rectangle.
    /// The texture is drawn at its native (1:1) size.
    ///
    /// Returns `Err` if the key is not in the cache.
    pub fn draw(
        &self,
        canvas: &mut Canvas<Window>,
        key: u32,
        x: i32,
        y: i32,
    ) -> Result<(), String> {
        let texture = self
            .cache
            .get(&key)
            .ok_or_else(|| format!("no cached texture for key {key}"))?;

        let query = texture.query();
        let dst = Rect::new(x, y, query.width, query.height);

        canvas.copy(texture, None, dst)?;

        trace!(key, x, y, w = query.width, h = query.height, "drew sprite");
        Ok(())
    }

    /// Draw a cached sprite texture scaled to fit a destination rectangle.
    pub fn draw_scaled(
        &self,
        canvas: &mut Canvas<Window>,
        key: u32,
        dst: Rect,
    ) -> Result<(), String> {
        let texture = self
            .cache
            .get(&key)
            .ok_or_else(|| format!("no cached texture for key {key}"))?;

        canvas.copy(texture, None, dst)?;
        Ok(())
    }

    /// Remove a texture from the cache, freeing its GPU memory.
    pub fn evict(&mut self, key: u32) {
        self.cache.remove(&key);
    }

    /// Remove all cached textures.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Returns `true` if a texture with the given key is cached.
    pub fn has(&self, key: u32) -> bool {
        self.cache.contains_key(&key)
    }
}
