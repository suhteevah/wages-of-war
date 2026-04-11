# WRI_FORMAT.md — Windows Write File Format (as used by Wages of War)

## Summary

The `.WRI` files in Wages of War are **standard Microsoft Windows Write documents** — the native format of the Write word processor bundled with Windows 3.x. The game uses them for mission briefings (`BRIEF##A.WRI`, `BRIEF##B.WRI`) and contract descriptions (`CONTR##.WRI`). There are 50 WRI files total, plus one `TEST.WRI` containing French-language test text.

The format is confirmed standard MS Write (not a custom game format) by:
- Magic bytes `0xBE31` at offset 0x00 (Write without OLE objects)
- Tool identifier `0x00AB` at offset 0x04 (Microsoft Write)
- 128-byte page structure matching the documented Write spec
- Font face name table containing "Arial"
- Paragraph/character property tables in the expected locations

## Why Not Just Use .DAT?

Plaintext `.DAT` equivalents exist only for missions 01-03. Missions 04-16 exist **only** in `.WRI` format, making this parser essential for extracting all mission text.

---

## File Structure Overview

A `.WRI` file is organized as a sequence of 128-byte **pages** (also called sectors). The page size is always 128 bytes — this distinguishes Write from the later Word format which uses 512-byte pages.

```
Page 0          : File header (128 bytes)
Pages 1..N      : Text body (Windows-1252 encoded, \r\n line endings)
Page N+1        : Character property (CHP) run table
Page pnPara     : Paragraph property (PAP) run table
Pages pnPara+1..: Additional PAP/CHP pages (if text overflows one page of entries)
Page pnFntb     : Font face name table (FFNTB)
                  (pnSep, pnSetb, pnPgtb all equal pnFntb in these files — no
                   section/page table data used)
```

Total file size = `pnMac * 128` bytes (always a multiple of 128).

---

## File Header (Page 0, offset 0x00-0x7F)

| Offset | Size | Field    | Value (all WoW files) | Description |
|--------|------|----------|-----------------------|-------------|
| 0x00   | 2    | wIdent   | `0xBE31`              | Write magic number. `0xBE31` = no OLE, `0xBE32` = has OLE objects. All WoW files use `0xBE31`. |
| 0x02   | 2    | dty      | `0x0000`              | Document type (always 0). |
| 0x04   | 2    | wTool    | `0x00AB`              | Creating application. `0x00AB` = Microsoft Write. (`0x0125` would be Microsoft Word.) |
| 0x06   | 2    | reserved | `0x0000`              | Reserved, always zero. |
| 0x08   | 2    | reserved | `0x0000`              | Reserved, always zero. |
| 0x0A   | 2    | reserved | `0x0000`              | Reserved, always zero. |
| 0x0C   | 2    | reserved | `0x0000`              | Reserved, always zero. |
| 0x0E   | 4    | fcMac    | varies                | **End-of-text pointer.** Byte offset (from file start) of the first byte past the text body. Text runs from `0x80` to `fcMac-1`. |
| 0x12   | 2    | pnPara   | varies                | Page number of the paragraph property (PAP) run table. |
| 0x14   | 2    | pnFntb   | varies                | Page number of the font face name table. In WoW files, also equals pnSep, pnSetb, and pnPgtb (those tables are empty/unused). |
| 0x16   | 2    | pnSep    | = pnFntb              | Page number of section properties (unused in WoW files). |
| 0x18   | 2    | pnSetb   | = pnFntb              | Page number of section table (unused in WoW files). |
| 0x1A   | 2    | pnPgtb   | = pnFntb              | Page number of page table (unused in WoW files). |
| 0x1C   | 2    | pnFfntb  | = pnFntb              | Page number of font face name table (same as pnFntb). |
| 0x1E-0x5F | 66 | reserved | all zeros            | Reserved/unused header space. |
| 0x60   | 4    | pnMac    | varies                | **Total page count.** File size = pnMac * 128. Stored as u32 but only bottom 16 bits meaningful for these small files. |
| 0x64-0x7F | 28 | reserved | all zeros            | Remainder of header page. |

### Key Relationships

```
text_start     = 0x80                    (always, page 1)
text_end       = fcMac                   (exclusive)
text_length    = fcMac - 0x80
text_pages     = ceil(fcMac / 128)       (pages consumed by header + text)
chp_page       = text_pages              (character properties immediately follow text)
pnPara         = text_pages + 1          (paragraph properties follow CHP)
file_size      = pnMac * 128
```

Note: For the largest files (BRIEF10A/B at 3968 bytes), the PAP table spans 2 pages (pnPara=0x1C, pnFntb=0x1E, so a second PAP page exists at pnPara+1). Most files have a single PAP page.

---

## Text Body (Pages 1..N, offset 0x80..fcMac-1)

The text body starts at the fixed offset `0x80` (byte 128, i.e., page 1) and extends to `fcMac - 1`. The text is encoded in **Windows-1252** with `\r\n` (0x0D 0x0A) line endings.

### Character Encoding

The text is mostly 7-bit ASCII. Non-ASCII bytes observed:

| Byte   | Windows-1252 | Actual meaning in WoW files | Unicode |
|--------|-------------|----------------------------|---------|
| `0xC6` | Æ (AE ligature) | **Right single quote / apostrophe** | U+2019 ' |
| `0xE9` | e-acute | e-acute (standard) | U+00E9 é |
| `0xE8` | e-grave | e-grave (standard) | U+00E8 è |
| `0xFB` | u-circumflex | u-circumflex (standard) | U+00FB û |
| `0xEA` | e-circumflex | e-circumflex (standard) | U+00EA ê |

**The `0xC6` apostrophe is a Write-specific encoding quirk.** In standard Windows-1252, `0xC6` is the Latin capital letter AE (Æ). But in the WoW `.WRI` files, it consistently appears in possessive contexts like `Salvatore's`, `LeClure's`, `Government's`. This must be mapped to U+2019 (right single quotation mark) or ASCII `'` during text extraction.

The companion `.DAT` plaintext files use `0x92` for the same apostrophe character (which IS the standard Windows-1252 right single quote). So the Write application used a different internal encoding for this character.

### Text Structure

Paragraphs are separated by `\r\n\r\n` (double line break). Some files begin with leading `\r\n\r\n\t` (TEST.WRI uses tab-indented paragraphs).

Contract files (CONTR##.WRI) are single-paragraph blocks of prose. Briefing files (BRIEF##A/B.WRI) contain 3-6 paragraphs covering:
1. Situation/background
2. Tactical details (map references, enemy disposition)
3. Insertion plan (time, method, location)
4. Weather/terrain notes

---

## Paragraph Property (PAP) Table (Page pnPara)

Located at offset `pnPara * 128`. Contains an array of paragraph run descriptors that record the byte offset of each paragraph boundary. Each entry is 6 bytes:

| Offset | Size | Field   | Description |
|--------|------|---------|-------------|
| 0      | 4    | fcFirst | Byte offset (from file start) where this paragraph begins. First entry is always `0x00000080` (text start). |
| 4      | 2    | props   | Paragraph properties. `0xFFFF` means "use default formatting" (all WoW files use this). |

The table contains entries for both paragraph text runs and the `\r\n` gaps between them. The final entry's `fcFirst` points just past the last text byte (≈ fcMac or fcMac+2).

The last 8 bytes of the page are a **page trailer**:

| Offset     | Size | Value    | Description |
|------------|------|----------|-------------|
| page+120   | 2    | `0x0042` | Trailer marker (66 decimal) |
| page+122   | 2    | `0x0300` | Unknown flags |
| page+124   | 2    | `0x0001` | Unknown |
| page+126   | 2    | varies   | `0x14NN` where NN encodes the entry count on this page |

For text extraction purposes, the PAP table can be **completely ignored** — the text region defined by `[0x80, fcMac)` is sufficient to extract all content.

---

## Character Property (CHP) Table (Page pnPara-1)

Located at offset `(pnPara - 1) * 128`. Describes character formatting runs (bold, italic, font changes). In the WoW files, all text uses a single formatting run with default properties, so this table contains essentially:

```
fcFirst = 0x00000080   (text start)
fcLim   = fcMac        (text end)
props   = 0x0077 0x0000 ... (default character properties)
```

The last 8 bytes are a page trailer identical in structure to the PAP page trailer.

For text extraction, the CHP table can be **completely ignored**.

---

## Font Face Name Table (FFNTB, Page pnFntb)

Located at offset `pnFntb * 128`. Contains the font names used in the document.

| Offset | Size | Field   | Description |
|--------|------|---------|-------------|
| 0      | 2    | count   | Number of font entries (always `0x0001` in WoW files) |
| 2      | 2    | cbFfn   | Byte length of font name entry (always `0x0007`) |
| 4      | 1    | ffid    | Font family ID (space character `0x20` = unknown/default) |
| 5      | N    | szFfn   | Null-terminated font name |

All WoW `.WRI` files use a single font: **Arial**.

Note: The font table page in larger files may contain residual text from the Write buffer — this is a known Write format artifact where the page was not zeroed before writing the font table. This "ghost text" should be ignored.

---

## Worked Example: CONTR08.WRI (Smallest File, 1024 bytes)

```
File size: 1024 bytes = 8 pages of 128 bytes

Header (page 0):
  wIdent  = 0xBE31 (Write, no OLE)
  wTool   = 0x00AB (Microsoft Write)
  fcMac   = 0x0238 (568) → text is 440 bytes (0x80..0x237)
  pnPara  = 0x06   → PAP table at offset 0x300
  pnFntb  = 0x07   → font table at offset 0x380
  pnMac   = 0x08   → 8 pages * 128 = 1024 bytes ✓

Text (pages 1-4, offset 0x80..0x237):
  "Mentor Finance, a world leader in corporate high-risk funding,
   wishes to hire Mercs, Inc. to terminate the following three
   individuals: Peter Hawk, Gordon Letrube, and Carlos Montrey..."

Page layout:
  Page 0 (0x000): Header
  Page 1 (0x080): Text body
  Page 2 (0x100): Text body (continued)
  Page 3 (0x180): Text body (continued)
  Page 4 (0x200): Text body (partial, ends at 0x237, rest zero-padded)
  Page 5 (0x280): CHP run table
  Page 6 (0x300): PAP run table
  Page 7 (0x380): Font face name table ("Arial")
```

---

## Text Extraction Algorithm

To extract plaintext from a `.WRI` file, only three header fields matter:

```
1. Validate magic:  bytes[0..2] == [0x31, 0xBE]  (LE u16 = 0xBE31)
2. Validate tool:   bytes[4..6] == [0xAB, 0x00]  (LE u16 = 0x00AB)
3. Read fcMac:      LE u32 at offset 0x0E
4. Extract text:    bytes[0x80 .. fcMac]
5. Decode:          Windows-1252, with 0xC6 → U+2019 (apostrophe)
6. Normalize:       Strip \r, keep \n
```

Everything after `fcMac` (CHP table, PAP table, font table, zero padding) is formatting metadata that can be completely ignored for text extraction.

### Rust Implementation

The parser lives in `crates/ow-data/src/wri.rs`. Key function: `parse_wri(path) -> Result<WriDocument, WriError>`. Returns the full text and a `Vec<String>` of paragraphs (split on `\n\n`, with hard-wraps within paragraphs collapsed to spaces).

---

## File Inventory

| File | Size | fcMac | Text bytes | Paragraphs | Content |
|------|------|-------|------------|------------|---------|
| BRIEF01.WRI | 2048 | 0x0656 | 1494 | 4 | Colombia rescue briefing (original/early) |
| BRIEF01A.WRI | 2048 | 0x065C | 1500 | 4 | Colombia rescue briefing (variant A) |
| BRIEF01B.WRI | 2048 | 0x065C | 1500 | 4 | Colombia rescue briefing (variant B) |
| BRIEF02A.WRI | 2560 | 0x0866 | 2022 | 7 | Dead Zone retrieval (variant A) |
| BRIEF02B.WRI | 2560 | 0x0866 | 2022 | 7 | Dead Zone retrieval (variant B) |
| BRIEF03A.WRI | 2048 | 0x0624 | 1444 | 5 | Mission 3 briefing (variant A) |
| BRIEF03B.WRI | 2048 | 0x0620 | 1440 | 5 | Mission 3 briefing (variant B) |
| BRIEF04A.WRI | 2944 | 0x09D7 | 2391 | 6 | Foster intelligence op (variant A) |
| BRIEF04B.WRI | 2944 | 0x09B7 | 2359 | 6 | Foster intelligence op (variant B) |
| BRIEF05A.WRI | 2944 | 0x0998 | 2328 | 5 | Mission 5 briefing (variant A) |
| BRIEF05B.WRI | 2944 | 0x09D9 | 2393 | 5 | Mission 5 briefing (variant B) |
| BRIEF06A.WRI | 2688 | 0x087D | 1917 | 5 | Mission 6 briefing (variant A) |
| BRIEF06B.WRI | 2688 | 0x089B | 1947 | 5 | Mission 6 briefing (variant B) |
| BRIEF07A.WRI | 3328 | 0x0B55 | 2773 | 6 | Mission 7 briefing (variant A) |
| BRIEF07B.WRI | 3328 | 0x0B8C | 2828 | 6 | Mission 7 briefing (variant B) |
| BRIEF08A.WRI | 2688 | 0x0876 | 1910 | 4 | Mission 8 briefing (variant A) |
| BRIEF08B.WRI | 2688 | 0x0886 | 1926 | 4 | Mission 8 briefing (variant B) |
| BRIEF09A.WRI | 2176 | 0x066A | 1514 | 4 | Mission 9 briefing (variant A) |
| BRIEF09B.WRI | 2176 | 0x066C | 1516 | 4 | Mission 9 briefing (variant B) |
| BRIEF10A.WRI | 3968 | 0x0D47 | 3271 | 9 | Mission 10 briefing (variant A) |
| BRIEF10B.WRI | 3968 | 0x0D5D | 3293 | 9 | Mission 10 briefing (variant B) |
| BRIEF11A.WRI | 2432 | 0x0786 | 1798 | 4 | Mission 11 briefing (variant A) |
| BRIEF11B.WRI | 2432 | 0x078A | 1802 | 4 | Mission 11 briefing (variant B) |
| BRIEF12A.WRI | 2944 | 0x09BE | 2366 | 6 | Mission 12 briefing (variant A) |
| BRIEF12B.WRI | 2816 | 0x0955 | 2261 | 6 | Mission 12 briefing (variant B) |
| BRIEF13A.WRI | 2048 | 0x0646 | 1478 | 4 | Mission 13 briefing (variant A) |
| BRIEF13B.WRI | 2048 | 0x0648 | 1480 | 4 | Mission 13 briefing (variant B) |
| BRIEF14A.WRI | 2816 | 0x0921 | 2081 | 5 | Mission 14 briefing (variant A) |
| BRIEF14B.WRI | 2816 | 0x093E | 2110 | 5 | Mission 14 briefing (variant B) |
| BRIEF15A.WRI | 2816 | 0x093E | 2110 | 5 | Mission 15 briefing (variant A) |
| BRIEF15B.WRI | 2816 | 0x0927 | 2087 | 5 | Mission 15 briefing (variant B) |
| BRIEF16A.WRI | 2432 | 0x07A3 | 1827 | 4 | Mission 16 briefing (variant A) |
| BRIEF16B.WRI | 2560 | 0x0822 | 1954 | 4 | Mission 16 briefing (variant B) |
| CONTR01.WRI | 3072 | 0x0A1F | 2463 | 1 | Mission 1 contract |
| CONTR02.WRI | 1536 | 0x0440 | 960 | 1 | Mission 2 contract |
| CONTR03.WRI | 1408 | 0x03BC | 828 | 1 | Mission 3 contract |
| CONTR04.WRI | 1536 | 0x0494 | 1044 | 1 | Mission 4 contract |
| CONTR05.WRI | 1920 | 0x060C | 1420 | 1 | Mission 5 contract |
| CONTR06.WRI | 2176 | 0x071E | 1694 | 1 | Mission 6 contract |
| CONTR07.WRI | 1920 | 0x05D7 | 1367 | 1 | Mission 7 contract |
| CONTR08.WRI | 1024 | 0x0238 | 440 | 1 | Mission 8 contract |
| CONTR09.WRI | 1664 | 0x04E8 | 1128 | 1 | Mission 9 contract |
| CONTR10.WRI | 1280 | 0x0338 | 696 | 1 | Mission 10 contract |
| CONTR11.WRI | 1408 | 0x03AA | 810 | 1 | Mission 11 contract |
| CONTR12.WRI | 1408 | 0x03C3 | 835 | 1 | Mission 12 contract |
| CONTR13.WRI | 1408 | 0x03AE | 814 | 1 | Mission 13 contract |
| CONTR14.WRI | 1536 | 0x0464 | 996 | 1 | Mission 14 contract |
| CONTR15.WRI | 1536 | 0x0475 | 1013 | 1 | Mission 15 contract |
| CONTR16.WRI | 1536 | 0x0477 | 1015 | 1 | Mission 16 contract |
| TEST.WRI | 3200 | 0x0AE8 | 2664 | ~8 | French-language test text (Norse mythology) |

---

## Differences from Standard MS Write

The WoW `.WRI` files are **fully standard** MS Write format. The only game-specific observation is:

1. **`0xC6` apostrophe**: The Write application encodes the right single quote (apostrophe) as byte `0xC6`, which in standard Windows-1252 would be the Latin capital letter AE (Æ). This is a known Write encoding quirk, not something specific to the game. The `.DAT` plaintext versions of the same text use `0x92` (the standard Windows-1252 right single quote).

2. **Simple formatting**: All files use a single font (Arial), no bold/italic, and default paragraph properties (`0xFFFF`). The formatting metadata exists structurally but carries no meaningful styling.

3. **All files are OLE-free**: `wIdent = 0xBE31` (no embedded OLE objects in any file).

---

## References

- Microsoft Write format: Documented in the Windows 3.x SDK and various reverse-engineering resources
- `wIdent` magic `0xBE31`/`0xBE32`: Distinguishes Write-without-OLE from Write-with-OLE
- Page size 128 bytes: Distinguishes Write from Word (which uses 512-byte pages)
- Rust parser: `crates/ow-data/src/wri.rs`
