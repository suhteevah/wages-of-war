# FORMAT: COR/DAT Animation System

## Overview

The Wages of War animation system uses paired `.COR` (correspondence) and `.DAT` (data) files. The `.COR` is a **plaintext** index that defines animation sequences -- actions, weapons, directions, frame counts, and sound triggers. The `.DAT` is a **binary** sprite archive containing the actual pixel data for every frame. A referenced `.ADD` file (not found on disk) presumably held overlay/additive sprite layers.

33 `.COR` files and 34 `.DAT` files exist in `data/WOW/ANIM/`. Two `.DAT` files have no matching `.COR`: `DOG.DAT` and `RIFLWALK.DAT`. One `.COR` has no matching `.DAT`: `SOLDIER1.COR` (references lowercase `soldier1.dat`).

Uses Windows-style line endings (CR/LF).

---

## COR File Structure

```
<dat_filename>                                          # Line 1
<add_filename>                                          # Line 2
<tile_size>                                             # Line 3: footprint in tiles (1=1x1, 2=2x2, 4=4x4)
[NrAnimations-action-weapon-direction-nrframes]         # Line 4: field legend (literal text)
<total_animation_count>                                 # Line 5
[<seq_number>. <human_readable_label>                   # Comment line (no closing bracket)
<f1>,<f2>,<f3>,<f4>,<f5>,<f6>,<f7>,<f8>,<f9>           # Data line (9 CSV integers)
...                                                     # Repeat for each animation
[END]                                                   # Terminator
```

### Header Fields

| Line | Content | Notes |
|------|---------|-------|
| 1 | DAT filename | Companion binary sprite data (e.g., `JUNGSLD.dat`) |
| 2 | ADD filename | Companion overlay data (e.g., `JUNGSLD.add`) -- files not present on disk |
| 3 | Tile footprint size | `1` for characters/animals (1x1 tile), `2` for trucks (2x2), `4` for helicopters/boats (4x4) |
| 4 | Field legend | Always literal `[NrAnimations-action-weapon-direction-nrframes]` |
| 5 | Animation count | Total number of data lines that follow |

### Tile Footprint (Line 3) Values

| Value | Entity Types | Examples |
|-------|-------------|----------|
| 1 | All characters, dogs, misc sprites | DSRTSLD, GUARDDOG, MISC, WOMAN, OFFCSPR |
| 2 | Trucks | TRUCK1 |
| 4 | Helicopters, boats | COPTER01, COPTER02, BOAT01 |

### Animation Entries

Each animation occupies two lines:

1. **Comment line:** `[<1-based_index>. <label>` -- human-readable description. No closing bracket. Labels encode weapon type, action, and direction (e.g., `rifle walk SW`, `dog attack NE`).

2. **Data line:** Nine comma-separated integers, no spaces.

### Terminator

`[END]` on its own line, optionally followed by blank lines.

---

## Data Line Fields (9 Fields)

```
f1, f2, f3, f4, f5, f6, f7, f8, f9
```

| Pos | Field | Values | Description |
|-----|-------|--------|-------------|
| 1 | `mirror_flag` | 1, 2 | `1` = normal. `2` = horizontally mirrored (W mirrors E). Doubles actual frame count in DAT. |
| 2 | `category` | 0, 1, 7, 8, 15, 16 | Animation category / sprite layer. See table below. |
| 3 | `action_id` | 0-203 | Action type identifier. Meaning is consistent across all soldier COR files. |
| 4 | `weapon_id` | 0-8, 60 | Weapon class for soldiers; entity subtype for non-soldiers. |
| 5 | `direction` | 0-7 | Facing direction (8 isometric directions, clockwise from South). |
| 6 | `frame_count` | 0-44 | Number of animation frames **beyond the base frame**. Actual DAT frames consumed = `frame_count + 1`. |
| 7 | `sound_id` | 0-357 | Sound effect to trigger during playback. `0` = silent. |
| 8 | `reserved` | 0 | Always zero in all observed files. |
| 9 | `playback_param` | 0, 1, 2, 3, 5, 15, 45 | Usually `1` for combat anims, `0` for SOLDIER1 (older format). Non-standard values only in OFFCSPR (office sprites). Likely playback speed/loop control. |

### Critical Formula: COR-to-DAT Frame Mapping

**Total frames in the DAT file = sum of (`frame_count + 1`) across all animation entries in the COR.**

This has been verified to produce an exact match for all 32 COR/DAT pairs:

```
For each animation entry:
    actual_frames_consumed = mirror_flag * (frame_count + 1)     # when mirror=2, double the frames
                                                                  # WRONG -- see below

Correction: mirror_flag does NOT multiply frames in the DAT.
The DAT stores (frame_count + 1) frames per entry regardless of mirror_flag.
Total DAT frames = sum(frame_count + 1) for ALL entries.
```

**Verified results (all 32 pairs match exactly):**

| COR File | Animations | Computed Frames | DAT Frames | Match |
|----------|-----------|----------------|------------|-------|
| DSRTSLD | 1120 | 10472 | 10472 | OK |
| GUARDDOG | 32 | 536 | 536 | OK |
| COPTER01 | 3 | 4 | 4 | OK |
| TRUCK1 | 9 | 41 | 41 | OK |
| MISC | 204 | 803 | 803 | OK |
| CANSTR | 1 | 1 | 1 | OK |
| WOMAN | 232 | 2864 | 2864 | OK |
| *(all others)* | | | | OK |

This means `frame_count = 0` still occupies 1 frame in the DAT (a static/single-frame sprite).

### Sequential Frame Indexing

Frames in the DAT are stored sequentially in the order the animations appear in the COR. To find the starting frame for animation entry N:

```
start_frame[0] = 0
start_frame[N] = sum(frame_count[i] + 1) for i in 0..N-1
```

### Category (f2)

| Value | Meaning | Typical Actions |
|-------|---------|-----------------|
| 0 | Movement / general action | Walk, run, kick door, throw, melee, crawl |
| 1 | Weapon fire | Ready weapon, fire, unready |
| 8 | Stance transition | Stand/kneel/prone changes, surrender, kneel/prone death |
| 15 | Death / damage | Death forward/backward, blast, bomb |
| 16 | Special attack | Dog attack only |
| 7 | (Data error) | Appears once in entry 1112 (prone death SE) across all 17 soldier COR files. Should be 8. |

### Direction Encoding (f5)

Eight isometric directions, numbered clockwise from South:

| Value | Direction | Screen Position |
|-------|-----------|-----------------|
| 0 | S | Toward camera (bottom) |
| 1 | SW | Bottom-left |
| 2 | W | Left |
| 3 | NW | Top-left |
| 4 | N | Away from camera (top) |
| 5 | NE | Top-right |
| 6 | E | Right |
| 7 | SE | Bottom-right |

### Mirror Flag (f1)

When `f1 = 2`, the engine horizontally flips the sprite at render time instead of storing separate artwork. This **always** occurs on W (dir=2) and E (dir=6) for walk/run animations, and for Carry Wounded W/E. The mirrored entry still consumes `frame_count + 1` frames in the DAT -- the mirroring is a render-time optimization that reuses the same frames as the opposite direction, but the DAT allocates space for both.

### Weapon IDs (f4) -- Soldier Entities

| ID | Weapon Class | Notes |
|----|-------------|-------|
| 0 | Rifle | Default weapon set |
| 1 | Crossbow | |
| 2 | Pistol | |
| 3 | Shotgun | |
| 4 | Heavy / BigMacGun | |
| 5 | SMG / Uzi | |
| 6 | Unarmed / Civilian | Used for no-weapon, knife, throw, melee, death, shared animations |
| 8 | Dog | Used in GUARDDOG.COR |
| 60 | Carry Wounded | Special action (entries 1113-1120 in soldier COR files) |

### Action IDs (f3) -- Complete Table

These are consistent across all soldier COR files:

| ID | Action | Category (f2) | Typical Frames |
|----|--------|---------------|----------------|
| 0 | Walk | 0 | 8 |
| 1 | Run | 0 | 8 |
| 2 | Stand to Kneel | 8 | 8 |
| 3 | Stand to Prone | 8 | 8 |
| 4 | Kneel to Stand | 8 | 8 |
| 5 | Kneel to Prone | 8 | 8 |
| 6 | Prone to Stand | 8 | 8 |
| 7 | Prone to Kneel | 8 | 8 |
| 11 | Throw (grenade) | 0 | 8 |
| 23 | Kick Door Open | 0 | 11 |
| 25 | Die Backwards #1 / Animal Die | 15 | 15 |
| 26 | Crawl | 0/15 | 15 |
| 29 | Punch | 0 | 15 |
| 30 | Kick | 0 | 15 |
| 31 | Death Forward #1 | 15 | 15 |
| 32 | Death Forward #2 | 15 | 15 |
| 34 | Death Backwards #2 | 15 | 15 |
| 35 | Blast Front | 15 | 15 |
| 36 | Bomb Back | 15 | 15 |
| 37 | Cut Fence | 0 | 15 |
| 38 | Ready Rocket (Standing) | 8 | 8 |
| 39 | Fire Rocket (Standing) | 0 | 8 |
| 40 | Ready Rocket (Kneeling) | 8 | 8 |
| 41 | Fire Rocket (Kneeling) | 0 | 8 |
| 42 | Knife Slash | 0 | 15 |
| 43 | Knife Stab | 0 | 11 |
| 44 | Kneel Throw | 0 | 8 |
| 45 | Rest Sequence (Standing) | 0 | 15 |
| 46 | Rest Sequence (Kneeling) | 0 | 15 |
| 50 | Ready Weapon (Standing) | 1 | 5 |
| 51 | Fire Weapon (Standing) | 1 | 3 |
| 52 | Unready Weapon (Standing) | 1 | 5 |
| 53 | Ready Weapon (Kneeling) | 1 | 5 |
| 54 | Fire Weapon (Kneeling) | 1 | 3 |
| 55 | Unready Weapon (Kneeling) | 1 | 5 |
| 56 | Fire Weapon (Prone) | 1 | 3-7 |
| 58 | Kneel Death | 8 | 8 |
| 59 | Prone Death | 8 | 8 |
| 61 | Animal Attack | 16 | 16 |
| 62 | Surrender | 8 | 8 |
| 99 | Destruction | 0 | 0 |

### Sound IDs (f7) -- Observed Values

| ID | Context |
|----|---------|
| 0 | Silent (most animations) |
| 10 | Walk footsteps |
| 13 | Run footsteps |
| 19 | Fax machine / office |
| 20 | Cabinet close |
| 21 | Cabinet open |
| 25 | Phone ring |
| 26 | Rolodex flip |
| 41 | Training |
| 85 | Pizza delivery |
| 92 | Dog attack |
| 93 | Dog run |
| 95 | Dog death |
| 99 | Flash/bang |
| 107 | Door kick |
| 114 | Smoke |
| 342 | Package delivery |
| 357 | Dead man |

---

## DAT Binary Format (Animation Sprite Archive)

The `.DAT` files are **binary** sprite archives containing all animation frames for the entity. They use little-endian byte ordering throughout.

### DAT File Layout

```
[Header: 32 bytes]
[Frame Offset Table: total_frames * 8 bytes]
[Frame Pixel Data: variable length]
```

### DAT Header (32 bytes)

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0 | 4 | u32 | Total number of frames in the file |
| 4 | 4 | u32 | Header size (always `32` / `0x20`) |
| 8 | 4 | u32 | Frame table size in bytes (`total_frames * 8`) |
| 12 | 4 | u32 | Frame data start offset (`32 + total_frames * 8`) |
| 16 | 4 | u32 | Total frame data size in bytes (file_size - data_start) |
| 20 | 4 | u32 | Unknown (varies per file, possibly checksum or palette ref) |
| 24 | 2 | u16 | Frame width (always `58` / `0x3A` in observed files) |
| 26 | 2 | u16 | Frame height (always `30` / `0x1E` in observed files) |
| 28 | 4 | - | Unknown (byte 28 is usually `1`, remaining bytes vary) |

**Verified header values across all DAT files:**
- Header size is always 32 bytes
- Frame dimensions are always 58x30 pixels
- `frame_table_size == total_frames * 8` (exact match verified)
- `data_start == 32 + frame_table_size` (exact match verified)
- `total_data_size == file_size - data_start` (verified via last-frame bounds check)

### Frame Offset Table

Starts immediately after the 32-byte header. Each entry is 8 bytes:

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0 | 4 | u32 | Frame data offset (relative to frame data start, NOT file start) |
| 4 | 4 | u32 | Frame data size in bytes |

Frames are packed sequentially: `frame[i].offset + frame[i].size == frame[i+1].offset` for all consecutive frames. The last frame's `offset + size == total_data_size`.

### Frame Pixel Data

Individual frame data starts at offset `data_start + frame_offset`. Frame sizes vary (typically 800-1400 bytes for 58x30 character sprites), suggesting the pixel data uses some form of compression (likely RLE or similar run-length encoding against a 256-color indexed palette). The raw uncompressed size for a 58x30 8-bit frame would be 1740 bytes; actual sizes are consistently smaller.

---

## Entity Types and Animation Counts

### Full Soldier (1120 animations)

Files: ARTICEMY, ARTICSLD, CAMBOEMY, CYBORG, DSRTEMY, DSRTSLD, FORSTSLD, GUARDEMY, JUNGEMY, JUNGSLD, LUMPY, LYBIAEMY, NIGHTSLD, RUSSNEMY, SALVITOR, SOUTHEMY, TERREMY

Structure: 7 weapon classes x ~20 actions x 8 directions = 1120 entries, consuming 10472 frames in the DAT.

Weapon animation blocks appear in order: rifle (0), crossbow (1), pistol (2), shotgun (3), heavy (4), uzi (5), unarmed (6). Each weapon block has: walk, run, stance transitions, door kick, rest, weapon fire sequences. Shared animations (death, melee, rockets, carry wounded) use weapon_id=6 and appear after all weapon blocks.

### Armed Civilian (232 animations)

Files: SCIGUY, WOMAN

Same structure as soldiers but reduced weapon set. Includes: walk, run, stance transitions, kick door, rest, throw, melee, death, rockets, knife attacks.

### Unarmed Civilian (48 animations)

Files: LABGAL, LABGUY, SUITGAL, SUITGUY, WORKER

Walk, run, stance transitions, surrender, death -- all with weapon_id=6 (unarmed).

### Guard Dog (32 animations)

File: GUARDDOG

Walk (8 dirs) + run (8) + attack (8) + die (8). Uses weapon_id=8, direction in f5. 536 frames.

### Vehicles

| File | Anims | Tile Size | Frames | Notes |
|------|-------|-----------|--------|-------|
| TRUCK1 | 9 | 2 | 41 | 8 drive directions + destroyed |
| COPTER01 | 3 | 4 | 4 | Idle + movement + destroyed |
| COPTER02 | 3 | 4 | 4 | Same structure as COPTER01 |
| BOAT01 | 6 | 4 | 8 | 2 directions (NW/NE) x 3 states |

### Misc Sprites (MISC.COR)

204 entries, 803 frames. Contains UI elements, projectiles, explosions, weather effects, map thumbnails, dialog boxes, weapon icons, markers, and many placeholder/bogus entries (reserved slots 15-71, 76-145).

### Office Sprites (OFFCSPR.COR)

17 entries, 229 frames. Office/base screen animations: coffee, fax, file cabinet, fan, rolodex, phone, magazines, pizza delivery, training, casualties. Uses non-standard f9 values (2, 3, 5, 15, 45) for playback timing.

### Static Object (CANSTR.COR)

1 entry, 1 frame. A single static canister sprite.

### Legacy/Test (SOLDIER1.COR)

72 entries, 704 frames. Older/test soldier format. Uses f9=0 instead of 1. No matching DAT file on disk. Covers only rifle weapon with basic actions.

---

## Orphan Files

| File | Notes |
|------|-------|
| DOG.DAT | Binary DAT with no matching COR. Possibly an earlier version of GUARDDOG. |
| RIFLWALK.DAT | Binary DAT with no matching COR. Isolated rifle walk animation data. |
| SOLDIER1.COR | References `soldier1.dat` (case-sensitive) which does not exist on disk. |
| PRINTME.TXT | Present in ANIM directory; likely a developer print/debug file. |

---

## Parser Implementation Notes

- All integer fields. No floating point values.
- Comment lines start with `[` but have no closing bracket (except `[END]` and the field legend on line 4).
- Strip `\r` (CR) before parsing -- files use Windows line endings.
- The animation count on line 5 must match the number of data lines.
- Empty/whitespace trailing lines may appear after `[END]`.
- Field values are never negative.
- The field legend `[NrAnimations-action-weapon-direction-nrframes]` is identical across all files.
- Entry 1112 in all 17 full-soldier COR files has `f2=7` instead of the expected `8` (prone death SE). This is a data error in the original game files -- handle gracefully.
- To compute the DAT frame index for animation entry N: sum `(frame_count + 1)` for all preceding entries 0..N-1.
- DAT frame offsets in the table are relative to the frame data start (offset 12 in DAT header), NOT relative to file start. Add `data_start` to get absolute file position.
