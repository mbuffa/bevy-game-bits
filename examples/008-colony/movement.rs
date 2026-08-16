use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::map;
use crate::nav::NavGrid;
use crate::units::Pawn;

/// Intent: walk to `goal`. Inserted by the right-click dev tool today and
/// the AI Director later; consumed by `plan_paths`.
#[derive(Component)]
pub struct MoveOrder {
    pub goal: GridCoords,
}

/// The waypoints (cell centers, at the entity's own height) still to visit.
/// Positions along the way are floats; only waypoints sit on cell centers.
#[derive(Component)]
pub struct Path {
    waypoints: Vec<Vec3>,
    next: usize,
    /// Final destination, kept around so a blocked pawn can re-plan a fresh
    /// route to the same place instead of giving up.
    goal: GridCoords,
    /// The cell this pawn is currently crossing into, held for the whole
    /// crossing so no one else can claim or cut through it meanwhile. `None`
    /// while standing still or freshly (re)planned.
    reserved: Option<GridCoords>,
    /// Seconds spent unable to advance because another pawn holds the next
    /// tile; past `BLOCKED_REPLAN_SECS` the pawn re-plans around it.
    blocked: f32,
}

/// Which pawn currently claims each tile: the cell it stands on, plus (while
/// mid-crossing) the cell it's walking into. Rebuilt fresh every frame from
/// `GridCoords` and each `Path`'s reservation; mutated live during
/// `move_along_path` so later pawns in the same frame see earlier ones'
/// fresh claims.
#[derive(Resource, Default)]
pub struct Occupancy(HashMap<GridCoords, Entity>);

impl Occupancy {
    fn clear(&mut self) {
        self.0.clear();
    }

    fn claim(&mut self, cell: GridCoords, entity: Entity) {
        self.0.insert(cell, entity);
    }

    /// True when `cell` is currently claimed by some pawn other than `me`.
    pub fn blocked_by_other(&self, cell: GridCoords, me: Entity) -> bool {
        matches!(self.0.get(&cell), Some(&owner) if owner != me)
    }

    /// Every cell claimed by some other pawn, except `goal` — a pawn may
    /// still plan a route *to* a cell someone else stands on; it simply
    /// yields there at runtime if the occupant hasn't moved on yet.
    fn blocked_for(&self, me: Entity, goal: GridCoords) -> HashSet<GridCoords> {
        self.0
            .iter()
            .filter(|&(&cell, &owner)| owner != me && cell != goal)
            .map(|(&cell, _)| cell)
            .collect()
    }
}

/// Runs before pathing/movement each frame so both see this frame's live
/// positions, not last frame's.
pub fn rebuild_occupancy(
    mut occupancy: ResMut<Occupancy>,
    movers: Query<(Entity, &GridCoords, Option<&Path>), With<Pawn>>,
) {
    occupancy.clear();
    for (entity, grid, path) in &movers {
        occupancy.claim(*grid, entity);
        if let Some(reserved) = path.and_then(|p| p.reserved) {
            occupancy.claim(reserved, entity);
        }
    }
}

/// Dips below its base height while crossing water (see `wade_dip`).
/// `base_y` is the entity's resting height on dry ground — waypoints use it
/// instead of the live translation, so a replan issued mid-ford doesn't
/// bake the dipped height into the rest of the route.
#[derive(Component)]
pub struct Wader {
    pub base_y: f32,
}

/// How far a wader's visual sits below its dry-ground height at a cell.
/// While water is present it walks the bed, capped so deep water reads as
/// a hard ford, not a drowning. On a *drained* bed there's nothing to
/// float in — the pawn simply stands on the lower ground, full recess.
pub fn wade_dip(bed: f32, depth: f32) -> f32 {
    if depth > 0.0 {
        bed.min(WADE_MAX_DEPTH)
    } else {
        bed
    }
}

pub fn plan_paths(
    mut commands: Commands,
    nav: Res<NavGrid>,
    occupancy: Res<Occupancy>,
    movers: Query<(Entity, &Transform, &MoveOrder, Option<&Wader>)>,
) {
    for (entity, transform, order, wader) in &movers {
        let from = map::world_to_grid(transform.translation);
        let mut entity_commands = commands.entity(entity);
        entity_commands.remove::<MoveOrder>();
        // Route around other pawns when possible; a pawn boxed in by
        // occupants (rather than terrain) still gets a route and yields at
        // runtime instead of having its order silently dropped.
        let blocked = occupancy.blocked_for(entity, order.goal);
        let cells = nav
            .find_path_avoiding(from, order.goal, &blocked)
            .or_else(|| nav.find_path(from, order.goal));
        match cells {
            Some(cells) => {
                let y = wader.map_or(transform.translation.y, |wader| wader.base_y);
                let waypoints = cells
                    .iter()
                    .map(|cell| map::grid_to_world(cell).with_y(y))
                    .collect();
                entity_commands.insert(Path {
                    waypoints,
                    next: 0,
                    goal: order.goal,
                    reserved: None,
                    blocked: 0.0,
                });
            }
            None => warn!("move order dropped: no path {from:?} -> {:?}", order.goal),
        }
    }
}

pub fn move_along_path(
    mut commands: Commands,
    time: Res<Time>,
    nav: Res<NavGrid>,
    mut occupancy: ResMut<Occupancy>,
    water: Res<map::WaterMap>,
    freeze: Res<crate::snow::FreezeLevel>,
    mut movers: Query<(Entity, &mut Transform, &mut Path, Option<&Wader>)>,
) {
    for (entity, mut transform, mut path, wader) in &mut movers {
        // Waders sink below their base height mid-ford; reset before the
        // waypoint math so the distance budget stays planar, re-dip after.
        if let Some(wader) = wader {
            transform.translation.y = wader.base_y;
        }
        // Terrain slows walking: the cell currently under the mover scales
        // its speed (dirt = full speed, water = the TERRAIN_COST_* ratio).
        let cell = map::world_to_grid(transform.translation);
        let terrain_factor = TERRAIN_COST_DIRT as f32 / nav.cost(cell) as f32;
        // Distance budget for this frame; crossing a waypoint spends only
        // the distance up to it, so corners don't cost an extra frame.
        let mut budget = PAWN_SPEED * terrain_factor * time.delta_secs();
        loop {
            let Some(target) = path.waypoints.get(path.next).copied() else {
                commands.entity(entity).remove::<Path>();
                break;
            };
            let here = map::world_to_grid(transform.translation);
            let there = map::world_to_grid(target);
            if there != here && occupancy.blocked_by_other(there, entity) {
                // Another pawn holds the next tile: wait this frame. If it
                // hasn't cleared after a short while, give up on this route
                // and re-plan around whatever is blocking it.
                path.blocked += time.delta_secs();
                if path.blocked >= BLOCKED_REPLAN_SECS {
                    let goal = path.goal;
                    commands
                        .entity(entity)
                        .remove::<Path>()
                        .insert(MoveOrder { goal });
                }
                break;
            }
            path.blocked = 0.0;
            if there != here {
                // Hold the destination for the whole crossing so no one else
                // can claim or cut through it meanwhile.
                path.reserved = Some(there);
                occupancy.claim(there, entity);
            }
            let to_target = target - transform.translation;
            let distance = to_target.length();
            if distance <= budget {
                transform.translation = target;
                path.next += 1;
                budget -= distance;
            } else {
                transform.translation += to_target / distance * budget;
                break;
            }
        }
        if let Some(wader) = wader {
            let cell = map::world_to_grid(transform.translation);
            // Solid ice carries the pawn on the surface — no dip while a
            // frozen column stands there. A drained bed still recesses
            // (there's ground down there, frozen or not).
            let dip = if freeze.is_frozen() && water.depth(cell) > 0.0 {
                0.0
            } else {
                wade_dip(water.bed(cell), water.depth(cell))
            };
            transform.translation.y = wader.base_y - dip;
        }
    }
}

/// Transform is the authoritative position of a moving entity; keep its
/// `GridCoords` cache (used by selection and nav queries) following along.
#[allow(clippy::type_complexity)]
pub fn sync_grid_coords(
    mut movers: Query<(&Transform, &mut GridCoords), (With<Path>, Changed<Transform>)>,
) {
    for (transform, mut grid) in &mut movers {
        let cell = map::world_to_grid(transform.translation);
        if *grid != cell {
            *grid = cell;
        }
    }
}

/// Dev aid: draws every active path from the mover's current position.
pub fn debug_draw_paths(mut gizmos: Gizmos, movers: Query<(&Transform, &Path)>) {
    for (transform, path) in &movers {
        let points = std::iter::once(transform.translation)
            .chain(path.waypoints[path.next..].iter().copied())
            .map(|point| point.with_y(0.1));
        gizmos.linestrip(points, PATH_GIZMO_COLOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    #[test]
    fn wade_dip_walks_the_bed_capped_only_while_wet() {
        assert_eq!(wade_dip(0.0, 0.0), 0.0, "flat dry ground: no dip");
        assert_eq!(
            wade_dip(WATER_BED_SHALLOW, WATER_SURFACE_Y + WATER_BED_SHALLOW),
            WATER_BED_SHALLOW,
            "shallow water: down to the bed"
        );
        assert_eq!(
            wade_dip(WATER_BED_DEEP, WATER_SURFACE_Y + WATER_BED_DEEP),
            WADE_MAX_DEPTH,
            "deep water: capped, the pawn fords rather than drowns"
        );
        assert_eq!(
            wade_dip(WATER_BED_DEEP, 0.0),
            WATER_BED_DEEP,
            "a drained deep bed is just lower ground: full recess, no cap"
        );
    }

    #[test]
    fn a_pawns_own_cell_is_not_blocked_for_it() {
        let mut occupancy = Occupancy::default();
        let me = entity(1);
        occupancy.claim(at(0, 0), me);
        assert!(!occupancy.blocked_by_other(at(0, 0), me));
    }

    #[test]
    fn another_pawns_cell_is_blocked() {
        let mut occupancy = Occupancy::default();
        let me = entity(1);
        let other = entity(2);
        occupancy.claim(at(0, 0), other);
        assert!(occupancy.blocked_by_other(at(0, 0), me));
    }

    #[test]
    fn an_empty_cell_is_never_blocked() {
        let occupancy = Occupancy::default();
        let me = entity(1);
        assert!(!occupancy.blocked_by_other(at(3, 3), me));
    }

    #[test]
    fn blocked_for_excludes_self_and_the_goal() {
        let mut occupancy = Occupancy::default();
        let me = entity(1);
        let other = entity(2);
        occupancy.claim(at(0, 0), me); // my own cell
        occupancy.claim(at(1, 0), other); // someone else, not the goal
        occupancy.claim(at(2, 0), other); // someone else, sitting on the goal

        let blocked = occupancy.blocked_for(me, at(2, 0));
        assert!(!blocked.contains(&at(0, 0)), "self isn't blocked for self");
        assert!(blocked.contains(&at(1, 0)), "another pawn off-goal blocks");
        assert!(
            !blocked.contains(&at(2, 0)),
            "the goal cell is never blocked, even if occupied"
        );
    }
}
