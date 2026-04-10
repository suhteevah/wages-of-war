//! Line-of-sight calculations on the tile grid.
//!
//! Uses Bresenham's line algorithm to trace rays across tiles, checking each
//! intermediate tile for sight-blocking terrain (walls, etc.).
//!
//! ## How Bresenham's line algorithm works
//!
//! Bresenham's algorithm efficiently computes which cells a straight line passes
//! through on an integer grid. It works by tracking an "error" accumulator that
//! tells us whether the true line has drifted far enough from the current row
//! (or column) to warrant stepping in the secondary axis.
//!
//! Starting from the origin, we always step one cell along the *major* axis
//! (whichever of dx/dy is larger). After each step we add `|dy|` (or `|dx|`)
//! to the error term. When the error exceeds `|dx|` (or `|dy|`) we also step
//! one cell in the minor axis and subtract the threshold. This produces a
//! staircase of cells that closely follows the true line, with no floating-point
//! math and no gaps.

use std::collections::HashSet;

use tracing::{debug, trace};

use crate::merc::TilePos;
use crate::pathfinding::TileMap;

// ---------------------------------------------------------------------------
// Bresenham's line
// ---------------------------------------------------------------------------

/// Trace a line of tiles from `from` to `to` using Bresenham's algorithm.
///
/// Returns all tiles along the line *including* both endpoints.
fn bresenham_line(from: TilePos, to: TilePos) -> Vec<TilePos> {
    let mut points = Vec::new();

    let mut x = from.x;
    let mut y = from.y;
    let dx = (to.x - from.x).abs();
    let dy = (to.y - from.y).abs();

    // Step direction along each axis: +1 or -1.
    let sx = if from.x < to.x { 1 } else { -1 };
    let sy = if from.y < to.y { 1 } else { -1 };

    // `err` tracks the accumulated deviation from the true line.
    // Positive means we've drifted too far in the x-direction relative to y
    // (when dx > dy) or vice versa.
    let mut err = dx - dy;

    loop {
        points.push(TilePos { x, y });

        // Reached the destination — done.
        if x == to.x && y == to.y {
            break;
        }

        // Double the error to avoid fractional comparisons.
        // If e2 > -dy, we step in x. If e2 < dx, we step in y.
        // Both can be true simultaneously for a diagonal step.
        let e2 = 2 * err;

        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }

    points
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check whether there is an unobstructed line of sight between two tiles.
///
/// Traces a Bresenham line from `from` to `to`. The starting tile (`from`) is
/// always ignored — you can always see out of your own position, even if it
/// contains a wall-like structure behind you. Every other tile on the line is
/// checked: if any of them block sight, the function returns `false`.
pub fn has_line_of_sight(map: &TileMap, from: TilePos, to: TilePos) -> bool {
    let line = bresenham_line(from, to);

    trace!(
        ?from,
        ?to,
        tiles = line.len(),
        "Checking line of sight"
    );

    // Skip the first tile (the observer's own position).
    for pos in line.iter().skip(1) {
        // Don't check the destination tile itself — you can *see* a wall,
        // you just can't see *through* it. But if the destination IS the
        // wall, you should still be able to see it. Only intermediate tiles
        // block. We skip the last tile as well.
        if *pos == to {
            break;
        }
        if map.blocks_sight(*pos) {
            debug!(
                ?from,
                ?to,
                blocked_at = ?pos,
                "Line of sight blocked"
            );
            return false;
        }
    }

    true
}

/// Compute the set of all tiles visible from `origin` within `sight_range`.
///
/// Casts Bresenham rays to every tile on the perimeter of a square with
/// half-side `sight_range`, then marks each unobstructed tile along the ray
/// as visible. This produces a reasonable approximation of a circular field
/// of view (the corners are slightly beyond true Euclidean range, but this
/// matches the style of classic tactical games).
///
/// The `sight_range` parameter is in tile units. Weather or time-of-day
/// modifiers should be applied by the caller before invoking this function.
pub fn visible_tiles(map: &TileMap, origin: TilePos, sight_range: u32) -> Vec<TilePos> {
    debug!(
        ?origin,
        sight_range,
        "Computing visible tiles"
    );

    let range = sight_range as i32;
    let mut visible: HashSet<TilePos> = HashSet::new();

    // The origin itself is always visible.
    visible.insert(origin);

    // Cast rays to every cell on the perimeter of the bounding square.
    // This ensures coverage in all directions. Interior cells are implicitly
    // covered because rays pass through them on the way to the perimeter.
    let min_x = origin.x - range;
    let max_x = origin.x + range;
    let min_y = origin.y - range;
    let max_y = origin.y + range;

    // Collect perimeter cells (top/bottom rows, left/right columns).
    let mut perimeter: Vec<TilePos> = Vec::new();
    for x in min_x..=max_x {
        perimeter.push(TilePos { x, y: min_y });
        perimeter.push(TilePos { x, y: max_y });
    }
    for y in (min_y + 1)..max_y {
        perimeter.push(TilePos { x: min_x, y });
        perimeter.push(TilePos { x: max_x, y });
    }

    let range_sq = (sight_range * sight_range) as i64;

    for target in &perimeter {
        let line = bresenham_line(origin, *target);
        // Walk the ray, skipping the origin (already added).
        for pos in line.iter().skip(1) {
            // Euclidean range check (squared to avoid sqrt).
            let dx = (pos.x - origin.x) as i64;
            let dy = (pos.y - origin.y) as i64;
            if dx * dx + dy * dy > range_sq {
                break;
            }

            // If this tile blocks sight, mark it visible (you can see the
            // wall) but stop the ray here.
            if map.blocks_sight(*pos) {
                visible.insert(*pos);
                break;
            }

            visible.insert(*pos);
        }
    }

    let mut result: Vec<TilePos> = visible.into_iter().collect();
    result.sort_by_key(|p| (p.y, p.x));

    debug!(count = result.len(), "Visible tiles computed");
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfinding::{TileInfo, TerrainType};

    /// Build a simple open map.
    fn open_map(w: u32, h: u32) -> TileMap {
        TileMap::new_uniform(w, h, TileInfo::open())
    }

    /// Place a wall at (x, y).
    fn set_wall(map: &mut TileMap, x: i32, y: i32) {
        if let Some(tile) = map.get_mut(x, y) {
            tile.terrain = TerrainType::Wall;
            tile.walkable = false;
        }
    }

    // -- Bresenham sanity --

    #[test]
    fn bresenham_horizontal() {
        let line = bresenham_line(
            TilePos { x: 0, y: 0 },
            TilePos { x: 4, y: 0 },
        );
        assert_eq!(line.len(), 5);
        for (i, pos) in line.iter().enumerate() {
            assert_eq!(pos.x, i as i32);
            assert_eq!(pos.y, 0);
        }
    }

    #[test]
    fn bresenham_diagonal() {
        let line = bresenham_line(
            TilePos { x: 0, y: 0 },
            TilePos { x: 3, y: 3 },
        );
        assert_eq!(line.len(), 4);
        for (i, pos) in line.iter().enumerate() {
            assert_eq!(pos.x, i as i32);
            assert_eq!(pos.y, i as i32);
        }
    }

    // -- LOS checks --

    #[test]
    fn clear_line_of_sight() {
        let map = open_map(10, 10);
        let from = TilePos { x: 0, y: 0 };
        let to = TilePos { x: 9, y: 0 };
        assert!(has_line_of_sight(&map, from, to));
    }

    #[test]
    fn wall_blocks_line_of_sight() {
        let mut map = open_map(10, 10);
        set_wall(&mut map, 5, 0);
        let from = TilePos { x: 0, y: 0 };
        let to = TilePos { x: 9, y: 0 };
        assert!(!has_line_of_sight(&map, from, to));
    }

    #[test]
    fn self_tile_does_not_block() {
        // Even if the origin tile is a wall, we can still "see out" from it.
        let mut map = open_map(10, 10);
        set_wall(&mut map, 0, 0);
        let from = TilePos { x: 0, y: 0 };
        let to = TilePos { x: 3, y: 0 };
        assert!(has_line_of_sight(&map, from, to));
    }

    #[test]
    fn can_see_the_wall_itself() {
        // You can see a wall tile, you just can't see through it.
        let mut map = open_map(10, 10);
        set_wall(&mut map, 5, 0);
        let from = TilePos { x: 0, y: 0 };
        let to_wall = TilePos { x: 5, y: 0 };
        let to_behind = TilePos { x: 6, y: 0 };
        assert!(has_line_of_sight(&map, from, to_wall));
        assert!(!has_line_of_sight(&map, from, to_behind));
    }

    #[test]
    fn los_same_tile() {
        let map = open_map(5, 5);
        let pos = TilePos { x: 2, y: 2 };
        assert!(has_line_of_sight(&map, pos, pos));
    }

    // -- Visible tiles --

    #[test]
    fn visible_tiles_open_map() {
        let map = open_map(20, 20);
        let origin = TilePos { x: 10, y: 10 };
        let vis = visible_tiles(&map, origin, 3);
        // With range 3, we should see a roughly circular area.
        // Origin is always visible.
        assert!(vis.contains(&origin));
        // Cardinal neighbours at distance 1, 2, 3 should be visible.
        assert!(vis.contains(&TilePos { x: 13, y: 10 }));
        assert!(vis.contains(&TilePos { x: 7, y: 10 }));
        assert!(vis.contains(&TilePos { x: 10, y: 13 }));
        assert!(vis.contains(&TilePos { x: 10, y: 7 }));
    }

    #[test]
    fn visible_tiles_wall_blocks_behind() {
        let mut map = open_map(20, 20);
        // Wall at (12, 10) should block vision to (13, 10) and beyond.
        set_wall(&mut map, 12, 10);
        let origin = TilePos { x: 10, y: 10 };
        let vis = visible_tiles(&map, origin, 5);
        // The wall itself should be visible.
        assert!(vis.contains(&TilePos { x: 12, y: 10 }));
        // Tiles behind the wall should NOT be visible.
        assert!(!vis.contains(&TilePos { x: 13, y: 10 }));
        assert!(!vis.contains(&TilePos { x: 14, y: 10 }));
    }

    #[test]
    fn visible_tiles_edge_of_range() {
        let map = open_map(20, 20);
        let origin = TilePos { x: 10, y: 10 };
        let vis = visible_tiles(&map, origin, 2);
        // (12, 10) is exactly at range 2 — should be visible.
        assert!(vis.contains(&TilePos { x: 12, y: 10 }));
        // (13, 10) is at range 3 — should NOT be visible with range 2.
        assert!(!vis.contains(&TilePos { x: 13, y: 10 }));
    }

    #[test]
    fn visible_tiles_zero_range() {
        let map = open_map(10, 10);
        let origin = TilePos { x: 5, y: 5 };
        let vis = visible_tiles(&map, origin, 0);
        // Only the origin should be visible.
        assert_eq!(vis.len(), 1);
        assert!(vis.contains(&origin));
    }
}
