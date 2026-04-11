//! Isometric coordinate math: screen ↔ tile conversions.
//!
//! Wages of War uses a **staggered isometric grid**, NOT a standard diamond
//! projection. The difference is critical:
//!
//! - **Standard diamond**: `screen_x = (col - row) * half_w`, `screen_y = (col + row) * half_h`
//! - **Staggered grid**: `screen_x = col * tile_w`, `screen_y = row * tile_h`, odd rows offset +half_w
//!
//! The staggered grid was confirmed by RE of Wow.exe's CellToScreen function
//! and neighbor offset constants (69, 70, 71 = half-row offsets).
//!
//! ## Tile dimensions
//!
//! - Tile width: 128px (0x80), confirmed by `shr 7` in screen-to-cell
//! - Tile height: 64px (0x40), confirmed by `sar 6` in screen-to-cell
//! - Diamond half-width: 64px, used for stagger offset and centering

use tracing::trace;

/// Position on the tile grid (column, row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TilePos {
    /// Column index (0-139 for Wages of War maps).
    pub x: i32,
    /// Row index (0-71 for Wages of War maps).
    pub y: i32,
}

/// Position in screen/world pixel coordinates.
#[derive(Debug, Clone, Copy)]
pub struct ScreenPos {
    pub x: f32,
    pub y: f32,
}

/// Configuration for the isometric projection.
///
/// For Wages of War, the correct values are:
/// - `tile_width = 128.0` (full diamond width)
/// - `tile_height = 64.0` (full diamond height, NOT half)
/// - `origin_x/y = 0.0` (camera handles scrolling)
pub struct IsoConfig {
    /// Full tile width in pixels (128 for WoW).
    pub tile_width: f32,
    /// Full tile height in pixels (64 for WoW).
    pub tile_height: f32,
    /// World origin X offset (usually 0, camera handles scrolling).
    pub origin_x: f32,
    /// World origin Y offset (usually 0).
    pub origin_y: f32,
}

impl IsoConfig {
    /// Convert tile grid position to world-space pixel coordinates.
    ///
    /// Uses a staggered grid with **half-height row spacing** so that
    /// diamond-shaped isometric tiles interlock vertically. Each row is
    /// placed at `row * (tile_height / 2)` pixels, and odd rows are
    /// shifted right by `tile_width / 2`.
    ///
    /// ```text
    /// screen_x = col * tile_width
    /// screen_y = row * (tile_height / 2)     // half-height for interlocking
    /// if row is odd: screen_x += tile_width / 2
    /// ```
    ///
    /// The exe's ScreenToCell uses `y / 64` because each 64px vertical band
    /// contains two interlocked sub-rows at 32px spacing. The cell index
    /// formula `row * 140 + col` maps to visual rows spaced 32px apart.
    pub fn tile_to_screen(&self, tile: TilePos) -> ScreenPos {
        let half_h = self.tile_height / 2.0;
        let mut sx = self.origin_x + tile.x as f32 * self.tile_width;
        // Half-height row spacing: diamonds overlap vertically by 50%,
        // creating the seamless interlocking pattern.
        let sy = self.origin_y + tile.y as f32 * half_h;

        // Staggered grid: odd rows shifted right by half a tile width.
        if tile.y % 2 != 0 {
            sx += self.tile_width / 2.0;
        }

        trace!(col = tile.x, row = tile.y, sx, sy, "tile->screen (staggered)");
        ScreenPos { x: sx, y: sy }
    }

    /// Convert screen/world pixel coordinates to tile grid position.
    ///
    /// Uses half-height row spacing to match the forward projection.
    /// The exe's ScreenToCell at 0x45FE11 uses `y / 64` because each
    /// 64px band contains two interlocked rows at 32px spacing.
    pub fn screen_to_tile(&self, screen: ScreenPos) -> TilePos {
        let half_w = self.tile_width / 2.0;
        let half_h = self.tile_height / 2.0;

        // Add half-tile offsets for centering (matches the +64, +32 in the exe).
        let x = screen.x - self.origin_x + half_w;
        let y = screen.y - self.origin_y + half_h / 2.0;

        // Coarse row from half-height spacing.
        let row = (y / half_h).floor() as i32;
        let col_raw = x - if row % 2 != 0 { half_w } else { 0.0 };
        let col = (col_raw / self.tile_width).floor() as i32;

        trace!(sx = screen.x, sy = screen.y, col, row, "screen->tile (staggered)");
        TilePos { x: col, y: row }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard WoW tile dimensions.
    fn wow_iso() -> IsoConfig {
        IsoConfig {
            tile_width: 128.0,
            tile_height: 64.0,
            origin_x: 0.0,
            origin_y: 0.0,
        }
    }

    #[test]
    fn even_row_no_stagger() {
        let cfg = wow_iso();
        let pos = cfg.tile_to_screen(TilePos { x: 5, y: 0 });
        assert_eq!(pos.x, 640.0); // 5 * 128
        assert_eq!(pos.y, 0.0);   // 0 * 32
    }

    #[test]
    fn odd_row_staggers_right() {
        let cfg = wow_iso();
        let pos = cfg.tile_to_screen(TilePos { x: 5, y: 1 });
        assert_eq!(pos.x, 704.0); // 5 * 128 + 64 (stagger)
        assert_eq!(pos.y, 32.0);  // 1 * 32 (half-height row spacing)
    }

    #[test]
    fn row_2_same_x_as_row_0() {
        let cfg = wow_iso();
        let pos = cfg.tile_to_screen(TilePos { x: 5, y: 2 });
        assert_eq!(pos.x, 640.0); // even row, no stagger
        assert_eq!(pos.y, 64.0);  // 2 * 32
    }

    #[test]
    fn origin_cell() {
        let cfg = wow_iso();
        let pos = cfg.tile_to_screen(TilePos { x: 0, y: 0 });
        assert_eq!(pos.x, 0.0);
        assert_eq!(pos.y, 0.0);
    }

    #[test]
    fn screen_to_tile_even_row() {
        let cfg = wow_iso();
        let tile = cfg.screen_to_tile(ScreenPos { x: 640.0, y: 0.0 });
        assert_eq!(tile.x, 5);
        assert_eq!(tile.y, 0);
    }

    #[test]
    fn screen_to_tile_odd_row() {
        let cfg = wow_iso();
        // Tile (5, 1) is at screen_x=704, screen_y=32 (half-height spacing).
        let tile = cfg.screen_to_tile(ScreenPos { x: 704.0, y: 32.0 });
        assert_eq!(tile.x, 5);
        assert_eq!(tile.y, 1);
    }
}
