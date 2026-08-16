//! Player-editable zones. Stockpiles are the first kind; the `Zone`/
//! `ZoneRegion` split (what it is vs. where it is) is meant to grow more
//! kinds later without touching the storage model.
use std::collections::HashSet;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::construction::{self, BedOrientation, Blueprint, ConstructionMap, Footprint};
use crate::crops::Crop;
use crate::flora::BerryBush;
use crate::game::GameAssets;
use crate::items::ItemStack;
use crate::map::{self, RoofMap, RoofPolicy, Terrain, TerrainMap};
use crate::nav::NavGrid;
use crate::scene::MainCamera;
use crate::selection::{cursor_to_cell, Selectable, SelectedEntity};
use crate::shrubs::Shrub;
use crate::trees::{ToCut, Tree};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    Stockpile,
    Growing,
    /// Auto-managed only: `temperature::sync_rooms` derives Room zones from
    /// the wall/door layout — never player-drawn, never selectable, and
    /// exempt from the one-zone-per-tile rule (a stockpile can sit inside a
    /// room; that's the point of building one).
    Room,
}

impl ZoneKind {
    pub fn label(&self) -> &'static str {
        match self {
            ZoneKind::Stockpile => "Stockpile",
            ZoneKind::Growing => "Growing zone",
            ZoneKind::Room => "Room",
        }
    }
}

#[derive(Component)]
pub struct Zone {
    pub kind: ZoneKind,
}

/// Whether a growing zone is actively taking Farming jobs (the Director
/// only plants on cells of zones with this on). Toggled by the Grow
/// checkbox in the selection info panel; only ever present on
/// `ZoneKind::Growing` zones.
#[derive(Component)]
pub struct GrowOrder {
    pub enabled: bool,
}

/// What a growing zone plants — cycled from the selection info panel; only
/// ever present on `ZoneKind::Growing` zones (alongside `GrowOrder`).
#[derive(Component, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ZonePlant {
    #[default]
    Wheat,
    Trees,
}

impl ZonePlant {
    pub fn label(&self) -> &'static str {
        match self {
            ZonePlant::Wheat => "Wheat",
            ZonePlant::Trees => "Trees",
        }
    }

    /// The next choice in the panel's cycle button.
    pub fn next(&self) -> ZonePlant {
        match self {
            ZonePlant::Wheat => ZonePlant::Trees,
            ZonePlant::Trees => ZonePlant::Wheat,
        }
    }

    /// May this plant go on `cell`? Wheat packs every zone cell; trees only
    /// take a spaced grid (one per `TREE_PLANT_SPACING` tiles on both axes,
    /// aligned to the map grid) — an orchard, not a wall of wood.
    pub fn accepts_cell(&self, cell: GridCoords) -> bool {
        match self {
            ZonePlant::Wheat => true,
            ZonePlant::Trees => {
                cell.x.rem_euclid(TREE_PLANT_SPACING) == 0
                    && cell.y.rem_euclid(TREE_PLANT_SPACING) == 0
            }
        }
    }
}

/// A zone's shape: an arbitrary set of cells (not necessarily rectangular,
/// contiguous, or hole-free — Create/Expand/Shrink all operate on it via
/// plain set membership). Every consumer here (terrain, tiles, hauling,
/// click-hit) already works cell-by-cell, so this stays cheap regardless of
/// map size: it scales with a zone's own footprint, not the map's area.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneRegion {
    cells: HashSet<GridCoords>,
}

impl ZoneRegion {
    pub fn from_cells(cells: impl IntoIterator<Item = GridCoords>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
        }
    }

    pub fn contains(&self, cell: GridCoords) -> bool {
        self.cells.contains(&cell)
    }

    pub fn cells(&self) -> impl Iterator<Item = GridCoords> + '_ {
        self.cells.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Expand: add cells (Create/Expand orders paint onto the set).
    pub fn extend(&mut self, cells: impl IntoIterator<Item = GridCoords>) {
        self.cells.extend(cells);
    }

    /// Shrink: drop a single cell from the set (a Shrink drag removes
    /// exactly what it covers, cell by cell — no rectangle heuristics).
    pub fn remove(&mut self, cell: GridCoords) {
        self.cells.remove(&cell);
    }
}

/// Normalize two drag corners into a rectangle of cells, clamped to the map
/// bounds. The *drawn* selection is still a rectangle (that's what a mouse
/// drag naturally is) — only the stored `ZoneRegion` is a free-form set, so
/// Create/Expand/Shrink all funnel their drag input through this.
fn rect_cells(a: GridCoords, b: GridCoords) -> Vec<GridCoords> {
    let clamp_x = |v: i32| v.clamp(0, MAP_WIDTH - 1);
    let clamp_y = |v: i32| v.clamp(0, MAP_HEIGHT - 1);
    let min = GridCoords::new(clamp_x(a.x.min(b.x)), clamp_y(a.y.min(b.y)));
    let max = GridCoords::new(clamp_x(a.x.max(b.x)), clamp_y(a.y.max(b.y)));
    (min.y..=max.y)
        .flat_map(move |y| (min.x..=max.x).map(move |x| GridCoords::new(x, y)))
        .collect()
}

/// Normalize a drag into a single one-cell-thick line along whichever axis
/// the cursor moved further on — a run of walls should be a wall, not a
/// filled square. Ties (equal |dx| and |dy|, including a zero-length drag)
/// go horizontal. The off-axis coordinate is held at `a`'s, so the line
/// always starts exactly where the drag started. Clamped to the map bounds
/// like `rect_cells`.
fn line_cells(a: GridCoords, b: GridCoords) -> Vec<GridCoords> {
    let clamp_x = |v: i32| v.clamp(0, MAP_WIDTH - 1);
    let clamp_y = |v: i32| v.clamp(0, MAP_HEIGHT - 1);
    let a = GridCoords::new(clamp_x(a.x), clamp_y(a.y));
    let b = GridCoords::new(clamp_x(b.x), clamp_y(b.y));
    if (b.x - a.x).abs() >= (b.y - a.y).abs() {
        let (min_x, max_x) = (a.x.min(b.x), a.x.max(b.x));
        (min_x..=max_x).map(|x| GridCoords::new(x, a.y)).collect()
    } else {
        let (min_y, max_y) = (a.y.min(b.y), a.y.max(b.y));
        (min_y..=max_y).map(|y| GridCoords::new(a.x, y)).collect()
    }
}

/// Anchor cells for a `step`-sized grid covering the drag rectangle `a..b` —
/// what a multi-cell building (`Tool::PlaceSolarPanel`, `Tool::PlaceBed`)
/// tiles instead of filling every drawn cell 1:1. `step` should always be
/// `construction::footprint_size(kind)`, so the tiling can never drift out of
/// sync with the shape it's tiling.
///
/// Anchors start exactly at the drag's own (clamped) min corner — wherever
/// the player pressed or clicked — and step across the rectangle. So a plain
/// click always lands one full unit anchored at the clicked cell, and an
/// N-wide drag fits floor(N/step)+1 units per axis anchored from that start.
/// Unlike a fixed global grid, two drags with different starting parities
/// can therefore produce overlapping anchors; `can_place_footprint` at
/// commit time is the existing backstop that silently skips any anchor
/// whose cells are already occupied or off the map.
pub(crate) fn snapped_footprint_anchors(
    a: GridCoords,
    b: GridCoords,
    step: (i32, i32),
) -> Vec<GridCoords> {
    let clamp_x = |v: i32| v.clamp(0, MAP_WIDTH - 1);
    let clamp_y = |v: i32| v.clamp(0, MAP_HEIGHT - 1);
    let min = GridCoords::new(clamp_x(a.x.min(b.x)), clamp_y(a.y.min(b.y)));
    let max = GridCoords::new(clamp_x(a.x.max(b.x)), clamp_y(a.y.max(b.y)));
    let (step_x, step_y) = step;
    let mut anchors = Vec::new();
    let mut x = min.x;
    while x <= max.x {
        let mut y = min.y;
        while y <= max.y {
            anchors.push(GridCoords::new(x, y));
            y += step_y;
        }
        x += step_x;
    }
    anchors
}

/// One overlay tile per cell of a zone; rebuilt whenever the zone's region
/// changes.
#[derive(Component)]
pub struct ZoneTile {
    zone: Entity,
}

/// Despawn a zone and every overlay tile it owns.
pub fn despawn_zone(commands: &mut Commands, zone: Entity, tiles: &Query<(Entity, &ZoneTile)>) {
    for (tile_entity, tile) in tiles {
        if tile.zone == zone {
            commands.entity(tile_entity).despawn();
        }
    }
    commands.entity(zone).despawn();
}

/// A just-placed building claims its cells outright: evict them from
/// whatever zone owned them, as if the zone had been manually shrunk around
/// the new footprint. Shared by the wall/door/turbine and solar-panel arms
/// of `drag_zone_tool`.
fn evict_zones_for_building(
    commands: &mut Commands,
    zones: &mut Query<(Entity, &Zone, &mut ZoneRegion)>,
    zone_tiles: &Query<(Entity, &ZoneTile)>,
    selected: &mut SelectedEntity,
    placed: &HashSet<GridCoords>,
) {
    for (zone_entity, _, mut region) in zones.iter_mut() {
        if !placed.iter().any(|cell| region.contains(*cell)) {
            continue;
        }
        for cell in placed {
            region.remove(*cell);
        }
        if region.is_empty() {
            despawn_zone(commands, zone_entity, zone_tiles);
            if selected.0 == Some(zone_entity) {
                selected.0 = None;
            }
        }
    }
}

/// The tool the player currently has armed (from the general Actions bar or
/// a zone's Expand/Shrink order). `None` means normal selection/move clicks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    PlaceStockpile,
    PlaceGrowing,
    Expand(Entity),
    Shrink(Entity),
    /// Fill the dragged cells with wall/door/turbine blueprints.
    PlaceWall,
    PlaceDoor,
    PlaceTurbine,
    /// Tile the dragged rectangle with 2x2 solar-panel blueprints, tiled
    /// from the drag's own start cell (see `snapped_footprint_anchors`)
    /// rather than filling every drawn cell like the single-tile buildings
    /// above.
    PlaceSolarPanel,
    /// Fill the dragged cells with lightpost blueprints, same 1x1 fill as
    /// wall/door/turbine.
    PlaceLightpost,
    /// Fill the dragged cells with battery blueprints, same 1x1 fill as
    /// wall/door/turbine/lightpost.
    PlaceBattery,
    /// Fill the dragged cells with cooler blueprints, same 1x1 fill as
    /// wall/door/turbine/lightpost/battery — a cooler stands in a wall run.
    PlaceCooler,
    /// Tile the dragged rectangle with 2x1 (or 1x2) bed blueprints, same
    /// `snapped_footprint_anchors` tiling as the solar panel. The orientation
    /// is a live toggle, not a separate tool per direction: R flips it while
    /// armed (`drag_zone_tool`), so the ghost/preview/commit all read the
    /// current value from here.
    PlaceBed(BedOrientation),
    /// Stamp a roof policy onto the dragged cells (see `map::RoofPolicy`) —
    /// these are paint orders on the map, no zone entity is created.
    StampRoof,
    StampNoRoof,
    StampClearRoof,
    /// Mark every cuttable tree in the dragged cells with `ToCut` — saplings
    /// are skipped via `Tree::is_cuttable`, same gate as the per-tile button.
    DesignateCut,
    /// Remove `ToCut` from every marked tree in the dragged cells — the drag
    /// counterpart of the per-tile Cancel button; the director then voids any
    /// in-flight Cut job (`cleanup_jobs`).
    CancelCut,
    /// Mark every built structure under the dragged cells with `ToDemolish`
    /// (`construction::ToDemolish`) — the player-facing Demolish order.
    /// Dedups by entity via `Footprint::contains` so a multi-cell building
    /// dragged over several times is only marked once. Contrast the instant,
    /// no-refund `Tool::Demolish` devtool below.
    DesignateDemolish,
    /// Remove `ToDemolish` from every marked structure in the dragged cells —
    /// the drag counterpart of the per-tile Cancel button; the director then
    /// voids any in-flight Demolish job (`cleanup_jobs`).
    CancelDemolish,
    /// Devtool (armed from the debug window's Fire tab): paint ignition
    /// requests onto the dragged cells.
    Ignite,
    /// Devtool (armed from the debug window's Build tab): instantly destroy
    /// any built Wall/Door/Turbine on the dragged cells.
    Demolish,
}

impl Tool {
    pub fn label(&self) -> &'static str {
        match self {
            Tool::PlaceStockpile => "Place Stockpile",
            Tool::PlaceGrowing => "Place Growing Zone",
            Tool::Expand(_) => "Expand",
            Tool::Shrink(_) => "Shrink",
            Tool::PlaceWall => "Place Walls",
            Tool::PlaceDoor => "Place Doors",
            Tool::PlaceTurbine => "Place Turbines",
            Tool::PlaceSolarPanel => "Place Solar Panels",
            Tool::PlaceLightpost => "Place Lightposts",
            Tool::PlaceBattery => "Place Batteries",
            Tool::PlaceCooler => "Place Coolers",
            Tool::PlaceBed(BedOrientation::Horizontal) => "Place Bed (Horizontal, R to rotate)",
            Tool::PlaceBed(BedOrientation::Vertical) => "Place Bed (Vertical, R to rotate)",
            Tool::StampRoof => "Roof Area",
            Tool::StampNoRoof => "No-Roof Area",
            Tool::StampClearRoof => "Clear Roof Area",
            Tool::DesignateCut => "Cut Trees",
            Tool::CancelCut => "Cancel Cut",
            Tool::DesignateDemolish => "Demolish",
            Tool::CancelDemolish => "Cancel Demolish",
            Tool::Ignite => "Ignite (dev)",
            Tool::Demolish => "Demolish (dev)",
        }
    }

    /// Does this tool draw a single-axis line instead of filling a
    /// rectangle? Only the 1x1 wall-run structures — a wall/door/turbine/
    /// lightpost/battery/cooler drag reads as "build a run", not "fill an
    /// area". Multi-cell tiling tools (solar, bed), zones, roof stamps, and
    /// the order/dev tools all still want the full drawn rectangle, so they
    /// stay `false`. `drag_zone_tool` also lets Shift override this back to
    /// a rectangle for the tools that return `true` here.
    pub fn is_linear(&self) -> bool {
        match self {
            Tool::PlaceWall
            | Tool::PlaceDoor
            | Tool::PlaceTurbine
            | Tool::PlaceLightpost
            | Tool::PlaceBattery
            | Tool::PlaceCooler => true,
            Tool::PlaceStockpile
            | Tool::PlaceGrowing
            | Tool::Expand(_)
            | Tool::Shrink(_)
            | Tool::PlaceSolarPanel
            | Tool::PlaceBed(_)
            | Tool::StampRoof
            | Tool::StampNoRoof
            | Tool::StampClearRoof
            | Tool::DesignateCut
            | Tool::CancelCut
            | Tool::DesignateDemolish
            | Tool::CancelDemolish
            | Tool::Ignite
            | Tool::Demolish => false,
        }
    }
}

#[derive(Resource, Default)]
pub struct ActiveTool(pub Option<Tool>);

/// Run condition: normal click-select/click-move only fire while no zone
/// tool is armed, so drawing a rectangle never also selects or moves things.
pub fn tool_inactive(tool: Res<ActiveTool>) -> bool {
    tool.0.is_none()
}

/// Where the current drag started, set by `drag_zone_tool` on mouse-down and
/// cleared on release/cancel. A resource (not a `Local`) so
/// `construction::update_build_ghost` can compute the same drag-anchor set
/// `drag_zone_tool` does, keeping the 3D ghost, the ground highlight, and
/// the actual placement in agreement.
#[derive(Resource, Default)]
pub struct DragAnchor(pub Option<GridCoords>);

/// Data marker for a stockpile zone cell — painted on the dedicated Zones
/// IntGrid layer (`colony.ldtk`'s `Zones` layer, IntGrid value `STOCKPILE`),
/// separate from the Ground layer's natural terrain. A stockpile cell's
/// Ground terrain is genuine `Dirt`.
#[derive(Default, Component)]
pub struct StockpileZoneCell;

#[derive(Default, Bundle, LdtkIntCell)]
pub struct StockpileZoneCellBundle {
    cell: StockpileZoneCell,
}

/// Convert the LDtk-painted stockpile block into a normal, editable `Zone`
/// (single code path with player-created ones) instead of a static overlay.
/// No dirt backfill needed: the Zones-layer paint sits above genuine `Dirt`
/// Ground terrain (see `StockpileZoneCell`'s doc comment), so the ordinary
/// ground-tile bridge (`map::spawn_tile_visuals`) already covers these
/// cells like any other dirt cell.
pub fn seed_zones_from_ldtk(
    mut commands: Commands,
    new_cells: Query<&GridCoords, Added<StockpileZoneCell>>,
    mut seeded: Local<bool>,
) {
    if *seeded {
        return;
    }
    let cells: Vec<GridCoords> = new_cells.iter().copied().collect();
    if cells.is_empty() {
        return;
    }
    *seeded = true;
    commands.spawn((
        Zone {
            kind: ZoneKind::Stockpile,
        },
        ZoneRegion::from_cells(cells),
        Name::new("Stockpile"),
    ));
}

/// Rebuild a zone's overlay tiles whenever its region changes (creation
/// counts: `Added` implies `Changed`).
#[allow(clippy::type_complexity)]
pub fn rebuild_zone_visuals(
    mut commands: Commands,
    assets: Res<GameAssets>,
    changed: Query<(Entity, &Zone, &ZoneRegion), Changed<ZoneRegion>>,
    tiles: Query<(Entity, &ZoneTile)>,
) {
    for (zone_entity, zone, region) in &changed {
        for (tile_entity, tile) in &tiles {
            if tile.zone == zone_entity {
                commands.entity(tile_entity).despawn();
            }
        }
        let material = match zone.kind {
            ZoneKind::Stockpile => assets.stockpile_material.clone(),
            ZoneKind::Growing => assets.growing_zone_material.clone(),
            // Rooms have no player-facing tiles (they overlap real zones;
            // the temperature overlay is how you see them).
            ZoneKind::Room => continue,
        };
        for cell in region.cells() {
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(material.clone()),
                // A hair above the dirt tile underneath (every zone cell now
                // has one — see `seed_zones_from_ldtk`) to avoid z-fighting.
                Transform::from_translation(
                    map::grid_to_world(&cell) + Vec3::Y * (TILE_THICKNESS / 2.0 + 0.002),
                ),
                // It's a flat gameplay overlay, not physical geometry: it has
                // no business shadowing the dirt tile 2mm below it (that gap
                // is far smaller than the shadow map's bias, so it was
                // self-shadowing and flickering as the sun moved).
                NotShadowCaster,
                ZoneTile { zone: zone_entity },
            ));
        }
    }
}

/// `map::Stockpile.cells` (what the Director hauls to, and what every
/// stockpile-aware predicate elsewhere consults — grass rooting, wildfire
/// fuel, blueprint placement, the tooltip) is derived from the current
/// stockpile zones here, so create/delete/resize stay consistent with a
/// single source of truth. Terrain is untouched: a stockpile cell's Ground
/// terrain is genuine `Dirt`, unaffected by zone membership.
pub fn sync_stockpile_cells(
    zones: Query<(&Zone, &ZoneRegion)>,
    mut stockpile: ResMut<map::Stockpile>,
) {
    let mut current = HashSet::new();
    for (zone, region) in &zones {
        if zone.kind == ZoneKind::Stockpile {
            current.extend(region.cells());
        }
    }
    if current != stockpile.cells {
        stockpile.cells = current;
    }
}

/// One overlay tile following the cursor/drag rectangle while a tool is
/// armed; half-opacity so it reads as a preview, not a committed zone.
#[derive(Component)]
pub(crate) struct PreviewTile;

/// Cell is walkable dirt substrate — what a stockpile may sit on (grass is
/// cover on dirt, so grassy cells qualify too). Fertile land shares dirt's
/// nav cost, so this checks the real `Terrain` kind rather than inferring
/// it from cost like the walkability check does. Ground terrain no longer
/// changes when something is built on it, so this also needs an explicit
/// "nothing built here" check — see `construction::ConstructionMap`.
fn is_buildable(
    terrain: &TerrainMap,
    construction: &ConstructionMap,
    nav: &NavGrid,
    water: &map::WaterMap,
    cell: GridCoords,
) -> bool {
    nav.is_walkable(cell)
        && terrain.get(cell) == Some(Terrain::Dirt)
        && !water.has_bed(cell)
        && construction.get(cell).is_none()
}

/// Cell is walkable dirt or fertile substrate — what a growing zone may sit
/// on, minus riverbeds: the gate is the authored bed (`WaterMap::has_bed`),
/// not live wetness, so a drained bed can't be zoned only to flood the
/// crops when the level returns. Grass cover on top only helps (fertility
/// bonus). Same "nothing built here" gate as `is_buildable`.
fn is_growable(
    terrain: &TerrainMap,
    construction: &ConstructionMap,
    nav: &NavGrid,
    water: &map::WaterMap,
    cell: GridCoords,
) -> bool {
    nav.is_walkable(cell)
        && matches!(terrain.get(cell), Some(Terrain::Dirt | Terrain::Fertile))
        && !water.has_bed(cell)
        && construction.get(cell).is_none()
}

/// Drive the armed tool: show a live preview rectangle and commit it on
/// mouse-up, but stay armed so drags can repeat — right-click or Escape is
/// what ends the tool.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn drag_zone_tool(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut nav: ResMut<NavGrid>,
    // Bundled into one tuple param (system-param arity limit).
    ground: (Res<TerrainMap>, Res<map::WaterMap>),
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    ui_nodes: Query<&Interaction>,
    mut tool: ResMut<ActiveTool>,
    mut selected: ResMut<SelectedEntity>,
    mut drag_start: ResMut<DragAnchor>,
    mut zones: Query<(Entity, &Zone, &mut ZoneRegion)>,
    zone_tiles: Query<(Entity, &ZoneTile)>,
    previews: Query<Entity, With<PreviewTile>>,
    // Construction/roof tool state, bundled into one tuple param to stay
    // inside the system-param arity limit.
    construction_state: (
        ResMut<RoofMap>,
        // Erase-on-overlap needs the entity (to despawn), the Blueprint
        // (for its delivered wood), and the anchor GridCoords (refund drop
        // origin) alongside the footprint — not just the footprint.
        Query<(Entity, &Footprint, &Blueprint, &GridCoords)>,
        Query<&GridCoords, Or<(With<Tree>, With<BerryBush>, With<Crop>, With<Shrub>)>>,
        Query<&GridCoords, With<ItemStack>>,
        ResMut<crate::fire::FireMap>,
        // Built structure target lookup, by footprint (a solar panel's 4
        // cells all resolve to the one entity): shared by the instant
        // Demolish devtool and the DesignateDemolish/CancelDemolish order
        // tools below. `Has<ToDemolish>` lets the order arms tell an
        // already-marked structure apart without a second query.
        Query<
            (Entity, &Footprint, Has<construction::ToDemolish>),
            Or<(
                With<construction::Wall>,
                With<construction::Door>,
                With<construction::Turbine>,
                With<construction::SolarPanel>,
                With<construction::Lightpost>,
                With<construction::Battery>,
                With<construction::Bed>,
                With<construction::Cooler>,
            )>,
        >,
        ResMut<ConstructionMap>,
        // Erase-on-overlap refund: mirrors cancel_blueprints's `stacks` +
        // `occupied` params.
        Query<(&mut ItemStack, &GridCoords)>,
        Query<&GridCoords, With<Selectable>>,
        // DesignateCut/CancelCut target lookup: needs the `Tree` itself (for
        // `is_cuttable`), unlike the coords-only `plant_cells` above.
        Query<(Entity, &Tree, &GridCoords, Has<ToCut>)>,
    ),
) {
    let (terrain, water) = ground;
    let (
        mut roofs,
        blueprint_footprints,
        plant_cells,
        stack_cells,
        mut fire,
        buildings,
        mut construction,
        mut stacks,
        refund_targets,
        trees,
    ) = construction_state;
    let Some(active) = tool.0 else {
        drag_start.0 = None;
        for entity in &previews {
            commands.entity(entity).despawn();
        }
        return;
    };

    if keys.just_pressed(KeyCode::Escape) || buttons.just_pressed(MouseButton::Right) {
        tool.0 = None;
        drag_start.0 = None;
        for entity in &previews {
            commands.entity(entity).despawn();
        }
        return;
    }

    // Rotate a bed while it's being placed; re-read `active` so the rest of
    // this frame (preview computation below) reflects the flip immediately
    // instead of lagging a frame behind (the ghost system, `construction::
    // update_build_ghost`, reads `tool.0` fresh next frame regardless).
    if keys.just_pressed(KeyCode::KeyR) {
        if let Tool::PlaceBed(orientation) = active {
            tool.0 = Some(Tool::PlaceBed(orientation.toggle()));
        }
    }
    let active = tool.0.expect("just confirmed Some above");

    // Linear tools (see `Tool::is_linear`) draw a one-cell-thick line along
    // the drag's dominant axis instead of filling the drawn rectangle;
    // holding Shift falls back to the old rectangle fill for them.
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let linear = active.is_linear() && !shift;
    let drawn_cells = |a: GridCoords, b: GridCoords| {
        if linear {
            line_cells(a, b)
        } else {
            rect_cells(a, b)
        }
    };

    let over_ui = ui_nodes
        .iter()
        .any(|interaction| *interaction != Interaction::None);
    let (camera, camera_transform) = *camera;
    let hovered = (!over_ui)
        .then(|| cursor_to_cell(&window, camera, camera_transform))
        .flatten();

    if buttons.just_pressed(MouseButton::Left) && !over_ui {
        drag_start.0 = hovered;
    }

    // Rebuilt every frame; drag rectangles are small, so this stays cheap.
    for entity in &previews {
        commands.entity(entity).despawn();
    }

    let Some(hover_cell) = hovered else {
        return;
    };

    // Multi-cell tools (solar panel, bed) tile a snapped grid rather than
    // filling every drawn cell, so their preview shows the actual building
    // footprints a release would place, not the raw drag rectangle.
    let multi_cell_kind = match active {
        Tool::PlaceSolarPanel => Some(construction::BuildableKind::SolarPanel),
        Tool::PlaceBed(orientation) => Some(construction::BuildableKind::Bed(orientation)),
        _ => None,
    };
    let preview_cells = if let Some(kind) = multi_cell_kind {
        let step = construction::footprint_size(kind);
        snapped_footprint_anchors(drag_start.0.unwrap_or(hover_cell), hover_cell, step)
            .into_iter()
            .flat_map(|anchor| construction::footprint_cells(kind, anchor))
            .collect()
    } else {
        drawn_cells(drag_start.0.unwrap_or(hover_cell), hover_cell)
    };
    for cell in &preview_cells {
        commands.spawn((
            Mesh3d(assets.tile_mesh.clone()),
            MeshMaterial3d(assets.zone_preview_material.clone()),
            Transform::from_translation(
                map::grid_to_world(cell) + Vec3::Y * (TILE_THICKNESS / 2.0 + 0.005),
            ),
            NotShadowCaster,
            PreviewTile,
        ));
    }

    if buttons.just_released(MouseButton::Left) {
        if let Some(start) = drag_start.0 {
            let drawn = drawn_cells(start, hover_cell);
            // At most one zone per tile: cells another zone already owns are
            // skipped, same as walls/water — see is_buildable/is_growable
            // below. Auto-derived Room zones don't count: they exist to
            // overlap whatever the player puts inside them.
            let occupied: HashSet<GridCoords> = zones
                .iter()
                .filter(|(_, zone, _)| zone.kind != ZoneKind::Room)
                .flat_map(|(_, _, region)| region.cells())
                .collect();
            match active {
                Tool::PlaceStockpile => {
                    // Stockpiles sit on plain, unclaimed dirt; cells outside
                    // that are silently skipped rather than rejecting the
                    // whole drag.
                    let cells: Vec<GridCoords> = drawn
                        .into_iter()
                        .filter(|cell| {
                            is_buildable(&terrain, &construction, &nav, &water, *cell)
                                && !occupied.contains(cell)
                        })
                        .collect();
                    if !cells.is_empty() {
                        commands.spawn((
                            Zone {
                                kind: ZoneKind::Stockpile,
                            },
                            ZoneRegion::from_cells(cells),
                            Name::new("Stockpile"),
                        ));
                    }
                }
                Tool::PlaceGrowing => {
                    // Growing zones sit on unclaimed dirt or fertile land.
                    let cells: Vec<GridCoords> = drawn
                        .into_iter()
                        .filter(|cell| {
                            is_growable(&terrain, &construction, &nav, &water, *cell)
                                && !occupied.contains(cell)
                        })
                        .collect();
                    if !cells.is_empty() {
                        commands.spawn((
                            Zone {
                                kind: ZoneKind::Growing,
                            },
                            ZoneRegion::from_cells(cells),
                            GrowOrder { enabled: true },
                            ZonePlant::default(),
                            Name::new("Growing zone"),
                        ));
                    }
                }
                Tool::Expand(zone_entity) => {
                    if let Ok((_, zone, mut region)) = zones.get_mut(zone_entity) {
                        let eligible = |cell: GridCoords| match zone.kind {
                            ZoneKind::Stockpile => {
                                is_buildable(&terrain, &construction, &nav, &water, cell)
                            }
                            ZoneKind::Growing => {
                                is_growable(&terrain, &construction, &nav, &water, cell)
                            }
                            // Auto-managed; the player never expands one.
                            ZoneKind::Room => false,
                        };
                        let cells: Vec<GridCoords> = drawn
                            .into_iter()
                            .filter(|cell| eligible(*cell) && !occupied.contains(cell))
                            .collect();
                        if !cells.is_empty() {
                            region.extend(cells);
                        }
                    }
                }
                Tool::Shrink(zone_entity) => {
                    if let Ok((_, _, mut region)) = zones.get_mut(zone_entity) {
                        for cell in drawn {
                            region.remove(cell);
                        }
                        if region.is_empty() {
                            despawn_zone(&mut commands, zone_entity, &zone_tiles);
                            if selected.0 == Some(zone_entity) {
                                selected.0 = None;
                            }
                            // Nothing left to shrink — leaving the tool armed
                            // on a despawned entity would be a silent no-op.
                            tool.0 = None;
                        }
                    }
                }
                Tool::PlaceWall
                | Tool::PlaceDoor
                | Tool::PlaceTurbine
                | Tool::PlaceLightpost
                | Tool::PlaceBattery
                | Tool::PlaceCooler => {
                    let kind = match active {
                        Tool::PlaceDoor => construction::BuildableKind::Door,
                        Tool::PlaceTurbine => construction::BuildableKind::Turbine,
                        Tool::PlaceLightpost => construction::BuildableKind::Lightpost,
                        Tool::PlaceBattery => construction::BuildableKind::Battery,
                        Tool::PlaceCooler => construction::BuildableKind::Cooler,
                        _ => construction::BuildableKind::Wall,
                    };
                    // Blueprint spawns are deferred commands, so cells taken
                    // earlier in this same drag need a local set.
                    let mut placed: HashSet<GridCoords> = HashSet::new();
                    // Dedup: a footprint spanning several drawn cells (e.g.
                    // a corner of a 2x2 solar panel blueprint) must only be
                    // despawned/refunded once.
                    let mut erased: HashSet<Entity> = HashSet::new();
                    // Existing blueprints no longer block placement outright
                    // — an overlapping one gets erased and replaced below.
                    // Plants/stacks/terrain/nav/construction/already-placed-
                    // this-drag still block exactly as before.
                    let taken = |cell: GridCoords, placed: &HashSet<GridCoords>| {
                        placed.contains(&cell)
                            || plant_cells.iter().any(|grid| *grid == cell)
                            || stack_cells.iter().any(|grid| *grid == cell)
                    };
                    for cell in drawn {
                        if !construction::can_place_blueprint(
                            &terrain,
                            &construction,
                            &nav,
                            &water,
                            cell,
                            |cell| taken(cell, &placed),
                        ) {
                            continue;
                        }
                        if let Some((old_entity, _, old_blueprint, old_grid)) = blueprint_footprints
                            .iter()
                            .find(|(_, fp, _, _)| fp.contains(cell))
                        {
                            if erased.insert(old_entity) {
                                construction::despawn_blueprint_with_refund(
                                    &mut commands,
                                    &assets,
                                    &nav,
                                    &mut selected,
                                    old_entity,
                                    old_blueprint,
                                    *old_grid,
                                    &mut stacks,
                                    &refund_targets,
                                    "replaced by a new blueprint",
                                );
                            }
                        }
                        construction::spawn_blueprint(&mut commands, &assets, kind, cell);
                        placed.insert(cell);
                    }
                    evict_zones_for_building(
                        &mut commands,
                        &mut zones,
                        &zone_tiles,
                        &mut selected,
                        &placed,
                    );
                }
                Tool::PlaceSolarPanel | Tool::PlaceBed(_) => {
                    // Tile blocks starting from the drag's own start cell
                    // (see `snapped_footprint_anchors`) — unlike the
                    // single-tile buildings above, even a plain click
                    // always lands one full unit. Two drags with different
                    // starting parities can produce overlapping anchors;
                    // `can_place_footprint` below still blocks an anchor
                    // overlapping bad terrain, a built structure, a plant, or
                    // a stack (silently skipped), but an overlapping unbuilt
                    // blueprint is erased and replaced instead of blocking.
                    let kind = match active {
                        Tool::PlaceBed(orientation) => {
                            construction::BuildableKind::Bed(orientation)
                        }
                        _ => construction::BuildableKind::SolarPanel,
                    };
                    let step = construction::footprint_size(kind);
                    let mut placed: HashSet<GridCoords> = HashSet::new();
                    let mut erased: HashSet<Entity> = HashSet::new();
                    let taken = |cell: GridCoords, placed: &HashSet<GridCoords>| {
                        placed.contains(&cell)
                            || plant_cells.iter().any(|grid| *grid == cell)
                            || stack_cells.iter().any(|grid| *grid == cell)
                    };
                    for anchor in snapped_footprint_anchors(start, hover_cell, step) {
                        if !construction::can_place_footprint(
                            &terrain,
                            &construction,
                            &nav,
                            &water,
                            kind,
                            anchor,
                            |cell| taken(cell, &placed),
                        ) {
                            continue;
                        }
                        for cell in construction::footprint_cells(kind, anchor) {
                            if let Some((old_entity, _, old_blueprint, old_grid)) =
                                blueprint_footprints
                                    .iter()
                                    .find(|(_, fp, _, _)| fp.contains(cell))
                            {
                                if erased.insert(old_entity) {
                                    construction::despawn_blueprint_with_refund(
                                        &mut commands,
                                        &assets,
                                        &nav,
                                        &mut selected,
                                        old_entity,
                                        old_blueprint,
                                        *old_grid,
                                        &mut stacks,
                                        &refund_targets,
                                        "replaced by a new blueprint",
                                    );
                                }
                            }
                        }
                        construction::spawn_blueprint(&mut commands, &assets, kind, anchor);
                        placed.extend(construction::footprint_cells(kind, anchor));
                    }
                    evict_zones_for_building(
                        &mut commands,
                        &mut zones,
                        &zone_tiles,
                        &mut selected,
                        &placed,
                    );
                }
                Tool::StampRoof | Tool::StampNoRoof | Tool::StampClearRoof => {
                    let policy = match active {
                        Tool::StampRoof => RoofPolicy::Roof,
                        Tool::StampNoRoof => RoofPolicy::NoRoof,
                        _ => RoofPolicy::None,
                    };
                    for cell in drawn {
                        if construction::is_roofable(&terrain, &water, cell) {
                            roofs.set_policy(cell, policy);
                        }
                    }
                }
                Tool::DesignateCut => {
                    let drawn: HashSet<GridCoords> = drawn.into_iter().collect();
                    for (entity, tree, cell, to_cut) in &trees {
                        if drawn.contains(cell) && tree.is_cuttable() && !to_cut {
                            commands.entity(entity).insert(ToCut);
                        }
                    }
                }
                Tool::CancelCut => {
                    let drawn: HashSet<GridCoords> = drawn.into_iter().collect();
                    for (entity, _, cell, to_cut) in &trees {
                        if drawn.contains(cell) && to_cut {
                            commands.entity(entity).remove::<ToCut>();
                        }
                    }
                }
                Tool::DesignateDemolish => {
                    // Iterate structures (not drawn cells): each building
                    // appears once regardless of how many of its footprint
                    // cells the drag covers, so no dedup set is needed here
                    // (contrast the instant devtool below, which is driven
                    // off drawn cells and must dedup by entity).
                    let drawn: HashSet<GridCoords> = drawn.into_iter().collect();
                    for (entity, footprint, to_demolish) in &buildings {
                        if !to_demolish && footprint.cells.iter().any(|cell| drawn.contains(cell)) {
                            commands.entity(entity).insert(construction::ToDemolish);
                        }
                    }
                }
                Tool::CancelDemolish => {
                    let drawn: HashSet<GridCoords> = drawn.into_iter().collect();
                    for (entity, footprint, to_demolish) in &buildings {
                        if to_demolish && footprint.cells.iter().any(|cell| drawn.contains(cell)) {
                            commands.entity(entity).remove::<construction::ToDemolish>();
                        }
                    }
                }
                Tool::Ignite => {
                    // Paint ignition requests; `fire::tick_fire` validates
                    // the fuel/humidity gates when it drains the queue.
                    for cell in drawn {
                        fire.pending.push(cell);
                    }
                }
                Tool::Demolish => {
                    // A multi-cell building (a 2x2 solar panel) can have
                    // several of its footprint cells dragged over in the
                    // same stroke; dedup so it's despawned/demolished once.
                    let mut demolished: HashSet<Entity> = HashSet::new();
                    for cell in drawn {
                        // Only touch nav/roofs/construction (all `ResMut`,
                        // which flag changed on any deref-mut) when a
                        // building actually sits here — dragging over empty
                        // ground must not needlessly re-trigger
                        // `sync_wall_connections`'s `construction.is_changed()`.
                        let Some((entity, footprint, _)) =
                            buildings.iter().find(|(_, fp, _)| fp.contains(cell))
                        else {
                            continue;
                        };
                        if !demolished.insert(entity) {
                            continue;
                        }
                        commands.entity(entity).despawn();
                        // No ground tile to respawn: building never removed
                        // it (see `ConstructionMap`'s docs), so the original
                        // one has sat under this cell the whole time.
                        for fp_cell in &footprint.cells {
                            construction::demolish_at(
                                *fp_cell,
                                &mut construction,
                                &mut nav,
                                &mut roofs,
                            );
                        }
                        if selected.0 == Some(entity) {
                            selected.0 = None;
                        }
                    }
                }
            }
        }
        // The tool itself stays armed past a commit, so the player can keep
        // drawing more patches; right-click/Escape (above) is what ends it.
        drag_start.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    #[test]
    fn from_cells_and_contains() {
        let region = ZoneRegion::from_cells([at(0, 0), at(1, 0), at(2, 1)]);
        assert_eq!(region.len(), 3);
        assert!(region.contains(at(1, 0)));
        assert!(!region.contains(at(5, 5)));
    }

    #[test]
    fn extend_adds_cells() {
        let mut region = ZoneRegion::from_cells([at(0, 0)]);
        region.extend([at(1, 0), at(0, 0)]);
        assert_eq!(region.len(), 2);
        assert!(region.contains(at(0, 0)));
        assert!(region.contains(at(1, 0)));
    }

    #[test]
    fn remove_drops_exactly_the_covered_cell() {
        let mut region = ZoneRegion::from_cells([at(0, 0), at(1, 0), at(2, 0)]);
        region.remove(at(1, 0));
        assert_eq!(region.len(), 2);
        assert!(region.contains(at(0, 0)));
        assert!(!region.contains(at(1, 0)));
        assert!(region.contains(at(2, 0)));
    }

    #[test]
    fn remove_can_carve_a_hole_or_split_the_shape() {
        // A 1x3 strip with the middle cell removed becomes two disjoint
        // cells in one region — any shape is allowed, no contiguity check.
        let mut region = ZoneRegion::from_cells([at(0, 0), at(1, 0), at(2, 0)]);
        region.remove(at(1, 0));
        assert!(region.contains(at(0, 0)) && region.contains(at(2, 0)));
        assert!(!region.contains(at(1, 0)));
    }

    #[test]
    fn removing_every_cell_becomes_empty() {
        let mut region = ZoneRegion::from_cells([at(0, 0), at(1, 0)]);
        region.remove(at(0, 0));
        region.remove(at(1, 0));
        assert!(region.is_empty());
    }

    #[test]
    fn placing_a_building_evicts_only_its_own_cells() {
        // Mirrors `drag_zone_tool`'s PlaceWall/Door/Turbine eviction: a
        // building claims its cell outright, as if the zone had been
        // manually shrunk around the new footprint.
        let mut region = ZoneRegion::from_cells([at(0, 0), at(1, 0), at(2, 0)]);
        let placed: HashSet<GridCoords> = [at(1, 0)].into_iter().collect();
        for cell in &placed {
            region.remove(*cell);
        }
        assert_eq!(region.len(), 2);
        assert!(region.contains(at(0, 0)) && region.contains(at(2, 0)));
        assert!(!region.contains(at(1, 0)));
        assert!(!region.is_empty());

        // A zone fully covered by the new footprint reports empty — the
        // system despawns it in that case.
        let mut fully_covered = ZoneRegion::from_cells([at(5, 5), at(6, 5)]);
        let placed_all: HashSet<GridCoords> = [at(5, 5), at(6, 5)].into_iter().collect();
        for cell in &placed_all {
            fully_covered.remove(*cell);
        }
        assert!(fully_covered.is_empty());
    }

    #[test]
    fn rect_cells_normalizes_unordered_corners() {
        let cells = rect_cells(at(2, 1), at(0, 0));
        assert_eq!(cells.len(), 6);
        assert!(cells.contains(&at(0, 0)));
        assert!(cells.contains(&at(2, 1)));
    }

    #[test]
    fn rect_cells_clamps_to_map_bounds() {
        let cells = rect_cells(at(-3, -3), at(MAP_WIDTH + 5, MAP_HEIGHT + 5));
        assert_eq!(cells.len(), (MAP_WIDTH * MAP_HEIGHT) as usize);
        assert!(cells.contains(&at(0, 0)));
        assert!(cells.contains(&at(MAP_WIDTH - 1, MAP_HEIGHT - 1)));
    }

    #[test]
    fn line_cells_picks_the_dominant_axis() {
        // |dx| > |dy|: horizontal run, y held at the start cell.
        let cells = line_cells(at(0, 0), at(3, 1));
        assert_eq!(cells, vec![at(0, 0), at(1, 0), at(2, 0), at(3, 0)]);

        // |dy| > |dx|: vertical run, x held at the start cell.
        let cells = line_cells(at(0, 0), at(1, 3));
        assert_eq!(cells, vec![at(0, 0), at(0, 1), at(0, 2), at(0, 3)]);

        // A tie goes horizontal.
        let cells = line_cells(at(0, 0), at(2, 2));
        assert_eq!(cells, vec![at(0, 0), at(1, 0), at(2, 0)]);
    }

    #[test]
    fn line_cells_zero_length_drag_is_the_start_cell() {
        assert_eq!(line_cells(at(4, 4), at(4, 4)), vec![at(4, 4)]);
    }

    #[test]
    fn line_cells_normalizes_reversed_corners() {
        // Dragging from high-x back to low-x still produces an ascending
        // run, same normalization as `rect_cells`.
        let cells = line_cells(at(3, 0), at(0, 0));
        assert_eq!(cells, vec![at(0, 0), at(1, 0), at(2, 0), at(3, 0)]);
    }

    #[test]
    fn line_cells_clamps_to_map_bounds() {
        let cells = line_cells(at(-3, 0), at(MAP_WIDTH + 5, 0));
        assert_eq!(cells.len(), MAP_WIDTH as usize);
        assert!(cells.contains(&at(0, 0)));
        assert!(cells.contains(&at(MAP_WIDTH - 1, 0)));
    }

    #[test]
    fn is_linear_flags_only_the_1x1_wall_run_tools() {
        assert!(Tool::PlaceWall.is_linear());
        assert!(Tool::PlaceDoor.is_linear());
        assert!(Tool::PlaceTurbine.is_linear());
        assert!(Tool::PlaceLightpost.is_linear());
        assert!(Tool::PlaceBattery.is_linear());
        assert!(Tool::PlaceCooler.is_linear());
        assert!(!Tool::PlaceSolarPanel.is_linear());
        assert!(!Tool::PlaceStockpile.is_linear());
        assert!(!Tool::PlaceGrowing.is_linear());
        assert!(!Tool::DesignateDemolish.is_linear());
    }

    #[test]
    fn snapped_footprint_anchors_tiles_from_the_drags_own_start() {
        // A 4x2 drag starting on an even corner tiles two side-by-side
        // panels, stepping by 2 from that corner.
        let anchors = snapped_footprint_anchors(at(0, 0), at(3, 1), (2, 2));
        assert_eq!(anchors, vec![at(0, 0), at(2, 0)]);

        // Starting on an odd corner anchors panels at that odd coordinate
        // instead of being pulled down to an even grid — this is the exact
        // case the user reported as broken ("we can only place a panel at
        // 0,0 or 2,2 — we should also be able to place one at 1,1").
        let anchors = snapped_footprint_anchors(at(1, 1), at(4, 4), (2, 2));
        assert_eq!(anchors, vec![at(1, 1), at(1, 3), at(3, 1), at(3, 3)]);

        // A plain click always lands one panel anchored exactly where you
        // clicked, even/odd alike.
        assert_eq!(
            snapped_footprint_anchors(at(5, 5), at(5, 5), (2, 2)),
            vec![at(5, 5)]
        );
        assert_eq!(
            snapped_footprint_anchors(at(1, 1), at(1, 1), (2, 2)),
            vec![at(1, 1)]
        );

        // A 2-wide drag fits entirely in one panel now, rather than being
        // forced to straddle two blocks of a fixed grid.
        assert_eq!(
            snapped_footprint_anchors(at(5, 5), at(6, 5), (2, 2)),
            vec![at(5, 5)]
        );
    }

    #[test]
    fn snapped_footprint_anchors_steps_independently_per_axis() {
        // A non-square step (a 2x1 bed) tiles side-by-side along its long
        // axis twice as densely as along its short one.
        let anchors = snapped_footprint_anchors(at(0, 0), at(3, 1), (2, 1));
        assert_eq!(anchors, vec![at(0, 0), at(0, 1), at(2, 0), at(2, 1)]);
    }

    #[test]
    fn wheat_packs_every_cell() {
        assert!(ZonePlant::Wheat.accepts_cell(at(0, 0)));
        assert!(ZonePlant::Wheat.accepts_cell(at(7, 11)));
    }

    #[test]
    fn trees_keep_their_spacing_grid() {
        let s = TREE_PLANT_SPACING;
        assert!(ZonePlant::Trees.accepts_cell(at(0, 0)));
        assert!(ZonePlant::Trees.accepts_cell(at(s, s)));
        assert!(ZonePlant::Trees.accepts_cell(at(2 * s, s)));
        // Off-grid on either axis: rejected.
        assert!(!ZonePlant::Trees.accepts_cell(at(1, 0)));
        assert!(!ZonePlant::Trees.accepts_cell(at(0, s - 1)));
        assert!(!ZonePlant::Trees.accepts_cell(at(s + 1, s + 2)));
        // rem_euclid keeps the grid stable for negative coords (off-map,
        // but the predicate shouldn't misbehave on them).
        assert!(ZonePlant::Trees.accepts_cell(at(-s, -s)));
        assert!(!ZonePlant::Trees.accepts_cell(at(-1, 0)));
    }

    #[test]
    fn plant_cycle_alternates() {
        assert_eq!(ZonePlant::Wheat.next(), ZonePlant::Trees);
        assert_eq!(ZonePlant::Trees.next(), ZonePlant::Wheat);
        assert_eq!(ZonePlant::default(), ZonePlant::Wheat);
    }
}
