//! # MAP File Parser
//!
//! Parses the binary `.MAP` files used by Wages of War for scenario tile grids.
//! All MAP files are exactly 248,384 bytes with a fixed layout:
//!
//! | Region | Offset | Size | Content |
//! |--------|--------|------|---------|
//! | Tile grid | `0x00000` | 201,600 B | 200 x 252 cells, 4 bytes each |
//! | String table | `0x31380` | 656 B | 4 null-padded path strings (164 B each) |
//! | Metadata | `0x31610` | 46,128 B | Map properties + elevation/terrain layer |
//!
//! ## Tile grid encoding
//!
//! Each cell is 4 bytes interpreted as two little-endian `u16` values:
//! - **Bytes 0-1 (`cell_flags`):** Object/placement data. `0xFF00` = unused border cell.
//! - **Bytes 2-3 (`tile_index`):** Tile sprite index into the `.TIL` tileset.
//!   Bit 15 may be a flag; the lower 15 bits are the actual tile ID.
//!
//! The grid is 200 columns x 252 rows. Rows 0..201 contain active map data;
//! rows 202..251 are border padding filled with `0xFF` in byte 0.
//!
//! ## String table
//!
//! Four 164-byte null-padded fields referencing original build paths:
//! 1. Tile sprite sheet (`.TIL`)
//! 2. Tile metadata (`TILES*.DAT`)
//! 3. Object sprite sheet (`.OBJ`)
//! 4. Object metadata (`OBJ*.DAT`)

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{self, Cursor};
use std::path::Path;
use tracing::{debug, trace};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Total file size of every MAP file.
const MAP_FILE_SIZE: usize = 248_384;

/// Grid width in cells.
const GRID_WIDTH: usize = 200;

/// Grid height in cells (including border padding rows).
const GRID_HEIGHT: usize = 252;

/// Active (non-border) rows in the grid.
const GRID_ACTIVE_ROWS: usize = 202;

/// Bytes per tile cell.
const CELL_SIZE: usize = 4;

/// Total tile grid region size in bytes.
const TILE_GRID_SIZE: usize = GRID_WIDTH * GRID_HEIGHT * CELL_SIZE;

/// File offset where the string table begins.
const STRING_TABLE_OFFSET: usize = 0x31380;

/// Fixed width of each string table entry (null-padded).
const STRING_ENTRY_SIZE: usize = 164;

/// Number of string table entries.
const STRING_ENTRY_COUNT: usize = 4;

/// File offset where the metadata footer begins.
const METADATA_OFFSET: usize = STRING_TABLE_OFFSET + STRING_ENTRY_COUNT * STRING_ENTRY_SIZE;

/// Size of the metadata section.
const METADATA_SIZE: usize = MAP_FILE_SIZE - METADATA_OFFSET;

/// Cell flag byte 0 value indicating an unused/border cell.
const BORDER_MARKER: u8 = 0xFF;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur when parsing MAP files.
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("I/O error reading MAP file: {0}")]
    Io(#[from] io::Error),

    #[error("invalid MAP file size: expected {MAP_FILE_SIZE} bytes, got {0}")]
    BadFileSize(usize),
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Header / dimensional info for a parsed map.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MapHeader {
    /// Grid width in cells (always 200).
    pub width: u32,
    /// Grid height in cells including padding (always 252).
    pub height: u32,
    /// Number of active (non-border) rows (always 202).
    pub active_rows: u32,
}

/// A single tile cell from the grid.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MapTile {
    /// Tile sprite index into the `.TIL` tileset (lower 15 bits of bytes 2-3).
    pub tile_index: u16,
    /// High bit of the tile index word — likely a flip or variant flag.
    pub tile_flag: bool,
    /// Raw object/placement data from bytes 0-1.
    pub cell_flags: u16,
    /// Whether this cell is a border/unused cell (byte 0 == 0xFF).
    pub is_border: bool,
}

/// References to associated asset files, extracted from the string table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MapAssetRefs {
    /// Path to the tile sprite sheet (`.TIL`).
    pub tileset_path: String,
    /// Path to the tile metadata file (`TILES*.DAT`).
    pub tile_meta_path: String,
    /// Path to the object sprite sheet (`.OBJ`).
    pub object_sprite_path: String,
    /// Path to the object metadata file (`OBJ*.DAT`).
    pub object_meta_path: String,
}

/// A fully parsed MAP file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GameMap {
    /// Dimensional header.
    pub header: MapHeader,
    /// Tile grid as a flat vec, row-major order (width * height entries).
    pub tiles: Vec<MapTile>,
    /// Asset file references from the string table.
    pub asset_refs: MapAssetRefs,
    /// Raw metadata footer (unparsed — format partially understood).
    pub metadata: Vec<u8>,
}

impl GameMap {
    /// Get the tile at grid position `(x, y)`. Returns `None` if out of bounds.
    pub fn get_tile(&self, x: usize, y: usize) -> Option<&MapTile> {
        if x >= GRID_WIDTH || y >= GRID_HEIGHT {
            return None;
        }
        Some(&self.tiles[y * GRID_WIDTH + x])
    }

    /// Grid width in cells.
    pub fn width(&self) -> usize {
        GRID_WIDTH
    }

    /// Grid height in cells (including border rows).
    pub fn height(&self) -> usize {
        GRID_HEIGHT
    }

    /// Number of active (non-border) rows.
    pub fn active_rows(&self) -> usize {
        GRID_ACTIVE_ROWS
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a MAP file from disk.
///
/// The file must be exactly 248,384 bytes.
pub fn parse_map(path: &Path) -> Result<GameMap, MapError> {
    debug!(path = %path.display(), "parsing MAP file");

    let data = std::fs::read(path)?;
    parse_map_bytes(&data, path)
}

/// Parse a MAP file from a byte buffer. `path` is used only for log messages.
pub fn parse_map_bytes(data: &[u8], path: &Path) -> Result<GameMap, MapError> {
    if data.len() != MAP_FILE_SIZE {
        return Err(MapError::BadFileSize(data.len()));
    }

    // --- Tile grid ---
    let tiles = parse_tile_grid(&data[..TILE_GRID_SIZE]);

    // Count non-border tiles for logging
    let active_count = tiles.iter().filter(|t| !t.is_border).count();
    debug!(
        total_cells = GRID_WIDTH * GRID_HEIGHT,
        active_cells = active_count,
        border_cells = tiles.len() - active_count,
        "tile grid parsed"
    );

    // --- String table ---
    let asset_refs = parse_string_table(&data[STRING_TABLE_OFFSET..]);
    debug!(
        tileset = %asset_refs.tileset_path,
        tile_meta = %asset_refs.tile_meta_path,
        obj_sprite = %asset_refs.object_sprite_path,
        obj_meta = %asset_refs.object_meta_path,
        "asset references parsed"
    );

    // --- Metadata footer ---
    let metadata = data[METADATA_OFFSET..].to_vec();
    debug_assert_eq!(metadata.len(), METADATA_SIZE);
    trace!(metadata_size = metadata.len(), "metadata footer captured (unparsed)");

    let header = MapHeader {
        width: GRID_WIDTH as u32,
        height: GRID_HEIGHT as u32,
        active_rows: GRID_ACTIVE_ROWS as u32,
    };

    debug!(path = %path.display(), "MAP file parsed successfully");

    Ok(GameMap {
        header,
        tiles,
        asset_refs,
        metadata,
    })
}

/// Parse the tile grid region into a vec of `MapTile`.
fn parse_tile_grid(data: &[u8]) -> Vec<MapTile> {
    let cell_count = GRID_WIDTH * GRID_HEIGHT;
    let mut tiles = Vec::with_capacity(cell_count);

    for i in 0..cell_count {
        let offset = i * CELL_SIZE;
        let mut cursor = Cursor::new(&data[offset..offset + CELL_SIZE]);

        let cell_flags = cursor.read_u16::<LittleEndian>().unwrap();
        let tile_word = cursor.read_u16::<LittleEndian>().unwrap();

        let tile_index = tile_word & 0x7FFF;
        let tile_flag = (tile_word & 0x8000) != 0;
        let is_border = (cell_flags >> 8) as u8 == BORDER_MARKER;

        tiles.push(MapTile {
            tile_index,
            tile_flag,
            cell_flags,
            is_border,
        });

        if i < 4 {
            trace!(
                cell = i,
                cell_flags = format_args!("0x{cell_flags:04X}"),
                tile_index,
                tile_flag,
                is_border,
                "tile cell"
            );
        }
    }

    tiles
}

/// Parse the string table (4 x 164-byte null-padded entries).
fn parse_string_table(data: &[u8]) -> MapAssetRefs {
    let read_entry = |idx: usize| -> String {
        let start = idx * STRING_ENTRY_SIZE;
        let end = start + STRING_ENTRY_SIZE;
        let slice = &data[start..end];
        // Find first null byte
        let len = slice.iter().position(|&b| b == 0).unwrap_or(STRING_ENTRY_SIZE);
        String::from_utf8_lossy(&slice[..len]).to_string()
    };

    MapAssetRefs {
        tileset_path: read_entry(0),
        tile_meta_path: read_entry(1),
        object_sprite_path: read_entry(2),
        object_meta_path: read_entry(3),
    }
}

/// Extract just the filename from a Windows-style path string.
///
/// Useful for resolving the original `C:\WOW\...` paths to local filenames.
pub fn filename_from_build_path(build_path: &str) -> &str {
    build_path
        .rsplit('\\')
        .next()
        .unwrap_or(build_path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid MAP buffer for testing.
    fn make_test_map() -> Vec<u8> {
        let mut data = vec![0u8; MAP_FILE_SIZE];

        // Write a known tile at (0, 0): cell_flags=0x0001, tile_index=7, tile_flag=true
        // cell_flags LE: 0x01, 0x00
        // tile_word LE: tile_index=7 | 0x8000 = 0x8007 => 0x07, 0x80
        data[0] = 0x01;
        data[1] = 0x00;
        data[2] = 0x07;
        data[3] = 0x80;

        // Write a border cell at row 202, col 0
        let border_offset = (202 * GRID_WIDTH) * CELL_SIZE;
        data[border_offset] = 0x00;
        data[border_offset + 1] = 0xFF; // high byte of cell_flags = 0xFF
        data[border_offset + 2] = 0x00;
        data[border_offset + 3] = 0x00;

        // Write string table entries
        let tileset = b"C:\\WOW\\SPR\\SCEN1\\TILSCN01.TIL";
        let tile_meta = b"C:\\WOW\\SPR\\SCEN1\\TILES1.DAT";
        let obj_sprite = b"C:\\WOW\\SPR\\SCEN1\\SCEN1.OBJ";
        let obj_meta = b"C:\\WOW\\SPR\\SCEN1\\OBJ01.DAT";

        for (i, entry) in [tileset.as_ref(), tile_meta, obj_sprite, obj_meta].iter().enumerate() {
            let offset = STRING_TABLE_OFFSET + i * STRING_ENTRY_SIZE;
            data[offset..offset + entry.len()].copy_from_slice(entry);
        }

        data
    }

    #[test]
    fn parse_known_tile() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        let tile = map.get_tile(0, 0).unwrap();
        assert_eq!(tile.tile_index, 7);
        assert!(tile.tile_flag);
        assert_eq!(tile.cell_flags, 0x0001);
        assert!(!tile.is_border);
    }

    #[test]
    fn parse_border_cell() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        let tile = map.get_tile(0, 202).unwrap();
        assert!(tile.is_border);
    }

    #[test]
    fn parse_string_table_entries() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.asset_refs.tileset_path, r"C:\WOW\SPR\SCEN1\TILSCN01.TIL");
        assert_eq!(map.asset_refs.tile_meta_path, r"C:\WOW\SPR\SCEN1\TILES1.DAT");
        assert_eq!(map.asset_refs.object_sprite_path, r"C:\WOW\SPR\SCEN1\SCEN1.OBJ");
        assert_eq!(map.asset_refs.object_meta_path, r"C:\WOW\SPR\SCEN1\OBJ01.DAT");
    }

    #[test]
    fn filename_extraction() {
        assert_eq!(filename_from_build_path(r"C:\WOW\SPR\SCEN1\TILSCN01.TIL"), "TILSCN01.TIL");
        assert_eq!(filename_from_build_path("TILES1.DAT"), "TILES1.DAT");
        assert_eq!(filename_from_build_path(""), "");
    }

    #[test]
    fn header_dimensions() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.header.width, 200);
        assert_eq!(map.header.height, 252);
        assert_eq!(map.header.active_rows, 202);
        assert_eq!(map.width(), 200);
        assert_eq!(map.height(), 252);
        assert_eq!(map.active_rows(), 202);
    }

    #[test]
    fn out_of_bounds_returns_none() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert!(map.get_tile(200, 0).is_none());
        assert!(map.get_tile(0, 252).is_none());
        assert!(map.get_tile(300, 300).is_none());
    }

    #[test]
    fn bad_file_size() {
        let data = vec![0u8; 1000];
        let err = parse_map_bytes(&data, Path::new("bad.MAP")).unwrap_err();
        assert!(matches!(err, MapError::BadFileSize(1000)));
    }

    #[test]
    fn metadata_captured() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.metadata.len(), METADATA_SIZE);
    }

    #[test]
    fn parse_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.MAP");
        let data = make_test_map();
        std::fs::write(&path, &data).unwrap();

        let map = parse_map(&path).unwrap();
        assert_eq!(map.header.width, 200);
        assert_eq!(map.asset_refs.tileset_path, r"C:\WOW\SPR\SCEN1\TILSCN01.TIL");
    }
}
