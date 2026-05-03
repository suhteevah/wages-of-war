# Open Wages — Cold Reassessment (2026-05-02)

## Why we've been spinning tires

Three structural lies in the codebase. Not bugs — lies. They make `cargo check` happy while the running game does nothing real.

1. **The "deep RE" is unsourced.** No Ghidra/IDA project exists anywhere on disk. Every `0x4xxxxx` address in `WOW_EXE_RE_ANALYSIS.md` and `BINARY_FORMATS_DEEP_RE.md` is uncited prose. `Wow.exe` (1,073,664 bytes, 1996-11-11 build) sits extracted at `data/extracted/Group1/Wow.exe` and **has never been opened in a real disassembler.** Sections 2.2–2.5 of the deep-RE doc (Word 2-5 cell layouts, the `0x031624` tileset table, the FLC magic claim) read like LLM-fabricated disassembly — structurally plausible, no provenance. The MAP file shape, sprite RLE, palette, and DAT-text parsers are genuinely VERIFIED-FROM-DATA. Everything that claims to come from the exe is currently fiction.

2. **The combat system is fake.** `ow-core::combat::CombatState`, `resolve_attack`, `check_suppression`, `decide_action`, `execute_action`, `find_path`, `accuracy_modifier` are all real, fully implemented, and tested — and **never called by the running game**. `MissionContext.combat: Option<CombatState>` is hardcoded `None` at game_loop.rs:756 and 1487. Live combat is two ad-hoc resolvers (`game_loop.rs:2008-2113` for player, `2338-2426` for AI) that roll `rng.gen_range(5..20)` damage in insertion order with weather hardcoded `Clear`. The "initiative-based combat with suppression" headline is a lie.

3. **Unit visuals and HUD are placeholders disguised as features.** `UnitRenderer` is a struct with one field `_private: ()` (`unit_renderer.rs:97`) — every unit on screen is a `fill_rect`. Every HUD label, button, and text element is a colored bar (`hud.rs`, `ui.rs`). The animation system (COR/DAT, `AnimController`, all four states tested) is wired but only `Idle` is ever dispatched (one site: `game_loop.rs:1085`).

These compound: each session looks at the running game, sees "buildings missing," patches the wrong layer, and moves on. The actual problems are deeper.

## What is actually correct (ground truth, keep)

- **MAP file shape** — 248,384 bytes, 5 sequential 40,320-byte cell arrays, 140×72 grid. Round-trip verified against `SCEN1A.MAP`. (`map_loader.rs`)
- **Sprite RLE** — custom format, NOT FLIC/PackBits. 32B file header, 8B offset table, 24B per-sprite header, scanline RLE with `0x80 NN` transparent runs and `0x00` EOL. Verified across 103/104 sprite files. (`sprite.rs`, `SPRITE_FORMAT_VERIFICATION.md`)
- **Palette** — `OFFPIC2.PCX` last 769 bytes is the master 8-bit palette. SCEN1 darkness is real, not a bug. (`PALETTE_ANALYSIS.md`)
- **Tile sheets** — `TILES1.DAT` (512 × 40 bytes) and `OBJ01.DAT` (512 × 48 bytes). Sizes match. **But the per-record properties are NEVER parsed.** Wall/fence/passability flags almost certainly live here.
- **DAT text parsers** — 6/6 verified against exe `sscanf` strings. (`PARSER_VERIFICATION.md`)
- **Build-path strings at MAP+0x31380** — 4×164-byte Windows paths giving each scenario's TIL/TILES.DAT/OBJ/OBJ.DAT bindings. **This is the actual TIL/OBJ binding** — no magic 0x031624 table needed.
- **Static iso projection math** — internally consistent. `tile_to_screen` round-trips with `screen_to_tile` for grid points (not arbitrary screen points). 32px row stagger is a deliberate divergence from the exe's 64px back-buffer convention; screenshots confirm no tile gaps.

## What is broken or fake (rip out or fix)

### Hard blockers (visible in screenshots)
- **Word 2 overlay pass is hard-disabled**: `game_loop.rs:3192` — `if false { ... }`. This is the dominant reason buildings/fences don't render. One line.
- **Word 2 → TIL/OBJ routing uses a magic threshold of 50** (`game_loop.rs:3247`). No RE basis. `tile_renderer.rs:264` uses 500 for the same split. Inconsistent guesses.
- **Skull markers (503-507) leak through**: filter is at `< 500` in tile_renderer but `object_id == 0 || 255` in game_loop.rs:3154. Word 5 `object_id` is u8 so 503-507 unreachable through that path; they're slipping in via Word 2 high indices.
- **`tileset_refs` (the 31×u16 table) is parsed and never consumed.** Zero references in `ow-render/`. Almost certainly hypothesized.
- **`OBJ01.DAT` properties never parsed.** 24,576 bytes of wall/passability/cover data sitting on disk, untouched.

### Structural fakes
- `CombatState` / `MissionState` never instantiated. Two parallel combat paths. Two enemy types (`ActiveMerc`, `EnemyUnit`) with a manual bridge.
- `NegotiationState` exists (counter-offers, 4 rounds, probabilistic accept) but is never imported. Contract acceptance is a click→`funds += advance` at `game_loop.rs:1466`. Debrief uses literal `let advance = 324_000i64` (`game_loop.rs:3953`).
- `HiringPool` exists, never imported. Hiring is inline at `game_loop.rs:1399-1448`.
- Mission setup (`mission_setup::setup_mission`) exists, tests pass, never called. Replaced by ad-hoc enemy generation at `game_loop.rs:1104-1144`.
- Animations: `set_action(Idle, S, 1)` is the **only** dispatch (`game_loop.rs:1085`). Walk/ShootStand/ShootCrouch/Die are implemented and tested in `anim_controller.rs:502-718` and never fired.
- Weather is hardcoded `Clear` at every mission creation site. `accuracy_modifier()` is unreachable.
- `UnitRenderer { _private: () }` — every unit is a `fill_rect`.
- All HUD/UI elements are `fill_rect` placeholders. No real text rendering.

### Doc rot
- `docs/FORMAT_MAP.md` claims 200×252; `BINARY_FORMATS_DEEP_RE.md` says 140×72. Both can't be right. Handoff endorses 140×72 and the parser uses it. **`FORMAT_MAP.md` should be deleted.**
- `BINARY_FORMATS_DEEP_RE.md` Sections 2.2–2.5 should be marked "UNSOURCED — re-verify against Ghidra." `WOW_EXE_RE_ANALYSIS.md` likewise.

### Code rot
- `game_loop.rs` is **4,252 lines** (handoff said 3,913 — already drifting). 800-line god function `run_game_loop_with_pump` (lines 396-1216). `render_combat` (3,648-3,715) is dead code — never invoked.
- Camera/iso config has three sources of truth (`game.iso_config`, `game.mission_iso`, local `mission_iso_config` in `run_game_loop`).

## The strategic move: stop guessing, start grounding

The pattern of every spinning-tires session has been: stare at the running game, hypothesize about a MAP bit layout, patch the renderer, move on. Each patch adds a magic number. None of the magic numbers are sourced from the exe. **The path forward is grounding the RE in actual disassembly + parsing the data files we already have.**

Two parallel tracks. Track A is "make it look real with what we already know." Track B is "verify or delete the unsourced RE."

---

## Phase 1 — Quick visual wins (1 session each, no new RE needed)

These ship visible improvements without committing to architectural rewrites. Order by leverage.

1. **Re-enable the Word 2 overlay pass with instrumentation.** Flip `game_loop.rs:3192` `if false` → `if true`. Add a one-shot histogram log of all `overlay_0/1/2` values in SCEN1.MAP at startup. Confirm whether values cluster <50 or are spread 0-499. If spread, the magic threshold is wrong; replace with "all overlays from TIL sheet, fall through to OBJ sheet on missing-frame." (Single biggest visible improvement: buildings start appearing.)
2. **Parse `OBJ01.DAT` and `TILES1.DAT`.** 512×48 and 512×40 byte property tables. Hex-dump first 20 records, identify wall/passability/height fields. This almost certainly resolves the wall/fence rendering question — walls in '90s iso engines are edge-attached objects with 4-direction variants gated by a properties table, not single-cell tiles.
3. **Wire the 4 animation dispatch sites.** Walk on `merc.position` change in `handle_combat_input` (~line 2104). Shoot on hit branch (~2042). Die on `enemy.current_hp == 0` (~2048) and merc HP=0 (~2381). Four function calls. The system is fully built.
4. **Replace `UnitRenderer { _private: () }` with real rendering.** `AnimController` already produces frame indices; `JUNGSLD.DAT` already parses. Wire frame → sprite blit. Units stop being rectangles.
5. **Skull marker filter unification.** Single `is_renderable_index(idx)` helper replacing the inconsistent <50/<500/`==0||==255` filters across `game_loop.rs:3154/3247` and `tile_renderer.rs:264`.

## Phase 2 — Replace the fakes with the real plumbing (1-2 sessions)

The work is mostly already done; we just have to call it.

6. **Populate `MissionContext.combat`.** At `game_loop.rs:1487` (mission start), call `mission_setup::setup_mission(...)` to build a real `MissionState` with `CombatState`. Delete the ad-hoc enemy generator at 1104-1144.
7. **Route combat through `execute_action` / `decide_action`.** Replace the ad-hoc resolvers at `game_loop.rs:2008-2113` and `2338-2426` with calls into `ow-core::actions::execute_action` and `ow-core::ai::decide_action`. Initiative ordering, suppression, weather, LOS, pathfinding all activate automatically.
8. **Route hiring/contracts through their `ow-core` modules.** `HiringPool` for the Hire phase, `NegotiationState` for Contracts. Stop the inline `funds += advance` patches; use `Ledger` transactions. The `324_000` literal in debrief becomes `accepted_contract.advance`.
9. **Delete `render_combat` (dead code, 3,648-3,715).**

## Phase 3 — Real text rendering (1 session)

10. **Bitmap font from `BUTTONS/` or `PIC/` sprite sheets.** The game has glyph atlases — the exe renders text by blitting glyphs, no TTF involved. Identify the glyph atlas, parse it, replace the placeholder bars in `hud.rs` / `ui.rs`. HUD goes from "colored boxes" to "actual game UI."

## Phase 4 — Architectural cleanup (1-2 sessions)

11. **Split `game_loop.rs`.**
    - `ow-app/src/asset_init.rs` — sprite/audio init blob from `run_game_loop` lines 439-670
    - `ow-app/src/screens/office.rs` — handlers + render (lines 1341-1702 + 2572-3063)
    - `ow-app/src/screens/deployment.rs`
    - `ow-app/src/screens/combat.rs` — once routed through `ow-core`
    - `ow-app/src/screens/debrief.rs` — lines 3776-4027
    - `ow-data/src/mission_loader.rs` — mission-map loader, lines 856-1153
    Target: `game_loop.rs` < 800 lines, dispatch only.
12. **One source of truth for iso config.** Delete `game.iso_config` and the local `mission_iso_config`; keep `mission_iso`.
13. **Unify enemies.** `EnemyUnit` → `ActiveMerc` everywhere. Drop the bridge at `mission_setup.rs:137`.

## Phase 5 — Real RE (parallel, when blocked elsewhere)

14. **Stand up Ghidra against `data/extracted/Group1/Wow.exe`.** Reuse the `J:\flir\crates\bb-re-tools\src\ghidra.rs` stub — it wraps `analyzeHeadless.bat` and is repointable in 30 minutes. New project root: `J:\wages of war\re\ghidra\` (outside `crates/` to preserve clean-room discipline).
15. **First targets in disasm:**
    - The function that consumes MAP+0x031624 (if anything does). Verify or delete that table.
    - Cell-word unpack functions. Verify Words 3/4/5 layouts. Word 4 (elevation) is all-zero in 16/16 maps — likely the parser is reading dead bits.
    - The renderer's WinG/blit dispatch. Settle the row-spacing question (32 vs 64) and the OBJ-vs-TIL routing for sure.
16. **Delete `docs/FORMAT_MAP.md`.** Mark `BINARY_FORMATS_DEEP_RE.md` Sections 2.2–2.5 and `WOW_EXE_RE_ANALYSIS.md` as "UNSOURCED — pending Ghidra verification" until step 15 is done.

## Sequencing recommendation

Start at Phase 1 step 1 today. It's a one-line diff plus a logging pass and produces an immediately visible result that tells us whether the bigger building-render problem is "we're not even running the code" or "we're running the wrong code." We currently can't distinguish those, which is the actual reason for the spinning.

Phase 2 (the combat un-faking) is the highest-impact non-visual change in the project. It's mostly deletion of ad-hoc code in favor of already-tested core code — net negative line count.

Phase 5 can run in parallel as a side track on any session where Phases 1-4 are blocked.
