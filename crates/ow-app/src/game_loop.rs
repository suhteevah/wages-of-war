//! # Game Loop — SDL2 state-machine event loop
//!
//! Implements the OXCE-style phase-driven game loop. Each [`GamePhase`] has its
//! own `handle_input` / `update` / `render` cycle, driven by the top-level
//! `run_game_loop` function.
//!
//! ## State Machine
//!
//! ```text
//! Office (overview)
//!   → HireMercs  (1)   select/deselect mercs from roster
//!   → Equipment  (2)   buy/sell gear
//!   → Intel      (3)   read reports
//!   → Contracts  (4)   view/accept contracts
//!   → Training   (5)   train mercs between missions
//!   → Begin Mission (B) → Travel
//! Travel → auto-transition → Mission(Deployment)
//! Mission
//!   → Deployment   place mercs on start tiles
//!   → Combat       initiative turns, move/shoot/AI
//!   → Extraction   mission complete, reach exit
//! Debrief → show results → Enter → back to Office
//! ```
//!
//! ## Frame Timing
//!
//! The loop targets 60 fps with delta-time tracking. Delta is capped at 33 ms
//! (floor of 30 fps) to prevent physics/animation explosions on hitches.

use std::time::Instant;

use anyhow::Result;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use sdl2::Sdl;
use tracing::{debug, info, trace, warn};

use ow_core::actions::Action;
use ow_core::game_state::{GamePhase, GameState, MissionPhase, OfficePhase};
use ow_core::merc::MercId;

use ow_core::ruleset::Ruleset;
use ow_render::camera::Camera;
use ow_render::iso_math::{IsoConfig, ScreenPos, TilePos};
use ow_render::text::TextRenderer;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Target frame duration for 60 fps (16.67 ms).
const TARGET_FRAME_MS: u32 = 16;

/// Maximum delta time in milliseconds. Frames longer than this are clamped
/// to prevent animation/physics blow-ups during hitches or debugger pauses.
const MAX_DELTA_MS: u32 = 33;

/// Window width at startup.
const WINDOW_WIDTH: u32 = 1280;
/// Window height at startup.
const WINDOW_HEIGHT: u32 = 720;

// ---------------------------------------------------------------------------
// Phase-specific state
// ---------------------------------------------------------------------------

/// Per-phase handler state. Each variant carries the mutable state that only
/// matters while that phase is active — released on phase transition.
#[derive(Debug)]
pub enum PhaseHandler {
    /// Office phase — tracks which sub-phase (overview, hiring, etc.) is shown.
    Office {
        sub_phase: OfficePhase,
    },

    /// Travel screen — purely cosmetic, auto-advances after a short delay.
    Travel {
        /// Accumulated time in this phase (ms). Auto-transitions to mission
        /// after a brief "traveling..." display.
        elapsed_ms: u32,
    },

    /// Deployment — player places mercs on start tiles before combat begins.
    Deployment {
        /// Index into `player_units` of the currently selected merc for placement.
        selected_unit: usize,
    },

    /// Active turn-based combat.
    Combat(CombatHandler),

    /// Extraction — objectives complete, move to exit zone.
    Extraction,

    /// Post-mission debrief showing results.
    Debrief {
        /// True if the mission was a success.
        success: bool,
    },

    /// Pause overlay — remembers the phase we paused from.
    Paused {
        /// The phase handler we were in before pausing.
        previous: Box<PhaseHandler>,
    },
}

/// Combat-specific state tracked across turns.
#[derive(Debug)]
pub struct CombatHandler {
    /// Initiative-sorted list of unit IDs for this round.
    /// Contains both player and enemy unit IDs.
    pub initiative_order: Vec<MercId>,
    /// Index into `initiative_order` for the currently acting unit.
    pub current_initiative_idx: usize,
    /// Currently selected player unit (for UI highlighting / input).
    pub selected_unit_id: Option<MercId>,
    /// True when the AI is processing enemy turns (blocks player input).
    pub ai_acting: bool,
    /// Index for Tab-cycling through player units.
    pub tab_cycle_index: usize,
}

// ---------------------------------------------------------------------------
// GameLoop — the top-level struct
// ---------------------------------------------------------------------------

/// Top-level game loop state, tying together game state, camera, and
/// phase-specific handling.
pub struct GameLoop {
    /// The campaign game state (phase, team, funds, mission context, etc.).
    pub game_state: GameState,
    /// Isometric camera controlling the viewport.
    pub camera: Camera,
    /// Isometric projection configuration (tile dimensions, origin).
    pub iso_config: IsoConfig,
    /// Phase-specific handler with per-phase mutable state.
    pub phase_handler: PhaseHandler,
}

impl GameLoop {
    /// Create a new game loop from an initialized game state.
    pub fn new(game_state: GameState) -> Self {
        let phase_handler = phase_handler_for(&game_state.phase);

        Self {
            game_state,
            camera: Camera::new(WINDOW_WIDTH, WINDOW_HEIGHT),
            iso_config: IsoConfig {
                tile_width: 64.0,
                tile_height: 32.0,
                origin_x: (WINDOW_WIDTH as f32) / 2.0,
                origin_y: 64.0,
            },
            phase_handler,
        }
    }
}

/// Build the appropriate `PhaseHandler` for a given `GamePhase`.
fn phase_handler_for(phase: &GamePhase) -> PhaseHandler {
    match phase {
        GamePhase::Office(sub) => PhaseHandler::Office { sub_phase: *sub },
        GamePhase::Travel => PhaseHandler::Travel { elapsed_ms: 0 },
        GamePhase::Mission(MissionPhase::Deployment) => PhaseHandler::Deployment {
            selected_unit: 0,
        },
        GamePhase::Mission(MissionPhase::Combat) => PhaseHandler::Combat(CombatHandler {
            initiative_order: Vec::new(),
            current_initiative_idx: 0,
            selected_unit_id: None,
            ai_acting: false,
            tab_cycle_index: 0,
        }),
        GamePhase::Mission(MissionPhase::Extraction) => PhaseHandler::Extraction,
        GamePhase::Debrief => PhaseHandler::Debrief { success: true },
    }
}

// ---------------------------------------------------------------------------
// Color palette for placeholder rendering
// ---------------------------------------------------------------------------

/// Background colors for each phase — used for placeholder rendering before
/// real art assets are wired up.
fn phase_background_color(handler: &PhaseHandler) -> Color {
    match handler {
        PhaseHandler::Office { sub_phase } => match sub_phase {
            OfficePhase::Overview => Color::RGB(30, 40, 60),
            OfficePhase::HireMercs => Color::RGB(40, 60, 40),
            OfficePhase::Equipment => Color::RGB(60, 50, 30),
            OfficePhase::Intel => Color::RGB(40, 40, 60),
            OfficePhase::Contracts => Color::RGB(50, 35, 35),
            OfficePhase::Training => Color::RGB(35, 55, 55),
        },
        PhaseHandler::Travel { .. } => Color::RGB(20, 20, 40),
        PhaseHandler::Deployment { .. } => Color::RGB(30, 50, 30),
        PhaseHandler::Combat(_) => Color::RGB(10, 10, 10),
        PhaseHandler::Extraction => Color::RGB(40, 50, 30),
        PhaseHandler::Debrief { success } => {
            if *success {
                Color::RGB(20, 50, 20)
            } else {
                Color::RGB(60, 20, 20)
            }
        }
        PhaseHandler::Paused { .. } => Color::RGB(30, 30, 30),
    }
}

/// Human-readable label for the current phase.
fn phase_label(handler: &PhaseHandler) -> &'static str {
    match handler {
        PhaseHandler::Office { sub_phase } => match sub_phase {
            OfficePhase::Overview => "OFFICE - Overview",
            OfficePhase::HireMercs => "OFFICE - Hire Mercs",
            OfficePhase::Equipment => "OFFICE - Equipment",
            OfficePhase::Intel => "OFFICE - Intel",
            OfficePhase::Contracts => "OFFICE - Contracts",
            OfficePhase::Training => "OFFICE - Training",
        },
        PhaseHandler::Travel { .. } => "TRAVELING...",
        PhaseHandler::Deployment { .. } => "MISSION - Deployment",
        PhaseHandler::Combat(_) => "MISSION - Combat",
        PhaseHandler::Extraction => "MISSION - Extraction",
        PhaseHandler::Debrief { success } => {
            if *success {
                "DEBRIEF - Mission Complete!"
            } else {
                "DEBRIEF - Mission Failed"
            }
        }
        PhaseHandler::Paused { .. } => "PAUSED",
    }
}

// ===========================================================================
// run_game_loop — the main entry point
// ===========================================================================

/// Run the SDL2 game loop until the player quits.
///
/// This is the beating heart of the engine. It owns the event pump and drives
/// the per-phase update/render cycle at 60 fps with delta-time tracking.
///
/// # Parameters
/// - `sdl_context`: Initialized SDL2 context (owns the event pump).
/// - `canvas`: SDL2 window canvas for rendering.
/// - `game_state`: Pre-initialized campaign state (from main.rs).
///
/// # Returns
/// `Ok(())` on clean exit, `Err` on SDL2 or fatal engine errors.
pub fn run_game_loop(
    sdl_context: &Sdl,
    mut canvas: Canvas<Window>,
    game_state: GameState,
    ruleset: Ruleset,
    data_dir: &std::path::Path,
) -> Result<()> {
    info!(phase = ?game_state.phase, "Starting game loop");

    let mut game = GameLoop::new(game_state);
    let mut event_pump = sdl_context
        .event_pump()
        .map_err(|e| anyhow::anyhow!("Failed to get SDL2 event pump: {e}"))?;

    // Initialize text rendering — loads a system font for UI text.
    let ttf_context = sdl2::ttf::init()
        .map_err(|e| anyhow::anyhow!("SDL2_ttf init failed: {e}"))?;
    let text_renderer = TextRenderer::new(&ttf_context, None)
        .map_err(|e| anyhow::anyhow!("Font loading failed: {e}"))?;
    let texture_creator = canvas.texture_creator();

    // Load the office background image — OFFICE.PCX is the main HQ screen.
    // The original game renders this as a 640x480 scene with clickable objects
    // (phone, fax, filing cabinet, pizza, etc.) overlaid on the background.
    let office_texture = {
        // OFFICE.PCX is the base layer of the office scene. The original engine
        // composites OBJ sprites on top for the interactive objects (phone, fax, etc.).
        // OFFPIC2.PCX is a pre-composited version with all objects baked in.
        // We use OFFPIC2 for now; proper compositing comes later.
        let pcx_path = data_dir.join("WOW").join("PIC").join("OFFPIC2.PCX");
        match ow_render::pcx::load_pcx(&pcx_path) {
            Ok(img) => {
                info!(width = img.width, height = img.height, "Office background loaded");
                match ow_render::pcx::pcx_to_texture(&img, &texture_creator) {
                    Ok(tex) => Some(tex),
                    Err(e) => { warn!("Failed to create office texture: {e}"); None }
                }
            }
            Err(e) => { warn!("Failed to load OFFICE.PCX: {e}"); None }
        }
    };

    let mut last_frame = Instant::now();
    let mut running = true;
    let mut screenshot_taken = false;

    // -----------------------------------------------------------------------
    // Main loop: poll events -> update -> render -> present -> sleep
    // -----------------------------------------------------------------------
    while running {
        // -- Delta time calculation --
        let now = Instant::now();
        let raw_delta_ms = now.duration_since(last_frame).as_millis() as u32;
        let delta_ms = raw_delta_ms.min(MAX_DELTA_MS);
        last_frame = now;

        // -- Event handling --
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => {
                    info!("Quit event received");
                    running = false;
                }

                // ESC toggles pause overlay (or quits from pause)
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => {
                    running = handle_escape(&mut game);
                }

                // Delegate all other input to the current phase handler
                _ => {
                    handle_phase_input(&mut game, &event);
                }
            }
        }

        if !running {
            break;
        }

        // -- Update --
        update_phase(&mut game, delta_ms);

        // -- Render --
        let bg = phase_background_color(&game.phase_handler);
        canvas.set_draw_color(bg);
        canvas.clear();

        render_phase(&game, &mut canvas, &text_renderer, &texture_creator, &ruleset, &office_texture);

        // Title bar shows the current phase (placeholder for real UI)
        let label = phase_label(&game.phase_handler);
        canvas
            .window_mut()
            .set_title(&format!("Open Wages \u{2014} {label}"))
            .ok();

        canvas.present();

        // Auto-save a debug screenshot on the 3rd frame (after rendering stabilizes).
        // F12 also saves a screenshot at any time.
        if !screenshot_taken {
            screenshot_taken = true;
            // Read pixels back from the canvas and save as BMP.
            let (sw, sh) = canvas.output_size().unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
            let pixel_format = canvas.default_pixel_format();
            if let Ok(pixels) = canvas.read_pixels(None, pixel_format) {
                if let Ok(surface) = sdl2::surface::Surface::from_data(
                    &mut pixels.clone(),
                    sw, sh,
                    sw * 4,
                    sdl2::pixels::PixelFormatEnum::ARGB8888,
                ) {
                    let path = std::path::Path::new("debug_screenshot.bmp");
                    if surface.save_bmp(path).is_ok() {
                        info!("Debug screenshot saved to debug_screenshot.bmp");
                    }
                }
            }
        }

        // -- Frame pacing --
        // Sleep for remaining frame budget to hit ~60 fps.
        let frame_elapsed = now.elapsed().as_millis() as u32;
        if frame_elapsed < TARGET_FRAME_MS {
            std::thread::sleep(std::time::Duration::from_millis(
                (TARGET_FRAME_MS - frame_elapsed) as u64,
            ));
        }
    }

    info!("Game loop exited cleanly");
    Ok(())
}

// ===========================================================================
// Escape / Pause handling
// ===========================================================================

/// Handle the ESC key. Returns `false` if the game should quit.
fn handle_escape(game: &mut GameLoop) -> bool {
    match &game.phase_handler {
        PhaseHandler::Paused { .. } => {
            // Second ESC from pause -> quit
            info!("Quitting from pause menu");
            false
        }
        _ => {
            // First ESC -> enter pause
            info!("Entering pause");
            let current = std::mem::replace(
                &mut game.phase_handler,
                // Temporary — immediately overwritten
                PhaseHandler::Office {
                    sub_phase: OfficePhase::Overview,
                },
            );
            game.phase_handler = PhaseHandler::Paused {
                previous: Box::new(current),
            };
            true
        }
    }
}

// ===========================================================================
// Phase-specific input handling
// ===========================================================================

/// Route input events to the active phase handler.
///
/// To satisfy the borrow checker, each branch extracts any needed values from
/// `game.phase_handler` by copy/clone *before* passing `game` to sub-handlers.
/// Phase transitions replace `game.phase_handler` wholesale rather than
/// mutating through a partial borrow.
fn handle_phase_input(game: &mut GameLoop, event: &Event) {
    // Take a snapshot of the current phase discriminant to route input.
    // We avoid borrowing game.phase_handler across the handler calls.
    enum Route {
        Paused,
        Office,
        Travel,
        Deployment,
        Combat,
        Extraction,
        Debrief,
    }

    let route = match &game.phase_handler {
        PhaseHandler::Paused { .. } => Route::Paused,
        PhaseHandler::Office { .. } => Route::Office,
        PhaseHandler::Travel { .. } => Route::Travel,
        PhaseHandler::Deployment { .. } => Route::Deployment,
        PhaseHandler::Combat(_) => Route::Combat,
        PhaseHandler::Extraction => Route::Extraction,
        PhaseHandler::Debrief { .. } => Route::Debrief,
    };

    match route {
        Route::Paused => handle_pause_input(game, event),
        Route::Office => handle_office_input(game, event),
        Route::Travel => { /* No player input during travel */ }
        Route::Deployment => handle_deployment_input(game, event),
        Route::Combat => handle_combat_input(game, event),
        Route::Extraction => handle_extraction_input(game, event),
        Route::Debrief => handle_debrief_input(game, event),
    }
}

// ---------------------------------------------------------------------------
// Pause input
// ---------------------------------------------------------------------------

/// While paused, Enter resumes.
fn handle_pause_input(game: &mut GameLoop, event: &Event) {
    if let Event::KeyDown {
        keycode: Some(Keycode::Return),
        ..
    } = event
    {
        info!("Resuming from pause");
        // Extract the previous handler from the Paused variant.
        let prev = match std::mem::replace(
            &mut game.phase_handler,
            PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            },
        ) {
            PhaseHandler::Paused { previous } => *previous,
            other => other, // shouldn't happen, but be safe
        };
        game.phase_handler = prev;
    }
}

// ---------------------------------------------------------------------------
// Office input
// ---------------------------------------------------------------------------

/// Handle input while in the Office phase.
///
/// Number keys 1-6 switch between sub-phases:
///   1 = Overview, 2 = Hire Mercs, 3 = Equipment,
///   4 = Intel, 5 = Contracts, 6 = Training
///
/// 'B' begins a mission (transitions to Travel) if preconditions are met:
///   - At least one merc hired
///   - A contract accepted (placeholder: always allowed for now)
fn handle_office_input(game: &mut GameLoop, event: &Event) {
    if let Event::KeyDown {
        keycode: Some(key), ..
    } = event
    {
        let new_sub = match *key {
            Keycode::Num1 => Some(OfficePhase::Overview),
            Keycode::Num2 => Some(OfficePhase::HireMercs),
            Keycode::Num3 => Some(OfficePhase::Equipment),
            Keycode::Num4 => Some(OfficePhase::Intel),
            Keycode::Num5 => Some(OfficePhase::Contracts),
            Keycode::Num6 => Some(OfficePhase::Training),
            _ => None,
        };

        if let Some(sub) = new_sub {
            debug!(sub_phase = ?sub, "Office sub-phase switch");
            game.game_state.set_phase(GamePhase::Office(sub));
            game.phase_handler = PhaseHandler::Office { sub_phase: sub };
            return;
        }

        // Begin mission
        if *key == Keycode::B {
            if game.game_state.team.is_empty() {
                warn!("Cannot begin mission: no mercs hired");
            } else {
                info!(
                    team_size = game.game_state.team.len(),
                    "Beginning mission -- transitioning to Travel"
                );
                game.game_state.set_phase(GamePhase::Travel);
                game.phase_handler = PhaseHandler::Travel { elapsed_ms: 0 };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deployment input
// ---------------------------------------------------------------------------

/// Handle input during the deployment phase.
///
/// - Tab: cycle through mercs to place.
/// - Click: place selected merc on the clicked tile.
/// - Enter: confirm deployment, start combat.
/// - WASD: scroll camera.
fn handle_deployment_input(game: &mut GameLoop, event: &Event) {
    match event {
        // Tab: cycle to next merc for placement
        Event::KeyDown {
            keycode: Some(Keycode::Tab),
            ..
        } => {
            let team_len = game.game_state.team.len();
            if team_len > 0 {
                if let PhaseHandler::Deployment { selected_unit } = &mut game.phase_handler {
                    *selected_unit = (*selected_unit + 1) % team_len;
                    debug!(
                        selected = *selected_unit,
                        name = %game.game_state.team[*selected_unit].name,
                        "Deployment: selected next merc"
                    );
                }
            }
        }

        // Click: place selected merc on the clicked tile
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let screen = ScreenPos {
                x: *x as f32,
                y: *y as f32,
            };
            let world = game.camera.screen_to_world(screen);
            let tile = game.iso_config.screen_to_tile(world);
            let core_tile = ow_core::merc::TilePos {
                x: tile.x,
                y: tile.y,
            };

            // Read the selected index, place the merc, then advance
            let selected = match &game.phase_handler {
                PhaseHandler::Deployment { selected_unit } => *selected_unit,
                _ => return,
            };
            let team_len = game.game_state.team.len();
            if selected < team_len {
                info!(
                    name = %game.game_state.team[selected].name,
                    tile_x = tile.x,
                    tile_y = tile.y,
                    "Deployment: placed merc"
                );
                game.game_state.team[selected].position = Some(core_tile);

                // Auto-advance to next unplaced merc
                if let PhaseHandler::Deployment { selected_unit } = &mut game.phase_handler {
                    *selected_unit = (*selected_unit + 1) % team_len;
                }
            }
        }

        // Enter: confirm deployment, transition to combat
        Event::KeyDown {
            keycode: Some(Keycode::Return),
            ..
        } => {
            let placed = game
                .game_state
                .team
                .iter()
                .filter(|m| m.position.is_some())
                .count();
            let total = game.game_state.team.len();

            if placed == 0 {
                warn!("Cannot start combat: no mercs placed on the map");
                return;
            }

            info!(
                placed,
                total, "Deployment confirmed -- transitioning to Combat"
            );
            game.game_state
                .set_phase(GamePhase::Mission(MissionPhase::Combat));

            // Build initiative order from placed, living player units.
            // TODO: Interleave enemy units sorted by initiative stat.
            let mut init_order: Vec<MercId> = Vec::new();
            for merc in &game.game_state.team {
                if merc.position.is_some() && merc.is_alive() {
                    init_order.push(merc.id);
                }
            }
            let first_id = init_order.first().copied();

            game.phase_handler = PhaseHandler::Combat(CombatHandler {
                initiative_order: init_order,
                current_initiative_idx: 0,
                selected_unit_id: first_id,
                ai_acting: false,
                tab_cycle_index: 0,
            });
        }

        // WASD camera scrolling
        Event::KeyDown {
            keycode: Some(key @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)),
            ..
        } => {
            apply_camera_scroll(&mut game.camera, *key);
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Combat input
// ---------------------------------------------------------------------------

/// Handle input during the combat phase.
///
/// - WASD: scroll camera
/// - Tab: cycle through player units
/// - Left click on tile: move selected unit (if reachable)
/// - Left click on enemy: shoot if in range and LOS
/// - 'E': end current unit's turn
/// - Mouse wheel: zoom in/out
///
/// When AI is acting, player input is blocked.
fn handle_combat_input(game: &mut GameLoop, event: &Event) {
    // Check if the AI is acting — block player input if so.
    let ai_acting = match &game.phase_handler {
        PhaseHandler::Combat(c) => c.ai_acting,
        _ => return,
    };
    if ai_acting {
        trace!("Combat input blocked: AI is acting");
        return;
    }

    match event {
        // Camera scrolling
        Event::KeyDown {
            keycode: Some(key @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)),
            ..
        } => {
            apply_camera_scroll(&mut game.camera, *key);
        }

        // Tab: cycle through living player units
        Event::KeyDown {
            keycode: Some(Keycode::Tab),
            ..
        } => {
            let living: Vec<MercId> = game
                .game_state
                .team
                .iter()
                .filter(|m| m.is_alive() && m.position.is_some())
                .map(|m| m.id)
                .collect();

            if let PhaseHandler::Combat(c) = &mut game.phase_handler {
                if !living.is_empty() {
                    c.tab_cycle_index = (c.tab_cycle_index + 1) % living.len();
                    c.selected_unit_id = Some(living[c.tab_cycle_index]);
                    debug!(selected = ?c.selected_unit_id, "Tab-cycled to next player unit");
                }
            }
        }

        // E: end current unit's turn
        Event::KeyDown {
            keycode: Some(Keycode::E),
            ..
        } => {
            let selected = match &game.phase_handler {
                PhaseHandler::Combat(c) => c.selected_unit_id,
                _ => None,
            };
            if let Some(unit_id) = selected {
                info!(unit_id, "Player ended unit's turn");
                advance_initiative(game);
            }
        }

        // Mouse click: move or shoot depending on what occupies the target tile
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let selected = match &game.phase_handler {
                PhaseHandler::Combat(c) => c.selected_unit_id,
                _ => None,
            };
            if let Some(unit_id) = selected {
                let screen = ScreenPos {
                    x: *x as f32,
                    y: *y as f32,
                };
                let world = game.camera.screen_to_world(screen);
                let tile = game.iso_config.screen_to_tile(world);
                let target_tile = ow_core::merc::TilePos {
                    x: tile.x,
                    y: tile.y,
                };

                // Check if an enemy occupies this tile
                // TODO: Wire through MissionState.enemy_units for real lookup.
                let enemy_at_tile: Option<MercId> = None;

                if let Some(enemy_id) = enemy_at_tile {
                    info!(
                        shooter = unit_id,
                        target = enemy_id,
                        "Player shooting at enemy"
                    );
                    debug!(
                        action = ?Action::Shoot(enemy_id),
                        "Queued shoot action (pending execute_action wiring)"
                    );
                } else {
                    // Move to the tile (placeholder: direct teleport, no AP cost)
                    info!(
                        unit_id,
                        tile_x = target_tile.x,
                        tile_y = target_tile.y,
                        "Player moving unit"
                    );
                    if let Some(merc) =
                        game.game_state.team.iter_mut().find(|m| m.id == unit_id)
                    {
                        merc.position = Some(target_tile);
                        debug!(
                            name = %merc.name,
                            ?target_tile,
                            "Unit moved (placeholder -- no AP deduction yet)"
                        );
                    }
                }
            }
        }

        // Mouse wheel: zoom
        Event::MouseWheel { y, .. } => {
            if *y > 0 {
                game.camera.zoom_in();
            } else if *y < 0 {
                game.camera.zoom_out();
            }
        }

        _ => {}
    }
}

/// Advance to the next unit in the initiative order.
///
/// If all player units have acted, trigger the AI turn for enemy units.
/// If all units (player + enemy) have acted, start a new round with AP resets.
fn advance_initiative(game: &mut GameLoop) {
    // Extract what we need, then mutate.
    let (order_len, mut next_idx) = match &game.phase_handler {
        PhaseHandler::Combat(c) => (c.initiative_order.len(), c.current_initiative_idx + 1),
        _ => return,
    };

    if next_idx >= order_len {
        // All units acted this round — start new round.
        info!("Round complete -- starting new round");
        next_idx = 0;

        // Reset AP for all player units
        for merc in &mut game.game_state.team {
            if merc.is_alive() {
                let base = merc.base_aps as u32;
                merc.current_ap = if merc.suppressed { base / 2 } else { base };
                merc.suppressed = false;
                trace!(name = %merc.name, ap = merc.current_ap, "AP reset for new round");
            }
        }
    }

    // Determine who acts next
    if let PhaseHandler::Combat(c) = &mut game.phase_handler {
        c.current_initiative_idx = next_idx;

        if let Some(&next_id) = c.initiative_order.get(next_idx) {
            let is_player = game.game_state.team.iter().any(|m| m.id == next_id);
            if is_player {
                c.selected_unit_id = Some(next_id);
                c.ai_acting = false;
                debug!(unit_id = next_id, "Player unit's turn");
            } else {
                c.ai_acting = true;
                debug!(unit_id = next_id, "Enemy unit's turn -- AI deciding");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction input
// ---------------------------------------------------------------------------

/// Handle input during the extraction phase.
/// Press Enter to finish the mission and go to Debrief.
/// WASD scrolls the camera.
fn handle_extraction_input(game: &mut GameLoop, event: &Event) {
    match event {
        Event::KeyDown {
            keycode: Some(Keycode::Return),
            ..
        } => {
            info!("Extraction complete -- transitioning to Debrief");
            game.game_state.set_phase(GamePhase::Debrief);
            game.phase_handler = PhaseHandler::Debrief { success: true };
        }
        Event::KeyDown {
            keycode: Some(key @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)),
            ..
        } => {
            apply_camera_scroll(&mut game.camera, *key);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Debrief input
// ---------------------------------------------------------------------------

/// Handle input during the debrief phase.
/// Press Enter to return to the Office.
fn handle_debrief_input(game: &mut GameLoop, event: &Event) {
    if let Event::KeyDown {
        keycode: Some(Keycode::Return),
        ..
    } = event
    {
        info!("Debrief acknowledged -- returning to Office");
        game.game_state.missions_completed += 1;
        game.game_state.current_mission = None;
        game.game_state
            .set_phase(GamePhase::Office(OfficePhase::Overview));
        game.phase_handler = PhaseHandler::Office {
            sub_phase: OfficePhase::Overview,
        };
    }
}

// ---------------------------------------------------------------------------
// Camera scroll helper
// ---------------------------------------------------------------------------

/// Apply a single discrete camera scroll step for a WASD key press.
fn apply_camera_scroll(camera: &mut Camera, key: Keycode) {
    let step = 32.0;
    match key {
        Keycode::W => camera.scroll(0.0, -step),
        Keycode::A => camera.scroll(-step, 0.0),
        Keycode::S => camera.scroll(0.0, step),
        Keycode::D => camera.scroll(step, 0.0),
        _ => {}
    }
}

// ===========================================================================
// Phase update logic
// ===========================================================================

/// Tick the current phase's update logic.
fn update_phase(game: &mut GameLoop, delta_ms: u32) {
    // Snapshot the phase discriminant to avoid borrowing game.phase_handler
    // across the update call.
    enum UpdateRoute {
        Travel,
        Combat,
        Other,
    }

    let route = match &game.phase_handler {
        PhaseHandler::Travel { .. } => UpdateRoute::Travel,
        PhaseHandler::Combat(_) => UpdateRoute::Combat,
        _ => UpdateRoute::Other,
    };

    match route {
        UpdateRoute::Travel => update_travel(game, delta_ms),
        UpdateRoute::Combat => update_combat(game, delta_ms),
        UpdateRoute::Other => {
            // Office, Deployment, Extraction, Debrief, Paused:
            // No per-frame update logic (purely input-driven).
        }
    }
}

/// Travel phase update: auto-advance to Mission(Deployment) after a brief delay.
fn update_travel(game: &mut GameLoop, delta_ms: u32) {
    const TRAVEL_DURATION_MS: u32 = 2000;

    let should_transition = match &mut game.phase_handler {
        PhaseHandler::Travel { elapsed_ms } => {
            *elapsed_ms += delta_ms;
            *elapsed_ms >= TRAVEL_DURATION_MS
        }
        _ => false,
    };

    if should_transition {
        info!("Travel complete -- transitioning to Mission Deployment");
        game.game_state
            .set_phase(GamePhase::Mission(MissionPhase::Deployment));
        game.phase_handler = PhaseHandler::Deployment { selected_unit: 0 };
    }
}

/// Combat phase update: process AI turns and check victory/defeat conditions.
///
/// When it's an enemy's turn, the AI picks and executes one action per frame.
/// This gives a visible cadence to enemy actions and keeps the frame rate smooth.
fn update_combat(game: &mut GameLoop, _delta_ms: u32) {
    // -- AI turn processing --
    let ai_acting = match &game.phase_handler {
        PhaseHandler::Combat(c) => c.ai_acting,
        _ => return,
    };

    if ai_acting {
        let current_id = match &game.phase_handler {
            PhaseHandler::Combat(c) => c.initiative_order.get(c.current_initiative_idx).copied(),
            _ => None,
        };

        if let Some(id) = current_id {
            let is_player = game.game_state.team.iter().any(|m| m.id == id);
            if !is_player {
                // AI decides an action.
                // TODO: Wire decide_action(mission_state, id) and execute_action()
                //       once MissionState is accessible from here.
                debug!(unit_id = id, "AI turn: would call decide_action() here");

                // For now, AI just ends its turn
                advance_initiative(game);
            } else {
                // Somehow landed on a player unit while AI is acting — hand back
                if let PhaseHandler::Combat(c) = &mut game.phase_handler {
                    c.ai_acting = false;
                    c.selected_unit_id = Some(id);
                }
            }
        } else {
            // Past the end of initiative order — reset
            if let PhaseHandler::Combat(c) = &mut game.phase_handler {
                c.ai_acting = false;
                c.current_initiative_idx = 0;
            }
        }
    }

    // -- Victory/defeat condition checks --

    // Defeat: all player mercs dead
    let all_dead = !game.game_state.team.is_empty()
        && game.game_state.team.iter().all(|m| !m.is_alive());

    if all_dead {
        warn!("All player mercs killed -- mission failed");
        game.game_state.set_phase(GamePhase::Debrief);
        game.phase_handler = PhaseHandler::Debrief { success: false };
        return;
    }

    // Victory: all enemies eliminated.
    // TODO: Check MissionState.enemy_units once wired. For now this is a no-op.
    // When wired:
    //   let all_enemies_dead = mission_state.enemy_units.iter().all(|e| e.current_hp == 0);
    //   if all_enemies_dead {
    //       game.game_state.set_phase(GamePhase::Mission(MissionPhase::Extraction));
    //       game.phase_handler = PhaseHandler::Extraction;
    //   }
}

// ===========================================================================
// Phase rendering
// ===========================================================================

/// Render the current phase to the canvas.
///
/// Most phases render a colored background (set in the main loop) with
/// geometric placeholders. Combat renders an isometric grid plus unit markers.
fn render_phase(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    office_bg: &Option<Texture>,
) {
    match &game.phase_handler {
        PhaseHandler::Office { sub_phase } => render_office(game, canvas, *sub_phase, text, tc, ruleset, office_bg),
        PhaseHandler::Travel { elapsed_ms } => render_travel(canvas, *elapsed_ms, text, tc),
        PhaseHandler::Deployment { selected_unit } => {
            render_deployment(game, canvas, *selected_unit)
        }
        PhaseHandler::Combat(combat) => render_combat(game, canvas, combat),
        PhaseHandler::Extraction => render_extraction(game, canvas),
        PhaseHandler::Debrief { success } => render_debrief(canvas, *success, text, tc),
        PhaseHandler::Paused { .. } => render_pause(canvas, text, tc),
    }
}

// ---------------------------------------------------------------------------
// Office rendering
// ---------------------------------------------------------------------------

/// Render the office screen.
///
/// Placeholder: colored background per sub-phase, tab indicators at top,
/// team size / funds indicators at bottom.
fn render_office(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    active_sub: OfficePhase,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    office_bg: &Option<Texture>,
) {
    let (w, h) = canvas.output_size().unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    // -- For the Overview tab, render the original OFFICE.PCX background --
    // This is the iconic desk scene the player sees when the game starts.
    // Other sub-phases overlay their own content on a dark background.
    match active_sub {
        OfficePhase::Overview => {
            if let Some(bg_tex) = office_bg {
                // Scale the 640x480 office background to fill the window.
                canvas.copy(bg_tex, None, Some(Rect::new(0, 0, w, h))).ok();
            }

            // Overlay help text on the office background.
            // Semi-transparent bar at bottom for readability.
            canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.fill_rect(Rect::new(0, (h - 55) as i32, w, 55)).ok();
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            let funds_text = format!("Funds: ${:>12}  |  Team: {}/8  |  Missions: {}",
                game.game_state.funds, game.game_state.team.len(), game.game_state.missions_completed);
            text.draw(canvas, tc, &funds_text, 15, (h - 45) as i32, Color::RGB(220, 220, 220)).ok();
            text.draw_small(canvas, tc,
                "1:Hire  2:Equip  3:Intel  4:Contracts  5:Train  |  B:Begin Mission  |  ESC:Quit",
                15, (h - 22) as i32, Color::RGB(160, 160, 180)).ok();

            return; // Overview renders the background only — no tab bar.
        }
        _ => {}
    }

    // -- For non-Overview tabs, dark background with tab bar --
    // -- Status bar at bottom: shows funds and team size --
    canvas.set_draw_color(Color::RGB(20, 20, 30));
    canvas.fill_rect(Rect::new(0, (h - 50) as i32, w, 50)).ok();
    let funds_text = format!("Funds: ${:>12}  |  Team: {}/8  |  Missions: {}",
        game.game_state.funds, game.game_state.team.len(), game.game_state.missions_completed);
    text.draw(canvas, tc, &funds_text, 15, (h - 35) as i32, Color::RGB(200, 200, 200)).ok();

    // -- Sub-phase tab bar along the top --
    let tab_names = ["1:Hire", "2:Equip", "3:Intel", "4:Contracts", "5:Train"];
    let sub_phases = [
        OfficePhase::HireMercs, OfficePhase::Equipment,
        OfficePhase::Intel, OfficePhase::Contracts, OfficePhase::Training,
    ];

    // Tab background
    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.fill_rect(Rect::new(0, 0, w, 35)).ok();

    // Back to office button
    text.draw_small(canvas, tc, "[ESC] Office", 10, 10, Color::RGB(140, 140, 160)).ok();

    for (i, (sp, name)) in sub_phases.iter().zip(tab_names.iter()).enumerate() {
        let x = 130 + (i as i32) * 130;
        let active = *sp == active_sub;
        let bg = if active { Color::RGB(60, 60, 100) } else { Color::RGB(30, 30, 45) };
        let fg = if active { Color::RGB(255, 255, 200) } else { Color::RGB(140, 140, 140) };
        canvas.set_draw_color(bg);
        canvas.fill_rect(Rect::new(x, 5, 120, 25)).ok();
        text.draw_small(canvas, tc, name, x + 8, 10, fg).ok();
    }

    // -- Main content area depends on active sub-phase --
    let content_y = 50;
    let content_h = h as i32 - 50 - 55;

    match active_sub {
        OfficePhase::Overview => {
            // Handled above with the background image.
        }
        OfficePhase::HireMercs => {
            text.draw_header(canvas, tc, "Mercenary Roster", 20, content_y, Color::RGB(220, 200, 100)).ok();

            // List available mercs from the ruleset, scrollable
            let mut y = content_y + 35;
            let mut count = 0;
            let mut sorted_mercs: Vec<_> = ruleset.mercs.values().collect();
            sorted_mercs.sort_by(|a, b| b.rating.cmp(&a.rating)); // best first

            for merc in sorted_mercs.iter().take(25) {
                // Check if already hired
                let hired = game.game_state.team.iter().any(|m| m.name == merc.name);
                let status_color = if hired {
                    Color::RGB(100, 200, 100) // green = on your team
                } else if merc.avail == 1 {
                    Color::RGB(200, 200, 200) // white = available
                } else {
                    Color::RGB(100, 100, 100) // gray = unavailable
                };

                let status_tag = if hired { "[HIRED]" } else if merc.avail == 0 { "[N/A]" } else { "" };
                let line = format!(
                    "{:<25} RAT:{:>3}  EXP:{:>3}  WSK:{:>3}  AGL:{:>3}  Hire:${:>7}  {}",
                    merc.name, merc.rating, merc.exp, merc.wsk, merc.agl, merc.fee_hire, status_tag
                );
                text.draw_small(canvas, tc, &line, 20, y, status_color).ok();
                y += 16;
                count += 1;
                if y > (content_y + content_h - 20) { break; }
            }

            text.draw_small(canvas, tc,
                &format!("Showing {count}/{} mercs (sorted by rating)", ruleset.mercs.len()),
                20, content_y + content_h, Color::RGB(100, 100, 100),
            ).ok();
        }
        OfficePhase::Equipment => {
            text.draw_header(canvas, tc, "Equipment Catalog", 20, content_y, Color::RGB(220, 200, 100)).ok();
            let mut y = content_y + 35;

            // Show weapons
            text.draw(canvas, tc, "--- WEAPONS ---", 20, y, Color::RGB(180, 140, 80)).ok();
            y += 22;
            let mut sorted_weapons: Vec<_> = ruleset.weapons.values().collect();
            sorted_weapons.sort_by_key(|w| format!("{:?}", w.weapon_type));
            for w in sorted_weapons.iter().take(20) {
                let line = format!(
                    "{:<25} RNG:{:>3}  DMG:{:>2}  PEN:{:>3}  AP:{:>3}  ${:>6}",
                    w.name, w.weapon_range, w.damage_class, w.penetration, w.ap_cost, w.cost
                );
                text.draw_small(canvas, tc, &line, 20, y, Color::RGB(200, 200, 200)).ok();
                y += 14;
                if y > (content_y + content_h - 20) { break; }
            }
        }
        OfficePhase::Contracts => {
            text.draw_header(canvas, tc, "Available Contracts", 20, content_y, Color::RGB(220, 200, 100)).ok();
            let mut y = content_y + 35;

            // Show mission contracts from the ruleset
            let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
            mission_ids.sort();
            for mid in &mission_ids {
                if let Some(mission) = ruleset.missions.get(*mid) {
                    let line = format!(
                        "{}: {} — Advance: ${}, Bonus: ${}",
                        mid, mission.contract.terms, mission.contract.advance, mission.contract.bonus
                    );
                    // Truncate long lines
                    let display = if line.len() > 120 { &line[..120] } else { &line };
                    text.draw_small(canvas, tc, display, 20, y, Color::RGB(200, 200, 200)).ok();
                    y += 18;
                    if y > (content_y + content_h - 20) { break; }
                }
            }
        }
        _ => {
            // Intel, Training — placeholder for now
            let label = format!("{:?}", active_sub);
            text.draw_header(canvas, tc, &label, 20, content_y, Color::RGB(220, 200, 100)).ok();
            text.draw(canvas, tc, "Coming soon...", 20, content_y + 35, Color::RGB(140, 140, 140)).ok();
        }
    }

    // -- Help text --
    text.draw_small(canvas, tc, "ESC: Pause  |  B: Begin Mission",
        (w - 280) as i32, (h - 35) as i32, Color::RGB(100, 100, 120)).ok();
}

// ---------------------------------------------------------------------------
// Travel rendering
// ---------------------------------------------------------------------------

/// Render the travel screen — a simple progress bar.
fn render_travel(canvas: &mut Canvas<Window>, elapsed_ms: u32, _text: &TextRenderer, _tc: &TextureCreator<WindowContext>) {
    let (w, h) = canvas.output_size().unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
    let progress = (elapsed_ms as f32 / 2000.0).min(1.0);
    let bar_width = (w as f32 * 0.6) as u32;
    let bar_x = ((w - bar_width) / 2) as i32;
    let bar_y = (h / 2) as i32;

    // Background bar
    canvas.set_draw_color(Color::RGB(40, 40, 40));
    canvas
        .fill_rect(sdl2::rect::Rect::new(bar_x, bar_y, bar_width, 20))
        .ok();

    // Filled portion
    let fill = (bar_width as f32 * progress) as u32;
    canvas.set_draw_color(Color::RGB(100, 180, 100));
    canvas
        .fill_rect(sdl2::rect::Rect::new(bar_x, bar_y, fill, 20))
        .ok();
}

// ---------------------------------------------------------------------------
// Deployment rendering
// ---------------------------------------------------------------------------

/// Render the deployment screen: placeholder grid + placed merc markers.
fn render_deployment(game: &GameLoop, canvas: &mut Canvas<Window>, selected_unit: usize) {
    render_placeholder_grid(canvas, &game.camera, &game.iso_config);

    // Draw placed mercs as colored squares
    for (i, merc) in game.game_state.team.iter().enumerate() {
        if let Some(pos) = merc.position {
            let iso_tile = TilePos {
                x: pos.x,
                y: pos.y,
            };
            let world = game.iso_config.tile_to_screen(iso_tile);
            let screen = game.camera.world_to_screen(world);

            let color = if i == selected_unit {
                Color::RGB(255, 255, 0) // yellow = currently selected for placement
            } else {
                Color::RGB(0, 200, 0) // green = placed
            };

            canvas.set_draw_color(color);
            canvas
                .fill_rect(sdl2::rect::Rect::new(
                    screen.x as i32 - 8,
                    screen.y as i32 - 8,
                    16,
                    16,
                ))
                .ok();
        }
    }
}

// ---------------------------------------------------------------------------
// Combat rendering
// ---------------------------------------------------------------------------

/// Render the combat screen: map grid, player units, selected-unit highlight,
/// AP bar, and turn indicator.
fn render_combat(game: &GameLoop, canvas: &mut Canvas<Window>, combat: &CombatHandler) {
    // TODO: Replace with TileMapRenderer::render_map() once wired.
    render_placeholder_grid(canvas, &game.camera, &game.iso_config);

    // Draw player units as colored squares
    for merc in &game.game_state.team {
        if !merc.is_alive() {
            continue;
        }
        if let Some(pos) = merc.position {
            let iso_tile = TilePos {
                x: pos.x,
                y: pos.y,
            };
            let world = game.iso_config.tile_to_screen(iso_tile);
            let screen = game.camera.world_to_screen(world);

            let is_selected = combat.selected_unit_id == Some(merc.id);
            let color = if is_selected {
                Color::RGB(255, 255, 100) // bright yellow = selected
            } else {
                Color::RGB(0, 200, 0) // green = friendly
            };

            canvas.set_draw_color(color);
            canvas
                .fill_rect(sdl2::rect::Rect::new(
                    screen.x as i32 - 8,
                    screen.y as i32 - 8,
                    16,
                    16,
                ))
                .ok();

            // AP indicator bar under the selected unit
            if is_selected {
                let ap_frac = if merc.base_aps > 0 {
                    merc.current_ap as f32 / merc.base_aps as f32
                } else {
                    0.0
                };
                let bar_w = (16.0 * ap_frac) as u32;
                canvas.set_draw_color(Color::RGB(0, 100, 255));
                canvas
                    .fill_rect(sdl2::rect::Rect::new(
                        screen.x as i32 - 8,
                        screen.y as i32 + 10,
                        bar_w,
                        3,
                    ))
                    .ok();
            }
        }
    }

    // TODO: Draw enemy units from MissionState.enemy_units as red squares.

    // Turn indicator in top-right corner
    let (w, _h) = canvas.output_size().unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
    let indicator_color = if combat.ai_acting {
        Color::RGB(200, 50, 50) // red = AI acting
    } else {
        Color::RGB(50, 200, 50) // green = player's turn
    };
    canvas.set_draw_color(indicator_color);
    canvas
        .fill_rect(sdl2::rect::Rect::new((w - 30) as i32, 10, 20, 20))
        .ok();
}

// ---------------------------------------------------------------------------
// Extraction rendering
// ---------------------------------------------------------------------------

/// Render the extraction screen: map, extraction zone marker, player units.
fn render_extraction(game: &GameLoop, canvas: &mut Canvas<Window>) {
    render_placeholder_grid(canvas, &game.camera, &game.iso_config);

    // Extraction zone indicator at tile (0,0)
    let exit_tile = TilePos { x: 0, y: 0 };
    let world = game.iso_config.tile_to_screen(exit_tile);
    let screen = game.camera.world_to_screen(world);
    canvas.set_draw_color(Color::RGB(255, 200, 0));
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            screen.x as i32 - 16,
            screen.y as i32 - 16,
            32,
            32,
        ))
        .ok();

    // Player units
    for merc in &game.game_state.team {
        if !merc.is_alive() {
            continue;
        }
        if let Some(pos) = merc.position {
            let t = TilePos {
                x: pos.x,
                y: pos.y,
            };
            let w = game.iso_config.tile_to_screen(t);
            let s = game.camera.world_to_screen(w);
            canvas.set_draw_color(Color::RGB(0, 200, 0));
            canvas
                .fill_rect(sdl2::rect::Rect::new(
                    s.x as i32 - 8,
                    s.y as i32 - 8,
                    16,
                    16,
                ))
                .ok();
        }
    }
}

// ---------------------------------------------------------------------------
// Debrief rendering
// ---------------------------------------------------------------------------

/// Render the debrief screen: large result indicator + "press enter" prompt.
fn render_debrief(canvas: &mut Canvas<Window>, success: bool, _text: &TextRenderer, _tc: &TextureCreator<WindowContext>) {
    let (w, h) = canvas.output_size().unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    let color = if success {
        Color::RGB(50, 180, 50)
    } else {
        Color::RGB(180, 50, 50)
    };

    let rect_w = 300u32;
    let rect_h = 100u32;
    canvas.set_draw_color(color);
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            ((w - rect_w) / 2) as i32,
            ((h - rect_h) / 2) as i32,
            rect_w,
            rect_h,
        ))
        .ok();

    // "Press enter" indicator
    canvas.set_draw_color(Color::RGB(150, 150, 150));
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            ((w - 100) / 2) as i32,
            ((h + rect_h) / 2 + 20) as i32,
            100,
            10,
        ))
        .ok();
}

// ---------------------------------------------------------------------------
// Pause rendering
// ---------------------------------------------------------------------------

/// Render the pause overlay: dark fill + pause icon (two vertical bars).
fn render_pause(canvas: &mut Canvas<Window>, _text: &TextRenderer, _tc: &TextureCreator<WindowContext>) {
    let (w, h) = canvas.output_size().unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    // Dark overlay
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
    canvas
        .fill_rect(sdl2::rect::Rect::new(0, 0, w, h))
        .ok();

    // Pause icon: two vertical bars
    let bar_w = 20u32;
    let bar_h = 60u32;
    let gap = 15i32;
    let cx = (w / 2) as i32;
    let cy = (h / 2) as i32;

    canvas.set_draw_color(Color::RGB(200, 200, 200));
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            cx - gap - bar_w as i32,
            cy - (bar_h / 2) as i32,
            bar_w,
            bar_h,
        ))
        .ok();
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            cx + gap,
            cy - (bar_h / 2) as i32,
            bar_w,
            bar_h,
        ))
        .ok();
}

// ---------------------------------------------------------------------------
// Placeholder grid renderer
// ---------------------------------------------------------------------------

/// Draw a simple isometric diamond grid as a stand-in for the real tile map.
///
/// Renders a 20x20 grid of diamond outlines using the camera and iso config.
/// This lets us test camera scrolling, zoom, and tile picking before wiring up
/// loaded map data through TileMapRenderer.
fn render_placeholder_grid(canvas: &mut Canvas<Window>, camera: &Camera, iso: &IsoConfig) {
    let grid_size = 20;
    canvas.set_draw_color(Color::RGB(60, 60, 60));

    for ty in 0..grid_size {
        for tx in 0..grid_size {
            let tile = TilePos { x: tx, y: ty };
            let world = iso.tile_to_screen(tile);
            let screen = camera.world_to_screen(world);

            let hw = (iso.tile_width / 2.0) * camera.zoom;
            let hh = (iso.tile_height / 2.0) * camera.zoom;
            let cx = screen.x;
            let cy = screen.y;

            // Diamond outline: top -> right -> bottom -> left -> top
            let top = sdl2::rect::Point::new(cx as i32, (cy - hh) as i32);
            let right = sdl2::rect::Point::new((cx + hw) as i32, cy as i32);
            let bottom = sdl2::rect::Point::new(cx as i32, (cy + hh) as i32);
            let left = sdl2::rect::Point::new((cx - hw) as i32, cy as i32);

            canvas.draw_line(top, right).ok();
            canvas.draw_line(right, bottom).ok();
            canvas.draw_line(bottom, left).ok();
            canvas.draw_line(left, top).ok();
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_handler_round_trip() {
        let office = phase_handler_for(&GamePhase::Office(OfficePhase::Overview));
        assert!(matches!(office, PhaseHandler::Office { .. }));

        let travel = phase_handler_for(&GamePhase::Travel);
        assert!(matches!(travel, PhaseHandler::Travel { elapsed_ms: 0 }));

        let deploy = phase_handler_for(&GamePhase::Mission(MissionPhase::Deployment));
        assert!(matches!(deploy, PhaseHandler::Deployment { selected_unit: 0 }));

        let combat = phase_handler_for(&GamePhase::Mission(MissionPhase::Combat));
        assert!(matches!(combat, PhaseHandler::Combat(_)));

        let extract = phase_handler_for(&GamePhase::Mission(MissionPhase::Extraction));
        assert!(matches!(extract, PhaseHandler::Extraction));

        let debrief = phase_handler_for(&GamePhase::Debrief);
        assert!(matches!(debrief, PhaseHandler::Debrief { success: true }));
    }

    #[test]
    fn game_loop_initializes_in_office() {
        let state = GameState::new(500_000);
        let game = GameLoop::new(state);
        assert!(matches!(game.phase_handler, PhaseHandler::Office { .. }));
        assert_eq!(game.camera.viewport_width, WINDOW_WIDTH);
        assert_eq!(game.camera.viewport_height, WINDOW_HEIGHT);
    }

    #[test]
    fn phase_labels_are_unique() {
        let handlers = [
            PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            },
            PhaseHandler::Travel { elapsed_ms: 0 },
            PhaseHandler::Deployment { selected_unit: 0 },
            PhaseHandler::Combat(CombatHandler {
                initiative_order: vec![],
                current_initiative_idx: 0,
                selected_unit_id: None,
                ai_acting: false,
                tab_cycle_index: 0,
            }),
            PhaseHandler::Extraction,
            PhaseHandler::Debrief { success: true },
            PhaseHandler::Debrief { success: false },
        ];

        let labels: Vec<&str> = handlers.iter().map(phase_label).collect();
        // Verify non-debrief labels are all distinct
        for i in 0..5 {
            for j in (i + 1)..5 {
                assert_ne!(labels[i], labels[j], "duplicate label at {i} and {j}");
            }
        }
    }

    #[test]
    fn phase_colors_are_distinct() {
        let handlers = [
            PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            },
            PhaseHandler::Travel { elapsed_ms: 0 },
            PhaseHandler::Combat(CombatHandler {
                initiative_order: vec![],
                current_initiative_idx: 0,
                selected_unit_id: None,
                ai_acting: false,
                tab_cycle_index: 0,
            }),
            PhaseHandler::Debrief { success: true },
            PhaseHandler::Debrief { success: false },
        ];

        let colors: Vec<Color> = handlers.iter().map(phase_background_color).collect();
        // Success and failure debrief must have different colors
        assert_ne!(colors[3], colors[4]);
    }
}
