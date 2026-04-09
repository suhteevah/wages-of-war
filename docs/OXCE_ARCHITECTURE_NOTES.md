# OXCE Architecture Patterns for Open Wages

Research notes from studying OpenXcom Extended (OXCE) source at
`github.com/MeridianOXC/OpenXcom`. Focus: architectural patterns to adopt,
not code to copy. Clean-room only.

---

## 1. Ruleset System

### Pattern: Typed Rule Objects Loaded from Data Files

OXCE defines one `Rule*` class per game concept: `RuleItem`, `RuleSoldier`,
`RuleCraft`, `RuleArmor`, `RuleTerrain`, etc. Each rule class:

- Holds all static/balancing data for that concept (no runtime state).
- Has a `load(YAML::Node)` method that populates itself from structured data.
- Is stored in a `std::map<std::string, Rule*>` keyed by string ID.
- Has a separate index vector for ordered iteration (UI display order).

The central `Mod` class owns all rule maps and provides typed getters:
`getItem(id)`, `getCraft(id)`, etc.

### Pattern: Declarative Rule Loading via Sectioned Data Files

OXCE rulesets are YAML files with top-level keys per concept type:
`items:`, `soldiers:`, `armors:`, `facilities:`, etc. The `loadFile()`
method iterates each section and calls a templated `loadRule()` that either
creates a new rule or retrieves an existing one for modification.

### Actionable for Open Wages

- Define one `Rule` struct per .DAT concept (weapons, mercs, contracts, etc.).
- Each struct implements `fn load(&mut self, &DatSection)`.
- Central `Ruleset` struct owns `HashMap<String, RuleWeapon>`, etc.
- Since our .DAT files are plaintext INI/CSV, parsing is simpler than YAML,
  but the typed-rule-map pattern still applies directly.
- Add a `list_order` field to every rule for UI sorting independent of
  definition order.

---

## 2. Mod Overlay / Data Layering System

### Pattern: Virtual Filesystem with First-Writer-Wins

OXCE's `FileMap` namespace builds a virtual filesystem overlay across all
active mod directories. Key behaviors:

- Mods are loaded in priority order (highest priority first).
- File paths are canonicalized to lowercase for case-insensitive lookup.
- First mod to register a file path wins; later mods cannot override the
  same file path. This is **first-writer-wins**, not last-writer-wins.
- Virtual directories merge contents from all mods (union of files).
- `getFilePath(relative)` resolves to the real filesystem path.

### Pattern: Rule Merging is Last-Writer-Wins

Confusingly, while *files* are first-writer-wins, *rules* use the opposite:
rules are loaded sequentially, and later definitions modify the same rule
object in-place. So the last mod to touch `items.soldiers.statCaps` wins.

This dual system means:
- **Asset files** (sprites, sounds): higher-priority mod's files are used.
- **Rule definitions** (stats, costs, behaviors): later-loaded mods patch
  existing rules, so the last mod in load order has final say.

### Pattern: Offset Allocation for New Assets

Each mod gets a reserved ID space (offset = 1000 * slot_index) for sprites
and sounds. This prevents ID collisions between mods adding new assets.

### Pattern: Master/Child Mod Hierarchy

Mods declare a `master` dependency. Total conversions are masters; small
mods declare which master they require. `canActivate(currentMaster)` gates
loading. This is simpler than a full dependency DAG.

### Actionable for Open Wages

- Our .DAT files are already text-editable, which is analogous to OXCE's
  YAML rulesets. We should build a layered loading system:
  1. Load base game .DAT files first.
  2. Load mod .DAT overrides in priority order.
  3. For rules: last-writer-wins (merge fields into existing structs).
  4. For assets: first-writer-wins (first mod to provide a file wins).
- Implement a `VirtualFs` that maps logical paths to physical paths across
  a mod stack.
- Reserve ID ranges per mod for any numeric-ID resources.

---

## 3. Save/Load Architecture

### Pattern: Two-Document Save Format

OXCE saves consist of two YAML documents in one file:

1. **Header document**: Lightweight metadata for save-list display (name,
   date, turn, mods, engine version). Can be parsed without loading the
   full game state.
2. **Full state document**: Complete serialized game state.

### Pattern: Atomic Writes via Temp File

Saves write to a temporary file first, then rename over the target. This
prevents corruption if the process crashes mid-write.

### Pattern: Graceful Degradation, Not Migration

OXCE records version fields but does **not** implement a formal migration
system. Compatibility is handled through:
- Optional fields with defaults (missing fields get sane defaults).
- Legacy key aliases (e.g., `terrorSites` loads as `missionSites`).
- Master mod filtering (saves only load with compatible mod sets).

There is no explicit schema version number or migration function chain.

### Pattern: Rule References by String ID

Saved state references rules by string ID, not by pointer or index. On load,
each saved object queries the `Mod` to resolve its rule reference. If the
rule is missing (mod removed), it logs a warning and skips gracefully.

### Actionable for Open Wages

- Use a two-section save format: brief header (JSON) + full state (JSON).
  `serde_json` handles this naturally.
- Always write to temp file, then `fs::rename()`.
- Reference all rules by string ID in saves, resolve on load via `Ruleset`.
- Prefer optional fields with `#[serde(default)]` over version migration.
- Record engine version + mod list in save header for compatibility checks.
- Consider `SaveConverter` equivalent for importing original WoW save files
  if the format is ever reverse-engineered.

---

## 4. Rendering Pipeline

### Pattern: 8-bit Paletted Surfaces

OXCE uses 256-color indexed palettes throughout. Every `Surface` is 8bpp.
Palettes are organized in 16-color blocks (blockOffset = block * 16).
Background colors start at palette index 224. Palette swaps achieve effects
like faction coloring without redrawing sprites.

### Pattern: Isometric Tile Rendering Order

The battlescape renderer (`Map::drawTerrain()`) uses painter's algorithm:

- **Outer loop**: Z levels, bottom to top.
- **Middle loop**: X coordinates.
- **Inner loop**: Y coordinates.

This Z-X-Y order ensures correct occlusion for isometric projection.

### Pattern: Per-Tile Layer Compositing

Each tile draws ~13 layers in strict order:

1. Floor sprite
2. Cursor (back layer)
3. Units from adjacent tiles (background pass)
4. West wall, North wall
5. Object sprites (by BigWall classification)
6. Floor items
7. Projectile shadow + body
8. Units on current tile
9. Units from adjacent tiles (foreground pass)
10. Smoke/fire overlays
11. Particle effects (via transparency LUT)
12. Path preview arrows
13. Cursor (front layer)

Units are drawn in **three passes from different tile positions** to handle
correct occlusion during movement animations across tile boundaries.

### Pattern: Camera Coordinate Math

Screen-to-map and map-to-screen use diamond projection formulas:

```
// Map to screen (conceptual):
screenX = (mapX - mapY) * (tileW / 2)
screenY = (mapX + mapY) * (tileH / 2) - mapZ * zStep
```

Where `zStep = (tileH + tileW/4) / 2`. The camera maintains scroll offset
and clips tile iteration to only render visible tiles plus a margin.

### Pattern: Viewport Culling

Before rendering, the camera calculates `beginX/Y/Z` and `endX/Y/Z` bounds
based on screen size and scroll position. Only tiles within these bounds are
iterated. A margin accounts for tall sprites that extend above their tile.

### Actionable for Open Wages

- Start with 8bpp paletted rendering if original WoW sprites are paletted.
  SDL2 supports indexed surfaces. This simplifies faction recoloring.
- Use Z-X-Y iteration order for painter's algorithm.
- Define a clear per-tile layer enum and draw each layer in fixed order.
- Implement the diamond projection formulas with our tile dimensions
  (likely 64x32 based on WoW's era).
- Viewport culling is essential from day one; do not iterate the full map.
- Plan for multi-pass unit rendering if units can span tile boundaries
  during animation.

---

## 5. State Machine

### Pattern: Stack-Based State Management

OXCE's `Game` class manages a `std::list<State*>` as a state stack:

- `pushState(state)`: Push a new screen/dialog on top.
- `popState()`: Remove the topmost state (returns to previous).
- `setState(state)`: Clear the entire stack, push one state (hard transition).

This enables modal dialogs (push), returning to previous screens (pop), and
full scene changes (setState for Geoscape -> Battlescape).

### Pattern: Deferred State Cleanup

States are never deleted during the frame they're popped. They move to a
`_deleted` list and are cleaned up at the start of the next frame cycle.
This prevents use-after-free when a state pops itself.

### Pattern: State Lifecycle

Every State has four virtual methods called by the game loop:

- `init()`: Called once when the state becomes active (top of stack).
- `handle(Action)`: Process input events.
- `think()`: Per-frame logic update.
- `blit()`: Render to screen.

All states in the stack get `blit()` called (bottom to top), but only the
topmost state gets `handle()` and `think()`. This means underlying states
remain visible (transparency/overlay) but frozen.

### Pattern: Phase-Specific State Classes

The codebase has ~120+ State subclasses organized by game phase:

- `Menu/`: MainMenuState, LoadGameState, OptionsState, etc.
- `Geoscape/`: GeoscapeState (the world map), DogfightState, etc.
- `Basescape/`: BasescapeState, CraftEquipmentState (~57 states).
- `Battlescape/`: BattlescapeState, InventoryState, etc.

Each screen/dialog is its own State class. No god-state with mode flags.

### Pattern: Nested State Machine for Combat

`BattlescapeGame` (not a State itself, but owned by `BattlescapeState`) runs
its own internal state stack for action resolution: `ProjectileFlyBState`,
`ExplosionBState`, `UnitWalkBState`, `UnitTurnBState`, etc. These "B-States"
(battle states) sequence animations and effects within a single turn action.

This is a **two-level state machine**: the outer Game state stack manages
screens, while the inner BattlescapeGame state stack manages combat actions.

### Actionable for Open Wages

- Implement a `StateStack` in `ow-app` with push/pop/set semantics.
- Define a `State` trait with `init()`, `handle_event()`, `update()`,
  `render()` methods.
- Use deferred cleanup (drop states next frame, not during pop).
- Blit all states in stack order; only tick/handle the topmost.
- Create one State struct per screen: `MainMenuState`, `GeoscapeState`
  (our contract/economy screen), `BattlescapeState`, `InventoryState`, etc.
- For combat, implement a separate `BattleActionQueue` that sequences
  animation states within BattlescapeState. This avoids coupling the
  outer state machine to combat animation details.

---

## 6. Cross-Cutting Patterns Worth Adopting

### String IDs Everywhere

Every game object type, rule, and resource is identified by a string ID.
No magic integers. This makes data files human-readable, saves debuggable,
and mod conflicts obvious.

### Separation of Rules vs Runtime State

`Mod/Rule*` classes hold static data. `Savegame/*` classes hold runtime
state and reference rules by ID. This clean separation means rules can be
reloaded without invalidating save state.

### Centralized Game Object Ownership

The `Game` class owns exactly three top-level objects: `Mod` (rules),
`SavedGame` (state), `Screen` (rendering). Everything else is accessed
through these three. For us: `Ruleset`, `GameState`, `Renderer`.

### Exhaustive Logging

OXCE logs file loads, rule parsing, resource resolution failures, and
state transitions. This aligns perfectly with our `tracing` requirement.

---

## Summary: Priority Patterns for Open Wages

| Priority | Pattern | Apply To |
|----------|---------|----------|
| P0 | Typed rule structs per concept | `ow-data` |
| P0 | Stack-based state machine | `ow-app` |
| P0 | Rules vs State separation | `ow-data` / `ow-core` |
| P0 | String ID references everywhere | All crates |
| P1 | Virtual filesystem overlay | `ow-data` (mod support) |
| P1 | Two-section save format | `ow-core` |
| P1 | Z-X-Y painter's algorithm | `ow-render` |
| P1 | Per-tile layer compositing | `ow-render` |
| P2 | Nested combat action queue | `ow-core` |
| P2 | Offset-based mod asset IDs | `ow-data` |
| P2 | Atomic save writes | `ow-core` |
