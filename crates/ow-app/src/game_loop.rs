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

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::mixer::Music;
use sdl2::mouse::MouseButton;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use sdl2::Sdl;
use tracing::{debug, info, trace, warn};

use ow_audio::sfx::{CombatSound, SfxManager};
use ow_audio::voice::VoicePlayer;
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
    Office { sub_phase: OfficePhase },

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
    /// The accountant calls on the video phone to deliver the financial report.
    Debrief {
        /// True if the mission was a success.
        success: bool,
        /// Accumulated time in this phase (ms). Drives the accountant sprite
        /// animation cycling on the video phone.
        anim_elapsed_ms: u32,
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
    /// Current window dimensions (updated on resize).
    pub window_width: u32,
    pub window_height: u32,
    /// Mission-specific IsoConfig (set when map loads, uses actual tile dimensions).
    pub mission_iso: Option<IsoConfig>,
    /// Enemy units for the current mission.
    pub enemies: Vec<ow_core::mission_setup::EnemyUnit>,
    /// Combat message log (max 8 entries, newest at bottom). Color-coded by type.
    pub combat_log: Vec<CombatLogEntry>,
}

/// Maximum number of combat log entries displayed on screen.
const COMBAT_LOG_MAX: usize = 8;

/// A single entry in the combat message log, with color-coding info.
#[derive(Debug, Clone)]
pub struct CombatLogEntry {
    /// The message text to display.
    pub text: String,
    /// The category determines the display color.
    pub kind: CombatLogKind,
}

/// Color categories for combat log entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLogKind {
    /// Player hit on enemy — green.
    PlayerHit,
    /// Enemy hit on player merc — red.
    EnemyHit,
    /// Any miss — gray.
    Miss,
    /// A unit was killed — yellow.
    Kill,
    /// Informational (movement, round changes) — white.
    Info,
}

impl CombatLogKind {
    /// Return the SDL2 color for this log category.
    fn color(self) -> Color {
        match self {
            CombatLogKind::PlayerHit => Color::RGB(80, 220, 80),
            CombatLogKind::EnemyHit => Color::RGB(220, 60, 60),
            CombatLogKind::Miss => Color::RGB(160, 160, 160),
            CombatLogKind::Kill => Color::RGB(255, 220, 50),
            CombatLogKind::Info => Color::RGB(200, 200, 200),
        }
    }
}

/// Push a message to the combat log, trimming to [`COMBAT_LOG_MAX`] entries.
fn log_combat(game: &mut GameLoop, msg: String, kind: CombatLogKind) {
    debug!(combat_log = %msg, "Combat log entry");
    game.combat_log.push(CombatLogEntry { text: msg, kind });
    if game.combat_log.len() > COMBAT_LOG_MAX {
        let excess = game.combat_log.len() - COMBAT_LOG_MAX;
        game.combat_log.drain(..excess);
    }
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
            window_width: WINDOW_WIDTH,
            window_height: WINDOW_HEIGHT,
            mission_iso: None,
            enemies: Vec::new(),
            combat_log: Vec::new(),
        }
    }
}

/// Build the appropriate `PhaseHandler` for a given `GamePhase`.
fn phase_handler_for(phase: &GamePhase) -> PhaseHandler {
    match phase {
        GamePhase::Office(sub) => PhaseHandler::Office { sub_phase: *sub },
        GamePhase::Travel => PhaseHandler::Travel { elapsed_ms: 0 },
        GamePhase::Mission(MissionPhase::Deployment) => {
            PhaseHandler::Deployment { selected_unit: 0 }
        }
        GamePhase::Mission(MissionPhase::Combat) => PhaseHandler::Combat(CombatHandler {
            initiative_order: Vec::new(),
            current_initiative_idx: 0,
            selected_unit_id: None,
            ai_acting: false,
            tab_cycle_index: 0,
        }),
        GamePhase::Mission(MissionPhase::Extraction) => PhaseHandler::Extraction,
        GamePhase::Debrief => PhaseHandler::Debrief { success: true, anim_elapsed_ms: 0 },
    }
}

// ---------------------------------------------------------------------------
// MIDI music playback helpers
// ---------------------------------------------------------------------------

/// Default music volume: 50% of SDL2_mixer's 0–128 range.
/// The original game's MIDI can be grating at full volume.
const MUSIC_VOLUME: i32 = 64;

/// Returns the MIDI track filename (stem only, no extension) appropriate for
/// the given phase, or `None` if no music should play.
///
/// For mission phases (deployment, combat, extraction), the track is selected
/// based on the current mission number (1–9). Falls back to `WOWMIS01` if the
/// mission number is out of range or unknown.
fn music_track_for_phase(handler: &PhaseHandler) -> Option<&'static str> {
    match handler {
        PhaseHandler::Office { .. } => Some("WOWOFICE"),
        PhaseHandler::Travel { .. } => Some("WOWARIVE"),
        PhaseHandler::Deployment { .. } => Some("WOWMIS01"),
        PhaseHandler::Combat(_) => Some("WOWMIS01"),
        PhaseHandler::Extraction => Some("WOWMIS01"),
        PhaseHandler::Debrief { success: true, .. } => Some("WOWDPARW"),
        PhaseHandler::Debrief { success: false, .. } => Some("WOWDPARL"),
        PhaseHandler::Paused { .. } => None, // keep whatever was playing
    }
}

/// Like [`music_track_for_phase`] but uses the mission number (1–9) to pick
/// the correct `WOWMISxx` track instead of always defaulting to 01.
fn music_track_for_phase_with_mission(
    handler: &PhaseHandler,
    mission_num: Option<u32>,
) -> Option<String> {
    match handler {
        PhaseHandler::Deployment { .. } | PhaseHandler::Combat(_) | PhaseHandler::Extraction => {
            let n = mission_num.unwrap_or(1).clamp(1, 9);
            Some(format!("WOWMIS{n:02}"))
        }
        _ => music_track_for_phase(handler).map(String::from),
    }
}

/// Try to load and play a MIDI track, returning the `Music` handle that must
/// be kept alive for the duration of playback. Returns `None` (with a warning
/// logged) if the file is missing or SDL2_mixer can't play it.
fn start_music<'a>(midi_dir: &Path, track_name: &str) -> Option<Music<'a>> {
    let mid_path = midi_dir.join(format!("{track_name}.MID"));
    if !mid_path.exists() {
        warn!(track = track_name, path = %mid_path.display(),
              "MIDI file not found -- skipping music");
        return None;
    }
    match Music::from_file(&mid_path) {
        Ok(music) => {
            Music::set_volume(MUSIC_VOLUME);
            if let Err(e) = music.play(-1) {
                warn!(track = track_name, error = %e,
                      "SDL2_mixer failed to play MIDI -- continuing without music");
                None
            } else {
                info!(
                    track = track_name,
                    volume = MUSIC_VOLUME,
                    "Now playing MIDI track"
                );
                Some(music)
            }
        }
        Err(e) => {
            warn!(track = track_name, error = %e,
                  "SDL2_mixer failed to load MIDI -- continuing without music");
            None
        }
    }
}

/// Stop any currently playing music. Safe to call even if nothing is playing.
fn stop_music() {
    Music::halt();
    debug!("Music halted");
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
        PhaseHandler::Debrief { success, .. } => {
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
        PhaseHandler::Debrief { success, .. } => {
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
    canvas: Canvas<Window>,
    game_state: GameState,
    ruleset: Ruleset,
    data_dir: &std::path::Path,
) -> Result<()> {
    let event_pump = sdl_context
        .event_pump()
        .map_err(|e| anyhow::anyhow!("Failed to get SDL2 event pump: {e}"))?;
    run_game_loop_with_pump(sdl_context, canvas, event_pump, game_state, ruleset, data_dir)
}

/// Game loop variant that accepts an existing event pump (used when
/// the intro cutscenes already created one before handing off).
pub fn run_game_loop_with_pump(
    sdl_context: &Sdl,
    mut canvas: Canvas<Window>,
    mut event_pump: sdl2::EventPump,
    game_state: GameState,
    ruleset: Ruleset,
    data_dir: &std::path::Path,
) -> Result<()> {
    info!(phase = ?game_state.phase, "Starting game loop");

    let mut game = GameLoop::new(game_state);

    // Initialize text rendering — loads a system font for UI text.
    let ttf_context =
        sdl2::ttf::init().map_err(|e| anyhow::anyhow!("SDL2_ttf init failed: {e}"))?;
    let text_renderer = TextRenderer::new(&ttf_context, None)
        .map_err(|e| anyhow::anyhow!("Font loading failed: {e}"))?;
    let texture_creator = canvas.texture_creator();

    // -----------------------------------------------------------------------
    // MIDI music via SDL2_mixer
    // -----------------------------------------------------------------------
    let midi_dir = data_dir.join("WOW").join("MIDI");
    let audio_available = match sdl2::mixer::open_audio(44100, sdl2::mixer::AUDIO_S16LSB, 2, 1024) {
        Ok(()) => {
            info!("SDL2_mixer audio device opened (44100 Hz, S16LSB, stereo)");
            true
        }
        Err(e) => {
            warn!(error = %e, "SDL2_mixer failed to open audio -- continuing without music");
            false
        }
    };

    // Start initial music for whatever phase we launched into.
    let mut current_music_track: Option<String> = None;
    let mut _music_handle: Option<Music> = if audio_available {
        let track = music_track_for_phase(&game.phase_handler);
        if let Some(name) = track {
            let handle = start_music(&midi_dir, name);
            if handle.is_some() {
                current_music_track = Some(name.to_string());
            }
            handle
        } else {
            None
        }
    } else {
        None
    };

    // -----------------------------------------------------------------------
    // Combat SFX — pre-load WAV files from WOW/SND/ as mixer Chunks.
    // Channels 2–7 are reserved for SFX; 0–1 stay free for voice/music.
    // -----------------------------------------------------------------------
    let snd_dir = data_dir.join("WOW").join("SND");
    let mut sfx_manager = SfxManager::new(&snd_dir, audio_available);

    // -----------------------------------------------------------------------
    // Voice line playback — on-demand WAV loading from WOW/WAV/.
    // Uses mixer channel 1 (separate from music and SFX channels 2-7).
    // Voice lines play when hiring a merc or selecting one in combat.
    // -----------------------------------------------------------------------
    let wav_dir = data_dir.join("WOW").join("WAV");
    let mut voice_player: Option<VoicePlayer> = if audio_available {
        Some(VoicePlayer::new(wav_dir))
    } else {
        debug!("Voice player disabled (no audio device)");
        None
    };

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
                info!(
                    width = img.width,
                    height = img.height,
                    "Office background loaded"
                );
                match ow_render::pcx::pcx_to_texture(&img, &texture_creator) {
                    Ok(tex) => Some(tex),
                    Err(e) => {
                        warn!("Failed to create office texture: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                warn!("Failed to load OFFICE.PCX: {e}");
                None
            }
        }
    };

    // -----------------------------------------------------------------------
    // Debrief screen sprites -- accountant + video phone
    // -----------------------------------------------------------------------
    // ACCT.OBJ contains the accountant character sprites (animated on the
    // video phone during the post-mission financial debrief). PHONSPR.OBJ
    // contains the phone scene background frames. Both use the same FLC
    // sprite container format as tilesets and OBJ files.
    //
    // We load all frames at startup and convert them to SDL2 textures so
    // the debrief renderer can just index into them by frame number.
    let (acct_textures, phone_textures) = {
        // Use OFFPIC2.PCX palette -- it is the closest match to the game's
        // master VGA palette and we already loaded it for the office scene.
        let pic_dir = data_dir.join("WOW").join("PIC");
        let palette = {
            let offpic = pic_dir.join("OFFPIC2.PCX");
            match ow_render::palette::load_pcx_palette(&offpic) {
                Ok(pal) => Some(pal),
                Err(e) => {
                    warn!("Failed to load palette for debrief sprites: {e}");
                    None
                }
            }
        };

        let spr_dir = data_dir.join("WOW").join("SPR");

        /// Decode all frames from a sprite sheet into RGBA SDL2 textures.
        /// Returns an empty vec if loading fails -- the renderer will fall
        /// back to the placeholder debrief display.
        fn load_sprite_textures<'a>(
            path: &Path,
            palette: &Option<ow_render::palette::Palette256>,
            tc: &'a TextureCreator<WindowContext>,
        ) -> Vec<Texture<'a>> {
            let pal = match palette {
                Some(p) => p,
                None => {
                    warn!(path = %path.display(),
                          "no palette available -- skipping sprite load");
                    return Vec::new();
                }
            };

            let sheet = match ow_data::sprite::parse_sprite_file(path) {
                Ok(s) => {
                    info!(
                        path = %path.display(),
                        frames = s.file_header.sprite_count,
                        "debrief sprite sheet loaded"
                    );
                    s
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e,
                          "failed to parse debrief sprite sheet");
                    return Vec::new();
                }
            };

            let mut textures = Vec::with_capacity(sheet.frames.len());
            for (i, frame) in sheet.frames.iter().enumerate() {
                let fw = frame.header.width as u32;
                let fh = frame.header.height as u32;
                if fw == 0 || fh == 0 {
                    // Some sprite sheets have empty placeholder frames.
                    trace!(frame = i, "skipping zero-size sprite frame");
                    continue;
                }
                match ow_data::sprite::decode_rle(
                    &frame.compressed_data,
                    frame.header.width,
                    frame.header.height,
                    i,
                ) {
                    Ok(pixels) => {
                        // Brightness boost of 1.5 to compensate for CRT->LCD gamma.
                        let rgba =
                            ow_render::palette::apply_palette_with_brightness(&pixels, pal, 1.5);
                        match tc.create_texture_static(
                            sdl2::pixels::PixelFormatEnum::RGBA32,
                            fw,
                            fh,
                        ) {
                            Ok(mut tex) => {
                                tex.set_blend_mode(sdl2::render::BlendMode::Blend);
                                if let Err(e) = tex.update(None, &rgba, (fw * 4) as usize) {
                                    warn!(frame = i, error = %e,
                                          "failed to upload sprite texture");
                                } else {
                                    textures.push(tex);
                                }
                            }
                            Err(e) => {
                                warn!(frame = i, error = %e,
                                      "failed to create sprite texture");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(frame = i, error = %e, "RLE decode failed for sprite frame");
                    }
                }
            }
            info!(
                path = %path.display(),
                decoded = textures.len(),
                total = sheet.frames.len(),
                "debrief sprite textures ready"
            );
            textures
        }

        let acct_path = spr_dir.join("ACCT.OBJ");
        let phone_path = spr_dir.join("PHONSPR.OBJ");

        let acct = load_sprite_textures(&acct_path, &palette, &texture_creator);
        let phone = load_sprite_textures(&phone_path, &palette, &texture_creator);
        (acct, phone)
    };

    // -- Mission map resources (loaded when entering deployment) --
    // These are Option because they don't exist until a mission starts.
    let mut tile_renderer: Option<ow_render::tile_renderer::TileMapRenderer> = None;
    let mut obj_renderer: Option<ow_render::tile_renderer::TileMapRenderer> = None;
    let mut loaded_map: Option<ow_data::map_loader::GameMap> = None;
    let mut mission_iso_config: Option<IsoConfig> = None;

    // Soldier animation system: all frames from ANIM/JUNGSLD.DAT decoded into
    // textures, indexed by the AnimController's current_frame_index().
    // The COR file maps (action, direction, weapon) → frame ranges.
    let mut soldier_textures: Vec<Option<Texture>> = Vec::new();
    let mut soldier_anim_set: Option<ow_data::animation::AnimationSet> = None;
    // Per-merc animation controllers, keyed by merc index in the team.
    let mut soldier_anims: Vec<ow_render::anim_controller::AnimController> = Vec::new();
    // Backwards compat — kept as fallback if full animation loading fails.
    let mut soldier_texture: Option<Texture> = None;

    // Enemy units generated from mission data. Stored here so they persist
    // across the deployment and combat phases.
    let mut enemy_units: Vec<ow_core::mission_setup::EnemyUnit> = Vec::new();

    let mut last_frame = Instant::now();
    let mut running = true;
    let mut _screenshot_count = 0u32;

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

                // Track window resizes so click coordinates scale correctly.
                Event::Window {
                    win_event: sdl2::event::WindowEvent::Resized(w, h),
                    ..
                } => {
                    game.window_width = w as u32;
                    game.window_height = h as u32;
                    debug!(width = w, height = h, "Window resized");
                }

                // F12 saves a screenshot to disk.
                Event::KeyDown {
                    keycode: Some(Keycode::F12),
                    ..
                } => {
                    save_screenshot(&canvas);
                }

                // ======= DEV HOTKEYS =======
                // F1: Skip to debrief (win current mission instantly)
                Event::KeyDown {
                    keycode: Some(Keycode::F1),
                    ..
                } => {
                    info!("[DEV] F1: Force win → Debrief");
                    game.game_state.set_phase(GamePhase::Debrief);
                    game.phase_handler = PhaseHandler::Debrief { success: true, anim_elapsed_ms: 0 };
                }

                // F2: Skip to office (abort mission, go home)
                Event::KeyDown {
                    keycode: Some(Keycode::F2),
                    ..
                } => {
                    info!("[DEV] F2: Force → Office");
                    game.game_state.set_phase(GamePhase::Office(ow_core::game_state::OfficePhase::Overview));
                    game.phase_handler = PhaseHandler::Office {
                        sub_phase: ow_core::game_state::OfficePhase::Overview,
                    };
                }

                // F3: Skip to deployment (start mission 1 with current team)
                Event::KeyDown {
                    keycode: Some(Keycode::F3),
                    ..
                } => {
                    if game.game_state.team.is_empty() {
                        info!("[DEV] F3: Can't deploy — no mercs hired");
                    } else {
                        info!("[DEV] F3: Force → Deployment (mission 1)");
                        if game.game_state.current_mission.is_none() {
                            game.game_state.current_mission = Some(ow_core::game_state::MissionContext {
                                name: "MSSN01".to_string(),
                                weather: ow_core::weather::Weather::Clear,
                                combat: None,
                                turn_number: 0,
                            });
                        }
                        game.game_state.set_phase(GamePhase::Mission(ow_core::game_state::MissionPhase::Deployment));
                        game.phase_handler = PhaseHandler::Deployment {
                            selected_unit: 0,
                        };
                    }
                }

                // F4: Kill all enemies (instant win condition)
                Event::KeyDown {
                    keycode: Some(Keycode::F4),
                    ..
                } => {
                    info!("[DEV] F4: Kill all enemies");
                    game.enemies.clear();
                }

                // F5: Add $500k funds
                Event::KeyDown {
                    keycode: Some(Keycode::F5),
                    ..
                } => {
                    game.game_state.funds += 500_000;
                    info!("[DEV] F5: +$500k → funds={}", game.game_state.funds);
                }

                // Delegate all other input to the current phase handler
                _ => {
                    handle_phase_input(&mut game, &event, &ruleset, &mut sfx_manager, &mut voice_player);
                }
            }
        }

        if !running {
            break;
        }

        // -- Update --
        update_phase(&mut game, delta_ms, &mut sfx_manager);

        // Tick soldier animation controllers so idle/walk/shoot frames advance.
        for ctrl in soldier_anims.iter_mut() {
            ctrl.update(delta_ms as f32);
        }

        // -- Music transitions on phase change --
        // Compare what we're currently playing to what the new phase wants.
        // If they differ, stop old music and start the new track.
        // Uses mission number for mission-phase track selection (WOWMIS01–09).
        if audio_available {
            let mission_num = game
                .game_state
                .current_mission
                .as_ref()
                .and_then(|m| m.name.strip_prefix("MSSN"))
                .and_then(|n| n.parse::<u32>().ok());
            let wanted = music_track_for_phase_with_mission(&game.phase_handler, mission_num);
            let need_change = match (&wanted, &current_music_track) {
                // Pause: don't touch music at all.
                _ if matches!(game.phase_handler, PhaseHandler::Paused { .. }) => false,
                (Some(w), Some(c)) => w.as_str() != c.as_str(),
                (Some(_), None) => true,
                (None, Some(_)) => true,
                (None, None) => false,
            };
            if need_change {
                stop_music();
                if let Some(track_name) = &wanted {
                    let handle = start_music(&midi_dir, track_name);
                    if handle.is_some() {
                        current_music_track = Some(track_name.clone());
                    } else {
                        current_music_track = None;
                    }
                    _music_handle = handle;
                } else {
                    _music_handle = None;
                    current_music_track = None;
                }
            }
        }

        // -- Load mission map when entering deployment for the first time --
        // We check if we just transitioned to Deployment and haven't loaded a map yet.
        if matches!(game.phase_handler, PhaseHandler::Deployment { .. }) && loaded_map.is_none() {
            // Determine which mission scenario to load from the accepted contract.
            let mission_num = game
                .game_state
                .current_mission
                .as_ref()
                .and_then(|m| m.name.strip_prefix("MSSN"))
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(1);

            info!(mission = mission_num, "Loading mission map for deployment");

            // Load MAP file from WOW/MAPS/SCEN{n}/
            // Try SCEN{n}.MAP first, then SCEN{n}A.MAP (the actual filename varies).
            let scen_dir = data_dir
                .join("WOW")
                .join("MAPS")
                .join(format!("SCEN{mission_num}"));
            let map_path = {
                let try1 = scen_dir.join(format!("SCEN{mission_num}.MAP"));
                let try2 = scen_dir.join(format!("SCEN{mission_num}A.MAP"));
                if try1.exists() {
                    try1
                } else {
                    try2
                }
            };

            match ow_data::map_loader::parse_map(&map_path) {
                Ok(map) => {
                    info!(width = map.width(), height = map.height(),
                          tileset = %map.asset_refs.tileset_path, "Map loaded");

                    // Load the TIL tileset referenced by the MAP's string table.
                    // The MAP references paths like "C:\WOW\SPR\SCEN1\TILSCN01.TIL".
                    // The TIL files live in WOW/SPR/SCEN{n}/, not WOW/MAPS/SCEN{n}/.
                    let til_name =
                        ow_data::map_loader::filename_from_build_path(&map.asset_refs.tileset_path);
                    let spr_scen_dir = data_dir
                        .join("WOW")
                        .join("SPR")
                        .join(format!("SCEN{mission_num}"));
                    let til_path = spr_scen_dir.join(til_name);
                    match ow_data::sprite::parse_sprite_file(&til_path) {
                        Ok(tileset) => {
                            info!(sprites = tileset.file_header.sprite_count, "Tileset loaded");

                            // Load the palette from a PCX in PIC/.
                            // TODO: The game uses a master VGA palette that differs from
                            // individual PCX palettes. For now we use OFFPIC2.PCX which
                            // has the closest match to the terrain colors.
                            let pic_dir = data_dir.join("WOW").join("PIC");
                            let pal_pcx = {
                                // Try OFFPIC2 first (office scene, closest to game palette)
                                let offpic = pic_dir.join("OFFPIC2.PCX");
                                if offpic.exists() {
                                    Some(offpic)
                                } else {
                                    std::fs::read_dir(&pic_dir).ok().and_then(|entries| {
                                        entries
                                            .flatten()
                                            .find(|e| {
                                                e.path().extension().map(|x| x.to_ascii_uppercase())
                                                    == Some("PCX".into())
                                            })
                                            .map(|e| e.path())
                                    })
                                }
                            };
                            if let Some(pcx_path) = pal_pcx {
                                match ow_render::palette::load_pcx_palette(&pcx_path) {
                                    Ok(pal) => {
                                        // Create tile renderer and load textures.
                                        let mut tr = ow_render::tile_renderer::TileMapRenderer::new(
                                            &texture_creator,
                                        );
                                        if let Err(e) = tr.load_tileset(&tileset, &pal) {
                                            warn!("Failed to load tileset textures: {e}");
                                        } else {
                                            let tw = tr.tile_pixel_width() as f32;
                                            let th = tr.tile_pixel_height() as f32;
                                            info!(
                                                tile_w = tw,
                                                tile_h = th,
                                                tiles = tr.tile_count(),
                                                "Tiles ready"
                                            );

                                            // Configure iso projection for the staggered grid.
                                            // Wages of War uses a staggered grid, NOT standard
                                            // diamond iso. Tile dimensions are 128x64 from the exe.
                                            // tile_width = 128 (full tile width, horizontal step)
                                            // tile_height = 64 (full tile height, vertical step)
                                            // Odd rows are offset +64px by tile_to_screen().
                                            let mis_iso = IsoConfig {
                                                tile_width: 128.0,
                                                tile_height: 64.0,
                                                origin_x: 0.0,
                                                origin_y: 0.0,
                                            };
                                            game.mission_iso = Some(IsoConfig {
                                                tile_width: 128.0,
                                                tile_height: 64.0,
                                                origin_x: 0.0,
                                                origin_y: 0.0,
                                            });
                                            mission_iso_config = Some(mis_iso);

                                            // Center the camera on the middle of the 140x72
                                            // staggered grid. Use the initial camera position
                                            // from the MAP file if available, otherwise center
                                            // on the map midpoint.
                                            // Camera position: use MAP's stored position if
                                            // available, otherwise center on the map.
                                            // Row spacing is half tile height (32px) for
                                            // interlocking diamonds.
                                            let mid_x = if map.header.camera_x != 0 {
                                                map.header.camera_x as f32
                                            } else {
                                                (map.width() as f32 / 2.0) * 128.0
                                            };
                                            let mid_y = if map.header.camera_y != 0 {
                                                map.header.camera_y as f32
                                            } else {
                                                (map.height() as f32 / 2.0) * 32.0
                                            };
                                            game.camera.x =
                                                mid_x - (game.window_width as f32 / 2.0);
                                            game.camera.y =
                                                mid_y - (game.window_height as f32 / 2.0);
                                            tile_renderer = Some(tr);

                                            // Load the OBJ sprite sheet for map objects
                                            // (buildings, walls, fences, trees).
                                            // Same sprite container format as TIL, lives
                                            // in the same SPR/SCEN{n}/ directory.
                                            let obj_name =
                                                ow_data::map_loader::filename_from_build_path(
                                                    &map.asset_refs.object_sprite_path,
                                                );
                                            let obj_path = spr_scen_dir.join(obj_name);
                                            if obj_path.exists() {
                                                match ow_data::sprite::parse_sprite_file(&obj_path)
                                                {
                                                    Ok(obj_sheet) => {
                                                        info!(
                                                            sprites = obj_sheet.file_header.sprite_count,
                                                            path = %obj_path.display(),
                                                            "OBJ sprite sheet loaded"
                                                        );
                                                        let mut or = ow_render::tile_renderer::TileMapRenderer::new(&texture_creator);
                                                        if let Err(e) =
                                                            or.load_tileset(&obj_sheet, &pal)
                                                        {
                                                            warn!(
                                                                "Failed to load OBJ textures: {e}"
                                                            );
                                                        } else {
                                                            info!(
                                                                obj_tiles = or.tile_count(),
                                                                obj_w = or.tile_pixel_width(),
                                                                obj_h = or.tile_pixel_height(),
                                                                "OBJ textures ready"
                                                            );
                                                            obj_renderer = Some(or);
                                                        }
                                                    }
                                                    Err(e) => warn!(
                                                        "Failed to load OBJ sheet {obj_name}: {e}"
                                                    ),
                                                }
                                            } else {
                                                warn!(path = %obj_path.display(), "OBJ sprite file not found");
                                            }

                                            // Load soldier animation: COR index + DAT sprite frames.
                                            let anim_dir = data_dir.join("WOW").join("ANIM");
                                            let cor_path = anim_dir.join("JUNGSLD.COR");
                                            let sld_path = anim_dir.join("JUNGSLD.DAT");

                                            if cor_path.exists() {
                                                match ow_data::animation::parse_animation(&cor_path) {
                                                    Ok(anim_set) => {
                                                        info!(entries = anim_set.entries.len(), "COR animation index loaded");
                                                        soldier_anim_set = Some(anim_set);
                                                    }
                                                    Err(e) => warn!("Failed to parse JUNGSLD.COR: {e}"),
                                                }
                                            }

                                            if sld_path.exists() {
                                                match ow_data::sprite::parse_sprite_file(&sld_path) {
                                                    Ok(sld_sheet) => {
                                                        let total = sld_sheet.frames.len();
                                                        let max_frames = total.min(2000);
                                                        info!(total, loading = max_frames, "Decoding soldier frames");
                                                        soldier_textures.clear();
                                                        let mut decoded = 0u32;
                                                        for i in 0..max_frames {
                                                            let frame = &sld_sheet.frames[i];
                                                            let fw = frame.header.width as u32;
                                                            let fh = frame.header.height as u32;
                                                            if fw == 0 || fh == 0 { soldier_textures.push(None); continue; }
                                                            let tex_opt = ow_data::sprite::decode_rle(
                                                                &frame.compressed_data, frame.header.width, frame.header.height, i,
                                                            ).ok().and_then(|pixels| {
                                                                let rgba = ow_render::palette::apply_palette_with_brightness(&pixels, &pal, 1.5);
                                                                let mut tex = texture_creator.create_texture_static(
                                                                    sdl2::pixels::PixelFormatEnum::RGBA32, fw, fh,
                                                                ).ok()?;
                                                                tex.set_blend_mode(sdl2::render::BlendMode::Blend);
                                                                tex.update(None, &rgba, (fw * 4) as usize).ok()?;
                                                                decoded += 1;
                                                                Some(tex)
                                                            });
                                                            soldier_textures.push(tex_opt);
                                                        }
                                                        info!(decoded, "Soldier animation frames ready");

                                                        // Create per-merc AnimControllers in idle pose.
                                                        if let Some(ref anim_set) = soldier_anim_set {
                                                            soldier_anims.clear();
                                                            for _merc in &game.game_state.team {
                                                                let mut ctrl = ow_render::anim_controller::AnimController::new(anim_set.clone());
                                                                ctrl.set_action(
                                                                    ow_render::anim_controller::AnimAction::Idle,
                                                                    ow_render::anim_controller::Direction::S, 1,
                                                                );
                                                                soldier_anims.push(ctrl);
                                                            }
                                                            info!(controllers = soldier_anims.len(), "AnimControllers ready");
                                                        }
                                                    }
                                                    Err(e) => warn!("Failed to load JUNGSLD.DAT: {e}"),
                                                }
                                            } else {
                                                warn!(path = %sld_path.display(), "JUNGSLD.DAT not found");
                                            }
                                        }
                                    }
                                    Err(e) => warn!("Palette error: {e}"),
                                }
                            }
                            // Generate enemy units from mission data.
                            let mission_key = format!("MSSN{mission_num:02}");
                            if let Some(mission_data) = ruleset.missions.get(&mission_key) {
                                let mut rng = rand::thread_rng();
                                // Generate enemies with random positions on the map.
                                let max_player_id =
                                    game.game_state.team.iter().map(|m| m.id).max().unwrap_or(0);
                                let mut next_id = max_player_id + 1000;

                                for (i, rating) in mission_data.enemy_ratings.iter().enumerate() {
                                    use rand::Rng;
                                    // Roll for presence
                                    let roll: u8 = rng.gen_range(0..100);
                                    if roll >= rating.presence_chance {
                                        continue;
                                    }
                                    // Generate enemy with a random position in the upper portion of the map.
                                    let ex: i32 = rng.gen_range(20..180);
                                    let ey: i32 = rng.gen_range(10..100);
                                    let default_weapon = ow_data::mission::EnemyWeapon {
                                        weapon1: -1,
                                        weapon2: -1,
                                        ammo1: 0,
                                        ammo2: 0,
                                        weapon3: -1,
                                        extra: 0,
                                    };
                                    let weapon = mission_data
                                        .enemy_weapons
                                        .get(i)
                                        .unwrap_or(&default_weapon);
                                    let mut enemy = ow_core::mission_setup::EnemyUnit::from_rating(
                                        next_id, rating, weapon,
                                    );
                                    enemy.position = Some(ow_core::merc::TilePos { x: ex, y: ey });
                                    next_id += 1;
                                    enemy_units.push(enemy);
                                }
                                game.enemies = enemy_units.clone();
                                info!(enemies = enemy_units.len(), "Enemies generated for mission");
                            }

                            loaded_map = Some(map);
                        }
                        Err(e) => warn!("Failed to load tileset {til_name}: {e}"),
                    }
                }
                Err(e) => warn!("Failed to load map {}: {e}", map_path.display()),
            }
        }

        // -- Update window dimensions every frame (handles fullscreen, DPI changes,
        // and resize events we might miss). Cheap call, prevents coordinate bugs. --
        let (cw, ch) = canvas.window().size();
        game.window_width = cw;
        game.window_height = ch;

        // -- Render --
        let bg = phase_background_color(&game.phase_handler);
        canvas.set_draw_color(bg);
        canvas.clear();

        render_phase(
            &game,
            &mut canvas,
            &text_renderer,
            &texture_creator,
            &ruleset,
            &office_texture,
            &tile_renderer,
            &obj_renderer,
            &loaded_map,
            &mission_iso_config,
            &soldier_texture,
            &acct_textures,
            &phone_textures,
            &soldier_textures,
            &soldier_anims,
        );

        // Title bar shows the current phase (placeholder for real UI)
        let label = phase_label(&game.phase_handler);
        canvas
            .window_mut()
            .set_title(&format!("Open Wages \u{2014} {label}"))
            .ok();

        canvas.present();

        // -- Frame pacing --
        // Sleep for remaining frame budget to hit ~60 fps.
        let frame_elapsed = now.elapsed().as_millis() as u32;
        if frame_elapsed < TARGET_FRAME_MS {
            std::thread::sleep(std::time::Duration::from_millis(
                (TARGET_FRAME_MS - frame_elapsed) as u64,
            ));
        }
    }

    // Clean up music before exit.
    drop(_music_handle);
    // Drop voice player before closing audio device — cached Chunks
    // must be freed while the mixer is still open.
    drop(voice_player);
    if audio_available {
        stop_music();
        sdl2::mixer::close_audio();
        debug!("SDL2_mixer audio closed");
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
        // If we're in an office sub-screen (not Overview), ESC goes back to
        // the office desk. This is how the original game works — ESC closes
        // the current overlay and returns to the main office scene.
        PhaseHandler::Office { sub_phase } if *sub_phase != OfficePhase::Overview => {
            info!(from = ?sub_phase, "Returning to office overview");
            game.game_state
                .set_phase(GamePhase::Office(OfficePhase::Overview));
            game.phase_handler = PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            };
            true
        }

        // From pause, ESC resumes (not quit — that was too aggressive).
        // Use the window X button or Alt+F4 to actually quit.
        PhaseHandler::Paused { previous } => {
            info!("Resuming from pause");
            let prev = std::mem::replace(
                &mut game.phase_handler,
                PhaseHandler::Office {
                    sub_phase: OfficePhase::Overview,
                },
            );
            if let PhaseHandler::Paused { previous } = prev {
                game.phase_handler = *previous;
            }
            true
        }

        // From the office overview or any other screen, ESC pauses.
        _ => {
            info!("Entering pause");
            let current = std::mem::replace(
                &mut game.phase_handler,
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
fn handle_phase_input(game: &mut GameLoop, event: &Event, ruleset: &Ruleset, sfx: &mut SfxManager, voice: &mut Option<VoicePlayer>) {
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
        Route::Office => handle_office_input(game, event, ruleset, voice),
        Route::Travel => { /* No player input during travel */ }
        Route::Deployment => handle_deployment_input(game, event),
        Route::Combat => handle_combat_input(game, event, sfx, voice),
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
/// Map a mouse click on the office scene to a game action.
///
/// The original office screen is 640x480. We scale mouse coordinates from
/// the actual window size down to 640x480 space, then check which clickable
/// object the player hit. Each object on the desk maps to a game function:
///
/// - Filing cabinet (left side)  → View Files
/// - Fax machine (lower left)    → Contracts (Use Fax)
/// - Calculator (center desk)    → Calculator
/// - Pizza box (center-low desk) → Eat Pizza (easter egg)
/// - Phone (right side)          → Hire Mercs / Arm Mercs
/// - World map (wall, right)     → World Map / Intel
/// - Door (far right)            → Begin Mission
/// - Magazines (desk, left)      → Equipment catalog
fn handle_office_input(game: &mut GameLoop, event: &Event, ruleset: &Ruleset, voice: &mut Option<VoicePlayer>) {
    // Get current sub-phase.
    let current_sub = if let PhaseHandler::Office { sub_phase } = &game.phase_handler {
        *sub_phase
    } else {
        return;
    };
    // Helper: check if a point is inside a rect defined in 640x480 space.
    // We scale the mouse coordinates from window size to 640x480.
    // Scale mouse coords to the 640x480 game coordinate space.
    // On high-DPI displays, SDL2 mouse events use LOGICAL pixels
    // (window size), not physical pixels (canvas output size).
    // We use game.window_width/height (logical) for mouse mapping.
    let check_hit =
        |mx: i32, my: i32, x1: i32, y1: i32, x2: i32, y2: i32, ww: u32, wh: u32| -> bool {
            let sx = (mx as f32 * 640.0 / ww as f32) as i32;
            let sy = (my as f32 * 480.0 / wh as f32) as i32;
            sx >= x1 && sx <= x2 && sy >= y1 && sy <= y2
        };

    match event {
        // Mouse click on the office scene — check which object was clicked.
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let (ww, wh) = (game.window_width, game.window_height);

            // --- HireMercs: clicking a merc row hires or fires them ---
            if current_sub == OfficePhase::HireMercs {
                // The merc list renders starting at y=85px (content_y=50 + header=35).
                // Each row is 16px tall. Match the render order: sorted by rating desc.
                let list_start_y = 85i32;
                let row_h = 16i32;
                let click_y = *y;

                if click_y >= list_start_y {
                    let row = ((click_y - list_start_y) / row_h) as usize;

                    // Build the same sorted merc list as the renderer.
                    let mut sorted_mercs: Vec<_> = ruleset.mercs.values().collect();
                    sorted_mercs.sort_by(|a, b| b.rating.cmp(&a.rating));

                    if let Some(merc) = sorted_mercs.get(row) {
                        let already_hired =
                            game.game_state.team.iter().any(|m| m.name == merc.name);

                        if already_hired {
                            // Fire the merc — remove from team (no refund, like the original).
                            game.game_state.team.retain(|m| m.name != merc.name);
                            info!(name = %merc.name, "Fired mercenary");
                        } else if merc.avail == 1 {
                            // Hire the merc — check funds and team size.
                            if game.game_state.team.len() >= 8 {
                                warn!("Team full (max 8 mercs)");
                            } else if game.game_state.funds < merc.fee_hire as i64 {
                                warn!(name = %merc.name, cost = merc.fee_hire, funds = game.game_state.funds,
                                      "Cannot afford to hire");
                            } else {
                                // Deduct funds and add to team.
                                game.game_state.funds -= merc.fee_hire as i64;
                                let id = game.game_state.team.len() as u32 + 1;
                                let active = ow_core::merc::ActiveMerc::from_data(id, merc);
                                info!(name = %merc.name, cost = merc.fee_hire,
                                      remaining_funds = game.game_state.funds, "Hired mercenary");
                                game.game_state.team.push(active);
                                // Play the mercs voice line on hire — the original game
                                // plays a greeting/intro clip when you add someone to your team.
                                if let Some(vp) = voice.as_mut() {
                                    vp.play(&merc.name);
                                }
                            }
                        } else {
                            info!(name = %merc.name, "Merc unavailable for hire");
                        }
                    }
                }
                return; // Don't fall through to office overview hotspots.
            }

            // --- Contracts: click a mission to accept/switch contracts ---
            if current_sub == OfficePhase::Contracts {
                // Contract list starts at y=107 (content_y=50 + header=35 + accepted_line=22).
                // If no contract is accepted yet, list starts at y=85.
                let has_accepted = game.game_state.current_mission.is_some();
                let list_start_y = if has_accepted { 107i32 } else { 85i32 };
                let row_h = 18i32;
                let click_y = *y;

                if click_y >= list_start_y {
                    let row = ((click_y - list_start_y) / row_h) as usize;

                    // Build sorted mission ID list (same order as render).
                    let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
                    mission_ids.sort();

                    if let Some(mid) = mission_ids.get(row) {
                        if let Some(mission) = ruleset.missions.get(*mid) {
                            // Accept this contract — credit the advance to funds.
                            let already_accepted = game
                                .game_state
                                .current_mission
                                .as_ref()
                                .map(|m| m.name == **mid)
                                .unwrap_or(false);

                            if already_accepted {
                                info!(mission = %mid, "Contract already accepted");
                            } else {
                                // If switching contracts, no refund on old advance.
                                let advance = mission.contract.advance;
                                game.game_state.funds += advance as i64;
                                game.game_state.current_mission =
                                    Some(ow_core::game_state::MissionContext {
                                        name: mid.to_string(),
                                        weather: ow_core::weather::Weather::Clear,
                                        combat: None,
                                        turn_number: 0,
                                    });
                                info!(mission = %mid, advance = advance,
                                      funds = game.game_state.funds, "Contract accepted!");
                            }
                        }
                    }
                }
                return;
            }

            // --- Equipment: clicking a weapon row leases it to the first unarmed merc ---
            if current_sub == OfficePhase::Equipment {
                // Weapon list starts at y=105 (content_y=50 + header=35 + section_header=20).
                // Each row is 14px tall. Match the render order: sorted by weapon_type name.
                let list_start_y = 105i32;
                let row_h = 14i32;
                let click_y = *y;

                if click_y >= list_start_y {
                    let row = ((click_y - list_start_y) / row_h) as usize;

                    // Build the same sorted weapon list as the renderer.
                    let mut sorted_weapons: Vec<_> = ruleset.weapons.values().collect();
                    sorted_weapons.sort_by_key(|w| format!("{:?}", w.weapon_type));

                    if let Some(weapon) = sorted_weapons.get(row) {
                        // Check if there's an unarmed merc to assign to.
                        let unarmed_idx = game
                            .game_state
                            .team
                            .iter()
                            .position(|m| m.inventory.is_empty());

                        if let Some(idx) = unarmed_idx {
                            // Check funds.
                            if game.game_state.funds < weapon.cost as i64 {
                                warn!(weapon = %weapon.name, cost = weapon.cost,
                                      funds = game.game_state.funds, "Cannot afford weapon lease");
                            } else {
                                // Deduct cost and assign weapon to the merc.
                                game.game_state.funds -= weapon.cost as i64;
                                let merc_name = game.game_state.team[idx].name.clone();
                                game.game_state.team[idx].inventory.push(
                                    ow_core::merc::InventoryItem {
                                        name: weapon.name.clone(),
                                        encumbrance: weapon.encumbrance,
                                    },
                                );
                                info!(weapon = %weapon.name, cost = weapon.cost,
                                      merc = %merc_name,
                                      remaining_funds = game.game_state.funds,
                                      "Leased weapon to merc");
                            }
                        } else {
                            warn!(weapon = %weapon.name, "No unarmed mercs to assign weapon to");
                        }
                    }
                }
                return;
            }

            // Check each clickable hotspot (640x480 coords from the original game).
            // Hotspots are checked in priority order — more specific areas first
            // to prevent overlap issues (e.g., phone vs world map).
            //
            // The office layout (from OFFPIC2.PCX):
            //   Top-left: window with desert view
            //   Left: filing cabinet (green, tall)
            //   Center: green desk pad with calculator, coffee mug
            //   Right: white telephone, desk lamp
            //   Far right wall: world map, fax machine on side table
            //   Background: door with "MERCS INC" glass, ceiling fan
            //   Bottom-left: magazines/catalogs on desk
            // Generous hotspots covering the full visual objects on OFFPIC2.
            // The original MAIN.BTN has tiny 22px icon buttons designed for
            // sprite overlays we don't render yet. These bigger rects match
            // what the player visually sees and can click comfortably.
            let sx = (*x as f32 * 640.0 / ww as f32) as i32;
            let sy = (*y as f32 * 480.0 / wh as f32) as i32;
            info!(
                window_x = x,
                window_y = y,
                game_x = sx,
                game_y = sy,
                "Office click"
            );

            // Coordinates measured from 640x480 grid overlay on OFFPIC2.PCX.
            let action = if check_hit(*x, *y, 400, 340, 520, 430, ww, wh) {
                // Phone (right side of desk) → Hire Mercenaries
                Some(("Hire Mercenaries", OfficePhase::HireMercs))
            } else if check_hit(*x, *y, 480, 230, 560, 310, ww, wh) {
                // Fax machine (on side table, far right) → Contracts
                Some(("Contracts (Fax)", OfficePhase::Contracts))
            } else if check_hit(*x, *y, 230, 330, 310, 380, ww, wh) {
                // Calculator (on green desk pad) → Training
                Some(("Training (Calculator)", OfficePhase::Training))
            } else if check_hit(*x, *y, 490, 50, 620, 190, ww, wh) {
                // World map (on wall, upper right) → Intel
                Some(("Mission Intel", OfficePhase::Intel))
            } else if check_hit(*x, *y, 70, 170, 130, 370, ww, wh) {
                // Filing cabinet (left wall) → View Files / Intel
                Some(("View Files (Cabinet)", OfficePhase::Intel))
            } else if check_hit(*x, *y, 100, 360, 220, 430, ww, wh) {
                // Magazines on desk (lower left) → Equipment
                Some(("Equipment (Magazines)", OfficePhase::Equipment))
            } else if check_hit(*x, *y, 240, 40, 370, 250, ww, wh) {
                // Door → Begin Mission (requires hired mercs AND accepted contract)
                if game.game_state.team.is_empty() {
                    warn!("Cannot begin mission: no mercs hired");
                    None
                } else if game.game_state.current_mission.is_none() {
                    warn!("Cannot begin mission: no contract accepted (click fax first)");
                    None
                } else {
                    info!(team_size = game.game_state.team.len(),
                          mission = %game.game_state.current_mission.as_ref().unwrap().name,
                          "Beginning mission");
                    game.game_state.set_phase(GamePhase::Travel);
                    game.phase_handler = PhaseHandler::Travel { elapsed_ms: 0 };
                    None
                }
            } else {
                None
            };

            if let Some((label, sub)) = action {
                info!(action = label, "Office click");
                game.game_state.set_phase(GamePhase::Office(sub));
                game.phase_handler = PhaseHandler::Office { sub_phase: sub };
            }
        }

        // Keyboard shortcuts still work as fallback.
        Event::KeyDown {
            keycode: Some(key), ..
        } => {
            let new_sub = match *key {
                // ESC returns to the overview (office desk scene).
                Keycode::Escape => {
                    // Only go to overview if we're in a sub-screen, not if we're
                    // already at overview (which would trigger the pause handler).
                    if let PhaseHandler::Office { sub_phase } = &game.phase_handler {
                        if *sub_phase != OfficePhase::Overview {
                            Some(OfficePhase::Overview)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Keycode::Num1 => Some(OfficePhase::HireMercs),
                Keycode::Num2 => Some(OfficePhase::Equipment),
                Keycode::Num3 => Some(OfficePhase::Intel),
                Keycode::Num4 => Some(OfficePhase::Contracts),
                Keycode::Num5 => Some(OfficePhase::Training),
                Keycode::U if current_sub == OfficePhase::Equipment => {
                    // Unequip all weapons from all mercs, refunding lease costs.
                    let mut total_refund: i64 = 0;
                    for merc in &mut game.game_state.team {
                        for item in merc.inventory.drain(..) {
                            // Look up the weapon cost for refund.
                            if let Some(weapon) =
                                ruleset.weapons.values().find(|w| w.name == item.name)
                            {
                                total_refund += weapon.cost as i64;
                                info!(weapon = %item.name, refund = weapon.cost,
                                      merc = %merc.name, "Returned leased weapon");
                            } else {
                                info!(item = %item.name, merc = %merc.name,
                                      "Returned item (no cost lookup)");
                            }
                        }
                    }
                    if total_refund > 0 {
                        game.game_state.funds += total_refund;
                        info!(
                            total_refund,
                            funds = game.game_state.funds,
                            "All weapons returned — funds refunded"
                        );
                    } else {
                        info!("No weapons to return");
                    }
                    None
                }
                Keycode::B => {
                    if game.game_state.team.is_empty() {
                        warn!("Cannot begin mission: no mercs hired");
                        None
                    } else if game.game_state.current_mission.is_none() {
                        warn!("Cannot begin mission: no contract accepted");
                        None
                    } else {
                        info!(team_size = game.game_state.team.len(),
                              mission = %game.game_state.current_mission.as_ref().unwrap().name,
                              "Beginning mission");
                        game.game_state.set_phase(GamePhase::Travel);
                        game.phase_handler = PhaseHandler::Travel { elapsed_ms: 0 };
                        None
                    }
                }
                _ => None,
            };

            if let Some(sub) = new_sub {
                debug!(sub_phase = ?sub, "Office sub-phase switch");
                game.game_state.set_phase(GamePhase::Office(sub));
                game.phase_handler = PhaseHandler::Office { sub_phase: sub };
            }
        }
        _ => {}
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
        // WASD / Arrow keys: scroll the camera around the map.
        Event::KeyDown {
            keycode: Some(key), ..
        } if matches!(
            *key,
            Keycode::W
                | Keycode::A
                | Keycode::S
                | Keycode::D
                | Keycode::Up
                | Keycode::Down
                | Keycode::Left
                | Keycode::Right
        ) =>
        {
            let speed = 32.0;
            match *key {
                Keycode::W | Keycode::Up => game.camera.scroll(0.0, -speed),
                Keycode::S | Keycode::Down => game.camera.scroll(0.0, speed),
                Keycode::A | Keycode::Left => game.camera.scroll(-speed, 0.0),
                Keycode::D | Keycode::Right => game.camera.scroll(speed, 0.0),
                _ => {}
            }
        }

        // +/- zoom
        Event::KeyDown {
            keycode: Some(Keycode::Equals),
            ..
        }
        | Event::KeyDown {
            keycode: Some(Keycode::Plus),
            ..
        } => {
            game.camera.zoom_in();
        }
        Event::KeyDown {
            keycode: Some(Keycode::Minus),
            ..
        } => {
            game.camera.zoom_out();
        }

        // Mouse wheel zoom
        Event::MouseWheel { y, .. } => {
            if *y > 0 {
                game.camera.zoom_in();
            } else if *y < 0 {
                game.camera.zoom_out();
            }
        }

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
            // Use mission iso config if available (actual tile dimensions),
            // fall back to default iso config.
            let iso = game.mission_iso.as_ref().unwrap_or(&game.iso_config);
            let tile = iso.screen_to_tile(world);
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
            // Build initiative order: interleave player mercs and enemies.
            // All units sorted by initiative (EXP + WIL) — highest first.
            // This is the core WoW mechanic: NOT I-go-you-go, but all
            // units mixed by initiative regardless of faction.
            let mut init_order: Vec<MercId> = Vec::new();
            for merc in &game.game_state.team {
                if merc.position.is_some() && merc.is_alive() {
                    init_order.push(merc.id);
                }
            }
            for enemy in &game.enemies {
                if enemy.current_hp > 0 && enemy.position.is_some() {
                    init_order.push(enemy.id);
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
fn handle_combat_input(game: &mut GameLoop, event: &Event, sfx: &mut SfxManager, voice: &mut Option<VoicePlayer>) {
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
                    // Play a voice line for the newly-selected merc so the player
                    // gets audio feedback on who they just tabbed to.
                    let sel_id = living[c.tab_cycle_index];
                    if let Some(vp) = voice.as_mut() {
                        if let Some(merc) = game.game_state.team.iter().find(|m| m.id == sel_id) {
                            vp.play(&merc.name);
                        }
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
                let iso = game.mission_iso.as_ref().unwrap_or(&game.iso_config);
                let tile = iso.screen_to_tile(world);
                let target_tile = ow_core::merc::TilePos {
                    x: tile.x,
                    y: tile.y,
                };

                // Check if an enemy is at or near the clicked tile.
                // If so, shoot them. Otherwise, move there.
                let enemy_idx = game.enemies.iter().position(|e| {
                    e.current_hp > 0
                        && e.position
                            .map(|p| {
                                // Click within 2 tiles of an enemy = target them
                                (p.x - target_tile.x).abs() <= 2 && (p.y - target_tile.y).abs() <= 2
                            })
                            .unwrap_or(false)
                });

                if let Some(eidx) = enemy_idx {
                    // SHOOT — deal damage to the enemy!
                    let attacker = game.game_state.team.iter().find(|m| m.id == unit_id);
                    let attacker_name = attacker
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| format!("Unit_{unit_id}"));
                    let wsk = attacker.map(|m| m.wsk).unwrap_or(50);

                    // Simple hit chance based on weapon skill.
                    use rand::Rng;
                    let mut rng = rand::thread_rng();
                    let hit_roll: u32 = rng.gen_range(0..100);
                    let hit_chance = (wsk as u32).min(95);

                    // Collect combat log message after resolving the shot so we
                    // can call log_combat outside the mutable enemy borrow.
                    let log_msg: (String, CombatLogKind);

                    let enemy = &mut game.enemies[eidx];
                    if hit_roll < hit_chance {
                        // Hit! Deal damage based on weapon skill.
                        let damage = rng.gen_range(5..20);
                        let old_hp = enemy.current_hp;
                        enemy.current_hp = enemy.current_hp.saturating_sub(damage);
                        info!(
                            shooter = unit_id,
                            target = %enemy.name,
                            damage,
                            old_hp,
                            new_hp = enemy.current_hp,
                            "HIT! Damage dealt"
                        );

                        // Deduct AP for shooting
                        if let Some(merc) =
                            game.game_state.team.iter_mut().find(|m| m.id == unit_id)
                        {
                            merc.current_ap = merc.current_ap.saturating_sub(8);
                        }

                        if enemy.current_hp == 0 {
                            info!(target = %enemy.name, "Enemy KILLED!");
                            log_msg = (
                                format!("{attacker_name} hits {ename} for {damage} damage! {ename} KILLED!",
                                        ename = enemy.name),
                                CombatLogKind::Kill,
                            );
                        } else {
                            log_msg = (
                                format!("{attacker_name} hits {} for {damage} damage!", enemy.name),
                                CombatLogKind::PlayerHit,
                            );
                        }
                    } else {
                        info!(
                            shooter = unit_id,
                            target = %enemy.name,
                            roll = hit_roll,
                            needed = hit_chance,
                            "MISS!"
                        );
                        log_msg = (
                            format!("{attacker_name} misses {}!", enemy.name),
                            CombatLogKind::Miss,
                        );
                        // Still costs AP to shoot
                        if let Some(merc) =
                            game.game_state.team.iter_mut().find(|m| m.id == unit_id)
                        {
                            merc.current_ap = merc.current_ap.saturating_sub(8);
                        }
                    }

                    // Play gunshot SFX first (always plays on a shot attempt),
                    // then layer a hit/kill sound on top if applicable.
                    sfx.play(CombatSound::Pistol);
                    match log_msg.1 {
                        CombatLogKind::Kill => sfx.play(CombatSound::Kill),
                        CombatLogKind::Miss => sfx.play(CombatSound::Miss),
                        _ => {} // Hit uses just the gunshot
                    }

                    // Push the combat log entry (outside the enemy borrow).
                    log_combat(game, log_msg.0, log_msg.1);
                } else {
                    // MOVE — teleport to the clicked tile, deduct AP.
                    if let Some(merc) = game.game_state.team.iter_mut().find(|m| m.id == unit_id) {
                        // Simple AP cost: 2 per tile (Manhattan distance).
                        let cost = if let Some(old_pos) = merc.position {
                            let dist = (old_pos.x - target_tile.x).unsigned_abs()
                                + (old_pos.y - target_tile.y).unsigned_abs();
                            (dist * 2).min(merc.current_ap)
                        } else {
                            2
                        };
                        merc.current_ap = merc.current_ap.saturating_sub(cost);
                        merc.position = Some(target_tile);
                        info!(
                            name = %merc.name,
                            ap_cost = cost,
                            remaining_ap = merc.current_ap,
                            "Unit moved"
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
        log_combat(game, "--- New Round ---".to_string(), CombatLogKind::Info);
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
            game.phase_handler = PhaseHandler::Debrief { success: true, anim_elapsed_ms: 0 };
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
        // Clear mission state for next contract.
        game.game_state.current_mission = None;
        game.enemies.clear();
        game.combat_log.clear();
        // Reset merc AP for next mission.
        for merc in &mut game.game_state.team {
            merc.reset_ap();
            merc.position = None;
        }
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
fn update_phase(game: &mut GameLoop, delta_ms: u32, sfx: &mut SfxManager) {
    // Snapshot the phase discriminant to avoid borrowing game.phase_handler
    // across the update call.
    enum UpdateRoute {
        Travel,
        Combat,
        Debrief,
        Other,
    }

    let route = match &game.phase_handler {
        PhaseHandler::Travel { .. } => UpdateRoute::Travel,
        PhaseHandler::Combat(_) => UpdateRoute::Combat,
        PhaseHandler::Debrief { .. } => UpdateRoute::Debrief,
        _ => UpdateRoute::Other,
    };

    match route {
        UpdateRoute::Travel => update_travel(game, delta_ms),
        UpdateRoute::Combat => update_combat(game, delta_ms, sfx),
        UpdateRoute::Debrief => {
            // Tick the accountant animation timer so sprite frames cycle
            // on the video phone during the debrief screen.
            if let PhaseHandler::Debrief { anim_elapsed_ms, .. } = &mut game.phase_handler {
                *anim_elapsed_ms = anim_elapsed_ms.saturating_add(delta_ms);
            }
        }
        UpdateRoute::Other => {
            // Office, Deployment, Extraction, Paused:
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
fn update_combat(game: &mut GameLoop, _delta_ms: u32, sfx: &mut SfxManager) {
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
                // AI decision: find the nearest player merc and shoot them.
                // If no merc in range, move toward the nearest one.
                //
                // We collect snapshot data (name, position, wsk) to avoid
                // holding borrows across log_combat / advance_initiative calls.
                let enemy_snapshot = game
                    .enemies
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| (e.name.clone(), e.current_hp, e.position, e.wsk));

                if let Some((enemy_name, enemy_hp, enemy_pos_opt, enemy_wsk)) = enemy_snapshot {
                    if enemy_hp == 0 {
                        // Dead enemy, skip turn
                        advance_initiative(game);
                    } else if let Some(enemy_pos) = enemy_pos_opt {
                        // Find nearest living player merc
                        let nearest_merc = game
                            .game_state
                            .team
                            .iter()
                            .filter(|m| m.is_alive() && m.position.is_some())
                            .min_by_key(|m| {
                                let mp = m.position.unwrap();
                                (mp.x - enemy_pos.x).abs() + (mp.y - enemy_pos.y).abs()
                            })
                            .map(|m| (m.id, m.name.clone(), m.position.unwrap()));

                        if let Some((target_id, target_name, tp)) = nearest_merc {
                            let dist = (tp.x - enemy_pos.x).abs() + (tp.y - enemy_pos.y).abs();

                            if dist <= 15 {
                                // In range — SHOOT!
                                use rand::Rng;
                                let mut rng = rand::thread_rng();
                                let hit_chance = (enemy_wsk as u32).min(80);
                                let roll: u32 = rng.gen_range(0..100);

                                // Enemy fires — play gunshot SFX regardless of hit/miss.
                                sfx.play(CombatSound::Rifle);

                                if roll < hit_chance {
                                    let damage = rng.gen_range(3..15);
                                    if let Some(merc) =
                                        game.game_state.team.iter_mut().find(|m| m.id == target_id)
                                    {
                                        merc.current_hp = merc.current_hp.saturating_sub(damage);
                                        info!(
                                            enemy = %enemy_name,
                                            target = %target_name,
                                            damage,
                                            remaining_hp = merc.current_hp,
                                            "Enemy HIT player merc!"
                                        );
                                        if merc.current_hp == 0 {
                                            sfx.play(CombatSound::Kill);
                                            log_combat(game,
                                                format!("{enemy_name} hits {target_name} for {damage} damage! {target_name} KILLED!"),
                                                CombatLogKind::Kill);
                                        } else {
                                            sfx.play(CombatSound::Hit);
                                            log_combat(game,
                                                format!("{enemy_name} hits {target_name} for {damage} damage!"),
                                                CombatLogKind::EnemyHit);
                                        }
                                    }
                                } else {
                                    info!(enemy = %enemy_name, "Enemy MISSED!");
                                    sfx.play(CombatSound::Miss);
                                    log_combat(
                                        game,
                                        format!("{enemy_name} misses {target_name}!"),
                                        CombatLogKind::Miss,
                                    );
                                }
                            } else {
                                // Too far — move toward the target
                                let dx = (tp.x - enemy_pos.x).signum() * 3;
                                let dy = (tp.y - enemy_pos.y).signum() * 3;
                                let new_pos = ow_core::merc::TilePos {
                                    x: enemy_pos.x + dx,
                                    y: enemy_pos.y + dy,
                                };
                                if let Some(e) = game.enemies.iter_mut().find(|e| e.id == id) {
                                    e.position = Some(new_pos);
                                }
                                log_combat(
                                    game,
                                    format!("{enemy_name} moves toward your team"),
                                    CombatLogKind::Info,
                                );
                            }
                        }
                        advance_initiative(game);
                    } else {
                        advance_initiative(game);
                    }
                } else {
                    advance_initiative(game);
                }
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
    let all_dead =
        !game.game_state.team.is_empty() && game.game_state.team.iter().all(|m| !m.is_alive());

    if all_dead {
        warn!("All player mercs killed -- mission failed");
        log_combat(
            game,
            "ALL MERCS DOWN -- MISSION FAILED!".to_string(),
            CombatLogKind::Kill,
        );
        game.game_state.set_phase(GamePhase::Debrief);
        game.phase_handler = PhaseHandler::Debrief { success: false, anim_elapsed_ms: 0 };
        return;
    }

    // Victory: all enemies eliminated — transition to extraction then debrief.
    let all_enemies_dead =
        !game.enemies.is_empty() && game.enemies.iter().all(|e| e.current_hp == 0);

    if all_enemies_dead {
        info!("All enemies eliminated — MISSION COMPLETE!");
        log_combat(
            game,
            "All enemies eliminated -- MISSION COMPLETE!".to_string(),
            CombatLogKind::Kill,
        );
        // Credit the mission bonus to funds.
        if let Some(ref mission_ctx) = game.game_state.current_mission {
            info!(mission = %mission_ctx.name, "Mission successful, transitioning to debrief");
        }
        game.game_state.missions_completed += 1;

        // Credit bonus from the contract.
        // The advance was already credited when the contract was accepted.
        // Now add the completion bonus.
        let mission_key = game
            .game_state
            .current_mission
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_default();
        // We don't have the ruleset here, so we'll add a flat bonus for now.
        // TODO: Look up actual bonus from ruleset.
        let bonus = 200_000i64;
        game.game_state.funds += bonus;
        info!(
            bonus,
            total_funds = game.game_state.funds,
            "Mission bonus credited"
        );

        game.game_state.set_phase(GamePhase::Debrief);
        game.phase_handler = PhaseHandler::Debrief { success: true, anim_elapsed_ms: 0 };
    }
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
    tile_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    obj_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    loaded_map: &Option<ow_data::map_loader::GameMap>,
    mission_iso: &Option<IsoConfig>,
    soldier_texture: &Option<Texture>,
    acct_textures: &[Texture],
    phone_textures: &[Texture],
    soldier_textures: &[Option<Texture>],
    soldier_anims: &[ow_render::anim_controller::AnimController],
) {
    match &game.phase_handler {
        PhaseHandler::Office { sub_phase } => {
            render_office(game, canvas, *sub_phase, text, tc, ruleset, office_bg)
        }
        PhaseHandler::Travel { elapsed_ms } => render_travel(canvas, *elapsed_ms, text, tc),
        PhaseHandler::Deployment { .. } => {
            render_mission_map(
                game,
                canvas,
                tile_renderer,
                obj_renderer,
                loaded_map,
                mission_iso,
                text,
                tc,
                &game.enemies,
                soldier_texture,
                soldier_textures,
                soldier_anims,
            );
        }
        PhaseHandler::Combat(_) => {
            render_mission_map(
                game,
                canvas,
                tile_renderer,
                obj_renderer,
                loaded_map,
                mission_iso,
                text,
                tc,
                &game.enemies,
                soldier_texture,
                soldier_textures,
                soldier_anims,
            );
        }
        PhaseHandler::Extraction => render_extraction(game, canvas),
        PhaseHandler::Debrief { success, anim_elapsed_ms } => {
            render_debrief(game, canvas, *success, *anim_elapsed_ms, text, tc, acct_textures, phone_textures)
        }
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
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

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

            let funds_text = format!(
                "Funds: ${:>12}  |  Team: {}/8  |  Missions: {}",
                game.game_state.funds,
                game.game_state.team.len(),
                game.game_state.missions_completed
            );
            text.draw(
                canvas,
                tc,
                &funds_text,
                15,
                (h - 45) as i32,
                Color::RGB(220, 220, 220),
            )
            .ok();
            text.draw_small(
                canvas,
                tc,
                "1:Hire  2:Equip  3:Intel  4:Contracts  5:Train  |  B:Begin Mission  |  ESC:Quit",
                15,
                (h - 22) as i32,
                Color::RGB(160, 160, 180),
            )
            .ok();

            // DEBUG: Draw labeled hotspot overlays so we can see where the
            // click regions are and fix them. Remove this once hotspots are correct.
            //
            // IMPORTANT: Use game.window_width/height (logical pixels from SDL2
            // mouse events), NOT canvas.output_size() (physical pixels). On high-DPI
            // displays these differ by the scale factor, causing misalignment.
            // Print dimensions to stdout for DPI debugging.
            // DEBUG: Hotspot overlays using window logical size (same as click handler).
            let ww = game.window_width as f32;
            let wh = game.window_height as f32;
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            let hotspots: &[((i32, i32, i32, i32), &str, Color)] = &[
                (
                    (400, 340, 520, 430),
                    "HIRE (Phone)",
                    Color::RGBA(255, 50, 50, 100),
                ),
                (
                    (480, 230, 560, 310),
                    "CONTRACTS (Fax)",
                    Color::RGBA(50, 50, 255, 100),
                ),
                (
                    (490, 50, 620, 190),
                    "INTEL (Map)",
                    Color::RGBA(255, 255, 50, 100),
                ),
                (
                    (70, 170, 130, 370),
                    "FILES (Cabinet)",
                    Color::RGBA(50, 255, 50, 100),
                ),
                (
                    (100, 360, 220, 430),
                    "EQUIP (Mags)",
                    Color::RGBA(50, 255, 50, 100),
                ),
                (
                    (230, 330, 310, 380),
                    "TRAIN (Calc)",
                    Color::RGBA(255, 50, 255, 100),
                ),
                (
                    (240, 40, 370, 250),
                    "MISSION (Door)",
                    Color::RGBA(255, 150, 0, 100),
                ),
            ];
            for &((x1, y1, x2, y2), label, color) in hotspots {
                // Scale 640x480 → window size. Uses same math as check_hit (inverse).
                let sx1 = (x1 as f32 * ww / 640.0) as i32;
                let sy1 = (y1 as f32 * wh / 480.0) as i32;
                let sx2 = (x2 as f32 * ww / 640.0) as i32;
                let sy2 = (y2 as f32 * wh / 480.0) as i32;
                canvas.set_draw_color(color);
                canvas
                    .fill_rect(Rect::new(sx1, sy1, (sx2 - sx1) as u32, (sy2 - sy1) as u32))
                    .ok();
                canvas.set_draw_color(Color::RGB(255, 255, 255));
                canvas
                    .draw_rect(Rect::new(sx1, sy1, (sx2 - sx1) as u32, (sy2 - sy1) as u32))
                    .ok();
                text.draw_small(
                    canvas,
                    tc,
                    label,
                    sx1 + 4,
                    sy1 + 4,
                    Color::RGB(255, 255, 255),
                )
                .ok();
            }
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            return; // Overview renders the background only — no tab bar.
        }
        _ => {}
    }

    // -- For non-Overview tabs, dark background with tab bar --
    // -- Status bar at bottom: shows funds and team size --
    canvas.set_draw_color(Color::RGB(20, 20, 30));
    canvas.fill_rect(Rect::new(0, (h - 50) as i32, w, 50)).ok();
    let funds_text = format!(
        "Funds: ${:>12}  |  Team: {}/8  |  Missions: {}",
        game.game_state.funds,
        game.game_state.team.len(),
        game.game_state.missions_completed
    );
    text.draw(
        canvas,
        tc,
        &funds_text,
        15,
        (h - 35) as i32,
        Color::RGB(200, 200, 200),
    )
    .ok();

    // -- Sub-phase tab bar along the top --
    let tab_names = ["1:Hire", "2:Equip", "3:Intel", "4:Contracts", "5:Train"];
    let sub_phases = [
        OfficePhase::HireMercs,
        OfficePhase::Equipment,
        OfficePhase::Intel,
        OfficePhase::Contracts,
        OfficePhase::Training,
    ];

    // Tab background
    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.fill_rect(Rect::new(0, 0, w, 35)).ok();

    // Back to office button
    text.draw_small(
        canvas,
        tc,
        "[ESC] Office",
        10,
        10,
        Color::RGB(140, 140, 160),
    )
    .ok();

    for (i, (sp, name)) in sub_phases.iter().zip(tab_names.iter()).enumerate() {
        let x = 130 + (i as i32) * 130;
        let active = *sp == active_sub;
        let bg = if active {
            Color::RGB(60, 60, 100)
        } else {
            Color::RGB(30, 30, 45)
        };
        let fg = if active {
            Color::RGB(255, 255, 200)
        } else {
            Color::RGB(140, 140, 140)
        };
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
            text.draw_header(
                canvas,
                tc,
                "Mercenary Roster",
                20,
                content_y,
                Color::RGB(220, 200, 100),
            )
            .ok();

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

                let status_tag = if hired {
                    "[HIRED]"
                } else if merc.avail == 0 {
                    "[N/A]"
                } else {
                    ""
                };
                let line = format!(
                    "{:<25} RAT:{:>3}  EXP:{:>3}  WSK:{:>3}  AGL:{:>3}  Hire:${:>7}  {}",
                    merc.name, merc.rating, merc.exp, merc.wsk, merc.agl, merc.fee_hire, status_tag
                );
                text.draw_small(canvas, tc, &line, 20, y, status_color).ok();
                y += 16;
                count += 1;
                if y > (content_y + content_h - 20) {
                    break;
                }
            }

            text.draw_small(
                canvas,
                tc,
                &format!(
                    "Showing {count}/{} mercs (sorted by rating)",
                    ruleset.mercs.len()
                ),
                20,
                content_y + content_h,
                Color::RGB(100, 100, 100),
            )
            .ok();
        }
        OfficePhase::Equipment => {
            text.draw_header(
                canvas,
                tc,
                "Equipment Catalog — Click weapon to lease",
                20,
                content_y,
                Color::RGB(220, 200, 100),
            )
            .ok();

            // Left pane: weapon list (clickable)
            let mut y = content_y + 35;
            text.draw(
                canvas,
                tc,
                "--- AVAILABLE WEAPONS ---",
                20,
                y,
                Color::RGB(180, 140, 80),
            )
            .ok();
            y += 20;
            let mut sorted_weapons: Vec<_> = ruleset.weapons.values().collect();
            sorted_weapons.sort_by_key(|w| format!("{:?}", w.weapon_type));
            // Collect names of all currently leased weapons for highlighting.
            let leased_names: Vec<String> = game
                .game_state
                .team
                .iter()
                .flat_map(|m| m.inventory.iter().map(|i| i.name.clone()))
                .collect();

            for w in sorted_weapons.iter().take(25) {
                let leased_count = leased_names.iter().filter(|n| *n == &w.name).count();
                let tag = if leased_count > 0 {
                    format!(" [x{}]", leased_count)
                } else {
                    String::new()
                };
                let affordable = game.game_state.funds >= w.cost as i64;
                let color = if leased_count > 0 {
                    Color::RGB(100, 200, 100) // green = already leased
                } else if !affordable {
                    Color::RGB(120, 80, 80) // dim red = can't afford
                } else {
                    Color::RGB(200, 200, 200) // white = available
                };
                let line = format!(
                    "{:<22} RNG:{:>2} DMG:{:>2} PEN:{:>2} AP:{:>2} ${:>5}{}",
                    w.name, w.weapon_range, w.damage_class, w.penetration, w.ap_cost, w.cost, tag
                );
                text.draw_small(canvas, tc, &line, 20, y, color).ok();
                y += 14;
                if y > (content_y + content_h - 40) {
                    break;
                }
            }

            // Right pane: your team with their equipment
            let team_x = (w / 2) as i32 + 20;
            let mut ty = content_y + 35;
            text.draw(
                canvas,
                tc,
                "--- YOUR TEAM ---",
                team_x,
                ty,
                Color::RGB(100, 180, 100),
            )
            .ok();
            ty += 20;
            if game.game_state.team.is_empty() {
                text.draw_small(
                    canvas,
                    tc,
                    "No mercs hired yet",
                    team_x,
                    ty,
                    Color::RGB(140, 140, 140),
                )
                .ok();
            } else {
                for merc in &game.game_state.team {
                    let equip_info = if merc.inventory.is_empty() {
                        "  [UNARMED]".to_string()
                    } else {
                        format!(
                            "  [{}]",
                            merc.inventory
                                .iter()
                                .map(|i| i.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    let line = format!("{}{}", merc.name, equip_info);
                    let color = if merc.inventory.is_empty() {
                        Color::RGB(200, 100, 100) // Red = unarmed
                    } else {
                        Color::RGB(100, 200, 100) // Green = armed
                    };
                    text.draw_small(canvas, tc, &line, team_x, ty, color).ok();
                    ty += 16;
                }
            }

            // Equipment instructions
            let armed_count = game
                .game_state
                .team
                .iter()
                .filter(|m| !m.inventory.is_empty())
                .count();
            let team_count = game.game_state.team.len();
            let equip_status = format!(
                "Click weapon to lease → assigned to first unarmed merc  |  U: Return all weapons  |  Armed: {}/{}",
                armed_count, team_count
            );
            text.draw_small(
                canvas,
                tc,
                &equip_status,
                20,
                content_y + content_h,
                Color::RGB(140, 140, 100),
            )
            .ok();
        }
        OfficePhase::Contracts => {
            text.draw_header(
                canvas,
                tc,
                "Available Contracts — Click to Accept",
                20,
                content_y,
                Color::RGB(220, 200, 100),
            )
            .ok();
            let mut y = content_y + 35;

            // Show which contract is currently accepted, if any.
            let accepted_id = game
                .game_state
                .current_mission
                .as_ref()
                .map(|m| m.name.clone());
            if let Some(ref aid) = accepted_id {
                text.draw(
                    canvas,
                    tc,
                    &format!("ACCEPTED: {} — Press B or click door to deploy!", aid),
                    20,
                    y,
                    Color::RGB(100, 255, 100),
                )
                .ok();
                y += 22;
            }

            // Show mission contracts from the ruleset.
            // Accepted contract shown in green, others in white.
            let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
            mission_ids.sort();
            for mid in &mission_ids {
                if let Some(mission) = ruleset.missions.get(*mid) {
                    let is_accepted = accepted_id.as_deref() == Some(mid.as_str());
                    let color = if is_accepted {
                        Color::RGB(100, 255, 100) // green = accepted
                    } else {
                        Color::RGB(200, 200, 200) // white = available
                    };
                    let tag = if is_accepted { " [ACCEPTED]" } else { "" };
                    let terms = if mission.contract.terms.len() > 60 {
                        &mission.contract.terms[..60]
                    } else {
                        &mission.contract.terms
                    };
                    let line = format!(
                        "{}: {}... Adv:${} Bon:${}{}",
                        mid, terms, mission.contract.advance, mission.contract.bonus, tag
                    );
                    text.draw_small(canvas, tc, &line, 20, y, color).ok();
                    y += 18;
                    if y > (content_y + content_h - 20) {
                        break;
                    }
                }
            }
        }
        _ => {
            // Intel, Training — placeholder for now
            let label = format!("{:?}", active_sub);
            text.draw_header(canvas, tc, &label, 20, content_y, Color::RGB(220, 200, 100))
                .ok();
            text.draw(
                canvas,
                tc,
                "Coming soon...",
                20,
                content_y + 35,
                Color::RGB(140, 140, 140),
            )
            .ok();
        }
    }

    // -- Help text --
    text.draw_small(
        canvas,
        tc,
        "ESC: Pause  |  B: Begin Mission",
        (w - 280) as i32,
        (h - 35) as i32,
        Color::RGB(100, 100, 120),
    )
    .ok();
}

// ---------------------------------------------------------------------------
// Travel rendering
// ---------------------------------------------------------------------------

/// Render the travel screen — a simple progress bar.
fn render_travel(
    canvas: &mut Canvas<Window>,
    elapsed_ms: u32,
    _text: &TextRenderer,
    _tc: &TextureCreator<WindowContext>,
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
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
/// Render the mission map using real tile sprites if loaded, or the placeholder grid.
/// Used by both Deployment and Combat phases.
fn render_mission_map(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    tile_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    obj_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    loaded_map: &Option<ow_data::map_loader::GameMap>,
    mission_iso: &Option<IsoConfig>,
    _text: &TextRenderer,
    _tc: &TextureCreator<WindowContext>,
    enemies: &[ow_core::mission_setup::EnemyUnit],
    soldier_texture: &Option<Texture>,
    soldier_textures: &[Option<Texture>],
    soldier_anims: &[ow_render::anim_controller::AnimController],
) {
    // If we have real tile data, render the actual map. Otherwise fall back
    // to the wireframe placeholder grid.
    if let (Some(tr), Some(map), Some(iso)) = (tile_renderer, loaded_map, mission_iso) {
        tr.render_map(canvas, map, &game.camera, iso);

        // === OBJ sprite pass (Cell Word 5) ===
        // Objects (buildings, trees, walls, fences) are stored in Cell Word 5
        // with an 8-bit object_id (0=none, 1-255=OBJ sprite index).
        // This replaces the old hack of subtracting 100 from overlay indices.
        if let Some(or) = obj_renderer {
            let (min_x, min_y, max_x, max_y) = game.camera.visible_tile_bounds(iso);
            let min_x = min_x.max(0) as usize;
            let min_y = min_y.max(0) as usize;
            let max_x = (max_x as usize).min(map.width().saturating_sub(1));
            let max_y = (max_y as usize).min(map.height().saturating_sub(1));

            let obj_pw = or.tile_pixel_width() as f32;
            let obj_ph = or.tile_pixel_height() as f32;
            // OBJ sprites are often taller than terrain tiles (e.g. 128x128 vs 128x64).
            // Offset upward so the bottom of the OBJ sprite sits on the terrain surface.
            let tile_h = 64.0_f32; // staggered grid tile height
            let y_offset_base = obj_ph - tile_h;

            let mut objs_drawn: u32 = 0;

            for ty in min_y..=max_y {
                for tx in min_x..=max_x {
                    // Access the full MapCell to read object_id from Word 5.
                    let cell = match map.get_cell(tx, ty) {
                        Some(c) => c,
                        None => continue,
                    };

                    // object_id == 0 or 255 means no object in this cell.
                    // The game uses 0xFF as the "empty" sentinel (10079/10080 cells
                    // have object_id=255 in a typical map).
                    if cell.object_id == 0 || cell.object_id == 255 {
                        continue;
                    }

                    let obj_idx = cell.object_id as usize;
                    let obj_tex = match or.get_texture(obj_idx) {
                        Some(t) => t,
                        None => continue,
                    };

                    // Position the OBJ sprite at this cell's screen location.
                    let world_pos = iso.tile_to_screen(TilePos {
                        x: tx as i32,
                        y: ty as i32,
                    });
                    let screen_pos = game.camera.world_to_screen(world_pos);

                    // Draw at top-left of tile position, offset up by the
                    // height difference so OBJ sprites sit on the terrain.
                    let draw_x = screen_pos.x;
                    let draw_y = screen_pos.y - (y_offset_base * game.camera.zoom);

                    let dst_w = (obj_pw * game.camera.zoom) as u32;
                    let dst_h = (obj_ph * game.camera.zoom) as u32;

                    let dst = Rect::new(draw_x as i32, draw_y as i32, dst_w, dst_h);
                    if let Err(e) = canvas.copy(obj_tex, None, dst) {
                        trace!(tx, ty, obj_idx, error = %e, "OBJ sprite draw failed");
                    }
                    objs_drawn += 1;
                }
            }

            trace!(objs_drawn, "OBJ pass complete (Cell Word 5)");
        }
    } else {
        render_placeholder_grid(canvas, &game.camera, &game.iso_config);
    }

    // Draw placed mercs as colored diamonds on the map.
    let iso = mission_iso.as_ref().unwrap_or(&game.iso_config);

    // Get selected unit ID if in combat
    let selected_id = match &game.phase_handler {
        PhaseHandler::Combat(ch) => ch.selected_unit_id,
        _ => None,
    };

    for merc in &game.game_state.team {
        if !merc.is_alive() {
            continue;
        }
        if let Some(pos) = merc.position {
            let iso_tile = TilePos { x: pos.x, y: pos.y };
            let world = iso.tile_to_screen(iso_tile);
            let screen = game.camera.world_to_screen(world);

            let is_selected = selected_id == Some(merc.id);

            // Try animated frame first (from COR/DAT system), then fall
            // back to the single static texture if animations aren't loaded.
            let merc_idx = game.game_state.team.iter().position(|m| m.id == merc.id);
            let anim_frame_tex = merc_idx
                .and_then(|idx| soldier_anims.get(idx))
                .map(|ctrl| ctrl.current_frame_index() as usize)
                .and_then(|fi| soldier_textures.get(fi))
                .and_then(|opt| opt.as_ref());

            let tex_to_draw = anim_frame_tex.or(soldier_texture.as_ref());

            if let Some(sld_tex) = tex_to_draw {
                // Draw the sprite frame at tile position. Frames are 128x138
                // with the soldier figure inside an isometric footprint.
                let sprite_w = 128.0;
                let sprite_h = 138.0;
                let draw_w = (sprite_w * game.camera.zoom) as u32;
                let draw_h = (sprite_h * game.camera.zoom) as u32;
                let draw_x = screen.x;
                let draw_y = screen.y - ((sprite_h - 64.0) * game.camera.zoom);

                let dst = Rect::new(draw_x as i32, draw_y as i32, draw_w, draw_h);

                // Check if the animation requires horizontal mirroring.
                let mirror = merc_idx
                    .and_then(|idx| soldier_anims.get(idx))
                    .map(|ctrl| ctrl.mirror_horizontal())
                    .unwrap_or(false);

                if mirror {
                    canvas.copy_ex(sld_tex, None, dst, 0.0, None, true, false).ok();
                } else {
                    canvas.copy(sld_tex, None, dst).ok();
                }

                // Selection highlight
                if is_selected {
                    canvas.set_draw_color(Color::RGB(255, 255, 0));
                    canvas.draw_rect(Rect::new(
                        draw_x as i32 - 1,
                        draw_y as i32 - 1,
                        draw_w + 2,
                        draw_h + 2,
                    )).ok();
                }
            } else {
                // Fallback: colored squares when no soldier sprite is loaded
                let color = if is_selected {
                    Color::RGB(255, 255, 0)
                } else {
                    Color::RGB(0, 220, 0)
                };

                canvas.set_draw_color(color);
                canvas
                    .fill_rect(Rect::new(screen.x as i32 - 6, screen.y as i32 - 6, 12, 12))
                    .ok();
                canvas.set_draw_color(Color::RGB(0, 0, 0));
                canvas
                    .draw_rect(Rect::new(screen.x as i32 - 6, screen.y as i32 - 6, 12, 12))
                    .ok();
            }
        }
    }

    // Draw enemy units — fog of war hides enemies beyond 20 tiles from all mercs.
    let fow_range = 20i32;
    for enemy in enemies {
        if enemy.current_hp == 0 {
            continue;
        }
        if let Some(pos) = enemy.position {
            let seen = game.game_state.team.iter().any(|m| {
                m.is_alive()
                    && m.position
                        .map(|mp| (mp.x - pos.x).abs() + (mp.y - pos.y).abs() <= fow_range)
                        .unwrap_or(false)
            });
            if !seen {
                continue;
            }
            let iso_tile = TilePos { x: pos.x, y: pos.y };
            let world = iso.tile_to_screen(iso_tile);
            let screen = game.camera.world_to_screen(world);

            // Red for enemies
            canvas.set_draw_color(Color::RGB(220, 30, 30));
            canvas
                .fill_rect(Rect::new(screen.x as i32 - 5, screen.y as i32 - 5, 10, 10))
                .ok();
            canvas.set_draw_color(Color::RGB(0, 0, 0));
            canvas
                .draw_rect(Rect::new(screen.x as i32 - 5, screen.y as i32 - 5, 10, 10))
                .ok();
        }
    }

    // -- Combat HUD: bottom panel + combat log + turn indicator --
    if matches!(game.phase_handler, PhaseHandler::Combat(_)) {
        let (w, h) = (game.window_width, game.window_height);

        // ---- Turn indicator at top of screen ----
        let is_ai = match &game.phase_handler {
            PhaseHandler::Combat(c) => c.ai_acting,
            _ => false,
        };

        // Semi-transparent banner at top center
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
        let banner_w: u32 = 220;
        let banner_x = (w as i32 - banner_w as i32) / 2;
        canvas.fill_rect(Rect::new(banner_x, 4, banner_w, 28)).ok();
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        if is_ai {
            _text
                .draw(
                    canvas,
                    _tc,
                    "ENEMY TURN",
                    banner_x + 10,
                    8,
                    Color::RGB(220, 50, 50),
                )
                .ok();
        } else {
            _text
                .draw(
                    canvas,
                    _tc,
                    "YOUR TURN",
                    banner_x + 10,
                    8,
                    Color::RGB(50, 220, 50),
                )
                .ok();
        }

        // ---- Dark panel at bottom ----
        let panel_height: u32 = 80;
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 200));
        canvas
            .fill_rect(Rect::new(
                0,
                h as i32 - panel_height as i32,
                w,
                panel_height,
            ))
            .ok();
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        // Thin top border on the panel
        canvas.set_draw_color(Color::RGB(80, 80, 80));
        canvas
            .draw_line(
                sdl2::rect::Point::new(0, h as i32 - panel_height as i32),
                sdl2::rect::Point::new(w as i32, h as i32 - panel_height as i32),
            )
            .ok();

        // ---- Selected unit info (left side) ----
        if let Some(sel_id) = selected_id {
            if let Some(merc) = game.game_state.team.iter().find(|m| m.id == sel_id) {
                let info = format!(
                    "{} | HP: {}/{} | AP: {}/{} | Tab=Next  Click=Move  E=EndTurn",
                    merc.name, merc.current_hp, merc.max_hp, merc.current_ap, merc.base_aps,
                );
                _text
                    .draw(
                        canvas,
                        _tc,
                        &info,
                        15,
                        h as i32 - (panel_height as i32) + 10,
                        Color::RGB(220, 220, 220),
                    )
                    .ok();
            }
        } else {
            _text
                .draw(
                    canvas,
                    _tc,
                    "No unit selected | Tab=Next  Enter=NextPhase",
                    15,
                    h as i32 - (panel_height as i32) + 10,
                    Color::RGB(180, 180, 180),
                )
                .ok();
        }

        // ---- Combat log (right side of bottom panel) ----
        // Draw up to COMBAT_LOG_MAX entries, small text, right-aligned area.
        let log_x = (w as i32 / 2) + 40; // Right half of the panel
        let log_start_y = h as i32 - (panel_height as i32) + 6;
        let line_h = 9i32; // Tight spacing for small text

        for (i, entry) in game.combat_log.iter().enumerate() {
            let y_pos = log_start_y + (i as i32) * line_h;
            let color = entry.kind.color();
            _text
                .draw_small(canvas, _tc, &entry.text, log_x, y_pos, color)
                .ok();
        }
    }

    // -- Deployment phase HUD --
    if matches!(game.phase_handler, PhaseHandler::Deployment { .. }) {
        let (w, h) = (game.window_width, game.window_height);
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 200));
        canvas.fill_rect(Rect::new(0, h as i32 - 40, w, 40)).ok();
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        let placed = game
            .game_state
            .team
            .iter()
            .filter(|m| m.position.is_some())
            .count();
        let total = game.game_state.team.len();
        let msg = format!(
            "DEPLOYMENT: Click map to place mercs ({placed}/{total} placed) | Enter=Start Combat"
        );
        _text
            .draw(
                canvas,
                _tc,
                &msg,
                15,
                h as i32 - 28,
                Color::RGB(220, 200, 100),
            )
            .ok();
    }

    // -- Minimap: overview in the bottom-right corner --
    // The minimap shows the map in top-down grid view (not isometric) since
    // an isometric minimap would be diamond-shaped and harder to read.
    // Each tile = 1 pixel, colored by terrain type.
    if let Some(map) = loaded_map {
        let (win_w, win_h) = (game.window_width, game.window_height);

        // Scale minimap to fit — 140x72 tiles at 1px each.
        let mm_w = map.width() as u32;
        let mm_h = map.height() as u32;
        let mm_x = win_w as i32 - mm_w as i32 - 15;
        let mm_y = win_h as i32 - mm_h as i32 - 15;

        // Semi-transparent background
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
        canvas
            .fill_rect(Rect::new(mm_x - 3, mm_y - 3, mm_w + 6, mm_h + 6))
            .ok();
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        // Draw each tile as a colored pixel based on its sprite index.
        for ty in 0..map.height() {
            for tx in 0..map.width() {
                if let Some(tile) = map.get_tile(tx, ty) {
                    let sid = tile.layer0() as u32;
                    let color = if sid == 0 {
                        Color::RGB(25, 45, 20)
                    } else if sid < 50 {
                        Color::RGB(35, 65, 30)
                    } else if sid < 150 {
                        Color::RGB(45, 75, 35)
                    } else if sid < 250 {
                        Color::RGB(55, 85, 45)
                    } else if sid < 350 {
                        Color::RGB(70, 60, 40)
                    } else {
                        Color::RGB(40, 55, 75)
                    };
                    canvas.set_draw_color(color);
                    canvas
                        .draw_point(sdl2::rect::Point::new(mm_x + tx as i32, mm_y + ty as i32))
                        .ok();
                }
            }
        }

        // Player mercs as bright green dots
        for merc in &game.game_state.team {
            if let Some(pos) = merc.position {
                canvas.set_draw_color(Color::RGB(0, 255, 0));
                canvas
                    .fill_rect(Rect::new(mm_x + pos.x, mm_y + pos.y, 3, 3))
                    .ok();
            }
        }

        // Border
        canvas.set_draw_color(Color::RGB(120, 120, 120));
        canvas
            .draw_rect(Rect::new(mm_x - 3, mm_y - 3, mm_w + 6, mm_h + 6))
            .ok();

        // "MINIMAP" label
        _text
            .draw_small(
                canvas,
                _tc,
                "MAP",
                mm_x,
                mm_y - 14,
                Color::RGB(180, 180, 180),
            )
            .ok();
    }
}

fn render_deployment(game: &GameLoop, canvas: &mut Canvas<Window>, selected_unit: usize) {
    render_placeholder_grid(canvas, &game.camera, &game.iso_config);

    // Draw placed mercs as colored squares
    for (i, merc) in game.game_state.team.iter().enumerate() {
        if let Some(pos) = merc.position {
            let iso_tile = TilePos { x: pos.x, y: pos.y };
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
            let iso_tile = TilePos { x: pos.x, y: pos.y };
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
    let (w, _h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
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
            let t = TilePos { x: pos.x, y: pos.y };
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

/// Render the debrief screen with the video phone showing the accountant.
///
/// The original game shows the accountant calling on a video phone to
/// deliver the post-mission financial report. PHONSPR.OBJ frames render
/// the phone scene background; ACCT.OBJ frames animate the accountant
/// character talking on screen. The financial text overlay sits to the
/// right of the phone.
///
/// Layout (at 1280x720):
///   Left half:  phone background + animated accountant sprite
///   Right half: battle results + financial report text
fn render_debrief(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    success: bool,
    anim_elapsed_ms: u32,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    acct_textures: &[Texture],
    phone_textures: &[Texture],
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    // Dark background -- the original game uses a dark charcoal backdrop
    // behind the phone scene to focus attention on the video call.
    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.clear();

    // -----------------------------------------------------------------------
    // Left side: Video phone with animated accountant
    // -----------------------------------------------------------------------
    // The phone occupies roughly the left 40% of the screen. We draw the
    // PHONSPR background first, then layer the accountant animation on top.

    // Milliseconds per animation frame -- controls how fast the accountant
    // cycles through talking/gesturing sprites. 200ms gives a natural feel
    // for a "talking head" animation without looking jittery.
    const ACCT_FRAME_PERIOD_MS: u32 = 200;

    // Phone scene background -- use frame 0 as the static backdrop.
    // The phone sprite is drawn centered in the left portion of the screen.
    let phone_area_w = (w * 2 / 5) as i32; // left 40% of screen
    if !phone_textures.is_empty() {
        let phone_tex = &phone_textures[0];
        let query = phone_tex.query();
        // Scale the phone scene to fit nicely in the left panel.
        // Maintain aspect ratio, fitting to the available height.
        let scale = ((h as f32 - 60.0) / query.height as f32).min(
            (phone_area_w as f32 - 40.0) / query.width as f32,
        );
        let draw_w = (query.width as f32 * scale) as u32;
        let draw_h = (query.height as f32 * scale) as u32;
        let draw_x = (phone_area_w as u32 / 2).saturating_sub(draw_w / 2) as i32 + 20;
        let draw_y = ((h / 2).saturating_sub(draw_h / 2)) as i32;
        canvas
            .copy(
                phone_tex,
                None,
                Some(Rect::new(draw_x, draw_y, draw_w, draw_h)),
            )
            .ok();

        // Accountant animation -- cycle through ACCT.OBJ frames on top of
        // the phone scene. The accountant is composited at the center of
        // the phone screen area (offset slightly to match the phone bezel).
        if !acct_textures.is_empty() {
            // Show frame 0 — the base accountant portrait. Frames 1+ are
            // mouth visemes driven by VLS lip-sync data during audio playback.
            // TODO: Wire up VLS viseme timeline to swap frames during voice playback.
            let frame_idx = 0;
            let acct_tex = &acct_textures[frame_idx];
            let aq = acct_tex.query();
            // Scale accountant to fit inside the phone screen area.
            // The accountant should be roughly 60% of the phone's dimensions
            // to leave room for the phone bezel/frame.
            let acct_scale = (draw_h as f32 * 0.6 / aq.height as f32).min(
                draw_w as f32 * 0.6 / aq.width as f32,
            );
            let acct_w = (aq.width as f32 * acct_scale) as u32;
            let acct_h = (aq.height as f32 * acct_scale) as u32;
            // Center the accountant on the phone screen.
            let acct_x = draw_x + (draw_w / 2) as i32 - (acct_w / 2) as i32;
            let acct_y = draw_y + (draw_h / 2) as i32 - (acct_h / 2) as i32;
            canvas
                .copy(
                    acct_tex,
                    None,
                    Some(Rect::new(acct_x, acct_y, acct_w, acct_h)),
                )
                .ok();
        }
    } else {
        // Fallback: no phone sprites loaded -- draw a placeholder phone frame
        // so the screen is not completely empty on the left side.
        let phone_rect = Rect::new(40, 80, (phone_area_w - 60) as u32, h - 160);
        canvas.set_draw_color(Color::RGB(30, 35, 50));
        canvas.fill_rect(phone_rect).ok();
        canvas.set_draw_color(Color::RGB(60, 70, 90));
        canvas.draw_rect(phone_rect).ok();
        text.draw(
            canvas,
            tc,
            "[ VIDEO PHONE ]",
            phone_rect.x() + 20,
            phone_rect.y() + 20,
            Color::RGB(100, 120, 160),
        )
        .ok();

        // Placeholder accountant indicator
        if acct_textures.is_empty() {
            text.draw(
                canvas,
                tc,
                "[ ACCOUNTANT ]",
                phone_rect.x() + 20,
                phone_rect.y() + 50,
                Color::RGB(80, 100, 140),
            )
            .ok();
        }
    }

    // -----------------------------------------------------------------------
    // Right side: Mission results + financial report
    // -----------------------------------------------------------------------
    // The text sits to the right of the phone, starting at ~45% of screen width.
    let text_x = (w * 9 / 20) as i32;

    // Title banner
    let title = if success {
        "MISSION COMPLETE"
    } else {
        "MISSION FAILED"
    };
    let title_color = if success {
        Color::RGB(80, 255, 80)
    } else {
        Color::RGB(255, 80, 80)
    };
    text.draw_header(canvas, tc, title, text_x, 40, title_color)
        .ok();

    // -- Battle Results --
    let mut y = 100i32;
    let survived = game.game_state.team.iter().filter(|m| m.is_alive()).count();
    let total = game.game_state.team.len();
    let killed = game.enemies.iter().filter(|e| e.current_hp == 0).count();
    let total_enemies = game.enemies.len();

    text.draw(canvas, tc, "BATTLE RESULTS", text_x, y, Color::RGB(180, 180, 100))
        .ok();
    y += 25;
    text.draw(
        canvas, tc,
        &format!("  Mercs survived:      {survived}/{total}"),
        text_x, y, Color::RGB(200, 200, 200),
    ).ok();
    y += 20;
    text.draw(
        canvas, tc,
        &format!("  Enemies eliminated:  {killed}/{total_enemies}"),
        text_x, y, Color::RGB(200, 200, 200),
    ).ok();
    y += 20;

    let kia = total - survived;
    let wia = game
        .game_state
        .team
        .iter()
        .filter(|m| m.is_alive() && m.current_hp < m.max_hp)
        .count();
    text.draw(
        canvas, tc,
        &format!("  KIA: {}  WIA: {}", kia, wia),
        text_x, y,
        if kia > 0 { Color::RGB(255, 100, 100) } else { Color::RGB(100, 200, 100) },
    ).ok();
    y += 35;

    // -- Financial Report (the accountant's video phone summary) --
    text.draw(canvas, tc, "FINANCIAL REPORT", text_x, y, Color::RGB(180, 180, 100))
        .ok();
    y += 25;

    let advance = 324_000i64; // TODO: get from accepted contract
    let bonus = if success { 200_000i64 } else { 0 };
    let hiring_costs = game.game_state.team.len() as i64 * 50_000; // approximate
    let medical = wia as i64 * 79_000; // WIA medical costs
    let death_insurance = kia as i64 * 89_000; // KIA death payouts
    let total_income = advance + bonus;
    let total_expenses = hiring_costs + medical + death_insurance;
    let profit = total_income - total_expenses;

    text.draw(
        canvas, tc,
        &format!("  Contract advance:    ${:>12}", advance),
        text_x, y, Color::RGB(150, 200, 150),
    ).ok();
    y += 18;
    if success {
        text.draw(
            canvas, tc,
            &format!("  Completion bonus:    ${:>12}", bonus),
            text_x, y, Color::RGB(150, 200, 150),
        ).ok();
        y += 18;
    }
    text.draw(
        canvas, tc,
        &format!("  Hiring costs:       -${:>12}", hiring_costs),
        text_x, y, Color::RGB(200, 150, 150),
    ).ok();
    y += 18;
    if medical > 0 {
        text.draw(
            canvas, tc,
            &format!("  Medical (WIA):      -${:>12}", medical),
            text_x, y, Color::RGB(200, 150, 150),
        ).ok();
        y += 18;
    }
    if death_insurance > 0 {
        text.draw(
            canvas, tc,
            &format!("  Death insurance:    -${:>12}", death_insurance),
            text_x, y, Color::RGB(255, 100, 100),
        ).ok();
        y += 18;
    }
    text.draw(
        canvas, tc,
        "  ─────────────────────────────",
        text_x, y, Color::RGB(100, 100, 100),
    ).ok();
    y += 18;
    let profit_color = if profit >= 0 {
        Color::RGB(100, 255, 100)
    } else {
        Color::RGB(255, 100, 100)
    };
    text.draw(
        canvas, tc,
        &format!("  NET PROFIT:          ${:>12}", profit),
        text_x, y, profit_color,
    ).ok();
    y += 25;
    text.draw(
        canvas, tc,
        &format!("  Current funds:       ${:>12}", game.game_state.funds),
        text_x, y, Color::RGB(200, 200, 200),
    ).ok();
    y += 35;

    text.draw(
        canvas, tc,
        "Press ENTER to return to office",
        text_x, y, Color::RGB(150, 150, 180),
    ).ok();
}

// ---------------------------------------------------------------------------
// Pause rendering
// ---------------------------------------------------------------------------

/// Render the pause overlay: dark fill + pause icon (two vertical bars).
fn render_pause(
    canvas: &mut Canvas<Window>,
    _text: &TextRenderer,
    _tc: &TextureCreator<WindowContext>,
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    // Dark overlay
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
    canvas.fill_rect(sdl2::rect::Rect::new(0, 0, w, h)).ok();

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
        assert!(matches!(
            deploy,
            PhaseHandler::Deployment { selected_unit: 0 }
        ));

        let combat = phase_handler_for(&GamePhase::Mission(MissionPhase::Combat));
        assert!(matches!(combat, PhaseHandler::Combat(_)));

        let extract = phase_handler_for(&GamePhase::Mission(MissionPhase::Extraction));
        assert!(matches!(extract, PhaseHandler::Extraction));

        let debrief = phase_handler_for(&GamePhase::Debrief);
        assert!(matches!(debrief, PhaseHandler::Debrief { success: true, .. }));
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
            PhaseHandler::Debrief { success: true, anim_elapsed_ms: 0 },
            PhaseHandler::Debrief { success: false, anim_elapsed_ms: 0 },
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
            PhaseHandler::Debrief { success: true, anim_elapsed_ms: 0 },
            PhaseHandler::Debrief { success: false, anim_elapsed_ms: 0 },
        ];

        let colors: Vec<Color> = handlers.iter().map(phase_background_color).collect();
        // Success and failure debrief must have different colors
        assert_ne!(colors[3], colors[4]);
    }
}

// ---------------------------------------------------------------------------
// Screenshot — F12 saves the current frame to disk as BMP
// ---------------------------------------------------------------------------

/// Save the current canvas contents to a BMP file.
/// Files are named screenshot_001.bmp, screenshot_002.bmp, etc.
fn save_screenshot(canvas: &Canvas<Window>) {
    // Find the next available screenshot number.
    let mut num = 1u32;
    loop {
        let path = format!("screenshot_{num:03}.bmp");
        if !std::path::Path::new(&path).exists() {
            // Read pixels from the canvas in its native format.
            let (w, h) = canvas.output_size().unwrap_or((1280, 720));
            match canvas.read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24) {
                Ok(pixels) => {
                    // RGB24 = 3 bytes per pixel, no alpha confusion.
                    match sdl2::surface::Surface::from_data_pixelmasks(
                        &mut pixels.clone(),
                        w,
                        h,
                        w * 3,
                        &sdl2::pixels::PixelMasks {
                            bpp: 24,
                            rmask: 0xFF0000,
                            gmask: 0x00FF00,
                            bmask: 0x0000FF,
                            amask: 0,
                        },
                    ) {
                        Ok(surface) => match surface.save_bmp(&path) {
                            Ok(()) => info!("Screenshot saved: {path}"),
                            Err(e) => warn!("Failed to save screenshot: {e}"),
                        },
                        Err(e) => warn!("Failed to create screenshot surface: {e}"),
                    }
                }
                Err(e) => warn!("Failed to read pixels for screenshot: {e}"),
            }
            break;
        }
        num += 1;
        if num > 999 {
            warn!("Too many screenshots (>999)");
            break;
        }
    }
}
