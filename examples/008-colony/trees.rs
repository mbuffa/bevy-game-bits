//! Trees: a living forest. LDtk seeds a few mature trees on the grass
//! pocket; mature trees periodically seed 0-growth saplings on nearby free
//! grass (`can_root`), so the forest fills its grass bounds on its own.
//! Trees are walkable but slow (`nav::mark_trees` raises the cell cost).

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use std::collections::HashSet;

use crate::config::*;
use crate::cover::{self, CoverMap, FloraKind};
use crate::crops::Crop;
use crate::director::WanderRng;
use crate::fire::{burn_kills_plants, BurnCommand, FireMap};
use crate::flora::BerryBush;
use crate::game::GameAssets;
use crate::items::{self, ItemKind, ItemStack};
use crate::map::{self, RoofMap, TerrainMap};
use crate::nav::NavGrid;
use crate::selection::Selectable;
use crate::shrubs::Shrub;
use crate::weather::is_outdoors;

/// Data marker inserted by bevy_ecs_ldtk for each `Tree` entity instance.
#[derive(Default, Component)]
pub struct TreeSpawn;

#[derive(Default, Bundle, LdtkEntity)]
pub struct TreeSpawnBundle {
    marker: TreeSpawn,
    #[grid_coords]
    grid_coords: GridCoords,
}

/// A tree, growing from 0.0 (sapling) to 1.0 (mature, and spreading). A
/// wildfire that burns its cell down chars it into a snag (`burnt`): it
/// stops growing and spreading, holds no fuel or wood, but stays cuttable
/// so the player can order it cleared.
#[derive(Component)]
pub struct Tree {
    pub growth: f32,
    pub burnt: bool,
}

impl Tree {
    /// Worth the axe? Deliberately NOT the bush/crop harvest path — trees
    /// are forestry, not farming — but the same shape of threshold. Snags
    /// are always cuttable: clearing them is the point.
    pub fn is_cuttable(&self) -> bool {
        self.burnt || self.growth >= TREE_CUTTABLE_GROWTH
    }

    /// Wood a felled tree drops, prorated by growth (`TREE_WOOD_YIELD` at
    /// 100%; never marked below `TREE_CUTTABLE_GROWTH`, so no sapling
    /// culls). A snag yields nothing — no burning the forest for wood.
    pub fn wood_yield(&self) -> u32 {
        if self.burnt {
            return 0;
        }
        (self.growth * TREE_WOOD_YIELD as f32).round() as u32
    }

    /// Wildfire fuel this tree contributes to its cell (`cover::fuel_at`).
    /// A snag is spent fuel.
    pub fn fuel(&self) -> f32 {
        if self.burnt {
            return 0.0;
        }
        self.growth * FUEL_TREE
    }
}

/// Queued for felling, the forestry mirror of `flora::ToHarvest`: ordered by
/// the player via the Cut button (never automatically — a mature forest is
/// not a work generator). This is what generates Cut jobs.
#[derive(Component)]
pub struct ToCut;

/// Request to fell a tree, written by the Director when a Cut job's work
/// timer completes with the pawn standing next to it.
#[derive(Message)]
pub struct CutCommand {
    pub target: Entity,
    /// The feller's tile: the wood lands beneath the pawn, spilling outward.
    pub drop_at: GridCoords,
}

/// Countdown to a mature tree's next attempt to seed a sapling.
#[derive(Component)]
pub struct SpreadTimer(pub Timer);

fn tree_scale(growth: f32) -> f32 {
    TREE_MIN_SCALE + (1.0 - TREE_MIN_SCALE) * growth
}

fn tree_transform(grid: &GridCoords, growth: f32) -> Transform {
    Transform::from_translation(map::grid_to_world(grid))
        .with_scale(Vec3::splat(tree_scale(growth)))
}

/// Can a sapling take root on `cell`? Only where grass cover has thickened
/// past `TREE_ROOT_MIN_COVER` (the forest follows the grass), standable,
/// and not already holding a plant.
fn can_root(
    cell: GridCoords,
    cover: Option<(FloraKind, f32)>,
    occupied: &impl Fn(GridCoords) -> bool,
    standable: &impl Fn(GridCoords) -> bool,
) -> bool {
    matches!(cover, Some((FloraKind::Grass, coverage)) if coverage >= TREE_ROOT_MIN_COVER)
        && standable(cell)
        && !occupied(cell)
}

/// Plant a tree at `cell`: trunk with a canopy riding on top, the whole
/// thing scaled by growth. LDtk seed trees pass 1.0, saplings 0.0.
pub fn spawn_tree(commands: &mut Commands, assets: &GameAssets, grid: GridCoords, growth: f32) {
    commands
        .spawn((
            Mesh3d(assets.tree_trunk_mesh.clone()),
            MeshMaterial3d(assets.tree_trunk_material.clone()),
            tree_transform(&grid, growth),
            Name::new("Tree"),
            Tree {
                growth,
                burnt: false,
            },
            grid,
            Selectable,
        ))
        .with_child((
            Mesh3d(assets.tree_canopy_mesh.clone()),
            MeshMaterial3d(assets.tree_canopy_material.clone()),
            Transform::from_translation(Vec3::Y * TREE_TRUNK_HEIGHT),
        ));
}

/// Bridge: LDtk `Tree` data entities become mature 3D trees.
pub fn spawn_tree_visual(
    mut commands: Commands,
    assets: Res<GameAssets>,
    spawns: Query<&GridCoords, Added<TreeSpawn>>,
) {
    for grid in &spawns {
        spawn_tree(&mut commands, &assets, *grid, 1.0);
    }
}

/// Grow every tree, scaled by its tile's fertility; a tree reaching
/// maturity starts its spread timer (staggered so seed trees don't all
/// fire in sync). Roofed cells don't grow at all — wild growth needs sun.
#[allow(clippy::too_many_arguments)]
pub fn grow_trees(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<TerrainMap>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    scorch: Res<crate::fire::ScorchMap>,
    roofs: Res<RoofMap>,
    mut rng: ResMut<WanderRng>,
    mut trees: Query<(Entity, &mut Tree, &mut Transform, &GridCoords)>,
) {
    for (entity, mut tree, mut transform, grid) in &mut trees {
        // A snag neither grows nor (via the maturity arm) starts spreading.
        if tree.burnt || tree.growth >= 1.0 || !is_outdoors(*grid, &roofs) {
            continue;
        }
        let fertility = cover::fertility_at(*grid, &terrain, &cover, &water)
            * crate::fire::fertility_factor(scorch.get(*grid));
        tree.growth =
            (tree.growth + TREE_GROWTH_PER_SECOND * fertility * time.delta_secs()).min(1.0);
        *transform = tree_transform(grid, tree.growth);
        if tree.growth >= 1.0 {
            let offset = rng.range(0, TREE_SPREAD_SECS as i32) as f32;
            commands
                .entity(entity)
                .insert(SpreadTimer(Timer::from_seconds(
                    TREE_SPREAD_SECS + offset,
                    TimerMode::Once,
                )));
        }
    }
}

/// Give LDtk-seeded (already mature) trees their spread timer too.
#[allow(clippy::type_complexity)]
pub fn arm_seed_trees(
    mut commands: Commands,
    mut rng: ResMut<WanderRng>,
    unarmed: Query<(Entity, &Tree), (Added<Tree>, Without<SpreadTimer>)>,
) {
    for (entity, tree) in &unarmed {
        if tree.growth >= 1.0 {
            let offset = rng.range(0, TREE_SPREAD_SECS as i32) as f32;
            commands
                .entity(entity)
                .insert(SpreadTimer(Timer::from_seconds(
                    TREE_SPREAD_SECS + offset,
                    TimerMode::Once,
                )));
        }
    }
}

/// Mature trees seed saplings: on timer expiry, roll one candidate cell
/// within `TREE_SPREAD_RADIUS` and plant if it can root; either way the
/// timer restarts (a failed roll just means "try again later"). A roofed
/// candidate never roots — the wild forest doesn't creep in under a roof.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn spread_trees(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<GameAssets>,
    cover: Res<CoverMap>,
    nav: Res<crate::nav::NavGrid>,
    roofs: Res<RoofMap>,
    mut rng: ResMut<WanderRng>,
    mut spreaders: Query<(&GridCoords, &mut SpreadTimer), With<Tree>>,
    plants: Query<&GridCoords, Or<(With<Tree>, With<BerryBush>, With<Crop>, With<Shrub>)>>,
) {
    // Includes this frame's earlier spawns? No — commands are deferred, so
    // two trees could seed the same cell in one frame; the local set closes
    // that hole.
    let mut taken: Vec<GridCoords> = Vec::new();
    for (grid, mut timer) in &mut spreaders {
        if !timer.0.tick(time.delta()).is_finished() {
            continue;
        }
        timer.0 = Timer::from_seconds(TREE_SPREAD_SECS, TimerMode::Once);
        let candidate = GridCoords::new(
            grid.x + rng.range(-TREE_SPREAD_RADIUS, TREE_SPREAD_RADIUS),
            grid.y + rng.range(-TREE_SPREAD_RADIUS, TREE_SPREAD_RADIUS),
        );
        let occupied =
            |cell: GridCoords| taken.contains(&cell) || plants.iter().any(|plant| *plant == cell);
        let standable = |cell: GridCoords| nav.is_walkable(cell);
        if is_outdoors(candidate, &roofs)
            && can_root(candidate, cover.get(candidate), &occupied, &standable)
        {
            info!("trees: sapling takes root at {candidate:?}");
            spawn_tree(&mut commands, &assets, candidate, 0.0);
            taken.push(candidate);
        }
    }
}

/// Deliberate planting by a Farming job on a growing zone set to Trees.
/// Unlike wild spreading (`can_root`), a farmer doesn't need grass cover —
/// just an empty cell, which the caller (the Director) has checked.
pub fn plant_sapling(commands: &mut Commands, assets: &GameAssets, cell: GridCoords) {
    spawn_tree(commands, assets, cell, 0.0);
}

/// Fell trees ordered by `CutCommand`: the tree despawns, its cell's nav
/// cost reverts to the bare terrain (the standing tree had raised it — see
/// `nav::mark_trees`), and the wood pours out at the feller's tile.
#[allow(clippy::too_many_arguments)]
pub fn fell_trees(
    mut commands: Commands,
    mut cuts: MessageReader<CutCommand>,
    assets: Res<GameAssets>,
    terrain: Res<TerrainMap>,
    mut nav: ResMut<NavGrid>,
    trees: Query<(&Tree, &GridCoords), With<ToCut>>,
    occupied: Query<&GridCoords, With<Selectable>>,
    mut stacks: Query<(&mut ItemStack, &GridCoords)>,
) {
    for cut in cuts.read() {
        // A canceled or already-felled target: the command is stale, skip.
        let Ok((tree, grid)) = trees.get(cut.target) else {
            continue;
        };
        let amount = tree.wood_yield();
        info!(
            "trees: felled {:?} at {grid:?} for {amount} wood",
            cut.target
        );
        commands.entity(cut.target).despawn();
        // Always the dirt baseline in practice: trees never stand on
        // riverbeds (grass, their rooting substrate, needs dry ground), and
        // if one ever did, `nav::sync_water_costs` re-stamps every bed
        // cell's water cost on the next level change anyway.
        let cost = terrain
            .get(*grid)
            .map_or(TERRAIN_COST_DIRT, |kind| kind.base_cost());
        nav.set_cost(*grid, cost);
        let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
        items::pour_yield(
            &mut commands,
            &assets,
            ItemKind::Wood,
            amount,
            cut.drop_at,
            &mut stacks,
            &occupied_cells,
            |cell| nav.is_walkable(cell),
        );
    }
}

/// Char trees on cells whose fire ended hot enough (`BurnCommand`,
/// `burn_kills_plants`): the canopy burns away (the child despawns),
/// growth and spreading stop, and the snag stands — still occupying its
/// cell against re-planting, still slow to walk through — until the
/// player orders it cleared (Cut, for no wood). Ungated: the
/// Extinguish-all devtool writes BurnCommands while paused.
pub fn burn_trees(
    mut commands: Commands,
    mut burns: MessageReader<BurnCommand>,
    mut trees: Query<(Entity, &mut Tree, &GridCoords, Option<&Children>)>,
) {
    for burn in burns.read() {
        if !burn_kills_plants(burn.completeness) {
            continue;
        }
        for (entity, mut tree, grid, children) in &mut trees {
            if *grid != burn.cell || tree.burnt {
                continue;
            }
            info!("trees: charred to a snag at {grid:?}");
            tree.burnt = true;
            commands.entity(entity).remove::<SpreadTimer>();
            for child in children.into_iter().flatten() {
                commands.entity(*child).despawn();
            }
        }
    }
}

/// Reconcile tree materials with the fire underneath: an alive tree on a
/// burning cell gets an ember-glow canopy, a saved one turns green again,
/// and a snag keeps its charred trunk. Handles are compared before
/// assigning, so calm frames are no-ops (shared-material batching stays
/// intact).
pub fn sync_tree_materials(
    fire: Res<FireMap>,
    assets: Res<GameAssets>,
    trees: Query<(Entity, &Tree, &GridCoords, Option<&Children>)>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    let mut apply = |entity: Entity, handle: &Handle<StandardMaterial>| {
        if let Ok(mut material) = materials.get_mut(entity) {
            if material.0 != *handle {
                material.0 = handle.clone();
            }
        }
    };
    for (entity, tree, grid, children) in &trees {
        let trunk = if tree.burnt {
            &assets.tree_trunk_charred_material
        } else {
            &assets.tree_trunk_material
        };
        apply(entity, trunk);
        let canopy = if !tree.burnt && fire.is_burning(*grid) {
            &assets.tree_canopy_burning_material
        } else {
            &assets.tree_canopy_material
        };
        for child in children.into_iter().flatten() {
            apply(*child, canopy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free(_: GridCoords) -> bool {
        false
    }
    fn walkable(_: GridCoords) -> bool {
        true
    }
    fn alive(growth: f32) -> Tree {
        Tree {
            growth,
            burnt: false,
        }
    }

    #[test]
    fn saplings_root_only_on_thick_grass() {
        let cell = GridCoords::new(3, 3);
        assert!(can_root(
            cell,
            Some((FloraKind::Grass, 1.0)),
            &free,
            &walkable
        ));
        assert!(can_root(
            cell,
            Some((FloraKind::Grass, TREE_ROOT_MIN_COVER)),
            &free,
            &walkable
        ));
        // Too-thin grass, algae, or bare ground: no rooting.
        assert!(!can_root(
            cell,
            Some((FloraKind::Grass, TREE_ROOT_MIN_COVER - 0.01)),
            &free,
            &walkable
        ));
        assert!(!can_root(
            cell,
            Some((FloraKind::Algae, 1.0)),
            &free,
            &walkable
        ));
        assert!(!can_root(cell, None, &free, &walkable));
    }

    #[test]
    fn occupied_or_unstandable_cells_reject_saplings() {
        let cell = GridCoords::new(3, 3);
        let grass = Some((FloraKind::Grass, 1.0));
        let occupied = |c: GridCoords| c == cell;
        assert!(!can_root(cell, grass, &occupied, &walkable));
        let blocked = |_: GridCoords| false;
        assert!(!can_root(cell, grass, &free, &blocked));
    }

    #[test]
    fn cuttable_from_half_growth_on() {
        assert!(!alive(0.0).is_cuttable());
        assert!(!alive(TREE_CUTTABLE_GROWTH - 0.01).is_cuttable());
        assert!(alive(TREE_CUTTABLE_GROWTH).is_cuttable());
        assert!(alive(1.0).is_cuttable());
    }

    #[test]
    fn wood_yield_prorated_by_growth() {
        assert_eq!(alive(1.0).wood_yield(), TREE_WOOD_YIELD);
        // Half growth rounds to the nearest whole log (25 * 0.5 -> 13).
        assert_eq!(alive(0.5).wood_yield(), 13);
        assert_eq!(alive(0.8).wood_yield(), 20);
    }

    #[test]
    fn burnt_trees_yield_nothing_but_stay_cuttable() {
        let snag = Tree {
            growth: 0.3, // below TREE_CUTTABLE_GROWTH: even burnt saplings clear
            burnt: true,
        };
        assert_eq!(snag.wood_yield(), 0);
        assert_eq!(snag.fuel(), 0.0);
        assert!(snag.is_cuttable());
    }
}
