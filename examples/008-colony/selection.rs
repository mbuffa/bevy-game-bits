use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::construction::Footprint;
use crate::map;
use crate::movement::MoveOrder;
use crate::scene::MainCamera;
use crate::units::{ManualMode, Pawn};
use crate::zones::ZoneRegion;

/// Anything that can be clicked on the map. Lives on the 3D game entities
/// (which carry `GridCoords`), not on the LDtk data entities.
#[derive(Component)]
pub struct Selectable;

#[derive(Resource, Default)]
pub struct SelectedEntity(pub Option<Entity>);

/// Cursor -> ray -> ground plane -> grid cell.
pub fn cursor_to_cell(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> Option<GridCoords> {
    let cursor = window.cursor_position()?;
    let ray = camera.viewport_to_world(camera_transform, cursor).ok()?;
    let t = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
    Some(map::world_to_grid(ray.get_point(t)))
}

/// Left click -> grid cell -> selectable entity on that cell (or deselect on
/// an empty cell). Clicking the tile of the current selection again cycles
/// through everything standing on it (pawn on a berry stack, etc.). Zones
/// aren't `Selectable` (they cover a whole rectangle, not one cell), so
/// they're appended after regular occupants: on an empty stockpile tile the
/// zone is picked first, but a pawn standing on it is picked before the
/// zone underneath.
pub fn click_select(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    selectables: Query<(Entity, &GridCoords), With<Selectable>>,
    footprints: Query<(Entity, &Footprint), With<Selectable>>,
    zones: Query<(Entity, &ZoneRegion)>,
    ui_nodes: Query<&Interaction>,
    mut selected: ResMut<SelectedEntity>,
) {
    // Drop a stale selection (e.g. the entity was despawned).
    if let Some(entity) = selected.0 {
        let still_exists = selectables.get(entity).is_ok() || zones.get(entity).is_ok();
        if !still_exists {
            selected.0 = None;
        }
    }

    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // The click landed on UI (info panel / button): don't pick through it.
    if ui_nodes.iter().any(|i| *i != Interaction::None) {
        return;
    }
    let (camera, camera_transform) = *camera;
    let Some(cell) = cursor_to_cell(&window, camera, camera_transform) else {
        return;
    };

    let mut on_cell: Vec<Entity> = selectables
        .iter()
        .filter(|(_, grid)| **grid == cell)
        .map(|(entity, _)| entity)
        .collect();
    // Multi-tile buildings (a 2x2 solar panel) select from any of their
    // footprint cells, not just the anchor `GridCoords` above — for a 1x1
    // building the footprint is just that one cell, so this never adds a
    // duplicate of what the pass above already found.
    for (entity, footprint) in &footprints {
        if footprint.contains(cell) && !on_cell.contains(&entity) {
            on_cell.push(entity);
        }
    }
    // Query iteration order is not stable frame to frame; sort so repeated
    // clicks visit every entity on the tile exactly once before wrapping.
    on_cell.sort();

    let mut zone_hits: Vec<Entity> = zones
        .iter()
        .filter(|(_, region)| region.contains(cell))
        .map(|(entity, _)| entity)
        .collect();
    zone_hits.sort();
    on_cell.extend(zone_hits);

    selected.0 = cycle_selection(&on_cell, selected.0);
}

/// Next selection after a click on a tile holding `on_cell` (sorted): the
/// first occupant normally, the next one (wrapping) when the current
/// selection already stands on this tile.
fn cycle_selection(on_cell: &[Entity], current: Option<Entity>) -> Option<Entity> {
    match current.and_then(|current| on_cell.iter().position(|entity| *entity == current)) {
        Some(index) => Some(on_cell[(index + 1) % on_cell.len()]),
        None => on_cell.first().copied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::new();
        let mut list: Vec<Entity> = (0..count).map(|_| world.spawn_empty().id()).collect();
        list.sort();
        list
    }

    #[test]
    fn empty_tile_deselects() {
        assert_eq!(cycle_selection(&[], None), None);
        let picked = entities(1);
        assert_eq!(cycle_selection(&[], Some(picked[0])), None);
    }

    #[test]
    fn fresh_click_picks_first() {
        let on_cell = entities(3);
        assert_eq!(cycle_selection(&on_cell, None), Some(on_cell[0]));
    }

    #[test]
    fn selection_elsewhere_picks_first() {
        let all = entities(3);
        let (on_cell, elsewhere) = all.split_at(2);
        assert_eq!(
            cycle_selection(on_cell, Some(elsewhere[0])),
            Some(on_cell[0])
        );
    }

    #[test]
    fn reclick_cycles_and_wraps() {
        let on_cell = entities(3);
        let mut current = None;
        // Four clicks: first pick, two cycles, then wrap back to the start.
        for expected in [on_cell[0], on_cell[1], on_cell[2], on_cell[0]] {
            current = cycle_selection(&on_cell, current);
            assert_eq!(current, Some(expected));
        }
    }

    #[test]
    fn single_occupant_stays_selected() {
        let on_cell = entities(1);
        assert_eq!(
            cycle_selection(&on_cell, Some(on_cell[0])),
            Some(on_cell[0])
        );
    }
}

/// Right click with a drafted (manual-mode) pawn selected orders it to walk
/// there. Auto-mode pawns ignore this — the Director drives them instead.
pub fn click_move(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    ui_nodes: Query<&Interaction>,
    selected: Res<SelectedEntity>,
    pawns: Query<(), (With<Pawn>, With<ManualMode>)>,
) {
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    if ui_nodes.iter().any(|i| *i != Interaction::None) {
        return;
    }
    let Some(entity) = selected.0 else {
        return;
    };
    if pawns.get(entity).is_err() {
        return;
    }
    let (camera, camera_transform) = *camera;
    let Some(goal) = cursor_to_cell(&window, camera, camera_transform) else {
        return;
    };
    commands.entity(entity).insert(MoveOrder { goal });
}
