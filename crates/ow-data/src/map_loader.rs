//! # MAP File Parser
//!
//! Parses the binary `.MAP` files used by Wages of War for scenario tile grids.
//! All MAP files are exactly 248,384 bytes with a fixed layout of 16 sequential
//! data blocks — no headers, no magic bytes, no compression.
//!
//! ## On-disk layout (byte-exact, from Wow.exe RE)
//!
//! ```text
//! Offset      Size (bytes)  Description
//! ─────────────────────────────────────────────────────
//! 0x000000      40,320      Cell Word 1: tile indices + flags
//! 0x009D80      40,320      Cell Word 2: overlay tiles + flags
//! 0x013B00      40,320      Cell Word 3: terrain/passability
//! 0x01D880      40,320      Cell Word 4: elevation (4 corners)
//! 0x027600      40,320      Cell Word 5: object/entity refs
//! 0x031380         164      Entity placement table A
//! 0x031424         164      Entity placement table B
//! 0x0314C8         164      Entity placement table C
//! 0x03156C         164      Entity placement table D
//! 0x031610           8      Map dimensions / params
//! 0x031618           8      Camera position (initial X, Y)
//! 0x031620           4      Map scroll bounds / flag
//! 0x031624          62      Tile set reference table
//! 0x031662      40,044      Scenario / objective data
//! 0x03B2CE       6,000      AI / patrol waypoint data
//! 0x03CA3E           2      Map version identifier
//! ─────────────────────────────────────────────────────
//! TOTAL:       248,384      (0x3CA40)
//! ```
//!
//! ## Grid dimensions
//!
//! The map is **140 columns x 72 rows = 10,080 cells**. Each cell has 5
//! parallel 32-bit words (one per array). Grid layout confirmed by `idiv 0x8C`
//! and `cmp 0x2760` in the exe's cell indexing functions.
//!
//! ## Cell Word 1 — tile indices + flags
//!
//! Each cell is a packed little-endian `u32` with three 9-bit tile layer
//! indices and 5 flag bits:
//!
//! ```text
//! [31..23] tile_layer_0  (9 bits, 0-511) — base terrain sprite
//! [22..14] tile_layer_1  (9 bits, 0-511) — secondary terrain
//! [13..5]  tile_layer_2  (9 bits, 0-511) — tertiary detail
//! [4]      flag_A        (wall/obstacle)
//! [3]      flag_B        (explored/fog)
//! [2]      flag_C        (roof/cover)
//! [1]      flag_D        (walkable)
//! [0]      (unused)
//! ```
//!
//! ## Cell Word 2 — overlay tiles
//!
//! ```text
//! [31..23] overlay_0     (9 bits) — overlay sprite A
//! [22..14] overlay_1     (9 bits) — overlay sprite B
//! [13]     flag_E        (h-flip)
//! [12]     flag_F        (v-flip)
//! [11]     flag_G        (animated)
//! [10]     flag_H        (transparent)
//! [9..1]   overlay_2     (9 bits) — overlay sprite C
//! [0]      (unused)
//! ```
//!
//! ## Cell Word 3 — terrain/passability
//!
//! ```text
//! [31..8]  12 x 2-bit terrain modifiers (per-edge/corner cover values)
//! [7..0]   terrain_base_type (0-255)
//! ```
//!
//! ## Cell Word 4 — elevation
//!
//! ```text
//! [31..24] 4 x 2-bit elevation flags (cliff/slope type)
//! [23..18] corner_3 height (NW, 0-63)
//! [17..12] corner_2 height (NE, 0-63)
//! [11..6]  corner_1 height (SE, 0-63)
//! [5..0]   corner_0 height (SW, 0-63)
//! ```
//!
//! ## Cell Word 5 — objects/entities
//!
//! ```text
//! [31..26] obj_param_3   (6 bits)
//! [25..20] obj_param_2   (6 bits)
//! [19..14] obj_param_1   (6 bits)
//! [13..8]  obj_param_0   (6 bits)
//! [7..0]   object_id     (8 bits, 0=none, 1-255=OBJ sprite index)
//! ```
//!
//! ## Isometric projection
//!
//! The grid uses a **staggered isometric** layout, NOT standard diamond.
//! Tile dimensions are 128x64 pixels. Odd rows are offset +64px horizontally.
//!
//! ```text
//! screen_x = col * 128
//! screen_y = row * 64
//! if row is odd: screen_x += 64   // half-tile stagger
//! ```

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;
use std::path::Path;
use tracing::{debug, info, trace};

// ---------------------------------------------------------------------------
// Constants — all confirmed by Wow.exe disassembly
// ---------------------------------------------------------------------------

/// Total file size of every MAP file (248,384 bytes = 0x3CA40).
const MAP_FILE_SIZE: usize = 248_384;

/// Grid width in cells (140 columns, confirmed by `idiv 0x8C`).
const GRID_WIDTH: usize = 140;

/// Grid height in cells (72 rows, 10080 / 140).
const GRID_HEIGHT: usize = 72;

/// Total number of cells per array (140 * 72 = 10,080, confirmed by `cmp 0x2760`).
const CELL_COUNT: usize = GRID_WIDTH * GRID_HEIGHT;

/// Bytes per cell in each array (4 bytes = one u32 DWORD).
const CELL_SIZE: usize = 4;

/// Size of one cell array on disk (10,080 * 4 = 40,320 bytes = 0x9D80).
const CELL_ARRAY_SIZE: usize = CELL_COUNT * CELL_SIZE;

/// Number of parallel cell arrays (tile, overlay, terrain, elevation, object).
const NUM_CELL_ARRAYS: usize = 5;

/// Total size of all 5 cell arrays combined (201,600 bytes).
const ALL_CELLS_SIZE: usize = CELL_ARRAY_SIZE * NUM_CELL_ARRAYS;

// -- Offsets for metadata blocks following the 5 cell arrays --

/// Offset where entity placement tables begin (0x031380).
const ENTITY_TABLES_OFFSET: usize = ALL_CELLS_SIZE;

/// Size of each entity placement table (164 bytes, 41 x 4-byte entries).
const ENTITY_TABLE_SIZE: usize = 164;

/// Number of entity placement tables.
const ENTITY_TABLE_COUNT: usize = 4;

/// Offset of map dimensions/params block (0x031610).
const MAP_PARAMS_OFFSET: usize = ENTITY_TABLES_OFFSET + ENTITY_TABLE_SIZE * ENTITY_TABLE_COUNT;

/// Offset of initial camera position (0x031618).
const CAMERA_POS_OFFSET: usize = MAP_PARAMS_OFFSET + 8;

/// Offset of scroll bounds flag (0x031620).
const SCROLL_BOUNDS_OFFSET: usize = CAMERA_POS_OFFSET + 8;

/// Offset of tile set reference table (0x031624).
const TILESET_REFS_OFFSET: usize = SCROLL_BOUNDS_OFFSET + 4;

/// Size of tile set reference table (62 bytes = 31 x u16 entries).
const TILESET_REFS_SIZE: usize = 62;

/// Offset of scenario/objective data (0x031662).
const SCENARIO_DATA_OFFSET: usize = TILESET_REFS_OFFSET + TILESET_REFS_SIZE;

/// Size of scenario/objective data block (40,044 bytes).
const SCENARIO_DATA_SIZE: usize = 40_044;

/// Offset of AI/patrol waypoint data (0x03B2CE).
const WAYPOINT_DATA_OFFSET: usize = SCENARIO_DATA_OFFSET + SCENARIO_DATA_SIZE;

/// Size of AI/patrol waypoint data (6,000 bytes).
const WAYPOINT_DATA_SIZE: usize = 6_000;

/// Offset of map version identifier (0x03CA3E).
const VERSION_OFFSET: usize = WAYPOINT_DATA_OFFSET + WAYPOINT_DATA_SIZE;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur when parsing MAP files.
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("I/O error reading MAP file: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid MAP file size: expected {MAP_FILE_SIZE} bytes, got {0}")]
    BadFileSize(usize),
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Unpacked cell data from all 5 parallel cell arrays.
///
/// Each map cell has 5 words on disk; this struct holds the unpacked fields
/// from all of them for a single grid cell.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MapCell {
    // --- Word 1: tile indices + flags ---

    /// Primary terrain tile sprite index (9 bits, 0-511).
    pub tile_layer_0: u16,
    /// Secondary terrain overlay index (9 bits, 0-511).
    pub tile_layer_1: u16,
    /// Tertiary terrain detail index (9 bits, 0-511).
    pub tile_layer_2: u16,
    /// Wall/obstacle flag.
    pub flag_wall: bool,
    /// Explored / fog-of-war flag.
    pub flag_explored: bool,
    /// Roof/cover flag.
    pub flag_roof: bool,
    /// Walkable flag.
    pub flag_walkable: bool,

    // --- Word 2: overlay tiles ---

    /// Overlay sprite A index (9 bits, 0-511).
    pub overlay_0: u16,
    /// Overlay sprite B index (9 bits, 0-511).
    pub overlay_1: u16,
    /// Overlay sprite C index (9 bits, 0-511).
    pub overlay_2: u16,
    /// Overlay horizontal flip flag.
    pub overlay_hflip: bool,
    /// Overlay vertical flip flag.
    pub overlay_vflip: bool,
    /// Overlay animated flag.
    pub overlay_animated: bool,
    /// Overlay transparency flag.
    pub overlay_transparent: bool,

    // --- Word 3: terrain/passability ---

    /// Base terrain type (8 bits, 0-255). 0=open, higher=different terrain.
    pub terrain_base: u8,
    /// 12 per-edge/corner passability modifiers (each 2 bits: 0=open, 1=partial, 2=full cover, 3=impassable).
    pub terrain_mods: [u8; 12],

    // --- Word 4: elevation ---

    /// Southwest corner height (6 bits, 0-63).
    pub elevation_sw: u8,
    /// Southeast corner height (6 bits, 0-63).
    pub elevation_se: u8,
    /// Northeast corner height (6 bits, 0-63).
    pub elevation_ne: u8,
    /// Northwest corner height (6 bits, 0-63).
    pub elevation_nw: u8,
    /// 4 x 2-bit elevation flags (cliff/slope rendering hints).
    pub elevation_flags: [u8; 4],

    // --- Word 5: objects/entities ---

    /// Object sprite index (8 bits, 0=none, 1-255=index into OBJ sprite file).
    pub object_id: u8,
    /// Object parameter 0 — sub-index or variant (6 bits, 0-63).
    pub obj_param_0: u8,
    /// Object parameter 1 — Y offset or height (6 bits, 0-63).
    pub obj_param_1: u8,
    /// Object parameter 2 — rotation or flip (6 bits, 0-63).
    pub obj_param_2: u8,
    /// Object parameter 3 — additional flags (6 bits, 0-63).
    pub obj_param_3: u8,
}

/// Map header with dimensional info and initial camera position.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MapHeader {
    /// Grid width in cells (always 140).
    pub width: u32,
    /// Grid height in cells (always 72).
    pub height: u32,
    /// Initial camera X position (from on-disk data).
    pub camera_x: i16,
    /// Initial camera Y position (from on-disk data).
    pub camera_y: i16,
    /// Map version identifier.
    pub version: u16,
}

/// References to tileset files, extracted from the tileset reference table
/// and entity placement tables.
///
/// The original MAP format stores 4 x 164-byte entity tables that contain
/// build-path strings (like `C:\WOW\SPR\SCEN1\TILSCN01.TIL`). We extract
/// these as asset refs for locating tileset/OBJ files.
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

/// A fully parsed MAP file with all 5 cell arrays unpacked.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GameMap {
    /// Dimensional header and camera info.
    pub header: MapHeader,
    /// All 10,080 cells in row-major order (row * 140 + col).
    pub cells: Vec<MapCell>,
    /// Asset file references parsed from entity placement tables.
    pub asset_refs: MapAssetRefs,
    /// Tileset reference table (31 x u16 indices, 62 bytes raw).
    pub tileset_refs: Vec<u16>,
    /// Raw scenario/objective data (40,044 bytes, partially understood).
    pub scenario_data: Vec<u8>,
    /// Raw AI/patrol waypoint data (6,000 bytes).
    pub waypoint_data: Vec<u8>,
    /// Raw entity placement tables (4 x 164 bytes).
    pub entity_tables: Vec<Vec<u8>>,
}

impl GameMap {
    /// Get the cell at grid position `(col, row)`. Returns `None` if out of bounds.
    pub fn get_cell(&self, col: usize, row: usize) -> Option<&MapCell> {
        if col >= GRID_WIDTH || row >= GRID_HEIGHT {
            return None;
        }
        Some(&self.cells[row * GRID_WIDTH + col])
    }

    /// Grid width in cells (always 140).
    pub fn width(&self) -> usize {
        GRID_WIDTH
    }

    /// Grid height in cells (always 72).
    pub fn height(&self) -> usize {
        GRID_HEIGHT
    }

    /// Total cell count (always 10,080).
    pub fn cell_count(&self) -> usize {
        CELL_COUNT
    }

    // -- Backwards-compatible shims for code that still calls the old API --
    // These will be removed once all callers migrate to get_cell().

    /// Backwards-compatible alias for get_cell that returns a MapTile view.
    pub fn get_tile(&self, col: usize, row: usize) -> Option<MapTileView<'_>> {
        self.get_cell(col, row).map(|cell| MapTileView { cell })
    }

    /// Alias for height() — the old parser had separate "active rows" vs total,
    /// but the real format is 72 rows total with no border padding.
    pub fn active_rows(&self) -> usize {
        GRID_HEIGHT
    }
}

/// Backwards-compatible view of a MapCell that exposes the old MapTile fields.
/// This allows existing rendering code to keep working while we migrate.
pub struct MapTileView<'a> {
    cell: &'a MapCell,
}

impl<'a> MapTileView<'a> {
    pub fn layer0(&self) -> u16 {
        self.cell.tile_layer_0
    }
    pub fn layer1(&self) -> u16 {
        self.cell.tile_layer_1
    }
    pub fn layer2(&self) -> u16 {
        self.cell.tile_layer_2
    }
    pub fn flags(&self) -> u8 {
        // Reconstruct the 5-bit flag byte for compat
        let mut f: u8 = 0;
        if self.cell.flag_wall {
            f |= 0x10;
        }
        if self.cell.flag_explored {
            f |= 0x08;
        }
        if self.cell.flag_roof {
            f |= 0x04;
        }
        if self.cell.flag_walkable {
            f |= 0x02;
        }
        f
    }
    /// The new parser has no "border" concept — all 10,080 cells are valid.
    /// Cells with tile_layer_0 == 0 may be empty terrain but are not borders.
    pub fn is_border(&self) -> bool {
        false
    }
    /// Access the full underlying MapCell.
    pub fn cell(&self) -> &MapCell {
        self.cell
    }
}

// Backwards-compatible MapTile type alias — old code references this struct.
// New code should use MapCell directly.
pub type MapTile = MapCell;

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a MAP file from disk.
///
/// The file must be exactly 248,384 bytes (0x3CA40). Returns the fully
/// parsed map with all 5 cell arrays unpacked and metadata extracted.
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

    info!(
        path = %path.display(),
        file_size = data.len(),
        grid = %format!("{GRID_WIDTH}x{GRID_HEIGHT}"),
        cells = CELL_COUNT,
        "parsing MAP: 5 parallel cell arrays"
    );

    // --- Read the 5 parallel cell arrays ---
    // Each array is 40,320 bytes (10,080 cells x 4 bytes/cell).
    // They are stored sequentially on disk: word1, word2, word3, word4, word5.

    let word1_data = &data[0..CELL_ARRAY_SIZE];
    let word2_data = &data[CELL_ARRAY_SIZE..CELL_ARRAY_SIZE * 2];
    let word3_data = &data[CELL_ARRAY_SIZE * 2..CELL_ARRAY_SIZE * 3];
    let word4_data = &data[CELL_ARRAY_SIZE * 3..CELL_ARRAY_SIZE * 4];
    let word5_data = &data[CELL_ARRAY_SIZE * 4..CELL_ARRAY_SIZE * 5];

    // Unpack all 10,080 cells from the 5 parallel arrays.
    let cells = unpack_all_cells(word1_data, word2_data, word3_data, word4_data, word5_data);

    // Log a sample of the first few cells for debugging.
    for i in 0..4.min(cells.len()) {
        let c = &cells[i];
        let col = i % GRID_WIDTH;
        let row = i / GRID_WIDTH;
        trace!(
            cell = i,
            col,
            row,
            tile_0 = c.tile_layer_0,
            tile_1 = c.tile_layer_1,
            tile_2 = c.tile_layer_2,
            wall = c.flag_wall,
            walkable = c.flag_walkable,
            overlay_0 = c.overlay_0,
            overlay_1 = c.overlay_1,
            overlay_2 = c.overlay_2,
            terrain = c.terrain_base,
            elev_sw = c.elevation_sw,
            elev_ne = c.elevation_ne,
            obj_id = c.object_id,
            "cell sample"
        );
    }

    // Count cells with non-zero tile indices for a quick sanity check.
    let non_empty = cells.iter().filter(|c| c.tile_layer_0 > 0).count();
    let with_objects = cells.iter().filter(|c| c.object_id > 0).count();
    debug!(
        non_empty_tiles = non_empty,
        cells_with_objects = with_objects,
        "cell arrays unpacked"
    );

    // --- Entity placement tables (4 x 164 bytes) ---
    // These contain build-path strings referencing the original dev machine's
    // file paths. We parse them as asset references for locating TIL/OBJ files.
    let entity_tables: Vec<Vec<u8>> = (0..ENTITY_TABLE_COUNT)
        .map(|i| {
            let start = ENTITY_TABLES_OFFSET + i * ENTITY_TABLE_SIZE;
            data[start..start + ENTITY_TABLE_SIZE].to_vec()
        })
        .collect();

    let asset_refs = parse_entity_tables_as_strings(&entity_tables);
    debug!(
        tileset = %asset_refs.tileset_path,
        tile_meta = %asset_refs.tile_meta_path,
        obj_sprite = %asset_refs.object_sprite_path,
        obj_meta = %asset_refs.object_meta_path,
        "asset references parsed from entity tables"
    );

    // --- Camera position (8 bytes at offset 0x031618) ---
    let mut cam_cursor = Cursor::new(&data[CAMERA_POS_OFFSET..CAMERA_POS_OFFSET + 8]);
    let camera_x = cam_cursor.read_i16::<LittleEndian>().unwrap_or(0);
    let camera_y = cam_cursor.read_i16::<LittleEndian>().unwrap_or(0);
    debug!(camera_x, camera_y, "initial camera position");

    // --- Tileset reference table (62 bytes = 31 x u16 at offset 0x031624) ---
    let tileset_refs: Vec<u16> = (0..31)
        .map(|i| {
            let off = TILESET_REFS_OFFSET + i * 2;
            let mut c = Cursor::new(&data[off..off + 2]);
            c.read_u16::<LittleEndian>().unwrap_or(0)
        })
        .collect();

    // --- Version identifier (2 bytes at offset 0x03CA3E) ---
    let mut ver_cursor = Cursor::new(&data[VERSION_OFFSET..VERSION_OFFSET + 2]);
    let version = ver_cursor.read_u16::<LittleEndian>().unwrap_or(0);
    debug!(version, "map version");

    // --- Scenario and waypoint data (raw, partially understood) ---
    let scenario_data = data[SCENARIO_DATA_OFFSET..SCENARIO_DATA_OFFSET + SCENARIO_DATA_SIZE].to_vec();
    let waypoint_data = data[WAYPOINT_DATA_OFFSET..WAYPOINT_DATA_OFFSET + WAYPOINT_DATA_SIZE].to_vec();

    let header = MapHeader {
        width: GRID_WIDTH as u32,
        height: GRID_HEIGHT as u32,
        camera_x,
        camera_y,
        version,
    };

    info!(
        path = %path.display(),
        grid = %format!("{}x{}", header.width, header.height),
        non_empty_tiles = non_empty,
        objects = with_objects,
        camera = %format!("({}, {})", camera_x, camera_y),
        version,
        "MAP file parsed successfully"
    );

    Ok(GameMap {
        header,
        cells,
        asset_refs,
        tileset_refs,
        scenario_data,
        waypoint_data,
        entity_tables,
    })
}

// ---------------------------------------------------------------------------
// Cell unpacking — all 5 words for each cell
// ---------------------------------------------------------------------------

/// Unpack all 10,080 cells from the 5 parallel cell arrays.
///
/// Each array is read as a sequence of little-endian u32 values. The bit
/// packing for each word was confirmed by RE analysis of the Pack/Unpack
/// functions in Wow.exe (0x41AF7B through 0x41B4D0).
fn unpack_all_cells(
    word1: &[u8],
    word2: &[u8],
    word3: &[u8],
    word4: &[u8],
    word5: &[u8],
) -> Vec<MapCell> {
    let mut cells = Vec::with_capacity(CELL_COUNT);

    for i in 0..CELL_COUNT {
        let off = i * CELL_SIZE;

        // Read raw u32 from each of the 5 arrays.
        let w1 = read_u32_le(&word1[off..off + 4]);
        let w2 = read_u32_le(&word2[off..off + 4]);
        let w3 = read_u32_le(&word3[off..off + 4]);
        let w4 = read_u32_le(&word4[off..off + 4]);
        let w5 = read_u32_le(&word5[off..off + 4]);

        cells.push(unpack_cell(w1, w2, w3, w4, w5));
    }

    cells
}

/// Read a little-endian u32 from a 4-byte slice.
#[inline]
fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Unpack a single cell from its 5 raw word values.
///
/// Bit layouts are documented in the module header and match the RE-confirmed
/// pack/unpack functions at 0x41AF7B-0x41B4D0 in Wow.exe.
fn unpack_cell(w1: u32, w2: u32, w3: u32, w4: u32, w5: u32) -> MapCell {
    // --- Word 1: three 9-bit tile layers + 5 flag bits ---
    let tile_layer_0 = ((w1 >> 23) & 0x1FF) as u16;
    let tile_layer_1 = ((w1 >> 14) & 0x1FF) as u16;
    let tile_layer_2 = ((w1 >> 5) & 0x1FF) as u16;
    let flag_wall = (w1 >> 4) & 1 != 0;
    let flag_explored = (w1 >> 3) & 1 != 0;
    let flag_roof = (w1 >> 2) & 1 != 0;
    let flag_walkable = (w1 >> 1) & 1 != 0;

    // --- Word 2: three 9-bit overlay layers + 4 flags ---
    let overlay_0 = ((w2 >> 23) & 0x1FF) as u16;
    let overlay_1 = ((w2 >> 14) & 0x1FF) as u16;
    let overlay_2 = ((w2 >> 1) & 0x1FF) as u16;
    let overlay_hflip = (w2 >> 13) & 1 != 0;
    let overlay_vflip = (w2 >> 12) & 1 != 0;
    let overlay_animated = (w2 >> 11) & 1 != 0;
    let overlay_transparent = (w2 >> 10) & 1 != 0;

    // --- Word 3: 12 x 2-bit terrain mods + 8-bit base ---
    let terrain_base = (w3 & 0xFF) as u8;
    let mut terrain_mods = [0u8; 12];
    for j in 0..12 {
        // Modifiers are packed from bit 8 upward, mod_0 at bits [9..8],
        // mod_1 at [11..10], ... mod_11 at [31..30].
        terrain_mods[j] = ((w3 >> (8 + j * 2)) & 0x03) as u8;
    }

    // --- Word 4: 4 x 6-bit corner heights + 4 x 2-bit flags ---
    let elevation_sw = (w4 & 0x3F) as u8;
    let elevation_se = ((w4 >> 6) & 0x3F) as u8;
    let elevation_ne = ((w4 >> 12) & 0x3F) as u8;
    let elevation_nw = ((w4 >> 18) & 0x3F) as u8;
    let mut elevation_flags = [0u8; 4];
    for j in 0..4 {
        elevation_flags[j] = ((w4 >> (24 + j * 2)) & 0x03) as u8;
    }

    // --- Word 5: 8-bit object_id + 4 x 6-bit params ---
    let object_id = (w5 & 0xFF) as u8;
    let obj_param_0 = ((w5 >> 8) & 0x3F) as u8;
    let obj_param_1 = ((w5 >> 14) & 0x3F) as u8;
    let obj_param_2 = ((w5 >> 20) & 0x3F) as u8;
    let obj_param_3 = ((w5 >> 26) & 0x3F) as u8;

    MapCell {
        tile_layer_0,
        tile_layer_1,
        tile_layer_2,
        flag_wall,
        flag_explored,
        flag_roof,
        flag_walkable,
        overlay_0,
        overlay_1,
        overlay_2,
        overlay_hflip,
        overlay_vflip,
        overlay_animated,
        overlay_transparent,
        terrain_base,
        terrain_mods,
        elevation_sw,
        elevation_se,
        elevation_ne,
        elevation_nw,
        elevation_flags,
        object_id,
        obj_param_0,
        obj_param_1,
        obj_param_2,
        obj_param_3,
    }
}

// ---------------------------------------------------------------------------
// Entity table / string extraction
// ---------------------------------------------------------------------------

/// Parse entity placement tables as null-terminated path strings.
///
/// The original game stores Windows build paths (e.g. `C:\WOW\SPR\SCEN1\...`)
/// in the entity tables. We extract the first null-terminated string from each.
fn parse_entity_tables_as_strings(tables: &[Vec<u8>]) -> MapAssetRefs {
    let read_string = |table: &[u8]| -> String {
        let len = table.iter().position(|&b| b == 0).unwrap_or(table.len());
        String::from_utf8_lossy(&table[..len]).to_string()
    };

    MapAssetRefs {
        tileset_path: if tables.len() > 0 { read_string(&tables[0]) } else { String::new() },
        tile_meta_path: if tables.len() > 1 { read_string(&tables[1]) } else { String::new() },
        object_sprite_path: if tables.len() > 2 { read_string(&tables[2]) } else { String::new() },
        object_meta_path: if tables.len() > 3 { read_string(&tables[3]) } else { String::new() },
    }
}

/// Extract just the filename from a Windows-style build path string.
///
/// The MAP file stores paths like `C:\WOW\SPR\SCEN1\TILSCN01.TIL`.
/// We need just the filename to resolve it against our local data directory.
pub fn filename_from_build_path(build_path: &str) -> &str {
    build_path.rsplit('\\').next().unwrap_or(build_path)
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

        // Write a known cell at index 0 (col=0, row=0) in Word 1:
        //   tile_layer_0=7, tile_layer_1=3, tile_layer_2=1
        //   flag_wall=true, flag_explored=false, flag_roof=true, flag_walkable=false
        //   Packed: (7 << 23) | (3 << 14) | (1 << 5) | (1 << 4) | (1 << 2)
        let w1: u32 = (7 << 23) | (3 << 14) | (1 << 5) | (1 << 4) | (1 << 2);
        data[0..4].copy_from_slice(&w1.to_le_bytes());

        // Write Word 5 for same cell: object_id=42, obj_param_0=5
        let w5: u32 = 42 | (5 << 8);
        let w5_offset = CELL_ARRAY_SIZE * 4; // word5 starts at array index 4
        data[w5_offset..w5_offset + 4].copy_from_slice(&w5.to_le_bytes());

        // Write entity table strings (asset paths).
        let paths = [
            b"C:\\WOW\\SPR\\SCEN1\\TILSCN01.TIL".as_ref(),
            b"C:\\WOW\\SPR\\SCEN1\\TILES1.DAT",
            b"C:\\WOW\\SPR\\SCEN1\\SCEN1.OBJ",
            b"C:\\WOW\\SPR\\SCEN1\\OBJ01.DAT",
        ];
        for (i, path) in paths.iter().enumerate() {
            let offset = ENTITY_TABLES_OFFSET + i * ENTITY_TABLE_SIZE;
            data[offset..offset + path.len()].copy_from_slice(path);
        }

        // Write camera position: (320, 240)
        data[CAMERA_POS_OFFSET..CAMERA_POS_OFFSET + 2].copy_from_slice(&320i16.to_le_bytes());
        data[CAMERA_POS_OFFSET + 2..CAMERA_POS_OFFSET + 4].copy_from_slice(&240i16.to_le_bytes());

        // Write version: 1
        data[VERSION_OFFSET..VERSION_OFFSET + 2].copy_from_slice(&1u16.to_le_bytes());

        data
    }

    #[test]
    fn parse_known_cell() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        let cell = map.get_cell(0, 0).unwrap();
        assert_eq!(cell.tile_layer_0, 7, "tile_layer_0");
        assert_eq!(cell.tile_layer_1, 3, "tile_layer_1");
        assert_eq!(cell.tile_layer_2, 1, "tile_layer_2");
        assert!(cell.flag_wall, "flag_wall should be set");
        assert!(!cell.flag_explored, "flag_explored should be clear");
        assert!(cell.flag_roof, "flag_roof should be set");
        assert!(!cell.flag_walkable, "flag_walkable should be clear");
    }

    #[test]
    fn parse_object_data() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        let cell = map.get_cell(0, 0).unwrap();
        assert_eq!(cell.object_id, 42, "object_id");
        assert_eq!(cell.obj_param_0, 5, "obj_param_0");
    }

    #[test]
    fn parse_asset_refs() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.asset_refs.tileset_path, r"C:\WOW\SPR\SCEN1\TILSCN01.TIL");
        assert_eq!(map.asset_refs.tile_meta_path, r"C:\WOW\SPR\SCEN1\TILES1.DAT");
        assert_eq!(map.asset_refs.object_sprite_path, r"C:\WOW\SPR\SCEN1\SCEN1.OBJ");
        assert_eq!(map.asset_refs.object_meta_path, r"C:\WOW\SPR\SCEN1\OBJ01.DAT");
    }

    #[test]
    fn parse_camera_position() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.header.camera_x, 320);
        assert_eq!(map.header.camera_y, 240);
    }

    #[test]
    fn parse_version() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.header.version, 1);
    }

    #[test]
    fn grid_dimensions() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert_eq!(map.width(), 140, "grid width");
        assert_eq!(map.height(), 72, "grid height");
        assert_eq!(map.cell_count(), 10_080, "cell count");
        assert_eq!(map.cells.len(), 10_080, "cells vec length");
    }

    #[test]
    fn out_of_bounds_returns_none() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        assert!(map.get_cell(140, 0).is_none(), "col 140 should be OOB");
        assert!(map.get_cell(0, 72).is_none(), "row 72 should be OOB");
        assert!(map.get_cell(200, 200).is_none(), "both OOB");
        assert!(map.get_cell(139, 71).is_some(), "max valid cell");
    }

    #[test]
    fn backwards_compat_get_tile() {
        let data = make_test_map();
        let map = parse_map_bytes(&data, Path::new("test.MAP")).unwrap();

        // The old API used get_tile() — make sure it still works.
        let tile = map.get_tile(0, 0).unwrap();
        assert_eq!(tile.layer0(), 7);
        assert_eq!(tile.layer1(), 3);
        assert_eq!(tile.layer2(), 1);
        assert!(!tile.is_border(), "new parser has no border concept");
    }

    #[test]
    fn bad_file_size() {
        let data = vec![0u8; 1000];
        let err = parse_map_bytes(&data, Path::new("bad.MAP")).unwrap_err();
        assert!(matches!(err, MapError::BadFileSize(1000)));
    }

    #[test]
    fn filename_extraction() {
        assert_eq!(
            filename_from_build_path(r"C:\WOW\SPR\SCEN1\TILSCN01.TIL"),
            "TILSCN01.TIL"
        );
        assert_eq!(filename_from_build_path("TILES1.DAT"), "TILES1.DAT");
        assert_eq!(filename_from_build_path(""), "");
    }

    #[test]
    fn cell_word_2_round_trip() {
        // Verify overlay unpacking: overlay_0=100, overlay_1=200, overlay_2=50
        let w2: u32 = (100 << 23) | (200 << 14) | (1 << 13) | (50 << 1);
        let cell = unpack_cell(0, w2, 0, 0, 0);
        assert_eq!(cell.overlay_0, 100);
        assert_eq!(cell.overlay_1, 200);
        assert_eq!(cell.overlay_2, 50);
        assert!(cell.overlay_hflip);
    }

    #[test]
    fn cell_word_4_elevation() {
        // SW=10, SE=20, NE=30, NW=40
        let w4: u32 = 10 | (20 << 6) | (30 << 12) | (40 << 18);
        let cell = unpack_cell(0, 0, 0, w4, 0);
        assert_eq!(cell.elevation_sw, 10);
        assert_eq!(cell.elevation_se, 20);
        assert_eq!(cell.elevation_ne, 30);
        assert_eq!(cell.elevation_nw, 40);
    }
}
