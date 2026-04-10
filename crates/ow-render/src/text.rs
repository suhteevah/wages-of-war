//! # Text Rendering — SDL2_ttf wrapper for drawing text on screen.
//!
//! Provides a simple text rendering API backed by SDL2_ttf. The game uses this
//! for all UI text: merc names, stat labels, contract terms, combat messages.
//!
//! We use a system font (Consolas on Windows) for now. The original game used
//! bitmap fonts baked into its sprite sheets — we may switch to those later
//! for pixel-perfect authenticity.

use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, TextureCreator, TextureQuery};
use sdl2::ttf::{Font, Sdl2TtfContext};
use sdl2::video::{Window, WindowContext};
use tracing::debug;

/// Default font size for UI text (merc names, labels, etc.)
pub const FONT_SIZE_NORMAL: u16 = 14;

/// Larger font size for headers and titles.
pub const FONT_SIZE_HEADER: u16 = 20;

/// Small font size for detailed stats and secondary info.
pub const FONT_SIZE_SMALL: u16 = 11;

/// Text renderer that manages font loading and rendering.
///
/// Holds the TTF context and loaded fonts at different sizes.
/// Create one of these at startup and pass it around to anything
/// that needs to draw text.
pub struct TextRenderer<'ttf> {
    pub font_normal: Font<'ttf, 'static>,
    pub font_header: Font<'ttf, 'static>,
    pub font_small: Font<'ttf, 'static>,
}

impl<'ttf> TextRenderer<'ttf> {
    /// Initialize the text renderer by loading a TTF font at multiple sizes.
    ///
    /// Tries these font paths in order:
    /// 1. The provided `font_path` if Some
    /// 2. Windows Consolas (clean monospace, good for stats)
    /// 3. Windows Arial (fallback)
    pub fn new(
        ttf_context: &'ttf Sdl2TtfContext,
        font_path: Option<&str>,
    ) -> Result<Self, String> {
        // Try font paths in preference order.
        let paths_to_try = if let Some(p) = font_path {
            vec![p.to_string()]
        } else {
            vec![
                "C:\\Windows\\Fonts\\consola.ttf".to_string(),
                "C:\\Windows\\Fonts\\arial.ttf".to_string(),
                // Linux/macOS fallbacks for future cross-platform support
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".to_string(),
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf".to_string(),
            ]
        };

        let mut last_err = String::from("no fonts found");
        for path in &paths_to_try {
            match Self::try_load(ttf_context, path) {
                Ok(renderer) => {
                    debug!(font = path, "Font loaded successfully");
                    return Ok(renderer);
                }
                Err(e) => {
                    debug!(font = path, error = %e, "Font not available, trying next");
                    last_err = e;
                }
            }
        }

        Err(format!("Could not load any font: {last_err}"))
    }

    fn try_load(ttf_context: &'ttf Sdl2TtfContext, path: &str) -> Result<Self, String> {
        let font_normal = ttf_context.load_font(path, FONT_SIZE_NORMAL)?;
        let font_header = ttf_context.load_font(path, FONT_SIZE_HEADER)?;
        let font_small = ttf_context.load_font(path, FONT_SIZE_SMALL)?;
        Ok(Self {
            font_normal,
            font_header,
            font_small,
        })
    }

    /// Draw text at the given position with the normal-sized font.
    /// Returns the width and height of the rendered text.
    pub fn draw(
        &self,
        canvas: &mut Canvas<Window>,
        texture_creator: &TextureCreator<WindowContext>,
        text: &str,
        x: i32,
        y: i32,
        color: Color,
    ) -> Result<(u32, u32), String> {
        self.draw_with_font(canvas, texture_creator, &self.font_normal, text, x, y, color)
    }

    /// Draw text with the header font (larger).
    pub fn draw_header(
        &self,
        canvas: &mut Canvas<Window>,
        texture_creator: &TextureCreator<WindowContext>,
        text: &str,
        x: i32,
        y: i32,
        color: Color,
    ) -> Result<(u32, u32), String> {
        self.draw_with_font(canvas, texture_creator, &self.font_header, text, x, y, color)
    }

    /// Draw text with the small font.
    pub fn draw_small(
        &self,
        canvas: &mut Canvas<Window>,
        texture_creator: &TextureCreator<WindowContext>,
        text: &str,
        x: i32,
        y: i32,
        color: Color,
    ) -> Result<(u32, u32), String> {
        self.draw_with_font(canvas, texture_creator, &self.font_small, text, x, y, color)
    }

    /// Core text rendering: render text to a surface, upload as texture, draw.
    fn draw_with_font(
        &self,
        canvas: &mut Canvas<Window>,
        texture_creator: &TextureCreator<WindowContext>,
        font: &Font,
        text: &str,
        x: i32,
        y: i32,
        color: Color,
    ) -> Result<(u32, u32), String> {
        // Empty strings cause SDL2_ttf to error — just skip them.
        if text.is_empty() {
            return Ok((0, 0));
        }

        // Render text to an SDL surface, then upload as a GPU texture.
        // blended() gives us anti-aliased text with alpha blending.
        let surface = font
            .render(text)
            .blended(color)
            .map_err(|e| format!("Font render error: {e}"))?;

        let texture = texture_creator
            .create_texture_from_surface(&surface)
            .map_err(|e| format!("Texture creation error: {e}"))?;

        let TextureQuery { width, height, .. } = texture.query();

        canvas
            .copy(&texture, None, Some(Rect::new(x, y, width, height)))
            .map_err(|e| format!("Canvas copy error: {e}"))?;

        Ok((width, height))
    }

    /// Measure text dimensions without drawing it.
    pub fn measure(&self, text: &str) -> Result<(u32, u32), String> {
        self.measure_with_font(&self.font_normal, text)
    }

    /// Measure text with the header font.
    pub fn measure_header(&self, text: &str) -> Result<(u32, u32), String> {
        self.measure_with_font(&self.font_header, text)
    }

    fn measure_with_font(&self, font: &Font, text: &str) -> Result<(u32, u32), String> {
        if text.is_empty() {
            return Ok((0, 0));
        }
        let (w, h) = font.size_of(text).map_err(|e| format!("Measure error: {e}"))?;
        Ok((w, h))
    }
}
