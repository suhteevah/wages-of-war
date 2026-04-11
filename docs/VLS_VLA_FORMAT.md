# VLS/VLA Voice Lip-Sync Format Specification

## Overview

VLS (Voice Lip-Sync) and VLA (Voice Lip-sync Alternate) files store voice line audio with embedded lip-sync timing data for character animation during dialogue. Both formats share the identical binary structure with the `VALS` magic header.

- **68 VLS files** — voice lines for mercenaries (ARTIE01-12, VINNIE01-03, PIZZAGUY, MOM01-02, PIZZA) and mission dialogue (MISHN*A/B/C)
- **44 VLA files** — alternate voice data for missions and special characters (ACCT, SHARK, WOMAN)
- Every VLS file has a corresponding `.WAV` file containing the identical raw RIFF/WAVE audio data (no lip-sync metadata)

## File Pairing

| Pattern | Description |
|---------|-------------|
| `FOO.VLS` + `FOO.WAV` | All 68 VLS files have a standalone WAV duplicate |
| `FOO.VLS` + `FOO.VLA` (identical) | Some mission files — VLA is a byte-for-byte copy |
| `FOO.VLS` + `FOO.VLA` (different) | VLA has different lip-sync timing and may have different/no audio |
| `FOO.VLA` only | 6 files (ACCT, SHARK, WOMAN — all identical; MISHN06A/B/C) |

When VLA differs from VLS, the VLA typically has more viseme entries and longer timestamps (possibly for a different voice take or language). Some VLA files are header-only (no audio data), containing just the lip-sync timeline.

## Binary Layout

All multi-byte integers are **little-endian**.

```
┌──────────────────────────────────────────────────┐
│ VALS Header                                      │
│   Magic: "VALS" (56 41 4C 53)         [4 bytes]  │
│   Header Size (offset to data section) [u32]     │
│   Viseme Entry Array                   [N × 8]   │
│     Entry 0: (viseme_id: i32, timestamp_ms: u32) │
│     Entry 1: (viseme_id: i32, timestamp_ms: u32) │
│     ...                                          │
│     Entry N-1                                    │
├──────────────────────────────────────────────────┤
│ Data Section (at offset = Header Size)           │
│   Sentinel: 0xFFFFFFFE (-2 as i32)    [4 bytes]  │
│   Sentinel: 0xFFFFFFFE (-2 as i32)    [4 bytes]  │
│                                                  │
│   WRDS Chunk (word boundary markers)             │
│     Tag: "WRDS" (57 52 44 53)         [4 bytes]  │
│     Size: byte count of entries        [u32]     │
│     Offset pairs: (start: u32, end: u32) × M    │
│                                                  │
│   Audio Chunk (optional — absent in some VLAs)   │
│     Wrapper Tag                        [4 bytes]  │
│     Wrapper Size                       [u32]     │
│     RIFF/WAVE data (standard WAV file)           │
└──────────────────────────────────────────────────┘
```

## Detailed Field Descriptions

### VALS Header (8 bytes + entry array)

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0x00 | 4 | char[4] | Magic signature `"VALS"` (0x56 0x41 0x4C 0x53) |
| 0x04 | 4 | u32 | Header size — byte offset to start of data section |

**Entry count** = (header_size - 8) / 8

### Viseme Entry (8 bytes each)

Each entry is a lip-sync keyframe:

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0x00 | 4 | i32 | Viseme ID (-1 to 23) |
| 0x04 | 4 | u32 | Timestamp in milliseconds from audio start |

Timestamps are **strictly monotonically increasing** across all entries within a file.

### Viseme ID Values

25 distinct values observed across all files. These are mouth shape (viseme) codes for character portrait animation during dialogue:

| ID | Probable Viseme | Notes |
|----|-----------------|-------|
| -1 | Silence / mouth closed | Most common (~28% of entries). Used for pauses between words |
| 0 | Rest / neutral | Second most common. Resting mouth position |
| 1-23 | Phoneme-specific mouth shapes | Maps to specific mouth/lip positions for speech animation |

The game likely uses these to drive a 2D sprite-based lip-sync system on the character portrait, selecting the appropriate mouth frame based on the current viseme ID.

Typical inter-entry timing is 3-200ms, with occasional longer gaps (up to ~280ms) for sustained phonemes.

### Sentinel Values (8 bytes)

Two i32 values, always `0xFFFFFFFE` (-2). Located at the start of the data section. Purpose uncertain — possibly marks the boundary between header and data, or serves as a version/format indicator.

### WRDS Chunk

Marks word or phrase boundaries within the audio stream as byte offset pairs into the raw PCM data.

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0x00 | 4 | char[4] | `"WRDS"` (0x57 0x52 0x44 0x53) |
| 0x04 | 4 | u32 | Data size in bytes |
| 0x08 | N×4 | u32[] | Array of byte offsets into PCM audio data |

The offsets are read as **pairs**: (start_byte, end_byte) for each word/phrase segment.

- **Segment count** = data_size / 8
- Offsets are byte positions within the `data` chunk of the embedded WAV
- At 22050 Hz 8-bit mono, each byte = one sample = ~0.0454 ms
- Typical segment duration: 26ms to 583ms (individual words/short phrases)
- First segment sometimes has start=end=0 (placeholder/unused)

### Audio Chunk (Optional)

Present in all VLS files and some VLA files. Absent (header-only) in 28 VLA files.

The audio is wrapped in a container with variable tag:

| Wrapper Tag | Occurrences | Description |
|-------------|-------------|-------------|
| `"WAVE"` (0x57415645) | 68 files | Most common wrapper |
| `"RIFF"` (0x52494646) | 10 files | Alternative wrapper |
| `\x00\x00\x00\x00` | 4 files | Null tag wrapper |

In all cases, the structure is:

```
[Wrapper Tag]  [4 bytes]  — "WAVE", "RIFF", or 0x00000000
[Wrapper Size] [4 bytes]  — u32, total size of wrapped content
[Standard RIFF/WAVE file] — begins with "RIFF" tag
```

The wrapper size = inner RIFF file size + 0 (wrapper size accounts for the inner RIFF exactly). The wrapper tag variation appears to be inconsequential — the engine likely just skips 8 bytes and reads the RIFF data regardless.

### Embedded Audio Format

All audio across all VLS/VLA files uses identical encoding:

| Parameter | Value |
|-----------|-------|
| Format | PCM (format code 1) |
| Channels | 1 (mono) |
| Sample Rate | 22,050 Hz |
| Bits per Sample | 8 (unsigned) |
| Byte Rate | 22,050 bytes/sec |
| Block Align | 1 |

Audio durations range from ~1.5 seconds (short acknowledgments) to ~46 seconds (long mission briefings).

## Standalone WAV Files

Every `.VLS` file has a corresponding `.WAV` file in the same directory. The WAV file is a **byte-for-byte copy** of the inner RIFF/WAVE data embedded in the VLS (after stripping the VALS header, sentinels, WRDS chunk, and audio wrapper). The WAV files exist as a convenience — the game could play audio from either source.

## File Categories

### Mercenary Voice Lines (ARTIE01-12, VINNIE01-03, MOM01-02, PIZZA, PIZZAGUY)

VLS-only (no VLA). Named after characters. Contain individual voice clips:
- ARTIE = character named Artie (12 clip sets)
- VINNIE = character named Vinnie (3 clip sets)  
- MOM = character's mother (2 clip sets)
- PIZZA/PIZZAGUY = pizza delivery character

### Mission Dialogue (MISHN*A/B/C)

Named `MISHN{NN}{suffix}` where NN = mission number (1-16), suffix = A/B/C (or I for MISHN15I). The A/B/C likely represents different dialogue branches or speakers within a mission.

Missions 1, 14 have identical VLS/VLA pairs. Others have differing VLA data or VLA-only.

### Special Characters (ACCT, SHARK, WOMAN — VLA only)

Three VLA files that are **byte-for-byte identical** to each other (293,502 bytes). These share the same voice data, suggesting they use a common voice actress with different character names.

## Parsing Algorithm

```
1. Read magic "VALS" (4 bytes) — verify signature
2. Read header_size (u32)
3. entry_count = (header_size - 8) / 8
4. Read entry_count × (viseme_id: i32, timestamp_ms: u32)
5. Seek to header_size
6. Read two i32 sentinels — verify both are -2
7. Read "WRDS" tag (4 bytes) + size (u32)
8. Read size bytes of WRDS data as u32 pairs: (start_byte, end_byte)
9. If file has remaining data:
   a. Skip wrapper tag (4 bytes) + wrapper size (u32)
   b. Read standard RIFF/WAVE file to end of file
```

## Development Context

The WAV directory also contains leftover development files:
- `FITZ.TMP`, `FITZ.BAK` — file lists referencing `Q:\SND_DEPT\WAGES\` (developer sound department network path)
- `WAGES.TXT` — numbered list of WAV file paths from the sound asset pipeline
- `MISHN11A.TXT`, `MISHN11B.TXT` — raw dialogue scripts used to record voice lines
- `SPEECH01.DAT` (in DATA directory) — dialogue transcript for mission 01

These artifacts suggest the VLS/VLA format was produced by an in-house tool that:
1. Imported recorded WAV audio
2. Generated lip-sync viseme data (possibly manually or via phoneme analysis)
3. Marked word boundaries (WRDS) for subtitle display timing
4. Packaged everything into the VALS container format

## Clean-Room RE Notes

- Format derived entirely from binary observation of 112 files (68 VLS + 44 VLA)
- No code was decompiled or disassembled
- Viseme ID meanings (which mouth shape each number represents) are inferred from the lip-sync context but not individually verified against game rendering
- The WRDS chunk purpose as "word boundaries" is inferred from the name and the timing characteristics of the segments
