//! # Unit Renderer — Draws soldiers, enemies, and NPCs on the isometric map
//!
//! Units are positioned at tile centers and drawn **after** terrain tiles using
//! the same Y-then-X painter's algorithm. This ensures correct overlapping:
//! units on tiles with higher Y (or equal Y but higher X) are drawn later and
//! appear in front of units on lower-coordinate tiles.
//!
//! Each unit carries a [`UnitVisual`] descriptor that tells the renderer what
//! to draw: sprite index, facing direction (mirror), faction, health, and
//! status flags. The renderer composes several layers per unit:
//!
//! 1. **Selection highlight** — a translucent diamond drawn *under* the
//!    selected unit so it stands out from the rest of the squad.
//! 2. **Unit sprite** — the character graphic, optionally mirrored
//!    horizontally when the unit faces east/west to reuse the same sprite
//!    sheet for both directions.
//! 3. **Suppression indicator** — a slight darkening overlay or icon drawn
//!    over suppressed units to communicate reduced combat effectiveness.
//! 4. **Health bar** — a small horizontal bar above the unit, using a
//!    green→yellow→red gradient that maps linearly to the unit's remaining
//!    HP percentage.
//!
//! Movement and attack overlays are rendered as translucent tile highlights
//! so the player can see reachable tiles and valid targets at a glance.

use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::{debug, trace};

use crate::camera::Camera;
use crate::iso_math::{IsoConfig, TilePos};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Width of the health bar in screen pixels.
const HEALTH_BAR_WIDTH: u32 = 24;
/// Height of the health bar in screen pixels.
const HEALTH_BAR_HEIGHT: u32 = 4;
/// Vertical offset of the health bar above the unit center, in pixels.
const HEALTH_BAR_Y_OFFSET: i32 = 20;
/// Alpha value for tile overlay highlights (movement, attack).
const OVERLAY_ALPHA: u8 = 80;
/// Alpha value for the selection diamond highlight.
const SELECTION_ALPHA: u8 = 100;
/// Alpha value for the suppression darkening overlay.
const SUPPRESSION_ALPHA: u8 = 90;
/// Placeholder unit sprite size — used until real sprite textures are wired up.
const UNIT_PLACEHOLDER_SIZE: u32 = 20;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Which side a unit fights for. Determines default health-bar colour and
/// any faction-specific rendering tweaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Faction {
    Player,
    Enemy,
    Npc,
}

/// Everything the renderer needs to draw one unit for a single frame.
///
/// This is a *view* struct — it does not own game state. The game-logic layer
/// builds a `Vec<UnitVisual>` each frame from [`ActiveMerc`] and enemy data
/// and hands it to [`UnitRenderer::render_units`].
#[derive(Debug, Clone)]
pub struct UnitVisual {
    /// Tile the unit currently occupies.
    pub tile_pos: TilePos,
    /// Index into the sprite sheet for this unit's current animation frame.
    pub sprite_index: u32,
    /// If `true`, flip the sprite horizontally (east↔west direction sharing).
    pub mirror: bool,
    /// Which side this unit belongs to.
    pub faction: Faction,
    /// Remaining HP as a fraction in `[0.0, 1.0]`.
    pub health_pct: f32,
    /// Whether the player has selected this unit.
    pub is_selected: bool,
    /// Whether this unit is under suppression from enemy fire.
    pub is_suppressed: bool,
}

/// Manages unit-sprite textures and draws units onto the isometric map.
///
/// Currently uses placeholder rectangles for unit bodies. Once the sprite
/// pipeline is connected, [`SpriteRenderer`] will supply the actual textures
/// and this struct will coordinate the draw calls.
pub struct UnitRenderer {
    // Future: texture atlas handle, animation state cache, etc.
    _private: (),
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

/// Sort key for painter's algorithm: units with larger `(y, x)` are drawn
/// later so they appear in front.
fn paint_order_key(u: &UnitVisual) -> (i32, i32) {
    (u.tile_pos.y, u.tile_pos.x)
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl UnitRenderer {
    /// Create a new `UnitRenderer`.
    pub fn new() -> Self {
        debug!("UnitRenderer created");
        Self { _private: () }
    }

    /// Draw all units at their isometric positions.
    ///
    /// Units are sorted into painter's order (Y-then-X ascending) so that
    /// units on "closer" tiles (higher screen Y) are drawn on top of units
    /// on "farther" tiles. This matches the terrain draw order established
    /// by the isometric tile renderer.
    ///
    /// For each unit the draw layers are:
    /// 1. Selection highlight (if selected)
    /// 2. Unit body sprite (or placeholder)
    /// 3. Suppression overlay (if suppressed)
    /// 4. Health bar (always visible)
    pub fn render_units(
        &self,
        canvas: &mut Canvas<Window>,
        camera: &Camera,
        iso: &IsoConfig,
        units: &[UnitVisual],
    ) {
        // Sort a working copy so we don't require the caller to pre-sort.
        let mut sorted: Vec<&UnitVisual> = units.iter().collect();
        sorted.sort_by_key(|u| paint_order_key(u));

        for unit in &sorted {
            // Convert tile position to world-space pixel position (tile center).
            let world = iso.tile_to_screen(unit.tile_pos);
            let screen = camera.world_to_screen(world);
            let sx = screen.x as i32;
            let sy = screen.y as i32;

            trace!(
                tx = unit.tile_pos.x,
                ty = unit.tile_pos.y,
                sx,
                sy,
                faction = ?unit.faction,
                "drawing unit"
            );

            // --- Layer 1: selection highlight ---
            if unit.is_selected {
                self.draw_selection_diamond(canvas, sx, sy, iso, camera.zoom);
            }

            // --- Layer 2: unit body ---
            self.draw_unit_body(canvas, sx, sy, unit, camera.zoom);

            // --- Layer 3: suppression overlay ---
            if unit.is_suppressed {
                self.draw_suppression_overlay(canvas, sx, sy, camera.zoom);
            }

            // --- Layer 4: health bar ---
            self.draw_health_bar(canvas, sx, sy, unit.health_pct, camera.zoom);
        }
    }

    /// Render a translucent blue/green overlay on every tile the selected unit
    /// can reach this turn.
    ///
    /// `reachable` is a slice of `(TilePos, remaining_ap)` pairs. Tiles with
    /// more remaining AP are drawn slightly brighter to give the player a sense
    /// of movement cost.
    pub fn render_movement_overlay(
        canvas: &mut Canvas<Window>,
        camera: &Camera,
        iso: &IsoConfig,
        reachable: &[(TilePos, u32)],
    ) {
        if reachable.is_empty() {
            return;
        }

        let max_ap = reachable.iter().map(|(_, ap)| *ap).max().unwrap_or(1).max(1);

        for (tile, remaining_ap) in reachable {
            let world = iso.tile_to_screen(*tile);
            let screen = camera.world_to_screen(world);

            // Brighter for tiles with more remaining AP (cheaper to reach).
            let intensity = (*remaining_ap as f32 / max_ap as f32 * 255.0) as u8;
            let color = Color::RGBA(0, intensity / 2 + 100, intensity, OVERLAY_ALPHA);

            draw_tile_diamond(canvas, screen.x as i32, screen.y as i32, iso, camera.zoom, color);

            trace!(
                tx = tile.x,
                ty = tile.y,
                remaining_ap,
                "movement overlay tile"
            );
        }
    }

    /// Render a translucent red overlay on every tile containing a valid
    /// attack target.
    pub fn render_attack_overlay(
        canvas: &mut Canvas<Window>,
        camera: &Camera,
        iso: &IsoConfig,
        targets: &[TilePos],
    ) {
        let color = Color::RGBA(220, 40, 40, OVERLAY_ALPHA);
        for tile in targets {
            let world = iso.tile_to_screen(*tile);
            let screen = camera.world_to_screen(world);

            draw_tile_diamond(canvas, screen.x as i32, screen.y as i32, iso, camera.zoom, color);

            trace!(tx = tile.x, ty = tile.y, "attack overlay tile");
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Draw a translucent diamond under the selected unit.
    fn draw_selection_diamond(
        &self,
        canvas: &mut Canvas<Window>,
        sx: i32,
        sy: i32,
        iso: &IsoConfig,
        zoom: f32,
    ) {
        let color = Color::RGBA(50, 200, 255, SELECTION_ALPHA);
        draw_tile_diamond(canvas, sx, sy, iso, zoom, color);
    }

    /// Draw the unit body. Currently a coloured placeholder rectangle; will be
    /// replaced with `SpriteRenderer::draw()` once sprite textures are loaded.
    fn draw_unit_body(
        &self,
        canvas: &mut Canvas<Window>,
        sx: i32,
        sy: i32,
        unit: &UnitVisual,
        zoom: f32,
    ) {
        let size = (UNIT_PLACEHOLDER_SIZE as f32 * zoom) as u32;
        let half = size as i32 / 2;

        let base_color = match unit.faction {
            Faction::Player => Color::RGB(60, 140, 220),
            Faction::Enemy => Color::RGB(200, 50, 50),
            Faction::Npc => Color::RGB(180, 180, 60),
        };

        // Mirror flag: in placeholder mode we just note it in the trace.
        // With real sprites the source rect would be flipped horizontally.
        if unit.mirror {
            trace!(sprite_index = unit.sprite_index, "sprite mirrored for facing");
        }

        canvas.set_draw_color(base_color);
        let _ = canvas.fill_rect(Rect::new(sx - half, sy - half, size, size));
    }

    /// Draw a dark overlay on the unit to indicate suppression.
    fn draw_suppression_overlay(&self, canvas: &mut Canvas<Window>, sx: i32, sy: i32, zoom: f32) {
        let size = (UNIT_PLACEHOLDER_SIZE as f32 * zoom) as u32;
        let half = size as i32 / 2;

        canvas.set_draw_color(Color::RGBA(0, 0, 0, SUPPRESSION_ALPHA));
        let _ = canvas.fill_rect(Rect::new(sx - half, sy - half, size, size));
    }

    /// Draw a health bar above the unit.
    ///
    /// The bar background is dark grey. The filled portion uses a
    /// green→yellow→red gradient:
    /// - `health_pct >= 0.6` → green
    /// - `0.3 <= health_pct < 0.6` → yellow
    /// - `health_pct < 0.3` → red
    fn draw_health_bar(
        &self,
        canvas: &mut Canvas<Window>,
        sx: i32,
        sy: i32,
        health_pct: f32,
        zoom: f32,
    ) {
        let bar_w = (HEALTH_BAR_WIDTH as f32 * zoom) as u32;
        let bar_h = (HEALTH_BAR_HEIGHT as f32 * zoom).max(2.0) as u32;
        let y_off = (HEALTH_BAR_Y_OFFSET as f32 * zoom) as i32;

        let bx = sx - (bar_w as i32 / 2);
        let by = sy - y_off;

        // Background
        canvas.set_draw_color(Color::RGB(40, 40, 40));
        let _ = canvas.fill_rect(Rect::new(bx, by, bar_w, bar_h));

        // Filled portion
        let pct = health_pct.clamp(0.0, 1.0);
        let fill_w = (bar_w as f32 * pct) as u32;

        let fill_color = health_color(pct);
        canvas.set_draw_color(fill_color);
        if fill_w > 0 {
            let _ = canvas.fill_rect(Rect::new(bx, by, fill_w, bar_h));
        }

        // Thin outline
        canvas.set_draw_color(Color::RGB(0, 0, 0));
        let _ = canvas.draw_rect(Rect::new(bx, by, bar_w, bar_h));
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Map a health percentage to a colour on the green→yellow→red gradient.
///
/// - `[0.6, 1.0]` → green (0, 200, 0)
/// - `[0.3, 0.6)` → yellow (220, 200, 0)
/// - `[0.0, 0.3)` → red (220, 40, 0)
fn health_color(pct: f32) -> Color {
    if pct >= 0.6 {
        Color::RGB(0, 200, 0)
    } else if pct >= 0.3 {
        Color::RGB(220, 200, 0)
    } else {
        Color::RGB(220, 40, 0)
    }
}

/// Draw a filled isometric diamond at the given screen position.
///
/// The diamond is axis-aligned to the tile grid: its horizontal half-width
/// is `tile_width / 2 * zoom` and its vertical half-height is
/// `tile_height / 2 * zoom`. We approximate the fill with horizontal
/// line segments (scanlines) because SDL2's basic renderer lacks polygon fill.
fn draw_tile_diamond(
    canvas: &mut Canvas<Window>,
    sx: i32,
    sy: i32,
    iso: &IsoConfig,
    zoom: f32,
    color: Color,
) {
    let hw = (iso.tile_width / 2.0 * zoom) as i32; // half-width in screen px
    let hh = (iso.tile_height / 2.0 * zoom) as i32; // half-height in screen px

    if hh == 0 || hw == 0 {
        return;
    }

    canvas.set_draw_color(color);

    // Rasterise the diamond with horizontal scanlines.
    // Top half: y goes from -hh to 0, width expands from 0 to hw.
    // Bottom half: y goes from 0 to +hh, width shrinks from hw to 0.
    for dy in -hh..=hh {
        let t = 1.0 - (dy.abs() as f32 / hh as f32); // 0 at tips, 1 at equator
        let half_span = (hw as f32 * t) as i32;
        let _ = canvas.draw_line(
            sdl2::rect::Point::new(sx - half_span, sy + dy),
            sdl2::rect::Point::new(sx + half_span, sy + dy),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Coordinate / sort-order tests (no SDL2 required) --

    #[test]
    fn paint_order_sorts_by_y_then_x() {
        let units = vec![
            unit_at(3, 1),
            unit_at(1, 2),
            unit_at(2, 2),
            unit_at(0, 0),
        ];

        let mut sorted: Vec<&UnitVisual> = units.iter().collect();
        sorted.sort_by_key(|u| paint_order_key(u));

        let positions: Vec<(i32, i32)> = sorted
            .iter()
            .map(|u| (u.tile_pos.x, u.tile_pos.y))
            .collect();

        // Expected order: (0,0), (3,1), (1,2), (2,2) — sorted by (y, x).
        assert_eq!(positions, vec![(0, 0), (3, 1), (1, 2), (2, 2)]);
    }

    #[test]
    fn paint_order_equal_y_sorted_by_x() {
        let units = vec![unit_at(5, 3), unit_at(2, 3), unit_at(4, 3)];

        let mut sorted: Vec<&UnitVisual> = units.iter().collect();
        sorted.sort_by_key(|u| paint_order_key(u));

        let xs: Vec<i32> = sorted.iter().map(|u| u.tile_pos.x).collect();
        assert_eq!(xs, vec![2, 4, 5]);
    }

    #[test]
    fn paint_order_single_unit() {
        let units = vec![unit_at(7, 4)];
        let mut sorted: Vec<&UnitVisual> = units.iter().collect();
        sorted.sort_by_key(|u| paint_order_key(u));
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].tile_pos.x, 7);
    }

    #[test]
    fn paint_order_empty() {
        let units: Vec<UnitVisual> = vec![];
        let mut sorted: Vec<&UnitVisual> = units.iter().collect();
        sorted.sort_by_key(|u| paint_order_key(u));
        assert!(sorted.is_empty());
    }

    // -- Health colour tests --

    #[test]
    fn health_color_green_above_60_pct() {
        assert_eq!(health_color(1.0), Color::RGB(0, 200, 0));
        assert_eq!(health_color(0.6), Color::RGB(0, 200, 0));
        assert_eq!(health_color(0.8), Color::RGB(0, 200, 0));
    }

    #[test]
    fn health_color_yellow_between_30_and_60_pct() {
        assert_eq!(health_color(0.59), Color::RGB(220, 200, 0));
        assert_eq!(health_color(0.3), Color::RGB(220, 200, 0));
        assert_eq!(health_color(0.45), Color::RGB(220, 200, 0));
    }

    #[test]
    fn health_color_red_below_30_pct() {
        assert_eq!(health_color(0.29), Color::RGB(220, 40, 0));
        assert_eq!(health_color(0.0), Color::RGB(220, 40, 0));
        assert_eq!(health_color(0.15), Color::RGB(220, 40, 0));
    }

    // -- Tile-to-screen position tests (verifies units land on tile centers) --

    #[test]
    fn unit_screen_position_matches_tile_center() {
        let iso = IsoConfig {
            tile_width: 64.0,
            tile_height: 32.0,
            origin_x: 400.0,
            origin_y: 100.0,
        };
        let camera = Camera::new(1280, 720);

        let tile = TilePos { x: 3, y: 5 };
        let world = iso.tile_to_screen(tile);
        let screen = camera.world_to_screen(world);

        // Expected world position:
        //   sx = 400 + (3 - 5) * 32 = 400 - 64 = 336
        //   sy = 100 + (3 + 5) * 16 = 100 + 128 = 228
        // Camera at (0,0) zoom 1.0 → screen == world.
        assert!((screen.x - 336.0).abs() < 0.01);
        assert!((screen.y - 228.0).abs() < 0.01);
    }

    #[test]
    fn unit_screen_position_with_camera_offset() {
        let iso = IsoConfig {
            tile_width: 64.0,
            tile_height: 32.0,
            origin_x: 0.0,
            origin_y: 0.0,
        };
        let mut camera = Camera::new(1280, 720);
        camera.scroll(100.0, 50.0);

        let tile = TilePos { x: 2, y: 2 };
        let world = iso.tile_to_screen(tile);
        let screen = camera.world_to_screen(world);

        // World: sx = (2-2)*32 = 0, sy = (2+2)*16 = 64
        // Camera offset (100, 50), zoom 1.0: screen = (0-100, 64-50) = (-100, 14)
        assert!((screen.x - (-100.0)).abs() < 0.01);
        assert!((screen.y - 14.0).abs() < 0.01);
    }

    #[test]
    fn unit_screen_position_with_zoom() {
        let iso = IsoConfig {
            tile_width: 64.0,
            tile_height: 32.0,
            origin_x: 0.0,
            origin_y: 0.0,
        };
        let camera = Camera {
            x: 0.0,
            y: 0.0,
            zoom: 2.0,
            viewport_width: 1280,
            viewport_height: 720,
        };

        let tile = TilePos { x: 1, y: 0 };
        let world = iso.tile_to_screen(tile);
        let screen = camera.world_to_screen(world);

        // World: sx = (1-0)*32 = 32, sy = (1+0)*16 = 16
        // Zoom 2.0: screen = (32*2, 16*2) = (64, 32)
        assert!((screen.x - 64.0).abs() < 0.01);
        assert!((screen.y - 32.0).abs() < 0.01);
    }

    // -- Helper --

    fn unit_at(x: i32, y: i32) -> UnitVisual {
        UnitVisual {
            tile_pos: TilePos { x, y },
            sprite_index: 0,
            mirror: false,
            faction: Faction::Player,
            health_pct: 1.0,
            is_selected: false,
            is_suppressed: false,
        }
    }
}
