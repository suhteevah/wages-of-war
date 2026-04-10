//! # PCX Palette Extraction
//!
//! Extracts the 256-color VGA palette from ZSoft PCX v5 image files.
//! The game's master palette is embedded in the last 769 bytes of each
//! PCX file: a `0x0C` marker byte followed by 256 RGB triplets (768 bytes).
//!
//! ## Usage
//!
//! ```no_run
//! # use std::path::Path;
//! # use ow_data::palette::extract_palette_from_pcx;
//! let palette = extract_palette_from_pcx(Path::new("data/WOW/PIC/MAINPIC.PCX")).unwrap();
//! let (r, g, b) = palette.get_color(42);
//! ```

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use tracing::{debug, trace};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur when extracting a PCX palette.
#[derive(Debug, thiserror::Error)]
pub enum PaletteError {
    #[error("I/O error reading PCX file: {0}")]
    Io(#[from] io::Error),

    #[error("file too small for PCX palette: need at least 769 bytes, got {0}")]
    FileTooSmall(u64),

    #[error("missing PCX palette marker: expected 0x0C at offset {offset}, got 0x{actual:02X}")]
    BadMarker { offset: u64, actual: u8 },
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A 256-color VGA palette (768 bytes of RGB triplets).
///
/// Index 0 is conventionally transparent in sprite rendering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    /// 256 RGB triplets, each `[R, G, B]` in 0..255 range.
    /// Always exactly 256 entries.
    pub colors: Vec<[u8; 3]>,
}

impl Palette {
    /// Look up a color by palette index.
    ///
    /// Returns `(R, G, B)` for the given 8-bit index.
    pub fn get_color(&self, index: u8) -> (u8, u8, u8) {
        let c = self.colors[index as usize];
        (c[0], c[1], c[2])
    }

    /// Returns `true` if index 0 is black (typical transparent sentinel).
    pub fn index_zero_is_black(&self) -> bool {
        self.colors[0] == [0, 0, 0]
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Size of the PCX palette block: 1 marker byte + 256 * 3 RGB bytes.
const PCX_PALETTE_SIZE: u64 = 769;

/// Expected marker byte preceding the palette data.
const PCX_PALETTE_MARKER: u8 = 0x0C;

/// Extract the 256-color palette from a PCX file.
///
/// Reads only the last 769 bytes of the file — does not parse the full PCX
/// image. Works with any standard ZSoft PCX v5, 8bpp file.
pub fn extract_palette_from_pcx(path: &Path) -> Result<Palette, PaletteError> {
    debug!(path = %path.display(), "extracting PCX palette");

    let mut file = std::fs::File::open(path)?;
    let file_size = file.metadata()?.len();

    if file_size < PCX_PALETTE_SIZE {
        return Err(PaletteError::FileTooSmall(file_size));
    }

    let palette_offset = file_size - PCX_PALETTE_SIZE;
    file.seek(SeekFrom::Start(palette_offset))?;

    let mut buf = [0u8; PCX_PALETTE_SIZE as usize];
    file.read_exact(&mut buf)?;

    // Verify the 0x0C marker
    if buf[0] != PCX_PALETTE_MARKER {
        return Err(PaletteError::BadMarker {
            offset: palette_offset,
            actual: buf[0],
        });
    }

    let mut colors = Vec::with_capacity(256);
    for i in 0..256 {
        let base = 1 + i * 3;
        colors.push([buf[base], buf[base + 1], buf[base + 2]]);
    }

    trace!(
        index_0 = ?colors[0],
        index_1 = ?colors[1],
        index_255 = ?colors[255],
        "palette extracted: 256 colors"
    );

    debug!(
        path = %path.display(),
        "palette extracted successfully (index 0 = [{}, {}, {}])",
        colors[0][0], colors[0][1], colors[0][2]
    );

    Ok(Palette { colors })
}

/// Extract palette from raw bytes (e.g. an in-memory PCX buffer).
///
/// The buffer must be at least 769 bytes. The palette is read from the
/// last 769 bytes, identical to the file-based extractor.
pub fn extract_palette_from_bytes(data: &[u8]) -> Result<Palette, PaletteError> {
    if (data.len() as u64) < PCX_PALETTE_SIZE {
        return Err(PaletteError::FileTooSmall(data.len() as u64));
    }

    let start = data.len() - PCX_PALETTE_SIZE as usize;

    if data[start] != PCX_PALETTE_MARKER {
        return Err(PaletteError::BadMarker {
            offset: start as u64,
            actual: data[start],
        });
    }

    let mut colors = Vec::with_capacity(256);
    for i in 0..256 {
        let base = start + 1 + i * 3;
        colors.push([data[base], data[base + 1], data[base + 2]]);
    }

    Ok(Palette { colors })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid PCX buffer with a known palette.
    fn make_test_pcx(palette: &[[u8; 3]]) -> Vec<u8> {
        // Minimal PCX header (128 bytes) + palette block (769 bytes)
        let mut buf = vec![0u8; 128 + 769];
        // PCX magic + version
        buf[0] = 0x0A; // PCX magic
        buf[1] = 0x05; // version 5

        // Write palette at end
        let pal_start = buf.len() - 769;
        buf[pal_start] = PCX_PALETTE_MARKER;
        for i in 0..256 {
            let base = pal_start + 1 + i * 3;
            buf[base] = palette[i][0];
            buf[base + 1] = palette[i][1];
            buf[base + 2] = palette[i][2];
        }
        buf
    }

    #[test]
    fn extract_from_bytes_roundtrip() {
        let mut palette = [[0u8; 3]; 256];
        palette[0] = [0, 0, 0]; // transparent black
        palette[1] = [255, 0, 0]; // red
        palette[42] = [10, 20, 30];
        palette[255] = [255, 255, 255]; // white

        let pcx = make_test_pcx(&palette);
        let result = extract_palette_from_bytes(&pcx).unwrap();

        assert_eq!(result.get_color(0), (0, 0, 0));
        assert_eq!(result.get_color(1), (255, 0, 0));
        assert_eq!(result.get_color(42), (10, 20, 30));
        assert_eq!(result.get_color(255), (255, 255, 255));
    }

    #[test]
    fn index_zero_is_black_check() {
        let mut palette = [[128u8; 3]; 256];
        palette[0] = [0, 0, 0];
        let pcx = make_test_pcx(&palette);
        let result = extract_palette_from_bytes(&pcx).unwrap();
        assert!(result.index_zero_is_black());

        palette[0] = [1, 0, 0];
        let pcx = make_test_pcx(&palette);
        let result = extract_palette_from_bytes(&pcx).unwrap();
        assert!(!result.index_zero_is_black());
    }

    #[test]
    fn file_too_small() {
        let data = vec![0u8; 100];
        let err = extract_palette_from_bytes(&data).unwrap_err();
        assert!(matches!(err, PaletteError::FileTooSmall(100)));
    }

    #[test]
    fn bad_marker() {
        let mut data = vec![0u8; 128 + 769];
        // Write wrong marker
        let pal_start = data.len() - 769;
        data[pal_start] = 0xFF;
        let err = extract_palette_from_bytes(&data).unwrap_err();
        assert!(matches!(err, PaletteError::BadMarker { actual: 0xFF, .. }));
    }

    #[test]
    fn extract_from_file() {
        let mut palette = [[0u8; 3]; 256];
        for i in 0..256 {
            palette[i] = [i as u8, (255 - i) as u8, (i / 2) as u8];
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pcx");
        let pcx = make_test_pcx(&palette);
        std::fs::write(&path, &pcx).unwrap();

        let result = extract_palette_from_pcx(&path).unwrap();
        for i in 0..256 {
            let (r, g, b) = result.get_color(i as u8);
            assert_eq!(r, i as u8);
            assert_eq!(g, (255 - i) as u8);
            assert_eq!(b, (i / 2) as u8);
        }
    }
}
