//! A* pathfinding on the isometric tile grid.
//!
//! The isometric map is stored as a flat 2D grid of [`TileInfo`] cells. Even though
//! the *rendered* tiles are diamond-shaped (64×32 pixels in a 2:1 projection), the
//! underlying pathfinding graph is a regular square grid with 8-directional
//! connectivity. The isometric projection is purely a rendering concern handled by
//! `ow-render` — here we work entirely in tile coordinates.
//!
//! Movement costs are integer "AP tenths" internally to avoid floating-point
//! accumulation errors. The public API converts back to whole AP on output.

use std::collections::HashMap;

use pathfinding::prelude::astar;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::merc::TilePos;

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// Terrain classification for a single tile.
///
/// Each variant carries an implicit movement-cost multiplier (see
/// [`TerrainType::cost_multiplier_tenths`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TerrainType {
    /// Flat open ground — baseline cost.
    Open,
    /// Paved road — faster than open ground.
    Road,
    /// Dense forest — slow, but provides cover.
    Forest,
    /// Loose sand / desert — moderately slow.
    Sand,
    /// Deep water — impassable on foot.
    Water,
    /// Solid wall — impassable, blocks sight.
    Wall,
    /// Door — passable (may be locked in future), does not block movement.
    Door,
}

impl TerrainType {
    /// Movement cost multiplier in *tenths* (10 = 1.0×, 14 = 1.4×, etc.).
    ///
    /// Using tenths avoids floating-point while still giving us sub-integer
    /// resolution for diagonal scaling.
    pub fn cost_multiplier_tenths(self) -> u32 {
        match self {
            TerrainType::Open => 10,
            TerrainType::Road => 7,   // roads are faster (0.7×)
            TerrainType::Forest => 18, // thick undergrowth (1.8×)
            TerrainType::Sand => 14,   // loose footing (1.4×)
            TerrainType::Water => 0,   // impassable — handled by walkable flag
            TerrainType::Wall => 0,    // impassable
            TerrainType::Door => 10,   // same as open
        }
    }

    /// Whether this terrain blocks line of sight.
    pub fn blocks_sight(self) -> bool {
        matches!(self, TerrainType::Wall)
    }
}

// ---------------------------------------------------------------------------
// TileInfo / TileMap
// ---------------------------------------------------------------------------

/// Per-tile data for the tactical map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileInfo {
    /// Terrain classification.
    pub terrain: TerrainType,
    /// Height above the base plane (affects LOS and elevation bonuses).
    pub elevation: i32,
    /// Whether a unit can stand on this tile at all.
    pub walkable: bool,
    /// Whether another unit currently occupies this tile.
    pub occupied: bool,
}

impl TileInfo {
    /// Convenience constructor for a simple walkable tile.
    pub fn open() -> Self {
        Self {
            terrain: TerrainType::Open,
            elevation: 0,
            walkable: true,
            occupied: false,
        }
    }
}

/// A rectangular grid of tiles representing the tactical map.
///
/// Tiles are stored in row-major order: index = y * width + x.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileMap {
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<TileInfo>,
}

impl TileMap {
    /// Create a map filled with a uniform tile type.
    pub fn new_uniform(width: u32, height: u32, tile: TileInfo) -> Self {
        Self {
            width,
            height,
            tiles: vec![tile; (width * height) as usize],
        }
    }

    /// Bounds-checked tile access.
    pub fn get(&self, x: i32, y: i32) -> Option<&TileInfo> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        self.tiles.get((y as u32 * self.width + x as u32) as usize)
    }

    /// Mutable bounds-checked tile access.
    pub fn get_mut(&mut self, x: i32, y: i32) -> Option<&mut TileInfo> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        self.tiles.get_mut((y as u32 * self.width + x as u32) as usize)
    }

    /// Whether `pos` is in-bounds and passable (walkable + unoccupied).
    pub fn is_walkable(&self, pos: TilePos) -> bool {
        self.get(pos.x, pos.y)
            .map(|t| t.walkable && !t.occupied)
            .unwrap_or(false)
    }

    /// AP cost (in tenths) to enter the tile at `pos`.
    ///
    /// Returns `None` for impassable tiles.
    pub fn movement_cost(&self, pos: TilePos) -> Option<u32> {
        let tile = self.get(pos.x, pos.y)?;
        if !tile.walkable || tile.occupied {
            return None;
        }
        Some(tile.terrain.cost_multiplier_tenths())
    }

    /// Whether the tile at `pos` blocks line of sight.
    pub fn blocks_sight(&self, pos: TilePos) -> bool {
        self.get(pos.x, pos.y)
            .map(|t| t.terrain.blocks_sight())
            .unwrap_or(true) // out-of-bounds blocks sight
    }
}

// ---------------------------------------------------------------------------
// 8-directional neighbours
// ---------------------------------------------------------------------------

/// The 8 movement directions on the grid. Cardinal directions have a diagonal
/// multiplier of 10 (i.e. 1.0×) and diagonals use 14 (≈ √2 ≈ 1.414×).
const DIRECTIONS: [(i32, i32, u32); 8] = [
    // (dx, dy, diagonal_multiplier_tenths)
    (0, -1, 10),  // N
    (1, 0, 10),   // E
    (0, 1, 10),   // S
    (-1, 0, 10),  // W
    (1, -1, 14),  // NE (diagonal)
    (1, 1, 14),   // SE (diagonal)
    (-1, 1, 14),  // SW (diagonal)
    (-1, -1, 14), // NW (diagonal)
];

/// Compute the walkable neighbours of `pos` and their movement costs in AP-tenths.
fn successors(map: &TileMap, pos: TilePos) -> Vec<(TilePos, u32)> {
    let mut out = Vec::with_capacity(8);
    for &(dx, dy, diag_mul) in &DIRECTIONS {
        let next = TilePos {
            x: pos.x + dx,
            y: pos.y + dy,
        };
        if let Some(terrain_cost) = map.movement_cost(next) {
            // Total cost = terrain_multiplier × diagonal_multiplier / 10
            // Both are already in tenths, so we divide once to stay in tenths.
            let cost = (terrain_cost * diag_mul) / 10;
            out.push((next, cost.max(1))); // floor at 1 tenth to avoid zero-cost moves
        }
    }
    out
}

/// Manhattan-style heuristic for A* on the tile grid.
///
/// We use Chebyshev distance (max of axis deltas) scaled to tenths, which is
/// admissible for 8-directional movement with a minimum per-tile cost of 7
/// (road terrain).
fn heuristic(a: TilePos, b: TilePos) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    // Chebyshev: max(dx, dy) cardinal moves, with diagonals "free"
    // Use minimum possible cost (7 = road) to stay admissible.
    let cardinal = dx.max(dy);
    let diagonal = dx.min(dy);
    // Cost estimate: diagonal steps cost 14*7/10=~10, cardinal steps cost 7.
    // Simplified admissible formula:
    (cardinal - diagonal) * 7 + diagonal * 10
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Find the shortest path from `start` to `goal` on the tile grid.
///
/// Returns `Some((path, total_ap_cost))` where `path` includes both endpoints
/// and `total_ap_cost` is in whole AP (tenths divided by 10, rounded up).
///
/// Returns `None` if no path exists or if the cheapest path exceeds `max_ap`.
pub fn find_path(
    map: &TileMap,
    start: TilePos,
    goal: TilePos,
    max_ap: u32,
) -> Option<(Vec<TilePos>, u32)> {
    debug!(
        ?start,
        ?goal,
        max_ap,
        "Pathfinding request"
    );

    if start == goal {
        trace!("Start equals goal — trivial path");
        return Some((vec![start], 0));
    }

    // Goal must itself be walkable (or be the start tile, handled above).
    if !map.is_walkable(goal) {
        debug!(?goal, "Goal tile is not walkable");
        return None;
    }

    let result = astar(
        &start,
        |&pos| successors(map, pos),
        |&pos| heuristic(pos, goal),
        |&pos| pos == goal,
    );

    match result {
        Some((path, cost_tenths)) => {
            // Convert tenths → whole AP, rounding up.
            let ap_cost = cost_tenths.div_ceil(10);
            if ap_cost > max_ap {
                debug!(
                    ap_cost,
                    max_ap,
                    "Path found but exceeds AP budget"
                );
                return None;
            }
            debug!(
                path_len = path.len(),
                ap_cost,
                cost_tenths,
                "Path found"
            );
            Some((path, ap_cost))
        }
        None => {
            debug!("No path exists to goal");
            None
        }
    }
}

/// Return every tile reachable from `start` within `max_ap`, along with the
/// cost to reach each one (in whole AP).
///
/// Implemented via Dijkstra flood-fill. The result is used by the UI to show
/// a movement-range overlay.
pub fn reachable_tiles(map: &TileMap, start: TilePos, max_ap: u32) -> Vec<(TilePos, u32)> {
    use std::collections::BinaryHeap;
    use std::cmp::Reverse;

    debug!(?start, max_ap, "Computing reachable tiles");

    let max_tenths = max_ap * 10;
    let mut best: HashMap<TilePos, u32> = HashMap::new();
    // Min-heap: (cost_tenths, pos)
    let mut heap: BinaryHeap<Reverse<(u32, TilePos)>> = BinaryHeap::new();

    best.insert(start, 0);
    heap.push(Reverse((0, start)));

    while let Some(Reverse((cost, pos))) = heap.pop() {
        // Skip if we already found a cheaper route.
        if cost > *best.get(&pos).unwrap_or(&u32::MAX) {
            continue;
        }

        for (next, step_cost) in successors(map, pos) {
            let new_cost = cost + step_cost;
            if new_cost > max_tenths {
                continue;
            }
            let prev = best.get(&next).copied().unwrap_or(u32::MAX);
            if new_cost < prev {
                best.insert(next, new_cost);
                heap.push(Reverse((new_cost, next)));
            }
        }
    }

    // Convert to whole AP (ceiling) and collect. Exclude the start tile itself.
    let mut result: Vec<(TilePos, u32)> = best
        .into_iter()
        .filter(|(pos, _)| *pos != start)
        .map(|(pos, tenths)| (pos, tenths.div_ceil(10)))
        .collect();

    result.sort_by_key(|(pos, cost)| (*cost, pos.x, pos.y));

    debug!(count = result.len(), "Reachable tiles computed");
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple open 10×10 map.
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

    /// Place an occupied tile at (x, y).
    fn set_occupied(map: &mut TileMap, x: i32, y: i32) {
        if let Some(tile) = map.get_mut(x, y) {
            tile.occupied = true;
        }
    }

    #[test]
    fn straight_line_path() {
        let map = open_map(10, 10);
        let start = TilePos { x: 0, y: 0 };
        let goal = TilePos { x: 5, y: 0 };
        let result = find_path(&map, start, goal, 100);
        assert!(result.is_some());
        let (path, cost) = result.unwrap();
        // Should go straight east: 5 steps, each costing 10 tenths = 1 AP.
        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&goal));
        assert_eq!(cost, 5);
    }

    #[test]
    fn path_around_obstacle() {
        let mut map = open_map(10, 10);
        // Wall across y=2, from x=0..=4
        for x in 0..=4 {
            set_wall(&mut map, x, 2);
        }
        let start = TilePos { x: 2, y: 0 };
        let goal = TilePos { x: 2, y: 4 };
        let result = find_path(&map, start, goal, 100);
        assert!(result.is_some());
        let (path, _cost) = result.unwrap();
        // Path must go around the wall — none of the path tiles should be walls.
        for pos in &path {
            assert!(map.is_walkable(*pos), "Path goes through non-walkable tile {:?}", pos);
        }
        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&goal));
    }

    #[test]
    fn diagonal_preference_shorter() {
        // Going diagonally to (3,3) should be cheaper than going L-shaped.
        let map = open_map(10, 10);
        let start = TilePos { x: 0, y: 0 };
        let goal = TilePos { x: 3, y: 3 };
        let result = find_path(&map, start, goal, 100);
        assert!(result.is_some());
        let (path, cost) = result.unwrap();
        // Pure diagonal: 3 steps × 14 tenths = 42 tenths → 5 AP (ceil).
        // L-shaped cardinal: 6 steps × 10 tenths = 60 tenths → 6 AP.
        // A* should prefer the diagonal.
        assert!(cost <= 5, "Diagonal path should cost ≤5 AP, got {cost}");
        assert_eq!(path.len(), 4); // start + 3 diagonal steps
    }

    #[test]
    fn max_ap_limit_blocks_path() {
        let map = open_map(10, 10);
        let start = TilePos { x: 0, y: 0 };
        let goal = TilePos { x: 9, y: 0 };
        // 9 tiles east = 9 AP; budget of 5 should fail.
        let result = find_path(&map, start, goal, 5);
        assert!(result.is_none());
    }

    #[test]
    fn unreachable_goal_walled_off() {
        let mut map = open_map(10, 10);
        // Surround (5,5) with walls on all 8 neighbours.
        for dx in -1..=1 {
            for dy in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                set_wall(&mut map, 5 + dx, 5 + dy);
            }
        }
        let start = TilePos { x: 0, y: 0 };
        let goal = TilePos { x: 5, y: 5 };
        let result = find_path(&map, start, goal, 100);
        // Goal is walkable, but all approaches are walled — unreachable.
        assert!(result.is_none());
    }

    #[test]
    fn occupied_tile_blocks_movement() {
        let mut map = open_map(5, 1);
        // Block the only path through (2,0).
        set_occupied(&mut map, 2, 0);
        let start = TilePos { x: 0, y: 0 };
        let goal = TilePos { x: 4, y: 0 };
        let result = find_path(&map, start, goal, 100);
        assert!(result.is_none());
    }

    #[test]
    fn trivial_path_start_equals_goal() {
        let map = open_map(5, 5);
        let pos = TilePos { x: 2, y: 2 };
        let result = find_path(&map, pos, pos, 0);
        assert!(result.is_some());
        let (path, cost) = result.unwrap();
        assert_eq!(path, vec![pos]);
        assert_eq!(cost, 0);
    }

    #[test]
    fn reachable_tiles_small_budget() {
        let map = open_map(10, 10);
        let start = TilePos { x: 5, y: 5 };
        let reachable = reachable_tiles(&map, start, 1);
        // With 1 AP (= 10 tenths), we can reach the 4 cardinal neighbours
        // (cost 10 tenths each). Diagonals cost 14 tenths → too expensive.
        assert_eq!(reachable.len(), 4, "Should reach exactly 4 cardinal tiles with 1 AP");
        for (pos, cost) in &reachable {
            assert_eq!(*cost, 1);
            let dx = (pos.x - 5).abs();
            let dy = (pos.y - 5).abs();
            assert_eq!(dx + dy, 1, "Should be cardinal neighbour");
        }
    }

    #[test]
    fn reachable_tiles_with_obstacle() {
        let mut map = open_map(5, 5);
        let start = TilePos { x: 2, y: 2 };
        // Wall off one cardinal direction.
        set_wall(&mut map, 3, 2);
        let reachable = reachable_tiles(&map, start, 1);
        // 3 cardinal neighbours (N, S, W) — E is walled.
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn road_terrain_is_cheaper() {
        let mut map = open_map(10, 1);
        // Pave the road from x=1..=8.
        for x in 1..=8 {
            if let Some(tile) = map.get_mut(x, 0) {
                tile.terrain = TerrainType::Road;
            }
        }
        let start = TilePos { x: 0, y: 0 };
        let goal = TilePos { x: 9, y: 0 };
        let result = find_path(&map, start, goal, 100);
        assert!(result.is_some());
        let (_path, cost) = result.unwrap();
        // 1 open step (10) + 8 road steps (7 each = 56) + last open step...
        // Actually tile 0 is open, tiles 1-8 are road, tile 9 is open.
        // Cost to enter each: open=10, road=7.
        // Path enters tiles 1..=9: 8×7 + 1×10 = 66 tenths → 7 AP.
        assert_eq!(cost, 7, "Road path should be 7 AP");
    }
}
