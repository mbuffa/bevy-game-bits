//! Shrubs: decorative standing fuel. A shrub grows like every other plant
//! (fertility-scaled), sparsely seeds neighbors on grassy ground, and is
//! never harvestable — no jobs, no yield, no nav effect. It exists to make
//! meadows scrubby and, once wildfires land, to burn. Spread is
//! self-limiting (a chance roll plus a local density cap), so patches stay
//! sparse instead of blanketing the map.

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::cover::{self, CoverMap, FloraKind};
use crate::crops::Crop;
use crate::director::WanderRng;
use crate::fire::{burn_kills_plants, BurnCommand, FireMap};
use crate::flora::{BerryBush, BushSpawn};
use crate::game::GameAssets;
use crate::map::{self, GrassCell, RoofMap, TerrainMap};
use crate::nav::NavGrid;
use crate::selection::Selectable;
use crate::weather::is_outdoors;
use crate::trees::{Tree, TreeSpawn};

/// A shrub, growing from 0.0 to 1.0. That's all it does — deliberately no
/// harvest thresholds or yields.
#[derive(Component)]
pub struct Shrub {
    pub growth: f32,
}

impl Shrub {
    /// Wildfire fuel this shrub contributes to its cell (`cover::fuel_at`).
    pub fn fuel(&self) -> f32 {
        self.growth * FUEL_SHRUB
    }
}

/// Countdown to a mature shrub's next attempt to seed a neighbor.
#[derive(Component)]
pub struct ShrubSpreadTimer(pub Timer);

fn shrub_scale(growth: f32) -> f32 {
    SHRUB_MIN_SCALE + (1.0 - SHRUB_MIN_SCALE) * growth
}

/// The dome mesh is a Y-squashed sphere centered on its origin, so lifting
/// by the squashed radius rests it on the ground.
fn shrub_transform(grid: &GridCoords, growth: f32) -> Transform {
    let scale = shrub_scale(growth);
    Transform::from_translation(
        map::grid_to_world(grid) + Vec3::Y * SHRUB_RADIUS * SHRUB_FLATTEN * scale,
    )
    .with_scale(Vec3::splat(scale))
}

/// Can a shrub take root on `cell`? Thinner grass than trees demand
/// (`SHRUB_ROOT_MIN_COVER` vs `TREE_ROOT_MIN_COVER`) — hardy scrub — but
/// the same standable + unoccupied rules.
fn can_root(
    cell: GridCoords,
    cover: Option<(FloraKind, f32)>,
    occupied: &impl Fn(GridCoords) -> bool,
    standable: &impl Fn(GridCoords) -> bool,
) -> bool {
    matches!(cover, Some((FloraKind::Grass, coverage)) if coverage >= SHRUB_ROOT_MIN_COVER)
        && standable(cell)
        && !occupied(cell)
}

/// Density cap: `candidate` is too crowded to root if `SHRUB_MAX_NEIGHBORS`
/// or more shrubs already stand within `SHRUB_NEIGHBOR_RADIUS` (Chebyshev
/// distance) of it. This is what keeps spread sparse instead of
/// exponential — without it, every mature shrub eventually seeds enough
/// children to blanket the map.
fn is_crowded(candidate: GridCoords, shrubs: &[GridCoords]) -> bool {
    shrubs
        .iter()
        .filter(|c| {
            (c.x - candidate.x).abs() <= SHRUB_NEIGHBOR_RADIUS
                && (c.y - candidate.y).abs() <= SHRUB_NEIGHBOR_RADIUS
        })
        .count()
        >= SHRUB_MAX_NEIGHBORS
}

pub fn spawn_shrub(commands: &mut Commands, assets: &GameAssets, grid: GridCoords, growth: f32) {
    commands.spawn((
        Mesh3d(assets.shrub_mesh.clone()),
        MeshMaterial3d(assets.shrub_material.clone()),
        shrub_transform(&grid, growth),
        Name::new("Shrub"),
        Shrub { growth },
        grid,
        Selectable,
    ));
}

/// Scatter the world-start shrubs onto LDtk grass cells once those exist
/// (the `seed_algae` one-shot model). Occupancy is checked against the
/// LDtk DATA entities (`TreeSpawn`/`BushSpawn`), which arrive in the same
/// batch as `GrassCell` — the 3D `Tree`/`BerryBush` entities spawn a
/// deferred frame later and would race.
#[allow(clippy::type_complexity)]
pub fn seed_shrubs(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut rng: ResMut<WanderRng>,
    mut done: Local<bool>,
    grass: Query<&GridCoords, With<GrassCell>>,
    plant_spawns: Query<&GridCoords, Or<(With<TreeSpawn>, With<BushSpawn>)>>,
) {
    if *done {
        return;
    }
    let cells: Vec<&GridCoords> = grass.iter().collect();
    if cells.is_empty() {
        return;
    }
    *done = true;
    let mut taken: Vec<GridCoords> = Vec::new();
    for _ in 0..SHRUB_SEED_COUNT {
        let cell = *cells[rng.range(0, cells.len() as i32 - 1) as usize];
        if taken.contains(&cell) || plant_spawns.iter().any(|grid| *grid == cell) {
            continue; // a failed roll just means one shrub fewer
        }
        spawn_shrub(&mut commands, &assets, cell, SHRUB_SEED_GROWTH);
        taken.push(cell);
    }
}

/// Grow every shrub, scaled by its tile's fertility; maturity arms the
/// (staggered) spread timer. Roofed cells don't grow at all — wild growth
/// needs sun.
#[allow(clippy::too_many_arguments)]
pub fn grow_shrubs(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<TerrainMap>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    scorch: Res<crate::fire::ScorchMap>,
    roofs: Res<RoofMap>,
    mut rng: ResMut<WanderRng>,
    mut shrubs: Query<(Entity, &mut Shrub, &mut Transform, &GridCoords)>,
) {
    for (entity, mut shrub, mut transform, grid) in &mut shrubs {
        if shrub.growth >= 1.0 || !is_outdoors(*grid, &roofs) {
            continue;
        }
        let fertility = cover::fertility_at(*grid, &terrain, &cover, &water)
            * crate::fire::fertility_factor(scorch.get(*grid));
        shrub.growth =
            (shrub.growth + SHRUB_GROWTH_PER_SECOND * fertility * time.delta_secs()).min(1.0);
        *transform = shrub_transform(grid, shrub.growth);
        if shrub.growth >= 1.0 {
            let offset = rng.range(0, SHRUB_SPREAD_SECS as i32) as f32;
            commands
                .entity(entity)
                .insert(ShrubSpreadTimer(Timer::from_seconds(
                    SHRUB_SPREAD_SECS + offset,
                    TimerMode::Once,
                )));
        }
    }
}

/// Mature shrubs seed neighbors: on timer expiry, roll a `SHRUB_SPREAD_CHANCE`
/// chance to even attempt it, then pick one candidate within
/// `SHRUB_SPREAD_RADIUS` and plant if it can root *and* isn't too crowded
/// (`is_crowded`); either way the timer restarts (the `spread_trees` model,
/// plus the chance roll and density cap that keep shrubs sparse). A roofed
/// candidate never roots — the wild scrub doesn't creep in under a roof.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn spread_shrubs(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<GameAssets>,
    cover: Res<CoverMap>,
    nav: Res<NavGrid>,
    roofs: Res<RoofMap>,
    mut rng: ResMut<WanderRng>,
    mut spreaders: Query<(&GridCoords, &mut ShrubSpreadTimer), With<Shrub>>,
    shrub_cells: Query<&GridCoords, With<Shrub>>,
    plants: Query<&GridCoords, Or<(With<Tree>, With<BerryBush>, With<Crop>, With<Shrub>)>>,
) {
    let existing: Vec<GridCoords> = shrub_cells.iter().copied().collect();
    // Commands are deferred, so two shrubs could seed the same cell (or
    // over-crowd the same neighborhood) in one frame; the local set closes
    // that hole for both occupancy and the density cap.
    let mut taken: Vec<GridCoords> = Vec::new();
    for (grid, mut timer) in &mut spreaders {
        if !timer.0.tick(time.delta()).is_finished() {
            continue;
        }
        timer.0 = Timer::from_seconds(SHRUB_SPREAD_SECS, TimerMode::Once);
        if !rng.chance(SHRUB_SPREAD_CHANCE) {
            continue;
        }
        let candidate = GridCoords::new(
            grid.x + rng.range(-SHRUB_SPREAD_RADIUS, SHRUB_SPREAD_RADIUS),
            grid.y + rng.range(-SHRUB_SPREAD_RADIUS, SHRUB_SPREAD_RADIUS),
        );
        let occupied =
            |cell: GridCoords| taken.contains(&cell) || plants.iter().any(|plant| *plant == cell);
        let standable = |cell: GridCoords| nav.is_walkable(cell);
        let nearby: Vec<GridCoords> = existing.iter().chain(taken.iter()).copied().collect();
        if is_outdoors(candidate, &roofs)
            && can_root(candidate, cover.get(candidate), &occupied, &standable)
            && !is_crowded(candidate, &nearby)
        {
            info!("shrubs: scrub takes root at {candidate:?}");
            spawn_shrub(&mut commands, &assets, candidate, 0.0);
            taken.push(candidate);
        }
    }
}

/// Shrubs on cells whose fire ended hot enough (`BurnCommand`,
/// `burn_kills_plants`) burn away entirely — the ground's scorch mark is
/// all that remains. Ungated: the Extinguish-all devtool writes
/// BurnCommands while paused.
pub fn burn_shrubs(
    mut commands: Commands,
    mut burns: MessageReader<BurnCommand>,
    shrubs: Query<(Entity, &GridCoords), With<Shrub>>,
) {
    for burn in burns.read() {
        if !burn_kills_plants(burn.completeness) {
            continue;
        }
        for (entity, grid) in &shrubs {
            if *grid == burn.cell {
                info!("shrubs: burnt away at {grid:?}");
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Reconcile shrub materials with the fire underneath: ember glow while
/// the cell burns, green again if the shrub survives a douse. Handles are
/// compared before assigning, so calm frames are no-ops.
pub fn sync_shrub_materials(
    fire: Res<FireMap>,
    assets: Res<GameAssets>,
    mut shrubs: Query<(&GridCoords, &mut MeshMaterial3d<StandardMaterial>), With<Shrub>>,
) {
    for (grid, mut material) in &mut shrubs {
        let desired = if fire.is_burning(*grid) {
            &assets.shrub_burning_material
        } else {
            &assets.shrub_material
        };
        if material.0 != *desired {
            material.0 = desired.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrubs_root_on_thinner_grass_than_trees() {
        let cell = GridCoords::new(5, 5);
        let free = |_: GridCoords| false;
        let standable = |_: GridCoords| true;
        // Right at the shrub threshold, below the tree one.
        assert!(SHRUB_ROOT_MIN_COVER < TREE_ROOT_MIN_COVER);
        assert!(can_root(
            cell,
            Some((FloraKind::Grass, SHRUB_ROOT_MIN_COVER)),
            &free,
            &standable
        ));
        assert!(!can_root(
            cell,
            Some((FloraKind::Grass, SHRUB_ROOT_MIN_COVER - 0.01)),
            &free,
            &standable
        ));
        // Algae and bare ground never root shrubs.
        assert!(!can_root(
            cell,
            Some((FloraKind::Algae, 1.0)),
            &free,
            &standable
        ));
        assert!(!can_root(cell, None, &free, &standable));
    }

    #[test]
    fn occupied_or_unstandable_cells_reject_shrubs() {
        let cell = GridCoords::new(5, 5);
        let grass = Some((FloraKind::Grass, 1.0));
        assert!(!can_root(cell, grass, &|c| c == cell, &|_| true));
        assert!(!can_root(cell, grass, &|_| false, &|_| false));
    }

    #[test]
    fn density_cap_blocks_crowded_candidates() {
        let candidate = GridCoords::new(5, 5);
        // Below the cap: fewer than SHRUB_MAX_NEIGHBORS shrubs in range.
        let sparse: Vec<GridCoords> = (0..SHRUB_MAX_NEIGHBORS - 1)
            .map(|i| GridCoords::new(5 + i as i32, 5))
            .collect();
        assert!(!is_crowded(candidate, &sparse));

        // At the cap: SHRUB_MAX_NEIGHBORS shrubs within SHRUB_NEIGHBOR_RADIUS.
        let crowded: Vec<GridCoords> = (0..SHRUB_MAX_NEIGHBORS)
            .map(|i| GridCoords::new(5 + i as i32, 5))
            .collect();
        assert!(is_crowded(candidate, &crowded));

        // Shrubs outside the radius don't count, no matter how many.
        let far = vec![GridCoords::new(5 + SHRUB_NEIGHBOR_RADIUS + 1, 5); SHRUB_MAX_NEIGHBORS + 5];
        assert!(!is_crowded(candidate, &far));
    }

    #[test]
    fn shrub_fuel_prorated_by_growth() {
        assert_eq!(Shrub { growth: 0.0 }.fuel(), 0.0);
        assert!((Shrub { growth: 0.5 }.fuel() - FUEL_SHRUB * 0.5).abs() < 1e-6);
        assert!((Shrub { growth: 1.0 }.fuel() - FUEL_SHRUB).abs() < 1e-6);
    }
}
