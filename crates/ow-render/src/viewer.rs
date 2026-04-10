//! # Sprite Viewer — Interactive developer tool
//!
//! Opens an SDL2 window and displays sprites from a sprite container file,
//! one at a time. Intended as a quick visual verification that the sprite
//! parser and palette extraction are working correctly.
//!
//! ## Controls
//!
//! - **Left / Right arrows** — cycle through sprites in the container.
//! - **ESC** — quit.
//!
//! ## Usage
//!
//! ```text
//! sprite-viewer <sprite-file> <pcx-palette-file>
//! ```
//!
//! The sprite file can be any of the game's binary sprite containers
//! (.OBJ, .SPR, .TIL, or ANIM/*.DAT). The PCX file provides the 256-color
//! palette (extracted from its trailing 769-byte palette block).

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use tracing::{debug, info, warn};

use crate::palette::{load_pcx_palette, Palette256};
use crate::sprite_renderer::SpriteRenderer;
use ow_data::sprite::SpriteFrame;

/// Dark grey background so transparency (index 0) is clearly visible.
const BG_COLOR: Color = Color::RGB(40, 40, 40);

/// Window dimensions.
const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;

/// Run the interactive sprite viewer.
///
/// `sprite_path` — path to a sprite container file.
/// `palette_path` — path to a PCX file whose embedded palette will be used.
pub fn run_viewer(
    sprite_path: &std::path::Path,
    palette_path: &std::path::Path,
) -> anyhow::Result<()> {
    // -----------------------------------------------------------------------
    // Load data
    // -----------------------------------------------------------------------
    info!(sprite = %sprite_path.display(), "loading sprite file");
    let sheet = ow_data::sprite::parse_sprite_file(sprite_path)?;
    info!(
        sprites = sheet.file_header.sprite_count,
        "sprite sheet loaded"
    );

    let palette = load_pcx_palette(palette_path)?;
    info!("palette loaded from PCX");

    if sheet.frames.is_empty() {
        anyhow::bail!("sprite file contains no frames");
    }

    // -----------------------------------------------------------------------
    // SDL2 init
    // -----------------------------------------------------------------------
    let sdl_context = sdl2::init().map_err(|e| anyhow::anyhow!("SDL2 init failed: {e}"))?;
    let video = sdl_context
        .video()
        .map_err(|e| anyhow::anyhow!("SDL2 video init failed: {e}"))?;

    let window = video
        .window("Open Wages — Sprite Viewer", WINDOW_WIDTH, WINDOW_HEIGHT)
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
    let mut renderer = SpriteRenderer::new(&texture_creator);

    let mut event_pump = sdl_context
        .event_pump()
        .map_err(|e| anyhow::anyhow!("event pump failed: {e}"))?;

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------
    let total = sheet.frames.len();
    let mut current_index: usize = 0;
    let mut needs_redraw = true;

    // Pre-create texture for the first sprite.
    create_sprite_texture(&mut renderer, current_index as u32, &sheet.frames[current_index], &palette);
    update_window_title(&mut canvas, current_index, &sheet.frames[current_index], total);

    // -----------------------------------------------------------------------
    // Main loop
    // -----------------------------------------------------------------------
    'main_loop: loop {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'main_loop,

                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    ..
                } => {
                    current_index = (current_index + 1) % total;
                    needs_redraw = true;
                    debug!(index = current_index, "next sprite");
                }

                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    ..
                } => {
                    current_index = if current_index == 0 {
                        total - 1
                    } else {
                        current_index - 1
                    };
                    needs_redraw = true;
                    debug!(index = current_index, "prev sprite");
                }

                _ => {}
            }
        }

        if needs_redraw {
            // Ensure texture exists for the current sprite.
            if !renderer.has(current_index as u32) {
                create_sprite_texture(
                    &mut renderer,
                    current_index as u32,
                    &sheet.frames[current_index],
                    &palette,
                );
            }

            update_window_title(&mut canvas, current_index, &sheet.frames[current_index], total);

            // Clear to dark background.
            canvas.set_draw_color(BG_COLOR);
            canvas.clear();

            // Draw the sprite centered in the window.
            let frame = &sheet.frames[current_index];
            let w = frame.header.width as i32;
            let h = frame.header.height as i32;
            let x = (WINDOW_WIDTH as i32 - w) / 2;
            let y = (WINDOW_HEIGHT as i32 - h) / 2;

            if let Err(e) = renderer.draw(&mut canvas, current_index as u32, x, y) {
                warn!(error = %e, index = current_index, "failed to draw sprite");
            }

            canvas.present();
            needs_redraw = false;
        }
    }

    info!("sprite viewer closed");
    Ok(())
}

/// Create a texture for a sprite frame, logging any errors.
fn create_sprite_texture(
    renderer: &mut SpriteRenderer<'_>,
    key: u32,
    frame: &SpriteFrame,
    palette: &Palette256,
) {
    if let Err(e) = renderer.create_texture(key, frame, palette) {
        warn!(key, error = %e, "failed to create sprite texture");
    }
}

/// Update the window title to show current sprite info.
fn update_window_title(
    canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
    index: usize,
    frame: &SpriteFrame,
    total: usize,
) {
    let title = format!(
        "Sprite Viewer — [{}/{}] {}x{} origin({},{}) flags(0x{:04X},0x{:04X})",
        index + 1,
        total,
        frame.header.width,
        frame.header.height,
        frame.header.origin_x,
        frame.header.origin_y,
        frame.header.flags_a,
        frame.header.flags_b,
    );
    if let Err(e) = canvas.window_mut().set_title(&title) {
        warn!(error = %e, "failed to set window title");
    }
}
