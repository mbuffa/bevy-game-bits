//! Wildfire: per-cell ground fire. Cells ignite (devtool for now, lightning
//! later), burn their fuel down, and spread to neighbors chance-per-tick —
//! driven by the target's fuel, the SOURCE cell's local wind
//! (`WindExposureMap::wind_at`: fire races downwind, creeps upwind), and
//! the target's humidity. Rain soaks the ground past
//! `FIRE_EXTINGUISH_HUMIDITY` and douses it. A burnt-out cell loses its
//! grass cover and keeps a healing scorch mark that debuffs fertility and
//! blocks grass re-seeding until it fades — burned land re-greens from the
//! edges inward.
//!
//! Trees and shrubs burn WITH their cell: while it's on fire they glow
//! ember (material swap), and when the fire ends `BurnCommand` decides
//! their fate — a fire that burned most of its fuel
//! (`burn_kills_plants`) chars trees into cuttable snags and erases
//! shrubs; an early douse saves them. Bushes and crops still feed spread
//! with their fuel but don't burn down yet, pawns neither notice nor fear
//! fire, and jobs on burning cells proceed — all of that is a later phase.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;
use bevy_hanabi::prelude::*;

use crate::config::*;
use crate::cover::{self, CoverMap, Flora};
use crate::crops::Crop;
use crate::director::WanderRng;
use crate::flora::BerryBush;
use crate::game::GameAssets;
use crate::map::{self, TerrainMap};
use crate::shrubs::Shrub;
use crate::trees::Tree;
use crate::weather::{HumidityMap, Wind, WindBand, WindExposureMap};

/// One burning cell. Fuel is captured at ignition, so burn duration scales
/// with how much there was to burn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FireCell {
    /// 0..=1, ramps up after ignition and dwindles near burnout; drives the
    /// overlay bucket and spread eligibility.
    pub intensity: f32,
    pub fuel_left: f32,
    pub fuel_initial: f32,
}

/// One fire step's outcome.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FireStep {
    Burning(FireCell),
    Burnout { scorch: f32 },
}

/// A cell's fire ended (burnout, rain, or the devtool) having burned
/// `completeness` of its fuel — the standing plants' cue to decide their
/// fate (`burn_kills_plants`). Written by `tick_fire` and the ungated
/// Extinguish-all devtool, so readers must stay ungated too.
#[derive(Message)]
pub struct BurnCommand {
    pub cell: GridCoords,
    pub completeness: f32,
}

/// Per-cell burning state — the flat-grid model — plus the devtool's
/// ignition queue (`pending`): a plain resource field instead of a message,
/// so ignitions painted while paused survive until the sim resumes.
#[derive(Resource)]
pub struct FireMap {
    cells: Vec<Option<FireCell>>,
    pub pending: Vec<GridCoords>,
}

impl Default for FireMap {
    fn default() -> Self {
        Self {
            cells: vec![None; (MAP_WIDTH * MAP_HEIGHT) as usize],
            pending: Vec::new(),
        }
    }
}

impl FireMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<FireCell> {
        Self::in_bounds(cell)
            .then(|| self.cells[(cell.y * MAP_WIDTH + cell.x) as usize])
            .flatten()
    }

    pub fn is_burning(&self, cell: GridCoords) -> bool {
        self.get(cell).is_some()
    }

    fn set(&mut self, cell: GridCoords, state: Option<FireCell>) {
        if Self::in_bounds(cell) {
            self.cells[(cell.y * MAP_WIDTH + cell.x) as usize] = state;
        }
    }

    pub fn ignite(&mut self, cell: GridCoords, fuel: f32) {
        self.set(
            cell,
            Some(FireCell {
                intensity: 0.0,
                fuel_left: fuel,
                fuel_initial: fuel,
            }),
        );
    }

    /// Put out one cell, returning how much of its fuel had burned (for
    /// the scorch mark). Used by the rain-douse path indirectly and the
    /// Extinguish-all devtool directly.
    pub fn extinguish(&mut self, cell: GridCoords) -> Option<f32> {
        let state = self.get(cell)?;
        self.set(cell, None);
        Some(1.0 - state.fuel_left / state.fuel_initial.max(f32::EPSILON))
    }

    /// Snapshot of every burning cell (the tick mutates while iterating).
    pub fn burning_cells(&self) -> Vec<(GridCoords, FireCell)> {
        let mut burning = Vec::new();
        for y in 0..MAP_HEIGHT {
            for x in 0..MAP_WIDTH {
                if let Some(state) = self.cells[(y * MAP_WIDTH + x) as usize] {
                    burning.push((GridCoords::new(x, y), state));
                }
            }
        }
        burning
    }
}

/// Per-cell scorch 0..=1, left behind by fire; heals slowly.
#[derive(Resource)]
pub struct ScorchMap {
    levels: Vec<f32>,
}

impl Default for ScorchMap {
    fn default() -> Self {
        Self {
            levels: vec![0.0; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl ScorchMap {
    pub fn get(&self, cell: GridCoords) -> f32 {
        if FireMap::in_bounds(cell) {
            self.levels[(cell.y * MAP_WIDTH + cell.x) as usize]
        } else {
            0.0
        }
    }

    pub fn set(&mut self, cell: GridCoords, level: f32) {
        if FireMap::in_bounds(cell) {
            self.levels[(cell.y * MAP_WIDTH + cell.x) as usize] = level.clamp(0.0, 1.0);
        }
    }
}

/// The fire sim's fixed-interval clock: honest per-tick probabilities
/// instead of per-frame ones.
#[derive(Resource)]
pub struct FireTickTimer(pub Timer);

impl Default for FireTickTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(FIRE_TICK_SECS, TimerMode::Repeating))
    }
}

/// May this cell catch fire at all?
pub fn can_ignite(fuel: f32, humidity: f32) -> bool {
    fuel >= FIRE_MIN_FUEL && humidity <= FIRE_IGNITE_MAX_HUMIDITY
}

/// World-plane direction of the grid offset `(dx, dy)`. THE grid/world
/// seam: grid +y is world -Z (`map::grid_to_world` negates y), so every
/// offset-to-direction conversion goes through here.
pub fn neighbor_world_dir(dx: i32, dy: i32) -> Vec2 {
    Vec2::new(dx as f32, -(dy as f32)).normalize_or_zero()
}

/// Chance (per tick) that a burning cell ignites one neighbor. Fuel feeds
/// it, the source's local wind pushes it downwind (strength matters:
/// heavy wind races), the floor keeps a slow upwind/crosswind creep, and
/// the target's wetness resists.
pub fn spread_chance(
    fuel_target: f32,
    wind_at_source: Vec2,
    dir_world: Vec2,
    humidity_target: f32,
) -> f32 {
    if !can_ignite(fuel_target, humidity_target) {
        return 0.0;
    }
    let alignment = wind_at_source.normalize_or_zero().dot(dir_world);
    let wind_factor = (1.0 + FIRE_WIND_ALIGNMENT * alignment * wind_at_source.length())
        .clamp(FIRE_CREEP_FLOOR, FIRE_WIND_FACTOR_MAX);
    (FIRE_SPREAD_BASE_CHANCE * fuel_target * wind_factor * (1.0 - humidity_target)).min(1.0)
}

/// Advance one burning cell by `dt`: intensity ramps toward 1, dwindles as
/// the fuel runs out, and the fire consumes fuel in proportion to how hot
/// it burns. Below `FIRE_MIN_FUEL` remaining, it's out.
pub fn next_fire(cell: FireCell, dt: f32) -> FireStep {
    let mut next = cell;
    next.intensity = (next.intensity + FIRE_RAMP_PER_SECOND * dt)
        .min(1.0)
        .min(next.fuel_left / FIRE_DWINDLE_FUEL);
    next.fuel_left -= FIRE_CONSUME_PER_SECOND * next.intensity * dt;
    if next.fuel_left <= FIRE_MIN_FUEL {
        FireStep::Burnout { scorch: 1.0 }
    } else {
        FireStep::Burning(next)
    }
}

/// Scorch left by a DOUSED fire: proportional to how much burned, floored —
/// even a briefly-burnt cell shows a mark. (Natural burnout is always 1.0.)
pub fn extinguish_scorch(completeness: f32) -> f32 {
    completeness.clamp(SCORCH_MIN, 1.0)
}

/// One healing step.
pub fn heal_step(level: f32, dt: f32) -> f32 {
    (level - SCORCH_HEAL_PER_SECOND * dt).max(0.0)
}

/// Fertility multiplier for a cell's scorch (1.0 = unburnt).
pub fn fertility_factor(scorch: f32) -> f32 {
    1.0 - SCORCH_FERTILITY_PENALTY * scorch.clamp(0.0, 1.0)
}

/// Does a fire that ended at `completeness` kill the standing plants on
/// its cell? Natural burnout (1.0) always does; a douse early enough in
/// the burn saves them.
pub fn burn_kills_plants(completeness: f32) -> bool {
    completeness >= FIRE_PLANT_KILL_COMPLETENESS
}

/// Overlay bucket for burning intensity (0 = none).
fn burning_bucket(intensity: f32) -> usize {
    if intensity < 0.05 {
        0
    } else {
        ((intensity * 4.0).ceil() as usize).min(4)
    }
}

/// Overlay bucket for scorch (0 = none).
fn scorch_bucket(level: f32) -> usize {
    if level < 0.05 {
        0
    } else {
        ((level * 4.0).ceil() as usize).min(4)
    }
}

/// Advance the whole fire sim on its fixed tick: light pending devtool
/// ignitions, step every burning cell (rain douses, fuel runs out), and
/// roll spread to the 8 neighbors. Sim-gated, chained after
/// `update_wind_exposure` so it reads this frame's wind field.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tick_fire(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<FireTickTimer>,
    wind: Res<Wind>,
    exposure: Res<WindExposureMap>,
    humidity: Res<HumidityMap>,
    terrain: Res<TerrainMap>,
    construction: Res<crate::construction::ConstructionMap>,
    cover_map: Res<CoverMap>,
    water: Res<map::WaterMap>,
    mut rng: ResMut<WanderRng>,
    mut fire: ResMut<FireMap>,
    mut scorch: ResMut<ScorchMap>,
    mut burns: MessageWriter<BurnCommand>,
    plant_queries: (
        Query<(&Tree, &GridCoords)>,
        Query<(&BerryBush, &GridCoords)>,
        Query<(&Crop, &GridCoords)>,
        Query<(&Shrub, &GridCoords)>,
    ),
    flora: Query<(Entity, &Flora, &GridCoords)>,
) {
    let ticks = timer.0.tick(time.delta()).times_finished_this_tick();
    if ticks == 0 && fire.pending.is_empty() {
        return;
    }

    let (trees, bushes, crops, shrubs) = &plant_queries;
    let plant_fuel = cover::collect_plant_fuel(
        trees
            .iter()
            .map(|(tree, grid)| (*grid, tree.fuel()))
            .chain(bushes.iter().map(|(bush, grid)| (*grid, bush.fuel())))
            .chain(crops.iter().map(|(crop, grid)| (*grid, crop.fuel())))
            .chain(shrubs.iter().map(|(shrub, grid)| (*grid, shrub.fuel()))),
    );
    let fuel_at = |cell: GridCoords| {
        let plant = if FireMap::in_bounds(cell) {
            plant_fuel[(cell.y * MAP_WIDTH + cell.x) as usize]
        } else {
            0.0
        };
        cover::fuel_at(cell, &terrain, &construction, &cover_map, &water, plant)
    };
    let wetness = |cell: GridCoords| humidity.get(cell).unwrap_or(0.0);

    // Devtool ignitions: validate the same gates natural spread uses.
    // Freshly scorched ground can't catch — this also closes the
    // re-ignition window where a burnt cell's Flora despawn (deferred)
    // leaves its fuel visible to neighbors for one frame.
    let pending = std::mem::take(&mut fire.pending);
    for cell in pending {
        if !fire.is_burning(cell)
            && scorch.get(cell) < SCORCH_REGROW_MAX
            && can_ignite(fuel_at(cell), wetness(cell))
        {
            info!("fire: ignited at {cell:?}");
            fire.ignite(cell, fuel_at(cell));
        }
    }

    // Flora despawns and BurnCommands are deferred/buffered: guard against
    // a cell burning out twice across multiple ticks in one frame.
    let mut burnt_cells: HashSet<GridCoords> = HashSet::new();
    for _ in 0..ticks {
        for (cell, state) in fire.burning_cells() {
            // Rain (or the river soaking in) douses the fire; a natural
            // burnout has consumed (essentially) everything.
            let (step, completeness) = if wetness(cell) >= FIRE_EXTINGUISH_HUMIDITY {
                let completeness = 1.0 - state.fuel_left / state.fuel_initial.max(f32::EPSILON);
                (
                    FireStep::Burnout {
                        scorch: extinguish_scorch(completeness),
                    },
                    completeness,
                )
            } else {
                (next_fire(state, FIRE_TICK_SECS), 1.0)
            };
            match step {
                FireStep::Burning(next) => {
                    fire.set(cell, Some(next));
                    if next.intensity < FIRE_SPREAD_MIN_INTENSITY {
                        continue;
                    }
                    let wind_here = exposure.wind_at(cell, &wind, WindBand::Ground);
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            let neighbor = GridCoords::new(cell.x + dx, cell.y + dy);
                            if !FireMap::in_bounds(neighbor)
                                || fire.is_burning(neighbor)
                                // Freshly scorched ground can't re-ignite
                                // (and this closes the one-frame stale-fuel
                                // window after a neighbor's burnout).
                                || scorch.get(neighbor) >= SCORCH_REGROW_MAX
                            {
                                continue;
                            }
                            let chance = spread_chance(
                                fuel_at(neighbor),
                                wind_here,
                                neighbor_world_dir(dx, dy),
                                wetness(neighbor),
                            );
                            if rng.chance(chance) {
                                info!("fire: spreads to {neighbor:?}");
                                fire.ignite(neighbor, fuel_at(neighbor));
                            }
                        }
                    }
                }
                FireStep::Burnout { scorch: mark } => {
                    fire.set(cell, None);
                    let level = scorch.get(cell);
                    scorch.set(cell, level.max(mark));
                    // The grass is gone; it re-seeds from unburnt neighbors
                    // once the scorch heals. (CoverMap catches up a frame
                    // later — deferred despawn — which is harmless.) The
                    // standing plants decide their own fate from the
                    // BurnCommand.
                    if burnt_cells.insert(cell) {
                        burns.write(BurnCommand { cell, completeness });
                        for (entity, patch, grid) in &flora {
                            if *grid == cell && matches!(patch.kind, cover::FloraKind::Grass) {
                                commands.entity(entity).despawn();
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Heal every cell's scorch. Sim-gated.
pub fn heal_scorch(time: Res<Time>, mut scorch: ResMut<ScorchMap>) {
    let dt = time.delta_secs();
    for level in &mut scorch.levels {
        if *level > 0.0 {
            *level = heal_step(*level, dt);
        }
    }
}

/// A translucent orange tile over one burning cell — always shown; fire is
/// game state, not a devtool overlay.
#[derive(Component)]
pub struct BurningOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// A dark tile over scorched ground — always shown.
#[derive(Component)]
pub struct ScorchOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Keep one orange overlay per burning cell (the humidity-overlay
/// reconciler).
pub fn sync_burning_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    fire: Res<FireMap>,
    mut overlays: Query<(
        Entity,
        &mut BurningOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let desired = |cell: GridCoords| -> usize {
        burning_bucket(fire.get(cell).map_or(0.0, |state| state.intensity))
    };
    let mut has_overlay = vec![false; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (entity, mut overlay, mut material) in &mut overlays {
        let cell = overlay.cell;
        has_overlay[(cell.y * MAP_WIDTH + cell.x) as usize] = true;
        let bucket = desired(cell);
        if bucket == 0 {
            commands.entity(entity).despawn();
        } else if bucket != overlay.bucket {
            overlay.bucket = bucket;
            material.0 = assets.burning_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            if has_overlay[(y * MAP_WIDTH + x) as usize] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = desired(cell);
            if bucket == 0 {
                continue;
            }
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(assets.burning_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * BURNING_OVERLAY_OFFSET,
                ),
                Name::new("BurningOverlay"),
                BurningOverlay { cell, bucket },
            ));
        }
    }
}

/// Keep one dark overlay per scorched cell (same reconciler).
pub fn sync_scorch_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    scorch: Res<ScorchMap>,
    mut overlays: Query<(
        Entity,
        &mut ScorchOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let desired = |cell: GridCoords| -> usize { scorch_bucket(scorch.get(cell)) };
    let mut has_overlay = vec![false; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (entity, mut overlay, mut material) in &mut overlays {
        let cell = overlay.cell;
        has_overlay[(cell.y * MAP_WIDTH + cell.x) as usize] = true;
        let bucket = desired(cell);
        if bucket == 0 {
            commands.entity(entity).despawn();
        } else if bucket != overlay.bucket {
            overlay.bucket = bucket;
            material.0 = assets.scorch_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            if has_overlay[(y * MAP_WIDTH + x) as usize] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = desired(cell);
            if bucket == 0 {
                continue;
            }
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(assets.scorch_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * SCORCH_OVERLAY_OFFSET,
                ),
                Name::new("ScorchOverlay"),
                ScorchOverlay { cell, bucket },
            ));
        }
    }
}

/// One pooled Hanabi emitter, parked inactive until assigned to a burning
/// cell. The whole pool is spawned at startup: bevy_hanabi reallocates GPU
/// buffers when effect instances are created at runtime, which crashes
/// wgpu mid-frame ("hanabi:buffer:spawner has been destroyed") — and a
/// fixed pool bounds the worst case anyway. Deactivating (instead of
/// despawning) also lets in-flight smoke fade out naturally.
#[derive(Component)]
pub struct FireEmitter {
    pub cell: Option<GridCoords>,
}

/// Author the flame->smoke effect and park the whole emitter pool:
/// particles spawn low over the tile, rise (velocity property, so the live
/// wind can drift the smoke), tint from flame orange through ember red to
/// fading smoke grey. Every instance starts INACTIVE; `sync_fire_emitters`
/// assigns them to burning cells.
pub fn setup(mut commands: Commands, mut effects: ResMut<Assets<EffectAsset>>) {
    let spawner = SpawnerSettings::rate(FIRE_SPAWN_RATE.into()).with_starts_active(false);

    let writer = ExprWriter::new();
    let wind_vel = writer.add_property(
        "wind_velocity",
        Value::Vector(Vec3::new(0.0, FIRE_RISE_SPEED, 0.0).into()),
    );
    let init_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        writer
            .lit(Vec3::new(-0.35, 0.0, -0.35))
            .uniform(writer.lit(Vec3::new(0.35, 0.1, 0.35)))
            .expr(),
    );
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, writer.prop(wind_vel).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(FIRE_PARTICLE_LIFETIME).expr(),
    );
    let module = writer.finish();

    // Flame -> ember -> smoke in one gradient. Explicit path: the Bevy
    // prelude has a UI `Gradient` of the same name.
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(1.0, 0.6, 0.1, 0.8));
    color.add_key(0.3, Vec4::new(0.9, 0.2, 0.05, 0.6));
    color.add_key(0.6, Vec4::new(0.4, 0.4, 0.4, 0.3));
    color.add_key(1.0, Vec4::new(0.3, 0.3, 0.3, 0.0));
    // The size gradient OVERWRITES the size each frame (it doesn't scale an
    // init value) — the flame swells slightly, the smoke thins to nothing.
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(FIRE_PARTICLE_SIZE * 0.7));
    size.add_key(0.3, Vec3::splat(FIRE_PARTICLE_SIZE));
    size.add_key(1.0, Vec3::ZERO);

    let effect = effects.add(
        EffectAsset::new(FIRE_PARTICLE_CAPACITY, spawner, module)
            .with_name("fire")
            .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
            .init(init_pos)
            .init(init_vel)
            .init(init_age)
            .init(init_lifetime)
            .render(SizeOverLifetimeModifier {
                gradient: size,
                ..default()
            })
            .render(ColorOverLifetimeModifier::new(color)),
    );

    for i in 0..FIRE_EMITTER_POOL {
        commands.spawn((
            ParticleEffect::new(effect.clone()),
            EffectProperties::default(),
            Transform::default(),
            Name::new(format!("FireEmitter {i}")),
            FireEmitter { cell: None },
        ));
    }
}

/// Assign pooled emitters to burning cells: free the ones whose fire went
/// out (deactivate — the smoke fades in place), then park a free emitter on
/// every uncovered burning cell. `EffectSpawner` is `Option` because Hanabi
/// inserts it a frame late (the `sync_rain_effect` reason).
pub fn sync_fire_emitters(
    fire: Res<FireMap>,
    mut emitters: Query<(&mut FireEmitter, &mut Transform, Option<&mut EffectSpawner>)>,
) {
    let mut covered: HashSet<GridCoords> = HashSet::new();
    // Pass 1: release emitters whose cell stopped burning, note the rest.
    for (mut emitter, _, spawner) in &mut emitters {
        if let Some(cell) = emitter.cell {
            if fire.is_burning(cell) {
                covered.insert(cell);
            } else {
                emitter.cell = None;
                if let Some(mut spawner) = spawner {
                    spawner.active = false;
                }
            }
        }
    }
    // Pass 2: park free emitters on uncovered burning cells.
    let mut uncovered: Vec<GridCoords> = fire
        .burning_cells()
        .into_iter()
        .map(|(cell, _)| cell)
        .filter(|cell| !covered.contains(cell))
        .collect();
    if uncovered.is_empty() {
        return;
    }
    for (mut emitter, mut transform, spawner) in &mut emitters {
        if emitter.cell.is_some() {
            continue;
        }
        let Some(cell) = uncovered.pop() else {
            break;
        };
        emitter.cell = Some(cell);
        transform.translation = map::grid_to_world(&cell);
        if let Some(mut spawner) = spawner {
            spawner.active = true;
        }
    }
}

/// Keep every ASSIGNED emitter's spawner on (Hanabi inserts `EffectSpawner`
/// a frame after startup, so `sync_fire_emitters`' activation can miss the
/// first assignment) and drift its smoke with the cell's LOCAL wind
/// (sheltered fires smoke straight up). Ungated: a paused wind is frozen,
/// `set_if_changed` makes it a no-op.
pub fn sync_fire_wind(
    wind: Res<Wind>,
    exposure: Res<WindExposureMap>,
    mut emitters: Query<(
        &FireEmitter,
        &mut EffectProperties,
        Option<&mut EffectSpawner>,
    )>,
) {
    for (emitter, properties, spawner) in &mut emitters {
        let Some(cell) = emitter.cell else {
            continue;
        };
        if let Some(mut spawner) = spawner {
            if !spawner.active {
                spawner.active = true;
            }
        }
        let drift = exposure.wind_at(cell, &wind, WindBand::Ground) * FIRE_SMOKE_DRIFT;
        EffectProperties::set_if_changed(
            properties,
            "wind_velocity",
            Vec3::new(drift.x, FIRE_RISE_SPEED, drift.y).into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbor_world_dir_flips_grid_y() {
        // Grid +x is world +x...
        assert!((neighbor_world_dir(1, 0) - Vec2::new(1.0, 0.0)).length() < 1e-6);
        // ...but grid +y is world -Z: wind toward world +Z must favor the
        // grid (0, -1) neighbor.
        assert!((neighbor_world_dir(0, -1) - Vec2::new(0.0, 1.0)).length() < 1e-6);
        let downwind = spread_chance(1.0, Vec2::new(0.0, 1.0), neighbor_world_dir(0, -1), 0.0);
        let upwind = spread_chance(1.0, Vec2::new(0.0, 1.0), neighbor_world_dir(0, 1), 0.0);
        assert!(downwind > upwind);
    }

    #[test]
    fn spread_prefers_downwind() {
        let wind = Vec2::new(1.5, 0.0); // calm strength, toward +X
        let downwind = spread_chance(1.0, wind, neighbor_world_dir(1, 0), 0.0);
        let crosswind = spread_chance(1.0, wind, neighbor_world_dir(0, 1), 0.0);
        let upwind = spread_chance(1.0, wind, neighbor_world_dir(-1, 0), 0.0);
        assert!(downwind > crosswind);
        assert!(crosswind > upwind);
    }

    #[test]
    fn upwind_still_creeps() {
        let wind = Vec2::new(6.0, 0.0); // heavy
        let upwind = spread_chance(1.0, wind, neighbor_world_dir(-1, 0), 0.0);
        let downwind = spread_chance(1.0, wind, neighbor_world_dir(1, 0), 0.0);
        assert!(upwind > 0.0);
        assert!(upwind < downwind / 10.0);
        // Exactly the floor: base x fuel x floor.
        assert!((upwind - FIRE_SPREAD_BASE_CHANCE * FIRE_CREEP_FLOOR).abs() < 1e-6);
    }

    #[test]
    fn heavy_wind_accelerates_downwind() {
        let calm = spread_chance(1.0, Vec2::new(1.5, 0.0), neighbor_world_dir(1, 0), 0.0);
        let heavy = spread_chance(1.0, Vec2::new(6.0, 0.0), neighbor_world_dir(1, 0), 0.0);
        assert!(heavy > calm);
        // The wind factor caps out.
        let gale = spread_chance(1.0, Vec2::new(100.0, 0.0), neighbor_world_dir(1, 0), 0.0);
        assert!((gale - FIRE_SPREAD_BASE_CHANCE * FIRE_WIND_FACTOR_MAX).abs() < 1e-5);
    }

    #[test]
    fn humidity_gates_ignition_and_scales_spread() {
        assert!(!can_ignite(1.0, FIRE_IGNITE_MAX_HUMIDITY + 0.01));
        assert!(can_ignite(1.0, 0.0));
        assert_eq!(
            spread_chance(1.0, Vec2::ZERO, Vec2::X, FIRE_IGNITE_MAX_HUMIDITY + 0.01),
            0.0
        );
        let dry = spread_chance(1.0, Vec2::ZERO, Vec2::X, 0.0);
        let damp = spread_chance(1.0, Vec2::ZERO, Vec2::X, 0.4);
        assert!((damp - dry * 0.6).abs() < 1e-6);
    }

    #[test]
    fn no_fuel_no_fire() {
        assert!(!can_ignite(FIRE_MIN_FUEL - 0.01, 0.0));
        assert_eq!(
            spread_chance(FIRE_MIN_FUEL - 0.01, Vec2::new(6.0, 0.0), Vec2::X, 0.0),
            0.0
        );
    }

    #[test]
    fn intensity_ramps_then_dwindles() {
        let fresh = FireCell {
            intensity: 0.0,
            fuel_left: 1.0,
            fuel_initial: 1.0,
        };
        let FireStep::Burning(stepped) = next_fire(fresh, 1.0) else {
            panic!("fresh fire burnt out");
        };
        assert!((stepped.intensity - FIRE_RAMP_PER_SECOND).abs() < 1e-6);
        // Near-empty fuel caps the flame low.
        let dwindling = FireCell {
            intensity: 1.0,
            fuel_left: FIRE_DWINDLE_FUEL / 2.0,
            fuel_initial: 1.0,
        };
        let FireStep::Burning(stepped) = next_fire(dwindling, 0.1) else {
            panic!("dwindling fire burnt out early");
        };
        assert!(stepped.intensity <= 0.5 + 1e-6);
    }

    #[test]
    fn burnout_accounting() {
        // Fuel drains in proportion to intensity, and a fire runs out.
        let mut state = FireCell {
            intensity: 0.0,
            fuel_left: 1.0,
            fuel_initial: 1.0,
        };
        let mut seconds = 0.0;
        loop {
            match next_fire(state, FIRE_TICK_SECS) {
                FireStep::Burning(next) => {
                    assert!(next.fuel_left < state.fuel_left);
                    state = next;
                    seconds += FIRE_TICK_SECS;
                    assert!(seconds < 120.0, "fire never burnt out");
                }
                FireStep::Burnout { scorch } => {
                    assert_eq!(scorch, 1.0);
                    break;
                }
            }
        }
        // A thin-fuel cell burns out sooner.
        let mut thin = FireCell {
            intensity: 0.0,
            fuel_left: 0.3,
            fuel_initial: 0.3,
        };
        let mut thin_seconds = 0.0;
        while let FireStep::Burning(next) = next_fire(thin, FIRE_TICK_SECS) {
            thin = next;
            thin_seconds += FIRE_TICK_SECS;
        }
        assert!(thin_seconds < seconds);
    }

    #[test]
    fn extinguish_scorch_is_partial_but_floored() {
        assert_eq!(extinguish_scorch(0.1), SCORCH_MIN);
        assert!((extinguish_scorch(0.7) - 0.7).abs() < 1e-6);
        assert_eq!(extinguish_scorch(1.5), 1.0);
    }

    #[test]
    fn scorch_heals_to_zero() {
        let mut level = 1.0;
        let healed = heal_step(level, 1.0);
        assert!(healed < level);
        level = 0.001;
        assert_eq!(heal_step(level, 10.0), 0.0);
    }

    #[test]
    fn fertility_factor_bounds() {
        assert_eq!(fertility_factor(0.0), 1.0);
        assert!((fertility_factor(1.0) - (1.0 - SCORCH_FERTILITY_PENALTY)).abs() < 1e-6);
        assert!(fertility_factor(0.5) > fertility_factor(1.0));
    }

    #[test]
    fn overlay_buckets_quantize_with_floor() {
        for bucket in [burning_bucket as fn(f32) -> usize, scorch_bucket] {
            assert_eq!(bucket(0.0), 0);
            assert_eq!(bucket(0.04), 0);
            assert_eq!(bucket(0.05), 1);
            assert_eq!(bucket(0.6), 3);
            assert_eq!(bucket(1.0), 4);
        }
    }

    #[test]
    fn burn_kill_threshold() {
        // A barely-started fire that gets doused spares the plants...
        assert!(!burn_kills_plants(0.0));
        assert!(!burn_kills_plants(FIRE_PLANT_KILL_COMPLETENESS - 0.01));
        // ...a mostly-burnt one doesn't, and natural burnout (1.0) never does.
        assert!(burn_kills_plants(FIRE_PLANT_KILL_COMPLETENESS));
        assert!(burn_kills_plants(1.0));
    }

    #[test]
    fn fire_particles_fit_capacity() {
        assert!(FIRE_SPAWN_RATE * FIRE_PARTICLE_LIFETIME < FIRE_PARTICLE_CAPACITY as f32);
    }
}
