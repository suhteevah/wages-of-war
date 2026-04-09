# CLAUDE.md — Open Wages Engine

## Identity
**Open Wages** is a clean-room, open-source Rust reimplementation of the engine for *Wages of War: The Business of Battle* (1996, Random Games / New World Computing / 3DO). All three companies are defunct. This follows the **OpenXCOM model**: we build a modern engine that reads the original game's data files. No copyrighted code or assets are redistributed.

## Project Principles
1. **Rust-only house.** All code is Rust — engine, tooling, everything. No Python.
2. **Verbose logging everywhere.** Every crate uses `tracing` with `RUST_LOG` env filtering. No silent failures. Log file reads, parse results, state transitions, combat rolls — everything.
3. **Clean-room only.** We document *behavior*, never copy code. Data format specs are derived from black-box observation of file contents + runtime behavior.
4. **Original data required.** The engine validates that the user has supplied their own copy of the original game files at startup. We never bundle or redistribute original assets.
5. **Workspace crate architecture.** Separation of concerns: `ow-data` (parsers), `ow-core` (game rules, no rendering), `ow-render` (isometric renderer), `ow-audio` (sound), `ow-app` (main binary, event loop).

## Tech Stack
- **Language:** Rust (2021 edition, stable toolchain)
- **Rendering:** SDL2 via `sdl2` crate (prototyping) → potential `wgpu` migration later
- **Audio:** SDL2_mixer or `rodio`
- **Serialization:** `serde` + `serde_json` for save files, custom parsers for original .dat files
- **Pathfinding:** `pathfinding` crate (A*)
- **Error handling:** `anyhow` (app) + `thiserror` (libraries)
- **Logging:** `tracing` + `tracing-subscriber` with env-filter
- **Binary parsing:** `byteorder` for LE integer reads

## Architecture

```
open-wages/
├── Cargo.toml                 # Workspace root
├── crates/
│   ├── ow-data/               # Original game file parsers (.dat, sprites, maps, audio)
│   ├── ow-core/               # Game state, combat, economy, AI — zero rendering deps
│   ├── ow-render/             # Isometric tile/sprite renderer, camera, UI
│   ├── ow-audio/              # Sound/music playback
│   └── ow-app/                # Main binary: window, event loop, wires everything together
├── crates/ow-tools/           # RE helper binaries (survey, triage) — pure Rust
├── docs/                      # Format specs, game mechanics docs, architecture notes
├── assets/                    # Placeholder/test assets ONLY (never original game files)
└── skills/                    # Claude skills (.skill files for future sessions)
```

## Key Game Mechanics to Implement
- **Initiative-based combat**: All units (player + enemy) sorted by initiative score each round. NOT IGOUGO.
- **Suppression system**: Incoming fire can suppress units even on a miss, reducing AP and initiative.
- **Weather effects**: Rain, fog, night, sandstorm — affect accuracy, sight range, smoke grenades.
- **Economy layer**: Hire mercs, buy equipment, accept contracts, manage profit/reputation.
- **Isometric rendering**: Diamond projection, 2:1 tile ratio (64×32 typical), painter's algorithm draw order.

## Data Files
The original game's `.dat` files are **plaintext** (confirmed text-editable with Notepad++). This is a massive simplification — the data layer is INI/CSV-style, not packed binary. Sprite/map data may still be binary.

Key data files to expect:
- Mercenary roster & stats
- Weapon/equipment definitions
- Mission parameters & objectives
- Map/terrain tile layouts (likely binary)
- AI behavior parameters
- Economic model (contracts, costs, reputation thresholds)

## Build & Run
```bash
# Build everything
cargo build --workspace

# Run with verbose logging
RUST_LOG=debug cargo run -p ow-app

# Run with trace-level logging for a specific crate
RUST_LOG=ow_data=trace,ow_core=debug cargo run -p ow-app

# Run RE survey tool against game files
cargo run -p ow-tools --bin survey -- /path/to/wages-of-war/

# Deep-inspect a specific file
cargo run -p ow-tools --bin triage -- /path/to/file.dat
```

## Code Style
- `cargo fmt` before every commit
- `cargo clippy -- -W clippy::all` must pass clean
- All public functions get doc comments
- **Human-readable inline comments everywhere.** Explain the WHY, not just the what. Annotate format quirks, game mechanic reasoning, and non-obvious parsing logic. Code should be approachable for contributors who don't know Wages of War internals.
- All structs that touch game data get `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`
- Error types per crate via `thiserror`
- No `unwrap()` in library crates — only in `ow-app/main.rs` for startup

## File Naming Conventions
- Rust: `snake_case.rs`
- Docs: `kebab-case.md`
- Data format specs: `FORMAT_NAME.md` in `docs/`

## Testing Strategy
- Unit tests in each crate (`#[cfg(test)]` modules)
- Integration tests with known .dat file snippets in `tests/fixtures/`
- Property-based tests for coordinate math (screen↔tile round-trips)
- No original game files in the repo — CI uses synthetic test data

## Git Workflow
- `main` branch is always buildable
- Feature branches: `feat/dat-parser`, `feat/combat-system`, `feat/iso-renderer`, etc.
- Commits are atomic and descriptive
- Tag releases: `v0.1.0` = can load and display data, `v0.2.0` = combat works, etc.

## Matt's Execution Context
- Matt is an expert Rust developer with deep systems knowledge. Skip the hand-holding.
- His timeline estimates are aggressive and accurate — don't pad.
- All tooling runs on Windows (kokonoe: i9-11900K, RTX 3070 Ti, Win11). Use PowerShell-safe scripts.
- The three `.skill` files in `skills/` contain detailed reference material for RE workflow, .dat parsing, and isometric engine architecture.
