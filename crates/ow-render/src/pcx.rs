//! # PCX Image Renderer — loads and displays 256-color PCX images.
//!
//! The original game uses PCX (ZSoft v5, 640x480, 8bpp) for background
//! screens: the office, menus, cutscenes, catalogs. Each PCX file contains
//! its own 256-color palette in the trailing 769 bytes.
//!
//! This module decodes PCX run-length encoding into raw RGBA pixel data
//! and uploads it as an SDL2 texture for display.

use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use sdl2::rect::Rect;
use tracing::{debug, info};

/// A decoded PCX image ready for display.
pub struct PcxImage {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data (width * height * 4 bytes).
    pub rgba_data: Vec<u8>,
}

/// Decode a PCX file from raw bytes into an RGBA pixel buffer.
///
/// PCX v5 format (8bpp, 256-color):
/// - 128-byte header: magic 0x0A, version, encoding, bpp, dimensions
/// - RLE-compressed scanlines of palette-indexed pixels
/// - Trailing 769 bytes: 0x0C marker + 256 RGB triplets (the palette)
///
/// We decode the RLE pixel data, then look up each palette index to get RGB,
/// converting to RGBA with full opacity (index 0 could be transparent but
/// for backgrounds we render it opaque).
pub fn decode_pcx(data: &[u8]) -> Result<PcxImage, String> {
    if data.len() < 128 + 769 {
        return Err("PCX file too small for header + palette".to_string());
    }

    // -- Parse header --
    // Byte 0: manufacturer (0x0A = ZSoft)
    if data[0] != 0x0A {
        return Err(format!("Bad PCX magic: expected 0x0A, got 0x{:02X}", data[0]));
    }

    // Bytes 4-7: window coordinates (xmin, ymin) as u16 LE
    // Bytes 8-11: window coordinates (xmax, ymax) as u16 LE
    let xmin = u16::from_le_bytes([data[4], data[5]]) as u32;
    let ymin = u16::from_le_bytes([data[6], data[7]]) as u32;
    let xmax = u16::from_le_bytes([data[8], data[9]]) as u32;
    let ymax = u16::from_le_bytes([data[10], data[11]]) as u32;
    let width = xmax - xmin + 1;
    let height = ymax - ymin + 1;

    // Byte 65: number of color planes (should be 1 for 8bpp)
    let planes = data[65];

    // Bytes 66-67: bytes per scanline per plane (u16 LE)
    let bytes_per_line = u16::from_le_bytes([data[66], data[67]]) as usize;

    debug!(
        width, height, planes, bytes_per_line,
        "PCX header parsed"
    );

    // -- Extract palette from trailing 769 bytes --
    let palette_start = data.len() - 769;
    if data[palette_start] != 0x0C {
        return Err(format!(
            "Bad PCX palette marker: expected 0x0C at offset {palette_start}, got 0x{:02X}",
            data[palette_start]
        ));
    }
    let palette = &data[palette_start + 1..];

    // -- Decode RLE pixel data --
    // Pixel data starts at offset 128, ends before the palette.
    let pixel_data = &data[128..palette_start];
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    let mut src_idx = 0;
    for y in 0..height as usize {
        let mut x = 0usize;
        // Decode one scanline — bytes_per_line bytes of indexed color data.
        while x < bytes_per_line && src_idx < pixel_data.len() {
            let byte = pixel_data[src_idx];
            src_idx += 1;

            // PCX RLE: if top 2 bits are set (0xC0), the lower 6 bits are
            // a run count, and the NEXT byte is the repeated value.
            // Otherwise, the byte itself is a single pixel value.
            let (count, value) = if byte >= 0xC0 {
                let count = (byte & 0x3F) as usize;
                if src_idx >= pixel_data.len() {
                    break;
                }
                let val = pixel_data[src_idx];
                src_idx += 1;
                (count, val)
            } else {
                (1, byte)
            };

            // Write `count` pixels at this palette index.
            for _ in 0..count {
                if x < width as usize {
                    let dst = (y * width as usize + x) * 4;
                    let pal_idx = value as usize;
                    // Look up RGB from the 256-color palette.
                    // ARGB8888 on little-endian x86: memory bytes [B, G, R, A].
                    rgba[dst] = palette[pal_idx * 3 + 2]; // B
                    rgba[dst + 1] = palette[pal_idx * 3 + 1]; // G
                    rgba[dst + 2] = palette[pal_idx * 3];     // R
                    rgba[dst + 3] = 255; // A
                }
                x += 1;
            }
        }
    }

    info!(width, height, "PCX image decoded");
    Ok(PcxImage { width, height, rgba_data: rgba })
}

/// Load a PCX file from disk and decode it.
pub fn load_pcx(path: &std::path::Path) -> Result<PcxImage, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read PCX: {e}"))?;
    decode_pcx(&data)
}

/// Create an SDL2 texture from a decoded PCX image.
///
/// Uses SDL2's `create_rgb_surface_from` with explicit channel masks so
/// SDL2 handles any format conversion needed for the GPU texture.
pub fn pcx_to_texture<'a>(
    image: &PcxImage,
    texture_creator: &'a TextureCreator<WindowContext>,
) -> Result<Texture<'a>, String> {
    // Our pixel data is [R, G, B, A] per pixel in memory.
    // On little-endian x86, a u32 read of these bytes gives 0xAABBGGRR.
    // We tell SDL2 exactly where each channel is with bitmasks:
    //   R in bits 0-7   = 0x000000FF
    //   G in bits 8-15  = 0x0000FF00
    //   B in bits 16-23 = 0x00FF0000
    //   A in bits 24-31 = 0xFF000000
    let mut pixel_buf = image.rgba_data.clone();
    let surface = sdl2::surface::Surface::from_data_pixelmasks(
        &mut pixel_buf,
        image.width,
        image.height,
        image.width * 4,
        &sdl2::pixels::PixelMasks {
            bpp: 32,
            rmask: 0x00FF0000,
            gmask: 0x0000FF00,
            bmask: 0x000000FF,
            amask: 0xFF000000,
        },
    ).map_err(|e| format!("Surface creation failed: {e}"))?;

    let texture = texture_creator
        .create_texture_from_surface(&surface)
        .map_err(|e| format!("Texture from surface failed: {e}"))?;

    Ok(texture)
}

/// Draw a PCX texture scaled to fill the canvas.
pub fn draw_pcx_scaled(
    canvas: &mut Canvas<Window>,
    texture: &Texture,
    dst_width: u32,
    dst_height: u32,
) -> Result<(), String> {
    canvas.copy(texture, None, Some(Rect::new(0, 0, dst_width, dst_height)))
}

/// Draw a PCX texture at its native resolution at position (x, y).
pub fn draw_pcx_at(
    canvas: &mut Canvas<Window>,
    texture: &Texture,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    canvas.copy(texture, None, Some(Rect::new(x, y, width, height)))
}
