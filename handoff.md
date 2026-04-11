# HANDOFF.md — Open Wages → Claude Code Session

## TL;DR
Clean-room Rust reimplementation of *Wages of War* (1996). **MAP parser rewritten** with correct 140x72 grid from deep Wow.exe RE. AVI cutscenes with audio working. Soldier animation system wired up. Combat SFX and voice playback added. Terrain rendering mostly correct — compound floors visible, buildings partially rendering from OBJ sprites. Dev hotkeys for fast testing.

## Current State (2026-04-11)

### Working
- Full game loop: Office → Hire → Contract → Deploy → Fight → Win → Debrief
- MAP parser: 140x72 grid, 5 parallel cell arrays, all metadata blocks
- Staggered isometric projection (128x64 tiles, 32px half-height row spacing, odd-row +64px stagger)
- AVI cutscene playback with audio (ffmpeg-sidecar, MSRLE + ADPCM)
- MIDI music playback (M key to mute)
- Combat SFX (pistol/rifle/shotgun from SND/ WAVs)
- Voice line system (WAV playback on hire/selection)
- Video phone debrief (ACCT.OBJ portrait, PHONSPR.OBJ background)
- Soldier animation system (COR/DAT parsed, AnimController per merc, 2000 frames decoded)
- Terrain with Word 1 overlays (indices 1-499 from TIL)
- Word 2 overlays split: low indices from TIL, high from OBJ
- Window icon (Wow.ico)
- Dev hotkeys: F1-F5, F12, M

### Known Issues
1. **Building walls/fences missing** — Word 2 high-index OBJ sprites showing some elements but not walls. Need to investigate the TIL/OBJ index mapping more carefully. The tileset reference table (31 x u16 at MAP offset 0x031624) may hold the key.
2. **Skull markers** (503-507) visible as black diamonds — filtered from rendering but some still appear
3. **Path alignment** — terrain transitions slightly off at diamond edges
4. **Animation triggers** — only idle plays, walk/shoot/die not wired to game actions
5. **VLS lip-sync** — accountant portrait is static, viseme timeline not connected
6. **Voice files** — per-merc voices are inside VLS/VLA containers, not standalone WAVs

### RE Docs Completed This Session
- `docs/BINARY_FORMATS_DEEP_RE.md` — Complete Wow.exe disassembly (MAP, cells, projection, sprites, WinG)
- `docs/COR_ANIM_FORMAT.md` — Animation index + DAT sprite archive, verified across 32 pairs
- `docs/VLS_VLA_FORMAT.md` — Voice lip-sync with viseme timelines
- `docs/WRI_FORMAT.md` — Microsoft Write mission brief extraction

### Key Discoveries
- MAP grid is 140x72 = 10,080 cells (NOT 200x252)
- 5 parallel cell arrays, 4 bytes each, sequential on disk
- Word 5 object_id: 0xFF = empty sentinel (10079/10080 cells are 0xFF)
- Elevation (Word 4): all zeros across ALL 16 missions — never used by map editor
- Staggered grid needs 32px row spacing for diamond interlocking (exe uses 64px internally)
- Tile sprites are diamonds with transparent corners (palette index 0)
- TIL and OBJ both have 512 frames — Word 2 overlays may reference OBJ for buildings

### Architecture
- `ow-data/src/map_loader.rs` — Rewritten MAP parser with MapCell struct (all 5 words)
- `ow-render/src/iso_math.rs` — Staggered grid projection (tile_to_screen, screen_to_tile)
- `ow-app/src/avi_player.rs` — NEW: AVI cutscene playback via ffmpeg-sidecar
- `ow-audio/src/sfx.rs` — NEW: Combat SFX manager
- `ow-audio/src/voice.rs` — NEW: Voice line playback
- `ow-app/src/game_loop.rs` — ~4000+ lines, needs splitting

## Next Steps (Priority Order)
1. Fix building/fence rendering — investigate OBJ sprite content and tileset reference table
2. Wire animation triggers (walk/shoot/die) to game actions
3. Filter skull marker sprites (503-507) completely
4. Extract per-merc voices from VLS containers
5. Wire VLS viseme timeline to accountant portrait
6. Split game_loop.rs into sub-modules
