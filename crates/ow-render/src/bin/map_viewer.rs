//! # map-viewer — Interactive isometric map viewer
//!
//! Developer tool for rendering and navigating Wages of War scenario maps.
//! Loads a MAP file, resolves its TIL tileset, decodes all tile sprites,
//! and renders the isometric tile grid with camera controls.
//!
//! ## Usage
//!
//! ```text
//! map-viewer --data-dir <path-to-WOW-folder> [--mission <number>]
//! ```
//!
//! ## Controls
//!
//! - **WASD / Arrow keys** — scroll the camera
//! - **+ / -** — zoom in / out
//! - **ESC** — quit

use std::path::{Path, PathBuf};

use clap::Parser;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

use ow_data::map_loader;
use ow_render::camera::Camera;
use ow_render::iso_math::IsoConfig;
use ow_render::palette::load_pcx_palette;
use ow_render::tile_renderer::TileMapRenderer;

/// Interactive isometric map viewer for Wages of War scenarios.
#[derive(Parser, Debug)]
#[command(name = "map-viewer", about = "Render and navigate WoW scenario maps")]
struct Args {
    /// Path to the game's data directory (the folder containing the WOW/ subfolder,
    /// or the WOW/ folder itself).
    #[arg(long)]
    data_dir: PathBuf,

    /// Mission/scenario number to load (default: 1).
    /// Loads MAPS/SCEN{n}/SCEN{n}.MAP (or SCEN{n}A.MAP as fallback).
    #[arg(long, default_value = "1")]
    mission: u32,
}

/// Dark grey background — makes transparent tile regions clearly visible.
const BG_COLOR: Color = Color::RGB(30, 30, 30);

/// Window dimensions.
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

/// Camera scroll speed in world-space pixels per key press.
const SCROLL_SPEED: f32 = 32.0;

fn main() -> anyhow::Result<()> {
    // Initialize tracing with env-filter support.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    info!(
        data_dir = %args.data_dir.display(),
        mission = args.mission,
        "starting map viewer"
    );

    // -----------------------------------------------------------------------
    // Resolve the WOW/ base directory
    // -----------------------------------------------------------------------
    let wow_dir = resolve_wow_dir(&args.data_dir)?;
    info!(wow_dir = %wow_dir.display(), "resolved WOW data directory");

    // -----------------------------------------------------------------------
    // Load the palette from the first .PCX file in WOW/PIC/
    // -----------------------------------------------------------------------
    let palette = load_palette_from_pic_dir(&wow_dir)?;

    // -----------------------------------------------------------------------
    // Load the MAP file for the specified mission
    // -----------------------------------------------------------------------
    let (map, map_path) = load_mission_map(&wow_dir, args.mission)?;
    info!(
        map = %map_path.display(),
        width = map.width(),
        height = map.height(),
        tileset_ref = %map.asset_refs.tileset_path,
        "map loaded"
    );

    // -----------------------------------------------------------------------
    // Resolve and load the TIL tileset referenced by the MAP
    // -----------------------------------------------------------------------
    let tileset_filename = map_loader::filename_from_build_path(&map.asset_refs.tileset_path);
    info!(filename = tileset_filename, "resolving tileset");

    let scen_dir = map_path.parent().unwrap();
    let tileset_path = find_file_case_insensitive(scen_dir, tileset_filename).ok_or_else(|| {
        anyhow::anyhow!(
            "tileset '{}' not found in {}",
            tileset_filename,
            scen_dir.display()
        )
    })?;

    info!(path = %tileset_path.display(), "loading tileset");
    let tileset = ow_data::sprite::parse_sprite_file(&tileset_path)?;
    info!(sprites = tileset.file_header.sprite_count, "tileset loaded");

    // -----------------------------------------------------------------------
    // SDL2 init
    // -----------------------------------------------------------------------
    let sdl_context = sdl2::init().map_err(|e| anyhow::anyhow!("SDL2 init failed: {e}"))?;
    let video = sdl_context
        .video()
        .map_err(|e| anyhow::anyhow!("SDL2 video init failed: {e}"))?;

    let window = video
        .window(
            &format!("Open Wages — Map Viewer — Mission {}", args.mission),
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        )
        .position_centered()
        .build()
        .map_err(|e| anyhow::anyhow!("window creation failed: {e}"))?;

    let mut canvas = window
        .into_canvas()
        .accelerated()
        .present_vsync()
        .build()
        .map_err(|e| anyhow::anyhow!("canvas creation failed: {e}"))?;

    let texture_creator = canvas.texture_creator();

    // -----------------------------------------------------------------------
    // Load tileset into GPU textures
    // -----------------------------------------------------------------------
    let mut tile_renderer = TileMapRenderer::new(&texture_creator);
    tile_renderer
        .load_tileset(&tileset, &palette)
        .map_err(|e| anyhow::anyhow!("failed to load tileset textures: {e}"))?;

    info!(
        tile_count = tile_renderer.tile_count(),
        tile_w = tile_renderer.tile_pixel_width(),
        tile_h = tile_renderer.tile_pixel_height(),
        "tileset textures loaded into GPU"
    );

    // -----------------------------------------------------------------------
    // Configure isometric projection
    // -----------------------------------------------------------------------
    // The IsoConfig defines how tile coordinates map to world-space pixel
    // coordinates. tile_width and tile_height control the spacing between
    // tile anchor points in the isometric diamond grid.
    //
    // For Wages of War, tiles are 128x63 pixel sprites, but the isometric
    // grid spacing uses the diamond footprint, not the full sprite dimensions.
    // The diamond footprint is the tile_width x tile_height of the grid cell.
    //
    // We use the actual tile sprite width for the horizontal spacing and half
    // the sprite height for vertical spacing (standard 2:1 isometric ratio
    // adapted to the actual tile art dimensions).
    let tile_w = tile_renderer.tile_pixel_width() as f32;
    let tile_h = tile_renderer.tile_pixel_height() as f32;

    let iso = IsoConfig {
        tile_width: tile_w,
        tile_height: tile_h,
        origin_x: (map.width() as f32 / 2.0) * (tile_w / 2.0),
        origin_y: 0.0,
    };

    debug!(
        tile_width = iso.tile_width,
        tile_height = iso.tile_height,
        origin_x = iso.origin_x,
        origin_y = iso.origin_y,
        "isometric projection configured"
    );

    // -----------------------------------------------------------------------
    // Camera setup
    // -----------------------------------------------------------------------
    let mut camera = Camera::new(WINDOW_WIDTH, WINDOW_HEIGHT);
    // Start with the camera roughly centered on the map.
    // The origin is at the top of the diamond; we offset to show the map center.
    camera.x = iso.origin_x - (WINDOW_WIDTH as f32 / 2.0);
    camera.y = 0.0;

    let mut event_pump = sdl_context
        .event_pump()
        .map_err(|e| anyhow::anyhow!("event pump failed: {e}"))?;

    let mut needs_redraw = true;

    // -----------------------------------------------------------------------
    // Main loop
    // -----------------------------------------------------------------------
    info!("entering main loop — WASD/arrows to scroll, +/- to zoom, ESC to quit");

    'main_loop: loop {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'main_loop,

                Event::KeyDown {
                    keycode: Some(keycode),
                    ..
                } => {
                    // Scroll speed adjusts with zoom so movement feels consistent.
                    let speed = SCROLL_SPEED / camera.zoom;

                    match keycode {
                        // Scroll controls: WASD and arrow keys
                        Keycode::W | Keycode::Up => {
                            camera.scroll(0.0, -speed);
                            needs_redraw = true;
                        }
                        Keycode::S | Keycode::Down => {
                            camera.scroll(0.0, speed);
                            needs_redraw = true;
                        }
                        Keycode::A | Keycode::Left => {
                            camera.scroll(-speed, 0.0);
                            needs_redraw = true;
                        }
                        Keycode::D | Keycode::Right => {
                            camera.scroll(speed, 0.0);
                            needs_redraw = true;
                        }

                        // Zoom controls: +/- keys (both main keyboard and numpad)
                        Keycode::Plus | Keycode::Equals | Keycode::KpPlus => {
                            camera.zoom_in();
                            needs_redraw = true;
                        }
                        Keycode::Minus | Keycode::KpMinus => {
                            camera.zoom_out();
                            needs_redraw = true;
                        }

                        _ => {}
                    }
                }

                // Mouse wheel zoom: scroll up to zoom in, scroll down to zoom out.
                Event::MouseWheel { y, .. } => {
                    if y > 0 {
                        camera.zoom_in();
                    } else if y < 0 {
                        camera.zoom_out();
                    }
                    needs_redraw = true;
                }

                _ => {}
            }
        }

        if needs_redraw {
            // Update window title with camera state
            let title = format!(
                "Open Wages — Mission {} — pos({:.0},{:.0}) zoom {:.2}x",
                args.mission, camera.x, camera.y, camera.zoom,
            );
            if let Err(e) = canvas.window_mut().set_title(&title) {
                warn!(error = %e, "failed to set window title");
            }

            // Clear to dark background
            canvas.set_draw_color(BG_COLOR);
            canvas.clear();

            // Render the isometric tile map
            tile_renderer.render_map(&mut canvas, &map, &camera, &iso);

            canvas.present();
            needs_redraw = false;
        }
    }

    info!("map viewer closed");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Resolve the WOW/ base directory from the user-provided data dir.
///
/// If the user points at the WOW/ folder directly, use it.
/// If they point at a parent that contains WOW/, use that subfolder.
fn resolve_wow_dir(data_dir: &Path) -> anyhow::Result<PathBuf> {
    // Check if data_dir itself is the WOW folder
    let dir_name = data_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if dir_name.eq_ignore_ascii_case("wow") {
        return Ok(data_dir.to_path_buf());
    }

    // Check for a WOW/ subfolder (case-insensitive)
    if let Some(wow) = find_subdir_case_insensitive(data_dir, "WOW") {
        return Ok(wow);
    }

    anyhow::bail!(
        "could not find WOW/ directory in or at '{}'",
        data_dir.display()
    );
}

/// Find a subdirectory by name (case-insensitive).
fn find_subdir_case_insensitive(parent: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            if let Some(n) = entry.file_name().to_str() {
                if n.eq_ignore_ascii_case(name) {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

/// Find a file by name (case-insensitive) within a directory.
fn find_file_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if let Some(n) = entry.file_name().to_str() {
            if n.eq_ignore_ascii_case(name) {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Load the 256-color palette from the first .PCX file found in WOW/PIC/.
fn load_palette_from_pic_dir(wow_dir: &Path) -> anyhow::Result<ow_render::palette::Palette256> {
    let pic_dir = wow_dir.join("PIC");
    if !pic_dir.exists() {
        // Try case-insensitive
        let pic_dir = find_subdir_case_insensitive(wow_dir, "PIC").ok_or_else(|| {
            anyhow::anyhow!("WOW/PIC/ directory not found in {}", wow_dir.display())
        })?;
        return load_first_pcx_palette(&pic_dir);
    }
    load_first_pcx_palette(&pic_dir)
}

/// Find the first .PCX file in a directory and extract its palette.
fn load_first_pcx_palette(dir: &Path) -> anyhow::Result<ow_render::palette::Palette256> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if name.to_ascii_uppercase().ends_with(".PCX") {
                let path = entry.path();
                info!(path = %path.display(), "loading palette from PCX file");
                return load_pcx_palette(&path);
            }
        }
    }
    anyhow::bail!("no .PCX files found in {}", dir.display());
}

/// Load the MAP file for a given mission number.
///
/// Tries these paths in order:
/// 1. `WOW/MAPS/SCEN{n}/SCEN{n}.MAP`
/// 2. `WOW/MAPS/SCEN{n}/SCEN{n}A.MAP`
fn load_mission_map(
    wow_dir: &Path,
    mission: u32,
) -> anyhow::Result<(ow_data::map_loader::GameMap, PathBuf)> {
    let maps_dir = find_subdir_case_insensitive(wow_dir, "MAPS")
        .ok_or_else(|| anyhow::anyhow!("WOW/MAPS/ directory not found"))?;

    let scen_name = format!("SCEN{mission}");
    let scen_dir = find_subdir_case_insensitive(&maps_dir, &scen_name).ok_or_else(|| {
        anyhow::anyhow!("{scen_name}/ directory not found in {}", maps_dir.display())
    })?;

    debug!(scen_dir = %scen_dir.display(), "found scenario directory");

    // Try SCEN{n}.MAP first, then SCEN{n}A.MAP
    let candidates = [format!("SCEN{mission}.MAP"), format!("SCEN{mission}A.MAP")];

    for candidate in &candidates {
        if let Some(path) = find_file_case_insensitive(&scen_dir, candidate) {
            info!(path = %path.display(), "loading MAP file");
            let map = map_loader::parse_map(&path)?;
            return Ok((map, path));
        }
    }

    anyhow::bail!(
        "no MAP file found for mission {} in {}",
        mission,
        scen_dir.display()
    );
}
