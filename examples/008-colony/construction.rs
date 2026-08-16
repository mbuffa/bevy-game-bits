//! Construction: player-ordered walls and doors, plus per-cell roofs.
//!
//! Walls and doors start as ghost `Blueprint` entities (walkable, selectable,
//! cancellable). Batched Supply runs haul wood to them (5 per unit), a timed
//! Build job raises the real thing, and the completed cell updates
//! `TerrainMap` + `NavGrid` at runtime — the first terrain mutation after
//! load. Completing a wall/door that encloses a region auto-stamps the
//! interior with the Roof policy; roof intent itself is per-cell state in
//! `map::RoofMap` (see the `RoofPolicy` docs), never a zone. The gap between
//! policy and physical roof is what the Director turns into fast, material-
//! free BuildRoof/RemoveRoof jobs.

use std::collections::{HashSet, VecDeque};
use std::f32::consts::FRAC_PI_2;

use bevy::light::NotShadowCaster;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::daynight::{self, DayPhase, GameClock};
use crate::game::GameAssets;
use crate::items::{self, ItemKind, ItemStack};
use crate::map::{self, RoofMap, RoofPolicy, TerrainMap};
use crate::nav::NavGrid;
use crate::power::{PowerConsumer, PowerOutput};
use crate::scene::MainCamera;
use crate::selection::{cursor_to_cell, Selectable, SelectedEntity};
use crate::zones::{snapped_footprint_anchors, ActiveTool, DragAnchor, Tool};

/// Which way a 2-tile bed lies. Placement (`zones::Tool::PlaceBed`) can
/// rotate between the two with the R key; a built bed's `Footprint.cells` is
/// all downstream code needs afterward, so nothing else keys off this once
/// construction finishes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BedOrientation {
    Horizontal,
    Vertical,
}

impl BedOrientation {
    pub fn toggle(self) -> Self {
        match self {
            BedOrientation::Horizontal => BedOrientation::Vertical,
            BedOrientation::Vertical => BedOrientation::Horizontal,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildableKind {
    Wall,
    Door,
    /// Wind turbine: free. The nacelle yaws upwind, the rotor spins with
    /// strength, and (`power::tick_power`) it feeds the grid at a rate
    /// scaled by the live wind reaching its cell.
    Turbine,
    /// Solar panel: the first building whose `footprint` spans more than one
    /// cell (2x2). The panel tilts to track the sun and (`power::tick_power`)
    /// feeds the grid at a rate scaled by daylight and sun elevation.
    SolarPanel,
    /// Cheap streetlight: warm `PointLight` that fades in from dusk and out
    /// at dawn (`sync_lightposts`), scaled by the grid's coverage
    /// (`PowerGrid::powered_fraction`) once it draws power. The first
    /// building that stays fully walkable AND makes no `NavGrid` change at
    /// all on completion (contrast `Door`, which is walkable but still
    /// raises the step cost).
    Lightpost,
    /// One-tile store that banks grid surplus and discharges it to cover a
    /// deficit (`power::tick_power`). Solid, same footprint/blocking shape
    /// as a turbine.
    Battery,
    /// A tired pawn's resting spot (`director::JobKind::Sleep`), sized for a
    /// lying pawn (2 tiles long, see `PAWN_HEIGHT`). Fully walkable and makes
    /// no `NavGrid` change, like `Lightpost` — a sleeper must be able to
    /// stand on it. The second multi-cell footprint after `SolarPanel`, and
    /// the first with more than one possible shape (`BedOrientation`).
    Bed(BedOrientation),
    /// A wall-segment climate unit: blocks movement and encloses/roofs like
    /// `Wall`, insulates perfectly (`COOLER_INSULATION`, `temperature::
    /// room_insulation`), and actively drives the room(s) it borders toward
    /// its `Cooler::target_c` (`temperature::update_temperature`), drawing
    /// power proportional to the gap (`power::tick_power`). Bidirectional —
    /// heats as readily as it cools.
    Cooler,
}

impl BuildableKind {
    pub fn label(&self) -> &'static str {
        match self {
            BuildableKind::Wall => "Wall",
            BuildableKind::Door => "Door",
            BuildableKind::Turbine => "Wind turbine",
            BuildableKind::SolarPanel => "Solar panel",
            BuildableKind::Lightpost => "Lightpost",
            BuildableKind::Battery => "Battery",
            BuildableKind::Bed(_) => "Bed",
            BuildableKind::Cooler => "Cooler",
        }
    }

    pub fn wood_cost(&self) -> u32 {
        match self {
            BuildableKind::Wall => WALL_WOOD_COST,
            BuildableKind::Door => DOOR_WOOD_COST,
            BuildableKind::Turbine => TURBINE_WOOD_COST,
            BuildableKind::SolarPanel => SOLAR_PANEL_WOOD_COST,
            BuildableKind::Lightpost => LIGHTPOST_WOOD_COST,
            BuildableKind::Battery => BATTERY_WOOD_COST,
            BuildableKind::Bed(_) => BED_WOOD_COST,
            BuildableKind::Cooler => COOLER_WOOD_COST,
        }
    }

    pub fn build_secs(&self) -> f32 {
        match self {
            BuildableKind::Wall => WALL_BUILD_SECS,
            BuildableKind::Door => DOOR_BUILD_SECS,
            BuildableKind::Turbine => TURBINE_BUILD_SECS,
            BuildableKind::SolarPanel => SOLAR_PANEL_BUILD_SECS,
            BuildableKind::Lightpost => LIGHTPOST_BUILD_SECS,
            BuildableKind::Battery => BATTERY_BUILD_SECS,
            BuildableKind::Bed(_) => BED_BUILD_SECS,
            BuildableKind::Cooler => COOLER_BUILD_SECS,
        }
    }

    /// Does the built (finished) structure block movement? Doors stay
    /// walkable (just slower, `TERRAIN_COST_DOOR`); every other kind is
    /// solid. The single seam both `complete_builds` and the Build job's
    /// entombment guard (`director::execute_jobs`) key off, so a new
    /// blocking building only needs updating here plus the nav write in
    /// `complete_builds`.
    pub fn blocks_movement(&self) -> bool {
        match self {
            BuildableKind::Wall
            | BuildableKind::Turbine
            | BuildableKind::SolarPanel
            | BuildableKind::Battery
            | BuildableKind::Cooler => true,
            BuildableKind::Door | BuildableKind::Lightpost | BuildableKind::Bed(_) => false,
        }
    }

    /// Cell offsets from the anchor cell (the `GridCoords`/`Blueprint`
    /// site) that this building occupies. Every 1x1 building shares the same
    /// single-cell offset; the solar panel and bed are the multi-cell shapes
    /// (the bed's the only one with more than one possible shape, picked by
    /// `BedOrientation`).
    pub fn footprint(&self) -> &'static [(i32, i32)] {
        match self {
            BuildableKind::Wall
            | BuildableKind::Door
            | BuildableKind::Turbine
            | BuildableKind::Lightpost
            | BuildableKind::Battery
            | BuildableKind::Cooler => &[(0, 0)],
            BuildableKind::SolarPanel => &[(0, 0), (1, 0), (0, 1), (1, 1)],
            BuildableKind::Bed(BedOrientation::Horizontal) => &[(0, 0), (1, 0)],
            BuildableKind::Bed(BedOrientation::Vertical) => &[(0, 0), (0, 1)],
        }
    }
}

/// `kind`'s footprint offsets translated to world cells anchored at `anchor`.
pub fn footprint_cells(kind: BuildableKind, anchor: GridCoords) -> Vec<GridCoords> {
    kind.footprint()
        .iter()
        .map(|(dx, dy)| GridCoords::new(anchor.x + dx, anchor.y + dy))
        .collect()
}

/// `(width, height)` of `kind`'s footprint — the bounding box of its offsets,
/// e.g. `(2, 2)` for the solar panel or `(2, 1)`/`(1, 2)` for a bed. Drives
/// `zones::snapped_footprint_anchors`'s tiling step, so the anchor tiler can
/// never drift out of sync with the actual shape it's tiling.
pub fn footprint_size(kind: BuildableKind) -> (i32, i32) {
    let offsets = kind.footprint();
    let width = offsets.iter().map(|(dx, _)| dx).max().copied().unwrap_or(0) + 1;
    let height = offsets.iter().map(|(_, dy)| dy).max().copied().unwrap_or(0) + 1;
    (width, height)
}

/// World-space center of `kind`'s footprint anchored at `anchor`: the
/// single tile's own center for a 1x1 building, the middle of the block for
/// a multi-cell one (the solar panel's 2x2) — the average of every
/// footprint cell's center covers both uniformly.
pub fn footprint_center_world(kind: BuildableKind, anchor: GridCoords) -> Vec3 {
    let cells = footprint_cells(kind, anchor);
    let sum = cells
        .iter()
        .fold(Vec3::ZERO, |acc, cell| acc + map::grid_to_world(cell));
    sum / cells.len() as f32
}

/// Which cells a blueprint/building occupies — the anchor cell alone for a
/// 1x1 building, all 4 cells for a 2x2 solar panel. Carried by every
/// `Blueprint` and every built structure so placement, selection, and job
/// adjacency work the same regardless of footprint size.
#[derive(Component, Clone, Debug)]
pub struct Footprint {
    pub cells: Vec<GridCoords>,
}

impl Footprint {
    pub fn new(kind: BuildableKind, anchor: GridCoords) -> Self {
        Self {
            cells: footprint_cells(kind, anchor),
        }
    }

    pub fn contains(&self, cell: GridCoords) -> bool {
        self.cells.contains(&cell)
    }
}

/// What's built on each cell, layered on top of natural ground —
/// parallel to `TerrainMap`/`RoofMap` (same O(1) flat-array shape), so a
/// cell's state is spread across independent layers: ground (`TerrainMap`,
/// never touched by construction), what's built on it (this), and whether
/// it's roofed (`RoofMap`). A cell's ground terrain no longer changes when
/// something is built on it — every cell always has real ground geometry
/// under it, construction or not.
#[derive(Resource)]
pub struct ConstructionMap {
    kinds: Vec<Option<BuildableKind>>,
}

impl Default for ConstructionMap {
    fn default() -> Self {
        Self {
            kinds: vec![None; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl ConstructionMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<BuildableKind> {
        Self::in_bounds(cell)
            .then(|| self.kinds[(cell.y * MAP_WIDTH + cell.x) as usize])
            .flatten()
    }

    pub fn set(&mut self, cell: GridCoords, kind: Option<BuildableKind>) {
        if Self::in_bounds(cell) {
            self.kinds[(cell.y * MAP_WIDTH + cell.x) as usize] = kind;
        }
    }
}

/// Cells `sync_construction_collision` currently holds solid — the set it
/// marked non-walkable last time it ran, so it can revert exactly those
/// cells once they leave construction (built or canceled), even though the
/// `Blueprint` entity that caused it is gone by then.
#[derive(Resource, Default)]
pub struct ConstructionBlocks(HashSet<GridCoords>);

/// Keep a supplied blueprint's footprint solid while it's under construction
/// — collision-free until the wood arrives, solid from `is_supplied` through
/// completion, so a pawn can never wander onto a site mid-build (the failure
/// mode the completion-time entombment guard in `director::execute_jobs`
/// exists to catch). Never solidifies a cell a pawn is standing on, and
/// re-solidifies it the frame after the pawn leaves — `find_path` refuses a
/// blocked `from`, so a pawn is never stranded. Reverts a cell once its
/// blueprint is gone (built or canceled) unless the finished structure is
/// itself solid (`BuildableKind::blocks_movement`), which `complete_builds`
/// already set and this leaves alone.
pub fn sync_construction_collision(
    blueprints: Query<(&Blueprint, &Footprint)>,
    pawns: Query<&GridCoords, With<crate::units::Pawn>>,
    construction: Res<ConstructionMap>,
    mut nav: ResMut<NavGrid>,
    mut blocks: ResMut<ConstructionBlocks>,
) {
    let pawn_cells: HashSet<GridCoords> = pawns.iter().copied().collect();
    let mut desired: HashSet<GridCoords> = HashSet::new();
    for (blueprint, footprint) in &blueprints {
        if blueprint.is_supplied() {
            for cell in &footprint.cells {
                if !pawn_cells.contains(cell) {
                    desired.insert(*cell);
                }
            }
        }
    }
    for cell in desired.difference(&blocks.0) {
        nav.set_walkable(*cell, false);
    }
    for cell in blocks.0.difference(&desired) {
        let built_solid = construction
            .get(*cell)
            .is_some_and(|kind| kind.blocks_movement());
        if !built_solid {
            nav.set_walkable(*cell, true);
        }
    }
    blocks.0 = desired;
}

/// A planned wall/door: a ghost on the map, walkable while wood is still
/// being poured into `delivered` by Supply runs. Once `is_supplied` the
/// Director queues a Build job for it AND `sync_construction_collision`
/// starts treating its footprint as solid (minus any cell a pawn is
/// currently standing on) — collision-free while queued, solid while
/// awaiting/under construction.
#[derive(Component)]
pub struct Blueprint {
    pub kind: BuildableKind,
    pub delivered: u32,
}

impl Blueprint {
    pub fn needed(&self) -> u32 {
        self.kind.wood_cost().saturating_sub(self.delivered)
    }

    pub fn is_supplied(&self) -> bool {
        self.needed() == 0
    }
}

/// Player canceled this blueprint (the selection HUD's Cancel button). A
/// component rather than a message so a cancel ordered while paused can't
/// be dropped by the message double-buffer.
#[derive(Component)]
pub struct ToCancel;

/// Queued for demolition, the construction mirror of `trees::ToCut`: ordered
/// by the player via the Demolish order (per-tile button or area drag —
/// never automatic). This is what generates Demolish jobs; both the per-tile
/// Cancel button and the drag `Tool::CancelDemolish` counterpart just remove
/// it, same as `ToCut`.
#[derive(Component)]
pub struct ToDemolish;

/// Marker on a built door's entity; enclosure and nav behavior come from
/// `ConstructionMap`, this is just for identification.
#[derive(Component)]
pub struct Door;

/// Marker on a built wall's visual entity. Queried by `sync_wall_connections`
/// to pick the right autotiled mesh.
#[derive(Component)]
pub struct Wall;

/// Marker on a built turbine's root entity.
#[derive(Component)]
pub struct Turbine;

/// Marker on a built solar panel's root entity.
#[derive(Component)]
pub struct SolarPanel;

/// The base pivot on a built solar panel: `aim_solar_panels` yaws it to face
/// the sun's azimuth, single-axis-tracker style. Ghost panels don't carry it
/// — blueprints stay still. The array's fixed backward tilt lives one level
/// further down (the `holder` child spawned by `spawn_solar_parts`), so it
/// applies to ghosts and built panels alike.
#[derive(Component)]
pub struct SolarPanelPivot;

/// Marker on a built lightpost's root entity.
#[derive(Component)]
pub struct Lightpost;

/// The lamp-head child of a built lightpost: carries the `PointLight` and
/// the (shared) emissive material `sync_lightposts` mutates. Ghost
/// lightposts don't carry it — blueprints stay unlit.
#[derive(Component)]
pub struct LightpostLamp;

/// A built battery's stored charge, units, capped at `BATTERY_CAPACITY` —
/// lives on the root entity alongside `PowerOutput`-less production markers
/// like `Turbine`/`SolarPanel`. `power::tick_power` is the only writer;
/// `sync_batteries` mirrors it onto the charge-strip material.
#[derive(Component)]
pub struct Battery {
    pub charge: f32,
}

/// The charge-indicator strip on a built battery: `sync_batteries` mixes its
/// material color from empty to full as `Battery.charge` climbs. Ghost
/// batteries don't carry it — blueprints stay at the empty color.
#[derive(Component)]
pub struct BatteryStrip;

/// Marker on a built bed's root entity. Orientation doesn't matter once
/// built — `director::JobKind::Sleep`/`footprint_rest_spot` only read
/// `Footprint.cells`, which already captures whichever shape was placed.
#[derive(Component)]
pub struct Bed;

/// A built cooler's setpoint, °C — edited by the info panel's target-
/// temperature row (`ui::run_target_temp_buttons`) and read by
/// `power::tick_power` (draw scales with the gap to the room's temperature)
/// and `temperature::update_temperature` (the active drive toward it). Also
/// a boundary/insulation kind: joins `Wall | Door` everywhere a room's
/// enclosure is derived (`sync_rooms`, `construction::auto_roof_around`/
/// `enclosed_interior`). Unlike a wall or door, its insulation doesn't blend
/// into a ring average — `temperature::room_insulation` seals the *whole
/// room* to `COOLER_INSULATION` the moment one borders it, since it's an
/// active climate unit rather than just another wall material.
#[derive(Component)]
pub struct Cooler {
    pub target_c: f32,
}

/// Data marker inserted by bevy_ecs_ldtk for each `Wall` entity instance on
/// the Entities layer — a wall is placed data (like `Tree`/`Bush`/`Pawn`),
/// not an IntGrid terrain paint, so its cell's ground stays whatever natural
/// terrain it actually is and gets a real ground tile from the ordinary
/// bridge (`map::spawn_tile_visuals` et al.) like any other cell.
#[derive(Default, Component)]
pub struct WallSpawn;

#[derive(Default, Bundle, LdtkEntity)]
pub struct WallSpawnBundle {
    marker: WallSpawn,
    #[grid_coords]
    grid_coords: GridCoords,
}

/// The nacelle pivot atop a built turbine's tower: `yaw_turbines` turns it
/// to face upwind. Ghost turbines don't carry it — blueprints stay still.
#[derive(Component)]
pub struct TurbineNacelle;

/// The rotor pivot on a built turbine's nacelle: `spin_turbines` rotates it
/// around its wind-facing axis, scaled by wind strength.
#[derive(Component)]
pub struct TurbineRotor;

/// Yaw (radians around Y) turning the nacelle's +X forward to face UPWIND
/// (into `wind_dir`, which points where the wind blows toward): rotating +X
/// by `a` lands on `(cos a, -sin a)` in XZ, and we want `-wind_dir`.
pub fn turbine_yaw(wind_dir: Vec2) -> f32 {
    wind_dir.y.atan2(-wind_dir.x)
}

// Neighbor bits for wall/door autotiling. N = grid +y (world -Z), S = grid
// -y (+Z), E = grid +x (+X), W = grid -x (-X).
pub const CONNECT_N: u8 = 1;
pub const CONNECT_E: u8 = 2;
pub const CONNECT_S: u8 = 4;
pub const CONNECT_W: u8 = 8;

/// Center post + an arm to the tile edge toward each connected neighbor, so
/// adjacent walls' arms meet flush at the shared tile boundary.
pub fn connected_wall_mesh(mask: u8) -> Mesh {
    let half = WALL_VISUAL_SIZE / 2.0;
    let arm_len = TILE_SIZE / 2.0 - half;
    let mid = half + arm_len / 2.0;
    let mut mesh = Mesh::from(Cuboid::new(WALL_VISUAL_SIZE, WALL_HEIGHT, WALL_VISUAL_SIZE));
    let ns_arm = Cuboid::new(WALL_VISUAL_SIZE, WALL_HEIGHT, arm_len);
    let ew_arm = Cuboid::new(arm_len, WALL_HEIGHT, WALL_VISUAL_SIZE);
    if mask & CONNECT_N != 0 {
        mesh.merge(&Mesh::from(ns_arm).translated_by(Vec3::new(0.0, 0.0, -mid)))
            .unwrap();
    }
    if mask & CONNECT_S != 0 {
        mesh.merge(&Mesh::from(ns_arm).translated_by(Vec3::new(0.0, 0.0, mid)))
            .unwrap();
    }
    if mask & CONNECT_E != 0 {
        mesh.merge(&Mesh::from(ew_arm).translated_by(Vec3::new(mid, 0.0, 0.0)))
            .unwrap();
    }
    if mask & CONNECT_W != 0 {
        mesh.merge(&Mesh::from(ew_arm).translated_by(Vec3::new(-mid, 0.0, 0.0)))
            .unwrap();
    }
    mesh
}

/// A cell is wall-connective if it holds a Wall or a Door (doors are part of
/// a run).
pub fn wall_connect_mask(cell: GridCoords, connective: impl Fn(GridCoords) -> bool) -> u8 {
    let mut m = 0;
    if connective(GridCoords::new(cell.x, cell.y + 1)) {
        m |= CONNECT_N;
    }
    if connective(GridCoords::new(cell.x + 1, cell.y)) {
        m |= CONNECT_E;
    }
    if connective(GridCoords::new(cell.x, cell.y - 1)) {
        m |= CONNECT_S;
    }
    if connective(GridCoords::new(cell.x - 1, cell.y)) {
        m |= CONNECT_W;
    }
    m
}

/// A door sitting in a vertical (N-S) run gets rotated 90 deg; horizontal is
/// the default (isolated and T-junction cases fall to horizontal too).
pub fn door_is_vertical(mask: u8) -> bool {
    let ns = (mask & CONNECT_N != 0) as u8 + (mask & CONNECT_S != 0) as u8;
    let ew = (mask & CONNECT_E != 0) as u8 + (mask & CONNECT_W != 0) as u8;
    ns > ew
}

/// A supplied blueprint finished building (written by the Director when the
/// Build job's work timer completes).
#[derive(Message)]
pub struct BuildCommand {
    pub site: Entity,
}

/// A roof job finished on `cell`: install or tear off the physical roof.
#[derive(Message)]
pub struct RoofCommand {
    pub cell: GridCoords,
    pub install: bool,
}

/// A structure marked `ToDemolish` finished coming down, written by the
/// Director when the Demolish job's work timer completes with the pawn
/// standing next to its footprint. Mirrors `trees::CutCommand`.
#[derive(Message)]
pub struct DemolishCommand {
    pub site: Entity,
    /// The demolisher's tile: the salvaged wood lands beneath the pawn,
    /// spilling outward.
    pub drop_at: GridCoords,
}

/// Marks the single merged surface covering every built-roof cell, whose
/// mesh `rebuild_roof_surfaces` regenerates whenever `RoofMap` changes —
/// the `WaterSurfaceMesh` model, one draw call instead of one translucent
/// slab per cell (which used to sort-fight every frame).
#[derive(Component)]
pub struct RoofSurfaceMesh;

/// Marks the merged surface of inset squares over cells planned for a roof
/// but not yet built (the "ToRoof" preview).
#[derive(Component)]
pub struct RoofGhostMesh;

/// May a roof policy be stamped here? Anything on the map but a riverbed.
/// The gate is the authored bed (`WaterMap::has_bed`), not live wetness —
/// a drained bed can reflood, and a roof standing over returning water
/// would look odd and grow odder once roofs matter.
pub fn is_roofable(terrain: &TerrainMap, water: &map::WaterMap, cell: GridCoords) -> bool {
    terrain.get(cell).is_some() && !water.has_bed(cell)
}

/// Does this cell want a roof built? The derived "ToRoof" designation:
/// stamped for roofing, not yet roofed, and holding ground a roof can span
/// (`on_map` = the cell has terrain, `water_bed` = `WaterMap::has_bed` —
/// beds never take roofs, drained or not). Walls and doors DO take a panel
/// — a roof that stops at the interior looks disconnected from the walls
/// that hold it up; the builder pawn just works from a free adjacent tile
/// instead of standing on the blocked cell (see
/// `director::nearest_stand_spot`).
pub fn wants_roof(policy: RoofPolicy, roofed: bool, on_map: bool, water_bed: bool) -> bool {
    policy == RoofPolicy::Roof && !roofed && on_map && !water_bed
}

/// Does this cell want its roof torn off? The derived "ToUnroof"
/// designation.
pub fn wants_roof_removed(policy: RoofPolicy, roofed: bool) -> bool {
    policy == RoofPolicy::NoRoof && roofed
}

/// May a wall/door blueprint go on `cell`? Walkable plain ground, no
/// riverbed (authored `WaterMap::has_bed`, not live wetness — building on a
/// drained bed would flood the wall when the level comes back), nothing
/// already built there, and nothing standing on it that a wall would entomb
/// (`occupied` covers other blueprints, plants, and item stacks; pawns are
/// fine — they move). A stockpile zone does NOT block placement — it's a
/// logic area, not a physical claim on the cell, same as a Growing zone.
pub fn can_place_blueprint(
    terrain: &TerrainMap,
    construction: &ConstructionMap,
    nav: &NavGrid,
    water: &map::WaterMap,
    cell: GridCoords,
    occupied: impl Fn(GridCoords) -> bool,
) -> bool {
    nav.is_walkable(cell)
        && terrain.get(cell).is_some()
        && !water.has_bed(cell)
        && construction.get(cell).is_none()
        && !occupied(cell)
}

/// May `kind`'s whole footprint go at `anchor`? Every cell it would occupy
/// must individually pass `can_place_blueprint` — the multi-cell version a
/// 2x2 solar panel needs, single-cell buildings just check the one cell.
pub fn can_place_footprint(
    terrain: &TerrainMap,
    construction: &ConstructionMap,
    nav: &NavGrid,
    water: &map::WaterMap,
    kind: BuildableKind,
    anchor: GridCoords,
    occupied: impl Fn(GridCoords) -> bool,
) -> bool {
    footprint_cells(kind, anchor)
        .into_iter()
        .all(|cell| can_place_blueprint(terrain, construction, nav, water, cell, &occupied))
}

/// How much wood one Supply trip picks up: no more than the sites still
/// need, the trip cap, or what the source stack holds.
pub fn supply_load(need: u32, available: u32) -> u32 {
    need.min(SUPPLY_LOAD_CAP).min(available)
}

/// 4-way flood fill from `from` through non-boundary cells: `Some(region)`
/// if the region never touches the map border (a room interior), `None` if
/// it leaks to the border (open to the outside — the map edge carries no
/// walls). 4-way is deliberate: it matches the no-corner-cutting nav, so a
/// diagonal gap between two walls does not count as an opening.
pub fn enclosed_interior(
    from: GridCoords,
    is_boundary: impl Fn(GridCoords) -> bool,
) -> Option<Vec<GridCoords>> {
    let in_bounds =
        |cell: GridCoords| (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y);
    if !in_bounds(from) || is_boundary(from) {
        return None;
    }
    let mut visited: HashSet<GridCoords> = HashSet::from([from]);
    let mut queue = VecDeque::from([from]);
    let mut region = Vec::new();
    while let Some(cell) = queue.pop_front() {
        if cell.x == 0 || cell.x == MAP_WIDTH - 1 || cell.y == 0 || cell.y == MAP_HEIGHT - 1 {
            return None;
        }
        region.push(cell);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let neighbor = GridCoords::new(cell.x + dx, cell.y + dy);
            if in_bounds(neighbor) && !is_boundary(neighbor) && visited.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
    Some(region)
}

/// Auto-roof after a wall/door completes at `built`: any of its 4 neighbors
/// now sealed off from the map border gets the Roof policy stamped on its
/// whole interior, PLUS the walls/doors ringing that interior (8-way, so
/// corner walls — only diagonally adjacent to an interior cell — are caught
/// too) — a roof that stopped at the interior looked disconnected from the
/// walls holding it up. Except cells the player already stamped (a NoRoof
/// courtyard, or a wall they explicitly excluded, stays as stamped) and
/// cells a roof can't span (water).
fn auto_roof_around(
    built: GridCoords,
    terrain: &TerrainMap,
    water: &map::WaterMap,
    construction: &ConstructionMap,
    roofs: &mut RoofMap,
) {
    // Off-map cells read as `None` from `construction.get` too (same as
    // "nothing built there"), unlike the old `terrain.get`-based check
    // which special-cased `None` as a boundary. That's fine here:
    // `enclosed_interior`'s own bounds check catches an off-map `start`
    // and returns `None` regardless of what `is_boundary` said about it.
    let is_boundary = |cell: GridCoords| {
        matches!(
            construction.get(cell),
            Some(BuildableKind::Wall | BuildableKind::Door | BuildableKind::Cooler)
        )
    };
    let mut seen: HashSet<GridCoords> = HashSet::new();
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let start = GridCoords::new(built.x + dx, built.y + dy);
        if is_boundary(start) || seen.contains(&start) {
            continue;
        }
        let Some(region) = enclosed_interior(start, is_boundary) else {
            continue;
        };
        info!(
            "construction: room completed at {built:?}, auto-roofing {} cells",
            region.len()
        );
        seen.extend(region.iter().copied());
        for cell in &region {
            if roofs.policy(*cell) == RoofPolicy::None && is_roofable(terrain, water, *cell) {
                roofs.set_policy(*cell, RoofPolicy::Roof);
            }
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
                let wall = GridCoords::new(cell.x + dx, cell.y + dy);
                if matches!(
                    construction.get(wall),
                    Some(BuildableKind::Wall | BuildableKind::Door | BuildableKind::Cooler)
                ) && roofs.policy(wall) == RoofPolicy::None
                {
                    roofs.set_policy(wall, RoofPolicy::Roof);
                }
            }
        }
    }
}

/// Revert a demolished cell's construction/nav/roof state to plain ground.
/// Entity despawn happens in the caller (needs `Commands` — see
/// `zones::drag_zone_tool`'s `Tool::Demolish` arm); this is the pure part,
/// testable like `auto_roof_around`/`wants_roof`. Ground terrain was never
/// touched by building in the first place (see `ConstructionMap`'s docs),
/// so there's nothing to revert there and no ground tile to respawn either
/// — the original one has sat under the building the whole time.
///
/// Deliberately not a re-flood-fill of the (possibly now unenclosed)
/// interior: `auto_roof_around` only ever upgrades `RoofPolicy::None` cells
/// and never touches an existing `Roof`/`NoRoof` stamp, so there's no way
/// to tell "auto-derived Roof" apart from "player explicitly stamped Roof".
/// A symmetric auto-unroof pass here would risk silently erasing a
/// legitimate player stamp elsewhere in the same interior — so only this
/// cell's own roof state is cleared. A room whose wall gets demolished
/// through this devtool can be left with a phantom roof; that's a known,
/// accepted limitation of the devtool, not an oversight.
pub fn demolish_at(
    cell: GridCoords,
    construction: &mut ConstructionMap,
    nav: &mut NavGrid,
    roofs: &mut RoofMap,
) {
    construction.set(cell, None);
    nav.set_walkable(cell, true);
    nav.set_cost(cell, TERRAIN_COST_DIRT);
    roofs.set_policy(cell, RoofPolicy::None);
    roofs.set_roofed(cell, false);
}

/// Spawn a blueprint anchored at `cell`: the final silhouette as ghost
/// children under a data parent carrying the `Blueprint`. The parent's
/// `Transform` sits at the footprint's center (the anchor cell itself for
/// every 1x1 building, the middle of the block for a 2x2 solar panel) —
/// `cell` stays the anchor recorded on `GridCoords`/`Footprint` regardless.
pub fn spawn_blueprint(
    commands: &mut Commands,
    assets: &GameAssets,
    kind: BuildableKind,
    cell: GridCoords,
) {
    let mut entity = commands.spawn((
        Transform::from_translation(footprint_center_world(kind, cell)),
        Visibility::default(),
        Name::new(match kind {
            BuildableKind::Wall => "Wall blueprint",
            BuildableKind::Door => "Door blueprint",
            BuildableKind::Turbine => "Wind turbine blueprint",
            BuildableKind::SolarPanel => "Solar panel blueprint",
            BuildableKind::Lightpost => "Lightpost blueprint",
            BuildableKind::Battery => "Battery blueprint",
            BuildableKind::Bed(_) => "Bed blueprint",
            BuildableKind::Cooler => "Cooler blueprint",
        }),
        Blueprint { kind, delivered: 0 },
        cell,
        Footprint::new(kind, cell),
        Selectable,
    ));
    entity.with_children(|parent| match kind {
        BuildableKind::Wall => {
            parent.spawn((
                Mesh3d(assets.wall_mesh.clone()),
                MeshMaterial3d(assets.blueprint_material.clone()),
                Transform::from_translation(Vec3::Y * WALL_HEIGHT / 2.0),
            ));
        }
        BuildableKind::Door => {
            spawn_door_parts(parent, assets, &assets.blueprint_material);
        }
        BuildableKind::Turbine => {
            spawn_turbine_parts(parent, assets, cell, true);
        }
        BuildableKind::SolarPanel => {
            spawn_solar_parts(parent, assets, true);
        }
        BuildableKind::Lightpost => {
            spawn_lightpost_parts(parent, assets, true);
        }
        BuildableKind::Battery => {
            spawn_battery_parts(parent, assets, true);
        }
        BuildableKind::Bed(orientation) => {
            spawn_bed_parts(parent, assets, orientation, true);
        }
        BuildableKind::Cooler => {
            spawn_cooler_parts(parent, assets, true);
        }
    });
}

/// The turbine's parts — tower, nacelle pivot, rotor pivot, three blades —
/// shared by the ghost and the built turbine. The ghost gets the blueprint
/// material everywhere and no animation markers (blueprints hold still);
/// the built one gets per-part materials plus the `TurbineNacelle` /
/// `TurbineRotor` pivots the wind systems drive (the rotor carries its
/// `cell` so `spin_turbines` can read the local wind exposure). At identity
/// yaw the nacelle faces +X, so the rotor hub sits on its +X nose.
fn spawn_turbine_parts(
    parent: &mut ChildSpawnerCommands,
    assets: &GameAssets,
    cell: GridCoords,
    ghost: bool,
) {
    let material = |built: &Handle<StandardMaterial>| {
        if ghost {
            assets.blueprint_material.clone()
        } else {
            built.clone()
        }
    };
    parent.spawn((
        Mesh3d(assets.turbine_tower_mesh.clone()),
        MeshMaterial3d(material(&assets.turbine_tower_material)),
        Transform::from_translation(Vec3::Y * TURBINE_TOWER_HEIGHT / 2.0),
    ));
    let mut nacelle = parent.spawn((
        Transform::from_translation(Vec3::Y * TURBINE_TOWER_HEIGHT),
        Visibility::default(),
    ));
    if !ghost {
        nacelle.insert(TurbineNacelle);
    }
    nacelle.with_children(|nacelle| {
        nacelle.spawn((
            Mesh3d(assets.turbine_nacelle_mesh.clone()),
            MeshMaterial3d(material(&assets.turbine_nacelle_material)),
            Transform::default(),
        ));
        let mut rotor = nacelle.spawn((
            Transform::from_translation(
                Vec3::X * (TURBINE_NACELLE_LENGTH / 2.0 + TURBINE_BLADE_THICKNESS),
            ),
            Visibility::default(),
        ));
        if !ghost {
            rotor.insert((TurbineRotor, cell));
        }
        rotor.with_children(|rotor| {
            for blade in 0..3 {
                rotor.spawn((
                    Mesh3d(assets.turbine_blade_mesh.clone()),
                    MeshMaterial3d(material(&assets.turbine_blade_material)),
                    Transform::from_rotation(Quat::from_rotation_x(
                        blade as f32 * std::f32::consts::TAU / 3.0,
                    )),
                ));
            }
        });
    });
}

/// Turn every built nacelle toward the upwind direction, at a deliberate
/// max rate — with today's fixed wind this settles once, but it is the
/// piece that future smooth wind-direction changes animate. Sim-gated.
pub fn yaw_turbines(
    time: Res<Time>,
    wind: Res<crate::weather::Wind>,
    mut nacelles: Query<&mut Transform, (With<TurbineNacelle>, Without<TurbineRotor>)>,
) {
    let target = Quat::from_rotation_y(turbine_yaw(wind.direction));
    let max_step = TURBINE_YAW_SPEED * time.delta_secs();
    for mut transform in &mut nacelles {
        if transform.rotation != target {
            transform.rotation = transform.rotation.rotate_towards(target, max_step);
        }
    }
}

/// Spin every built rotor around its wind-facing axis, scaled by the live
/// (gusting) strength AND the cell's ALTITUDE wind exposure — a turbine
/// tucked into a forest's lee visibly idles, but a wall (too short to reach
/// rotor height) doesn't slow it. Sim-gated.
pub fn spin_turbines(
    time: Res<Time>,
    wind: Res<crate::weather::Wind>,
    exposure: Res<crate::weather::WindExposureMap>,
    mut rotors: Query<(&mut Transform, &GridCoords), (With<TurbineRotor>, Without<TurbineNacelle>)>,
) {
    let base_step = TURBINE_SPIN_PER_STRENGTH * wind.current * time.delta_secs();
    for (mut transform, cell) in &mut rotors {
        transform.rotate_local_x(
            base_step
                * exposure
                    .get(*cell, crate::weather::WindBand::Altitude)
                    .unwrap_or(1.0),
        );
    }
}

/// The solar panel's parts — a central post, a `SolarPanelPivot` that
/// `aim_solar_panels` yaws to the sun's azimuth, and a fixed-tilt `holder`
/// carrying a small grid of cells on a backing frame — shared by the ghost
/// and the built panel via the same ghost-material swap `spawn_turbine_parts`
/// uses. The parent's `Transform` already sits at the footprint's center
/// (`spawn_blueprint`/`complete_builds`), so the array sits centered on the
/// 2x2 block, inset well within it.
fn spawn_solar_parts(parent: &mut ChildSpawnerCommands, assets: &GameAssets, ghost: bool) {
    let material = |built: &Handle<StandardMaterial>| {
        if ghost {
            assets.blueprint_material.clone()
        } else {
            built.clone()
        }
    };
    parent.spawn((
        Mesh3d(assets.solar_post_mesh.clone()),
        MeshMaterial3d(material(&assets.solar_post_material)),
        Transform::from_translation(Vec3::Y * SOLAR_POST_HEIGHT / 2.0),
    ));
    let mut pivot = parent.spawn((
        Transform::from_translation(Vec3::Y * SOLAR_POST_HEIGHT),
        Visibility::default(),
    ));
    if !ghost {
        pivot.insert(SolarPanelPivot);
    }
    pivot.with_children(|pivot| {
        pivot
            .spawn((
                // Tilting about local X tips the flat plate's +Y normal
                // toward +Z, making Vec3::Z (not X) the panel's rest-forward
                // axis — `daynight::sun_yaw`'s doc comment derives the yaw
                // formula from that fact. Changing this axis requires
                // re-deriving `sun_yaw` too.
                Transform::from_rotation(Quat::from_rotation_x(SOLAR_ARRAY_TILT)),
                Visibility::default(),
            ))
            .with_children(|holder| {
                holder.spawn((
                    Mesh3d(assets.solar_frame_mesh.clone()),
                    MeshMaterial3d(material(&assets.solar_post_material)),
                    Transform::default(),
                ));
                let step = SOLAR_CELL_SIZE + SOLAR_CELL_GAP;
                let extent_x = (SOLAR_ARRAY_COLS as f32) * step - SOLAR_CELL_GAP;
                let extent_z = (SOLAR_ARRAY_ROWS as f32) * step - SOLAR_CELL_GAP;
                for row in 0..SOLAR_ARRAY_ROWS {
                    for col in 0..SOLAR_ARRAY_COLS {
                        let x = (col as f32 + 0.5) * step - extent_x / 2.0;
                        let z = (row as f32 + 0.5) * step - extent_z / 2.0;
                        holder.spawn((
                            Mesh3d(assets.solar_cell_mesh.clone()),
                            MeshMaterial3d(material(&assets.solar_panel_material)),
                            Transform::from_translation(Vec3::new(
                                x,
                                SOLAR_FRAME_THICKNESS / 2.0 + SOLAR_CELL_THICKNESS / 2.0,
                                z,
                            )),
                        ));
                    }
                }
            });
    });
}

/// Yaw every built solar panel's base to face the sun's azimuth during the
/// day — the array itself stays at its fixed tilt (`spawn_solar_parts`), so
/// this is a single-axis tracker. Visual only, like the turbine's yaw (no
/// power/energy accounting yet). At night the base eases back to the
/// sunrise heading, ready for the next day. Sim-gated, same chase pattern as
/// `yaw_turbines`.
pub fn aim_solar_panels(
    time: Res<Time>,
    clock: Res<GameClock>,
    mut pivots: Query<&mut Transform, With<SolarPanelPivot>>,
) {
    let progress = if clock.phase() == DayPhase::Day {
        clock.day_progress()
    } else {
        0.0
    };
    let target = Quat::from_rotation_y(daynight::sun_yaw(progress));
    let max_step = SOLAR_TRACK_SPEED * time.delta_secs();
    for mut transform in &mut pivots {
        if transform.rotation != target {
            transform.rotation = transform.rotation.rotate_towards(target, max_step);
        }
    }
}

/// The lightpost's parts — a pole and a lamp head — shared by the ghost and
/// the built lightpost via the same ghost-material swap
/// `spawn_turbine_parts`/`spawn_solar_parts` use. Only the built lamp head
/// carries a `PointLight` and the `LightpostLamp` marker `sync_lightposts`
/// drives; a ghost stays unlit like every other blueprint.
fn spawn_lightpost_parts(parent: &mut ChildSpawnerCommands, assets: &GameAssets, ghost: bool) {
    parent.spawn((
        Mesh3d(assets.lightpost_pole_mesh.clone()),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.lightpost_pole_material.clone()
        }),
        Transform::from_translation(Vec3::Y * LIGHTPOST_POLE_HEIGHT / 2.0),
    ));
    let mut lamp = parent.spawn((
        Mesh3d(assets.lightpost_lamp_mesh.clone()),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.lightpost_lamp_material.clone()
        }),
        Transform::from_translation(Vec3::Y * LIGHTPOST_POLE_HEIGHT),
    ));
    if !ghost {
        lamp.insert((
            LightpostLamp,
            PointLight {
                color: LIGHTPOST_LIGHT_COLOR,
                intensity: 0.0,
                range: LIGHTPOST_LIGHT_RANGE,
                shadows_enabled: false,
                ..default()
            },
        ));
    }
}

/// The bed's one part — a flat slab, no moving pieces, so unlike the other
/// multi-part buildings the ghost/built swap is just the material. Picks the
/// mesh matching `orientation` (`GameAssets::bed_mesh_horizontal`/
/// `bed_mesh_vertical` — two precomputed shapes rather than rotating one
/// mesh's `Transform` at runtime, which built structures never do here).
fn spawn_bed_parts(
    parent: &mut ChildSpawnerCommands,
    assets: &GameAssets,
    orientation: BedOrientation,
    ghost: bool,
) {
    let mesh = match orientation {
        BedOrientation::Horizontal => assets.bed_mesh_horizontal.clone(),
        BedOrientation::Vertical => assets.bed_mesh_vertical.clone(),
    };
    parent.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.bed_material.clone()
        }),
        Transform::from_translation(Vec3::Y * BED_HEIGHT / 2.0),
    ));
}

/// The cooler's parts — a wall-sized body (`wall_mesh`, so it sits flush in
/// a wall run) plus a small vent block on one face so it reads as machinery
/// rather than a plain wall panel — shared by the ghost and the built unit
/// via the same ghost-material swap the other buildings use. No moving
/// pieces and no `target_c`-driven visual yet (the info panel carries that).
fn spawn_cooler_parts(parent: &mut ChildSpawnerCommands, assets: &GameAssets, ghost: bool) {
    parent.spawn((
        Mesh3d(assets.wall_mesh.clone()),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.cooler_body_material.clone()
        }),
        Transform::from_translation(Vec3::Y * WALL_HEIGHT / 2.0),
    ));
    parent.spawn((
        Mesh3d(assets.cooler_vent_mesh.clone()),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.cooler_vent_material.clone()
        }),
        Transform::from_translation(Vec3::new(
            0.0,
            WALL_HEIGHT / 2.0,
            WALL_VISUAL_SIZE / 2.0 + COOLER_VENT_THICKNESS / 2.0,
        )),
    ));
}

/// Fade every built lightpost's lamp with the day/night clock AND the grid's
/// live coverage: dark by day regardless, and at night scaled by
/// `PowerGrid::powered_fraction` so an under-supplied grid dims every lamp
/// together instead of some staying lit while others go dark — cross-fading
/// through dusk and dawn like `daynight::move_sky_bodies` does for the
/// moon's `PointLight`. The lamp material is shared across every lightpost
/// (`GameAssets::lightpost_lamp_material`), so one `materials.get_mut` ramps
/// every lamp's glow together, mirroring `daynight::paint_sun_ball`.
pub fn sync_lightposts(
    clock: Res<GameClock>,
    grid: Res<crate::power::PowerGrid>,
    assets: Res<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lamps: Query<&mut PointLight, With<LightpostLamp>>,
) {
    let lit = (1.0 - clock.daylight()) * grid.powered_fraction;
    for mut light in &mut lamps {
        light.intensity = LIGHTPOST_LIGHT_INTENSITY * lit;
    }
    if let Some(material) = materials.get_mut(&assets.lightpost_lamp_material) {
        material.emissive = LIGHTPOST_LAMP_EMISSIVE * lit;
    }
}

/// The battery's parts — a boxy body and a charge-indicator strip on its
/// front face — shared by the ghost and the built battery via the same
/// ghost-material swap the other buildings use. Only the built strip
/// carries `BatteryStrip`, starting at the empty-charge material;
/// `sync_batteries` swaps it toward the full-charge material as
/// `Battery.charge` climbs.
fn spawn_battery_parts(parent: &mut ChildSpawnerCommands, assets: &GameAssets, ghost: bool) {
    parent.spawn((
        Mesh3d(assets.battery_body_mesh.clone()),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.battery_body_material.clone()
        }),
        Transform::from_translation(Vec3::Y * BATTERY_BODY_HEIGHT / 2.0),
    ));
    let mut strip = parent.spawn((
        Mesh3d(assets.battery_strip_mesh.clone()),
        MeshMaterial3d(if ghost {
            assets.blueprint_material.clone()
        } else {
            assets.battery_strip_materials[0].clone()
        }),
        Transform::from_translation(Vec3::new(
            0.0,
            BATTERY_BODY_HEIGHT / 2.0,
            BATTERY_BODY_SIZE / 2.0 + BATTERY_STRIP_THICKNESS / 2.0,
        )),
    ));
    if !ghost {
        strip.insert(BatteryStrip);
    }
}

/// Charge bucket for the strip material (4 steps mixing empty toward full
/// color), same shape as `cover::coverage_bucket`.
fn battery_bucket(fraction: f32) -> usize {
    ((fraction * 4.0) as usize).min(3)
}

/// Swap each built battery's strip to the material bucket matching its live
/// charge fraction — the same swap-a-shared-material approach every coverage
/// overlay uses (`cover::flora_material`, the wet/snow/wind/fuel overlays),
/// so no material is created at runtime. Ungated: `Battery.charge` is only
/// written by the sim-gated `power::tick_power`, so a frozen charge during
/// pause just freezes the strip color with it.
pub fn sync_batteries(
    assets: Res<GameAssets>,
    batteries: Query<(&Battery, &Children)>,
    mut strips: Query<&mut MeshMaterial3d<StandardMaterial>, With<BatteryStrip>>,
) {
    for (battery, children) in &batteries {
        let fraction = (battery.charge / BATTERY_CAPACITY).clamp(0.0, 1.0);
        let material = assets.battery_strip_materials[battery_bucket(fraction)].clone();
        for child in children {
            if let Ok(mut handle) = strips.get_mut(*child) {
                handle.0 = material.clone();
            }
        }
    }
}

/// Marker on a ghost previewing the armed building tool — the "landing
/// point" shown instead of relying on the mouse cursor alone. Companion to
/// `zones::drag_zone_tool`'s per-cell `PreviewTile`s (the footprint
/// outline): this is the building's own silhouette. A 1x1 building
/// (Wall/Door/Turbine) keeps one persistent entity, repositioned every
/// frame; the solar panel's anchor count varies with the drag, so its
/// ghosts are despawned and rebuilt fresh every frame instead — the same
/// tradeoff `PreviewTile` makes, justified there as cheap since drag
/// rectangles are small.
#[derive(Component)]
pub struct BuildGhost {
    kind: BuildableKind,
}

/// Show a translucent preview of the armed building tool. A 1x1 building
/// shows a single ghost following the hovered cell. A multi-cell building
/// (solar panel, bed) shows one ghost per anchor
/// `zones::snapped_footprint_anchors(drag_anchor, hovered, footprint_size)`
/// would place — the exact same call `drag_zone_tool` uses for the ground
/// highlight and the eventual commit, so the ghost can never disagree with
/// what a release actually builds.
#[allow(clippy::too_many_arguments)]
pub fn update_build_ghost(
    mut commands: Commands,
    assets: Res<GameAssets>,
    tool: Res<ActiveTool>,
    drag_anchor: Res<DragAnchor>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    ui_nodes: Query<&Interaction>,
    mut ghost: Query<(Entity, &BuildGhost, &mut Transform)>,
) {
    let kind = match tool.0 {
        Some(Tool::PlaceWall) => Some(BuildableKind::Wall),
        Some(Tool::PlaceDoor) => Some(BuildableKind::Door),
        Some(Tool::PlaceTurbine) => Some(BuildableKind::Turbine),
        Some(Tool::PlaceSolarPanel) => Some(BuildableKind::SolarPanel),
        Some(Tool::PlaceLightpost) => Some(BuildableKind::Lightpost),
        Some(Tool::PlaceBattery) => Some(BuildableKind::Battery),
        Some(Tool::PlaceBed(orientation)) => Some(BuildableKind::Bed(orientation)),
        Some(Tool::PlaceCooler) => Some(BuildableKind::Cooler),
        _ => None,
    };
    let over_ui = ui_nodes.iter().any(|i| *i != Interaction::None);
    let (camera, camera_transform) = *camera;
    let hovered = (!over_ui)
        .then(|| cursor_to_cell(&window, camera, camera_transform))
        .flatten();

    let Some((kind, cell)) = kind.zip(hovered) else {
        for (entity, ..) in &ghost {
            commands.entity(entity).despawn();
        }
        return;
    };

    if kind.footprint().len() > 1 {
        for (entity, ..) in &ghost {
            commands.entity(entity).despawn();
        }
        let step = footprint_size(kind);
        for anchor in snapped_footprint_anchors(drag_anchor.0.unwrap_or(cell), cell, step) {
            let world = footprint_center_world(kind, anchor);
            spawn_build_ghost(&mut commands, &assets, kind, world);
        }
        return;
    }

    // Reuse one matching-kind entity if there is one; despawn every other
    // leftover (e.g. the several stale per-anchor ghosts a solar-panel drag
    // can leave behind when the player switches to a 1x1 tool mid-frame).
    let world = footprint_center_world(kind, cell);
    let mut reused = false;
    for (entity, marker, mut transform) in &mut ghost {
        if !reused && marker.kind == kind {
            transform.translation = world;
            reused = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    if !reused {
        spawn_build_ghost(&mut commands, &assets, kind, world);
    }
}

/// The ghost's silhouette, keyed off the same per-kind `spawn_*_parts`
/// helpers the real blueprint uses, always at the shared `blueprint_material`.
fn spawn_build_ghost(
    commands: &mut Commands,
    assets: &GameAssets,
    kind: BuildableKind,
    world: Vec3,
) {
    commands
        .spawn((
            Transform::from_translation(world),
            Visibility::default(),
            Name::new("Build ghost"),
            BuildGhost { kind },
        ))
        .with_children(|parent| match kind {
            BuildableKind::Wall => {
                parent.spawn((
                    Mesh3d(assets.wall_mesh.clone()),
                    MeshMaterial3d(assets.blueprint_material.clone()),
                    Transform::from_translation(Vec3::Y * WALL_HEIGHT / 2.0),
                ));
            }
            BuildableKind::Door => {
                spawn_door_parts(parent, assets, &assets.blueprint_material);
            }
            BuildableKind::Turbine => {
                // Not anchored to a real cell (it may not even be
                // placeable), so the ghost's `cell` is a dummy — harmless,
                // since ghost=true skips the only thing that reads it
                // (`TurbineRotor`'s wind-exposure lookup).
                spawn_turbine_parts(parent, assets, GridCoords::new(0, 0), true);
            }
            BuildableKind::SolarPanel => {
                spawn_solar_parts(parent, assets, true);
            }
            BuildableKind::Lightpost => {
                spawn_lightpost_parts(parent, assets, true);
            }
            BuildableKind::Battery => {
                spawn_battery_parts(parent, assets, true);
            }
            BuildableKind::Bed(orientation) => {
                spawn_bed_parts(parent, assets, orientation, true);
            }
            BuildableKind::Cooler => {
                spawn_cooler_parts(parent, assets, true);
            }
        });
}

/// The door's three parts (two posts + lintel), shared by the ghost and the
/// built door — only the material differs.
fn spawn_door_parts(
    parent: &mut ChildSpawnerCommands,
    assets: &GameAssets,
    material: &Handle<StandardMaterial>,
) {
    for side in [-1.0, 1.0] {
        parent.spawn((
            Mesh3d(assets.door_post_mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(Vec3::new(side * DOOR_POST_OFFSET, WALL_HEIGHT / 2.0, 0.0)),
        ));
    }
    parent.spawn((
        Mesh3d(assets.door_lintel_mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(Vec3::Y * (WALL_HEIGHT - DOOR_POST_SIZE / 2.0)),
    ));
}

/// Bridge: LDtk `Wall` data entities become real walls. Sets `ConstructionMap`
/// + `NavGrid` and spawns the mesh — the same three things `complete_builds`
/// sets for a player-built wall, so LDtk-seeded and player-built walls end
/// up identical. No ground-tile backfill needed here (unlike the old
/// IntGrid-paint version of this bridge): a wall's cell is genuine natural
/// terrain now, so `map::spawn_tile_visuals` already gave it a ground tile
/// like any other cell.
pub fn spawn_walls_from_ldtk(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut construction: ResMut<ConstructionMap>,
    mut nav: ResMut<NavGrid>,
    spawns: Query<&GridCoords, Added<WallSpawn>>,
) {
    for grid in &spawns {
        construction.set(*grid, Some(BuildableKind::Wall));
        nav.set_walkable(*grid, false);
        commands.spawn((
            Mesh3d(assets.wall_mesh.clone()),
            MeshMaterial3d(assets.wall_material.clone()),
            // Standing on the tile tops at y = 0.
            Transform::from_translation(map::grid_to_world(grid) + Vec3::Y * WALL_HEIGHT / 2.0),
            Name::new("Wall"),
            *grid,
            Selectable,
            Wall,
        ));
    }
}

/// Turn completed builds into the real thing: despawn the blueprint, write
/// `ConstructionMap` + `NavGrid` (walls block, doors slow), spawn the
/// visual, then check for newly enclosed rooms. Ungated on purpose: the
/// commands are only written while running, and an ungated reader can't
/// lose one to the message double-buffer across a pause.
#[allow(clippy::too_many_arguments)]
pub fn complete_builds(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut builds: MessageReader<BuildCommand>,
    blueprints: Query<(&Blueprint, &GridCoords, &Footprint)>,
    terrain: Res<TerrainMap>,
    water: Res<map::WaterMap>,
    mut construction: ResMut<ConstructionMap>,
    mut nav: ResMut<NavGrid>,
    mut roofs: ResMut<RoofMap>,
) {
    for build in builds.read() {
        // A canceled blueprint: the command is stale, skip.
        let Ok((blueprint, grid, footprint)) = blueprints.get(build.site) else {
            continue;
        };
        info!("construction: {} built at {grid:?}", blueprint.kind.label());
        commands.entity(build.site).despawn();
        match blueprint.kind {
            BuildableKind::Wall => {
                construction.set(*grid, Some(BuildableKind::Wall));
                nav.set_walkable(*grid, false);
                // Same bundle as the LDtk bridge (spawn_walls_from_ldtk).
                commands.spawn((
                    Mesh3d(assets.wall_mesh.clone()),
                    MeshMaterial3d(assets.wall_material.clone()),
                    Transform::from_translation(
                        map::grid_to_world(grid) + Vec3::Y * WALL_HEIGHT / 2.0,
                    ),
                    Name::new("Wall"),
                    *grid,
                    footprint.clone(),
                    Selectable,
                    Wall,
                ));
            }
            BuildableKind::Door => {
                construction.set(*grid, Some(BuildableKind::Door));
                nav.set_cost(*grid, TERRAIN_COST_DOOR);
                commands
                    .spawn((
                        Transform::from_translation(map::grid_to_world(grid)),
                        Visibility::default(),
                        Name::new("Door"),
                        Door,
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_door_parts(parent, &assets, &assets.wall_material);
                    });
            }
            BuildableKind::Turbine => {
                construction.set(*grid, Some(BuildableKind::Turbine));
                nav.set_walkable(*grid, false);
                commands
                    .spawn((
                        Transform::from_translation(map::grid_to_world(grid)),
                        Visibility::default(),
                        Name::new("Wind turbine"),
                        Turbine,
                        PowerOutput::default(),
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_turbine_parts(parent, &assets, *grid, false);
                    });
            }
            BuildableKind::SolarPanel => {
                for cell in &footprint.cells {
                    construction.set(*cell, Some(BuildableKind::SolarPanel));
                    nav.set_walkable(*cell, false);
                }
                commands
                    .spawn((
                        Transform::from_translation(footprint_center_world(
                            BuildableKind::SolarPanel,
                            *grid,
                        )),
                        Visibility::default(),
                        Name::new("Solar panel"),
                        SolarPanel,
                        PowerOutput::default(),
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_solar_parts(parent, &assets, false);
                    });
            }
            BuildableKind::Lightpost => {
                construction.set(*grid, Some(BuildableKind::Lightpost));
                // No nav write: fully walkable, natural terrain cost stays —
                // unlike Door, which still raises the step cost.
                commands
                    .spawn((
                        Transform::from_translation(map::grid_to_world(grid)),
                        Visibility::default(),
                        Name::new("Lightpost"),
                        Lightpost,
                        PowerConsumer {
                            demand: LIGHTPOST_POWER_DRAW,
                        },
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_lightpost_parts(parent, &assets, false);
                    });
            }
            BuildableKind::Battery => {
                construction.set(*grid, Some(BuildableKind::Battery));
                nav.set_walkable(*grid, false);
                commands
                    .spawn((
                        Transform::from_translation(map::grid_to_world(grid)),
                        Visibility::default(),
                        Name::new("Battery"),
                        Battery { charge: 0.0 },
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_battery_parts(parent, &assets, false);
                    });
            }
            BuildableKind::Bed(orientation) => {
                for cell in &footprint.cells {
                    construction.set(*cell, Some(BuildableKind::Bed(orientation)));
                    // No nav write: fully walkable, like Lightpost — a
                    // sleeper must be able to stand on every cell.
                }
                commands
                    .spawn((
                        Transform::from_translation(footprint_center_world(
                            BuildableKind::Bed(orientation),
                            *grid,
                        )),
                        Visibility::default(),
                        Name::new("Bed"),
                        Bed,
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_bed_parts(parent, &assets, orientation, false);
                    });
            }
            BuildableKind::Cooler => {
                construction.set(*grid, Some(BuildableKind::Cooler));
                nav.set_walkable(*grid, false);
                commands
                    .spawn((
                        Transform::from_translation(map::grid_to_world(grid)),
                        Visibility::default(),
                        Name::new("Cooler"),
                        Cooler {
                            target_c: COOLER_DEFAULT_TARGET_C,
                        },
                        *grid,
                        footprint.clone(),
                        Selectable,
                    ))
                    .with_children(|parent| {
                        spawn_cooler_parts(parent, &assets, false);
                    });
            }
        }
        auto_roof_around(*grid, &terrain, &water, &construction, &mut roofs);
    }
}

/// Flip the physical roof bit; everything else (visuals, humidity, job
/// generation) derives from the map.
pub fn apply_roof_commands(mut roof_cmds: MessageReader<RoofCommand>, mut roofs: ResMut<RoofMap>) {
    for cmd in roof_cmds.read() {
        roofs.set_roofed(cmd.cell, cmd.install);
    }
}

/// Despawn a blueprint, pouring any `delivered` wood back onto the ground
/// at its site — the shared body behind both the Cancel button
/// (`cancel_blueprints`) and drag-placement erase-on-overlap
/// (`zones::drag_zone_tool`). `reason` is just for the log line. The Build
/// job (if any) goes void via `cleanup_jobs`; a Supply pawn en route simply
/// re-targets next frame.
#[allow(clippy::too_many_arguments)]
pub fn despawn_blueprint_with_refund(
    commands: &mut Commands,
    assets: &GameAssets,
    nav: &NavGrid,
    selected: &mut SelectedEntity,
    entity: Entity,
    blueprint: &Blueprint,
    grid: GridCoords,
    stacks: &mut Query<(&mut ItemStack, &GridCoords)>,
    occupied: &Query<&GridCoords, With<Selectable>>,
    reason: &str,
) {
    info!(
        "construction: {} blueprint at {grid:?} {reason} (refunding {})",
        blueprint.kind.label(),
        blueprint.delivered
    );
    if blueprint.delivered > 0 {
        let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
        items::pour_yield(
            commands,
            assets,
            ItemKind::Wood,
            blueprint.delivered,
            grid,
            stacks,
            &occupied_cells,
            |cell| nav.is_walkable(cell),
        );
    }
    commands.entity(entity).despawn();
    if selected.0 == Some(entity) {
        selected.0 = None;
    }
}

/// Despawn canceled blueprints; see `despawn_blueprint_with_refund`.
pub fn cancel_blueprints(
    mut commands: Commands,
    assets: Res<GameAssets>,
    nav: Res<NavGrid>,
    mut selected: ResMut<SelectedEntity>,
    canceled: Query<(Entity, &Blueprint, &GridCoords), With<ToCancel>>,
    occupied: Query<&GridCoords, With<Selectable>>,
    mut stacks: Query<(&mut ItemStack, &GridCoords)>,
) {
    for (entity, blueprint, grid) in &canceled {
        despawn_blueprint_with_refund(
            &mut commands,
            &assets,
            &nav,
            &mut selected,
            entity,
            blueprint,
            *grid,
            &mut stacks,
            &occupied,
            "canceled",
        );
    }
}

/// Tear down structures ordered by `DemolishCommand` (the Demolish job's
/// completion, `director::execute_jobs`): the built entity despawns, every
/// footprint cell's construction/nav/roof state reverts to plain ground
/// (`demolish_at`), and `DEMOLISH_REFUND_FRACTION` of the structure's wood
/// cost pours out at the demolisher's tile — the same shape as
/// `trees::fell_trees`. A structure's `BuildableKind` is read off
/// `ConstructionMap` at the anchor cell before `demolish_at` clears it: built
/// entities don't carry their kind as a plain field, `ConstructionMap` is the
/// single source of truth (the same map `complete_builds` writes it into).
#[allow(clippy::too_many_arguments)]
pub fn do_demolitions(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut demolitions: MessageReader<DemolishCommand>,
    sites: Query<(&GridCoords, &Footprint), With<ToDemolish>>,
    mut nav: ResMut<NavGrid>,
    mut construction: ResMut<ConstructionMap>,
    mut roofs: ResMut<RoofMap>,
    mut selected: ResMut<SelectedEntity>,
    occupied: Query<&GridCoords, With<Selectable>>,
    mut stacks: Query<(&mut ItemStack, &GridCoords)>,
) {
    for cmd in demolitions.read() {
        // A canceled or already-demolished target: the command is stale,
        // skip (mirrors `fell_trees`'s stale-`ToCut` guard).
        let Ok((grid, footprint)) = sites.get(cmd.site) else {
            continue;
        };
        let Some(kind) = construction.get(*grid) else {
            warn!(
                "construction: demolish target {:?} has no ConstructionMap entry at {grid:?}, skipping",
                cmd.site
            );
            continue;
        };
        let amount = (kind.wood_cost() as f32 * DEMOLISH_REFUND_FRACTION).round() as u32;
        info!(
            "construction: demolished {} at {grid:?} for {amount} wood",
            kind.label()
        );
        commands.entity(cmd.site).despawn();
        for cell in &footprint.cells {
            demolish_at(*cell, &mut construction, &mut nav, &mut roofs);
        }
        if selected.0 == Some(cmd.site) {
            selected.0 = None;
        }
        if amount > 0 {
            let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
            items::pour_yield(
                &mut commands,
                &assets,
                ItemKind::Wood,
                amount,
                cmd.drop_at,
                &mut stacks,
                &occupied_cells,
                |cell| nav.is_walkable(cell),
            );
        }
    }
}

/// Push one flat, upward-facing quad (two triangles) centered at `center`
/// with half-extent `half` in both local X and Z. Shared by both roof
/// surfaces below — the only difference between a built panel and a ghost
/// panel is which cells qualify and how big the square is.
fn push_roof_quad(positions: &mut Vec<[f32; 3]>, indices: &mut Vec<u32>, center: Vec3, half: f32) {
    let base = positions.len() as u32;
    positions.push([center.x - half, center.y, center.z - half]);
    positions.push([center.x - half, center.y, center.z + half]);
    positions.push([center.x + half, center.y, center.z + half]);
    positions.push([center.x + half, center.y, center.z - half]);
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Geometry for the two merged roof surfaces: every built-roof cell as a
/// full-tile quad, every planned-but-unbuilt cell (the "ToRoof" ghost) as a
/// smaller inset quad. Positions only — normals are a constant `+Y` the
/// caller fills in, same as `map::build_water_surface_geometry`.
pub fn build_roof_geometry(
    roofs: &RoofMap,
    terrain: &TerrainMap,
    water: &map::WaterMap,
) -> ((Vec<[f32; 3]>, Vec<u32>), (Vec<[f32; 3]>, Vec<u32>)) {
    let mut built_positions = Vec::new();
    let mut built_indices = Vec::new();
    let mut ghost_positions = Vec::new();
    let mut ghost_indices = Vec::new();
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            let center = map::grid_to_world(&cell) + Vec3::Y * ROOF_PANEL_Y;
            if roofs.is_roofed(cell) {
                push_roof_quad(&mut built_positions, &mut built_indices, center, TILE_SIZE / 2.0);
            } else if wants_roof(
                roofs.policy(cell),
                false,
                terrain.get(cell).is_some(),
                water.has_bed(cell),
            ) {
                push_roof_quad(
                    &mut ghost_positions,
                    &mut ghost_indices,
                    center,
                    TILE_SIZE * ROOF_GHOST_INSET / 2.0,
                );
            }
        }
    }
    ((built_positions, built_indices), (ghost_positions, ghost_indices))
}

/// Regenerate the two merged roof meshes from `RoofMap` — the
/// `map::rebuild_water_surface` model, one draw call per surface instead of
/// one translucent slab per cell (which used to sort-fight every frame and
/// read as a paved floor rather than a roof). Idle at steady state: only
/// rebuilds when `RoofMap` changed.
pub fn rebuild_roof_surfaces(
    mut commands: Commands,
    roofs: Res<RoofMap>,
    terrain: Res<TerrainMap>,
    water: Res<map::WaterMap>,
    assets: Res<GameAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut spawned: Local<bool>,
    mut built_surface: Query<&mut Visibility, (With<RoofSurfaceMesh>, Without<RoofGhostMesh>)>,
    mut ghost_surface: Query<&mut Visibility, (With<RoofGhostMesh>, Without<RoofSurfaceMesh>)>,
) {
    if !roofs.is_changed() {
        return;
    }
    let ((built_positions, built_indices), (ghost_positions, ghost_indices)) =
        build_roof_geometry(&roofs, &terrain, &water);
    let built_visible = if built_positions.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::default()
    };
    let ghost_visible = if ghost_positions.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::default()
    };

    if let Some(mesh) = meshes.get_mut(&assets.roof_mesh) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0, 1.0, 0.0]; built_positions.len()],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, built_positions);
        mesh.insert_indices(Indices::U32(built_indices));
    }
    if let Some(mesh) = meshes.get_mut(&assets.roof_ghost_mesh) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0, 1.0, 0.0]; ghost_positions.len()],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, ghost_positions);
        mesh.insert_indices(Indices::U32(ghost_indices));
    }

    if !*spawned {
        *spawned = true;
        commands.spawn((
            Mesh3d(assets.roof_mesh.clone()),
            MeshMaterial3d(assets.roof_material.clone()),
            Name::new("RoofSurface"),
            // A flat translucent overlay a hair above the walls/floor has no
            // business shadowing them — without this, the tessellated
            // coplanar mesh self-shadows at every cell boundary, showing up
            // as a grid of dark lines (same bug as `zones::ZoneTile`).
            NotShadowCaster,
            RoofSurfaceMesh,
            built_visible,
        ));
        commands.spawn((
            Mesh3d(assets.roof_ghost_mesh.clone()),
            MeshMaterial3d(assets.roof_ghost_material.clone()),
            Name::new("RoofGhostSurface"),
            NotShadowCaster,
            RoofGhostMesh,
            ghost_visible,
        ));
        return;
    }
    for mut visibility in &mut built_surface {
        *visibility = built_visible;
    }
    for mut visibility in &mut ghost_surface {
        *visibility = ghost_visible;
    }
}

/// Assign each built wall its autotiled mesh and rotate each built door to
/// align with its wall run, from `TerrainMap` connectivity. Idle at steady
/// state: only recomputes when terrain changed (a build happened) or a new
/// wall/door entity arrived (deferred spawn from `complete_builds` catching
/// up a frame late, see module docs).
#[allow(clippy::type_complexity)]
pub fn sync_wall_connections(
    assets: Res<GameAssets>,
    construction: Res<ConstructionMap>,
    added: Query<(), Or<(Added<Wall>, Added<Door>)>>,
    mut walls: Query<(&GridCoords, &mut Mesh3d), With<Wall>>,
    mut doors: Query<(&GridCoords, &mut Transform), With<Door>>,
) {
    if !construction.is_changed() && added.is_empty() {
        return;
    }
    let is_connective = |cell: GridCoords| {
        matches!(
            construction.get(cell),
            Some(BuildableKind::Wall | BuildableKind::Door | BuildableKind::Cooler)
        )
    };
    for (grid, mut mesh) in &mut walls {
        let mask = wall_connect_mask(*grid, is_connective);
        let want = assets.wall_meshes[mask as usize].clone();
        if mesh.0 != want {
            mesh.0 = want;
        }
    }
    for (grid, mut transform) in &mut doors {
        let vertical = door_is_vertical(wall_connect_mask(*grid, is_connective));
        let rot = if vertical {
            Quat::from_rotation_y(FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        if transform.rotation != rot {
            transform.rotation = rot;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    /// A 3x3 wall square centered on (10, 10): one interior cell.
    fn small_room() -> HashSet<GridCoords> {
        let mut walls = HashSet::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if (dx, dy) != (0, 0) {
                    walls.insert(at(10 + dx, 10 + dy));
                }
            }
        }
        walls
    }

    #[test]
    fn open_field_leaks_to_the_border() {
        assert_eq!(enclosed_interior(at(10, 10), |_| false), None);
    }

    #[test]
    fn walled_room_encloses_its_interior() {
        let walls = small_room();
        let region = enclosed_interior(at(10, 10), |cell| walls.contains(&cell)).unwrap();
        assert_eq!(region, vec![at(10, 10)]);
    }

    #[test]
    fn a_door_counts_as_wall_for_enclosure() {
        // Swap one wall for a door: the boundary predicate treats both as
        // boundary, so the room still encloses.
        let mut boundary = small_room();
        boundary.remove(&at(10, 9));
        let door = at(10, 9);
        let is_boundary = |cell: GridCoords| boundary.contains(&cell) || cell == door;
        let region = enclosed_interior(at(10, 10), is_boundary).unwrap();
        assert_eq!(region, vec![at(10, 10)]);
    }

    #[test]
    fn a_cardinal_gap_leaks_but_a_diagonal_one_does_not() {
        // Missing side wall: open.
        let mut walls = small_room();
        walls.remove(&at(10, 9));
        assert_eq!(
            enclosed_interior(at(10, 10), |cell| walls.contains(&cell)),
            None
        );
        // Missing corner only: 4-way flood can't slip through diagonally.
        let mut walls = small_room();
        walls.remove(&at(9, 9));
        assert!(enclosed_interior(at(10, 10), |cell| walls.contains(&cell)).is_some());
    }

    #[test]
    fn border_touching_regions_are_open() {
        // Start on the border itself.
        assert_eq!(enclosed_interior(at(0, 5), |_| false), None);
        // A "room" using the map edge as its fourth wall doesn't count.
        let mut walls = HashSet::new();
        for y in 0..=2 {
            walls.insert(at(2, y));
        }
        for x in 0..=2 {
            walls.insert(at(x, 2));
        }
        assert_eq!(
            enclosed_interior(at(1, 1), |cell| walls.contains(&cell)),
            None
        );
    }

    #[test]
    fn boundary_start_is_not_a_region() {
        let walls = small_room();
        assert_eq!(
            enclosed_interior(at(9, 9), |cell| walls.contains(&cell)),
            None
        );
    }

    #[test]
    fn turbines_face_upwind() {
        // Rotating +X by the yaw must land on -wind_dir (facing into the
        // incoming flow).
        for dir in [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(1.0, 0.35).normalize(),
            Vec2::new(-0.6, 0.8).normalize(),
        ] {
            let facing = Quat::from_rotation_y(turbine_yaw(dir)) * Vec3::X;
            let expected = Vec3::new(-dir.x, 0.0, -dir.y);
            assert!(
                (facing - expected).length() < 1e-5,
                "wind {dir}: facing {facing}, expected {expected}"
            );
        }
    }

    #[test]
    fn zero_cost_blueprints_are_born_supplied() {
        let blueprint = Blueprint {
            kind: BuildableKind::Turbine,
            delivered: 0,
        };
        assert_eq!(blueprint.needed(), 0);
        assert!(blueprint.is_supplied());
    }

    #[test]
    fn supply_load_caps_by_need_trip_and_stock() {
        assert_eq!(supply_load(5, 80), 5);
        assert_eq!(supply_load(100, 80), SUPPLY_LOAD_CAP);
        assert_eq!(supply_load(100, 12), 12);
        assert_eq!(supply_load(0, 80), 0);
        assert_eq!(supply_load(5, 0), 0);
    }

    #[test]
    fn blueprints_track_their_wood() {
        let mut blueprint = Blueprint {
            kind: BuildableKind::Wall,
            delivered: 0,
        };
        assert_eq!(blueprint.needed(), WALL_WOOD_COST);
        assert!(!blueprint.is_supplied());
        blueprint.delivered = 3;
        assert_eq!(blueprint.needed(), WALL_WOOD_COST - 3);
        blueprint.delivered = WALL_WOOD_COST;
        assert_eq!(blueprint.needed(), 0);
        assert!(blueprint.is_supplied());
        // Over-delivery (shouldn't happen) still saturates safely.
        blueprint.delivered = WALL_WOOD_COST + 2;
        assert_eq!(blueprint.needed(), 0);
    }

    #[test]
    fn roofs_want_building_only_on_bare_stamped_ground() {
        // (policy, roofed, on_map, water_bed)
        assert!(wants_roof(RoofPolicy::Roof, false, true, false));
        assert!(!wants_roof(RoofPolicy::Roof, true, true, false));
        assert!(!wants_roof(RoofPolicy::None, false, true, false));
        assert!(!wants_roof(RoofPolicy::NoRoof, false, true, false));
        // Only riverbeds (and off-map) never roof — walls/doors sit on Dirt
        // or Fertile ground (their terrain never changes when built on, see
        // `ConstructionMap`) and take a panel too, exercised in
        // `auto_roof_respects_player_stamps_and_water` below via the
        // construction-aware `auto_roof_around`. The bed gate holds even
        // drained: the water can come back.
        assert!(!wants_roof(RoofPolicy::Roof, false, true, true));
        assert!(!wants_roof(RoofPolicy::Roof, false, false, false));
    }

    #[test]
    fn roofs_want_removal_only_when_roofed_under_noroof() {
        assert!(wants_roof_removed(RoofPolicy::NoRoof, true));
        assert!(!wants_roof_removed(RoofPolicy::NoRoof, false));
        assert!(!wants_roof_removed(RoofPolicy::Roof, true));
        assert!(!wants_roof_removed(RoofPolicy::None, true));
    }

    #[test]
    fn auto_roof_respects_player_stamps_and_water() {
        let terrain = TerrainMap::default();
        let mut construction = ConstructionMap::default();
        // 4x3 room interior: (10,10), (11,10) with (11,10) being a
        // riverbed cell (water is WaterMap's business now, not terrain's).
        for x in 9..=12 {
            construction.set(at(x, 9), Some(BuildableKind::Wall));
            construction.set(at(x, 11), Some(BuildableKind::Wall));
        }
        construction.set(at(9, 10), Some(BuildableKind::Wall));
        construction.set(at(12, 10), Some(BuildableKind::Door));
        let mut water = map::WaterMap::default();
        water.set_bed(at(11, 10), WATER_BED_SHALLOW);

        let mut roofs = RoofMap::default();
        roofs.set_policy(at(10, 10), RoofPolicy::NoRoof);
        // The player also explicitly excluded one perimeter wall.
        roofs.set_policy(at(9, 9), RoofPolicy::NoRoof);
        auto_roof_around(at(12, 10), &terrain, &water, &construction, &mut roofs);
        // The player's NoRoof stamps survive (interior and wall alike);
        // water stays open sky.
        assert_eq!(roofs.policy(at(10, 10)), RoofPolicy::NoRoof);
        assert_eq!(roofs.policy(at(11, 10)), RoofPolicy::None);
        assert_eq!(roofs.policy(at(9, 9)), RoofPolicy::NoRoof);
        // Un-stamped perimeter walls still got roofed.
        assert_eq!(roofs.policy(at(10, 9)), RoofPolicy::Roof);

        // Same room, no stamps: the dirt cell gets auto-roofed, and so does
        // the entire wall/door ring around it — the roof now reaches the
        // structure, not just the interior.
        let mut roofs = RoofMap::default();
        auto_roof_around(at(12, 10), &terrain, &water, &construction, &mut roofs);
        assert_eq!(roofs.policy(at(10, 10)), RoofPolicy::Roof);
        assert_eq!(roofs.policy(at(11, 10)), RoofPolicy::None);
        for x in 9..=12 {
            assert_eq!(roofs.policy(at(x, 9)), RoofPolicy::Roof);
            assert_eq!(roofs.policy(at(x, 11)), RoofPolicy::Roof);
        }
        assert_eq!(roofs.policy(at(9, 10)), RoofPolicy::Roof);
        assert_eq!(roofs.policy(at(12, 10)), RoofPolicy::Roof); // the door
    }

    #[test]
    fn demolish_reverts_construction_nav_and_the_cells_own_roof_state() {
        let mut construction = ConstructionMap::default();
        construction.set(at(5, 5), Some(BuildableKind::Wall));
        let mut nav = NavGrid::default();
        nav.set_walkable(at(5, 5), false);
        let mut roofs = RoofMap::default();
        roofs.set_policy(at(5, 5), RoofPolicy::Roof);
        roofs.set_roofed(at(5, 5), true);

        demolish_at(at(5, 5), &mut construction, &mut nav, &mut roofs);

        assert_eq!(construction.get(at(5, 5)), None);
        assert!(nav.is_walkable(at(5, 5)));
        assert_eq!(nav.cost(at(5, 5)), TERRAIN_COST_DIRT);
        assert_eq!(roofs.policy(at(5, 5)), RoofPolicy::None);
        assert!(!roofs.is_roofed(at(5, 5)));
    }

    #[test]
    fn demolish_resets_a_door_cost_back_to_dirt_baseline() {
        let mut construction = ConstructionMap::default();
        construction.set(at(5, 5), Some(BuildableKind::Door));
        let mut nav = NavGrid::default();
        nav.set_cost(at(5, 5), TERRAIN_COST_DOOR);
        let mut roofs = RoofMap::default();

        demolish_at(at(5, 5), &mut construction, &mut nav, &mut roofs);

        assert_eq!(construction.get(at(5, 5)), None);
        assert!(nav.is_walkable(at(5, 5)));
        assert_eq!(nav.cost(at(5, 5)), TERRAIN_COST_DIRT);
    }

    #[test]
    fn blueprints_go_on_clear_walkable_ground() {
        let terrain = TerrainMap::default();
        let construction = ConstructionMap::default();
        let nav = NavGrid::default();
        let dry = map::WaterMap::default();
        let free = |_| false;
        assert!(can_place_blueprint(
            &terrain,
            &construction,
            &nav,
            &dry,
            at(5, 5),
            free
        ));
        // Occupied cell: no.
        assert!(!can_place_blueprint(
            &terrain,
            &construction,
            &nav,
            &dry,
            at(5, 5),
            |c| c == at(5, 5)
        ));
        // A riverbed: no — even drained (no body built here, so the cell
        // holds no live water at all), because the water can come back.
        let mut bed = map::WaterMap::default();
        bed.set_bed(at(5, 5), WATER_BED_SHALLOW);
        assert!(!can_place_blueprint(
            &terrain,
            &construction,
            &nav,
            &bed,
            at(5, 5),
            free
        ));
        // Already built: no.
        let mut construction = ConstructionMap::default();
        construction.set(at(5, 5), Some(BuildableKind::Wall));
        assert!(!can_place_blueprint(
            &terrain,
            &construction,
            &nav,
            &dry,
            at(5, 5),
            free
        ));
        // Unwalkable: no.
        let construction = ConstructionMap::default();
        let mut nav = NavGrid::default();
        nav.set_walkable(at(5, 5), false);
        assert!(!can_place_blueprint(
            &terrain,
            &construction,
            &nav,
            &dry,
            at(5, 5),
            free
        ));
    }

    #[test]
    fn single_cell_buildings_have_a_one_cell_footprint() {
        for kind in [
            BuildableKind::Wall,
            BuildableKind::Door,
            BuildableKind::Turbine,
            BuildableKind::Lightpost,
        ] {
            assert_eq!(footprint_cells(kind, at(5, 5)), vec![at(5, 5)]);
        }
    }

    #[test]
    fn solar_panel_footprint_spans_a_2x2_block() {
        let cells = footprint_cells(BuildableKind::SolarPanel, at(5, 5));
        assert_eq!(cells, vec![at(5, 5), at(6, 5), at(5, 6), at(6, 6)]);
    }

    #[test]
    fn footprint_center_matches_the_single_cell_for_1x1_buildings() {
        for kind in [
            BuildableKind::Wall,
            BuildableKind::Door,
            BuildableKind::Turbine,
            BuildableKind::Lightpost,
        ] {
            assert_eq!(
                footprint_center_world(kind, at(5, 5)),
                map::grid_to_world(&at(5, 5))
            );
        }
    }

    #[test]
    fn footprint_center_sits_between_the_solar_panels_four_cells() {
        let center = footprint_center_world(BuildableKind::SolarPanel, at(5, 5));
        let expected = (map::grid_to_world(&at(5, 5))
            + map::grid_to_world(&at(6, 5))
            + map::grid_to_world(&at(5, 6))
            + map::grid_to_world(&at(6, 6)))
            / 4.0;
        assert_eq!(center, expected);
    }

    #[test]
    fn can_place_footprint_requires_every_cell_to_pass() {
        let terrain = TerrainMap::default();
        let nav = NavGrid::default();
        let dry = map::WaterMap::default();
        let free = |_| false;
        let mut construction = ConstructionMap::default();
        assert!(can_place_footprint(
            &terrain,
            &construction,
            &nav,
            &dry,
            BuildableKind::SolarPanel,
            at(5, 5),
            free
        ));
        // One cell of the 2x2 already built on: the whole footprint fails.
        construction.set(at(6, 6), Some(BuildableKind::Wall));
        assert!(!can_place_footprint(
            &terrain,
            &construction,
            &nav,
            &dry,
            BuildableKind::SolarPanel,
            at(5, 5),
            free
        ));
    }

    #[test]
    fn footprint_contains_checks_every_cell_not_just_the_anchor() {
        let footprint = Footprint::new(BuildableKind::SolarPanel, at(5, 5));
        assert!(footprint.contains(at(5, 5)));
        assert!(footprint.contains(at(6, 6)));
        assert!(!footprint.contains(at(7, 5)));
    }

    #[test]
    fn wall_connect_mask_reads_the_four_neighbors() {
        let none = |_| false;
        assert_eq!(wall_connect_mask(at(5, 5), none), 0);

        let north_only: HashSet<GridCoords> = HashSet::from([at(5, 6)]);
        assert_eq!(
            wall_connect_mask(at(5, 5), |c| north_only.contains(&c)),
            CONNECT_N
        );
        let east_only: HashSet<GridCoords> = HashSet::from([at(6, 5)]);
        assert_eq!(
            wall_connect_mask(at(5, 5), |c| east_only.contains(&c)),
            CONNECT_E
        );
        let south_only: HashSet<GridCoords> = HashSet::from([at(5, 4)]);
        assert_eq!(
            wall_connect_mask(at(5, 5), |c| south_only.contains(&c)),
            CONNECT_S
        );
        let west_only: HashSet<GridCoords> = HashSet::from([at(4, 5)]);
        assert_eq!(
            wall_connect_mask(at(5, 5), |c| west_only.contains(&c)),
            CONNECT_W
        );

        // Straight E-W run.
        let ew_run: HashSet<GridCoords> = HashSet::from([at(4, 5), at(6, 5)]);
        assert_eq!(
            wall_connect_mask(at(5, 5), |c| ew_run.contains(&c)),
            CONNECT_E | CONNECT_W
        );
        // Corner: N + E.
        let corner: HashSet<GridCoords> = HashSet::from([at(5, 6), at(6, 5)]);
        assert_eq!(
            wall_connect_mask(at(5, 5), |c| corner.contains(&c)),
            CONNECT_N | CONNECT_E
        );

        // A door neighbor counts as connective too.
        let mut construction = ConstructionMap::default();
        construction.set(at(5, 6), Some(BuildableKind::Door));
        let is_connective = |c: GridCoords| {
            matches!(
                construction.get(c),
                Some(BuildableKind::Wall | BuildableKind::Door | BuildableKind::Cooler)
            )
        };
        assert_eq!(wall_connect_mask(at(5, 5), is_connective), CONNECT_N);
    }

    /// A cooler stands in for a wall segment in a run — a neighboring wall
    /// must still autotile as if the run continues through it.
    #[test]
    fn a_cooler_neighbor_counts_as_connective_too() {
        let mut construction = ConstructionMap::default();
        construction.set(at(5, 6), Some(BuildableKind::Cooler));
        let is_connective = |c: GridCoords| {
            matches!(
                construction.get(c),
                Some(BuildableKind::Wall | BuildableKind::Door | BuildableKind::Cooler)
            )
        };
        assert_eq!(wall_connect_mask(at(5, 5), is_connective), CONNECT_N);
    }

    #[test]
    fn door_orientation_follows_the_dominant_run() {
        assert!(door_is_vertical(CONNECT_N | CONNECT_S));
        assert!(!door_is_vertical(CONNECT_E | CONNECT_W));
        // Isolated: default horizontal.
        assert!(!door_is_vertical(0));
        // T-junction (three neighbors): horizontal unless N-S strictly dominates.
        assert!(!door_is_vertical(CONNECT_N | CONNECT_E | CONNECT_W));
        assert!(door_is_vertical(CONNECT_N | CONNECT_S | CONNECT_E));
    }

    #[test]
    fn connected_wall_mesh_grows_a_vertex_delta_per_arm() {
        let post = connected_wall_mesh(0);
        let post_vertices = post.count_vertices();

        let one_arm = connected_wall_mesh(CONNECT_N);
        let arm_delta = one_arm.count_vertices() - post_vertices;
        assert!(arm_delta > 0);

        let two_arms = connected_wall_mesh(CONNECT_N | CONNECT_S);
        assert_eq!(two_arms.count_vertices(), post_vertices + 2 * arm_delta);

        let all_arms = connected_wall_mesh(CONNECT_N | CONNECT_E | CONNECT_S | CONNECT_W);
        assert_eq!(all_arms.count_vertices(), post_vertices + 4 * arm_delta);
    }

    #[test]
    fn only_movement_blocking_kinds_report_blocks_movement() {
        assert!(BuildableKind::Wall.blocks_movement());
        assert!(BuildableKind::Turbine.blocks_movement());
        assert!(BuildableKind::SolarPanel.blocks_movement());
        assert!(!BuildableKind::Door.blocks_movement());
        assert!(!BuildableKind::Lightpost.blocks_movement());
    }

    #[test]
    fn lightpost_is_a_cheap_walkable_1x1_building() {
        assert_eq!(BuildableKind::Lightpost.wood_cost(), 2);
        assert!(!BuildableKind::Lightpost.blocks_movement());
        assert_eq!(BuildableKind::Lightpost.footprint(), &[(0, 0)]);
    }
}
