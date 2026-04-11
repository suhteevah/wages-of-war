# Binary Formats Deep RE Analysis

**Source:** Static disassembly of `Wow.exe` (PE32, 1,073,664 bytes)
**Build:** MSVC 4.x, Mon Nov 11 14:27:00 1996
**Method:** Clean-room behavioral analysis from disassembly. No code copied.
**Date:** 2026-04-09

---

## 1. MAP File Format

### 1.1 File Size and Structure

**Total file size: 248,384 bytes (0x3CA40) -- CONFIRMED**

The MAP file is a flat, headerless binary blob with no magic number. It consists of 16 sequential data blocks written/read in fixed order. There is no compression, no checksums, no alignment padding between blocks.

**Loading function: `0x4A0680` (LoadMap)**
**Saving function: `0x4A0882` (SaveMap)**

Both call `0x40269E` (OpenFile wrapper), then perform sequential reads/writes via `_hread` (IAT `0x693534`) or `_hwrite` (IAT `0x69351C`), then `0x401181` (CloseFile).

### 1.2 On-Disk Layout (byte-exact)

```
Offset      Size (bytes)  Hex Size  Destination Address  Description
─────────────────────────────────────────────────────────────────────────────
0x000000      40,320      0x9D80    0x59D8C0            Cell Word 1: Tile indices + flags
0x009D80      40,320      0x9D80    0x5A7640            Cell Word 2: Overlay tiles + flags
0x013B00      40,320      0x9D80    0x5D07C0            Cell Word 3: Terrain/passability
0x01D880      40,320      0x9D80    0x5DA540            Cell Word 4: Elevation
0x027600      40,320      0x9D80    0x5E42C0            Cell Word 5: Object/entity refs
0x031380         164      0xA4      0x625870            Entity placement table A
0x031424         164      0xA4      0x630EF0            Entity placement table B
0x0314C8         164      0xA4      0x630E40            Entity placement table C
0x03156C         164      0xA4      0x625710            Entity placement table D
0x031610           8      0x08      0x62B920            Map dimensions / params A
0x031618           8      0x08      0x630EE8            Camera position (initial X, Y)
0x031620           4      0x04      0x625864            Map scroll bounds / flag
0x031624          62      0x3E      0x59D190            Tile set reference table
0x031662      40,044      0x9C6C    0x53FA50            Scenario / objective data
0x03B2CE       6,000      0x1770    0x5F57D0            AI / patrol waypoint data
0x03CA3E           2      0x02      0x4F1318            Map version identifier
─────────────────────────────────────────────────────────────────────────────
TOTAL:       248,384      0x3CA40
```

### 1.3 Cell Data: 4 Bytes Per Cell, 5 Parallel Arrays

Each of the 5 cell arrays stores **10,080 cells x 4 bytes = 40,320 bytes**.

The map grid is **140 columns x 72 rows = 10,080 cells**.
- Grid width (columns): **140** (0x8C) -- confirmed by `idiv $0x8C` at multiple locations
- Grid height (rows): **72** (10080 / 140 = 72)

Cell index formula: `cell_index = row * 140 + column`

### 1.4 Metadata Blocks

**Entity placement tables (4 x 164 bytes):**
- 164 bytes / 4 bytes = 41 entries each
- Likely: 41 entity/merc placement slots per table (max units on map)
- 4 tables for: player start positions, enemy positions, objective locations, reinforcements

**Camera position (8 bytes at offset 0x031618):**
- Loaded to `0x630EE8` (camera_x as short) and `0x630EEA` (camera_y as short)
- Remaining 4 bytes: likely camera zoom or map bounds

**Tile set reference (62 bytes at offset 0x031624):**
- Loaded to `0x59D190`
- 31 short entries: indices into available tile sets for this map

**Scenario data (40,044 bytes at offset 0x031662):**
- Loaded to `0x53FA50`
- Mission objectives, triggers, scripted events

**AI waypoints (6,000 bytes at offset 0x03B2CE):**
- Loaded to `0x5F57D0`
- Patrol routes, alert level movement grids

**Map version (2 bytes at offset 0x03CA3E):**
- Stored to `0x4F1318`
- After loading, `0x4F35D8` is set to 2 and `0x4F35E0` is set to 0 (editor state flags)

### 1.5 Third Map Function: Scenario Data Loader (0x4A0A83)

A separate function at `0x4A0A83` loads an additional scenario overlay file:
- Opens file with mode `0x01` (read-only)
- Reads **0x5000 = 20,480 bytes** into `0x62B930`
- This is likely a separate `.scen` file, not part of the `.map` file

---

## 2. Cell Word Packing Format (Complete)

All cell data is stored as **packed 32-bit DWORDs** on disk, identical to in-memory representation. The on-disk format IS the in-memory format -- a direct flat binary dump of the 5 arrays.

### 2.1 Cell Word 1 (0x59D8C0) -- Tile Indices + Flags

**Pack function: 0x41AF7B | Unpack function: 0x41B2A6**

```
Bit layout (32 bits, MSB first during packing):

Pack order (high bits packed first):
  temp = 0
  temp = (field_5EE060 & 0x1FF)              // tile_layer_0: 9 bits
  temp = (temp << 9) | (field_5EE062 & 0x1FF) // tile_layer_1: 9 bits
  temp = (temp << 9) | (field_5EE064 & 0x1FF) // tile_layer_2: 9 bits
  temp = (temp << 1) | (field_5EE071 & 0x1)   // flag_A: 1 bit
  temp = (temp << 1) | (field_5EE072 & 0x1)   // flag_B: 1 bit
  temp = (temp << 1) | (field_5EE073 & 0x1)   // flag_C: 1 bit
  temp = (temp << 1) | (field_5EE074 & 0x1)   // flag_D: 1 bit
  temp = (temp << 1)                           // unused: 1 bit (always 0)

Resulting bit positions (from LSB):
  [31..23]  tile_layer_0     9 bits (0-511) -- base terrain tile index
  [22..14]  tile_layer_1     9 bits (0-511) -- secondary tile/transition
  [13..5]   tile_layer_2     9 bits (0-511) -- tertiary tile/decoration
  [4]       flag_A           1 bit  -- (wall/obstacle flag)
  [3]       flag_B           1 bit  -- (explored/fog-of-war flag)
  [2]       flag_C           1 bit  -- (roof/cover flag)
  [1]       flag_D           1 bit  -- (walkable flag)
  [0]       (unused)         1 bit
```

**Unpack (at 0x41B2A6):**
```
temp = cellArray1[cellIndex]    // read 4-byte DWORD
shr 1 -> flag_D = temp & 1     // bit 1
shr 1 -> flag_C = temp & 1     // bit 2
shr 1 -> flag_B = temp & 1     // bit 3
shr 1 -> flag_A = temp & 1     // bit 4
shr 1 -> tile_layer_2 = temp & 0x1FF   // bits 5-13
shr 9 -> tile_layer_1 = temp & 0x1FF   // bits 14-22
shr 9 -> tile_layer_0 = temp (remaining) // bits 23-31
```

**CRITICAL: Tile index extraction formula:**
```
tile_layer_0 = (cell_word_1 >> 23) & 0x1FF   // primary terrain (0-511)
tile_layer_1 = (cell_word_1 >> 14) & 0x1FF   // secondary terrain (0-511)
tile_layer_2 = (cell_word_1 >>  5) & 0x1FF   // tertiary (0-511)
```

### 2.2 Cell Word 2 (0x5A7640) -- Overlay Tiles

**Pack function: 0x41B006 | Unpack function: 0x41B31D**

```
Pack order:
  temp = (field_5EE066 & 0x1FF)              // overlay_0: 9 bits
  temp = (temp << 9) | (field_5EE068 & 0x1FF) // overlay_1: 9 bits
  temp = (temp << 1) | (field_5EE06A & 0x1)   // flag_E: 1 bit
  temp = (temp << 1) | (field_5EE06B & 0x1)   // flag_F: 1 bit
  temp = (temp << 1) | (field_5EE06C & 0x1)   // flag_G: 1 bit
  temp = (temp << 1) | (field_5EE06D & 0x1)   // flag_H: 1 bit
  temp = (temp << 9) | (field_5EE06E & 0x1FF) // overlay_2: 9 bits
  temp = (temp << 1)                           // unused: 1 bit

Bit positions (from LSB):
  [31..23]  overlay_tile_0   9 bits (0-511) -- overlay sprite A
  [22..14]  overlay_tile_1   9 bits (0-511) -- overlay sprite B
  [13]      flag_E           1 bit  -- (overlay horizontal flip?)
  [12]      flag_F           1 bit  -- (overlay vertical flip?)
  [11]      flag_G           1 bit  -- (overlay animated?)
  [10]      flag_H           1 bit  -- (overlay transparent?)
  [9..1]    overlay_tile_2   9 bits (0-511) -- overlay sprite C
  [0]       (unused)         1 bit
```

**Overlay index extraction:**
```
overlay_0 = (cell_word_2 >> 23) & 0x1FF
overlay_1 = (cell_word_2 >> 14) & 0x1FF
overlay_2 = (cell_word_2 >>  1) & 0x1FF
```

### 2.3 Cell Word 3 (0x5D07C0) -- Terrain/Passability

**Pack function: 0x41B091 | Unpack function: 0x41B3AD**

This word uses a mix of 2-bit fields and an 8-bit base value:

```
Pack order:
  temp = (field_5EE075 & 0x03)       // terrain_mod_11: 2 bits
  (repeat 10 more 2-bit fields, each shifted left by 2)
  temp <<= 2; temp |= (5EE076 & 0x3)  // terrain_mod_10
  temp <<= 2; temp |= (5EE077 & 0x3)  // terrain_mod_9
  temp <<= 2; temp |= (5EE078 & 0x3)  // terrain_mod_8
  temp <<= 2; temp |= (5EE079 & 0x3)  // terrain_mod_7
  temp <<= 2; temp |= (5EE07A & 0x3)  // terrain_mod_6
  temp <<= 2; temp |= (5EE07B & 0x3)  // terrain_mod_5
  temp <<= 2; temp |= (5EE07C & 0x3)  // terrain_mod_4
  temp <<= 2; temp |= (5EE07D & 0x3)  // terrain_mod_3
  temp <<= 2; temp |= (5EE07E & 0x3)  // terrain_mod_2
  temp <<= 2; temp |= (5EE07F & 0x3)  // terrain_mod_1
  temp <<= 2; temp |= (5EE080 & 0x3)  // terrain_mod_0
  temp <<= 8; temp |= (5EE070 & 0xFF) // terrain_base: 8 bits

Bit positions (from LSB):
  [31..8]   12 x 2-bit terrain modifiers (24 bits total)
            Packed MSB-first: mod_11 is highest, mod_0 is lowest above base
  [7..0]    terrain_base_type (8 bits, 0-255)
```

**Interpretation of 12 modifiers:** Per-edge/per-corner passability and cover values for the isometric diamond. Each 2-bit value encodes: 0=open, 1=partial cover, 2=full cover, 3=impassable.

### 2.4 Cell Word 4 (0x5DA540) -- Elevation

**Pack function: 0x41B175 | Unpack function: 0x41B454**

```
Pack order:
  temp = (field_5EE081 & 0x03)       // elev_flag_3: 2 bits
  temp <<= 2; temp |= (5EE082 & 0x3)  // elev_flag_2
  temp <<= 2; temp |= (5EE083 & 0x3)  // elev_flag_1
  temp <<= 2; temp |= (5EE084 & 0x3)  // elev_flag_0
  temp <<= 6; temp |= (5EE086 & 0x3F) // corner_3: 6 bits (0-63)
  temp <<= 6; temp |= (5EE088 & 0x3F) // corner_2: 6 bits
  temp <<= 6; temp |= (5EE08A & 0x3F) // corner_1: 6 bits
  temp <<= 6; temp |= (5EE08C & 0x3F) // corner_0: 6 bits

Bit positions (from LSB):
  [31..24]  4 x 2-bit elevation flags (8 bits total)
            flag_3 (MSB) through flag_0
  [23..18]  corner_3 height    6 bits (0-63) -- NW corner
  [17..12]  corner_2 height    6 bits (0-63) -- NE corner
  [11..6]   corner_1 height    6 bits (0-63) -- SE corner
  [5..0]    corner_0 height    6 bits (0-63) -- SW corner
```

**Elevation extraction:**
```
corner_0 = (cell_word_4 >>  0) & 0x3F   // SW corner height (0-63)
corner_1 = (cell_word_4 >>  6) & 0x3F   // SE corner height
corner_2 = (cell_word_4 >> 12) & 0x3F   // NE corner height
corner_3 = (cell_word_4 >> 18) & 0x3F   // NW corner height
flag_0   = (cell_word_4 >> 24) & 0x03   // cliff/slope type
flag_1   = (cell_word_4 >> 26) & 0x03
flag_2   = (cell_word_4 >> 28) & 0x03
flag_3   = (cell_word_4 >> 30) & 0x03
```

The 4 corner elevations enable smooth terrain slopes -- each corner of the isometric diamond can have a different height. The 2-bit flags likely encode slope rendering hints (cliff edge rendering, water, etc.).

### 2.5 Cell Word 5 (0x5E42C0) -- Objects/Entities

**Pack function: 0x41B207 | Unpack function: 0x41B4D0**

```
Pack order:
  temp = (field_5EE08E & 0x3F)       // obj_param_3: 6 bits
  temp <<= 6; temp |= (5EE090 & 0x3F) // obj_param_2: 6 bits
  temp <<= 6; temp |= (5EE092 & 0x3F) // obj_param_1: 6 bits
  temp <<= 6; temp |= (5EE094 & 0x3F) // obj_param_0: 6 bits
  temp <<= 8; temp |= (5EE096 & 0xFF) // object_id: 8 bits

Bit positions (from LSB):
  [31..26]  obj_param_3     6 bits (0-63) -- object param/rotation?
  [25..20]  obj_param_2     6 bits (0-63) -- object sub-variant?
  [19..14]  obj_param_1     6 bits (0-63) -- object height offset?
  [13..8]   obj_param_0     6 bits (0-63) -- object sub-index?
  [7..0]    object_id       8 bits (0-255) -- OBJ sprite index
```

**OBJ sprite index extraction:**
```
object_id  = (cell_word_5 >>  0) & 0xFF   // 0-255: index into OBJ sprite set
obj_param_0 = (cell_word_5 >>  8) & 0x3F  // sub-index or variant
obj_param_1 = (cell_word_5 >> 14) & 0x3F  // Y offset or height
obj_param_2 = (cell_word_5 >> 20) & 0x3F  // rotation or flip
obj_param_3 = (cell_word_5 >> 26) & 0x3F  // additional flags
```

When `object_id == 0`, the cell has no object. Non-zero values index into the OBJ sprite file loaded for the map (e.g., `mis01.obj`).

---

## 3. Isometric Projection System

### 3.1 Tile Dimensions (CONFIRMED from disassembly)

```
Tile pixel width:   128 px (0x80)  -- screen_x >> 7 for column calc
Tile pixel height:   64 px (0x40)  -- screen_y >> 6 for row calc
Diamond half-width:  64 px (0x40)  -- added to screen_x for centering
Diamond half-height: 32 px (0x20)  -- added to screen_y for centering
```

The projection is a standard **2:1 isometric diamond** (128 wide, 64 tall).

### 3.2 Grid Dimensions

```
Map columns:  140 (0x8C)  -- confirmed by idiv 0x8C at 0x41B8D2
Map rows:      72 (10080 / 140)
Max cells:  10,080 (0x2760) -- confirmed by cmp $0x2760 at 0x41AF14
```

### 3.3 Screen-to-Tile Conversion (0x45FE11)

The function at `0x45FE11` converts screen pixel coordinates to cell index:

```
function ScreenToCell(screen_x, screen_y):
    // Add camera scroll offset (stored at 0x630EE8, 0x630EEA)
    x = screen_x + camera_x + 64    // +0x40 diamond half-width
    y = screen_y + camera_y + 32    // +0x20 diamond half-height

    // Compute cell index
    //   row = (y / 64)
    //   The LEA chain computes: (y/64) * 140
    //     sar $6 -> /64
    //     then: x7 = (v*8 - v) = v*7
    //     then: x28 = v*7 * 4
    //     then: x140 = v*28 * 5
    row_x_140 = (y >> 6) * 140
    col       = x >> 7                // x / 128
    cell_index = row_x_140 + col

    // Sub-tile coordinates for quadrant detection
    sub_x_grid = x >> 6              // 64px sub-grid for X parity
    sub_y_grid = y >> 5              // 32px sub-grid for Y parity

    // Intra-tile pixel position (for diamond edge detection)
    intra_x = x & 0x7F              // 0-127 within tile
    intra_y = y & 0x3F              // 0-63 within tile
```

### 3.4 Tile-to-Screen Conversion (inverse)

From the neighbor navigation functions (0x41B8AD, 0x41B920, 0x41B982):

```
function CellToScreen(cell_index):
    col = cell_index % 140           // mod 0x8C
    row = cell_index / 140           // div 0x8C

    // Standard staggered isometric:
    screen_x = col * 128 - camera_x
    screen_y = row * 64  - camera_y

    // Staggered rows: odd rows offset by half-tile
    if (row % 2 == 1):
        screen_x += 64              // half tile width offset
```

### 3.5 Cell Neighbor Functions

**Next cell (right): 0x41B8AD**
```
function GetNextCell(cell_index):
    result = call_0x401FFF(cell_index) + cell_index + 0x45  // +69
    if (cell_index % 0x8C == 1): result = 0   // boundary check
    clamp to [0, 10080]
    return result
```

**Previous cell (left): 0x41B920**
```
function GetPrevCell(cell_index):
    result = cell_index - 1
    if (cell_index % 0x46 == 1): result = 0   // boundary (0x46 = 70 = half-row)
    clamp to [0, 10080]
    return result
```

**Below cell: 0x41B982**
```
function GetBelowCell(cell_index):
    result = call_0x401FFF(cell_index) + cell_index - 0x47  // -71
    if (cell_index % 0x8C == 1): result = 0   // boundary
    clamp to [0, 10080]
    return result
```

Constants: **0x45 = 69, 0x46 = 70, 0x47 = 71** (half-row and full-row offsets for staggered grid)

### 3.6 Quadrant Detection (0x45FE72)

Within each tile diamond, the exe determines which quadrant the mouse is in using two diagonal lines defined by floating-point constants:

```
Constants (extracted from .rdata at 0x4EB0A0):
  SLOPE_A   = -0.5    [0x4EB0A0] (double)
  OFFSET_Y  =  32.0   [0x4EB0A8] (double)
  SLOPE_B   =  0.5    [0x4EB0B0] (double)
  OFFSET_X  =  64.0   [0x4EB0B8] (double)

Diamond edge equations (within 128x64 tile):
  Line 1: y = -0.5 * x + 32     (top-left to bottom-right diagonal)
  Line 2: y =  0.5 * x          (bottom-left to top-right diagonal)

Quadrant assignment:
  if (y <= line1_y):             // above top-left diagonal
      cell_index -= 71 (0x47)    // move to cell above-left
      if (y <= line2_y): quadrant = 3 (NW)
      else:              quadrant = 4 (NE)
  else:                          // below top-left diagonal
      if (y <= line2_y): quadrant = 1 (SW)
      else:              quadrant = 2 (SE)
```

There are **4 variant paths** in the quadrant function for the 4 parity combinations of `(sub_x_grid & 1, sub_y_grid & 1)`, each using different combinations of the slope/offset constants.

The quadrant result (1-4) is stored at `0x4EEA2C`.

### 3.7 Scroll Speed Calculation (0x45FA7B area)

The scroll system uses the constant **0x253 = 595** for horizontal scroll:

```
function CalculateHScrollSpeed(mouse_x):
    x = mouse_x - 15                  // 15px dead zone
    // Complex multiply chain: x * 5 * 41 * 3 * 64 = x * 39360
    // Actually: x*(x*4+1)*8*2+x)*3*64
    speed = (computed_value) / 595     // divide by 0x253
    speed = (speed + 3) >> 2 << 2     // round to nearest 4
```

Vertical scroll uses **0x124 = 292**:
```
function CalculateVScrollSpeed(mouse_y):
    y = mouse_y - 15
    speed_raw = (y * 64 - y) * 64     // y * 63 * 64 = y * 4032
    speed = speed_raw / 292
    speed = round_to_4(speed)
```

---

## 4. Sprite System

### 4.1 Sprite File Format (FLC/FLI Container)

All sprite files (`.obj`, `.spr`, `.til`) use the **Autodesk FLC/FLI animation format**:

**File Header (132 bytes = 0x84):**
```
Offset  Size  Field
0x00    4     file_size (DWORD)
0x04    2     magic (0xAF12 for FLC, 0xAF11 for FLI)
0x06    2     num_frames (short)
0x08    2     width (short)
0x0A    2     height (short)
0x0C    2     bits_per_pixel (short, always 8)
0x0E    2     flags
0x10    4     speed (delay between frames, milliseconds)
0x14    108   reserved / padding
0x80    4     data_offset (first frame offset from file start)
```

**Magic number check at 0x4132C3:** `cmp $0xF1FA` -- note this is the FLC frame magic, not file magic. The exe checks frame-level magic, not file-level.

**Frame structure:**
```
Offset  Size  Field
0x00    4     frame_size (DWORD, includes this header)
0x04    2     frame_type (short)
```

**Frame types (from decoder table at 0x4ECEF0):**

| Type | Name | Description |
|------|------|-------------|
| 0x04 | COLOR_256 | 256-color palette (8-bit per channel) |
| 0x0B | COLOR_64 | 64-color palette (6-bit VGA, scale by 4) |
| 0x07 | DELTA_FLC | Delta-encoded frame (FLC format) |
| 0x0C | DELTA_FLI | Delta-encoded frame (FLI format) |
| 0x0D | BLACK | Clear frame to index 0 |
| 0x0F | BYTE_RUN | ByteRun1 / PackBits RLE compression |
| 0x10 | (subheader) | Frame has 2-byte subheader before data |

### 4.2 ByteRun1 (PackBits) RLE Decoding

```
for each scanline (height lines):
    bytes_remaining = width
    while bytes_remaining > 0:
        control = read_byte()
        if control > 128:
            // Run: repeat next byte (256 - control + 1) times
            run_length = 257 - control
            value = read_byte()
            fill(output, value, run_length)
            bytes_remaining -= run_length
        else if control < 128:
            // Literal: copy (control + 1) bytes verbatim
            copy_length = control + 1
            copy(input, output, copy_length)
            bytes_remaining -= copy_length
        // control == 128: NOP (skip)
```

### 4.3 Tile Sprite Indexing

**Tile sprites are accessed with `shl $0x6` (x 64) offset calculations.**

The `shl $0x6` (multiply by 64) at 0x417F24, 0x41815E, etc., computes byte offsets into a sprite metadata table at `0x67BB24`:

```
function GetSpriteInfo(sprite_slot):
    offset = (sprite_slot + 1) * 64    // shl $6 with inc
    info = *(short*)(0x67BB24 + offset)
    // info == 2 means "loaded/active"
    // info == 0 means "empty slot"
    // info == -1 (0xFFFF) means "disabled"
```

This is a **64-byte sprite descriptor** per slot, with the status word at offset +0x24 (36 bytes into the struct).

### 4.4 OBJ Sprite Files

Object sprites are loaded from per-mission files:
```
mis01.obj, mis02.obj, ... mis15.obj   // mission-specific objects
phonspr.obj                           // phone scene sprites
shark.obj                             // shark sprites
mom.obj                               // character sprites
acct.obj                              // accounting screen sprites
```

The OBJ file format is identical to the FLC container. Each frame in the FLC is one object sprite. The `object_id` byte from Cell Word 5 indexes into the frame list.

### 4.5 Tile Set Files

```
Tiles.spr    -- base terrain tile sprites
obj1.spr     -- object/decoration sprites
inven.spr    -- inventory item sprites
cursors.spr  -- cursor sprites
```

The 9-bit tile index (0-511) from Cell Word 1 selects a frame from `Tiles.spr`.

### 4.6 Transparency

**Palette index 0 is transparent.** During blitting, pixels with value 0 are skipped. The first and last palette entries (0 and 255) are reserved as system colors and not modified by `AnimatePalette`.

---

## 5. Animation System (COR Files)

### 5.1 COR File References

```
canstr.cor    -- construction/building animations
lumpy.cor     -- character animations  
misc.cor      -- miscellaneous animations
offcspr.cor   -- officer/character sprites
```

The `.cor` extension likely stands for "CORrespondence" or "COoRdinate" file -- mapping animation sequence names to frame ranges within a sprite sheet.

### 5.2 Animation Loading Functions

```
LoadAnimationData()          -- 0x4xx (error: "cound not load file")
OpenAnimationDataFile()      -- opens .cor file for reading
CloseAnimationDataFile()     -- closes file handle
LoadDataToSlot()             -- loads sprite data into numbered slot
SetupSprite()                -- initializes sprite from file
```

### 5.3 Character Sprite Structure

Characters use a compound indexing system. The array at `0x549760` is accessed with a stride of **892 bytes per character**:

```
LEA chain for character offset:
  ecx = char_index
  eax = char_index * 5       // lea (eax,eax,4)
  eax = char_index + eax * 2 // lea (ecx,eax,2) = char_index * 11
  eax = eax * 9              // lea (eax,eax,8) = char_index * 99
  eax = eax * 9              // lea (eax,eax,8) = char_index * 891
  // Then add ecx (char_index) = 892 total

Sprite data address: 0x549760 + char_index * 892 + sub_offset
```

**892 bytes per character** stores all animation frame indices, directional variants, and state data for one character on the battlefield.

### 5.4 Character Descriptor (64-byte blocks at 0x67BB24)

Each character/sprite slot uses a **64-byte descriptor** at `0x67BB24 + slot * 64`:

```
Offset  Field
+0x00   unknown (possibly position data)
+0x24   status_word (short): 0=empty, 2=active, -1=disabled
+0x26   additional state data
...
```

---

## 6. WinG Rendering Pipeline

### 6.1 Surface Architecture

The game maintains **5 WinG offscreen surfaces** for compositing:

```
Surface  Address    Purpose
1        0x68C2F0   Front buffer / display compositing
2        0x68F950   Back buffer / scene rendering
3        0x6815D0   Work buffer (tile rendering)
4        0x68FEE0   Sprite compositing
5        0x68BE90   UI overlay
```

All surfaces are 640x480 @ 8bpp (256 colors).

### 6.2 WinGBitBlt Call Sites

```
0x40E414   Primary blit: back buffer -> screen
0x40E448   Secondary blit
0x40E694   Sprite compositing
0x40E923   UI overlay rendering
0x40EBB0   Map/tile layer blit
0x4134A2   Individual sprite rendering
0x413FEC   Additional compositing pass
0x4BE68E   Final screen present
```

`WinGBitBlt` is called via thunk at `0x4D4B88` -> IAT `0x69381C`.

Signature: `WinGBitBlt(destDC, destX, destY, width, height, srcDC, srcX, srcY)`

### 6.3 Rendering Order

```
1. Clear back buffer
2. Render terrain tiles (bottom layer)
   - Iterate visible cells in painter's algorithm order
   - For each cell: draw tile_layer_0, tile_layer_1, tile_layer_2
3. Render OBJ sprites (objects on terrain)
   - Object_id from Cell Word 5 selects frame from OBJ file
   - Positioned at cell's screen coordinates
4. Render character sprites
   - From character array at 0x549760
   - Animation frame from COR mapping
5. Render overlays from Cell Word 2
6. Render UI layer
7. WinGBitBlt to screen DC (0x68FF90)
```

---

## 7. Temporary Cell Structure at 0x5EE060

All pack/unpack operations use a shared 56-byte workspace:

```
Address   Type    Pack/Unpack  Field Name
──────────────────────────────────────────────────
0x5EE060  short   Word 1       tile_layer_0 (9-bit)
0x5EE062  short   Word 1       tile_layer_1 (9-bit)
0x5EE064  short   Word 1       tile_layer_2 (9-bit)
0x5EE066  short   Word 2       overlay_tile_0 (9-bit)
0x5EE068  short   Word 2       overlay_tile_1 (9-bit)
0x5EE06A  byte    Word 2       flag_E
0x5EE06B  byte    Word 2       flag_F
0x5EE06C  byte    Word 2       flag_G
0x5EE06D  byte    Word 2       flag_H
0x5EE06E  short   Word 2       overlay_tile_2 (9-bit)
0x5EE070  byte    Word 3       terrain_base_type (8-bit)
0x5EE071  byte    Word 1       flag_A
0x5EE072  byte    Word 1       flag_B
0x5EE073  byte    Word 1       flag_C
0x5EE074  byte    Word 1       flag_D
0x5EE075  byte    Word 3       terrain_mod_11 (2-bit)
0x5EE076  byte    Word 3       terrain_mod_10 (2-bit)
0x5EE077  byte    Word 3       terrain_mod_9 (2-bit)
0x5EE078  byte    Word 3       terrain_mod_8 (2-bit)
0x5EE079  byte    Word 3       terrain_mod_7 (2-bit)
0x5EE07A  byte    Word 3       terrain_mod_6 (2-bit)
0x5EE07B  byte    Word 3       terrain_mod_5 (2-bit)
0x5EE07C  byte    Word 3       terrain_mod_4 (2-bit)
0x5EE07D  byte    Word 3       terrain_mod_3 (2-bit)
0x5EE07E  byte    Word 3       terrain_mod_2 (2-bit)
0x5EE07F  byte    Word 3       terrain_mod_1 (2-bit)
0x5EE080  byte    Word 3       terrain_mod_0 (2-bit)
0x5EE081  byte    Word 4       elev_flag_3 (2-bit)
0x5EE082  byte    Word 4       elev_flag_2 (2-bit)
0x5EE083  byte    Word 4       elev_flag_1 (2-bit)
0x5EE084  byte    Word 4       elev_flag_0 (2-bit)
0x5EE086  short   Word 4       elevation_corner_3 (6-bit, NW)
0x5EE088  short   Word 4       elevation_corner_2 (6-bit, NE)
0x5EE08A  short   Word 4       elevation_corner_1 (6-bit, SE)
0x5EE08C  short   Word 4       elevation_corner_0 (6-bit, SW)
0x5EE08E  short   Word 5       obj_param_3 (6-bit)
0x5EE090  short   Word 5       obj_param_2 (6-bit)
0x5EE092  short   Word 5       obj_param_1 (6-bit)
0x5EE094  short   Word 5       obj_param_0 (6-bit)
0x5EE096  short   Word 5       object_id (8-bit)
```

---

## 8. Key Constants Summary

| Constant | Hex | Decimal | Meaning |
|----------|-----|---------|---------|
| Tile width (pixels) | 0x80 | 128 | Full isometric diamond width |
| Tile height (pixels) | 0x40 | 64 | Full isometric diamond height |
| Diamond half-width | 0x40 | 64 | Centering offset X |
| Diamond half-height | 0x20 | 32 | Centering offset Y |
| Map columns | 0x8C | 140 | Cells per row |
| Map rows | -- | 72 | Total rows (10080/140) |
| Max cells | 0x2760 | 10,080 | Total cells in map |
| Bytes per cell array | 0x9D80 | 40,320 | 10080 * 4 |
| MAP file size | 0x3CA40 | 248,384 | Total bytes on disk |
| Neighbor offset right | 0x45 | 69 | Half-row + 1 for stagger |
| Neighbor offset down-left | 0x46 | 70 | Half-row width |
| Neighbor offset up-left | 0x47 | 71 | Half-row + 1 |
| Full row width | 0x8C | 140 | Cells per row |
| Screen width | 0x280 | 640 | Display resolution X |
| Screen height | 0x1E0 | 480 | Display resolution Y |
| Diamond slope A | -- | -0.5 | Top-left edge slope |
| Diamond slope B | -- | 0.5 | Top-right edge slope |
| Diamond Y intercept | -- | 32.0 | Edge line Y offset |
| Diamond X intercept | -- | 64.0 | Edge line X offset |
| H-scroll divisor | 0x253 | 595 | Scroll speed normalization |
| V-scroll divisor | 0x124 | 292 | Scroll speed normalization |
| Tile index mask | 0x1FF | 511 | 9-bit tile index max |
| Elevation mask | 0x3F | 63 | 6-bit height max |
| Terrain mod mask | 0x03 | 3 | 2-bit modifier max |
| Object ID mask | 0xFF | 255 | 8-bit object index max |
| Sprite slot size | 0x40 | 64 | Bytes per sprite descriptor |
| Character struct size | -- | 892 | Bytes per character entry |
| Timer interval | -- | 10ms | ~100 ticks/sec game loop |

---

## 9. Implementation Checklist for ow-data Parser

### MAP File Parser
```rust
struct MapFile {
    cell_word_1: [u32; 10080],  // tile indices + flags
    cell_word_2: [u32; 10080],  // overlay tiles + flags
    cell_word_3: [u32; 10080],  // terrain/passability
    cell_word_4: [u32; 10080],  // elevation (4 corners)
    cell_word_5: [u32; 10080],  // object references
    entity_tables: [[u8; 164]; 4],  // placement data
    map_params_a: [u8; 8],
    camera_pos: (i16, i16),     // initial camera X, Y
    scroll_bound: u32,
    tileset_refs: [u8; 62],
    scenario_data: [u8; 40044],
    waypoint_data: [u8; 6000],
    version: u16,
}
```

### Cell Unpacking
```rust
fn unpack_cell(words: [u32; 5]) -> Cell {
    let w1 = words[0];
    Cell {
        tile_0: ((w1 >> 23) & 0x1FF) as u16,
        tile_1: ((w1 >> 14) & 0x1FF) as u16,
        tile_2: ((w1 >>  5) & 0x1FF) as u16,
        flag_a: (w1 >> 4) & 1 != 0,
        flag_b: (w1 >> 3) & 1 != 0,
        flag_c: (w1 >> 2) & 1 != 0,
        flag_d: (w1 >> 1) & 1 != 0,
        // ... Word 2-5 similarly
    }
}
```

### Isometric Projection
```rust
const TILE_WIDTH: i32 = 128;
const TILE_HEIGHT: i32 = 64;
const MAP_COLS: i32 = 140;

fn cell_to_screen(cell_index: i32, camera_x: i32, camera_y: i32) -> (i32, i32) {
    let col = cell_index % MAP_COLS;
    let row = cell_index / MAP_COLS;
    let mut sx = col * TILE_WIDTH - camera_x;
    let sy = row * TILE_HEIGHT - camera_y;
    if row % 2 == 1 {
        sx += TILE_WIDTH / 2;  // stagger odd rows
    }
    (sx, sy)
}

fn screen_to_cell(sx: i32, sy: i32, camera_x: i32, camera_y: i32) -> i32 {
    let x = sx + camera_x + 64;  // +half tile width
    let y = sy + camera_y + 32;  // +half tile height
    let row = y / TILE_HEIGHT;
    let col = x / TILE_WIDTH;
    row * MAP_COLS + col
}
```
