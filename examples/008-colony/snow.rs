//! Snow and ice: the cold half of the weather system. When the ambient
//! drops to freezing, precipitation falls as snow (`weather::precipitation_form`)
//! and accumulates per-cell in the `SnowMap` — but never on roofed cells,
//! including free-standing player roofs. Warmth melts it back: the melt rate
//! is proportional to the cell's °C above zero, so the sun's daily
//! temperature hump IS the radiance coupling (melt peaks at noon), and
//! melting cells refill the `HumidityMap` (see `weather::update_humidity`).
//!
//! Water bodies freeze on a gradient tracked by the global `FreezeLevel`:
//! it climbs whenever the ambient is sub-zero (faster the colder — one
//! -20 °C night, `FREEZE_FULL_BELOW_C`, freezes a river solid) and thaws
//! only above 0 °C, per-degree. Fully frozen water is walkable ice
//! (`nav::sync_water_costs`) that carries snow cover; snow depth adds a
//! bucketed nav penalty so pawns trudge through drifts and paths route
//! around them.

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::construction::{BuildableKind, ConstructionMap};
use crate::daynight::GameClock;
use crate::game::GameAssets;
use crate::map::{self, RoofMap};
use crate::nav::NavGrid;
use crate::temperature::{ambient_temperature, ActiveSeason, TemperatureMap};
use crate::weather::{is_outdoors, precipitation_form, Precipitation, Weather};

/// Per-cell snow cover 0..=1 — the `TerrainMap`/`HumidityMap` flat-grid
/// model.
#[derive(Resource)]
pub struct SnowMap {
    levels: Vec<f32>,
}

impl Default for SnowMap {
    fn default() -> Self {
        Self {
            levels: vec![0.0; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl SnowMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<f32> {
        Self::in_bounds(cell).then(|| self.levels[(cell.y * MAP_WIDTH + cell.x) as usize])
    }

    /// Direct write, clamped 0..=1 — test-only; the sim itself only ever
    /// advances a level through `next_snow`.
    #[cfg(test)]
    pub fn set(&mut self, cell: GridCoords, level: f32) {
        if Self::in_bounds(cell) {
            self.levels[(cell.y * MAP_WIDTH + cell.x) as usize] = level.clamp(0.0, 1.0);
        }
    }
}

/// How frozen the map's water bodies are, 0..=1. One global scalar, not a
/// grid: open water sits under the open sky, so the ambient drives it all
/// alike (an indoor water body is an accepted edge case).
#[derive(Resource, Default)]
pub struct FreezeLevel(pub f32);

impl FreezeLevel {
    /// Frozen hard enough to carry a pawn: bed cells cost like dirt and
    /// waders stop dipping.
    pub fn is_frozen(&self) -> bool {
        self.0 >= ICE_WALKABLE_MIN_FREEZE
    }
}

/// One freeze step. Ice below 0 °C never melts, so freezing is monotone:
/// while the ambient is sub-zero the level only climbs, at a rate scaled
/// by how far below zero it sits — full speed at `FREEZE_FULL_BELOW_C`
/// (one -20 °C night freezes a river solid: the guarantee), a crawl just
/// under 0. Thaw runs only above 0 °C, proportional to the degrees (the
/// snow-melt shape). A drift-toward-target model was tried first and
/// rejected: the winter noon (-4 °C → a low target) dragged the level
/// back down every day and it never reached the walkable threshold.
pub fn next_freeze(current: f32, ambient_c: f32, dt: f32) -> f32 {
    if ambient_c <= 0.0 {
        let cold = (ambient_c / FREEZE_FULL_BELOW_C).clamp(0.0, 1.0);
        (current + FREEZE_PER_SECOND_AT_FULL_COLD * cold * dt).min(1.0)
    } else {
        (current - ICE_THAW_PER_DEGREE_SECOND * ambient_c * dt).max(0.0)
    }
}

/// One snow step for one cell.
/// - A cell that can't hold snow (liquid, unfrozen water) sheds it
///   instantly — flakes vanish into the river.
/// - While snowed on, cover builds at the accumulation rate.
/// - Otherwise it melts in proportion to the cell's °C above zero (nothing
///   happens below freezing: snow keeps under a clear winter sky).
pub fn next_snow(level: f32, snowing: bool, holds_snow: bool, cell_temp_c: f32, dt: f32) -> f32 {
    if !holds_snow {
        return 0.0;
    }
    if snowing {
        (level + SNOW_ACCUMULATE_PER_SECOND * dt).min(1.0)
    } else if cell_temp_c > 0.0 {
        (level - SNOW_MELT_PER_DEGREE_SECOND * cell_temp_c * dt).max(0.0)
    } else {
        level
    }
}

/// Cover bucket for the overlay and the nav penalty (0 = bare, 1..=4 pick
/// a `snow_overlay_materials` shade). Same shape as `humidity_bucket`,
/// with the same barely-dusted floor.
pub fn snow_bucket(level: f32) -> usize {
    if level < 0.05 {
        0
    } else {
        ((level * 4.0).ceil() as usize).min(4)
    }
}

/// Nav penalty added on top of a cell's base cost: deep drifts read as
/// worse ground without ever touching the base-cost writers.
pub fn snow_cost_penalty(level: f32) -> u32 {
    snow_bucket(level) as u32 * SNOW_COST_PER_BUCKET
}

/// Drift the global freeze level toward the ambient's target. Sim-gated.
pub fn update_freeze(
    time: Res<Time>,
    clock: Res<GameClock>,
    season: Res<ActiveSeason>,
    mut freeze: ResMut<FreezeLevel>,
) {
    let ambient = ambient_temperature(season.0, clock.elapsed);
    freeze.0 = next_freeze(freeze.0, ambient, time.delta_secs());
}

/// Advance every cell's snow cover. Sim-gated. Snow lands only where the
/// sky reaches (`is_outdoors` — any roof blocks it, room or free-standing
/// stamp) and only sticks where the ground holds it: land, or water frozen
/// solid. When the ice thaws under a drift, the drift goes with it.
pub fn update_snow(
    time: Res<Time>,
    weather: Res<Weather>,
    clock: Res<GameClock>,
    season: Res<ActiveSeason>,
    roofs: Res<RoofMap>,
    water: Res<map::WaterMap>,
    freeze: Res<FreezeLevel>,
    temps: Res<TemperatureMap>,
    mut snow: ResMut<SnowMap>,
) {
    let dt = time.delta_secs();
    let ambient = ambient_temperature(season.0, clock.elapsed);
    let snow_falls = weather.rain && precipitation_form(ambient) == Precipitation::Snow;
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            let index = (y * MAP_WIDTH + x) as usize;
            let snowing = snow_falls && is_outdoors(cell, &roofs);
            let holds_snow = !water.is_wet(cell) || freeze.is_frozen();
            let cell_temp = temps.get(cell).unwrap_or(ambient);
            snow.levels[index] = next_snow(snow.levels[index], snowing, holds_snow, cell_temp, dt);
        }
    }
}

/// A whitening tile over one snowy cell; the entity IS the overlay (the
/// `WetOverlay` model).
#[derive(Component)]
pub struct SnowOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Overlay bucket for a cell: walls hide the ground, so they draw nothing
/// (their cover still tracks — it's zero anyway under a roofless wall's
/// footprint next to them). Frozen water DOES draw: snow on the ice.
fn overlay_bucket(level: f32, construction: Option<BuildableKind>) -> usize {
    if construction == Some(BuildableKind::Wall) {
        0
    } else {
        snow_bucket(level)
    }
}

/// Keep one overlay tile per snowy cell: spawn on the first dusting, swap
/// the material shade on bucket change, despawn once bare.
pub fn sync_snow_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    snow: Res<SnowMap>,
    construction: Res<ConstructionMap>,
    water: Res<map::WaterMap>,
    mut overlays: Query<(
        Entity,
        &mut SnowOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let mut has_overlay = vec![false; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (entity, mut overlay, mut material) in &mut overlays {
        let cell = overlay.cell;
        has_overlay[(cell.y * MAP_WIDTH + cell.x) as usize] = true;
        let bucket = overlay_bucket(snow.get(cell).unwrap_or(0.0), construction.get(cell));
        if bucket == 0 {
            commands.entity(entity).despawn();
        } else if bucket != overlay.bucket {
            overlay.bucket = bucket;
            material.0 = assets.snow_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let index = (y * MAP_WIDTH + x) as usize;
            if has_overlay[index] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = overlay_bucket(snow.levels[index], construction.get(cell));
            if bucket == 0 {
                continue;
            }
            let mask = map::decal_shore_mask(cell, &water);
            commands.spawn((
                Mesh3d(assets.cover_shore_meshes[mask as usize].clone()),
                MeshMaterial3d(assets.snow_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * SNOW_OVERLAY_OFFSET,
                ),
                Name::new("SnowOverlay"),
                SnowOverlay { cell, bucket },
            ));
        }
    }
}

/// Mirror the snow buckets into the nav grid's additive penalty layer.
/// Compare-then-set: the immutable read never marks `NavGrid` changed, so
/// steady snow doesn't spam whatever change-detects the grid.
pub fn sync_snow_costs(snow: Res<SnowMap>, mut nav: ResMut<NavGrid>) {
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            let penalty = snow_cost_penalty(snow.levels[(y * MAP_WIDTH + x) as usize]);
            if nav.snow_penalty(cell) != penalty {
                nav.set_snow_penalty(cell, penalty);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snow_accumulates_while_snowing_and_clamps_at_full() {
        let some = next_snow(0.0, true, true, -10.0, 1.0);
        assert!(some > 0.0, "was {some}");
        assert_eq!(next_snow(0.999, true, true, -10.0, 1e6), 1.0);
    }

    #[test]
    fn snow_keeps_under_a_clear_freezing_sky() {
        assert_eq!(next_snow(0.5, false, true, -5.0, 100.0), 0.5);
        assert_eq!(next_snow(0.5, false, true, 0.0, 100.0), 0.5);
    }

    #[test]
    fn melt_scales_with_degrees_above_zero_and_bottoms_at_bare() {
        let mild = next_snow(0.5, false, true, 5.0, 1.0);
        let hot = next_snow(0.5, false, true, 20.0, 1.0);
        assert!(mild < 0.5, "was {mild}");
        assert!(hot < mild, "hot {hot} vs mild {mild}");
        assert_eq!(next_snow(0.1, false, true, 20.0, 1e6), 0.0);
    }

    #[test]
    fn liquid_water_sheds_snow_instantly() {
        assert_eq!(next_snow(0.8, true, false, -10.0, 1.0), 0.0);
    }

    #[test]
    fn freeze_climbs_faster_the_colder_it_gets_and_clamps() {
        let deep = next_freeze(0.0, -20.0, 1.0);
        let mild = next_freeze(0.0, -5.0, 1.0);
        assert!(deep > mild && mild > 0.0, "deep {deep} vs mild {mild}");
        // Colder than the guarantee point freezes no faster.
        assert_eq!(next_freeze(0.0, -40.0, 1.0), deep);
        assert_eq!(next_freeze(0.9, -20.0, 1e6), 1.0);
        // One full -20 °C night (60 s) freezes solid from nothing.
        assert_eq!(next_freeze(0.0, -20.0, NIGHT_LENGTH_SECS), 1.0);
    }

    #[test]
    fn sub_zero_ice_never_thaws() {
        assert_eq!(next_freeze(1.0, -0.5, 1e6), 1.0);
        assert_eq!(next_freeze(1.0, 0.0, 1e6), 1.0);
    }

    #[test]
    fn thaw_scales_with_degrees_above_zero_and_bottoms_out() {
        let mild = next_freeze(1.0, 5.0, 1.0);
        let hot = next_freeze(1.0, 20.0, 1.0);
        assert!(mild < 1.0, "was {mild}");
        assert!(hot < mild, "hot {hot} vs mild {mild}");
        assert_eq!(next_freeze(1.0, 20.0, 1e6), 0.0);
    }

    #[test]
    fn ice_carries_pawns_only_near_solid() {
        assert!(!FreezeLevel(0.0).is_frozen());
        assert!(!FreezeLevel(ICE_WALKABLE_MIN_FREEZE - 0.01).is_frozen());
        assert!(FreezeLevel(ICE_WALKABLE_MIN_FREEZE).is_frozen());
        assert!(FreezeLevel(1.0).is_frozen());
    }

    #[test]
    fn snow_penalty_follows_the_buckets() {
        assert_eq!(snow_cost_penalty(0.0), 0);
        assert_eq!(snow_cost_penalty(0.04), 0, "dusting stays free");
        assert_eq!(snow_cost_penalty(0.1), SNOW_COST_PER_BUCKET);
        assert_eq!(snow_cost_penalty(1.0), 4 * SNOW_COST_PER_BUCKET);
    }
}
