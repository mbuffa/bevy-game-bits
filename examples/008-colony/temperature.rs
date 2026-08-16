//! Seasonal temperature: an `ActiveSeason` resource and the pure ambient
//! formula mapping sim time to an outdoor °C. The day warms along a sine
//! hump (night base at sunrise, seasonal peak at noon, back to base by
//! sunset) so the sun's heat builds and fades gradually; the night sits flat
//! at the base. `snow.rs` consumes temperature (melt rate, freeze level) and
//! `weather.rs` derives the precipitation form (rain vs snow) from ambient.
//! `fire.rs` is the intended future heat source; rain coupling is deferred.
//! Coolers (`construction::Cooler`) are the one active heat source/sink: a
//! wall-segment building that drives the room(s) it borders toward a
//! player-set target instead of merely slowing the leak toward ambient (see
//! `next_climate_temperature`), drawing power proportional to the gap
//! (`power::tick_power`, which shares `cooler_room_temperature` below).

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::construction::{BuildableKind, ConstructionMap, Cooler};
use crate::daynight::GameClock;
use crate::game::GameAssets;
use crate::map::{self, RoofMap};
use crate::power::PowerGrid;
use crate::weather::is_outdoors;
use crate::zones::{Zone, ZoneKind, ZoneRegion};

/// The season sets the ambient temperature range. Spring and Winter exist;
/// the season devtool (Weather debug tab) switches between them until a
/// calendar (day counter → season) is worth having.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Season {
    #[default]
    Spring,
    Winter,
}

impl Season {
    /// Ambient at night and at the coldest edges of the day (sunrise/sunset).
    pub fn night_c(self) -> f32 {
        match self {
            Season::Spring => SPRING_NIGHT_TEMP_C,
            Season::Winter => WINTER_NIGHT_TEMP_C,
        }
    }

    /// Ambient at the warmest point of the day (noon).
    pub fn day_c(self) -> f32 {
        match self {
            Season::Spring => SPRING_DAY_TEMP_C,
            Season::Winter => WINTER_DAY_TEMP_C,
        }
    }

    /// The devtool's Spring↔Winter switch.
    pub fn toggled(self) -> Season {
        match self {
            Season::Spring => Season::Winter,
            Season::Winter => Season::Spring,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Season::Spring => "Spring",
            Season::Winter => "Winter",
        }
    }
}

#[derive(Resource, Default)]
pub struct ActiveSeason(pub Season);

/// Outdoor ambient °C at `elapsed` seconds of sim time. A sine hump over the
/// day (base at sunrise, peak at noon, base again at sunset), flat base all
/// night. Deliberately not `GameClock::daylight()` — its short dusk/dawn
/// fade is tuned for lighting and would turn temperature into a square wave.
pub fn ambient_temperature(season: Season, elapsed: f32) -> f32 {
    let t = elapsed.rem_euclid(CYCLE_LENGTH_SECS);
    let warmth = if t < DAY_LENGTH_SECS {
        (std::f32::consts::PI * t / DAY_LENGTH_SECS).sin()
    } else {
        0.0
    };
    season.night_c() + (season.day_c() - season.night_c()) * warmth
}

/// Per-cell °C — the `TerrainMap`/`HumidityMap` flat-grid model. Outdoor
/// cells simply ARE the ambient; roofed cells drift toward it, so the map
/// only diverges from ambient indoors.
#[derive(Resource)]
pub struct TemperatureMap {
    temps: Vec<f32>,
}

impl Default for TemperatureMap {
    fn default() -> Self {
        Self {
            // The world starts at sunrise, when ambient sits on the base.
            temps: vec![SPRING_NIGHT_TEMP_C; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl TemperatureMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<f32> {
        Self::in_bounds(cell).then(|| self.temps[(cell.y * MAP_WIDTH + cell.x) as usize])
    }
}

/// The one attribute a Room zone carries: how well its walls hold the
/// indoor temperature (0..=1, from `room_insulation`).
#[derive(Component)]
pub struct Room {
    pub insulation: f32,
}

/// Every enclosed region on the map: a full connected-components pass over
/// non-boundary cells (4-way, the same no-corner-cutting rule as
/// `construction::enclosed_interior`), keeping the components that never
/// touch the map border. One shared visited set — flooding per seed would
/// re-walk the huge outside component over and over.
pub fn detect_rooms(is_boundary: impl Fn(GridCoords) -> bool) -> Vec<Vec<GridCoords>> {
    let mut visited = vec![false; (MAP_WIDTH * MAP_HEIGHT) as usize];
    let mut rooms = Vec::new();
    for start_y in 0..MAP_HEIGHT {
        for start_x in 0..MAP_WIDTH {
            let start = GridCoords::new(start_x, start_y);
            if visited[(start_y * MAP_WIDTH + start_x) as usize] || is_boundary(start) {
                continue;
            }
            visited[(start_y * MAP_WIDTH + start_x) as usize] = true;
            let mut queue = VecDeque::from([start]);
            let mut region = Vec::new();
            let mut touches_border = false;
            while let Some(cell) = queue.pop_front() {
                if cell.x == 0 || cell.x == MAP_WIDTH - 1 || cell.y == 0 || cell.y == MAP_HEIGHT - 1
                {
                    touches_border = true;
                }
                region.push(cell);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let neighbor = GridCoords::new(cell.x + dx, cell.y + dy);
                    if !TemperatureMap::in_bounds(neighbor) || is_boundary(neighbor) {
                        continue;
                    }
                    let index = (neighbor.y * MAP_WIDTH + neighbor.x) as usize;
                    if !visited[index] {
                        visited[index] = true;
                        queue.push_back(neighbor);
                    }
                }
            }
            if !touches_border {
                rooms.push(region);
            }
        }
    }
    rooms
}

/// The boundary cells ringing an interior (the 8-neighborhood minus the
/// interior — the same ring `construction::auto_roof_around` stamps roofs
/// onto). Shared by `room_insulation` (what seals the room) and
/// `room_climate_target`/`cooler_room_temperature` (what drives and reads
/// it) so a cooler anywhere in this ring is treated consistently by all
/// three — a cooler that seals a room but isn't consulted for its target
/// would freeze the room at whatever temperature it held (see
/// `room_climate_target`'s doc comment).
fn boundary_ring(interior: &[GridCoords]) -> HashSet<GridCoords> {
    let cells: HashSet<GridCoords> = interior.iter().copied().collect();
    let mut boundary: HashSet<GridCoords> = HashSet::new();
    for cell in interior {
        for (dx, dy) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let ring = GridCoords::new(cell.x + dx, cell.y + dy);
            if !cells.contains(&ring) {
                boundary.insert(ring);
            }
        }
    }
    boundary
}

/// A room's insulation: the average contribution of its boundary ring
/// (`boundary_ring`). Walls insulate `WALL_INSULATION`, doors
/// `DOOR_INSULATION`, anything else 0.0 — for a 4-way-enclosed room only
/// diagonal corner gaps can be "anything else", and cold leaking through a
/// missing corner is exactly right. A fully wooden-walled room lands on
/// `WALL_INSULATION` exactly.
///
/// A `Cooler` anywhere on the ring is the one exception to averaging: it's
/// an active climate unit, not just another wall material, so its presence
/// seals the *whole room* to `COOLER_INSULATION` outright rather than
/// blending in as one ring cell's share of the average (which would dilute
/// to almost nothing in anything bigger than a tiny room — confirmed by
/// playtesting: a single cooler in an otherwise-wooden room barely moved
/// the average past plain wood's `WALL_INSULATION`, and the room never got
/// meaningfully colder than ambient no matter how low the target went).
pub fn room_insulation(
    interior: &[GridCoords],
    kind_at: impl Fn(GridCoords) -> Option<BuildableKind>,
) -> f32 {
    let boundary = boundary_ring(interior);
    if boundary.is_empty() {
        return 0.0;
    }
    if boundary
        .iter()
        .any(|cell| matches!(kind_at(*cell), Some(BuildableKind::Cooler)))
    {
        return COOLER_INSULATION;
    }
    let total: f32 = boundary
        .iter()
        .map(|cell| match kind_at(*cell) {
            Some(BuildableKind::Wall) => WALL_INSULATION,
            Some(BuildableKind::Door) => DOOR_INSULATION,
            _ => 0.0,
        })
        .sum();
    total / boundary.len() as f32
}

/// A room's climate setpoint: the average `target_c` of every `Cooler`
/// sitting on its boundary ring (`boundary_ring`) — the *same* ring
/// `room_insulation` reads to decide whether the room is sealed. `None` if
/// no ring cell is a cooler.
///
/// This must stay in lockstep with `room_insulation`'s cooler check: a
/// cooler behind a double wall, or tucked into a corner, isn't 4-way
/// adjacent to any interior floor cell, but it *is* on the ring — so it
/// seals the room to `COOLER_INSULATION` (zeroing the passive leak) without
/// this function driving it toward a target. The room then becomes a
/// perfect, frozen thermos: full power draw, temperature pinned wherever it
/// happened to be when the seal engaged, never approaching the target no
/// matter how long it runs. Matching the ring here (instead of the
/// 4-way-to-interior scan this replaced) fixes that.
pub fn room_climate_target(
    interior: &[GridCoords],
    target_at: impl Fn(GridCoords) -> Option<f32>,
) -> Option<f32> {
    let boundary = boundary_ring(interior);
    let mut sum = 0.0;
    let mut count = 0u32;
    for cell in &boundary {
        if let Some(target) = target_at(*cell) {
            sum += target;
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f32)
}

/// Re-derive the Room zones whenever the built layout changes (player
/// builds, demolition, the LDtk walls arriving after startup). Wholesale
/// despawn-and-respawn: rooms carry no player state and nothing selects
/// them, so recomputing from scratch handles merge/split/removal with zero
/// bookkeeping — cheap on a 64x64 map. Ungated on purpose: demolition works
/// while paused and rooms must follow the map.
pub fn sync_rooms(
    mut commands: Commands,
    construction: Res<ConstructionMap>,
    rooms: Query<Entity, With<Room>>,
    mut count_last_logged: Local<usize>,
) {
    if !construction.is_changed() {
        return;
    }
    for entity in &rooms {
        commands.entity(entity).despawn();
    }
    let is_boundary = |cell: GridCoords| {
        matches!(
            construction.get(cell),
            Some(BuildableKind::Wall | BuildableKind::Door | BuildableKind::Cooler)
        )
    };
    let detected = detect_rooms(is_boundary);
    if detected.len() != *count_last_logged {
        info!("temperature: {} room(s) detected", detected.len());
        *count_last_logged = detected.len();
    }
    for interior in detected {
        let insulation = room_insulation(&interior, |cell| construction.get(cell));
        commands.spawn((
            Zone {
                kind: ZoneKind::Room,
            },
            ZoneRegion::from_cells(interior),
            Room { insulation },
            Name::new("Room"),
        ));
    }
}

/// One temperature step for one indoor cell: exponential drift toward the
/// ambient, slowed by insulation (1.0 = a perfect thermos, 0.0 = a bare
/// roof). The `.min(1.0)` keeps a huge `dt` from overshooting past ambient.
fn next_indoor_temperature(current: f32, ambient: f32, insulation: f32, dt: f32) -> f32 {
    current + (ambient - current) * (INDOOR_TEMP_RATE_PER_SECOND * (1.0 - insulation) * dt).min(1.0)
}

/// One temperature step for a cell under active climate control: the same
/// ambient leak as `next_indoor_temperature` (insulation still matters — a
/// cooler in a drafty room fights a bigger leak), then an active pull toward
/// `target`, scaled by how much of the grid's demand was actually met this
/// tick (`PowerGrid::powered_fraction`) — an unpowered cooler drives nothing
/// and this reduces to exactly `next_indoor_temperature`. Bidirectional: the
/// pull's sign follows `target - after_leak`, so it heats as readily as it
/// cools.
fn next_climate_temperature(
    current: f32,
    target: f32,
    ambient: f32,
    insulation: f32,
    powered: f32,
    dt: f32,
) -> f32 {
    let after_leak = next_indoor_temperature(current, ambient, insulation, dt);
    after_leak + (target - after_leak) * (CLIMATE_RATE_PER_SECOND * powered * dt).min(1.0)
}

/// Average interior temperature of the room(s) `cell` (a cooler's own cell)
/// sits on the boundary ring of — the same ring `room_insulation`/
/// `room_climate_target` read, so a cooler that's sealing a room is always
/// the one whose temperature gets reported for it, even tucked behind a
/// double wall or a corner where it isn't 4-way adjacent to any floor cell.
/// Falls back to the plain indoor 4-neighbour average (the original
/// behaviour) when `cell` borders no detected `Room` at all — a freestanding
/// cooler with a bare indoor neighbour still reads something sensible.
/// `None` only when there's truly no indoor cell to read (a fully
/// freestanding cooler with no room to condition), so it drives nothing and
/// draws no power. Shared by `power::tick_power` (draw) and the info panel
/// (`ui::update_panel`'s readout) so both agree on what "the room's
/// temperature" means for a given cooler.
pub fn cooler_room_temperature(
    cell: GridCoords,
    temps: &TemperatureMap,
    roofs: &RoofMap,
    rooms: &Query<(&ZoneRegion, &Room)>,
) -> Option<f32> {
    let mut sum = 0.0;
    let mut count = 0u32;
    for (region, _) in rooms {
        let interior: Vec<GridCoords> = region.cells().collect();
        if !boundary_ring(&interior).contains(&cell) {
            continue;
        }
        for room_cell in &interior {
            if let Some(celsius) = temps.get(*room_cell) {
                sum += celsius;
                count += 1;
            }
        }
    }
    if count > 0 {
        return Some(sum / count as f32);
    }
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let neighbor = GridCoords::new(cell.x + dx, cell.y + dy);
        if !is_outdoors(neighbor, roofs) {
            if let Some(celsius) = temps.get(neighbor) {
                sum += celsius;
                count += 1;
            }
        }
    }
    (count > 0).then(|| sum / count as f32)
}

/// Advance every cell's temperature. Sim-gated: pausing freezes the drift
/// along with the clock driving the ambient. Reads `PowerGrid` one frame
/// stale relative to `power::tick_power` (no `.after()` pin, both sit in
/// sibling sim-gated chains) — the same lag `tick_power` itself tolerates
/// for `GameClock`.
pub fn update_temperature(
    time: Res<Time>,
    clock: Res<GameClock>,
    season: Res<ActiveSeason>,
    roofs: Res<RoofMap>,
    rooms: Query<(&ZoneRegion, &Room)>,
    coolers: Query<(&GridCoords, &Cooler)>,
    grid: Res<PowerGrid>,
    mut temps: ResMut<TemperatureMap>,
) {
    let dt = time.delta_secs();
    let ambient = ambient_temperature(season.0, clock.elapsed);
    // Every built cooler's setpoint, keyed by its own cell — looked up by
    // each room below via `room_climate_target`'s boundary ring, the exact
    // ring `room_insulation` reads off `ConstructionMap` to seal the room.
    let cooler_targets: HashMap<GridCoords, f32> = coolers
        .iter()
        .map(|(cell, cooler)| (*cell, cooler.target_c))
        .collect();
    // Per-cell insulation stamp from the room footprints (scales with the
    // rooms' own area). A roofed cell outside any room — a leftover after a
    // wall was demolished, or a lone player roof stamp — keeps the bare-roof
    // 0.0: it still lags, just poorly. `climate_target` stamps alongside it:
    // a room bordered by one or more coolers gets the average of their
    // targets across every one of its cells (mirroring insulation's uniform-
    // per-room stamp), so the whole room chases the same setpoint together
    // instead of only the cells touching a cooler — a cooler shared between
    // two rooms (built on the wall between them) drives both independently.
    let mut insulation = vec![0.0; (MAP_WIDTH * MAP_HEIGHT) as usize];
    let mut climate_target: Vec<Option<f32>> = vec![None; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (region, room) in &rooms {
        let interior: Vec<GridCoords> = region.cells().collect();
        let target = room_climate_target(&interior, |cell| cooler_targets.get(&cell).copied());
        for cell in region.cells() {
            if TemperatureMap::in_bounds(cell) {
                let index = (cell.y * MAP_WIDTH + cell.x) as usize;
                insulation[index] = room.insulation;
                climate_target[index] = target;
            }
        }
    }
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            let index = (y * MAP_WIDTH + x) as usize;
            temps.temps[index] = if is_outdoors(cell, &roofs) {
                ambient
            } else if let Some(target) = climate_target[index] {
                next_climate_temperature(
                    temps.temps[index],
                    target,
                    ambient,
                    insulation[index],
                    grid.powered_fraction,
                    dt,
                )
            } else {
                next_indoor_temperature(temps.temps[index], ambient, insulation[index], dt)
            };
        }
    }
}

/// Devtool switch for the temperature debug overlay (Weather debug tab).
#[derive(Resource, Default)]
pub struct ShowTemperatureOverlay(pub bool);

/// A translucent cold-blue..warm-red tile over one cell while the overlay
/// is on — the `FuelOverlay` model: the entity IS the visual.
#[derive(Component)]
pub struct TempOverlay {
    cell: GridCoords,
    bucket: usize,
}

/// Overlay bucket 1..=5 across the season's night..day ambient range (an
/// indoor cell mid-drift lands in between; anything past the range clamps
/// to the end tints).
fn temperature_bucket(celsius: f32, season: Season) -> usize {
    let range = season.day_c() - season.night_c();
    let normalized = ((celsius - season.night_c()) / range).clamp(0.0, 1.0);
    1 + (normalized * 4.0).round() as usize
}

/// Keep one overlay tile per cell while the devtool is on — the fuel
/// overlay reconciler with `TemperatureMap` as the signal. Walls hide the
/// ground and standing water has its own surface, so neither draws (their
/// temperature still tracks).
pub fn sync_temperature_overlay(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowTemperatureOverlay>,
    season: Res<ActiveSeason>,
    construction: Res<ConstructionMap>,
    water: Res<map::WaterMap>,
    temps: Res<TemperatureMap>,
    mut overlays: Query<(
        Entity,
        &mut TempOverlay,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let desired = |cell: GridCoords| -> usize {
        if !show.0
            || water.is_wet(cell)
            || matches!(construction.get(cell), Some(BuildableKind::Wall))
        {
            return 0;
        }
        temps
            .get(cell)
            .map_or(0, |celsius| temperature_bucket(celsius, season.0))
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
            material.0 = assets.temp_overlay_materials[bucket - 1].clone();
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
                MeshMaterial3d(assets.temp_overlay_materials[bucket - 1].clone()),
                Transform::from_translation(
                    map::grid_to_world(&cell) - Vec3::Y * (TILE_THICKNESS / 2.0)
                        + Vec3::Y * TEMP_OVERLAY_OFFSET,
                ),
                Name::new("TempOverlay"),
                TempOverlay { cell, bucket },
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambient_is_base_at_sunrise_and_sunset() {
        assert_eq!(ambient_temperature(Season::Spring, 0.0), 12.0);
        let sunset = ambient_temperature(Season::Spring, DAY_LENGTH_SECS);
        assert!((sunset - 12.0).abs() < 1e-4, "sunset was {sunset}");
    }

    #[test]
    fn ambient_peaks_at_noon() {
        let noon = ambient_temperature(Season::Spring, DAY_LENGTH_SECS / 2.0);
        assert!((noon - 20.0).abs() < 1e-4, "noon was {noon}");
    }

    #[test]
    fn ambient_is_flat_base_through_the_night() {
        let early = ambient_temperature(Season::Spring, DAY_LENGTH_SECS + 1.0);
        let late = ambient_temperature(Season::Spring, CYCLE_LENGTH_SECS - 1.0);
        assert_eq!(early, 12.0);
        assert_eq!(late, 12.0);
    }

    #[test]
    fn ambient_rises_monotonically_through_the_morning() {
        let mut previous = f32::MIN;
        for step in 0..=10 {
            let t = DAY_LENGTH_SECS / 2.0 * step as f32 / 10.0;
            let now = ambient_temperature(Season::Spring, t);
            assert!(now > previous, "dipped to {now} at t={t}");
            previous = now;
        }
    }

    #[test]
    fn ambient_wraps_across_days() {
        let day_two = ambient_temperature(Season::Spring, CYCLE_LENGTH_SECS + 30.0);
        let day_one = ambient_temperature(Season::Spring, 30.0);
        assert_eq!(day_two, day_one);
    }

    #[test]
    fn winter_ambient_spans_deep_freeze_to_slush() {
        assert_eq!(ambient_temperature(Season::Winter, 0.0), -20.0);
        let noon = ambient_temperature(Season::Winter, DAY_LENGTH_SECS / 2.0);
        assert!((noon - -4.0).abs() < 1e-4, "noon was {noon}");
        assert_eq!(
            ambient_temperature(Season::Winter, DAY_LENGTH_SECS + 1.0),
            -20.0
        );
    }

    #[test]
    fn indoor_step_moves_toward_ambient_without_overshoot() {
        let warmer = next_indoor_temperature(12.0, 20.0, 0.0, 1.0);
        assert!(warmer > 12.0 && warmer < 20.0, "was {warmer}");
        let cooler = next_indoor_temperature(20.0, 12.0, 0.0, 1.0);
        assert!(cooler < 20.0 && cooler > 12.0, "was {cooler}");
        // A silly dt lands exactly on ambient instead of shooting past it.
        assert_eq!(next_indoor_temperature(12.0, 20.0, 0.0, 1e6), 20.0);
    }

    #[test]
    fn insulation_slows_the_drift() {
        let bare = next_indoor_temperature(20.0, 12.0, 0.0, 1.0);
        let wooden = next_indoor_temperature(20.0, 12.0, 0.5, 1.0);
        let thermos = next_indoor_temperature(20.0, 12.0, 1.0, 1.0);
        assert!(wooden > bare, "wooden {wooden} vs bare {bare}");
        assert_eq!(thermos, 20.0);
    }

    /// Unpowered, the active climate drive contributes nothing — the room
    /// just leaks toward ambient exactly like a plain insulated cell.
    #[test]
    fn unpowered_climate_step_matches_plain_indoor_drift() {
        let plain = next_indoor_temperature(20.0, 12.0, 0.5, 1.0);
        let unpowered = next_climate_temperature(20.0, 30.0, 12.0, 0.5, 0.0, 1.0);
        assert_eq!(unpowered, plain);
    }

    /// Fully powered, a cooler pulls the room toward its target regardless
    /// of which way that is from the current temperature (bidirectional) —
    /// past the plain ambient leak alone.
    #[test]
    fn powered_climate_step_pulls_toward_target_both_ways() {
        // Room is warmer than ambient AND warmer than the (cooling) target:
        // the active pull should push it down further than the leak alone.
        let leak_only = next_indoor_temperature(25.0, 20.0, 1.0, 1.0);
        let cooling = next_climate_temperature(25.0, 10.0, 20.0, 1.0, 1.0, 1.0);
        assert!(cooling < leak_only, "cooling {cooling} vs leak {leak_only}");
        assert!(cooling < 25.0);

        // Room is colder than ambient AND colder than the (heating) target:
        // the active pull should push it up further than the leak alone.
        let leak_only = next_indoor_temperature(0.0, 5.0, 1.0, 1.0);
        let heating = next_climate_temperature(0.0, 25.0, 5.0, 1.0, 1.0, 1.0);
        assert!(heating > leak_only, "heating {heating} vs leak {leak_only}");
        assert!(heating > 0.0);
    }

    fn at(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    /// A 3x3 wall square around the single interior cell (11, 11) — the
    /// `construction::tests::small_room` fixture, rebuilt here as a set.
    fn small_room() -> HashSet<GridCoords> {
        let mut walls = HashSet::new();
        for x in 10..=12 {
            for y in 10..=12 {
                if (x, y) != (11, 11) {
                    walls.insert(at(x, y));
                }
            }
        }
        walls
    }

    #[test]
    fn detects_a_single_enclosed_room() {
        let walls = small_room();
        let rooms = detect_rooms(|cell| walls.contains(&cell));
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0], vec![at(11, 11)]);
    }

    #[test]
    fn a_gap_in_the_ring_leaks_the_room() {
        let mut walls = small_room();
        walls.remove(&at(10, 11));
        let rooms = detect_rooms(|cell| walls.contains(&cell));
        assert!(rooms.is_empty());
    }

    #[test]
    fn two_rooms_sharing_a_wall_stay_separate() {
        // Two 3x3 squares sharing the x=12 wall column: interiors (11, 11)
        // and (13, 11).
        let mut walls = small_room();
        for x in 12..=14 {
            for y in 10..=12 {
                if (x, y) != (13, 11) {
                    walls.insert(at(x, y));
                }
            }
        }
        let mut rooms = detect_rooms(|cell| walls.contains(&cell));
        rooms.sort_by_key(|room| room[0].x);
        assert_eq!(rooms.len(), 2);
        assert_eq!(rooms[0], vec![at(11, 11)]);
        assert_eq!(rooms[1], vec![at(13, 11)]);
    }

    /// The full ring of a fully wooden-walled room averages to (within f32
    /// summation dust) the wall value. `WALL_INSULATION` (0.8) isn't exactly
    /// representable in binary floating point, so summing eight copies and
    /// dividing lands a hair off bit-identical — hence the epsilon.
    #[test]
    fn wooden_room_insulates_at_the_wall_value() {
        let walls = small_room();
        let insulation = room_insulation(&[at(11, 11)], |cell| {
            walls.contains(&cell).then_some(BuildableKind::Wall)
        });
        assert!((insulation - WALL_INSULATION).abs() < 1e-5);
    }

    #[test]
    fn a_door_drags_the_average_down() {
        let walls = small_room();
        let insulation = room_insulation(&[at(11, 11)], |cell| {
            walls.contains(&cell).then_some(if cell == at(10, 11) {
                BuildableKind::Door
            } else {
                BuildableKind::Wall
            })
        });
        // 7 walls + 1 door over the 8-cell ring.
        assert!((insulation - (7.0 * WALL_INSULATION + DOOR_INSULATION) / 8.0).abs() < 1e-5);
    }

    /// A missing diagonal corner doesn't open the room (4-way enclosure)
    /// but does leak heat: the empty ring cell contributes zero.
    #[test]
    fn a_missing_corner_leaks_heat_but_not_the_room() {
        let mut walls = small_room();
        walls.remove(&at(10, 10));
        assert_eq!(detect_rooms(|cell| walls.contains(&cell)).len(), 1);
        let insulation = room_insulation(&[at(11, 11)], |cell| {
            walls.contains(&cell).then_some(BuildableKind::Wall)
        });
        assert!((insulation - 7.0 * WALL_INSULATION / 8.0).abs() < 1e-5);
    }

    /// A cooler anywhere on the ring overrides the average entirely — the
    /// room seals fully rather than blending the cooler's perfect seal with
    /// the other 7 wall cells' partial one.
    #[test]
    fn a_cooler_anywhere_in_the_ring_seals_the_room_fully() {
        let walls = small_room();
        let insulation = room_insulation(&[at(11, 11)], |cell| {
            walls.contains(&cell).then_some(if cell == at(10, 11) {
                BuildableKind::Cooler
            } else {
                BuildableKind::Wall
            })
        });
        assert_eq!(insulation, COOLER_INSULATION);
    }

    /// The override has no minimum-quorum requirement: a lone cooler with
    /// every other ring cell unbuilt still fully seals the room.
    #[test]
    fn a_lone_cooler_seals_the_room_with_no_walls_backing_it_up() {
        let insulation = room_insulation(&[at(11, 11)], |cell| {
            (cell == at(10, 11)).then_some(BuildableKind::Cooler)
        });
        assert_eq!(insulation, COOLER_INSULATION);
    }

    /// The straightforward case: a cooler built directly in a wall (4-way
    /// adjacent to the interior) sets the room's climate target.
    #[test]
    fn a_cooler_directly_in_a_wall_sets_the_room_target() {
        let target = room_climate_target(&[at(11, 11)], |cell| {
            (cell == at(10, 11)).then_some(18.0)
        });
        assert_eq!(target, Some(18.0));
    }

    /// Regression for the "frozen thermos" bug: a cooler sitting on a
    /// diagonal corner of the ring — on the ring `room_insulation` reads (so
    /// it still seals the room to `COOLER_INSULATION`), but *not* 4-way
    /// adjacent to the lone interior cell (e.g. a corner of the room, or a
    /// cooler built behind a double wall). Before this fixed
    /// `room_climate_target` to read the same ring as `room_insulation`, this
    /// case sealed the room's leak to zero without ever driving it toward a
    /// target — full power draw, temperature pinned wherever it happened to
    /// be, never approaching the target no matter how long it ran.
    #[test]
    fn a_cooler_in_a_ring_corner_still_sets_the_room_target() {
        let target = room_climate_target(&[at(11, 11)], |cell| {
            (cell == at(10, 10)).then_some(-10.0)
        });
        assert_eq!(target, Some(-10.0));
    }

    /// No cooler anywhere on the ring: no target, matching
    /// `room_insulation`'s plain wall/door averaging path for the same room.
    #[test]
    fn no_cooler_on_the_ring_means_no_room_target() {
        let target = room_climate_target(&[at(11, 11)], |_| None);
        assert_eq!(target, None);
    }
}
