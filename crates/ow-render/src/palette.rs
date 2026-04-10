//! # Palette — Runtime palette for SDL2 rendering
//!
//! Converts 8-bit palette-indexed pixel buffers into RGBA pixel buffers
//! suitable for SDL2 texture creation.
//!
//! Also provides a minimal PCX palette extractor so the sprite viewer can
//! pull a 256-color palette from any of the game's PCX files. This lives
//! here (rather than in ow-data) as a convenience; we'll refactor into
//! ow-data once we build a proper PCX parser.

use std::path::Path;
use tracing::{debug, trace};

/// A 256-entry RGB palette (one (R, G, B) triplet per index).
pub type Palette256 = [(u8, u8, u8); 256];

/// Convert a buffer of palette-indexed pixels to RGBA.
///
/// The conversion rule per pixel:
/// - Index 0 → fully transparent (R=0, G=0, B=0, A=0).
///   Index 0 is the transparency key in the Wages of War sprite format.
/// - Index 1..=255 → look up (R, G, B) in the palette, set A=255 (fully opaque).
///
/// The output buffer is exactly `pixels.len() * 4` bytes, laid out as
/// consecutive [R, G, B, A] quads — matching `PixelFormatEnum::RGBA32`
/// (which is endian-aware RGBA on all platforms).
pub fn apply_palette(pixels: &[u8], palette: &Palette256) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len() * 4);

    for &idx in pixels {
        if idx == 0 {
            // Transparent pixel — zero all channels including alpha.
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let (r, g, b) = palette[idx as usize];
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }

    trace!(
        indexed_pixels = pixels.len(),
        rgba_bytes = rgba.len(),
        "palette applied: indexed → RGBA"
    );

    rgba
}

/// Extract a 256-color palette from a PCX file.
///
/// PCX files store an optional 256-color VGA palette in the last 769 bytes:
///   - Byte at offset (file_len - 769) must be 0x0C (palette marker).
///   - The following 768 bytes are 256 × (R, G, B) triplets.
///
/// Returns `None` if the file is too small or the marker byte is missing.
pub fn extract_pcx_palette(data: &[u8]) -> Option<Palette256> {
    if data.len() < 769 {
        debug!(
            file_len = data.len(),
            "PCX file too small for embedded palette"
        );
        return None;
    }

    let marker_offset = data.len() - 769;
    if data[marker_offset] != 0x0C {
        debug!(
            marker = data[marker_offset],
            "PCX palette marker byte is not 0x0C"
        );
        return None;
    }

    let palette_data = &data[marker_offset + 1..];
    let mut palette: Palette256 = [(0, 0, 0); 256];

    for (i, entry) in palette.iter_mut().enumerate() {
        let base = i * 3;
        *entry = (palette_data[base], palette_data[base + 1], palette_data[base + 2]);
    }

    debug!("extracted 256-color palette from PCX");
    trace!(
        first_entry = ?palette[0],
        last_entry = ?palette[255],
        "palette range"
    );

    Some(palette)
}

/// Convenience: read a PCX file from disk and extract its palette.
pub fn load_pcx_palette(path: &Path) -> anyhow::Result<Palette256> {
    debug!(path = %path.display(), "loading PCX palette");
    let data = std::fs::read(path)?;
    extract_pcx_palette(&data)
        .ok_or_else(|| anyhow::anyhow!("no valid 256-color palette found in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_palette_transparent() {
        let palette: Palette256 = {
            let mut p = [(0u8, 0u8, 0u8); 256];
            p[1] = (255, 0, 0);
            p[2] = (0, 255, 0);
            p
        };

        // Index 0 = transparent, index 1 = red, index 2 = green
        let pixels = vec![0, 1, 2];
        let rgba = apply_palette(&pixels, &palette);

        assert_eq!(rgba.len(), 12);
        // Transparent
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
        // Red, fully opaque
        assert_eq!(&rgba[4..8], &[255, 0, 0, 255]);
        // Green, fully opaque
        assert_eq!(&rgba[8..12], &[0, 255, 0, 255]);
    }

    #[test]
    fn test_extract_pcx_palette_valid() {
        // Build a minimal fake PCX file: just enough for the palette tail.
        let mut data = vec![0u8; 128]; // some PCX header junk
        data.push(0x0C); // palette marker
        for i in 0u8..=255 {
            data.push(i); // R
            data.push(0); // G
            data.push(0); // B
        }
        // Total = 128 + 1 + 768 = 897 bytes

        let palette = extract_pcx_palette(&data).expect("should extract palette");
        assert_eq!(palette[0], (0, 0, 0));
        assert_eq!(palette[128], (128, 0, 0));
        assert_eq!(palette[255], (255, 0, 0));
    }

    #[test]
    fn test_extract_pcx_palette_bad_marker() {
        let data = vec![0u8; 897];
        // Marker position would be at offset 128, but we leave it as 0x00.
        assert!(extract_pcx_palette(&data).is_none());
    }
}
