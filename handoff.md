# HANDOFF.md — Open Wages → Claude Code Session

## TL;DR
You're picking up a clean-room Rust reimplementation of *Wages of War: The Business of Battle* (1996). Phases 1 and 2 are complete — all text game file formats are documented and parsed. **Current work: Phase 3 (ow-core game rules), sprite container RE, and OXCE architecture study.**

## What's Done

### Phase 1: Data Reconnaissance — COMPLETE
- [x] Game ISO obtained and extracted to `data/WOW/` (gitignored)
- [x] Survey tool run against all 1254 files — classified as text/binary/mixed
- [x] 10 format specification documents in `docs/FORMAT_*.md`
- [x] Binary formats triaged with entropy/struct analysis
- [x] Key discovery: .OBJ/.SPR/.TIL/ANIM .DAT all share ONE sprite container format (120+ files)

### Phase 2: Data Parsers — COMPLETE
- [x] 12 strongly-typed Rust parsers in `ow-data` crate — **87 tests, all passing**
- [x] Parsers: mercs, weapons, equip, strings, mission, ai_nodes, moves, shop, buttons, animation, target, textrect
- [x] Full game data validator (case-insensitive, checks 69+ required files)
- [x] Rust RE tools (survey + triage) in `ow-tools` crate — replaced Python

### Infrastructure
- [x] Full Cargo workspace scaffold (6 crates: ow-data, ow-core, ow-render, ow-audio, ow-app, ow-tools)
- [x] CLAUDE.md, README.md with full FOSS stance (dual MIT/Apache-2.0, no monetization ever)
- [x] Three `.skill` files with deep RE/parsing/isometric engine reference material
- [x] GitHub repo live at `suhteevah/open-wages`

## In Progress

### Phase 3: Core Rules (ow-core crate)
- [ ] Runtime mercenary structs (ActiveMerc, MercStatus)
- [ ] Initiative-based combat system (TurnOrder, CombatState)
- [ ] Damage resolution + suppression system
- [ ] Weather effects on combat
- [ ] Game state machine (Office → Travel → Mission → Debrief)
- [ ] Pathfinding (A* on isometric grid)
- [ ] Line of sight (Bresenham)
- [ ] Economy (contracts, payments, reputation)
- [ ] AI decision trees

### Binary Format RE
- [ ] Sprite container format (.OBJ/.SPR/.TIL/ANIM .DAT) — P1 priority, unlocks all visual assets
- [ ] MAP format (fixed 248,384 bytes, tile grid)
- [ ] PCX palette extraction (master 256-color VGA palette)
- [ ] VLA/VLS audio ("VALS" magic, embedded WAV + subtitle timing)
- [ ] WRI format (Windows Write — needed for mission 4-16 briefing text)

### Architecture
- [ ] OXCE (OpenXCOM Extended) architecture study — ruleset system, mod support, rendering, saves

## What's NOT Done — Future Phases

### Phase 4: Renderer (ow-render crate)
1. Isometric tile rendering (diamond projection, 64×32 tiles)
2. Sprite rendering with palette
3. Camera (scroll, zoom)
4. HUD/UI panels
5. Animation state machine

### Phase 5: Integration (ow-app)
1. Wire data→core→render
2. Mission flow: deploy→fight→extract
3. Office/strategic layer UI
4. Save/load
5. Sound

## Key Technical Facts
- **The .dat files are plaintext.** Editable with Notepad++. The data layer is label-based text, not packed binary.
- **57 mercenaries** with full stat blocks, bios, and 3-tier fee structures.
- **57 weapons** across 14 categories with penetration/damage/range/AP cost.
- **16 missions** with 14-section definition files (contracts, enemy rosters, weather, AI).
- **AI behavior scripts** with 6 alert escalation levels and 8 action codes.
- **3 arms dealers** (SERG, ABDULS, FITZ) with per-mission stock cycling.
- **Hit probability table** (TARGET.DAT) — 100+ row x 20 column lookup, core of combat math.
- **Combat is initiative-based**, NOT I-go-you-go. All units sorted by (EXP + WIL) each round.
- **Suppression is a core mechanic** — incoming fire reduces AP even on a miss.
- **Weather matters** — affects accuracy, sight range, smoke grenades.
- **Strategic office layer** (hiring, equipment, contracts, intel) + tactical mission layer.
- **Isometric diamond projection**, standard 2:1 tile ratio.

## Environment
- **Machine:** kokonoe (i9-11900K, RTX 3070 Ti, 64GB, Win11)
- **Toolchain:** Rust stable (all code is Rust, no Python)
- **IDE:** VS Code / Claude Code
- **RE tools available:** Ghidra 11.x, x32dbg, HxD, PE-bear, Process Monitor, Strings (Sysinternals)
- **SDL2:** Install via vcpkg or pre-built binaries
- **Reference project:** OpenXCOM Extended (OXCE) — architectural model

## File Layout
```
open-wages/
├── CLAUDE.md              # Project soul document — read this first
├── README.md              # Public-facing readme
├── HANDOFF.md             # This file
├── Cargo.toml             # Workspace root
├── crates/
│   ├── ow-core/           # Game logic (combat, economy, AI)
│   ├── ow-data/           # Original file parsers (12 modules, 87 tests)
│   ├── ow-render/         # Isometric renderer
│   ├── ow-audio/          # Sound/music
│   ├── ow-app/            # Main binary
│   └── ow-tools/          # RE helper binaries (survey, triage)
├── docs/                  # 10+ format specs, architecture notes
│   ├── FORMAT_MERCS.md
│   ├── FORMAT_WEAPONS.md
│   ├── FORMAT_EQUIP.md
│   ├── FORMAT_MSSN.md
│   ├── FORMAT_AI_NODES.md
│   ├── FORMAT_TEXT.md
│   ├── FORMAT_COR.md
│   ├── FORMAT_BTN.md
│   ├── FORMAT_CONTRACTS.md
│   ├── FORMAT_BINARY_SURVEY.md
│   └── architecture.md
├── assets/                # Placeholder/test assets only
└── skills/                # Reference .skill files
    ├── win95-binary-re.skill
    ├── game-dat-parser.skill
    └── isometric-engine.skill
```

## GitHub
Repo at `suhteevah/open-wages`. Public from day one — FOSS preservation project, no monetization.
