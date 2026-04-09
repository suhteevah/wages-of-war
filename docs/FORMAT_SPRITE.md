# Sprite Container Format (.OBJ, .SPR, .TIL, ANIM .DAT)

Reverse-engineered binary format specification for the shared sprite container
used across 120+ files in Wages of War (1996).

---

## Overview

All sprite/tile/animation graphics share a single container format:

```
[File Header: 32 bytes]
[Offset Table: sprite_count * 8 bytes]
[Pixel Data Region: concatenated per-sprite blocks]
```

File types using this format:

| Extension      | Count | Content                              |
|----------------|-------|--------------------------------------|
| `.OBJ`         | 67    | UI sprites, per-scene map objects    |
| `.SPR`         | 3     | Sprite sheets (cursors, inventory)   |
| `.TIL`         | 16    | Isometric terrain tiles (512/scene)  |
| `ANIM/*.DAT`   | 34    | Character animation frames           |

---

## File Header (32 bytes)

All multi-byte integers are unsigned little-endian.

```
Offset  Size  Type   Field               Notes
------  ----  -----  ------------------  -------------------------------------
0x00    4     u32    sprite_count        Number of sprites in the file
0x04    4     u32    header_size         Always 0x00000020 (32)
0x08    4     u32    offset_table_size   = sprite_count * 8
0x0C    4     u32    pixel_data_start    = header_size + offset_table_size
0x10    4     u32    pixel_data_size     Total bytes of the pixel data region
0x14    12    bytes  reserved            Zero in .OBJ/.SPR/.TIL; non-zero in ANIM .DAT
```

**Invariants (verified across all 120+ files):**
- `header_size == 0x20` (always)
- `offset_table_size == sprite_count * 8`
- `pixel_data_start == header_size + offset_table_size`
- `pixel_data_start + pixel_data_size == file_size`

### ANIM .DAT Reserved Fields

ANIM .DAT files store extra animation metadata in the reserved region:

```
Offset  Size  Type   Observed values (GUARDDOG.DAT)
------  ----  -----  --------------------------------
0x14    4     u32    e.g. 0x000581B8 (animation-specific)
0x18    2     u16    e.g. 0x0547 (frame count or pointer)
0x1A    2     u16    e.g. 0x003A (58 — possibly direction count?)
0x1C    2     u16    e.g. 0x001E (30 — possibly frames per direction?)
0x1E    2     u16    e.g. 0x0001
```

---

## Offset Table

Starts at `header_size` (0x20), contains `sprite_count` entries of 8 bytes each.

```
Offset  Size  Type   Field
------  ----  -----  ----------------------------------------
+0      4     u32    offset   Byte offset into pixel data region
+4      4     u32    size     Total size of sprite block (header + compressed data)
```

Offsets are relative to `pixel_data_start`, NOT to the beginning of the file.

---

## Per-Sprite Block

Each sprite block begins with a 24-byte header followed by RLE-compressed pixel data.

### Sprite Header (24 bytes)

```
Offset  Size  Type   Field             Notes
------  ----  -----  ----------------  ------------------------------------------
+0      2     u16    origin_x          X position / hotspot
+2      2     u16    origin_y          Y position / hotspot
+4      2     u16    width             Sprite width in pixels
+6      2     u16    height            Sprite height in pixels
+8      2     u16    flags_a           0 or 0xFFFE in UI sprites; animation flags
+10     2     u16    flags_b           0 usually; non-zero in some ANIM sprites
+12     4     u32    compressed_size   Byte count of RLE data following this header
+16     4     u32    unknown_a         0 usually; leaked pointer in some ANIM files
+20     4     u32    unknown_b         Always 0 in observed files
```

**Invariant:** `24 + compressed_size == entry size` (from offset table).

### Origin / Hotspot

For `.OBJ` (UI) files, `origin_x` and `origin_y` appear to encode the sprite's
position on a shared sprite sheet atlas (values increase across sprites).

For `.TIL` and `CURSORS.SPR`, origin is typically `(1, 1)`.

For `ANIM` files, origin encodes the character's drawing anchor point
(e.g., `(256, 148)` for a centered 128-wide frame).

---

## RLE Pixel Compression

Pixel data is RLE-compressed 8-bit palette-indexed. Each scanline is encoded
independently and terminated by a `0x00` end-of-line marker.

### Command Bytes

| Byte value   | Name            | Size  | Action                                    |
|--------------|-----------------|-------|-------------------------------------------|
| `0x00`       | End of scanline | 1     | Fill remaining row pixels with index 0    |
| `0x01..0x7F` | RLE run         | 2     | Repeat next byte `N` times               |
| `0x80`       | Transparent skip| 2     | Emit `next_byte` transparent (0) pixels   |
| `0x81..0xFF` | Literal copy    | 1+N   | Copy next `N` bytes as literal pixels     |
|              |                 |       | where `N = byte - 0x80`                   |

### Encoding Details

**End of scanline (`0x00`):**
All remaining pixels in the current row are filled with palette index 0
(transparent). This allows rows with trailing transparency to be stored
compactly.

**RLE run (`0x01..0x7F`):**
The command byte is the repeat count (1-127). The next byte is the palette
index to repeat. Example: `0x04 0x5A` = four pixels of palette index 0x5A.

**Transparent skip (`0x80`):**
Followed by one byte giving the count of transparent pixels to emit.
Example: `0x80 0x3E` = skip 62 pixels (fill with index 0).

**Literal copy (`0x81..0xFF`):**
The count is `byte - 0x80` (1-127). The following `count` bytes are copied
verbatim as palette indices. Example: `0x84 0x4E 0x4F 0x1E 0x4B` = four
literal pixels with those index values.

### Worked Example: Isometric Tile Row

Encoding a 128-pixel-wide isometric tile row with 4 visible pixels at center:

```
Bytes:  80 3E  84 4E 4F 1E 4B  80 3E  00
        ^^^^   ^^^^^^^^^^^^^^  ^^^^   ^^
        skip   literal-4       skip   EOL
        62     pixels          62

Decodes to: [62 transparent] [4E 4F 1E 4B] [62 transparent] [2 transparent]
Total: 62 + 4 + 62 + 0 = 128 pixels (remaining filled by EOL)
```

### Palette

Sprites use 8-bit palette indices. The 256-color VGA palette is NOT stored
in the sprite container. It is embedded in the game's PCX files (last 769
bytes: `0x0C` marker + 256 RGB triplets). Extract from any full-screen PCX
such as `MAINPIC.PCX`.

---

## Observed Dimensions

| File type     | Typical width x height | Sprite count |
|---------------|------------------------|--------------|
| `.TIL`        | 128 x 63               | 512          |
| `.OBJ` (UI)   | 111-117 x 83-89       | 3-105        |
| `.OBJ` (scene) | varies               | 512          |
| `.SPR` (cursor) | 32 x 32             | 97           |
| `ANIM .DAT`   | 128 x 138 (typical)   | 536-10472    |

---

## Validation Checklist

When implementing a parser, verify:

1. `header_size == 0x20`
2. `offset_table_size == sprite_count * 8`
3. `pixel_data_start + pixel_data_size == file_size`
4. For each sprite: `24 + compressed_size == entry_size`
5. RLE decoding of each sprite produces exactly `width * height` pixels
6. RLE stream is fully consumed (all `compressed_size` bytes read)
