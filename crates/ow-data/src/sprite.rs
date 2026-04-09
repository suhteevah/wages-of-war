//! # Sprite Container Parser (.OBJ, .SPR, .TIL, ANIM .DAT)
//!
//! Parses the shared binary sprite container format used across 120+ game files.
//! All sprite containers share an identical header/offset-table/pixel-data layout,
//! with RLE-compressed 8-bit palette-indexed pixel data.
//!
//! ## Supported file types
//!
//! | Extension | Usage |
//! |-----------|-------|
//! | `.OBJ`    | UI sprite sheets, per-scene map objects |
//! | `.SPR`    | Sprite sheets (cursors, inventory, characters) |
//! | `.TIL`    | Isometric terrain tiles (512 per scenario) |
//! | `ANIM/*.DAT` | Character animation frames (walk/shoot/die) |

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{self, Cursor, Read};
use std::path::Path;
use tracing::{debug, trace, warn};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur when parsing sprite container files.
#[derive(Debug, thiserror::Error)]
pub enum SpriteError {
    #[error("I/O error reading sprite file: {0}")]
    Io(#[from] io::Error),

    #[error("invalid header: expected header_size=0x20, got 0x{0:X}")]
    BadHeaderSize(u32),

    #[error("offset table size mismatch: expected {expected}, got {actual}")]
    OffsetTableMismatch { expected: u32, actual: u32 },

    #[error("sprite {index} data extends past end of file (offset {offset} + size {size} > pixel region {pixel_region_size})")]
    SpriteOutOfBounds {
        index: usize,
        offset: u32,
        size: u32,
        pixel_region_size: u32,
    },

    #[error("RLE decode error in sprite {index}, row {row}: unexpected end of data at byte {pos}")]
    RleTruncated { index: usize, row: u32, pos: usize },
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Top-level file header (32 bytes, at offset 0x00).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpriteFileHeader {
    /// Number of sprites/frames in this container.
    pub sprite_count: u32,
    /// Always 0x20 (32). Size of this header in bytes.
    pub header_size: u32,
    /// Size of the offset table in bytes (`sprite_count * 8`).
    pub offset_table_size: u32,
    /// Absolute file offset where pixel data begins (`header_size + offset_table_size`).
    pub pixel_data_start: u32,
    /// Total size of the pixel data region in bytes.
    pub pixel_data_size: u32,
    /// Reserved bytes 0x14..0x1F. Non-zero in ANIM .DAT files.
    pub reserved: [u8; 12],
}

/// Per-sprite entry from the offset table (8 bytes each).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpriteIndex {
    /// Byte offset of this sprite's data, relative to the pixel data region start.
    pub offset: u32,
    /// Total byte size of this sprite's data (24-byte header + compressed pixels).
    pub size: u32,
}

/// Per-sprite header (24 bytes, at the start of each sprite's data).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpriteFrameHeader {
    /// X origin / hotspot / positional offset.
    pub origin_x: u16,
    /// Y origin / hotspot / positional offset.
    pub origin_y: u16,
    /// Width of the sprite in pixels.
    pub width: u16,
    /// Height of the sprite in pixels.
    pub height: u16,
    /// Flags field A (purpose varies: 0xFFFE in some UI sprites, animation flags in ANIM).
    pub flags_a: u16,
    /// Flags field B (purpose varies: non-zero in some ANIM sprites).
    pub flags_b: u16,
    /// Size of the RLE-compressed pixel data in bytes.
    pub compressed_size: u32,
    /// Unknown field (often 0; contains leaked pointer values in some ANIM files).
    pub unknown_a: u32,
    /// Unknown field (always 0 in observed files).
    pub unknown_b: u32,
}

/// A single decoded sprite frame.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpriteFrame {
    /// Per-sprite header with dimensions and metadata.
    pub header: SpriteFrameHeader,
    /// Raw RLE-compressed pixel data (not yet decoded to pixels).
    pub compressed_data: Vec<u8>,
}

/// A complete parsed sprite container file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpriteSheet {
    /// File-level header.
    pub file_header: SpriteFileHeader,
    /// Index table entries (one per sprite).
    pub index: Vec<SpriteIndex>,
    /// Decoded sprite frames (one per sprite).
    pub frames: Vec<SpriteFrame>,
}

// ---------------------------------------------------------------------------
// RLE decoder
// ---------------------------------------------------------------------------

/// Decode RLE-compressed pixel data into a flat row-major pixel buffer.
///
/// The RLE scheme uses three command types:
/// - `0x00`: End of scanline. Remaining pixels in the row are transparent (index 0).
/// - `0x80 NN`: Skip — emit `NN` transparent pixels.
/// - `0x81..0xFF`: Literal run — emit `(byte - 0x80)` literal pixel bytes that follow.
/// - `0x01..0x7F`: RLE run — repeat the next byte `N` times.
///
/// Returns a `width * height` pixel buffer of palette indices.
pub fn decode_rle(
    compressed: &[u8],
    width: u16,
    height: u16,
    sprite_index: usize,
) -> Result<Vec<u8>, SpriteError> {
    let w = width as usize;
    let h = height as usize;
    let mut pixels = vec![0u8; w * h];
    let mut pos = 0usize;

    for row in 0..h {
        let row_start = row * w;
        let mut col = 0usize;

        while pos < compressed.len() {
            let cmd = compressed[pos];

            if cmd == 0x00 {
                // End of scanline — remaining pixels stay transparent.
                pos += 1;
                break;
            } else if cmd == 0x80 {
                // Transparent skip.
                if pos + 1 >= compressed.len() {
                    return Err(SpriteError::RleTruncated {
                        index: sprite_index,
                        row: row as u32,
                        pos,
                    });
                }
                let count = compressed[pos + 1] as usize;
                col += count;
                pos += 2;
            } else if cmd > 0x80 {
                // Literal copy.
                let count = (cmd - 0x80) as usize;
                if pos + 1 + count > compressed.len() {
                    return Err(SpriteError::RleTruncated {
                        index: sprite_index,
                        row: row as u32,
                        pos,
                    });
                }
                for i in 0..count {
                    if col + i < w {
                        pixels[row_start + col + i] = compressed[pos + 1 + i];
                    }
                }
                col += count;
                pos += 1 + count;
            } else {
                // RLE run: repeat next byte `cmd` times.
                let count = cmd as usize;
                if pos + 1 >= compressed.len() {
                    return Err(SpriteError::RleTruncated {
                        index: sprite_index,
                        row: row as u32,
                        pos,
                    });
                }
                let value = compressed[pos + 1];
                for i in 0..count {
                    if col + i < w {
                        pixels[row_start + col + i] = value;
                    }
                }
                col += count;
                pos += 2;
            }
        }

        trace!(
            sprite = sprite_index,
            row,
            pixels_in_row = col,
            width = w,
            "decoded scanline"
        );
    }

    Ok(pixels)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a sprite container file from raw bytes.
///
/// Reads the file header, offset table, and per-sprite headers + compressed
/// pixel data. Does NOT decode the RLE pixel data — call [`decode_rle`] on
/// individual frames when you need pixel buffers.
pub fn parse_sprite_sheet(data: &[u8]) -> Result<SpriteSheet, SpriteError> {
    let mut cursor = Cursor::new(data);

    // -- File header (32 bytes) --
    let sprite_count = cursor.read_u32::<LittleEndian>()?;
    let header_size = cursor.read_u32::<LittleEndian>()?;
    let offset_table_size = cursor.read_u32::<LittleEndian>()?;
    let pixel_data_start = cursor.read_u32::<LittleEndian>()?;
    let pixel_data_size = cursor.read_u32::<LittleEndian>()?;
    let mut reserved = [0u8; 12];
    cursor.read_exact(&mut reserved)?;

    debug!(
        sprite_count,
        header_size,
        offset_table_size,
        pixel_data_start,
        pixel_data_size,
        "parsed sprite file header"
    );

    if header_size != 0x20 {
        return Err(SpriteError::BadHeaderSize(header_size));
    }

    let expected_ot_size = sprite_count * 8;
    if offset_table_size != expected_ot_size {
        return Err(SpriteError::OffsetTableMismatch {
            expected: expected_ot_size,
            actual: offset_table_size,
        });
    }

    let file_header = SpriteFileHeader {
        sprite_count,
        header_size,
        offset_table_size,
        pixel_data_start,
        pixel_data_size,
        reserved,
    };

    // -- Offset table --
    let mut index = Vec::with_capacity(sprite_count as usize);
    for _ in 0..sprite_count {
        let offset = cursor.read_u32::<LittleEndian>()?;
        let size = cursor.read_u32::<LittleEndian>()?;
        index.push(SpriteIndex { offset, size });
    }

    // -- Per-sprite data --
    let mut frames = Vec::with_capacity(sprite_count as usize);
    for (i, idx) in index.iter().enumerate() {
        let abs_offset = pixel_data_start as usize + idx.offset as usize;

        if abs_offset + 24 > data.len() {
            return Err(SpriteError::SpriteOutOfBounds {
                index: i,
                offset: idx.offset,
                size: idx.size,
                pixel_region_size: pixel_data_size,
            });
        }

        let mut hdr_cursor = Cursor::new(&data[abs_offset..]);
        let origin_x = hdr_cursor.read_u16::<LittleEndian>()?;
        let origin_y = hdr_cursor.read_u16::<LittleEndian>()?;
        let width = hdr_cursor.read_u16::<LittleEndian>()?;
        let height = hdr_cursor.read_u16::<LittleEndian>()?;
        let flags_a = hdr_cursor.read_u16::<LittleEndian>()?;
        let flags_b = hdr_cursor.read_u16::<LittleEndian>()?;
        let compressed_size = hdr_cursor.read_u32::<LittleEndian>()?;
        let unknown_a = hdr_cursor.read_u32::<LittleEndian>()?;
        let unknown_b = hdr_cursor.read_u32::<LittleEndian>()?;

        let frame_header = SpriteFrameHeader {
            origin_x,
            origin_y,
            width,
            height,
            flags_a,
            flags_b,
            compressed_size,
            unknown_a,
            unknown_b,
        };

        // Verify size consistency: 24 (header) + compressed_size == entry size.
        let expected_entry_size = 24 + compressed_size;
        if expected_entry_size != idx.size {
            warn!(
                sprite = i,
                expected_entry_size,
                actual_entry_size = idx.size,
                "per-sprite entry size mismatch"
            );
        }

        let data_start = abs_offset + 24;
        let data_end = data_start + compressed_size as usize;
        if data_end > data.len() {
            return Err(SpriteError::SpriteOutOfBounds {
                index: i,
                offset: idx.offset,
                size: idx.size,
                pixel_region_size: pixel_data_size,
            });
        }

        let compressed_data = data[data_start..data_end].to_vec();

        trace!(
            sprite = i,
            origin_x,
            origin_y,
            width,
            height,
            compressed_size,
            "parsed sprite frame header"
        );

        frames.push(SpriteFrame {
            header: frame_header,
            compressed_data,
        });
    }

    debug!(
        frames_parsed = frames.len(),
        "sprite sheet parsing complete"
    );

    Ok(SpriteSheet {
        file_header,
        index,
        frames,
    })
}

/// Convenience wrapper: read a file from disk and parse it as a sprite sheet.
pub fn parse_sprite_file(path: &Path) -> Result<SpriteSheet, SpriteError> {
    debug!(path = %path.display(), "loading sprite file");
    let data = std::fs::read(path)?;
    parse_sprite_sheet(&data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid sprite container with one 4x2 sprite.
    ///
    /// The sprite is a simple 4x2 image:
    ///   Row 0: [0x10, 0x20, 0x30, 0x40]  — 4 literal pixels
    ///   Row 1: [0x05, 0x05, 0x05, 0x05]  — run of 4 with value 0x05
    fn make_test_container() -> Vec<u8> {
        // RLE for the sprite:
        //   Row 0: 0x84 0x10 0x20 0x30 0x40 0x00   (literal-4, end-of-row)
        //   Row 1: 0x04 0x05 0x00                   (run-4 of 0x05, end-of-row)
        let rle: Vec<u8> = vec![
            0x84, 0x10, 0x20, 0x30, 0x40, 0x00, // row 0
            0x04, 0x05, 0x00, // row 1
        ];
        let compressed_size = rle.len() as u32; // 9

        // Per-sprite header (24 bytes)
        let mut sprite_hdr = Vec::new();
        sprite_hdr.extend_from_slice(&1u16.to_le_bytes()); // origin_x
        sprite_hdr.extend_from_slice(&1u16.to_le_bytes()); // origin_y
        sprite_hdr.extend_from_slice(&4u16.to_le_bytes()); // width
        sprite_hdr.extend_from_slice(&2u16.to_le_bytes()); // height
        sprite_hdr.extend_from_slice(&0u16.to_le_bytes()); // flags_a
        sprite_hdr.extend_from_slice(&0u16.to_le_bytes()); // flags_b
        sprite_hdr.extend_from_slice(&compressed_size.to_le_bytes()); // compressed_size
        sprite_hdr.extend_from_slice(&0u32.to_le_bytes()); // unknown_a
        sprite_hdr.extend_from_slice(&0u32.to_le_bytes()); // unknown_b

        let sprite_data_size = (sprite_hdr.len() + rle.len()) as u32; // 33

        // File header (32 bytes)
        let sprite_count: u32 = 1;
        let header_size: u32 = 0x20;
        let offset_table_size: u32 = sprite_count * 8; // 8
        let pixel_data_start: u32 = header_size + offset_table_size; // 40
        let pixel_data_size: u32 = sprite_data_size; // 33

        let mut buf = Vec::new();
        buf.extend_from_slice(&sprite_count.to_le_bytes());
        buf.extend_from_slice(&header_size.to_le_bytes());
        buf.extend_from_slice(&offset_table_size.to_le_bytes());
        buf.extend_from_slice(&pixel_data_start.to_le_bytes());
        buf.extend_from_slice(&pixel_data_size.to_le_bytes());
        buf.extend_from_slice(&[0u8; 12]); // reserved

        // Offset table: 1 entry
        buf.extend_from_slice(&0u32.to_le_bytes()); // offset (relative to pixel region)
        buf.extend_from_slice(&sprite_data_size.to_le_bytes()); // size

        // Pixel data region
        buf.extend_from_slice(&sprite_hdr);
        buf.extend_from_slice(&rle);

        buf
    }

    #[test]
    fn test_parse_synthetic_sprite() {
        let data = make_test_container();
        let sheet = parse_sprite_sheet(&data).expect("parse should succeed");

        assert_eq!(sheet.file_header.sprite_count, 1);
        assert_eq!(sheet.file_header.header_size, 0x20);
        assert_eq!(sheet.frames.len(), 1);

        let frame = &sheet.frames[0];
        assert_eq!(frame.header.width, 4);
        assert_eq!(frame.header.height, 2);
        assert_eq!(frame.header.origin_x, 1);
        assert_eq!(frame.header.origin_y, 1);
        assert_eq!(frame.header.compressed_size, 9);
        assert_eq!(frame.compressed_data.len(), 9);
    }

    #[test]
    fn test_decode_rle_literal_and_run() {
        // Row 0: literal-4 [0x10, 0x20, 0x30, 0x40], end
        // Row 1: run-4 of 0x05, end
        let compressed: Vec<u8> = vec![
            0x84, 0x10, 0x20, 0x30, 0x40, 0x00, // row 0
            0x04, 0x05, 0x00, // row 1
        ];

        let pixels = decode_rle(&compressed, 4, 2, 0).unwrap();
        assert_eq!(pixels.len(), 8);
        assert_eq!(&pixels[0..4], &[0x10, 0x20, 0x30, 0x40]);
        assert_eq!(&pixels[4..8], &[0x05, 0x05, 0x05, 0x05]);
    }

    #[test]
    fn test_decode_rle_transparent_skip() {
        // 4x1 sprite: skip 2, literal 2 [0xAA, 0xBB], end
        let compressed: Vec<u8> = vec![0x80, 0x02, 0x82, 0xAA, 0xBB, 0x00];

        let pixels = decode_rle(&compressed, 4, 1, 0).unwrap();
        assert_eq!(pixels, vec![0x00, 0x00, 0xAA, 0xBB]);
    }

    #[test]
    fn test_decode_rle_early_eol() {
        // 4x1 sprite: literal 1 [0x42], end-of-line — remaining 3 pixels should be 0.
        let compressed: Vec<u8> = vec![0x81, 0x42, 0x00];

        let pixels = decode_rle(&compressed, 4, 1, 0).unwrap();
        assert_eq!(pixels, vec![0x42, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_decode_rle_full_row_run() {
        // 32x1 sprite: run of 32 with value 0x00, end
        // This matches the pattern seen in blank cursor sprites.
        let compressed: Vec<u8> = vec![0x20, 0x00, 0x00];

        let pixels = decode_rle(&compressed, 32, 1, 0).unwrap();
        assert_eq!(pixels.len(), 32);
        assert!(pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn test_roundtrip_synthetic_container() {
        let data = make_test_container();
        let sheet = parse_sprite_sheet(&data).unwrap();
        let frame = &sheet.frames[0];

        let pixels = decode_rle(
            &frame.compressed_data,
            frame.header.width,
            frame.header.height,
            0,
        )
        .unwrap();

        assert_eq!(pixels.len(), 8);
        assert_eq!(&pixels[0..4], &[0x10, 0x20, 0x30, 0x40]);
        assert_eq!(&pixels[4..8], &[0x05, 0x05, 0x05, 0x05]);
    }

    #[test]
    fn test_bad_header_size() {
        let mut data = make_test_container();
        // Corrupt header_size field (offset 4..8)
        data[4] = 0x10;
        data[5] = 0x00;
        data[6] = 0x00;
        data[7] = 0x00;

        let err = parse_sprite_sheet(&data).unwrap_err();
        assert!(matches!(err, SpriteError::BadHeaderSize(0x10)));
    }
}
