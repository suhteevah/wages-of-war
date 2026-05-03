# HANDOFF.md — Open Wages → Claude Code Session

## 2026-05-03 — wall structure decoded, wall-rendering pass landed

**Last Updated:** 2026-05-03
**Project Status:** 🟡 wall data is now extracted from Wow.exe disasm and a debug rendering pass is wired up; needs a visual verification pass to confirm the segment-to-slot mapping.

### What Was Done This Session

Continuation of 2026-05-02 MVP push. After Matt corrected my overclaim about Word 2 buildings rendering, pivoted to actual disasm-driven RE.

**Ghidra harness extended** (`re/ghidra/scripts/`):
- `FindWagesArtifacts.java` — first-pass artifact extractor (1,158 strings, 2,599 functions, 143 imports, 300 data-file-related strings with xrefs).
- `DecompKeyFunctions.java` — decompiles a curated list of high-value functions plus every caller of `WinG*` / `RealizePalette` / `AnimatePalette` / `GetOpenFileNameA`.
- `WallHunt.java` — searches all defined strings for wall/fence/edge/Pass/etc. patterns and decompiles every caller. Found `"Can't Pass Wall: "` at 0x004ed658 (caller `FUN_0041d0f5`) and `"In GridPass: Tile is: "` at 0x004ed678 (caller `FUN_0041d2c6`).
- `WallStructure.java` — decompiles `FUN_0041b26d` (the cell-data unpacker) and the four tile-neighbor helpers (`FUN_0041bdeb/bc69/bf6d/bae7`) plus `FUN_0041c81c` (grid-direction → wall-slot translator).

**Disasm findings** (cited from `re/ghidra/projects/analysis/decomp/wallstruct_FUN_0041b26d.c`):
- Cell data lives at five known addresses, indexed by `cell_index * 4`:
  - `0x0059d8c0` → Word 1
  - `0x005a7640` → Word 2
  - `0x005d07c0` → Word 3 ← **the walls**
  - `0x005da540` → Word 4
  - `0x005e42c0` → Word 5
- **Word 3 layout (the breakthrough):** byte 0 = property byte (movement cost / terrain class), bits 8-31 = **TWELVE 2-bit wall slots** (direction 1 at the high end, direction 12 at the low end). The Rust parser already extracts these into `MapCell.terrain_mods: [u8; 12]` (`map_loader.rs:626`) — the data has been parsed all along; nothing rendered it.
- **Each cell has 4 sub-grids** (NW/NE/SE/SW quadrants — `FUN_0041c81c(grid, dir)` switch on grid 1-4). Up to 4 units per cell. Movement is grid-to-grid; walls block both interior (between quadrants of same cell) and exterior (cell-edge) movement.
- **GridPass cardinal moves sample 8 walls across 4 tiles** (current cell + 3 neighbors); diagonal moves sample 2.
- Word 4 layout = 4×6-bit + 4×2-bit. Currently parsed as "elevation" but the semantic interpretation (corner heights) isn't proven; the bit positions are right.
- Word 5 layout = 8-bit object_id + 4×6-bit. Current Rust only reads the 8-bit; the four 6-bit fields are extracted but not yet consumed.

**Wall-rendering pass added** at `game_loop.rs:3572-3672`:
- Iterates visible cells, skips those with all-zero `terrain_mods`.
- Computes diamond corners (N/E/S/W cardinal points + center + 4 edge midpoints) in screen space.
- Draws colored line segments for each non-zero wall slot:
  - Slots 1-8 = perimeter halves (clockwise from N)
  - Slots 9-12 = interior spokes from each cardinal corner to cell center
  - Color: green = value 1, yellow = value 2, red = value 3
- Slot-to-segment mapping is a **working hypothesis**, not yet validated against `FUN_0041c81c`.

**Earlier in same session (still 2026-05-02 by clock; entry below has details):**
- Word 2 overlay `if false` → `if true` flip (modest visual delta — see honesty correction in 2026-05-02 entry).
- Walk/Shoot/Die animation triggers wired with idle-revert.
- Player damage now weapon-driven via `ruleset.weapons` lookup.
- Auto-screenshot loop env-gated by `OW_AUTO_SCREENSHOT_MS`.

### Current State

**Working:**
- Full mission loop (Office → Hire → Contract → Deploy → Combat → Win → Debrief).
- Floor terrain, OBJ Word 5 sprites, Word 2 overlays, soldier sprites, animations, weapon-aware player damage, weapon-aware AP cost, range-checked shots.
- Auto-screenshot dev loop (env-gated, gitignored output).
- Ghidra workbench operational against `Wow.exe` at `re/ghidra/projects/wages-re.gpr`.
- 22 functions decompiled and saved as readable C under `re/ghidra/projects/analysis/decomp/`.

**Newly added (needs visual verification):**
- Wall rendering pass — should draw colored lines for `terrain_mods != 0` cells. Has not yet been confirmed visually.

**Stubbed / broken / incorrect:**
- AI damage still `rng.gen_range(3..15)` because enemy weapons in `mission_setup` are stored as `"Weapon_{idx}"` placeholders that don't resolve against `ruleset.weapons`.
- `MissionContext.combat: Option<CombatState>` is still `None` — full ow-core combat plumbing (initiative, suppression, weather, LOS) deferred.
- Black-diamond artifacts in late combat screenshots — possibly skull leakage past `>=500`, possibly empty-cell renders.
- Green vertical glitch line on the accountant portrait during debrief.
- `UnitRenderer { _private: () }` in `ow-render/` is dead code; real rendering happens inline in `render_mission_map`.

### Blocking Issues

None. Next session can iterate freely on the wall-rendering visual.

### What's Next

Ordered by value:

1. **Visual verification of the wall pass.** Run with auto-screenshot, look at combat frames, decide which case applies:
   - Walls appear roughly in the right places → tune segment-to-slot mapping by rotating the array.
   - Walls appear offset/wrong direction → rotate.
   - Walls appear but interior spokes 9-12 are wrong → re-derive that half of the layout from `FUN_0041c81c`.
   - No walls → screen-space math bug.
2. **Transcribe `FUN_0041c81c` fully** to lock in the canonical slot-to-edge mapping. The decomp is already in `re/ghidra/projects/analysis/decomp/wallstruct_FUN_0041c81c.c` — just needs careful translation.
3. **Upgrade walls from line-segments → sprite blits.** Walls almost certainly have a sprite atlas (TIL-low-index? OBJ-high-index? a separate wall sheet?). Find via xref of the wall-rendering function in Wow.exe (we have GridPass + the wall accessors but not the wall *renderer* yet — needs another decomp pass).
4. **Word 5's 4×6-bit fields** — currently extracted but never used. Could be vegetation, additional sprite slots, or sub-grid object placement. Investigate.
5. **AI weapon hookup** — needs a parallel `Vec<Weapon>` indexed lookup or rewrite `EnemyUnit` to carry a resolved weapon name. See marker comment at `game_loop.rs:2625`.
6. **Full ow-core combat routing** — populate `MissionContext.combat`, replace ad-hoc resolvers with `execute_action` / `decide_action`. Net negative LOC.
7. Cosmetics — debrief portrait green-line, black-diamond artifact investigation.

### Notes for Next Session

- **Wall slot indexing convention:** `terrain_mods[0]` = wall slot 1 in the disasm (highest 2 bits at >>30); `terrain_mods[11]` = slot 12 (lowest at >>8). The renderer assumes this in the segments array — preserve.
- **The 4 sub-grid concept changes the model.** Cells aren't atomic; each holds a 2×2 sub-grid plus 12 walls. Up to 4 units fit per cell. Pathfinding/movement should ultimately work in (tile, grid) coordinates, not just (tile). Out of scope for now but informs future work.
- **`FUN_0041c81c` lookup table** (from disasm): 4×4 grid×direction → slot 1-12. Use this to confirm/derive any geometric interpretation. The exact mapping is in `re/ghidra/projects/analysis/decomp/wallstruct_FUN_0041c81c.c`.
- **Don't trust the "Word 4 = elevation" interpretation** without independent evidence. The bit positions match a 4×6-bit + 4×2-bit layout, but the semantic interpretation might be cover percentages, light levels, or sub-grid heights rather than per-corner ground elevation. Word 4 is all-zero in 16/16 ship maps, so currently unfalsifiable.
- **Auto-screenshot is env-gated.** Set `OW_AUTO_SCREENSHOT_MS=1500` to capture every 1.5s into `dev-screenshots/run-<unix-ts>/ss_NNNNN_<phase>.bmp`. Gitignored. Helpful for next session if it needs to see what's actually rendering without running the game directly.
- **Ghidra workbench location:** `C:\Tools\ghidra_12.0.4_PUBLIC\`. Project at `J:\wages of war\re\ghidra\projects\wages-re.gpr`. Re-run any decomp script via `analyzeHeadless.bat <projDir> <projName> -process Wow.exe -scriptPath <scriptDir> -postScript <Name>.java -noanalysis`. **Java scripts only — Ghidra 12 dropped Jython.**
- **Recurring Semgrep false positive** at `game_loop.rs:1086` (the OFFPIC2.PCX asset loader's `read_dir`). Not Actix, not network input. The CLAUDE.md / hooks should ignore this rule for desktop-game crates.

---

## 2026-05-02 — MVP unstuck

Big session. Project went from "stuck spinning, half-assed" to a playable mission loop with real visuals, real animations, and weapon-driven player combat. Five concrete moves:

1. **Re-enabled the Word 2 overlay pass** at `game_loop.rs:3192` (was `if false`). The pass now runs and emits a one-shot histogram on first frame (176 unique overlay indices, 2,066 total occurrences, skull markers 503-507 cleanly clustered and filtered). The visual delta is **modest, not transformative** — what was already rendering as compound floors via the Word 5 OBJ pass still renders, and the Word 2 pass adds incremental decoration, but **walls and fences are still mostly absent**. The original handoff blocker stands. Investigation continues via Ghidra (see below).
2. **Wired Walk / ShootStand / Die animation triggers.** State-watcher in the per-frame loop diffs each merc's position / hp / ap against a previous-frame snapshot and dispatches `set_action` accordingly. ShootStand reverts to Idle on `is_finished()`; Walk reverts after a 24-frame grace period (since in-game movement is teleport-based, not interpolated).
3. **`UnitRenderer { _private: () }` is dead code.** Real unit rendering already lives in `render_mission_map` and uses `AnimController.current_frame_index()` to look up `soldier_textures[i]`. The audit was wrong — the placeholder struct in `ow-render/` is just unused.
4. **Player damage is now weapon-driven.** Replaced `rng.gen_range(5..20)` with a lookup of the merc's equipped weapon in `ruleset.weapons`. Damage scales with `weapon.damage_class` (±25% jitter), AP cost from `weapon.ap_cost`, and shots beyond `weapon.weapon_range` are forced misses with a clear log message and no AP burn.
5. **Ground-truth RE infra installed.** Ghidra 12.0.4 at `C:\Tools\ghidra_12.0.4_PUBLIC\`. `Wow.exe` imported into a fresh project at `J:\wages of war\re\ghidra\projects\wages-re.gpr`. First-pass artifact extraction (`re/ghidra/scripts/FindWagesArtifacts.java`) produced JSON for 1,158 strings, 2,599 functions, 143 imports across 7 DLLs, and 300 data-file-related strings with xrefs. Output at `re/ghidra/projects/analysis/`. **Major finding: Wow.exe contains a built-in scenario/tile/object editor** (file-open dialog filter `Tile Map (*.map)|*.map|...`). Renderer is WinG (5 functions: `WinGBitBlt`, `WinGCreateBitmap`, `WinGCreateDC`, `WinGRecommendDIBFormat`, `WinGSetDIBColorTable`) — no DirectX. Audio is `mciSendCommandA` for MIDI, `sndPlaySoundA` for WAVs.

Dev infrastructure added:
- **Auto-screenshot loop** — set `OW_AUTO_SCREENSHOT_MS=1500` and dump every 1.5s to `dev-screenshots/run-<unix-ts>/ss_NNNNN_<phase>.bmp`. Phase tag in filename means combat frames are findable in a 200-shot run. Gitignored.
- **Reassessment doc** at `REASSESSMENT_2026-05-02.md` — brutal "what's real / what's fake" audit of every subsystem.
- **Honesty checkpoints** prepended to `flir/HANDOFF.md` and `flightsuite/HANDOFF.md` — cross-project audit found that the assumed Ghidra workbench had never been used; specs are hand-authored from datasheets, not disasm-derived.

## What still needs doing (post-MVP)

Visible:
- **Deployment view shows a black void in the lower-left** of mission 1. Probably just the camera defaulting near the edge of the 140x72 map; probably not a bug. Confirm before fixing.
- **Black-diamond artifacts in late combat** (visible in `dev-screenshots/run-1777774109/ss_00055_*`). Could be skull-marker leakage past `>=500`, could be unit shadows, could be empty-cell renders. Quick instrumentation pass would settle it.
- **Green vertical glitch line on the accountant portrait** during debrief. Palette or scanline issue. Cosmetic.

Invisible-but-fake:
- **AI damage is still `rng.gen_range(3..15)`.** Enemy weapons in `mission_setup::EnemyUnit::from_rating` are stored as `"Weapon_{idx}"` placeholders that don't resolve against `ruleset.weapons` (which is keyed by name). Wiring requires either a parallel `Vec<Weapon>` indexed lookup or rewriting EnemyUnit to store the resolved name. Search `game_loop.rs:2625` for the marker comment.
- **`MissionContext.combat: Option<CombatState>` is still `None`.** Full ow-core combat plumbing (initiative, suppression, weather, LOS, real `setup_mission`) was deferred — the simpler weapon-driven damage swap covers 80% of the visible delta. Real ow-core routing is the right next refactor and is partly written: `setup_mission`, `execute_action`, `decide_action`, `find_path`, `resolve_attack`, `check_suppression`, `accuracy_modifier`, all real and tested in `ow-core`.

RE follow-ups (requires Ghidra now installed):
- Decompile the function that reads MAP+0x031624 if any function references it. Verifies or kills the "31×u16 tileset reference table" claim.
- Decompile cell-word unpack functions to confirm Word 3/4/5 layouts (Word 4 elevation is all-zero in 16/16 maps; could be parser misreading dead bits).
- Decompile the WinG blit dispatch to settle the row-spacing question (32 vs 64 px).
- `BINARY_FORMATS_DEEP_RE.md` Sections 2.2-2.5 should be marked unverified at the top until the above is done.

---

## TL;DR (pre-2026-05-02)
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
