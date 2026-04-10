# MAP File Format Specification

Binary tile grid format for Wages of War scenario maps.

---

## Overview

All `.MAP` files are **exactly 248,384 bytes**. The format is fixed-size with no variable-length fields.

Each scenario directory (`MAPS/SCENxx/`) contains 2-4 MAP files:
- `SCENxxA.MAP` — primary/active map
- `SCENxxA0.MAP` — backup of primary
- `SCENxxB.MAP`, `SCENxxC.MAP` — alternate map variants (some scenarios)
- `SCENxx.MAP` — base template (some scenarios)

---

## File Layout

```
Offset      Size        Content
────────    ────────    ─────────────────────────────────
0x00000     201,600 B   Tile grid (200 x 252 x 4 bytes)
0x31380       656 B     String table (4 x 164-byte entries)
0x31610    46,128 B     Metadata footer
────────    ────────    ─────────────────────────────────
Total      248,384 B
```

---

## Section 1: Tile Grid (0x00000 - 0x31380)

### Dimensions

- **Width:** 200 cells
- **Height:** 252 cells (202 active + 50 border padding rows)
- **Cell size:** 4 bytes
- **Total:** 200 x 252 x 4 = 201,600 bytes

Rows 0-201 contain active map data. Rows 202-251 are border padding (byte 1 = 0xFF).

### Cell Format (4 bytes, little-endian)

```
Offset  Size  Field
0       u16   cell_flags      Object/placement data (LE)
2       u16   tile_word       Tile sprite index + flag (LE)
```

**cell_flags (bytes 0-1):**
- `0xFF00` in the high byte marks a border/unused cell
- Other values encode object placement or terrain modifiers (partially understood)

**tile_word (bytes 2-3):**
- Bits 0-14: Tile sprite index into the `.TIL` tileset (0-511 typical)
- Bit 15: Flag (possibly flip/variant/mirror)

### Common Cell Values

| Raw bytes (hex) | cell_flags | tile_index | tile_flag | Meaning |
|-----------------|------------|------------|-----------|---------|
| `00 00 00 00`   | 0x0000     | 0          | false     | Empty/default |
| `00 00 07 80`   | 0x0000     | 7          | true      | Common fill tile |
| `FF 00 00 00`   | 0x00FF     | 0          | false     | Border cell |
| `01 00 07 80`   | 0x0001     | 7          | true      | Tile 7 with flag 1 |

### Grid Layout (row-major)

Cell at position (x, y) is at file offset: `(y * 200 + x) * 4`

---

## Section 2: String Table (0x31380 - 0x31610)

Four 164-byte null-padded ASCII strings containing original build paths.

```
Entry  Offset    Content
0      0x31380   Tile sprite sheet path     (e.g. C:\WOW\SPR\SCEN1\TILSCN01.TIL)
1      0x31424   Tile metadata path         (e.g. C:\WOW\SPR\SCEN1\TILES1.DAT)
2      0x314C8   Object sprite sheet path   (e.g. C:\WOW\SPR\SCEN1\SCEN1.OBJ)
3      0x3156C   Object metadata path       (e.g. C:\WOW\SPR\SCEN1\OBJ01.DAT)
```

Each entry is a Windows-style absolute path, null-terminated, padded to 164 bytes.
These reference the asset files this map was built against in the original development environment.

---

## Section 3: Metadata Footer (0x31610 - 0x3CA40)

46,128 bytes of additional map data. **Partially understood.**

### Observed Structure

```
Offset    Size    Content
0x31610   16 B    Map parameters (viewport coords? scroll bounds?)
0x31620   16 B    Additional parameters (indices, flags)
0x31630   ~192 B  Sparse parameters (mostly zero)
0x316EE   ~45.7K  Elevation/terrain overlay grid
```

### Map Parameters (0x31610)

The first 16 bytes contain what appear to be coordinate pairs or viewport bounds:

```
Example (SCEN1A): 68 02 30 00 E7 04 6F 01 68 11 30 0F E7 13 6F 10
  Interpreted as 4x u16 LE pairs:
    0x0268, 0x0030  (616, 48)
    0x04E7, 0x016F  (1255, 367)
    0x1168, 0x0F30  (4456, 3888)
    0x13E7, 0x106F  (5095, 4207)

Example (SCEN10A): 48 01 00 01 C7 03 3F 02 48 15 80 03 C7 17 BF 04
  Interpreted as 4x u16 LE pairs:
    0x0148, 0x0100  (328, 256)
    0x03C7, 0x023F  (967, 575)
    0x1548, 0x0380  (5448, 896)
    0x17C7, 0x04BF  (6087, 1215)
```

These likely define viewport/scroll boundaries for the isometric camera.

### Elevation/Terrain Overlay (~0x316EE onward)

A grid of single-byte values overlaid on the tile grid:
- `0x01` = default/flat terrain
- `0x4C` ('L') = terrain type L
- `0x4E` ('N') = terrain type N
- `0x4F` ('O') = terrain type O

The exact grid dimensions and cell mapping for this overlay are not yet confirmed.

---

## Cross-References

Each MAP file references four companion files via the string table:

| File | Format | Content |
|------|--------|---------|
| `TILSCNxx.TIL` | Sprite container | 512 isometric tile graphics |
| `TILESx.DAT` | Binary (512 x 40B) | Tile properties (movement cost, cover, etc.) |
| `SCENx.OBJ` | Sprite container | 512 map object graphics |
| `OBJxx.DAT` | Binary (512 x 48B) | Object properties (collision, health, etc.) |

---

## Notes

- All MAP files across all scenarios are exactly the same size (248,384 bytes)
- The grid is always 200 x 252, with only 200 x 202 cells containing active data
- The tile grid uses big-endian-looking byte ordering for some values, but is actually LE u16 pairs
- The metadata footer needs further RE work, particularly the elevation overlay grid dimensions
