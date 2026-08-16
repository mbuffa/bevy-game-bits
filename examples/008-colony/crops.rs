//! Planted crops. A crop is spawned directly by a Farming job (not seeded
//! from LDtk like bushes) on an empty cell of a Grow-enabled growing zone,
//! grows from 0.0 to 1.0 at a rate scaled by its tile's fertility (see
//! `map::Terrain::fertility`), and — unlike a berry bush, which regrows
//! after being picked — is consumed when harvested.

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::cover::{self, CoverMap};
use crate::flora::{Harvestable, ToHarvest};
use crate::game::GameAssets;
use crate::map::{self, TerrainMap};
use crate::selection::Selectable;

/// Crop species. Hardcoded to Wheat for now; more kinds later just add a
/// variant here plus arms in `label` and `growth_per_second` (and, if
/// yields/visuals differ, arms where those are computed).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CropKind {
    Wheat,
}

impl CropKind {
    pub fn label(&self) -> &'static str {
        match self {
            CropKind::Wheat => "Wheat",
        }
    }

    /// Growth gained per second at fertility 1.0 (scaled by tile fertility
    /// in `grow_crops`); each kind grows at its own pace.
    fn growth_per_second(&self) -> f32 {
        match self {
            CropKind::Wheat => WHEAT_GROWTH_PER_SECOND,
        }
    }
}

/// A planted crop, grown from 0.0 to 1.0. Only harvestable at 100% growth —
/// no prorated early harvest like bushes get at 80%, since a half-grown crop
/// isn't worth picking.
#[derive(Component, Clone, Copy)]
pub struct Crop {
    pub growth: f32,
    pub kind: CropKind,
}

impl Crop {
    pub fn yield_amount(&self) -> u32 {
        (self.growth * CROP_MAX_YIELD as f32).round() as u32
    }

    pub fn is_harvestable(&self) -> bool {
        self.growth >= 1.0
    }

    /// Wildfire fuel this crop contributes to its cell (`cover::fuel_at`).
    pub fn fuel(&self) -> f32 {
        self.growth * FUEL_CROP
    }
}

/// Visual scale of a crop stalk at the given growth (grows upward from the
/// ground, unlike a bush's uniform scale-up).
fn crop_scale(growth: f32) -> f32 {
    CROP_MIN_SCALE + (1.0 - CROP_MIN_SCALE) * growth
}

fn crop_transform(grid: &GridCoords, growth: f32) -> Transform {
    // The cluster mesh has its base at y=0, so it sits on the tile directly;
    // only the growth scale varies.
    Transform::from_translation(map::grid_to_world(grid)).with_scale(Vec3::new(
        1.0,
        crop_scale(growth),
        1.0,
    ))
}

/// The shared crop visual: a cluster of tall thin stalks merged into one
/// mesh (one draw, one entity). Each stalk is a stack of
/// `CROP_STALK_SEGMENTS` cuboids sharing boundary vertices, so the wind
/// vertex shader bends it into a curve instead of shearing a single box.
/// Base of every stalk is at y=0; the tallest reaches `CROP_HEIGHT`.
pub fn wheat_cluster_mesh() -> Mesh {
    // (XZ offset in units of CROP_STALK_SPREAD, height factor): hand-placed
    // so the cluster reads organic, with an uneven top line.
    const STALKS: [(Vec2, f32); CROP_STALK_COUNT] = [
        (Vec2::new(-0.6, -0.4), 1.0),
        (Vec2::new(0.5, -0.6), 0.85),
        (Vec2::new(-0.3, 0.6), 0.9),
        (Vec2::new(0.6, 0.5), 0.95),
    ];
    let mut mesh: Option<Mesh> = None;
    for (offset, height_factor) in STALKS {
        let segment_height = CROP_HEIGHT * height_factor / CROP_STALK_SEGMENTS as f32;
        for segment in 0..CROP_STALK_SEGMENTS {
            let piece = Mesh::from(Cuboid::new(
                CROP_STALK_SIZE,
                segment_height,
                CROP_STALK_SIZE,
            ))
            .translated_by(Vec3::new(
                offset.x * CROP_STALK_SPREAD,
                (segment as f32 + 0.5) * segment_height,
                offset.y * CROP_STALK_SPREAD,
            ));
            match &mut mesh {
                Some(mesh) => mesh.merge(&piece).unwrap(),
                None => mesh = Some(piece),
            }
        }
    }
    mesh.unwrap()
}

/// Plant a fresh (0% grown) crop at `cell`. Called by the Director when a
/// Farming job's pawn arrives at its target cell.
pub fn spawn_crop(commands: &mut Commands, assets: &GameAssets, grid: GridCoords) {
    commands.spawn((
        Mesh3d(assets.crop_mesh.clone()),
        MeshMaterial3d(assets.crop_material.clone()),
        crop_transform(&grid, 0.0),
        Name::new("Wheat"),
        Crop {
            growth: 0.0,
            kind: CropKind::Wheat,
        },
        grid,
        Selectable,
    ));
}

/// Grow every crop, scaling by its tile's fertility; queue it for harvest
/// (via the shared `flora::ToHarvest`) the moment it hits 100%.
pub fn grow_crops(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<TerrainMap>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    scorch: Res<crate::fire::ScorchMap>,
    mut crops: Query<(Entity, &mut Crop, &mut Transform, &GridCoords)>,
) {
    for (entity, mut crop, mut transform, grid) in &mut crops {
        if crop.growth >= 1.0 {
            continue;
        }
        let fertility = cover::fertility_at(*grid, &terrain, &cover, &water)
            * crate::fire::fertility_factor(scorch.get(*grid));
        crop.growth =
            (crop.growth + crop.kind.growth_per_second() * fertility * time.delta_secs()).min(1.0);
        *transform = crop_transform(grid, crop.growth);
        if crop.growth >= 1.0 {
            // Bushes distinguish `Harvestable` (>= 80%) from `ToHarvest`
            // (queued); a crop only has the one threshold, so both land
            // together.
            commands.entity(entity).insert((Harvestable, ToHarvest));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn cluster_positions() -> Vec<[f32; 3]> {
        let mesh = wheat_cluster_mesh();
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            VertexAttributeValues::Float32x3(positions) => positions.clone(),
            other => panic!("unexpected position format: {other:?}"),
        }
    }

    #[test]
    fn cluster_sits_on_the_ground_and_tops_out_at_crop_height() {
        let ys: Vec<f32> = cluster_positions().iter().map(|p| p[1]).collect();
        let min = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let max = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(min.abs() < 1e-6, "stalk bases should sit at y=0, got {min}");
        assert!(
            (max - CROP_HEIGHT).abs() < 1e-6,
            "tallest stalk should reach CROP_HEIGHT, got {max}"
        );
    }

    #[test]
    fn cluster_merges_every_stalk_segment() {
        // 24 vertices per cuboid (4 per face, unshared for flat normals).
        assert_eq!(
            cluster_positions().len(),
            CROP_STALK_COUNT * CROP_STALK_SEGMENTS * 24
        );
    }

    #[test]
    fn cluster_stays_inside_the_stalk_spread() {
        let reach = CROP_STALK_SPREAD + CROP_STALK_SIZE / 2.0;
        for p in cluster_positions() {
            assert!(p[0].abs() <= reach + 1e-6);
            assert!(p[2].abs() <= reach + 1e-6);
        }
    }
}
