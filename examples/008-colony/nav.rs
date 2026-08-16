use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::map;

/// Integer step costs (14/10 ~ sqrt(2)) keep A* free of float ordering.
const CARDINAL_COST: u32 = 10;
const DIAGONAL_COST: u32 = 14;

const DIRECTIONS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// Octile distance: the exact cost of the best obstacle-free 8-way path.
fn heuristic(a: GridCoords, b: GridCoords) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    DIAGONAL_COST * dx.min(dy) + CARDINAL_COST * (dx.max(dy) - dx.min(dy))
}

/// Which cells can be traversed and how expensive they are. Starts
/// all-walkable at dirt cost; carved/priced as the LDtk cells spawn (over
/// the first few frames after asset load).
#[derive(Resource)]
pub struct NavGrid {
    walkable: Vec<bool>,
    /// Per-cell traversal cost (TERRAIN_COST_DIRT baseline; water is more).
    cost: Vec<u32>,
    /// Additive snow-drift penalty layered over `cost` (see
    /// `snow::sync_snow_costs`). Its own vec so the base-cost writers
    /// (water, trees, construction) and the snow never stomp each other.
    snow: Vec<u32>,
}

impl Default for NavGrid {
    fn default() -> Self {
        Self {
            walkable: vec![true; (MAP_WIDTH * MAP_HEIGHT) as usize],
            cost: vec![TERRAIN_COST_DIRT; (MAP_WIDTH * MAP_HEIGHT) as usize],
            snow: vec![0; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl NavGrid {
    fn index(cell: GridCoords) -> usize {
        (cell.y * MAP_WIDTH + cell.x) as usize
    }

    fn cell(index: usize) -> GridCoords {
        GridCoords::new(index as i32 % MAP_WIDTH, index as i32 / MAP_WIDTH)
    }

    pub fn is_walkable(&self, cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x)
            && (0..MAP_HEIGHT).contains(&cell.y)
            && self.walkable[Self::index(cell)]
    }

    pub fn set_walkable(&mut self, cell: GridCoords, walkable: bool) {
        if (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y) {
            self.walkable[Self::index(cell)] = walkable;
        }
    }

    /// Traversal cost of a cell: the terrain base plus the snow penalty
    /// (dirt baseline when out of bounds; callers check walkability
    /// separately). The single source for A* step costs AND walk speed, so
    /// snow slows pawns exactly as much as it deters the planner.
    pub fn cost(&self, cell: GridCoords) -> u32 {
        if (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y) {
            let index = Self::index(cell);
            self.cost[index] + self.snow[index]
        } else {
            TERRAIN_COST_DIRT
        }
    }

    pub fn set_cost(&mut self, cell: GridCoords, cost: u32) {
        if (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y) {
            self.cost[Self::index(cell)] = cost;
        }
    }

    pub fn snow_penalty(&self, cell: GridCoords) -> u32 {
        if (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y) {
            self.snow[Self::index(cell)]
        } else {
            0
        }
    }

    pub fn set_snow_penalty(&mut self, cell: GridCoords, penalty: u32) {
        if (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y) {
            self.snow[Self::index(cell)] = penalty;
        }
    }

    /// A* from `from` to `to`, 8-way with no corner cutting: a diagonal step
    /// is legal only if both flanking cardinal cells are walkable. Returns
    /// the cells to visit in order (start excluded, goal included), an empty
    /// vec if already there, or `None` if unreachable.
    pub fn find_path(&self, from: GridCoords, to: GridCoords) -> Option<Vec<GridCoords>> {
        self.find_path_avoiding(from, to, &HashSet::new())
    }

    /// Same as `find_path`, but any cell in `blocked` is treated as
    /// unwalkable for this search only (used to route pawns around each
    /// other without mutating the shared terrain grid). Callers must not
    /// include `to` in `blocked`, or the goal becomes unreachable.
    pub fn find_path_avoiding(
        &self,
        from: GridCoords,
        to: GridCoords,
        blocked: &HashSet<GridCoords>,
    ) -> Option<Vec<GridCoords>> {
        if !self.is_walkable(from) || !self.is_walkable(to) {
            return None;
        }
        if from == to {
            return Some(Vec::new());
        }

        let start = Self::index(from);
        let goal = Self::index(to);
        let mut g = vec![u32::MAX; self.walkable.len()];
        let mut came_from = vec![usize::MAX; self.walkable.len()];
        // Reverse((f, g, index)) makes the max-heap pop the lowest f first.
        let mut frontier = BinaryHeap::new();

        g[start] = 0;
        frontier.push(Reverse((heuristic(from, to), 0, start)));

        while let Some(Reverse((_, popped_g, current))) = frontier.pop() {
            if popped_g != g[current] {
                continue; // stale duplicate, a cheaper route got there first
            }
            if current == goal {
                let mut path = Vec::new();
                let mut index = goal;
                while index != start {
                    path.push(Self::cell(index));
                    index = came_from[index];
                }
                path.reverse();
                return Some(path);
            }

            let cell = Self::cell(current);
            for (dx, dy) in DIRECTIONS {
                let neighbor = GridCoords::new(cell.x + dx, cell.y + dy);
                if !self.is_walkable(neighbor) || blocked.contains(&neighbor) {
                    continue;
                }
                let diagonal = dx != 0 && dy != 0;
                if diagonal
                    && !(self.is_walkable(GridCoords::new(cell.x + dx, cell.y))
                        && self.is_walkable(GridCoords::new(cell.x, cell.y + dy)))
                {
                    continue;
                }
                // Step cost scales with the destination cell's terrain plus
                // its snow drift (dirt = the 10-unit baseline the heuristic
                // assumes; penalties only ever add, so it stays admissible).
                let neighbor_index = Self::index(neighbor);
                let terrain = self.cost[neighbor_index] + self.snow[neighbor_index];
                let step = if diagonal {
                    terrain * DIAGONAL_COST / CARDINAL_COST
                } else {
                    terrain
                };
                let tentative = g[current] + step;
                if tentative < g[neighbor_index] {
                    g[neighbor_index] = tentative;
                    came_from[neighbor_index] = current;
                    frontier.push(Reverse((
                        tentative + heuristic(neighbor, to),
                        tentative,
                        neighbor_index,
                    )));
                }
            }
        }
        None
    }
}

/// Traversal cost for a cell holding `depth` of water (world units). Dry
/// land (including a drained bed) falls back to the dirt baseline;
/// anything past the shallow class costs like deep water.
pub fn water_cost(depth: f32) -> u32 {
    if depth <= 0.0 {
        TERRAIN_COST_DIRT
    } else if map::is_deep(depth) {
        TERRAIN_COST_DEEP
    } else {
        TERRAIN_COST_SHALLOW
    }
}

/// What a bed cell costs right now: solid ice walks like dirt, otherwise
/// the water class for the live depth.
pub fn ice_or_water_cost(depth: f32, frozen: bool) -> u32 {
    if frozen && depth > 0.0 {
        TERRAIN_COST_ICE
    } else {
        water_cost(depth)
    }
}

pub fn mark_water(
    shallows: Query<&GridCoords, Added<map::ShallowWaterCell>>,
    deeps: Query<&GridCoords, Added<map::DeepWaterCell>>,
    water: Res<map::WaterMap>,
    mut nav: ResMut<NavGrid>,
) {
    for grid in shallows.iter().chain(deeps.iter()) {
        nav.set_cost(*grid, water_cost(water.depth(*grid)));
    }
}

/// Follow a level change — or the ice threshold flipping — by re-deriving
/// every bed cell's cost from its live column and the freeze state.
/// Stomping is safe — nothing else may occupy a bed cell (walls, doors and
/// trees are all gated off the beds), so the water/ice class is the only
/// cost a bed cell can carry. Costs steer *future* plans; pawns already
/// mid-river keep their path and just wade (or stride the fresh ice) at
/// the new cost.
pub fn sync_water_costs(
    water: Res<map::WaterMap>,
    freeze: Res<crate::snow::FreezeLevel>,
    mut nav: ResMut<NavGrid>,
    mut last_frozen: Local<Option<bool>>,
) {
    let frozen = freeze.is_frozen();
    if !water.is_changed() && *last_frozen == Some(frozen) {
        return;
    }
    *last_frozen = Some(frozen);
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            if water.has_bed(cell) {
                nav.set_cost(cell, ice_or_water_cost(water.depth(cell), frozen));
            }
        }
    }
}

/// A tree slows its cell down (pushing through the woods). Trees never
/// despawn yet; a future chopping mechanic must revert the cost from
/// `TerrainMap`.
pub fn mark_trees(trees: Query<&GridCoords, Added<crate::trees::Tree>>, mut nav: ResMut<NavGrid>) {
    for grid in &trees {
        nav.set_cost(*grid, TERRAIN_COST_TREE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    #[test]
    fn water_cost_tracks_the_depth_classes() {
        assert_eq!(
            water_cost(0.0),
            TERRAIN_COST_DIRT,
            "dry land — including a drained bed — is baseline"
        );
        // The columns beds carry at the starting level.
        assert_eq!(
            water_cost(WATER_SURFACE_Y + WATER_BED_SHALLOW),
            TERRAIN_COST_SHALLOW
        );
        assert_eq!(
            water_cost(WATER_SURFACE_Y + WATER_BED_DEEP),
            TERRAIN_COST_DEEP
        );
        assert_eq!(
            water_cost(WATER_DEEP_MIN_DEPTH),
            TERRAIN_COST_SHALLOW,
            "the shallow class runs up to and including the threshold"
        );
    }

    #[test]
    fn straight_line() {
        let nav = NavGrid::default();
        let path = nav.find_path(at(0, 0), at(4, 0)).unwrap();
        assert_eq!(path.len(), 4);
        assert_eq!(*path.last().unwrap(), at(4, 0));
    }

    #[test]
    fn diagonal_line() {
        let nav = NavGrid::default();
        let path = nav.find_path(at(0, 0), at(3, 3)).unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path, vec![at(1, 1), at(2, 2), at(3, 3)]);
    }

    #[test]
    fn detours_around_wall() {
        let mut nav = NavGrid::default();
        // Wall column at x = 5 with a single gap at y = 10.
        for y in 0..MAP_HEIGHT {
            if y != 10 {
                nav.set_walkable(at(5, y), false);
            }
        }
        let path = nav.find_path(at(0, 5), at(10, 5)).unwrap();
        assert!(path.contains(&at(5, 10)), "must pass through the gap");
        assert_eq!(*path.last().unwrap(), at(10, 5));
    }

    #[test]
    fn no_corner_cutting() {
        let mut nav = NavGrid::default();
        // Seal (0,0) except for the diagonal squeeze toward (1,1).
        nav.set_walkable(at(1, 0), false);
        nav.set_walkable(at(0, 1), false);
        assert_eq!(nav.find_path(at(0, 0), at(1, 1)), None);
    }

    #[test]
    fn unreachable_enclosed_goal() {
        let mut nav = NavGrid::default();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if (dx, dy) != (0, 0) {
                    nav.set_walkable(at(15 + dx, 15 + dy), false);
                }
            }
        }
        assert_eq!(nav.find_path(at(0, 0), at(15, 15)), None);
    }

    #[test]
    fn out_of_bounds() {
        let nav = NavGrid::default();
        assert_eq!(nav.find_path(at(0, 0), at(-1, 0)), None);
        assert_eq!(nav.find_path(at(0, 0), at(MAP_WIDTH, 0)), None);
        assert_eq!(nav.find_path(at(-1, 0), at(0, 0)), None);
    }

    #[test]
    fn same_cell() {
        let nav = NavGrid::default();
        assert_eq!(nav.find_path(at(7, 7), at(7, 7)), Some(Vec::new()));
    }

    #[test]
    fn deep_water_detour() {
        let mut nav = NavGrid::default();
        // Deep river column at x = 2 with a shallow ford at (2, 7).
        for y in 0..MAP_HEIGHT {
            nav.set_cost(at(2, y), TERRAIN_COST_DEEP);
        }
        nav.set_cost(at(2, 7), TERRAIN_COST_SHALLOW);
        // Straight through deep water costs 40; angling down to the ford
        // (14 + 21 + 14 + 14 = 63 total) beats the straight 70.
        let path = nav.find_path(at(0, 5), at(4, 5)).unwrap();
        assert!(path.contains(&at(2, 7)), "must cross at the shallow ford");
    }

    #[test]
    fn ice_cost_tracks_the_freeze_state() {
        let deep = WATER_SURFACE_Y + WATER_BED_DEEP;
        assert_eq!(ice_or_water_cost(deep, false), TERRAIN_COST_DEEP);
        assert_eq!(ice_or_water_cost(deep, true), TERRAIN_COST_ICE);
        assert_eq!(ice_or_water_cost(0.1, true), TERRAIN_COST_ICE);
        // A drained bed is dirt whether or not the world is frozen.
        assert_eq!(ice_or_water_cost(0.0, true), TERRAIN_COST_DIRT);
    }

    #[test]
    fn a_frozen_river_is_no_longer_a_detour() {
        let mut nav = NavGrid::default();
        // The `deep_water_detour` river, frozen solid: every bed cell costs
        // like dirt, so the straight line beats angling to the old ford.
        for y in 0..MAP_HEIGHT {
            nav.set_cost(
                at(2, y),
                ice_or_water_cost(WATER_SURFACE_Y + WATER_BED_DEEP, true),
            );
        }
        let path = nav.find_path(at(0, 5), at(4, 5)).unwrap();
        assert_eq!(path.len(), 4, "straight across the ice");
        assert!(path.contains(&at(2, 5)));
    }

    #[test]
    fn paths_route_around_deep_drifts() {
        let mut nav = NavGrid::default();
        // Full-cover drift column at x = 2 with a bare gap at (2, 6): the
        // snow-penalty analogue of `deep_water_detour`. The gap sits one
        // row off the straight line: angling through it costs 48 vs 52
        // through the drift (a +12 drift deters short detours, not the
        // two-row trek deep water justifies).
        for y in 0..MAP_HEIGHT {
            if y != 6 {
                nav.set_snow_penalty(at(2, y), 4 * SNOW_COST_PER_BUCKET);
            }
        }
        let path = nav.find_path(at(0, 5), at(4, 5)).unwrap();
        assert!(path.contains(&at(2, 6)), "must cross at the bare gap");
        // And the penalty reads back through the combined cost.
        assert_eq!(
            nav.cost(at(2, 5)),
            TERRAIN_COST_DIRT + 4 * SNOW_COST_PER_BUCKET
        );
    }

    #[test]
    fn water_passable_when_only_option() {
        let mut nav = NavGrid::default();
        // Unbroken deep river column: expensive, but never a wall.
        for y in 0..MAP_HEIGHT {
            nav.set_cost(at(2, y), TERRAIN_COST_DEEP);
        }
        let path = nav.find_path(at(0, 5), at(4, 5)).unwrap();
        assert_eq!(*path.last().unwrap(), at(4, 5));
        assert_eq!(
            path.iter().filter(|cell| cell.x == 2).count(),
            1,
            "crosses the river exactly once"
        );
    }

    #[test]
    fn shortest_around_single_wall() {
        let mut nav = NavGrid::default();
        nav.set_walkable(at(2, 5), false);
        // Best route around one blocking cell: two diagonals + two cardinals,
        // still 4 steps like the straight line would have been.
        let path = nav.find_path(at(0, 5), at(4, 5)).unwrap();
        assert_eq!(path.len(), 4);
        assert!(!path.contains(&at(2, 5)));
    }

    #[test]
    fn avoiding_detours_around_a_blocked_cell_like_a_wall() {
        let nav = NavGrid::default();
        // Terrain is fully open; only the `blocked` set (another pawn)
        // stands in the way, exactly like `shortest_around_single_wall`.
        let blocked: HashSet<GridCoords> = [at(2, 5)].into_iter().collect();
        let path = nav
            .find_path_avoiding(at(0, 5), at(4, 5), &blocked)
            .unwrap();
        assert_eq!(path.len(), 4);
        assert!(!path.contains(&at(2, 5)));
    }

    #[test]
    fn avoiding_a_sealed_corridor_is_unreachable() {
        let nav = NavGrid::default();
        // Block every cell of the only corridor at x = 5 (mirrors
        // `detours_around_wall`, but via occupancy instead of terrain).
        let blocked: HashSet<GridCoords> = (0..MAP_HEIGHT).map(|y| at(5, y)).collect();
        assert_eq!(nav.find_path_avoiding(at(0, 5), at(10, 5), &blocked), None);
    }

    #[test]
    fn avoiding_the_goal_itself_makes_it_unreachable() {
        // Documents the contract: callers must keep `to` out of `blocked`,
        // or the goal becomes unreachable like any other blocked cell.
        let nav = NavGrid::default();
        let blocked: HashSet<GridCoords> = [at(4, 5)].into_iter().collect();
        assert!(nav
            .find_path_avoiding(at(0, 5), at(4, 5), &blocked)
            .is_none());
    }
}
