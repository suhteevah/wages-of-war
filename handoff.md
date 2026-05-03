# HANDOFF.md — Open Wages → Claude Code Session

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
