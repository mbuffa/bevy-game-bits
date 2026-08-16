//! Weather: precipitation over the colony — rain, or snow when the ambient
//! is freezing (`precipitation_form`) — rendered as bevy_hanabi GPU particle
//! effects, plus its first consequence: per-cell humidity that rain (and
//! melting snow, see `snow.rs`) raises and clear weather dries back out.
//! For now the debug window's Rain button is the only thing driving the
//! `Weather` resource; a dynamic weather director will replace it later.
//! Humidity has no gameplay effect yet: it's stored, read by the cursor
//! tooltip, and drawn as a darkening overlay on wet ground.
//!
//! Also home to the permanent `Wind`: gusting strength and a slowly
//! wandering heading (devtool-shiftable, always turning smoothly) that
//! slants the rain, bends the wheat (via `WindSwayMaterial`), drives the
//! always-on gust "snakes" — ribbon trails behind wind-gliding `GustHead`
//! emitters — steers the turbines' yaw chase, feeds fire spread, and (scaled
//! by per-cell `WindExposureMap`) drives turbine power. Exposure is banded
//! by height (`WindBand`): short blockers (walls, doors, turbine bases)
//! only shelter `Ground`, so a wall can't wrongly becalm a taller turbine's
//! rotor — only tree canopies (and later, mountains/tall buildings) reach
//! high enough to shelter `Altitude` too.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy_ecs_ldtk::prelude::GridCoords;
use bevy_hanabi::prelude::*;

use crate::config::*;
use crate::construction::{BuildableKind, ConstructionMap};
use crate::cover::{self, CoverMap};
use crate::crops::Crop;
use crate::flora::BerryBush;
use crate::game::GameAssets;
use crate::map::{self, RoofMap, TerrainMap};
use crate::shrubs::Shrub;
use crate::trees::Tree;

/// Current weather, the single source of truth. `rain` means "precipitation
/// on" — whether it falls as rain or snow is `precipitation_form`'s call.
#[derive(Resource, Default)]
pub struct Weather {
    pub rain: bool,
}

/// What precipitation falls right now, derived from the ambient — one
/// switch (the Rain devtool, later a weather director) covers both forms,
/// and a season change re-skins the sky for free.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Precipitation {
    Rain,
    Snow,
}

/// Freezing air turns the rain to snow.
pub fn precipitation_form(ambient_c: f32) -> Precipitation {
    if ambient_c <= 0.0 {
        Precipitation::Snow
    } else {
        Precipitation::Rain
    }
}

/// Permanent ambient wind on the XZ ground plane: a strength that gusts
/// around its base value and a heading that slowly wanders around
/// `WIND_DIRECTION` (plus devtool shifts), always turning smoothly at
/// `WIND_TURN_SPEED`. Its own resource (not a `Weather` field) because wind
/// never turns off and will grow more consumers — fire spread, wind
/// turbines — that shouldn't depend on rain state.
#[derive(Resource)]
pub struct Wind {
    /// Normalized XZ direction the wind blows toward — the smoothed ACTUAL
    /// direction every consumer reads. Kept in sync with `heading`.
    pub direction: Vec2,
    /// Devtool switch: swap the calm base for `WIND_HEAVY_STRENGTH`.
    pub heavy: bool,
    /// Effective strength right now (base x gust), kept fresh by
    /// `update_wind`.
    pub current: f32,
    /// Sim-time accumulator driving gusts, wander, and the sway shader;
    /// freezes with the rest of the sim on pause.
    pub elapsed: f32,
    /// Current heading in radians (`direction = Vec2::from_angle(heading)`),
    /// chasing base + wander + shift at `WIND_TURN_SPEED`.
    pub heading: f32,
    /// Devtool offset added to the desired heading (`Shift Wind` clicks).
    pub shift_offset: f32,
}

impl Default for Wind {
    fn default() -> Self {
        let heading = WIND_DIRECTION.normalize().to_angle();
        Self {
            direction: Vec2::from_angle(heading),
            heavy: false,
            current: WIND_BASE_STRENGTH,
            elapsed: 0.0,
            heading,
            shift_offset: 0.0,
        }
    }
}

impl Wind {
    pub fn base_strength(&self) -> f32 {
        if self.heavy {
            WIND_HEAVY_STRENGTH
        } else {
            WIND_BASE_STRENGTH
        }
    }

    /// The read-side API for wind consumers (rain slant today; fire and
    /// turbines later): direction scaled by the gusting strength, on XZ.
    pub fn current_vector(&self) -> Vec2 {
        self.direction * self.current
    }
}

/// Vertex-sway extension over `StandardMaterial` (the repo's first shader):
/// `wheat_sway.wgsl` bends vertices along the wind, weighted by height², so
/// bases stay pinned while tips bend. PBR fragment/lighting is inherited
/// from the base material untouched.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct WindSwayExtension {
    /// xy = wind direction (XZ), z = current strength, w = sim time.
    #[uniform(100)]
    pub wind: Vec4,
}

impl MaterialExtension for WindSwayExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/wheat_sway.wgsl".into()
    }
}

pub type WindSwayMaterial = ExtendedMaterial<StandardMaterial, WindSwayExtension>;

/// Mirror the live `Wind` into the shared sway materials' uniform — wheat
/// AND algae reeds both use `WindSwayMaterial`. Ungated (like
/// `sync_rain_effect`): the sim-gated `update_wind` freezes `elapsed` on
/// pause, and a frozen time uniform freezes the sway with it.
pub fn sync_wind_material(
    wind: Res<Wind>,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<WindSwayMaterial>>,
) {
    let uniform = Vec4::new(
        wind.direction.x,
        wind.direction.y,
        wind.current,
        wind.elapsed,
    );
    for handle in [&assets.crop_material, &assets.algae_reed_material] {
        if let Some(material) = materials.get_mut(handle) {
            material.extension.wind = uniform;
        }
    }
}

/// Mirror the wind clock and strength into the water ripple material
/// (`map::WaterExtension`). Same shape as `sync_wind_material` above:
/// ungated, with `Wind.elapsed` as the time uniform so ripples freeze with
/// the sim. The flow *direction* is the constant `WATER_FLOW_DIRECTION`
/// (rivers don't follow the wind); wind strength only scales the chop.
pub fn sync_water_material(
    wind: Res<Wind>,
    freeze: Res<crate::snow::FreezeLevel>,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<map::WaterMaterial>>,
) {
    if let Some(material) = materials.get_mut(&assets.water_material) {
        material.extension.params = Vec4::new(
            WATER_FLOW_DIRECTION.x,
            WATER_FLOW_DIRECTION.y,
            wind.elapsed,
            wind.current,
        );
        material.extension.ice = Vec4::new(freeze.0, 0.0, 0.0, 0.0);
    }
}

/// Gust factor around 1.0: two incommensurate sine frequencies layered so
/// the swell doesn't visibly loop. Always positive (amplitudes sum < 1).
fn gust_multiplier(t: f32) -> f32 {
    use std::f32::consts::TAU;
    1.0 + WIND_GUST_AMPLITUDE_PRIMARY * (TAU * WIND_GUST_FREQUENCY_PRIMARY * t).sin()
        + WIND_GUST_AMPLITUDE_SECONDARY * (TAU * WIND_GUST_FREQUENCY_SECONDARY * t).sin()
}

/// Ambient heading wander (radians around the base direction): the gust
/// model an order of magnitude slower, so the wind meanders over minutes.
fn wander_offset(t: f32) -> f32 {
    use std::f32::consts::TAU;
    WIND_WANDER_PRIMARY * (TAU * WIND_WANDER_FREQUENCY_PRIMARY * t).sin()
        + WIND_WANDER_SECONDARY * (TAU * WIND_WANDER_FREQUENCY_SECONDARY * t).sin()
}

/// One smoothing step of an angle toward a desired angle: the SHORTEST way
/// around (a `desired` across the +-PI seam turns through the seam, not the
/// long way through zero), clamped to `max_step` radians.
fn turn_toward(current: f32, desired: f32, max_step: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let difference = (desired - current + PI).rem_euclid(TAU) - PI;
    current + difference.clamp(-max_step, max_step)
}

/// Advance the wind clock, refresh the gusting strength, and swing the
/// heading toward base + wander + devtool shift at the capped turn rate.
/// Sim-gated: pausing freezes the wind (and with it the wheat sway) like
/// everything else.
pub fn update_wind(time: Res<Time>, mut wind: ResMut<Wind>) {
    wind.elapsed += time.delta_secs();
    wind.current = wind.base_strength() * gust_multiplier(wind.elapsed);
    let desired =
        WIND_DIRECTION.normalize().to_angle() + wander_offset(wind.elapsed) + wind.shift_offset;
    wind.heading = turn_toward(wind.heading, desired, WIND_TURN_SPEED * time.delta_secs());
    wind.direction = Vec2::from_angle(wind.heading);
}

/// Per-cell wetness 0..=1 — the `TerrainMap` model: a flat grid resource,
/// not per-tile entities.
#[derive(Resource)]
pub struct HumidityMap {
    levels: Vec<f32>,
}

impl Default for HumidityMap {
    fn default() -> Self {
        Self {
            levels: vec![0.0; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl HumidityMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<f32> {
        Self::in_bounds(cell).then(|| self.levels[(cell.y * MAP_WIDTH + cell.x) as usize])
    }

    /// Direct write, clamped 0..=1 — test-only; the sim itself only ever
    /// advances a level through `next_humidity`.
    #[cfg(test)]
    pub fn set(&mut self, cell: GridCoords, level: f32) {
        if Self::in_bounds(cell) {
            self.levels[(cell.y * MAP_WIDTH + cell.x) as usize] = level.clamp(0.0, 1.0);
        }
    }
}

/// Whether rain reaches this cell: anything without a built roof overhead.
pub fn is_outdoors(cell: GridCoords, roofs: &RoofMap) -> bool {
    !roofs.is_roofed(cell)
}

/// Which height a wind reading is taken at. Short blockers (walls, doors,
/// turbine bases — anything under rotor height) only shelter `Ground`;
/// something tall enough to reach a turbine's rotor (today, just tree
/// canopies; later mountains/tall buildings) also shelters `Altitude`. Keeps
/// a wall from wrongly becalming a turbine towering over it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindBand {
    /// What walls, doors, pawns, and fire feel.
    Ground,
    /// What a turbine's rotor feels.
    Altitude,
}

/// Per-cell wind exposure 0..=1, one grid per `WindBand` — the
/// `TerrainMap`/`HumidityMap` flat-grid model. Blockers upwind of a cell
/// shelter it (see `cell_exposure`); roofed cells are becalmed outright in
/// both bands. The wildfire mechanic reads `Ground` through `wind_at`;
/// turbines read `Altitude`.
#[derive(Resource)]
pub struct WindExposureMap {
    ground: Vec<f32>,
    altitude: Vec<f32>,
}

impl Default for WindExposureMap {
    fn default() -> Self {
        Self {
            // Open field everywhere until the first recompute.
            ground: vec![1.0; (MAP_WIDTH * MAP_HEIGHT) as usize],
            altitude: vec![1.0; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl WindExposureMap {
    pub fn get(&self, cell: GridCoords, band: WindBand) -> Option<f32> {
        let grid = match band {
            WindBand::Ground => &self.ground,
            WindBand::Altitude => &self.altitude,
        };
        HumidityMap::in_bounds(cell).then(|| grid[(cell.y * MAP_WIDTH + cell.x) as usize])
    }

    /// The wind this cell actually feels at `band`: the ambient wind scaled
    /// by local exposure — the consumer API for fire spread, turbines, and
    /// anything else that cares where the wind reaches. Out of bounds reads
    /// as open field.
    pub fn wind_at(&self, cell: GridCoords, wind: &Wind, band: WindBand) -> Vec2 {
        wind.current_vector() * self.get(cell, band).unwrap_or(1.0)
    }
}

/// The grid cell `k` steps UPWIND of `cell` against `dir` (world XZ, wind
/// blows toward `dir`). Grid +y runs opposite world +z (`grid_to_world`
/// negates y), hence the plus on the y component.
fn upwind_cell(cell: GridCoords, dir: Vec2, k: i32) -> GridCoords {
    GridCoords::new(
        (cell.x as f32 - dir.x * k as f32).round() as i32,
        (cell.y as f32 + dir.y * k as f32).round() as i32,
    )
}

/// Exposure of one cell: march `range` cells upwind and let every blocker
/// found shelter it, stronger the closer it stands (linear falloff — full
/// at distance 1, 1/range at max range). `blocker` maps a cell to shelter
/// strength 0..=1 (0 for open ground and off-map). The result is floored
/// at `WIND_MIN_EXPOSURE`: eddies keep open-air cells breathing.
fn cell_exposure(
    cell: GridCoords,
    dir: Vec2,
    range: i32,
    blocker: impl Fn(GridCoords) -> f32,
) -> f32 {
    let mut shelter = 0.0;
    let mut previous = cell;
    for k in 1..=range {
        let sample = upwind_cell(cell, dir, k);
        // Shallow-diagonal marches round two steps onto the same cell;
        // counting it twice would make shelter angle-dependent.
        if sample == previous {
            continue;
        }
        previous = sample;
        shelter += blocker(sample) * (1.0 - (k - 1) as f32 / range as f32);
    }
    (1.0 - shelter).clamp(WIND_MIN_EXPOSURE, 1.0)
}

/// Shelter strength of what's built on a cell, at `band`. Only solid built
/// blockers stop wind at all (walls, doors, turbine bases, solar-panel
/// posts, battery crates), and every one of them today is well under rotor
/// height, so they shelter `Ground` only — none of them shelter `Altitude`.
/// A future mountain or tall building is the place to add an `Altitude` arm
/// here. Nothing built (including off-map, which reads as `None` too) is
/// open field.
fn terrain_shelter(construction: Option<BuildableKind>, band: WindBand) -> f32 {
    match (construction, band) {
        (
            Some(
                BuildableKind::Wall
                | BuildableKind::Door
                | BuildableKind::Turbine
                | BuildableKind::SolarPanel
                | BuildableKind::Battery,
            ),
            WindBand::Ground,
        ) => WIND_WALL_SHELTER,
        // A lightpost is a slim pole, and a bed is low furniture — neither
        // is a solid blocker, so both read as open field. Nothing built
        // reaches rotor height yet, so `Altitude` is always open field too.
        _ => 0.0,
    }
}

/// Debug-overlay bucket for a cell, keyed on SHELTER (1 - exposure) so the
/// open field draws nothing. Walls hide their ground (no overlay, like the
/// wet overlay); water keeps its shadow — a becalmed river cell is real
/// information. Ground-band only: walls don't shelter altitude, so the
/// altitude overlay never needs the wall special-case (see
/// `sync_altitude_wind_overlay`).
fn wind_overlay_bucket(exposure: f32, construction: Option<BuildableKind>) -> usize {
    if construction == Some(BuildableKind::Wall) {
        return 0;
    }
    let shelter = 1.0 - exposure;
    if shelter < 0.05 {
        0
    } else {
        ((shelter * 4.0).ceil() as usize).min(4)
    }
}

/// Rebuild every cell's exposure from the live wind heading, in both bands.
/// Runs every frame (the wind wanders continuously, so change detection
/// would fire anyway; 1024 cells x 2 bands x 4 samples is trivial),
/// sim-gated and chained after `update_wind` so it reads the same frame's
/// heading. Blockers never shade themselves — the march starts one cell
/// upwind, so a turbine's base can't becalm its own rotor.
pub fn update_wind_exposure(
    wind: Res<Wind>,
    construction: Res<ConstructionMap>,
    roofs: Res<RoofMap>,
    trees: Query<(&Tree, &GridCoords)>,
    mut exposure: ResMut<WindExposureMap>,
) {
    // Trees are the one blocker tall enough to shelter both bands (trunk
    // shelters ground, canopy shelters altitude) — a forest can becalm a
    // turbine even though no built structure can yet.
    let mut tree_shelter = vec![0.0f32; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (tree, grid) in &trees {
        if HumidityMap::in_bounds(*grid) {
            let index = (grid.y * MAP_WIDTH + grid.x) as usize;
            tree_shelter[index] = tree_shelter[index].max(tree.growth * WIND_TREE_SHELTER);
        }
    }
    let tree_shelter_at = |cell: GridCoords| {
        if HumidityMap::in_bounds(cell) {
            tree_shelter[(cell.y * MAP_WIDTH + cell.x) as usize]
        } else {
            0.0
        }
    };
    let ground_blocker = |cell: GridCoords| {
        terrain_shelter(construction.get(cell), WindBand::Ground).max(tree_shelter_at(cell))
    };
    let altitude_blocker = |cell: GridCoords| {
        terrain_shelter(construction.get(cell), WindBand::Altitude).max(tree_shelter_at(cell))
    };
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            let index = (y * MAP_WIDTH + x) as usize;
            let outdoors = is_outdoors(cell, &roofs);
            // Indoors: no wind at all, in either band.
            exposure.ground[index] = if outdoors {
                cell_exposure(cell, wind.direction, WIND_SHELTER_RANGE, ground_blocker)
            } else {
                0.0
            };
            exposure.altitude[index] = if outdoors {
                cell_exposure(cell, wind.direction, WIND_SHELTER_RANGE, altitude_blocker)
            } else {
                0.0
            };
        }
    }
}

/// One humidity step for one cell. A submerged cell (`wet`, from
/// `map::WaterMap::is_wet`) is simply always saturated; land — including a
/// drained riverbed — soaks while rained on and dries back out under a
/// clear sky.
fn next_humidity(level: f32, raining: bool, dt: f32, wet: bool) -> f32 {
    if wet {
        return 1.0;
    }
    if raining {
        (level + HUMIDITY_RAIN_PER_SECOND * dt).min(1.0)
    } else {
        (level - HUMIDITY_DRY_PER_SECOND * dt).max(0.0)
    }
}

/// Wetness bucket for the overlay visual (0 = dry, no overlay; 1..=4 pick
/// a `wet_overlay_materials` shade). The dry floor keeps a barely-damp
/// tile from flashing an overlay.
pub fn humidity_bucket(level: f32) -> usize {
    if level < 0.05 {
        0
    } else {
        ((level * 4.0).ceil() as usize).min(4)
    }
}

/// Overlay bucket for a cell: standing water reads wet already, walls hide
/// the ground, and settled snow whites out the dark sheen — none of them
/// get an overlay tile (their humidity still tracks). A drained bed is
/// ordinary ground and shows its dampness.
fn overlay_bucket(
    level: f32,
    wet: bool,
    construction: Option<BuildableKind>,
    snow_level: f32,
) -> usize {
    if wet || construction == Some(BuildableKind::Wall) || snow_level >= SNOW_SUPPRESS_WET_MIN {
        0
    } else {
        humidity_bucket(level)
    }
}

/// Advance every cell's humidity. Sim-gated: pausing freezes the soak/dry
/// like everything else. Falling snow does NOT wet the ground — the water
/// is banked in the `SnowMap` — but a cell actively melting its snow soaks
/// as if rained on, releasing it back.
pub fn update_humidity(
    time: Res<Time>,
    weather: Res<Weather>,
    water: Res<map::WaterMap>,
    roofs: Res<RoofMap>,
    clock: Res<crate::daynight::GameClock>,
    season: Res<crate::temperature::ActiveSeason>,
    snow: Res<crate::snow::SnowMap>,
    temps: Res<crate::temperature::TemperatureMap>,
    mut humidity: ResMut<HumidityMap>,
) {
    let dt = time.delta_secs();
    let ambient = crate::temperature::ambient_temperature(season.0, clock.elapsed);
    let rain_falls = weather.rain && precipitation_form(ambient) == Precipitation::Rain;
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            let melting =
                snow.get(cell).unwrap_or(0.0) > 0.05 && temps.get(cell).unwrap_or(ambient) > 0.0;
            let raining = (rain_falls && is_outdoors(cell, &roofs)) || melting;
            let index = (y * MAP_WIDTH + x) as usize;
            humidity.levels[index] =
                next_humidity(humidity.levels[index], raining, dt, water.is_wet(cell));
        }
    }
}

/// A translucent darkening tile over one wet cell; the entity IS the
/// overlay (the `Flora` model).
#[derive(Component)]
pub struct WetOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Keep one overlay tile per wet cell: spawn on first wetness, swap the
/// material shade on bucket change, despawn once dry.
pub fn sync_humidity_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    humidity: Res<HumidityMap>,
    water: Res<map::WaterMap>,
    construction: Res<ConstructionMap>,
    snow: Res<crate::snow::SnowMap>,
    mut overlays: Query<(
        Entity,
        &mut WetOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let mut has_overlay = vec![false; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (entity, mut overlay, mut material) in &mut overlays {
        let cell = overlay.cell;
        has_overlay[(cell.y * MAP_WIDTH + cell.x) as usize] = true;
        let bucket = overlay_bucket(
            humidity.get(cell).unwrap_or(0.0),
            water.is_wet(cell),
            construction.get(cell),
            snow.get(cell).unwrap_or(0.0),
        );
        if bucket == 0 {
            commands.entity(entity).despawn();
        } else if bucket != overlay.bucket {
            overlay.bucket = bucket;
            material.0 = assets.wet_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let index = (y * MAP_WIDTH + x) as usize;
            if has_overlay[index] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = overlay_bucket(
                humidity.levels[index],
                water.is_wet(cell),
                construction.get(cell),
                snow.get(cell).unwrap_or(0.0),
            );
            if bucket == 0 {
                continue;
            }
            let mask = map::decal_shore_mask(cell, &water);
            commands.spawn((
                Mesh3d(assets.cover_shore_meshes[mask as usize].clone()),
                MeshMaterial3d(assets.wet_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * WET_OVERLAY_OFFSET,
                ),
                Name::new("WetOverlay"),
                WetOverlay { cell, bucket },
            ));
        }
    }
}

/// Devtool switch for the wind-shadow debug overlay (Weather debug tab).
#[derive(Resource, Default)]
pub struct ShowWindOverlay(pub bool);

/// A translucent slate tile over one sheltered cell while the wind overlay
/// is on — the `WetOverlay` model: the entity IS the visual.
#[derive(Component)]
pub struct WindShadowOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Keep one overlay tile per sheltered cell while the devtool is on: spawn
/// on first shelter, swap the shade on bucket change, despawn when the
/// shelter fades or the overlay is toggled off (a forced bucket 0 rides
/// the same reconciler).
pub fn sync_wind_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowWindOverlay>,
    exposure: Res<WindExposureMap>,
    construction: Res<ConstructionMap>,
    mut overlays: Query<(
        Entity,
        &mut WindShadowOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let desired = |cell: GridCoords| -> usize {
        if !show.0 {
            return 0;
        }
        wind_overlay_bucket(
            exposure.get(cell, WindBand::Ground).unwrap_or(1.0),
            construction.get(cell),
        )
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
            material.0 = assets.wind_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let index = (y * MAP_WIDTH + x) as usize;
            if has_overlay[index] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = desired(cell);
            if bucket == 0 {
                continue;
            }
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(assets.wind_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * WIND_OVERLAY_OFFSET,
                ),
                Name::new("WindShadowOverlay"),
                WindShadowOverlay { cell, bucket },
            ));
        }
    }
}

/// Devtool switch for the altitude-wind debug overlay (Weather debug tab) —
/// the `ShowWindOverlay` model, one band over.
#[derive(Resource, Default)]
pub struct ShowAltitudeWindOverlay(pub bool);

/// A translucent tile over one altitude-sheltered cell while the altitude
/// wind overlay is on — the `WindShadowOverlay` model. No wall special-case:
/// walls don't shelter altitude, so a walled cell draws like open field
/// unless a tree looms over it.
#[derive(Component)]
pub struct AltitudeWindOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Altitude-band twin of `sync_wind_overlay`: same reconciler shape, reads
/// `WindBand::Altitude` and its own material set/offset so the two overlays
/// can run together without z-fighting or visual confusion.
pub fn sync_altitude_wind_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowAltitudeWindOverlay>,
    exposure: Res<WindExposureMap>,
    mut overlays: Query<(
        Entity,
        &mut AltitudeWindOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let desired = |cell: GridCoords| -> usize {
        if !show.0 {
            return 0;
        }
        let shelter = 1.0 - exposure.get(cell, WindBand::Altitude).unwrap_or(1.0);
        if shelter < 0.05 {
            0
        } else {
            ((shelter * 4.0).ceil() as usize).min(4)
        }
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
            material.0 = assets.altitude_wind_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let index = (y * MAP_WIDTH + x) as usize;
            if has_overlay[index] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = desired(cell);
            if bucket == 0 {
                continue;
            }
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(assets.altitude_wind_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * WIND_ALTITUDE_OVERLAY_OFFSET,
                ),
                Name::new("AltitudeWindOverlay"),
                AltitudeWindOverlay { cell, bucket },
            ));
        }
    }
}

/// Devtool switch for the fuel debug overlay (Weather debug tab).
#[derive(Resource, Default)]
pub struct ShowFuelOverlay(pub bool);

/// A translucent amber tile over one fuel-bearing cell while the fuel
/// overlay is on — the `WetOverlay` model: the entity IS the visual.
#[derive(Component)]
pub struct FuelOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Overlay bucket for a cell's fuel. No terrain param needed: `fuel_at`
/// already zeroes inert ground (walls, water, stockpiles), so those cells
/// draw nothing for free.
fn fuel_overlay_bucket(fuel: f32) -> usize {
    if fuel < 0.05 {
        0
    } else {
        ((fuel * 4.0).ceil() as usize).min(4)
    }
}

/// Keep one overlay tile per fuel-bearing cell while the devtool is on —
/// the wind-overlay reconciler with `cover::fuel_at` as the signal. Plant
/// queries bundled into one tuple param to stay inside the system-param
/// arity limit.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sync_fuel_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowFuelOverlay>,
    terrain: Res<TerrainMap>,
    construction: Res<ConstructionMap>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    plant_queries: (
        Query<(&Tree, &GridCoords)>,
        Query<(&BerryBush, &GridCoords)>,
        Query<(&Crop, &GridCoords)>,
        Query<(&Shrub, &GridCoords)>,
    ),
    mut overlays: Query<(
        Entity,
        &mut FuelOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let (trees, bushes, crops, shrubs) = plant_queries;
    // Only fold the plant queries while the overlay is actually shown.
    let plant_fuel = if show.0 {
        cover::collect_plant_fuel(
            trees
                .iter()
                .map(|(tree, grid)| (*grid, tree.fuel()))
                .chain(bushes.iter().map(|(bush, grid)| (*grid, bush.fuel())))
                .chain(crops.iter().map(|(crop, grid)| (*grid, crop.fuel())))
                .chain(shrubs.iter().map(|(shrub, grid)| (*grid, shrub.fuel()))),
        )
    } else {
        Vec::new()
    };
    let desired = |cell: GridCoords| -> usize {
        if !show.0 {
            return 0;
        }
        let plant = plant_fuel[(cell.y * MAP_WIDTH + cell.x) as usize];
        fuel_overlay_bucket(cover::fuel_at(
            cell,
            &terrain,
            &construction,
            &cover,
            &water,
            plant,
        ))
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
            material.0 = assets.fuel_overlay_materials[bucket - 1].clone();
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let index = (y * MAP_WIDTH + x) as usize;
            if has_overlay[index] {
                continue;
            }
            let cell = GridCoords::new(x, y);
            let bucket = desired(cell);
            if bucket == 0 {
                continue;
            }
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(assets.fuel_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * FUEL_OVERLAY_OFFSET,
                ),
                Name::new("FuelOverlay"),
                FuelOverlay { cell, bucket },
            ));
        }
    }
}

/// The rain particle effect entity.
#[derive(Component)]
pub struct RainEffect;

/// The snowfall particle effect entity.
#[derive(Component)]
pub struct SnowEffect;

/// One falling-precipitation `EffectAsset`: particles spawn uniformly over
/// the map footprint at `RAIN_SPAWN_HEIGHT`, fall on a "wind_velocity"
/// property (rewritten live by `sync_rain_wind`, so only the fall speed and
/// look differ per form), and die just under ground level (lifetime = fall
/// time; no collision needed).
fn falling_effect(
    effects: &mut Assets<EffectAsset>,
    name: &str,
    capacity: u32,
    spawn_rate: f32,
    fall_speed: f32,
    orient: OrientMode,
    size: Vec3,
    color: Vec4,
) -> Handle<EffectAsset> {
    let spawner = SpawnerSettings::rate(spawn_rate.into()).with_starts_active(false);

    let writer = ExprWriter::new();
    let half_w = MAP_WIDTH as f32 * TILE_SIZE / 2.0;
    let half_h = MAP_HEIGHT as f32 * TILE_SIZE / 2.0;
    let init_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        writer
            .lit(Vec3::new(-half_w, RAIN_SPAWN_HEIGHT, -half_h))
            .uniform(writer.lit(Vec3::new(half_w, RAIN_SPAWN_HEIGHT, half_h)))
            .expr(),
    );
    // Velocity is a property, not a literal: `sync_rain_wind` rewrites it as
    // the wind gusts, so the fall slants with the live wind. Only newly
    // spawned particles read it, but the lag is invisible.
    let calm = Wind::default().current_vector();
    let wind_vel = writer.add_property(
        "wind_velocity",
        Value::Vector(Vec3::new(calm.x, -fall_speed, calm.y).into()),
    );
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, writer.prop(wind_vel).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // A hair past the ground so particles end hidden inside the tiles, not
    // frozen mid-air.
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer
            .lit((RAIN_SPAWN_HEIGHT + TILE_THICKNESS) / fall_speed)
            .expr(),
    );
    let module = writer.finish();

    effects.add(
        EffectAsset::new(capacity, spawner, module)
            .with_name(name)
            .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
            .init(init_pos)
            .init(init_vel)
            .init(init_age)
            .init(init_lifetime)
            .render(OrientModifier::new(orient))
            .render(SetSizeModifier { size: size.into() })
            .render(SetColorModifier::new(color)),
    )
}

/// Author both precipitation effects and spawn their (initially inactive)
/// entities. Rain streaks along its velocity (`OrientMode::AlongVelocity` —
/// dots become streaks); snow is small billboarded flakes drifting down at
/// a fifth of the speed. Both authored here at startup: Hanabi effects
/// can't be created at runtime (see `FIRE_EMITTER_POOL`).
pub fn setup(mut commands: Commands, mut effects: ResMut<Assets<EffectAsset>>) {
    let rain = falling_effect(
        &mut effects,
        "rain",
        RAIN_PARTICLE_CAPACITY,
        RAIN_SPAWN_RATE,
        RAIN_FALL_SPEED,
        OrientMode::AlongVelocity,
        Vec3::new(RAIN_DROP_LENGTH, RAIN_DROP_WIDTH, 1.0),
        RAIN_COLOR,
    );
    commands.spawn((
        ParticleEffect::new(rain),
        EffectProperties::default(),
        Transform::default(),
        Name::new("RainEffect"),
        RainEffect,
    ));

    let snow = falling_effect(
        &mut effects,
        "snow",
        SNOW_PARTICLE_CAPACITY,
        SNOW_SPAWN_RATE,
        SNOW_FALL_SPEED,
        OrientMode::FaceCameraPosition,
        Vec3::new(SNOW_FLAKE_SIZE, SNOW_FLAKE_SIZE, 1.0),
        SNOW_COLOR,
    );
    commands.spawn((
        ParticleEffect::new(snow),
        EffectProperties::default(),
        Transform::default(),
        Name::new("SnowEffect"),
        SnowEffect,
    ));
}

/// One gust "snake": the entity is a Hanabi emitter head that a system
/// glides downwind; the trail behind it is the ribbon its stationary
/// particles form.
#[derive(Component)]
pub struct GustHead {
    /// Serpentine phase offset so heads don't wiggle in unison.
    phase: f32,
    /// Wrap counter, doubling as the current ribbon id: bumping it on wrap
    /// severs the trail, otherwise the ribbon would draw one giant quad
    /// from the exit point to the re-entry point.
    wraps: u32,
    /// xorshift32 state for re-entry placement.
    rng: u32,
}

/// Tiny xorshift32 step (the director's wander RNG pattern), returning
/// 0..1.
fn gust_rand(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Where a wrapped gust head re-enters: a point just outside the upwind
/// boundary, on the x- or z-edge picked proportionally to the wind's
/// components (`pick`), spread along that edge (`along`), so entries cover
/// the whole upwind side.
fn gust_entry_point(dir: Vec2, pick: f32, along: f32) -> Vec2 {
    let half_w = MAP_WIDTH as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
    let half_h = MAP_HEIGHT as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
    let wx = dir.x.abs();
    let wz = dir.y.abs();
    if pick * (wx + wz) < wx {
        Vec2::new(
            -dir.x.signum() * half_w,
            (along * 2.0 - 1.0) * (half_h - GUST_EDGE_MARGIN),
        )
    } else {
        Vec2::new(
            (along * 2.0 - 1.0) * (half_w - GUST_EDGE_MARGIN),
            -dir.y.signum() * half_h,
        )
    }
}

/// Author the shared gust-ribbon `EffectAsset` and spawn `GUST_COUNT` head
/// entities scattered over the map. The Hanabi ribbon trick (its ribbon
/// example): particles spawn at the moving emitter and never move
/// (`MotionIntegration::None`, global space); consecutive particles of one
/// `RIBBON_ID` render as a connected strip, so the head's path *is* the
/// trail, fading and tapering toward the tail.
pub fn setup_gusts(mut commands: Commands, mut effects: ResMut<Assets<EffectAsset>>) {
    let spawner = SpawnerSettings::rate(GUST_RIBBON_SPAWN_RATE.into());

    let writer = ExprWriter::new();
    let ribbon_id = writer.add_property("ribbon_id", Value::Scalar(0u32.into()));
    let init_pos = SetAttributeModifier::new(Attribute::POSITION, writer.lit(Vec3::ZERO).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Constant lifetime keeps deaths strictly at the tail end — a particle
    // dying mid-ribbon would visually reorder the strip.
    let init_lifetime =
        SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(GUST_RIBBON_LIFETIME).expr());
    let init_ribbon_id =
        SetAttributeModifier::new(Attribute::RIBBON_ID, writer.prop(ribbon_id).expr());
    let module = writer.finish();

    // Head-bright to tail-invisible. Explicit path: the Bevy prelude has a
    // UI `Gradient` of the same name.
    let gradient = bevy_hanabi::Gradient::linear(GUST_COLOR, GUST_COLOR.with_w(0.0));

    let effect = effects.add(
        EffectAsset::new(GUST_PARTICLE_CAPACITY, spawner, module)
            .with_name("gusts")
            .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
            // Trail particles stay where the head dropped them...
            .with_motion_integration(MotionIntegration::None)
            // ...in world space, even as the emitter keeps moving.
            .with_simulation_space(SimulationSpace::Global)
            .init(init_pos)
            .init(init_age)
            .init(init_lifetime)
            .init(init_ribbon_id)
            // The gradient OVERWRITES the size each frame (it doesn't scale
            // an init value), so the head width lives here.
            .render(SizeOverLifetimeModifier {
                gradient: bevy_hanabi::Gradient::linear(Vec3::splat(GUST_RIBBON_WIDTH), Vec3::ZERO),
                ..default()
            })
            .render(ColorOverLifetimeModifier::new(gradient)),
    );

    for i in 0..GUST_COUNT {
        let mut rng = 0x9E37_79B9u32 ^ (i as u32).wrapping_mul(2_654_435_761);
        let half_w = MAP_WIDTH as f32 * TILE_SIZE / 2.0;
        let half_h = MAP_HEIGHT as f32 * TILE_SIZE / 2.0;
        let position = Vec3::new(
            (gust_rand(&mut rng) * 2.0 - 1.0) * half_w,
            GUST_BAND_MIN + gust_rand(&mut rng) * (GUST_BAND_MAX - GUST_BAND_MIN),
            (gust_rand(&mut rng) * 2.0 - 1.0) * half_h,
        );
        commands.spawn((
            ParticleEffect::new(effect.clone()),
            EffectProperties::default(),
            Transform::from_translation(position),
            Name::new(format!("GustHead {i}")),
            GustHead {
                phase: std::f32::consts::TAU * i as f32 / GUST_COUNT as f32,
                wraps: 0,
                rng,
            },
        ));
    }
}

/// Glide every gust head downwind with a flat serpentine sway (fixed
/// height — trails stay ground-parallel), wrapping back to the upwind edge
/// once it leaves the map. Sim-gated: pause freezes the heads, and the
/// paused `Time<EffectSimulation>` freezes their trails.
pub fn move_gust_heads(
    time: Res<Time>,
    wind: Res<Wind>,
    mut heads: Query<(&mut Transform, &mut GustHead, &mut EffectProperties)>,
) {
    use std::f32::consts::TAU;
    let dir = wind.direction;
    let perp = Vec2::new(-dir.y, dir.x);
    let speed = wind.current * GUST_SPEED_FACTOR;
    let half_w = MAP_WIDTH as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
    let half_h = MAP_HEIGHT as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
    for (mut transform, mut head, properties) in &mut heads {
        let sway =
            GUST_WIGGLE_SPEED * (TAU * GUST_WIGGLE_FREQUENCY * wind.elapsed + head.phase).sin();
        let velocity = dir * speed + perp * sway;
        transform.translation.x += velocity.x * time.delta_secs();
        transform.translation.z += velocity.y * time.delta_secs();

        let out_of_bounds =
            transform.translation.x.abs() > half_w || transform.translation.z.abs() > half_h;
        if out_of_bounds {
            head.wraps += 1;
            let entry = gust_entry_point(dir, gust_rand(&mut head.rng), gust_rand(&mut head.rng));
            transform.translation = Vec3::new(
                entry.x,
                GUST_BAND_MIN + gust_rand(&mut head.rng) * (GUST_BAND_MAX - GUST_BAND_MIN),
                entry.y,
            );
            // New ribbon id = the old trail is severed and fades out where
            // it is, instead of connecting across the map.
            EffectProperties::set_if_changed(
                properties,
                "ribbon_id",
                Value::Scalar(head.wraps.into()),
            );
        }
    }
}

/// Keep both precipitation effects' lateral velocity on the live wind.
/// Ungated like `sync_precipitation_effects`; `set_if_changed` compares
/// values, so a frozen wind doesn't re-upload anything. The `Without`
/// filters keep the two mutable queries provably disjoint.
pub fn sync_rain_wind(
    wind: Res<Wind>,
    mut rain: Query<&mut EffectProperties, (With<RainEffect>, Without<SnowEffect>)>,
    mut snow: Query<&mut EffectProperties, (With<SnowEffect>, Without<RainEffect>)>,
) {
    let v = wind.current_vector();
    for properties in &mut rain {
        let velocity = Vec3::new(v.x, -RAIN_FALL_SPEED, v.y);
        EffectProperties::set_if_changed(properties, "wind_velocity", velocity.into());
    }
    for properties in &mut snow {
        let velocity = Vec3::new(v.x, -SNOW_FALL_SPEED, v.y);
        EffectProperties::set_if_changed(properties, "wind_velocity", velocity.into());
    }
}

/// Keep the spawners following `Weather.rain`, with the ambient picking
/// which form falls: exactly one of rain/snow runs while precipitation is
/// on. Polled (not change-driven): Hanabi only inserts `EffectSpawner` a
/// frame after the effect spawns.
pub fn sync_precipitation_effects(
    weather: Res<Weather>,
    clock: Res<crate::daynight::GameClock>,
    season: Res<crate::temperature::ActiveSeason>,
    mut rain: Query<&mut EffectSpawner, (With<RainEffect>, Without<SnowEffect>)>,
    mut snow: Query<&mut EffectSpawner, (With<SnowEffect>, Without<RainEffect>)>,
) {
    let ambient = crate::temperature::ambient_temperature(season.0, clock.elapsed);
    let form = precipitation_form(ambient);
    let rain_on = weather.rain && form == Precipitation::Rain;
    let snow_on = weather.rain && form == Precipitation::Snow;
    for mut spawner in &mut rain {
        if spawner.active != rain_on {
            spawner.active = rain_on;
        }
    }
    for mut spawner in &mut snow {
        if spawner.active != snow_on {
            spawner.active = snow_on;
        }
    }
}

/// Freeze airborne drops with the rest of the sim (Hanabi runs on its own
/// virtual clock, so Space wouldn't touch it otherwise).
pub fn pause_rain(mut time: ResMut<Time<EffectSimulation>>) {
    time.pause();
}

pub fn resume_rain(mut time: ResMut<Time<EffectSimulation>>) {
    time.unpause();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humidity_rises_in_rain_and_saturates() {
        let wetter = next_humidity(0.5, true, 1.0, false);
        assert!((wetter - (0.5 + HUMIDITY_RAIN_PER_SECOND)).abs() < 1e-6);
        assert_eq!(next_humidity(0.999, true, 60.0, false), 1.0);
    }

    #[test]
    fn humidity_dries_when_clear_and_bottoms_out() {
        let drier = next_humidity(0.5, false, 1.0, false);
        assert!((drier - (0.5 - HUMIDITY_DRY_PER_SECOND)).abs() < 1e-6);
        assert_eq!(next_humidity(0.001, false, 60.0, false), 0.0);
    }

    #[test]
    fn submerged_cells_are_always_saturated() {
        // Rain or shine, standing water pins humidity; a drained bed
        // (wet = false) dries like any other ground.
        assert_eq!(next_humidity(0.2, false, 1.0, true), 1.0);
        assert_eq!(next_humidity(0.2, true, 1.0, true), 1.0);
    }

    #[test]
    fn humidity_buckets_quantize_with_a_dry_floor() {
        assert_eq!(humidity_bucket(0.0), 0);
        assert_eq!(humidity_bucket(0.04), 0);
        assert_eq!(humidity_bucket(0.05), 1);
        assert_eq!(humidity_bucket(0.25), 1);
        assert_eq!(humidity_bucket(0.3), 2);
        assert_eq!(humidity_bucket(0.6), 3);
        assert_eq!(humidity_bucket(0.8), 4);
        assert_eq!(humidity_bucket(1.0), 4);
    }

    #[test]
    fn roofed_cells_are_indoors() {
        let mut roofs = RoofMap::default();
        let cell = GridCoords::new(3, 3);
        assert!(is_outdoors(cell, &roofs));
        roofs.set_roofed(cell, true);
        assert!(!is_outdoors(cell, &roofs));
        // Neighbors stay under the sky.
        assert!(is_outdoors(GridCoords::new(3, 4), &roofs));
    }

    #[test]
    fn gusts_start_neutral_and_stay_bounded() {
        assert_eq!(gust_multiplier(0.0), 1.0);
        let bound = WIND_GUST_AMPLITUDE_PRIMARY + WIND_GUST_AMPLITUDE_SECONDARY + 1e-5;
        for i in 0..10_000 {
            let g = gust_multiplier(i as f32 * 0.1);
            assert!((1.0 - bound..=1.0 + bound).contains(&g), "gust {g} at {i}");
        }
    }

    #[test]
    fn default_wind_blows_at_base_strength() {
        let wind = Wind::default();
        assert!((wind.direction.length() - 1.0).abs() < 1e-6);
        assert!((wind.current_vector().length() - WIND_BASE_STRENGTH).abs() < 1e-5);
        // The heading reproduces the configured base direction.
        assert!((Vec2::from_angle(wind.heading) - WIND_DIRECTION.normalize()).length() < 1e-6);
    }

    #[test]
    fn wander_starts_neutral_and_stays_bounded() {
        assert_eq!(wander_offset(0.0), 0.0);
        let bound = WIND_WANDER_PRIMARY + WIND_WANDER_SECONDARY + 1e-5;
        for i in 0..10_000 {
            let w = wander_offset(i as f32);
            assert!((-bound..=bound).contains(&w), "wander {w} at {i}");
        }
    }

    #[test]
    fn turning_clamps_to_the_max_step_and_lands_exactly() {
        use std::f32::consts::PI;
        // Far away: move by exactly max_step, in the right direction.
        assert!((turn_toward(0.0, 1.0, 0.1) - 0.1).abs() < 1e-6);
        assert!((turn_toward(1.0, 0.0, 0.1) - 0.9).abs() < 1e-6);
        // Close: land on the target, no overshoot.
        assert!((turn_toward(0.95, 1.0, 0.1) - 1.0).abs() < 1e-6);
        // Repeated steps converge.
        let mut heading = 0.0;
        for _ in 0..100 {
            heading = turn_toward(heading, PI / 2.0, 0.05);
        }
        assert!((heading - PI / 2.0).abs() < 1e-4);
    }

    #[test]
    fn turning_takes_the_shortest_way_around_the_seam() {
        use std::f32::consts::PI;
        // 3.0 -> -3.0 is 0.28 rad across the +-PI seam, not 6.0 back
        // through zero: the heading must INCREASE toward PI.
        let stepped = turn_toward(3.0, -3.0, 0.1);
        assert!(stepped > 3.0, "went the long way: {stepped}");
        // And symmetrically the other way.
        let stepped = turn_toward(-3.0, 3.0, 0.1);
        assert!(stepped < -3.0, "went the long way: {stepped}");
        // Whole-turn offsets in the desired angle are irrelevant.
        let a = turn_toward(0.5, 1.0, 10.0);
        let b = turn_toward(0.5, 1.0 + 2.0 * PI, 10.0);
        assert!((a - b).abs() < 1e-5);
    }

    #[test]
    fn upwind_march_flips_grid_y() {
        // Wind toward world +X: upwind samples walk grid -x, same y.
        assert_eq!(
            upwind_cell(GridCoords::new(10, 10), Vec2::new(1.0, 0.0), 2),
            GridCoords::new(8, 10)
        );
        // Wind toward world +Z: grid +y is world -Z, so upwind walks +y.
        assert_eq!(
            upwind_cell(GridCoords::new(10, 10), Vec2::new(0.0, 1.0), 2),
            GridCoords::new(10, 12)
        );
    }

    #[test]
    fn open_field_is_fully_exposed() {
        let exposure = cell_exposure(
            GridCoords::new(10, 10),
            Vec2::new(1.0, 0.0),
            WIND_SHELTER_RANGE,
            |_| 0.0,
        );
        assert_eq!(exposure, 1.0);
    }

    #[test]
    fn adjacent_wall_pins_exposure_to_the_floor() {
        let wall = GridCoords::new(9, 10);
        let exposure = cell_exposure(
            GridCoords::new(10, 10),
            Vec2::new(1.0, 0.0),
            WIND_SHELTER_RANGE,
            |c| if c == wall { 1.0 } else { 0.0 },
        );
        assert_eq!(exposure, WIND_MIN_EXPOSURE);
    }

    #[test]
    fn shelter_falls_off_with_distance() {
        let dir = Vec2::new(1.0, 0.0);
        let at_distance = |d: i32| {
            let wall = GridCoords::new(10 - d, 10);
            cell_exposure(GridCoords::new(10, 10), dir, WIND_SHELTER_RANGE, |c| {
                if c == wall {
                    1.0
                } else {
                    0.0
                }
            })
        };
        // Farthest sample carries 1/range of the shelter.
        assert!((at_distance(4) - 0.75).abs() < 1e-6);
        // Monotonic: closer blockers shelter at least as much.
        assert!(at_distance(1) <= at_distance(2));
        assert!(at_distance(2) <= at_distance(3));
        assert!(at_distance(3) <= at_distance(4));
    }

    #[test]
    fn shadows_fall_downwind_not_upwind() {
        // Blocker DOWNWIND of the cell (wind has already passed it).
        let wall = GridCoords::new(11, 10);
        let exposure = cell_exposure(
            GridCoords::new(10, 10),
            Vec2::new(1.0, 0.0),
            WIND_SHELTER_RANGE,
            |c| if c == wall { 1.0 } else { 0.0 },
        );
        assert_eq!(exposure, 1.0);
    }

    #[test]
    fn stacked_blockers_clamp_at_the_floor() {
        let exposure = cell_exposure(
            GridCoords::new(10, 10),
            Vec2::new(1.0, 0.0),
            WIND_SHELTER_RANGE,
            |_| 1.0,
        );
        assert_eq!(exposure, WIND_MIN_EXPOSURE);
    }

    #[test]
    fn diagonal_rounding_duplicates_count_once() {
        // At 45 degrees, k=1 and k=2 can round onto the same neighbor; the
        // dedupe must count it once (k=1 weight only).
        let dir = Vec2::splat(1.0).normalize();
        let cell = GridCoords::new(10, 10);
        let first = upwind_cell(cell, dir, 1);
        assert_eq!(first, upwind_cell(cell, dir, 2), "test premise");
        let exposure = cell_exposure(cell, dir, WIND_SHELTER_RANGE, |c| {
            if c == first {
                1.0
            } else {
                0.0
            }
        });
        assert_eq!(exposure, WIND_MIN_EXPOSURE); // k=1 weight 1.0, once
    }

    #[test]
    fn only_solid_built_terrain_shelters() {
        for solid in [
            BuildableKind::Wall,
            BuildableKind::Door,
            BuildableKind::Turbine,
            BuildableKind::SolarPanel,
        ] {
            assert_eq!(
                terrain_shelter(Some(solid), WindBand::Ground),
                WIND_WALL_SHELTER
            );
        }
        // Nothing built (including off-map, which also reads `None`).
        assert_eq!(terrain_shelter(None, WindBand::Ground), 0.0);
    }

    #[test]
    fn no_built_structure_shelters_altitude() {
        // Every buildable today is well under rotor height — none of them
        // should shelter the altitude band, only ground.
        for solid in [
            BuildableKind::Wall,
            BuildableKind::Door,
            BuildableKind::Turbine,
            BuildableKind::SolarPanel,
            BuildableKind::Battery,
        ] {
            assert_eq!(terrain_shelter(Some(solid), WindBand::Altitude), 0.0);
        }
    }

    #[test]
    fn wind_overlay_buckets_skip_walls_and_open_field() {
        assert_eq!(wind_overlay_bucket(1.0, None), 0);
        assert_eq!(wind_overlay_bucket(0.97, None), 0); // under the floor
        assert_eq!(wind_overlay_bucket(0.5, None), 2);
        assert_eq!(wind_overlay_bucket(0.0, None), 4);
        // Walls hide their ground.
        assert_eq!(wind_overlay_bucket(0.2, Some(BuildableKind::Wall)), 0);
    }

    #[test]
    fn default_exposure_is_open_and_wind_at_scales() {
        let map = WindExposureMap::default();
        assert_eq!(map.get(GridCoords::new(5, 5), WindBand::Ground), Some(1.0));
        assert_eq!(
            map.get(GridCoords::new(5, 5), WindBand::Altitude),
            Some(1.0)
        );
        assert_eq!(map.get(GridCoords::new(-1, 5), WindBand::Ground), None);
        let wind = Wind::default();
        assert_eq!(
            map.wind_at(GridCoords::new(5, 5), &wind, WindBand::Ground),
            wind.current_vector()
        );
        // Out of bounds reads as open field, not dead air.
        assert_eq!(
            map.wind_at(GridCoords::new(-1, 5), &wind, WindBand::Altitude),
            wind.current_vector()
        );
    }

    #[test]
    fn wall_shelters_ground_but_not_altitude() {
        let mut construction = ConstructionMap::default();
        let wall = GridCoords::new(9, 10);
        construction.set(wall, Some(BuildableKind::Wall));
        let cell = GridCoords::new(10, 10);
        let dir = Vec2::new(1.0, 0.0);

        let ground = cell_exposure(cell, dir, WIND_SHELTER_RANGE, |c| {
            terrain_shelter(construction.get(c), WindBand::Ground)
        });
        assert_eq!(ground, WIND_MIN_EXPOSURE);

        let altitude = cell_exposure(cell, dir, WIND_SHELTER_RANGE, |c| {
            terrain_shelter(construction.get(c), WindBand::Altitude)
        });
        assert_eq!(altitude, 1.0);
    }

    #[test]
    fn full_grown_tree_shelters_both_bands() {
        let tree_shelter = WIND_TREE_SHELTER; // full growth
        let cell = GridCoords::new(10, 10);
        let dir = Vec2::new(1.0, 0.0);
        let tree_cell = GridCoords::new(9, 10);
        let blocker = |c: GridCoords| if c == tree_cell { tree_shelter } else { 0.0 };

        let ground = cell_exposure(cell, dir, WIND_SHELTER_RANGE, blocker);
        let altitude = cell_exposure(cell, dir, WIND_SHELTER_RANGE, blocker);
        assert_eq!(ground, 1.0 - tree_shelter);
        assert_eq!(altitude, 1.0 - tree_shelter);
    }

    #[test]
    fn fuel_overlay_buckets_quantize_with_a_floor() {
        assert_eq!(fuel_overlay_bucket(0.0), 0);
        assert_eq!(fuel_overlay_bucket(0.04), 0);
        assert_eq!(fuel_overlay_bucket(0.05), 1);
        assert_eq!(fuel_overlay_bucket(0.6), 3);
        assert_eq!(fuel_overlay_bucket(1.0), 4);
    }

    #[test]
    fn gust_ribbons_never_outgrow_their_particle_capacity() {
        // Alive trail particles per head ~= spawn rate x lifetime.
        let alive = GUST_RIBBON_SPAWN_RATE * GUST_RIBBON_LIFETIME;
        assert!(
            alive < GUST_PARTICLE_CAPACITY as f32,
            "trail of {alive} particles exceeds capacity {GUST_PARTICLE_CAPACITY}"
        );
    }

    #[test]
    fn gust_entry_is_upwind_and_on_the_extended_boundary() {
        let half_w = MAP_WIDTH as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
        let half_h = MAP_HEIGHT as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
        let dir = WIND_DIRECTION.normalize();
        for pick in [0.0, 0.3, 0.7, 0.99] {
            for along in [0.0, 0.5, 1.0] {
                let entry = gust_entry_point(dir, pick, along);
                // Whichever edge was picked, it must be the upwind one —
                // the head drifts back over the map, not straight out.
                let on_upwind_x_edge = (entry.x + dir.x.signum() * half_w).abs() < 1e-5;
                let on_upwind_z_edge = (entry.y + dir.y.signum() * half_h).abs() < 1e-5;
                assert!(
                    on_upwind_x_edge || on_upwind_z_edge,
                    "entry {entry} not on an upwind boundary"
                );
            }
        }
    }

    #[test]
    fn gust_entry_uses_the_only_edge_an_axis_aligned_wind_can_enter_from() {
        let half_w = MAP_WIDTH as f32 * TILE_SIZE / 2.0 + GUST_EDGE_MARGIN;
        let entry = gust_entry_point(Vec2::new(-1.0, 0.0), 0.5, 0.5);
        assert!((entry.x - half_w).abs() < 1e-5, "entry {entry}");
    }

    #[test]
    fn heavy_wind_raises_the_base() {
        let wind = Wind {
            heavy: true,
            ..default()
        };
        assert_eq!(wind.base_strength(), WIND_HEAVY_STRENGTH);
    }

    #[test]
    fn no_overlay_on_water_or_walls() {
        assert_eq!(overlay_bucket(1.0, true, None, 0.0), 0);
        assert_eq!(
            overlay_bucket(1.0, false, Some(BuildableKind::Wall), 0.0),
            0
        );
        // Dry ground — a drained riverbed included — shows its dampness.
        assert_eq!(overlay_bucket(1.0, false, None, 0.0), 4);
    }

    #[test]
    fn settled_snow_hides_the_wet_sheen() {
        assert_eq!(overlay_bucket(1.0, false, None, SNOW_SUPPRESS_WET_MIN), 0);
        // A light dusting doesn't.
        assert_eq!(overlay_bucket(1.0, false, None, 0.1), 4);
    }

    #[test]
    fn freezing_air_turns_rain_to_snow() {
        assert_eq!(precipitation_form(0.0), Precipitation::Snow);
        assert_eq!(precipitation_form(-20.0), Precipitation::Snow);
        assert_eq!(precipitation_form(0.1), Precipitation::Rain);
        assert_eq!(precipitation_form(12.0), Precipitation::Rain);
    }
}
