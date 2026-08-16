use std::collections::HashSet;

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::cover::{self, CoverMap};
use crate::crops::Crop;
use crate::game::GameAssets;
use crate::items::{self, ItemKind, ItemStack};
use crate::map::{self, RoofMap, TerrainMap};
use crate::nav::NavGrid;
use crate::selection::Selectable;
use crate::weather::is_outdoors;

/// Request to harvest a plant — a bush or a crop, whichever entity `target`
/// is (the 3D game entity). Written by the AI Director when a pawn works a
/// harvest job (the pawn's tile, so the item lands beneath it).
#[derive(Message)]
pub struct HarvestCommand {
    pub target: Entity,
    /// Where to drop the item; `None` picks a free tile next to the plant
    /// (no current writer passes it, but the scan doubles as the overflow
    /// fallback when the drop tile's stack tops out).
    pub drop_at: Option<GridCoords>,
}

/// Data marker inserted by bevy_ecs_ldtk for each `Bush` entity instance
/// placed in the LDtk map.
#[derive(Default, Component)]
pub struct BushSpawn;

/// A bush grows from 0.0 to 1.0 over time; harvest yield is prorated by
/// growth. Harvest mechanics will mutate this on the 3D game entity (the
/// LDtk data entity only carries the initial growth).
#[derive(Default, Component, Clone, Copy)]
pub struct BerryBush {
    pub growth: f32,
}

impl BerryBush {
    /// Harvest yield, prorated by growth (`BUSH_MAX_YIELD` at 100%).
    pub fn yield_amount(&self) -> u32 {
        (self.growth * BUSH_MAX_YIELD as f32).round() as u32
    }

    pub fn is_harvestable(&self) -> bool {
        self.growth >= BUSH_HARVESTABLE_GROWTH
    }

    /// Wildfire fuel this bush contributes to its cell (`cover::fuel_at`).
    pub fn fuel(&self) -> f32 {
        self.growth * FUEL_BUSH
    }
}

/// Marker kept in sync with `BerryBush::is_harvestable` (>= 80% growth):
/// the bush *can* be harvested — ripe visual, Harvest button eligibility.
#[derive(Component)]
pub struct Harvestable;

/// Queued for harvesting: full-grown (inserted automatically at 100%) or
/// ordered early by the player via the Harvest button. This — not
/// `Harvestable` — is what generates harvest jobs, so pawns wait for the
/// optimal yield unless told otherwise.
#[derive(Component)]
pub struct ToHarvest;

fn berry_bush_from_field(entity_instance: &EntityInstance) -> BerryBush {
    let growth = *entity_instance.get_int_field("Growth").unwrap_or(&0);
    BerryBush {
        growth: growth.clamp(0, 100) as f32 / 100.0,
    }
}

#[derive(Default, Bundle, LdtkEntity)]
pub struct BushSpawnBundle {
    marker: BushSpawn,
    #[grid_coords]
    grid_coords: GridCoords,
    #[with(berry_bush_from_field)]
    berry_bush: BerryBush,
}

/// Visual scale of a bush sphere at the given growth.
fn bush_scale(growth: f32) -> f32 {
    BUSH_MIN_SCALE + (1.0 - BUSH_MIN_SCALE) * growth
}

/// Transform keeping the (scaled) sphere resting on the ground.
fn bush_transform(grid: &GridCoords, growth: f32) -> Transform {
    let scale = bush_scale(growth);
    Transform::from_translation(map::grid_to_world(grid) + Vec3::Y * BUSH_RADIUS * scale)
        .with_scale(Vec3::splat(scale))
}

pub fn spawn_bush_visual(
    mut commands: Commands,
    assets: Res<GameAssets>,
    spawns: Query<(&GridCoords, &BerryBush), Added<BushSpawn>>,
) {
    for (grid, berry_bush) in &spawns {
        let material = if berry_bush.is_harvestable() {
            assets.bush_ripe_material.clone()
        } else {
            assets.bush_material.clone()
        };
        let mut bush = commands.spawn((
            Mesh3d(assets.bush_mesh.clone()),
            MeshMaterial3d(material),
            bush_transform(grid, berry_bush.growth),
            Name::new("Berry bush"),
            *berry_bush,
            *grid,
            Selectable,
        ));
        if berry_bush.is_harvestable() {
            bush.insert(Harvestable);
        }
        if berry_bush.growth >= 1.0 {
            bush.insert(ToHarvest);
        }
    }
}

/// Consume `HarvestCommand`s: take the yield, then either reset the plant
/// (a bush regrows) or despawn it (a crop is consumed), and drop the item on
/// the nearest usable cell (`items::find_drop_cell`) — merging into started
/// same-kind stacks first, spilling outward when the surroundings are full.
#[allow(clippy::too_many_arguments)]
pub fn harvest_plants(
    mut commands: Commands,
    mut harvests: MessageReader<HarvestCommand>,
    assets: Res<GameAssets>,
    nav: Res<NavGrid>,
    mut bushes: Query<(
        &mut BerryBush,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
        &GridCoords,
    )>,
    crops: Query<(&Crop, &GridCoords)>,
    occupied: Query<&GridCoords, With<Selectable>>,
    mut stacks: Query<(&mut ItemStack, &GridCoords)>,
) {
    for harvest in harvests.read() {
        let (kind, amount, grid) = if let Ok((mut bush, mut transform, mut material, grid)) =
            bushes.get_mut(harvest.target)
        {
            if !bush.is_harvestable() {
                continue;
            }
            let amount = bush.yield_amount();
            bush.growth = 0.0;
            *transform = bush_transform(grid, bush.growth);
            material.0 = assets.bush_material.clone();
            commands
                .entity(harvest.target)
                .remove::<(Harvestable, ToHarvest)>();
            (ItemKind::Berries, amount, *grid)
        } else if let Ok((crop, grid)) = crops.get(harvest.target) {
            if !crop.is_harvestable() {
                continue;
            }
            // Crops are consumed, unlike bushes which regrow.
            commands.entity(harvest.target).despawn();
            (ItemKind::Wheat, crop.yield_amount(), *grid)
        } else {
            continue;
        };
        // Pour the yield onto the nearest usable cells, starting from the
        // harvester's tile (or the plant itself for the devmode button),
        // spilling outward as stacks fill.
        let origin = harvest.drop_at.unwrap_or(grid);
        let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
        items::pour_yield(
            &mut commands,
            &assets,
            kind,
            amount,
            origin,
            &mut stacks,
            &occupied_cells,
            |cell| nav.is_walkable(cell),
        );
    }
}

/// Grow every bush, scaled by its tile's fertility. Roofed cells don't grow
/// at all — wild growth needs sun.
#[allow(clippy::too_many_arguments)]
pub fn grow_bushes(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<GameAssets>,
    terrain: Res<TerrainMap>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    scorch: Res<crate::fire::ScorchMap>,
    roofs: Res<RoofMap>,
    mut bushes: Query<(
        Entity,
        &mut BerryBush,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
        &GridCoords,
    )>,
) {
    for (entity, mut bush, mut transform, mut material, grid) in &mut bushes {
        if bush.growth >= 1.0 || !is_outdoors(*grid, &roofs) {
            continue;
        }
        let was_harvestable = bush.is_harvestable();
        // Growth inherits the tile's effective fertility (substrate plus
        // grass-cover bonus), debuffed while the ground is scorched.
        let fertility = cover::fertility_at(*grid, &terrain, &cover, &water)
            * crate::fire::fertility_factor(scorch.get(*grid));
        bush.growth =
            (bush.growth + BUSH_GROWTH_PER_SECOND * fertility * time.delta_secs()).min(1.0);
        *transform = bush_transform(grid, bush.growth);
        if !was_harvestable && bush.is_harvestable() {
            material.0 = assets.bush_ripe_material.clone();
            commands.entity(entity).insert(Harvestable);
        }
        // Full-grown: queue it for harvesting (the early-out above makes
        // this fire exactly once per growth cycle).
        if bush.growth >= 1.0 {
            commands.entity(entity).insert(ToHarvest);
        }
    }
}
