use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::game::GameAssets;
use crate::snow::{snow_bucket, SnowMap};
use crate::weather::{humidity_bucket, HumidityMap};

/// Data marker for a dirt cell, inserted by bevy_ecs_ldtk on every IntGrid
/// tile entity with the `DIRT` value.
#[derive(Default, Component)]
pub struct DirtCell;

#[derive(Default, Bundle, LdtkIntCell)]
pub struct DirtCellBundle {
    cell: DirtCell,
}

/// River cells: walkable, but slow to traverse (see TERRAIN_COST_*).
#[derive(Default, Component)]
pub struct ShallowWaterCell;

#[derive(Default, Bundle, LdtkIntCell)]
pub struct ShallowWaterCellBundle {
    cell: ShallowWaterCell,
}

#[derive(Default, Component)]
pub struct DeepWaterCell;

#[derive(Default, Bundle, LdtkIntCell)]
pub struct DeepWaterCellBundle {
    cell: DeepWaterCell,
}

/// Data marker for a fertile-land cell (IntGrid value `FERTILE`): same
/// traversal as dirt, but plants on it grow much faster.
#[derive(Default, Component)]
pub struct FertileCell;

#[derive(Default, Bundle, LdtkIntCell)]
pub struct FertileCellBundle {
    cell: FertileCell,
}

/// Data marker for a grass-seed cell (IntGrid value `GRASS`): the substrate
/// is plain dirt; the paint seeds a full-coverage grass `cover::Flora` on
/// the cell (see `cover::seed_grass_from_ldtk`).
#[derive(Default, Component)]
pub struct GrassCell;

#[derive(Default, Bundle, LdtkIntCell)]
pub struct GrassCellBundle {
    cell: GrassCell,
}

/// All stockpile zone cells, for haul-target lookups.
#[derive(Resource, Default)]
pub struct Stockpile {
    pub cells: HashSet<GridCoords>,
}

/// Devtool switch (Terrain debug tab): dirt tiles, grass cover, and shore
/// wedges all render as one flat, seamless shade by default. Flipping this
/// on brings back their usual `(x+y)%2` checkerboard — a debug view for
/// spotting the grid, not the shipped look. Off by default.
#[derive(Resource, Default)]
pub struct ShowSeams(pub bool);

/// What a map cell's natural ground is made of, for the hover tooltip (and,
/// eventually, richer terrain queries). Deliberately natural-ground-only:
/// what's built on a cell (walls, doors, turbines) never changes its
/// terrain value — see `construction::ConstructionMap`, a separate layer
/// on top. Stockpiles are a separate layer too now (see `map::Stockpile` /
/// the Zones IntGrid layer in `colony.ldtk`) — a stockpile cell's terrain
/// is genuinely `Dirt`. Water is a layer as well: a river cell's terrain is
/// `Dirt` shaped into an authored bed, and the water standing on it lives
/// in `WaterMap`. Every cell always has real ground geometry under it,
/// built/zoned or not.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Terrain {
    #[default]
    Dirt,
    Fertile,
}

impl Terrain {
    pub fn label(&self) -> &'static str {
        match self {
            Terrain::Dirt => "Dirt",
            Terrain::Fertile => "Fertile land",
        }
    }

    /// Traversal cost of the bare terrain, without any cover on top — what
    /// a cell's nav cost reverts to when a tree is felled (a standing tree
    /// overrides it with `TERRAIN_COST_TREE`, see `nav::mark_trees`).
    /// Water is not terrain: the live cost of a riverbed cell comes from
    /// `nav::water_cost(WaterMap::depth())` instead.
    pub fn base_cost(&self) -> u32 {
        match self {
            Terrain::Dirt | Terrain::Fertile => TERRAIN_COST_DIRT,
        }
    }

    /// Growth-rate multiplier for plants rooted on this tile (bushes and
    /// crops alike). Wet cells are excluded at the `fertility_at` call site
    /// in `cover.rs` (wetness lives in `WaterMap`, not here); built cells
    /// (walls/doors/turbines) and stockpile-zoned cells likewise — see
    /// `ConstructionMap` / `map::Stockpile`.
    pub fn fertility(&self) -> f32 {
        match self {
            Terrain::Dirt => DIRT_FERTILITY,
            Terrain::Fertile => FERTILE_FERTILITY,
        }
    }
}

/// Per-cell terrain lookup, filled by the LDtk bridge systems.
#[derive(Resource)]
pub struct TerrainMap {
    kinds: Vec<Terrain>,
}

impl Default for TerrainMap {
    fn default() -> Self {
        Self {
            kinds: vec![Terrain::default(); (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl TerrainMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<Terrain> {
        Self::in_bounds(cell).then(|| self.kinds[(cell.y * MAP_WIDTH + cell.x) as usize])
    }

    pub(crate) fn set(&mut self, cell: GridCoords, terrain: Terrain) {
        if Self::in_bounds(cell) {
            self.kinds[(cell.y * MAP_WIDTH + cell.x) as usize] = terrain;
        }
    }
}

/// The single authority on water. Two kinds of data live here:
///
/// - **Beds** — immutable authored geometry from the LDtk paint (via
///   `mark_water_map`): how far a cell's ground is recessed below the
///   ground top (y = 0). `0.0` = ordinary dry land.
/// - **Bodies** — every 4-way-contiguous group of bed cells becomes one
///   water body (`build_water_bodies`, once the LDtk cells have arrived),
///   carrying a **level**: the surface world-y, starting at
///   `WATER_SURFACE_Y`. The water-level devtool moves levels; rain/seasons
///   may drive them later.
///
/// The live water column on a cell is `depth() = (level + bed).max(0)`.
/// Water can never occupy a cell with no bed — there is no flow
/// simulation, so extent is bounded by the authored paint by construction.
/// Sim consumers (nav cost, wading, fertility, fuel, humidity, overlays,
/// tooltip) read the live `depth()`/`is_wet()`; *placement* checks
/// (blueprints, zones, roofs) gate on `has_bed()` instead, so nothing can
/// be built on a drained bed only to be flooded when the level rises.
#[derive(Resource)]
pub struct WaterMap {
    /// Authored bed recess per cell, world units. 0.0 = no bed (dry land).
    bed: Vec<f32>,
    /// Which body each cell belongs to (index into `levels`); None on
    /// bedless cells, and everywhere until `build_water_bodies` runs.
    body_of: Vec<Option<u16>>,
    /// Surface world-y per body.
    levels: Vec<f32>,
}

impl Default for WaterMap {
    fn default() -> Self {
        Self {
            bed: vec![0.0; (MAP_WIDTH * MAP_HEIGHT) as usize],
            body_of: vec![None; (MAP_WIDTH * MAP_HEIGHT) as usize],
            levels: Vec::new(),
        }
    }
}

impl WaterMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    fn index(cell: GridCoords) -> usize {
        (cell.y * MAP_WIDTH + cell.x) as usize
    }

    /// Authored bed recess. Out-of-bounds cells read as dry land.
    pub fn bed(&self, cell: GridCoords) -> f32 {
        if Self::in_bounds(cell) {
            self.bed[Self::index(cell)]
        } else {
            0.0
        }
    }

    /// The placement gate: a cell with an authored bed can carry water at
    /// some level, so it never accepts blueprints, zones or roofs — even
    /// while drained.
    pub fn has_bed(&self, cell: GridCoords) -> bool {
        self.bed(cell) > 0.0
    }

    /// Live water column on the cell (world units); 0.0 when dry, drained
    /// below the bed, or out of bounds.
    pub fn depth(&self, cell: GridCoords) -> f32 {
        if !Self::in_bounds(cell) {
            return 0.0;
        }
        let i = Self::index(cell);
        self.body_of[i].map_or(0.0, |body| {
            (self.levels[body as usize] + self.bed[i]).max(0.0)
        })
    }

    pub fn is_wet(&self, cell: GridCoords) -> bool {
        self.depth(cell) > 0.0
    }

    /// The cell's body surface world-y, regardless of whether any column is
    /// left above the bed; None on bedless cells.
    pub fn surface_level(&self, cell: GridCoords) -> Option<f32> {
        if !Self::in_bounds(cell) {
            return None;
        }
        self.body_of[Self::index(cell)].map(|body| self.levels[body as usize])
    }

    pub fn body_count(&self) -> usize {
        self.levels.len()
    }

    /// Per-body surface levels (world-y), for the devtool label.
    pub fn levels(&self) -> &[f32] {
        &self.levels
    }

    pub(crate) fn set_bed(&mut self, cell: GridCoords, bed: f32) {
        if Self::in_bounds(cell) {
            self.bed[Self::index(cell)] = bed;
        }
    }

    /// Group every 4-way-contiguous run of bed cells into one body at the
    /// starting level. Same flood-fill shape as
    /// `construction::enclosed_interior`, minus the border bail — a river
    /// may run off the map edge. Bodies never change afterwards (beds are
    /// immutable), so this runs once.
    pub(crate) fn rebuild_bodies(&mut self) {
        self.body_of.fill(None);
        self.levels.clear();
        for y in 0..MAP_HEIGHT {
            for x in 0..MAP_WIDTH {
                let start = GridCoords::new(x, y);
                let i = Self::index(start);
                if self.bed[i] <= 0.0 || self.body_of[i].is_some() {
                    continue;
                }
                let body = self.levels.len() as u16;
                self.levels.push(WATER_SURFACE_Y);
                let mut queue = std::collections::VecDeque::from([start]);
                self.body_of[i] = Some(body);
                while let Some(cell) = queue.pop_front() {
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let next = GridCoords::new(cell.x + dx, cell.y + dy);
                        if !Self::in_bounds(next) {
                            continue;
                        }
                        let j = Self::index(next);
                        if self.bed[j] > 0.0 && self.body_of[j].is_none() {
                            self.body_of[j] = Some(body);
                            queue.push_back(next);
                        }
                    }
                }
            }
        }
    }

    /// Move every body's level by `step` world units, clamped to the
    /// devtool range (`WATER_LEVEL_MIN`..=`WATER_LEVEL_MAX`).
    pub(crate) fn shift_all_levels(&mut self, step: f32) {
        for level in &mut self.levels {
            *level = (*level + step).clamp(WATER_LEVEL_MIN, WATER_LEVEL_MAX);
        }
    }
}

/// A live column deeper than the class threshold counts as deep water.
pub fn is_deep(depth: f32) -> bool {
    depth > WATER_DEEP_MIN_DEPTH
}

/// What the tooltip calls a cell: live water when a column stands on it
/// (ice once frozen solid), the natural terrain otherwise (a drained bed
/// reads as its ground).
pub fn surface_label(terrain: Terrain, depth: f32, frozen: bool) -> &'static str {
    if depth <= 0.0 {
        terrain.label()
    } else if frozen {
        "Ice"
    } else if is_deep(depth) {
        "Deep water"
    } else {
        "Shallow water"
    }
}

/// Record the LDtk water paint as per-cell authored beds. Runs chained
/// before `spawn_water_visuals` and `nav::mark_water`, which read the
/// same-frame depths (see the bridge tuple in `game.rs`).
pub fn mark_water_map(
    mut water: ResMut<WaterMap>,
    shallows: Query<&GridCoords, Added<ShallowWaterCell>>,
    deeps: Query<&GridCoords, Added<DeepWaterCell>>,
) {
    for grid in &shallows {
        water.set_bed(*grid, WATER_BED_SHALLOW);
    }
    for grid in &deeps {
        water.set_bed(*grid, WATER_BED_DEEP);
    }
}

/// One-shot: flood the authored beds into water bodies once the LDtk cells
/// have arrived (they land frames after startup — same guard pattern as
/// `cover::seed_algae`). Chained after `mark_water_map` so the beds are
/// complete when the query first turns non-empty.
#[allow(clippy::type_complexity)]
pub fn build_water_bodies(
    mut water: ResMut<WaterMap>,
    mut built: Local<bool>,
    cells: Query<(), Or<(With<ShallowWaterCell>, With<DeepWaterCell>)>>,
) {
    if *built || cells.is_empty() {
        return;
    }
    water.rebuild_bodies();
    *built = true;
    info!("water bodies: {}", water.body_count());
}

/// The player's standing roof intent for a cell, stamped by the Roof /
/// No-roof / Clear-roof drag orders (and by room completion, which
/// auto-stamps `Roof` on newly enclosed interiors). Stamping one policy
/// overwrites the other, so a cell can never be both.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoofPolicy {
    #[default]
    None,
    Roof,
    NoRoof,
}

/// Per-cell roof state: what IS roofed (`roofed`, the physical attribute
/// weather reads) and what SHOULD be (`policy`, the player's intent). The
/// gap between the two is what generates roof build/remove jobs — there are
/// no stored designation markers.
#[derive(Resource)]
pub struct RoofMap {
    roofed: Vec<bool>,
    policy: Vec<RoofPolicy>,
}

impl Default for RoofMap {
    fn default() -> Self {
        Self {
            roofed: vec![false; (MAP_WIDTH * MAP_HEIGHT) as usize],
            policy: vec![RoofPolicy::None; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl RoofMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    fn index(cell: GridCoords) -> usize {
        (cell.y * MAP_WIDTH + cell.x) as usize
    }

    /// Out-of-bounds cells read as unroofed open sky.
    pub fn is_roofed(&self, cell: GridCoords) -> bool {
        Self::in_bounds(cell) && self.roofed[Self::index(cell)]
    }

    pub fn set_roofed(&mut self, cell: GridCoords, roofed: bool) {
        if Self::in_bounds(cell) {
            self.roofed[Self::index(cell)] = roofed;
        }
    }

    pub fn policy(&self, cell: GridCoords) -> RoofPolicy {
        if Self::in_bounds(cell) {
            self.policy[Self::index(cell)]
        } else {
            RoofPolicy::None
        }
    }

    pub fn set_policy(&mut self, cell: GridCoords, policy: RoofPolicy) {
        if Self::in_bounds(cell) {
            self.policy[Self::index(cell)] = policy;
        }
    }
}

/// Record every non-dirt cell's terrain as the LDtk data entities arrive.
/// Stockpiles aren't terrain at all — they're painted on their own Zones
/// IntGrid layer (see `zones::StockpileZoneCell` /
/// `zones::seed_zones_from_ldtk`) and tracked in `map::Stockpile`, so they
/// never come through here; their Ground cell is plain `Dirt`. Walls
/// likewise aren't terrain — they're a placed `Entity` on the Entities
/// layer (see `construction::spawn_walls_from_ldtk`), not an IntGrid paint.
/// Water cells don't come through either: their terrain stays the default
/// `Dirt` (the bed), and the water itself is `WaterMap`'s job (see
/// `mark_water_map`).
pub fn mark_terrain(
    mut terrain: ResMut<TerrainMap>,
    fertiles: Query<&GridCoords, Added<FertileCell>>,
    grasses: Query<&GridCoords, Added<GrassCell>>,
) {
    for grid in &fertiles {
        terrain.set(*grid, Terrain::Fertile);
    }
    // Grass-seed cells are dirt substrate; the grass itself is cover.
    for grid in &grasses {
        terrain.set(*grid, Terrain::Dirt);
    }
}

/// Marker for the 3D tile entities we spawn ourselves.
#[derive(Component)]
pub struct TileVisual;

/// Marker on land ground tiles (dirt/fertile — never riverbeds): their mesh
/// is re-picked from the shore-corner mask by `sync_shore_corners` whenever
/// the WaterMap changes.
#[derive(Component)]
pub struct LandTile;

/// Marker on underwater riverbed tiles: their mesh is re-picked from
/// `deep_bed_tip_mask` by `sync_bed_corners` whenever the WaterMap changes,
/// so a shallow/deep bed boundary bevels the same way a shoreline does.
#[derive(Component)]
pub struct BedTile;

pub fn spawn_ldtk_world(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(LdtkWorldBundle {
        ldtk_handle: asset_server.load(LDTK_PROJECT_PATH).into(),
        ..default()
    });
}

/// LDtk `GridCoords` (y-up) -> XZ-plane world position, map centered on the
/// origin. The editor's top row ends up on -Z, away from the camera.
pub fn grid_to_world(grid: &GridCoords) -> Vec3 {
    Vec3::new(
        (grid.x as f32 - (MAP_WIDTH as f32 - 1.0) / 2.0) * TILE_SIZE,
        0.0,
        -(grid.y as f32 - (MAP_HEIGHT as f32 - 1.0) / 2.0) * TILE_SIZE,
    )
}

/// Inverse of `grid_to_world` for a point on the ground plane.
pub fn world_to_grid(pos: Vec3) -> GridCoords {
    GridCoords::new(
        (pos.x / TILE_SIZE + (MAP_WIDTH as f32 - 1.0) / 2.0).round() as i32,
        (-pos.z / TILE_SIZE + (MAP_HEIGHT as f32 - 1.0) / 2.0).round() as i32,
    )
}

/// One dirt tile at `grid` (top face at y = 0), flat or checkered per
/// `seams`. Shared by the LDtk bridge below; a stockpile cell is genuine
/// `Dirt` on the Ground layer (its zone is a separate paint on the Zones
/// layer — see `zones::seed_zones_from_ldtk`), so it already gets one of
/// these like any other dirt cell, no backfill needed.
pub fn spawn_dirt_tile(
    commands: &mut Commands,
    assets: &GameAssets,
    grid: &GridCoords,
    seams: bool,
) {
    // Optional checker (Terrain debug tab's Show-seams devtool) so the grid
    // can be made readable on the otherwise plain-color map.
    let material = if seams && (grid.x + grid.y) % 2 != 0 {
        assets.dirt_material_b.clone()
    } else {
        assets.dirt_material_a.clone()
    };

    commands.spawn((
        Mesh3d(assets.ground_mesh.clone()),
        MeshMaterial3d(material),
        Transform::from_translation(grid_to_world(grid) - Vec3::Y * GROUND_THICKNESS / 2.0),
        TileVisual,
        LandTile,
        *grid,
    ));
}

pub fn spawn_tile_visuals(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowSeams>,
    new_cells: Query<&GridCoords, Added<DirtCell>>,
) {
    for grid in &new_cells {
        spawn_dirt_tile(&mut commands, &assets, grid, show.0);
    }
}

/// Fertile tiles get their own flat color (no checker — the tint alone
/// reads as "special ground" against the dirt checkerboard).
pub fn spawn_fertile_visuals(
    mut commands: Commands,
    assets: Res<GameAssets>,
    new_cells: Query<&GridCoords, Added<FertileCell>>,
) {
    for grid in &new_cells {
        commands.spawn((
            Mesh3d(assets.ground_mesh.clone()),
            MeshMaterial3d(assets.fertile_material.clone()),
            Transform::from_translation(grid_to_world(grid) - Vec3::Y * GROUND_THICKNESS / 2.0),
            TileVisual,
            LandTile,
            *grid,
        ));
    }
}

/// Grass-seed cells get a plain dirt ground tile; the green comes from the
/// `cover::Flora` overlay seeded on top.
pub fn spawn_grass_visuals(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowSeams>,
    new_cells: Query<&GridCoords, Added<GrassCell>>,
) {
    for grid in &new_cells {
        spawn_dirt_tile(&mut commands, &assets, grid, show.0);
    }
}

/// Reconciles dirt `LandTile`s against `ShowSeams`: restores the `(x+y)%2`
/// checker while the devtool is on, flattens to `dirt_material_a` when it's
/// off (the shipped look). Runs on the toggle flipping *and* on freshly
/// spawned dirt tiles (e.g. a stockpile-delete backfill) — `spawn_dirt_tile`
/// already spawns them correct, so this is a cheap no-op pass for those, not
/// load-bearing. Fertile tiles are skipped — they're already one flat color.
pub fn sync_seamless_dirt(
    show: Res<ShowSeams>,
    terrain: Res<TerrainMap>,
    assets: Res<GameAssets>,
    added: Query<(), Added<LandTile>>,
    mut tiles: Query<(&GridCoords, &mut MeshMaterial3d<StandardMaterial>), With<LandTile>>,
) {
    if !show.is_changed() && added.is_empty() {
        return;
    }
    for (grid, mut material) in &mut tiles {
        if terrain.get(*grid) != Some(Terrain::Dirt) {
            continue;
        }
        let wanted = if !show.0 {
            assets.dirt_material_a.clone()
        } else if (grid.x + grid.y) % 2 == 0 {
            assets.dirt_material_a.clone()
        } else {
            assets.dirt_material_b.clone()
        };
        if material.0 != wanted {
            material.0 = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roof_policies_overwrite_each_other() {
        let mut roofs = RoofMap::default();
        let cell = GridCoords::new(4, 7);
        assert_eq!(roofs.policy(cell), RoofPolicy::None);
        roofs.set_policy(cell, RoofPolicy::Roof);
        assert_eq!(roofs.policy(cell), RoofPolicy::Roof);
        // A cell can never hold both intents: NoRoof replaces Roof outright.
        roofs.set_policy(cell, RoofPolicy::NoRoof);
        assert_eq!(roofs.policy(cell), RoofPolicy::NoRoof);
        roofs.set_policy(cell, RoofPolicy::None);
        assert_eq!(roofs.policy(cell), RoofPolicy::None);
    }

    #[test]
    fn roofed_state_is_independent_of_policy() {
        let mut roofs = RoofMap::default();
        let cell = GridCoords::new(1, 1);
        roofs.set_roofed(cell, true);
        assert!(roofs.is_roofed(cell));
        assert_eq!(roofs.policy(cell), RoofPolicy::None);
        roofs.set_policy(cell, RoofPolicy::NoRoof);
        assert!(roofs.is_roofed(cell));
    }

    #[test]
    fn out_of_bounds_reads_open_sky_and_writes_are_ignored() {
        let mut roofs = RoofMap::default();
        let outside = GridCoords::new(-1, MAP_HEIGHT);
        assert!(!roofs.is_roofed(outside));
        assert_eq!(roofs.policy(outside), RoofPolicy::None);
        roofs.set_roofed(outside, true);
        roofs.set_policy(outside, RoofPolicy::Roof);
        assert!(!roofs.is_roofed(outside));
        assert_eq!(roofs.policy(outside), RoofPolicy::None);
    }

    #[test]
    fn water_map_defaults_dry_and_beds_make_depth() {
        let mut water = WaterMap::default();
        let cell = GridCoords::new(5, 9);
        assert_eq!(water.depth(cell), 0.0);
        assert!(!water.is_wet(cell));
        assert!(!water.has_bed(cell));
        water.set_bed(cell, WATER_BED_DEEP);
        assert!(water.has_bed(cell));
        // A bed alone holds no water — the body brings the level.
        assert_eq!(water.depth(cell), 0.0);
        water.rebuild_bodies();
        assert_eq!(water.depth(cell), WATER_SURFACE_Y + WATER_BED_DEEP);
        assert!(water.is_wet(cell));
        assert_eq!(water.surface_level(cell), Some(WATER_SURFACE_Y));
    }

    #[test]
    fn water_map_out_of_bounds_reads_dry_and_writes_are_ignored() {
        let mut water = WaterMap::default();
        let outside = GridCoords::new(MAP_WIDTH, -1);
        water.set_bed(outside, WATER_BED_DEEP);
        water.rebuild_bodies();
        assert_eq!(water.depth(outside), 0.0);
        assert!(!water.is_wet(outside));
        assert_eq!(water.surface_level(outside), None);
        assert_eq!(water.body_count(), 0);
    }

    #[test]
    fn contiguous_beds_form_one_body_disjoint_beds_their_own() {
        let mut water = WaterMap::default();
        // A 2-cell run on the map border (bodies may touch the edge)...
        water.set_bed(GridCoords::new(0, 0), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(1, 0), WATER_BED_DEEP);
        // ...and a diagonal neighbor: touching corners is not contiguous
        // (4-way flood, same as nav).
        water.set_bed(GridCoords::new(2, 1), WATER_BED_SHALLOW);
        water.rebuild_bodies();
        assert_eq!(water.body_count(), 2);
        assert_eq!(
            water.surface_level(GridCoords::new(0, 0)),
            water.surface_level(GridCoords::new(1, 0))
        );
    }

    #[test]
    fn shifting_levels_clamps_and_drains() {
        let mut water = WaterMap::default();
        let shallow = GridCoords::new(3, 3);
        let deep = GridCoords::new(4, 3);
        water.set_bed(shallow, WATER_BED_SHALLOW);
        water.set_bed(deep, WATER_BED_DEEP);
        water.rebuild_bodies();
        // Drain far past the bottom clamp: depth bottoms out at exactly 0.
        water.shift_all_levels(-10.0);
        assert_eq!(water.levels(), &[WATER_LEVEL_MIN]);
        assert_eq!(water.depth(shallow), 0.0);
        assert_eq!(water.depth(deep), 0.0);
        assert!(!water.is_wet(deep));
        // Still a bed — placement stays gated while drained.
        assert!(water.has_bed(deep));
        // Refill past the top clamp: the shallow cell's column reappears.
        water.shift_all_levels(10.0);
        assert_eq!(water.levels(), &[WATER_LEVEL_MAX]);
        assert_eq!(water.depth(shallow), WATER_LEVEL_MAX + WATER_BED_SHALLOW);
    }

    #[test]
    fn deep_starts_past_the_class_threshold() {
        assert!(!is_deep(0.0));
        assert!(!is_deep(WATER_DEEP_MIN_DEPTH));
        // The default-level columns land on the classes the paint authored.
        assert!(!is_deep(WATER_SURFACE_Y + WATER_BED_SHALLOW));
        assert!(is_deep(WATER_SURFACE_Y + WATER_BED_DEEP));
    }

    #[test]
    fn surface_label_reads_the_live_water_first() {
        assert_eq!(surface_label(Terrain::Dirt, 0.30, false), "Deep water");
        assert_eq!(surface_label(Terrain::Dirt, 0.10, false), "Shallow water");
        // Frozen solid, any depth reads as ice...
        assert_eq!(surface_label(Terrain::Dirt, 0.30, true), "Ice");
        assert_eq!(surface_label(Terrain::Dirt, 0.10, true), "Ice");
        // ...but a drained bed is just its ground again, frozen or not.
        assert_eq!(surface_label(Terrain::Dirt, 0.0, true), "Dirt");
        assert_eq!(surface_label(Terrain::Dirt, 0.0, false), "Dirt");
        assert_eq!(surface_label(Terrain::Fertile, 0.0, false), "Fertile land");
    }

    #[test]
    fn water_flow_direction_is_a_unit_vector() {
        // The ripple shader treats it as a pure direction; a non-unit value
        // would silently scale the waves' speed and wavelength.
        assert!((WATER_FLOW_DIRECTION.length() - 1.0).abs() < 1e-6);
    }

    /// Every surface triangle must wind CCW seen from above (+Y normal).
    fn assert_faces_up(positions: &[[f32; 3]], indices: &[u32]) {
        for tri in indices.chunks(3) {
            let a = positions[tri[0] as usize];
            let b = positions[tri[1] as usize];
            let c = positions[tri[2] as usize];
            let ab = [b[0] - a[0], b[2] - a[2]];
            let ac = [c[0] - a[0], c[2] - a[2]];
            let up = ab[1] * ac[0] - ab[0] * ac[1];
            assert!(up > 0.0, "triangle {tri:?} winds clockwise");
        }
    }

    #[test]
    fn surface_mesh_shares_vertices_between_neighbors_of_one_body() {
        let mut water = WaterMap::default();
        water.set_bed(GridCoords::new(3, 3), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(4, 3), WATER_BED_DEEP);
        water.rebuild_bodies();
        let (positions, colors, indices) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        // Each end cell of the strip has its two outer corners clipped to
        // the midpoint diagonal (pentagon, 3 triangles); the two pentagons
        // share their full inner edge: 5 + 5 - 2 welded vertices.
        assert_eq!(positions.len(), 8);
        assert_eq!(colors.len(), 8);
        assert_eq!(indices.len(), 18);
        assert_faces_up(&positions, &indices);
    }

    #[test]
    fn surface_mesh_keeps_diagonal_bodies_apart() {
        let mut water = WaterMap::default();
        // Diagonal contact = two bodies (4-way flood); they share a corner
        // location but may sit at different levels, so no shared vertex —
        // and the saddle corner stays square (no clip), only the three
        // land-wrapped corners of each cell get clipped.
        water.set_bed(GridCoords::new(3, 3), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(4, 4), WATER_BED_DEEP);
        water.rebuild_bodies();
        assert_eq!(water.body_count(), 2);
        let (positions, _, indices) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        assert_eq!(positions.len(), 10);
        assert_eq!(indices.len(), 18);
    }

    #[test]
    fn surface_colors_blend_deep_interior_and_fade_at_shores() {
        let mut water = WaterMap::default();
        // A 3x3 deep pool: its center corner is pure open deep water...
        for y in 3..=5 {
            for x in 3..=5 {
                water.set_bed(GridCoords::new(x, y), WATER_BED_DEEP);
            }
        }
        water.rebuild_bodies();
        let (positions, colors, _) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        // Vertex lookup in DOUBLED grid coords (corners even, midpoints odd).
        let vertex = |dx: i32, dy: i32| {
            let x = (dx as f32 / 2.0 - MAP_WIDTH as f32 / 2.0) * TILE_SIZE;
            let z = -(dy as f32 / 2.0 - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE;
            positions
                .iter()
                .position(|p| p[0] == x && p[2] == z)
                .expect("vertex exists")
        };
        // ...full deep tint and unfaded deep alpha (shore factor 1.0)...
        let center = colors[vertex(8, 8)];
        let deep = DEEP_WATER_COLOR.to_linear().to_f32_array();
        for channel in 0..3 {
            assert!((center[channel] - deep[channel]).abs() < 1e-5);
        }
        assert!((center[3] - WATER_DEEP_ALPHA).abs() < 1e-5);
        // ...while a shoreline edge midpoint (on the SW cell's south edge —
        // that cell's outermost corner got clipped away entirely) fades
        // below the open-water alpha.
        assert!(colors[vertex(7, 6)][3] < WATER_DEEP_ALPHA);
    }

    #[test]
    fn shore_cut_mask_flags_only_fully_wrapped_corners() {
        let mut water = WaterMap::default();
        // Water wrapping land cell (5,5)'s NE corner — the ":." case.
        water.set_bed(GridCoords::new(6, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(6, 6), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_SHALLOW);
        let land = GridCoords::new(5, 5);
        assert_eq!(shore_cut_mask(land, &water), SHORE_NE);
        // A bed cell never cuts.
        assert_eq!(shore_cut_mask(GridCoords::new(6, 6), &water), 0);
        // Without the diagonal bed, two orthogonal beds don't wrap the
        // corner: straight shores stay straight.
        water.set_bed(GridCoords::new(6, 6), 0.0);
        assert_eq!(shore_cut_mask(land, &water), 0);
    }

    #[test]
    fn saddle_corners_stay_square() {
        let mut water = WaterMap::default();
        // Two beds touching only diagonally: ambiguous orientation and two
        // distinct bodies — neither the land cells cut nor the beds clip
        // at the shared corner.
        water.set_bed(GridCoords::new(3, 3), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(4, 4), WATER_BED_DEEP);
        assert_eq!(shore_cut_mask(GridCoords::new(4, 3), &water), 0);
        assert_eq!(shore_cut_mask(GridCoords::new(3, 4), &water), 0);
        assert_eq!(water_clip_mask(GridCoords::new(3, 3), &water) & SHORE_NE, 0);
        assert_eq!(water_clip_mask(GridCoords::new(4, 4), &water) & SHORE_SW, 0);
    }

    #[test]
    fn lone_pond_clips_all_four_corners() {
        let mut water = WaterMap::default();
        water.set_bed(GridCoords::new(3, 3), WATER_BED_SHALLOW);
        assert_eq!(
            water_clip_mask(GridCoords::new(3, 3), &water),
            SHORE_SW | SHORE_SE | SHORE_NE | SHORE_NW
        );
        // Land never clips.
        assert_eq!(water_clip_mask(GridCoords::new(4, 3), &water), 0);
    }

    #[test]
    fn chamfered_polygon_replaces_cut_corners_with_midpoints() {
        assert_eq!(chamfered_top_polygon(0).len(), 4);
        assert_eq!(chamfered_top_polygon(SHORE_SW).len(), 5);
        // Adjacent cuts share the midpoint of their common edge...
        assert_eq!(chamfered_top_polygon(SHORE_SW | SHORE_SE).len(), 5);
        // ...and all four cuts close into a diamond.
        assert_eq!(
            chamfered_top_polygon(SHORE_SW | SHORE_SE | SHORE_NE | SHORE_NW).len(),
            4
        );
    }

    #[test]
    fn shore_meshes_are_consistent_triangle_lists() {
        for mask in 0..16u8 {
            for mesh in [shore_dirt_mesh(mask), shore_overlay_mesh(mask)] {
                let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().len();
                let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().len();
                assert_eq!(positions, normals);
                let Some(Indices::U32(indices)) = mesh.indices() else {
                    panic!("expected u32 indices");
                };
                assert_eq!(indices.len() % 3, 0);
                assert!(indices.iter().all(|&i| (i as usize) < positions));
            }
        }
    }

    #[test]
    fn wedges_fill_tips_and_floor_concave_corners() {
        let mut water = WaterMap::default();
        // The same L of water around land cell (5,5)'s NE corner.
        for (x, y) in [(6, 5), (6, 6), (5, 6)] {
            water.set_bed(GridCoords::new(x, y), WATER_BED_SHALLOW);
        }
        let snow = SnowMap::default();
        let humidity = HumidityMap::default();
        let (positions, colors, normals, indices) =
            build_shore_wedge_geometry(&water, &snow, &humidity, false);
        // 5 convex tips (top triangle + chord quad = 7 verts / 9 indices)
        // and 1 concave corner (top triangle + 2 side skirts = 11 / 15).
        assert_eq!(positions.len(), 5 * 7 + 11);
        assert_eq!(colors.len(), positions.len());
        assert_eq!(normals.len(), positions.len());
        assert_eq!(indices.len(), 5 * 9 + 15);
        // Horizontal faces all wind CCW seen from above.
        for tri in indices.chunks(3) {
            let a = positions[tri[0] as usize];
            let b = positions[tri[1] as usize];
            let c = positions[tri[2] as usize];
            if a[1] == b[1] && b[1] == c[1] {
                let up = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                assert!(up > 0.0, "horizontal face {tri:?} winds clockwise");
            }
        }
        // Tip wedges continue the checkerboard (both dirt shades occur —
        // owners (6,5)/(5,6) are odd parity, (6,6) even), each floored
        // toward SHALLOW_WATER_COLOR by SHORE_TIP_WATER_BLEND so a
        // water-tagged tip never reads as pure dirt, and the concave wedge
        // carries the bed tint at the flanking beds' recess.
        let has = |color: Color| {
            let rgba = color.to_linear().to_f32_array();
            colors.iter().any(|c| *c == rgba)
        };
        let tip_a = DIRT_COLOR_A.mix(&SHALLOW_WATER_COLOR, SHORE_TIP_WATER_BLEND);
        let tip_b = DIRT_COLOR_B.mix(&SHALLOW_WATER_COLOR, SHORE_TIP_WATER_BLEND);
        assert!(has(tip_a));
        assert!(has(tip_b));
        assert!(has(WATER_BED_COLOR));
        let floor = -WATER_BED_SHALLOW;
        assert!(positions.iter().any(|p| p[1] == floor));
    }

    #[test]
    fn concave_wedge_whitens_toward_snow_color() {
        let mut water = WaterMap::default();
        // Same L of water around land cell (5,5)'s NE corner as above.
        for (x, y) in [(6, 5), (6, 6), (5, 6)] {
            water.set_bed(GridCoords::new(x, y), WATER_BED_SHALLOW);
        }
        let mut snow = SnowMap::default();
        snow.set(GridCoords::new(5, 5), 1.0);
        let humidity = HumidityMap::default();
        let (_, colors, _, _) = build_shore_wedge_geometry(&water, &snow, &humidity, false);
        let has = |color: Color| {
            let rgba = color.to_linear().to_f32_array();
            colors.iter().any(|c| *c == rgba)
        };
        // Fully snowed land cell (bucket 4, the deepest alpha step): the
        // concave wedge floor whitens toward snow instead of the bare bed
        // color, bucket-aligned with the flat overlay tile's own alpha.
        let snowy_bed = WATER_BED_COLOR.mix(&SNOW_OVERLAY_COLOR, SNOW_OVERLAY_ALPHAS[3]);
        assert!(has(snowy_bed));
        assert!(!has(WATER_BED_COLOR));
    }

    #[test]
    fn convex_tip_darkens_toward_wet_color_when_dry_of_snow() {
        let mut water = WaterMap::default();
        // A lone bed cell (6, 6) surrounded by land on all 4 sides makes 4
        // convex dirt tips, all owned by (6, 6), none concave.
        water.set_bed(GridCoords::new(6, 6), WATER_BED_SHALLOW);
        let snow = SnowMap::default();
        let mut humidity = HumidityMap::default();
        for (x, y) in [
            (5, 5),
            (6, 5),
            (7, 5),
            (5, 6),
            (7, 6),
            (5, 7),
            (6, 7),
            (7, 7),
        ] {
            humidity.set(GridCoords::new(x, y), 1.0);
        }
        let (_, colors, _, _) = build_shore_wedge_geometry(&water, &snow, &humidity, false);
        let has = |color: Color| {
            let rgba = color.to_linear().to_f32_array();
            colors.iter().any(|c| *c == rgba)
        };
        // Every land cell around the tips is soaked: all 4 tips (owner
        // (6,6), even parity -> DIRT_COLOR_A) carry the darkest wet tint
        // over the water-floored tip base, instead of the bare checker.
        let tip_base = DIRT_COLOR_A.mix(&SHALLOW_WATER_COLOR, SHORE_TIP_WATER_BLEND);
        let wet_tip = tip_base.mix(&WET_OVERLAY_COLOR, WET_OVERLAY_ALPHAS[3]);
        assert!(has(wet_tip));
        assert!(!has(DIRT_COLOR_A));
    }

    #[test]
    fn concave_wedge_darkens_toward_wet_color_when_dry_of_snow() {
        let mut water = WaterMap::default();
        // Same L of water around land cell (5,5)'s NE corner as the other
        // concave-corner tests.
        for (x, y) in [(6, 5), (6, 6), (5, 6)] {
            water.set_bed(GridCoords::new(x, y), WATER_BED_SHALLOW);
        }
        let snow = SnowMap::default();
        let mut humidity = HumidityMap::default();
        humidity.set(GridCoords::new(5, 5), 1.0);
        let (_, colors, _, _) = build_shore_wedge_geometry(&water, &snow, &humidity, false);
        let has = |color: Color| {
            let rgba = color.to_linear().to_f32_array();
            colors.iter().any(|c| *c == rgba)
        };
        // The concave floor is no longer a bare, uniformly bright bed color
        // on a soaked shoreline — it darkens toward the wet tint like the
        // convex tip and the flat overlay tile flush against it.
        let wet_bed = WATER_BED_COLOR.mix(&WET_OVERLAY_COLOR, WET_OVERLAY_ALPHAS[3]);
        assert!(has(wet_bed));
        assert!(!has(WATER_BED_COLOR));
    }

    #[test]
    fn concave_corner_triangle_duplicates_positions_without_a_crack() {
        let mut water = WaterMap::default();
        // An L of water around land cell (5,5)'s NE corner.
        for (x, y) in [(6, 5), (6, 6), (5, 6)] {
            water.set_bed(GridCoords::new(x, y), WATER_BED_SHALLOW);
        }
        water.rebuild_bodies();
        assert_eq!(water.body_count(), 1);
        let (positions, colors, indices) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        // Three clipped-corner pentagons (3 triangles each) plus the
        // concave triangle over the land cell's cut corner: 10 triangles on
        // 30 indices. The concave triangle pushes its own 3 vertices rather
        // than welding into the pentagons' shared ones — so it can be
        // independently retinted by nearby snow/wet/ice without disturbing
        // the pentagons' colors (see `build_water_surface_geometry`'s doc
        // comment) — 11 vertices from the pentagons plus 3 duplicates.
        assert_eq!(indices.len(), 30);
        assert_eq!(positions.len(), 14);
        assert_eq!(colors.len(), 14);
        // No crack: each of the concave triangle's 3 (duplicated) vertices
        // sits at the exact same position as one of the pentagons' own.
        let (pentagon_positions, concave_positions) = positions.split_at(11);
        for p in concave_positions {
            assert!(
                pentagon_positions.contains(p),
                "concave vertex {p:?} has no matching pentagon vertex"
            );
        }
        assert_faces_up(&positions, &indices);
    }

    #[test]
    fn concave_triangle_retints_toward_snow_without_disturbing_pentagon_colors() {
        let mut water = WaterMap::default();
        // Same L of water around land cell (5,5)'s NE corner as the
        // crack-free test above.
        for (x, y) in [(6, 5), (6, 6), (5, 6)] {
            water.set_bed(GridCoords::new(x, y), WATER_BED_SHALLOW);
        }
        water.rebuild_bodies();
        let humidity = HumidityMap::default();

        let dry_snow = SnowMap::default();
        let (positions, dry_colors, _) = build_water_surface_geometry(&water, &dry_snow, &humidity);
        let (pentagon_positions, concave_positions) = positions.split_at(11);
        let (pentagon_colors_dry, concave_colors_dry) = dry_colors.split_at(11);
        // Zero snow: `snow_wet_tint` is a no-op, so each (duplicated-
        // position) concave vertex matches the pentagon vertex at the same
        // position exactly — no visual change from before this triangle
        // gained its own retinting.
        for (p, c) in concave_positions.iter().zip(concave_colors_dry) {
            let i = pentagon_positions.iter().position(|q| q == p).unwrap();
            assert_eq!(*c, pentagon_colors_dry[i]);
        }

        let mut wet_snow = SnowMap::default();
        wet_snow.set(GridCoords::new(5, 5), 1.0);
        let (positions_wet, wet_colors, _) =
            build_water_surface_geometry(&water, &wet_snow, &humidity);
        let (pentagon_positions_wet, concave_positions_wet) = positions_wet.split_at(11);
        let (pentagon_colors_wet, concave_colors_wet) = wet_colors.split_at(11);
        // The pentagon vertices (ordinary cell-clip corners, untouched by
        // the retint) are completely unaffected by the land cell's snow —
        // only the concave triangle's own vertices shift toward it.
        assert_eq!(pentagon_positions_wet, pentagon_positions);
        assert_eq!(pentagon_colors_wet, pentagon_colors_dry);
        assert_eq!(concave_positions_wet, concave_positions);
        assert!(concave_colors_wet
            .iter()
            .zip(concave_colors_dry)
            .all(|(a, b)| a != b));
    }

    #[test]
    fn deep_boundary_tip_mask_flags_only_fully_wrapped_corners() {
        let mut water = WaterMap::default();
        // Shallow cell (5,5) with its NE corner wrapped by three deep
        // cells — the deep-tip case.
        water.set_bed(GridCoords::new(5, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(6, 5), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(6, 6), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_DEEP);
        water.rebuild_bodies();
        let shallow = GridCoords::new(5, 5);
        assert_eq!(deep_boundary_tip_mask(shallow, &water), SHORE_NE);
        // The wholly-deep corner cell isn't itself a tip (surrounded by
        // more deep, not by three shallow).
        assert_eq!(deep_boundary_tip_mask(GridCoords::new(6, 6), &water), 0);
        // Without the diagonal deep cell, two orthogonal deep neighbors
        // don't wrap the corner: the boundary stays straight, no tip.
        water.set_bed(GridCoords::new(6, 6), WATER_BED_SHALLOW);
        water.rebuild_bodies();
        assert_eq!(deep_boundary_tip_mask(shallow, &water), 0);
    }

    #[test]
    fn deep_boundary_tip_mask_flags_the_symmetric_shallow_tip() {
        let mut water = WaterMap::default();
        // Deep cell (5,5) with its NE corner wrapped by three shallow
        // cells — the shallow-tip case, mirroring the deep-tip test above.
        water.set_bed(GridCoords::new(5, 5), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(6, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(6, 6), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_SHALLOW);
        water.rebuild_bodies();
        assert_eq!(
            deep_boundary_tip_mask(GridCoords::new(5, 5), &water),
            SHORE_NE
        );
    }

    #[test]
    fn deep_boundary_saddle_stays_square() {
        let mut water = WaterMap::default();
        // Two deep cells touching only diagonally inside a shallow 2x2
        // block: the shared corner is ambiguous (the diagonal deep
        // neighbor blocks each cell's own tip predicate), so neither side
        // is flagged — mirrors `saddle_corners_stay_square`.
        water.set_bed(GridCoords::new(3, 3), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(4, 3), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(3, 4), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(4, 4), WATER_BED_DEEP);
        water.rebuild_bodies();
        assert_eq!(
            deep_boundary_tip_mask(GridCoords::new(3, 3), &water) & SHORE_NE,
            0
        );
        assert_eq!(
            deep_boundary_tip_mask(GridCoords::new(4, 4), &water) & SHORE_SW,
            0
        );
    }

    #[test]
    fn deep_bed_tip_mask_ignores_live_depth() {
        let mut water = WaterMap::default();
        // Same L-shape as the deep-tip test above, but checking the floor's
        // mask instead of the surface's.
        water.set_bed(GridCoords::new(5, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(6, 5), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(6, 6), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_DEEP);
        let shallow = GridCoords::new(5, 5);
        assert_eq!(deep_bed_tip_mask(shallow, &water), SHORE_NE);
        // Unlike `deep_boundary_tip_mask`, this reads the immutable bed, not
        // the live column: draining the body all the way (which flips
        // `deep_wet` for every cell in the surface's mask) leaves the
        // floor's mask untouched — the physical riverbed never moves.
        water.rebuild_bodies();
        water.shift_all_levels(-10.0);
        assert_eq!(deep_bed_tip_mask(shallow, &water), SHORE_NE);
    }

    #[test]
    fn bed_wedge_fills_the_cut_corner_at_the_majority_depth() {
        let mut water = WaterMap::default();
        water.set_bed(GridCoords::new(5, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(6, 5), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(6, 6), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_DEEP);
        let (positions, normals, indices) = build_bed_wedge_geometry(&water);
        // One top face (3 verts / 1 tri) + three skirts (4 verts / 2 tris
        // each, unshared per `push_face`).
        assert_eq!(positions.len(), 3 + 3 * 4);
        assert_eq!(normals.len(), positions.len());
        assert_eq!(indices.len(), 3 + 3 * 6);
        // The top face floors at the majority (deep) depth, continuing the
        // surrounding deep floor smoothly through the cut shallow corner.
        let floor = -WATER_BED_DEEP;
        assert!(positions.iter().any(|p| p[1] == floor));
        // Horizontal faces wind CCW seen from above.
        for tri in indices.chunks(3) {
            let a = positions[tri[0] as usize];
            let b = positions[tri[1] as usize];
            let c = positions[tri[2] as usize];
            if a[1] == b[1] && b[1] == c[1] {
                let up = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                assert!(up > 0.0, "horizontal face {tri:?} winds clockwise");
            }
        }
    }

    #[test]
    fn surface_boundary_band_follows_diagonal() {
        let mut water = WaterMap::default();
        // A shallow cell with its NE corner wrapped by three deep cells —
        // same L-shape as `concave_corner_triangle_duplicates_positions_without_a_crack`,
        // but the "land" role is now played by deep water instead of dry
        // land.
        water.set_bed(GridCoords::new(5, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(6, 5), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(6, 6), WATER_BED_DEEP);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_DEEP);
        water.rebuild_bodies();
        assert_eq!(water.body_count(), 1);
        let (positions, colors, indices) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        assert_faces_up(&positions, &indices);
        assert_eq!(indices.len() % 3, 0);

        let vertex = |dx: i32, dy: i32| {
            let x = (dx as f32 / 2.0 - MAP_WIDTH as f32 / 2.0) * TILE_SIZE;
            let z = -(dy as f32 / 2.0 - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE;
            positions
                .iter()
                .position(|p| p[0] == x && p[2] == z)
                .expect("vertex exists")
        };
        // The shared tip corner (S/E/N/NE's common grid corner) is a single
        // welded vertex, not duplicated per triangle that uses it.
        let tip_corner = (2 * 5 + 2, 2 * 5 + 2);
        let occurrences = positions
            .iter()
            .filter(|p| {
                let x = (tip_corner.0 as f32 / 2.0 - MAP_WIDTH as f32 / 2.0) * TILE_SIZE;
                let z = -(tip_corner.1 as f32 / 2.0 - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE;
                p[0] == x && p[2] == z
            })
            .count();
        assert_eq!(occurrences, 1);

        // The shallow/deep edge midpoints (east and north of the shallow
        // cell) carry a tint strictly between the pure shallow and pure
        // deep endpoints — a soft, intermediate color, not a hard step —
        // and it's the same value on both mixed edges by symmetry.
        let east_mid = colors[vertex(2 * 5 + 2, 2 * 5 + 1)];
        let north_mid = colors[vertex(2 * 5 + 1, 2 * 5 + 2)];
        assert_eq!(east_mid, north_mid);
        let shallow_only = water_vertex_color(WATER_SURFACE_Y + WATER_BED_SHALLOW, 1.0);
        let deep_only = water_vertex_color(WATER_SURFACE_Y + WATER_BED_DEEP, 1.0);
        for channel in 0..4 {
            assert!(
                east_mid[channel] > shallow_only[channel].min(deep_only[channel]) - 1e-5
                    && east_mid[channel] < shallow_only[channel].max(deep_only[channel]) + 1e-5,
            );
        }
        assert_ne!(east_mid, shallow_only);
        assert_ne!(east_mid, deep_only);
    }

    #[test]
    fn surface_boundary_has_no_tjunctions() {
        let mut water = WaterMap::default();
        // A shallow cell with a single deep neighbor to the north: neither
        // cell has a fully-wrapped tip corner here (both of the shallow
        // cell's north corners have land on their other side), so the
        // shared edge midpoint relies purely on the per-edge mixed-edge
        // insertion (`edge_is_mixed`) rather than a tip cut. If either side
        // inserted its own, undeduped copy this would crack.
        water.set_bed(GridCoords::new(5, 5), WATER_BED_SHALLOW);
        water.set_bed(GridCoords::new(5, 6), WATER_BED_DEEP);
        water.rebuild_bodies();
        assert_eq!(water.body_count(), 1);
        assert_eq!(deep_boundary_tip_mask(GridCoords::new(5, 5), &water), 0);
        assert_eq!(deep_boundary_tip_mask(GridCoords::new(5, 6), &water), 0);

        let (positions, _, _) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        let x = (11.0 / 2.0 - MAP_WIDTH as f32 / 2.0) * TILE_SIZE;
        let z = -(12.0 / 2.0 - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE;
        let occurrences = positions.iter().filter(|p| p[0] == x && p[2] == z).count();
        assert_eq!(
            occurrences, 1,
            "the shared mixed-edge midpoint must be a single welded vertex, not one per flanking cell"
        );
    }

    #[test]
    fn drained_water_builds_no_surface() {
        let mut water = WaterMap::default();
        water.set_bed(GridCoords::new(3, 3), WATER_BED_DEEP);
        water.rebuild_bodies();
        water.shift_all_levels(-10.0);
        let (positions, colors, indices) =
            build_water_surface_geometry(&water, &SnowMap::default(), &HumidityMap::default());
        assert!(positions.is_empty());
        assert!(colors.is_empty());
        assert!(indices.is_empty());
    }
}

/// Ripple extension over `StandardMaterial` (the repo's second shader,
/// after wheat's `weather::WindSwayExtension`): `water_surface.wgsl`
/// perturbs the fragment normal with wave noise scrolling along
/// `WATER_FLOW_DIRECTION`, so lighting shimmers as if the flat surface
/// quads carried small waves. The pattern is computed from world position,
/// so it runs seamlessly across the per-cell quads. Geometry, translucency
/// and PBR lighting are inherited from the base material untouched.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct WaterExtension {
    /// xy = flow direction (world XZ), z = sim time in seconds (frozen on
    /// pause — that's why time comes in through the uniform rather than
    /// the shader's own clock), w = live wind strength (scales the chop).
    #[uniform(100)]
    pub params: Vec4,
    /// x = freeze level 0..1 (`snow::FreezeLevel`): flattens the waves and
    /// whitens/opaques the surface toward ice. yzw spare.
    #[uniform(101)]
    pub ice: Vec4,
}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/water_surface.wgsl".into()
    }
}

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

/// Marks the single merged water-surface entity whose mesh
/// `rebuild_water_surface` regenerates whenever the water changes.
#[derive(Component)]
pub struct WaterSurfaceMesh;

/// Water cells render as an opaque riverbed cuboid per cell, recessed by
/// the authored bed (the neighboring ground tiles' side faces are the
/// banks), plus ONE merged translucent surface mesh for all wet cells —
/// see `rebuild_water_surface`. The beds never move.
pub fn spawn_water_visuals(
    mut commands: Commands,
    assets: Res<GameAssets>,
    water: Res<WaterMap>,
    shallows: Query<&GridCoords, Added<ShallowWaterCell>>,
    deeps: Query<&GridCoords, Added<DeepWaterCell>>,
) {
    for grid in shallows.iter().chain(deeps.iter()) {
        commands.spawn((
            Mesh3d(assets.ground_mesh.clone()),
            MeshMaterial3d(assets.water_bed_material.clone()),
            Transform::from_translation(
                grid_to_world(grid) - Vec3::Y * (water.bed(*grid) + GROUND_THICKNESS / 2.0),
            ),
            TileVisual,
            BedTile,
            *grid,
        ));
    }
}

/// Per-corner water samples for the surface mesh, averaged over the up-to-4
/// cells meeting at corner `(cx, cy)` (the corner between cells `cx-1..=cx`
/// x `cy-1..=cy`). Returns `(depth, shore)`:
/// - `depth` averages only over cells WITH beds (dry beds count as 0, land
///   is excluded) — so a 1-wide channel keeps its full tint instead of
///   being washed out by its banks;
/// - `shore` = fraction of the 4 cells that have beds — 1.0 in open water,
///   dropping toward the banks, which drives the rim's alpha fade.
fn corner_water(water: &WaterMap, cx: i32, cy: i32) -> (f32, f32) {
    let mut depth_sum = 0.0;
    let mut beds = 0u32;
    for (dx, dy) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
        let cell = GridCoords::new(cx + dx, cy + dy);
        if water.has_bed(cell) {
            beds += 1;
            depth_sum += water.depth(cell);
        }
    }
    if beds == 0 {
        return (0.0, 0.0);
    }
    (depth_sum / beds as f32, beds as f32 / 4.0)
}

/// Water sample for an edge-midpoint vertex (the shore-bevel diagonals end
/// on edge midpoints): like `corner_water` but over the edge's two flanking
/// cells — `shore` = beds/2, so a shoreline midpoint fades exactly like a
/// straight-shore corner does.
fn edge_water(water: &WaterMap, a: GridCoords, b: GridCoords) -> (f32, f32) {
    let mut depth_sum = 0.0;
    let mut beds = 0u32;
    for cell in [a, b] {
        if water.has_bed(cell) {
            beds += 1;
            depth_sum += water.depth(cell);
        }
    }
    if beds == 0 {
        return (0.0, 0.0);
    }
    (depth_sum / beds as f32, beds as f32 / 2.0)
}

/// Depth-tinted base color and alpha before any shore/snow modulation —
/// split out of `water_vertex_color` so the concave-corner loop in
/// `build_water_surface_geometry` can retint the RGB (toward snow/wet/ice)
/// while keeping the exact same depth/shore-fade alpha math.
fn water_depth_color(depth: f32, shore: f32) -> (Color, f32) {
    let t = (depth / WATER_TINT_DEEP_DEPTH).clamp(0.0, 1.0);
    let alpha = WATER_SHALLOW_ALPHA + (WATER_DEEP_ALPHA - WATER_SHALLOW_ALPHA) * t;
    let fade = WATER_SHORE_FADE + (1.0 - WATER_SHORE_FADE) * shore;
    (SHALLOW_WATER_COLOR.mix(&DEEP_WATER_COLOR, t), alpha * fade)
}

/// Linear-RGBA vertex color for a surface corner: tint and opacity blend
/// from the shallow to the deep endpoints as the column approaches
/// WATER_TINT_DEEP_DEPTH, then the alpha fades by shore exposure
/// (WATER_SHORE_FADE) so the water thins out visually at its edges.
fn water_vertex_color(depth: f32, shore: f32) -> [f32; 4] {
    let (color, alpha) = water_depth_color(depth, shore);
    let mut rgba = color.to_linear().to_f32_array();
    rgba[3] = alpha;
    rgba
}

/// Depth/shore-exposure sample for a water-surface vertex at DOUBLED grid
/// coordinates `(dx, dy)` (cell corners have both components even, edge
/// midpoints one odd). Split out of `surface_vertex` so the concave-corner
/// loop can sample identically while pushing its own (non-welded) vertices.
fn surface_vertex_depth_shore(water: &WaterMap, (dx, dy): (i32, i32)) -> (f32, f32) {
    if dx % 2 == 0 && dy % 2 == 0 {
        corner_water(water, dx / 2, dy / 2)
    } else if dy % 2 == 0 {
        // Midpoint of an east-west cell edge: flanked by the cells south
        // and north of it.
        let (cx, cy) = (dx / 2, dy / 2);
        edge_water(water, GridCoords::new(cx, cy - 1), GridCoords::new(cx, cy))
    } else {
        // Midpoint of a north-south cell edge: flanked west and east.
        let (cx, cy) = (dx / 2, dy / 2);
        edge_water(water, GridCoords::new(cx - 1, cy), GridCoords::new(cx, cy))
    }
}

/// World-space position for a water-surface vertex at DOUBLED grid
/// coordinates and a given surface `level` (world y).
fn surface_vertex_position(level: f32, (dx, dy): (i32, i32)) -> [f32; 3] {
    [
        (dx as f32 / 2.0 - MAP_WIDTH as f32 / 2.0) * TILE_SIZE,
        level,
        -(dy as f32 / 2.0 - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE,
    ]
}

/// One water-surface vertex at DOUBLED grid coordinates `(dx, dy)`, deduped
/// per body like the old per-corner map.
fn surface_vertex(
    water: &WaterMap,
    vertex_ids: &mut HashMap<(u16, i32, i32), u32>,
    positions: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    body: u16,
    level: f32,
    (dx, dy): (i32, i32),
) -> u32 {
    *vertex_ids.entry((body, dx, dy)).or_insert_with(|| {
        let (depth, shore) = surface_vertex_depth_shore(water, (dx, dy));
        positions.push(surface_vertex_position(level, (dx, dy)));
        colors.push(water_vertex_color(depth, shore));
        (positions.len() - 1) as u32
    })
}

/// The merged water surface: one convex polygon per wet cell — the square
/// with its `water_clip_mask` corners clipped to the edge-midpoint diagonal
/// — plus one triangle per concave (3-beds-around-a-corner) land corner,
/// covering that corner's cut shelf (`shore_cut_mask` /
/// `sync_shore_corners`), plus one triangle per `deep_boundary_tip_mask`
/// corner where a cell's shallow/deep class differs from all three other
/// cells meeting there. Vertices are shared between neighbors OF THE SAME
/// BODY (diagonally-touching beds are distinct bodies that may sit at
/// different levels, so the dedup key includes the body id), so clips,
/// concave triangles and deep-boundary tips weld into one 45-degree line —
/// shoreline for the land/water clips, and (since a mixed edge's midpoint
/// already samples an in-between depth via `edge_water`) a diagonal soft
/// band for the deep-boundary tips. Vertex colors carry the depth tint +
/// shore fade; the GPU interpolating them across triangles is what smooths
/// the color gradient across the diagonal. The concave land-corner triangle
/// additionally retints toward `snow`/`humidity` (see the loop below) — it's
/// the one water-surface patch that sits directly over a shore wedge's own
/// snow/wet tint (`build_shore_wedge_geometry`), and being the top-most
/// opaque-vs-translucent layer there it would otherwise hide that tint under
/// a plain untinted triangle. Returns `(positions, colors, indices)` —
/// world-space positions, the mesh entity's transform stays identity.
pub fn build_water_surface_geometry(
    water: &WaterMap,
    snow: &SnowMap,
    humidity: &HumidityMap,
) -> (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut vertex_ids: HashMap<(u16, i32, i32), u32> = HashMap::new();
    // Neighbor cell offset for the edge running from corner i to corner
    // i+1, in the same SW,SE,NE,NW walk order as `corners` below (south,
    // east, north, west).
    const EDGE_NEIGHBORS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            if !water.is_wet(cell) {
                continue;
            }
            let body = water.body_of[WaterMap::index(cell)].expect("wet cell has a body");
            let level = water.levels[body as usize];
            let clip = water_clip_mask(cell, water);
            let tips = deep_boundary_tip_mask(cell, water);
            let cut = clip | tips;
            // Corners counter-clockwise seen from above (+Y normal) in
            // doubled coords: grid +y is world -z, walk order matching
            // SHORE_CORNER_XZ. A cut corner (shore clip or deep-boundary
            // tip) is replaced by its two edge midpoints (the dedup handles
            // adjacent cut corners sharing one, and a lone pond closing
            // into a diamond); a tip additionally keeps its own triangle
            // (unlike a shore clip, which discards the corner — a
            // deep-boundary tip is still water) so the chord between the
            // midpoints becomes a real mesh edge. A kept corner picks up
            // the midpoint of its next edge too, when that edge is a
            // shallow/deep mix and the next corner is kept (a cut next
            // corner already emits that same midpoint via its own chamfer)
            // — this is what welds a boundary cell that isn't itself a tip
            // to its neighbor along the mixed edge instead of cracking.
            let corners = [
                (SHORE_SW, (2 * x, 2 * y)),
                (SHORE_SE, (2 * x + 2, 2 * y)),
                (SHORE_NE, (2 * x + 2, 2 * y + 2)),
                (SHORE_NW, (2 * x, 2 * y + 2)),
            ];
            let mut outline: Vec<(i32, i32)> = Vec::with_capacity(8);
            let mut tip_tris: Vec<[(i32, i32); 3]> = Vec::new();
            for (i, (bit, corner)) in corners.iter().enumerate() {
                let prev = corners[(i + 3) % 4].1;
                let next = corners[(i + 1) % 4].1;
                let mid_prev = ((corner.0 + prev.0) / 2, (corner.1 + prev.1) / 2);
                let mid_next = ((corner.0 + next.0) / 2, (corner.1 + next.1) / 2);
                if cut & bit != 0 {
                    outline.push(mid_prev);
                    outline.push(mid_next);
                    if tips & bit != 0 {
                        tip_tris.push([mid_next, mid_prev, *corner]);
                    }
                } else {
                    outline.push(*corner);
                    let next_is_cut = cut & corners[(i + 1) % 4].0 != 0;
                    if !next_is_cut {
                        let (ndx, ndy) = EDGE_NEIGHBORS[i];
                        let neighbor = GridCoords::new(x + ndx, y + ndy);
                        if edge_is_mixed(water, cell, neighbor) {
                            outline.push(mid_next);
                        }
                    }
                }
            }
            outline.dedup();
            if outline.len() > 1 && outline.first() == outline.last() {
                outline.pop();
            }
            let ids: Vec<u32> = outline
                .into_iter()
                .map(|d| {
                    surface_vertex(
                        water,
                        &mut vertex_ids,
                        &mut positions,
                        &mut colors,
                        body,
                        level,
                        d,
                    )
                })
                .collect();
            for i in 1..ids.len() - 1 {
                // A kept corner's own point and a mixed-edge midpoint
                // inserted next to it can end up collinear with the fan
                // pivot (ids[0]) when that pivot is itself an endpoint of
                // the same edge — e.g. the wraparound edge back to corner
                // 0. Skip the resulting zero-area sliver instead of
                // emitting a degenerate triangle; it contributes nothing
                // to the rendered shape either way.
                let a = positions[ids[0] as usize];
                let b = positions[ids[i] as usize];
                let c = positions[ids[i + 1] as usize];
                let cross = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                if cross.abs() > 1e-6 {
                    indices.extend([ids[0], ids[i], ids[i + 1]]);
                }
            }
            for tri in tip_tris {
                for d in tri {
                    let id = surface_vertex(
                        water,
                        &mut vertex_ids,
                        &mut positions,
                        &mut colors,
                        body,
                        level,
                        d,
                    );
                    indices.push(id);
                }
            }
        }
    }
    // Concave ":." corners: a land corner wrapped by three beds gets one
    // extra triangle of surface over its cut shelf. Its 3 vertices are
    // pushed directly rather than through `surface_vertex`'s weld map: they
    // sample the exact same depth/shore (via `surface_vertex_depth_shore`,
    // same doubled coords, so positions stay bit-identical to the welded
    // vertices the neighboring cell polygons use — no crack), but the RGB
    // is independently retinted toward the adjacent land's snow/wet state.
    // When there's none, `snow_wet_tint` is a no-op and the color comes out
    // identical to the shared/welded value — zero visual change in normal
    // weather, and no seam against the neighbors either, since nothing
    // differs until there's actually something to tint toward.
    for cy in 0..=MAP_HEIGHT {
        for cx in 0..=MAP_WIDTH {
            let mut land = None;
            let mut sample = None;
            let mut beds = 0;
            for (dx, dy) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
                let cell = GridCoords::new(cx + dx, cy + dy);
                if water.has_bed(cell) {
                    beds += 1;
                    sample = Some(cell);
                } else {
                    land = Some(cell);
                }
            }
            if beds != 3 {
                continue;
            }
            // Three beds around one corner are always 4-way connected
            // through the diagonal cell: one body, one level. Skip while
            // drained. (With three in-bounds beds the land cell is in
            // bounds too — out-of-bounds cells come in pairs at an edge.)
            let sample = sample.expect("count said three beds");
            if !water.is_wet(sample) {
                continue;
            }
            let land = land.expect("count said one dry cell");
            let body = water.body_of[WaterMap::index(sample)].expect("wet cell has a body");
            let level = water.levels[body as usize];
            // The shared corner plus the midpoints of the land cell's two
            // edges meeting there, wound CCW seen from above.
            let mut tri = [
                (2 * cx, 2 * cy),
                (2 * land.x + 1, 2 * cy),
                (2 * cx, 2 * land.y + 1),
            ];
            if (land.x == cx) != (land.y == cy) {
                tri.swap(1, 2);
            }
            let land_snow = snow.get(land).unwrap_or(0.0);
            let land_humidity = humidity.get(land).unwrap_or(0.0);
            for d in tri {
                let (depth, shore) = surface_vertex_depth_shore(water, d);
                let (base, alpha) = water_depth_color(depth, shore);
                let tinted = snow_wet_tint(base, land_snow, land_humidity);
                let mut rgba = tinted.to_linear().to_f32_array();
                rgba[3] = alpha;
                positions.push(surface_vertex_position(level, d));
                colors.push(rgba);
                indices.push((positions.len() - 1) as u32);
            }
        }
    }
    (positions, colors, indices)
}

/// Regenerate the merged surface mesh in place whenever the water changes
/// (startup seeding and devtool level clicks — rare, so a full rebuild is
/// fine) — or whenever a cell's snow/humidity bucket crosses a threshold, so
/// the concave-corner retint (`build_water_surface_geometry`) keeps up with
/// a snowy or wet shoreline. Compare-then-set on a `Local` snapshot, same
/// shape as `rebuild_shore_wedges`: a cheap per-frame scan, but the (much
/// larger) merged mesh only rebuilds on an actual bucket crossing.
pub fn rebuild_water_surface(
    mut commands: Commands,
    water: Res<WaterMap>,
    snow: Res<SnowMap>,
    humidity: Res<HumidityMap>,
    assets: Res<GameAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut spawned: Local<bool>,
    mut surface: Query<&mut Visibility, With<WaterSurfaceMesh>>,
    mut buckets: Local<Vec<(usize, usize)>>,
) {
    let mut current_buckets = Vec::with_capacity((MAP_WIDTH * MAP_HEIGHT) as usize);
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            current_buckets.push((
                snow_bucket(snow.get(cell).unwrap_or(0.0)),
                humidity_bucket(humidity.get(cell).unwrap_or(0.0)),
            ));
        }
    }
    let buckets_changed = *buckets != current_buckets;
    if !water.is_changed() && !buckets_changed {
        return;
    }
    *buckets = current_buckets;
    let (positions, colors, indices) = build_water_surface_geometry(&water, &snow, &humidity);
    let visibility = if positions.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::default()
    };
    if let Some(mesh) = meshes.get_mut(&assets.water_surface_mesh) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0, 1.0, 0.0]; positions.len()],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_indices(Indices::U32(indices));
    }
    if !*spawned {
        *spawned = true;
        commands.spawn((
            Mesh3d(assets.water_surface_mesh.clone()),
            MeshMaterial3d(assets.water_material.clone()),
            Name::new("WaterSurface"),
            WaterSurfaceMesh,
            visibility,
        ));
        return;
    }
    for mut entity_visibility in &mut surface {
        *entity_visibility = visibility;
    }
}

// Corner bits for shore autotiling, CCW from the cell's south-west corner.
// Grid N = +y = world -Z, matching CONNECT_* in construction.rs.
pub const SHORE_SW: u8 = 1;
pub const SHORE_SE: u8 = 2;
pub const SHORE_NE: u8 = 4;
pub const SHORE_NW: u8 = 8;

/// (bit, dx, dy): the diagonal neighbor's offset per corner; the two edge
/// neighbors sharing the corner are (dx, 0) and (0, dy).
const SHORE_CORNERS: [(u8, i32, i32); 4] = [
    (SHORE_SW, -1, -1),
    (SHORE_SE, 1, -1),
    (SHORE_NE, 1, 1),
    (SHORE_NW, -1, 1),
];

/// Bit set per corner of `cell` whose three OTHER meeting cells (the two
/// edge neighbors plus the diagonal) all satisfy the predicate.
fn shore_corner_bits(cell: GridCoords, mut all_of: impl FnMut(GridCoords) -> bool) -> u8 {
    let mut mask = 0;
    for (bit, dx, dy) in SHORE_CORNERS {
        let others = [
            GridCoords::new(cell.x + dx, cell.y),
            GridCoords::new(cell.x, cell.y + dy),
            GridCoords::new(cell.x + dx, cell.y + dy),
        ];
        if others.into_iter().all(&mut all_of) {
            mask |= bit;
        }
    }
    mask
}

/// LAND cell: bit per corner fully wrapped by beds (the concave ":." shore
/// case) — the corner is cut to a diagonal between the two edge midpoints,
/// its dirt lowered to the SHORE_SHELF_DEPTH shelf. Reads the immutable
/// authored beds, not the live level, so a drained river keeps its beveled
/// banks and level changes never churn tile meshes. Purely visual: nav,
/// terrain and water semantics stay square per cell.
pub fn shore_cut_mask(cell: GridCoords, water: &WaterMap) -> u8 {
    if water.has_bed(cell) {
        return 0;
    }
    shore_corner_bits(cell, |c| water.has_bed(c))
}

/// BED cell: bit per corner fully wrapped by land (a convex water tip) —
/// the surface polygon's corner is clipped to the same midpoint diagonal,
/// so concave cuts and convex clips chain into one 45-degree shoreline.
/// The bed cuboid stays square: the sliver it shows past the clipped
/// surface reads as a mud shelf.
pub fn water_clip_mask(cell: GridCoords, water: &WaterMap) -> u8 {
    if !water.has_bed(cell) {
        return 0;
    }
    shore_corner_bits(cell, |c| !water.has_bed(c))
}

/// Corner mask for a per-cell decal tile (snow, wet ground) laid over
/// `cell`: the land cut mask on dry ground, the water clip mask on a bedded
/// (e.g. frozen) cell — so overlays index the same `cover_shore_meshes`
/// family the ground itself uses and follow the beveled shoreline instead of
/// overhanging it as a square.
pub fn decal_shore_mask(cell: GridCoords, water: &WaterMap) -> u8 {
    if water.has_bed(cell) {
        water_clip_mask(cell, water)
    } else {
        shore_cut_mask(cell, water)
    }
}

/// Live-depth classifier for the shallow/deep surface boundary: a cell
/// counts as "deep" only while it's actually wet AND past the deep
/// threshold — unlike `has_bed`/`shore_cut_mask` this reads the live column,
/// so the diagonal boundary re-forms as the water-level devtool or rain
/// shifts depths, instead of being fixed by the authored paint.
fn deep_wet(water: &WaterMap, cell: GridCoords) -> bool {
    is_deep(water.depth(cell))
}

/// Whether the edge between two flanking cells crosses the shallow/deep
/// class line: both wet, but one deep and the other not. Symmetric in `a`
/// and `b`, so both cells flanking a mixed edge agree it's mixed and insert
/// the identical midpoint vertex — the crack-avoidance half of the diagonal
/// boundary (see `deep_boundary_tip_mask` for the shaping half).
fn edge_is_mixed(water: &WaterMap, a: GridCoords, b: GridCoords) -> bool {
    water.is_wet(a) && water.is_wet(b) && deep_wet(water, a) != deep_wet(water, b)
}

/// WET cell: bit per corner whose three OTHER meeting cells are all on the
/// opposite side of the shallow/deep class line — a deep cell wrapped by
/// three shallow, or a shallow cell wrapped by three deep. Mirrors
/// `water_clip_mask`'s shore-tip detection but for the shallow/deep line
/// instead of the land/water one, reusing the same `shore_corner_bits`
/// corner-wrap check. `build_water_surface_geometry` cuts these corners into
/// a separate triangle (the area is kept — it's still water, unlike a shore
/// clip) so the chord between the corner's two already-`edge_is_mixed`
/// midpoints becomes a real mesh edge: that's what bends the soft
/// shallow/deep color band diagonally instead of tracing the grid
/// staircase.
pub fn deep_boundary_tip_mask(cell: GridCoords, water: &WaterMap) -> u8 {
    if !water.is_wet(cell) {
        return 0;
    }
    if deep_wet(water, cell) {
        // Deep cell: corners wrapped entirely by shallow water.
        shore_corner_bits(cell, |c| water.is_wet(c) && !deep_wet(water, c))
    } else {
        // Shallow (or dry-bed) cell: corners wrapped entirely by deep water.
        shore_corner_bits(cell, |c| deep_wet(water, c))
    }
}

/// Immutable-bed classifier for the underwater FLOOR — as opposed to
/// `deep_wet`, which reads the live column for the surface tint, this reads
/// only the authored bed recess, so the floor geometry never churns as the
/// water-level devtool moves levels (matching why `shore_cut_mask`/
/// `water_clip_mask` read `has_bed` rather than `is_wet`).
fn deep_bed(water: &WaterMap, cell: GridCoords) -> bool {
    water.bed(cell) >= WATER_BED_DEEP
}

/// BED cell: bit per corner whose three OTHER meeting cells are all beds of
/// the opposite depth class — the floor counterpart of
/// `deep_boundary_tip_mask`, immutable like `shore_cut_mask`/
/// `water_clip_mask` rather than live-depth-driven. `sync_bed_corners`
/// re-picks each bed tile's mesh from this mask, reusing the same
/// chamfered `ground_shore_meshes` family land tiles use (the cuboid shape
/// is generic — only the material differs), and
/// `build_bed_wedge_geometry` floors the resulting cut corner at the
/// majority class's own depth.
pub fn deep_bed_tip_mask(cell: GridCoords, water: &WaterMap) -> u8 {
    if !water.has_bed(cell) {
        return 0;
    }
    if deep_bed(water, cell) {
        shore_corner_bits(cell, |c| water.has_bed(c) && !deep_bed(water, c))
    } else {
        shore_corner_bits(cell, |c| deep_bed(water, c))
    }
}

/// Corner local XZ (cell center at origin) in the same walk order as
/// SHORE_CORNERS — CCW seen from above; grid +y is world -Z, so SW is
/// (-x, +z).
const SHORE_CORNER_XZ: [(u8, Vec2); 4] = [
    (SHORE_SW, Vec2::new(-0.5 * TILE_SIZE, 0.5 * TILE_SIZE)),
    (SHORE_SE, Vec2::new(0.5 * TILE_SIZE, 0.5 * TILE_SIZE)),
    (SHORE_NE, Vec2::new(0.5 * TILE_SIZE, -0.5 * TILE_SIZE)),
    (SHORE_NW, Vec2::new(-0.5 * TILE_SIZE, -0.5 * TILE_SIZE)),
];

/// The cell's top outline with the masked corners cut back to their edge
/// midpoints: a convex polygon, CCW seen from above, in local XZ. Midpoints
/// are computed identically for both corners of an edge, so the exact float
/// dedup below is safe.
fn chamfered_top_polygon(mask: u8) -> Vec<Vec2> {
    let mut points = Vec::with_capacity(8);
    for (i, (bit, corner)) in SHORE_CORNER_XZ.iter().enumerate() {
        if mask & bit == 0 {
            points.push(*corner);
        } else {
            let prev = SHORE_CORNER_XZ[(i + 3) % 4].1;
            let next = SHORE_CORNER_XZ[(i + 1) % 4].1;
            points.push(corner.midpoint(prev));
            points.push(corner.midpoint(next));
        }
    }
    points.dedup();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

/// Append one flat convex face (fan-triangulated, flat-shaded: vertices are
/// duplicated per face, not shared).
fn push_face(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    face: &[Vec3],
    normal: Vec3,
) {
    let base = positions.len() as u32;
    for p in face {
        positions.push(p.to_array());
        normals.push(normal.to_array());
    }
    for i in 1..face.len() as u32 - 1 {
        indices.extend([base, base + i, base + i + 1]);
    }
}

/// Ground cuboid with the masked corner prisms cut away entirely: the top
/// face keeps the chamfered outline, each cut corner becomes a full-height
/// vertical bank face along the diagonal chord (matching the plain
/// cuboid-side banks), and the cut-adjacent side half-quads are dropped —
/// the chord IS the tile's boundary there. The hole left under a cut
/// corner is floored by a bed-colored wedge from
/// `build_shore_wedge_geometry`. No UVs (plain-color materials) and no
/// bottom face (the ground is the world floor). Mask 0 reproduces the
/// plain ground cuboid.
pub fn shore_dirt_mesh(mask: u8) -> Mesh {
    let top = GROUND_THICKNESS / 2.0;
    let bottom = -GROUND_THICKNESS / 2.0;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let at = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);

    let outline: Vec<Vec3> = chamfered_top_polygon(mask)
        .into_iter()
        .map(|p| at(p, top))
        .collect();
    push_face(
        &mut positions,
        &mut normals,
        &mut indices,
        &outline,
        Vec3::Y,
    );

    for (i, (bit, corner)) in SHORE_CORNER_XZ.iter().enumerate() {
        let prev = SHORE_CORNER_XZ[(i + 3) % 4].1;
        let (next_bit, next) = SHORE_CORNER_XZ[(i + 1) % 4];
        if mask & bit != 0 {
            // Full-height bank face along the diagonal chord.
            let mid_in = corner.midpoint(prev);
            let mid_out = corner.midpoint(next);
            push_face(
                &mut positions,
                &mut normals,
                &mut indices,
                &[
                    at(mid_in, top),
                    at(mid_in, bottom),
                    at(mid_out, bottom),
                    at(mid_out, top),
                ],
                Vec3::new(corner.x, 0.0, corner.y).normalize(),
            );
        }
        // The outer side toward the next corner, split at its midpoint;
        // halves adjacent to a cut corner are dropped (the tile no longer
        // reaches the boundary there — the chord face closes it instead).
        let mid = corner.midpoint(next);
        let dir = next - *corner;
        let out = Vec3::new(-dir.y, 0.0, dir.x).normalize();
        for (a, b, cut) in [
            (*corner, mid, mask & bit != 0),
            (mid, next, mask & next_bit != 0),
        ] {
            if cut {
                continue;
            }
            push_face(
                &mut positions,
                &mut normals,
                &mut indices,
                &[at(a, top), at(a, bottom), at(b, bottom), at(b, top)],
                out,
            );
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Thin overlay slab (the `tile_mesh` footprint and thickness) with the
/// masked corners chamfered off entirely — grass cover on a shore cell
/// picks this so the square overlay doesn't overhang the cut corner as a
/// floating green triangle. Mask 0 reproduces the plain tile slab (minus
/// the never-visible bottom face).
pub fn shore_overlay_mesh(mask: u8) -> Mesh {
    let hi = TILE_THICKNESS / 2.0;
    let lo = -TILE_THICKNESS / 2.0;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let at = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);

    let outline = chamfered_top_polygon(mask);
    let top: Vec<Vec3> = outline.iter().map(|p| at(*p, hi)).collect();
    push_face(&mut positions, &mut normals, &mut indices, &top, Vec3::Y);
    for (i, p) in outline.iter().enumerate() {
        let q = outline[(i + 1) % outline.len()];
        let dir = q - *p;
        let out = Vec3::new(-dir.y, 0.0, dir.x).normalize();
        push_face(
            &mut positions,
            &mut normals,
            &mut indices,
            &[at(*p, hi), at(*p, lo), at(q, lo), at(q, hi)],
            out,
        );
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Re-pick every land tile's mesh from its shore-corner mask, so concave
/// shore corners show their beveled variant. Mirrors
/// `construction::sync_wall_connections`: a cheap full pass, but only when
/// the WaterMap changed (startup seeding — beds never change afterwards) or
/// tiles were just spawned (LDtk cells and their visuals arrive over a few
/// frames).
pub fn sync_shore_corners(
    assets: Res<GameAssets>,
    water: Res<WaterMap>,
    added: Query<(), Added<LandTile>>,
    mut tiles: Query<(&GridCoords, &mut Mesh3d), With<LandTile>>,
) {
    if !water.is_changed() && added.is_empty() {
        return;
    }
    for (grid, mut mesh) in &mut tiles {
        let mask = shore_cut_mask(*grid, &water);
        let want = if mask == 0 {
            assets.ground_mesh.clone()
        } else {
            assets.ground_shore_meshes[mask as usize].clone()
        };
        if mesh.0 != want {
            mesh.0 = want;
        }
    }
}

/// Re-pick every riverbed tile's mesh from its `deep_bed_tip_mask`, mirroring
/// `sync_shore_corners` exactly but for the shallow/deep floor boundary
/// instead of the land/water one — reuses the same `ground_shore_meshes`
/// family since the chamfered cuboid shape is generic, only the material
/// (already `water_bed_material`, set once at spawn) differs.
pub fn sync_bed_corners(
    assets: Res<GameAssets>,
    water: Res<WaterMap>,
    added: Query<(), Added<BedTile>>,
    mut tiles: Query<(&GridCoords, &mut Mesh3d), With<BedTile>>,
) {
    if !water.is_changed() && added.is_empty() {
        return;
    }
    for (grid, mut mesh) in &mut tiles {
        let mask = deep_bed_tip_mask(*grid, &water);
        let want = if mask == 0 {
            assets.ground_mesh.clone()
        } else {
            assets.ground_shore_meshes[mask as usize].clone()
        };
        if mesh.0 != want {
            mesh.0 = want;
        }
    }
}

/// Snow-then-wet tint, bucket-aligned with `SNOW_OVERLAY_ALPHAS`/
/// `WET_OVERLAY_ALPHAS` — the same steps the flat overlay tiles use — so a
/// shore-adjacent patch (a wedge, or the water surface's own concave-corner
/// triangle) never visibly disagrees with the flat tile it's flush against
/// mid-transition (a continuous fraction would drift from the flat tile's
/// stepped alpha everywhere except the two ends of the range). Priority
/// mirrors `weather::overlay_bucket`: enough snow suppresses the wet tint;
/// otherwise humidity wins; a light dusting below that threshold still shows
/// if there's no wetness to eclipse it.
///
/// Deliberately has no freeze/ice pass: an earlier version mixed toward the
/// icy tone by freeze level here too, but that meant a shore corner paled
/// out of step with the flat land tile right next to it (which has no
/// freeze response at all) — and for the water surface's own concave-corner
/// triangle specifically, it double-applied on top of the water material's
/// existing shader-side ice ramp (`assets/shaders/water_surface.wgsl`),
/// since that triangle renders through the same shared material. Freezing
/// water already reads as icy via the shader, uniformly, with no CPU
/// involvement needed; these shore patches only need to track snow/wet like
/// their flat neighbors.
fn snow_wet_tint(base: Color, land_snow: f32, land_humidity: f32) -> Color {
    let land_snow = land_snow.clamp(0.0, 1.0);
    let snow_b = snow_bucket(land_snow);
    let wet_b = humidity_bucket(land_humidity);
    if snow_b > 0 && land_snow >= SNOW_SUPPRESS_WET_MIN {
        base.mix(&SNOW_OVERLAY_COLOR, SNOW_OVERLAY_ALPHAS[snow_b - 1])
    } else if wet_b > 0 {
        base.mix(&WET_OVERLAY_COLOR, WET_OVERLAY_ALPHAS[wet_b - 1])
    } else if snow_b > 0 {
        base.mix(&SNOW_OVERLAY_COLOR, SNOW_OVERLAY_ALPHAS[snow_b - 1])
    } else {
        base
    }
}

/// World-space geometry for ALL shore wedges, one merged mesh (see
/// `rebuild_shore_wedges`). Two kinds, one per qualifying grid corner:
///
/// - **Convex water tip** (1 bed / 3 land): a full-height dirt wedge over
///   the bed cell's clipped corner — top flush with the ground (y = 0),
///   vertical bank face along the clip chord — so the land silhouette runs
///   straight through the tip instead of leaving a square notch of exposed
///   bed.
/// - **Concave cut corner** (3 beds / 1 land, see `shore_cut_mask`): a
///   bed-colored wedge flooring the hole the cut land tile leaves, at the
///   shallower flanking bed's recess, with side skirts down the tile
///   boundaries so deeper neighbor beds don't show a gap.
///
/// Returns `(positions, colors, normals, indices)`; colors are linear RGBA
/// vertex colors over a white material — the dirt checker by the owning
/// cell's own parity (continuing the checkerboard), or the bed color, each
/// tinted toward `SNOW_OVERLAY_COLOR`/`WET_OVERLAY_COLOR` by the surrounding
/// land's snow/humidity (bucket-aligned with the flat overlay tiles — see
/// `snow_wet_tint` — so a wedge never visibly disagrees with the flat tile
/// it's flush against mid-transition). Otherwise a wedge is the one patch of
/// "land" the flat overlay tiles never cover (their footprint follows the
/// clipped shore outline, not the wedge that fills past it), and it reads as
/// a mismatched triangle poking through an otherwise wet or snowy shoreline.
///
/// `seamless` flattens the dirt-tip checker to `DIRT_COLOR_A` for both
/// parities — `rebuild_shore_wedges` passes `!ShowSeams` (checker shows only
/// while that devtool is on) — everything else (bed/snow/wet tints) is
/// unaffected.
pub fn build_shore_wedge_geometry(
    water: &WaterMap,
    snow: &SnowMap,
    humidity: &HumidityMap,
    seamless: bool,
) -> (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let bottom = -GROUND_THICKNESS;
    // World XZ helpers; grid +y is world -Z (see grid_to_world).
    let world_x = |gx: f32| (gx - MAP_WIDTH as f32 / 2.0) * TILE_SIZE;
    let world_z = |gy: f32| -(gy - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE;
    let at = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);
    for cy in 0..=MAP_HEIGHT {
        for cx in 0..=MAP_WIDTH {
            let corners = [(-1, -1), (0, -1), (-1, 0), (0, 0)]
                .map(|(dx, dy)| GridCoords::new(cx + dx, cy + dy));
            let mut beds = 0;
            let mut bed_cell = None;
            let mut land_cell = None;
            for cell in corners {
                if water.has_bed(cell) {
                    beds += 1;
                    bed_cell = Some(cell);
                } else {
                    land_cell = Some(cell);
                }
            }
            let (owner, dirt_tip) = match beds {
                1 => (bed_cell.expect("count said one bed"), true),
                3 => (land_cell.expect("count said one dry cell"), false),
                _ => continue,
            };
            // The wedge footprint: the shared corner plus the midpoints of
            // the owner cell's two edges meeting there, wound CCW seen
            // from above (same quadrant swap rule as the water surface's
            // concave triangles). Vec2 = world (x, z).
            let corner = Vec2::new(world_x(cx as f32), world_z(cy as f32));
            let ew_mid = Vec2::new(world_x(owner.x as f32 + 0.5), world_z(cy as f32));
            let ns_mid = Vec2::new(world_x(cx as f32), world_z(owner.y as f32 + 0.5));
            let mut tri = [corner, ew_mid, ns_mid];
            if (owner.x == cx) != (owner.y == cy) {
                tri.swap(1, 2);
            }
            // A CCW edge's outward normal, for the vertical faces.
            let side = |p: Vec2, q: Vec2| {
                let dir = q - p;
                Vec3::new(-dir.y, 0.0, dir.x).normalize()
            };
            let (top, color) = if dirt_tip {
                // Averaged over the 3 land cells around this tip, not just
                // `owner` (the bed cell) — the wedge continues their drift
                // or dampness across the water corner.
                let land_avg = |get: &dyn Fn(GridCoords) -> f32| {
                    corners
                        .iter()
                        .filter(|c| !water.has_bed(**c))
                        .map(|c| get(*c))
                        .sum::<f32>()
                        / 3.0
                };
                let land_snow = land_avg(&|c| snow.get(c).unwrap_or(0.0));
                let land_humidity = land_avg(&|c| humidity.get(c).unwrap_or(0.0));
                let checker = if seamless || (owner.x + owner.y) % 2 == 0 {
                    DIRT_COLOR_A
                } else {
                    DIRT_COLOR_B
                };
                let base = checker.mix(&SHALLOW_WATER_COLOR, SHORE_TIP_WATER_BLEND);
                let tinted = snow_wet_tint(base, land_snow, land_humidity);
                (0.0, tinted.to_linear().to_f32_array())
            } else {
                // Floor at the shallower of the two beds flanking the land
                // cell's edges (bed-vs-bed steps already exist elsewhere).
                let ew_neighbor = GridCoords::new(owner.x, if owner.y == cy { cy - 1 } else { cy });
                let ns_neighbor = GridCoords::new(if owner.x == cx { cx - 1 } else { cx }, owner.y);
                let floor = -water.bed(ew_neighbor).min(water.bed(ns_neighbor));
                let land_snow = snow.get(owner).unwrap_or(0.0);
                let land_humidity = humidity.get(owner).unwrap_or(0.0);
                let tinted = snow_wet_tint(WATER_BED_COLOR, land_snow, land_humidity);
                (floor, tinted.to_linear().to_f32_array())
            };
            push_face(
                &mut positions,
                &mut normals,
                &mut indices,
                &[at(tri[0], top), at(tri[1], top), at(tri[2], top)],
                Vec3::Y,
            );
            // Vertical faces: a dirt tip closes only the chord toward the
            // water (its boundary edges hide behind the flanking land
            // tiles' full-height sides); a concave wedge closes only its
            // boundary edges toward the flanking beds (its chord hides
            // behind the cut land tile's full-height bank).
            let edges: &[(Vec2, Vec2)] = if dirt_tip {
                &[(tri[1], tri[2])]
            } else {
                &[(tri[0], tri[1]), (tri[2], tri[0])]
            };
            for &(p, q) in edges {
                push_face(
                    &mut positions,
                    &mut normals,
                    &mut indices,
                    &[at(p, top), at(p, bottom), at(q, bottom), at(q, top)],
                    side(p, q),
                );
            }
            colors.resize(positions.len(), color);
        }
    }
    (positions, colors, normals, indices)
}

/// Marks the single merged shore-wedge mesh entity.
#[derive(Component)]
pub struct ShoreWedgeMesh;

/// Regenerate the merged wedge mesh whenever the water changes — beds are
/// immutable, so in practice once, after the LDtk seeding (same trigger and
/// shape as `rebuild_water_surface`; an empty mesh draws nothing, so no
/// visibility juggling) — or whenever a cell's snow or humidity bucket
/// crosses a threshold, so the wedge tint keeps up with a snowy or wet
/// shoreline. Compare-then-set on a `Local` snapshot (mirrors
/// `snow::sync_snow_costs`): cheap per-frame scan, but only rebuilds the
/// merged mesh on an actual bucket crossing, not every tick snow or
/// humidity drifts by a fraction.
pub fn rebuild_shore_wedges(
    mut commands: Commands,
    water: Res<WaterMap>,
    snow: Res<SnowMap>,
    humidity: Res<HumidityMap>,
    show: Res<ShowSeams>,
    assets: Res<GameAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut spawned: Local<bool>,
    mut buckets: Local<Vec<(usize, usize)>>,
) {
    let mut current_buckets = Vec::with_capacity((MAP_WIDTH * MAP_HEIGHT) as usize);
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            current_buckets.push((
                snow_bucket(snow.get(cell).unwrap_or(0.0)),
                humidity_bucket(humidity.get(cell).unwrap_or(0.0)),
            ));
        }
    }
    let buckets_changed = *buckets != current_buckets;
    if !water.is_changed() && !buckets_changed && !show.is_changed() {
        return;
    }
    *buckets = current_buckets;
    // `build_shore_wedge_geometry`'s `seamless` flag flattens the checker;
    // `ShowSeams` is the opposite sense (on = checker), so invert it here.
    let (positions, colors, normals, indices) =
        build_shore_wedge_geometry(&water, &snow, &humidity, !show.0);
    if let Some(mesh) = meshes.get_mut(&assets.shore_wedge_mesh) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_indices(Indices::U32(indices));
    }
    if !*spawned {
        *spawned = true;
        commands.spawn((
            Mesh3d(assets.shore_wedge_mesh.clone()),
            MeshMaterial3d(assets.shore_wedge_material.clone()),
            Name::new("ShoreWedges"),
            ShoreWedgeMesh,
        ));
    }
}

/// World-space geometry for the shallow/deep bed-floor wedges: one triangle
/// per `deep_bed_tip_mask` corner, filling the hole `sync_bed_corners`'s
/// chamfer leaves in the minority-class tile with a floor at the majority
/// class's own depth — the underwater counterpart of
/// `build_shore_wedge_geometry`. Simpler than that function in two ways:
/// no vertex colors (the bed is one uniform color regardless of depth), and
/// full skirts on all three edges rather than a case-by-case subset — a tip
/// corner requires all three majority cells to share the exact same class
/// by construction, so there's no depth-mismatch gap to route around; the
/// skirts here are just overlap insurance against the chamfered tile's own
/// bank face. Returns `(positions, normals, indices)` in world space.
pub fn build_bed_wedge_geometry(water: &WaterMap) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let bottom = -(GROUND_THICKNESS + WATER_BED_DEEP);
    let world_x = |gx: f32| (gx - MAP_WIDTH as f32 / 2.0) * TILE_SIZE;
    let world_z = |gy: f32| -(gy - MAP_HEIGHT as f32 / 2.0) * TILE_SIZE;
    let at = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);
    let side = |p: Vec2, q: Vec2| {
        let dir = q - p;
        Vec3::new(-dir.y, 0.0, dir.x).normalize()
    };
    for cy in 0..=MAP_HEIGHT {
        for cx in 0..=MAP_WIDTH {
            let mut deep_count = 0;
            let mut shallow_count = 0;
            let mut deep_cell = None;
            let mut shallow_cell = None;
            let mut all_beds = true;
            for (dx, dy) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
                let cell = GridCoords::new(cx + dx, cy + dy);
                if !water.has_bed(cell) {
                    all_beds = false;
                    break;
                }
                if deep_bed(water, cell) {
                    deep_count += 1;
                    deep_cell = Some(cell);
                } else {
                    shallow_count += 1;
                    shallow_cell = Some(cell);
                }
            }
            if !all_beds {
                continue;
            }
            let (owner, floor) = match (deep_count, shallow_count) {
                (1, 3) => (deep_cell.expect("count said one deep"), -WATER_BED_SHALLOW),
                (3, 1) => (
                    shallow_cell.expect("count said one shallow"),
                    -WATER_BED_DEEP,
                ),
                _ => continue,
            };
            // Footprint: the shared corner plus the midpoints of the
            // owner's two edges meeting there, wound CCW seen from above —
            // same quadrant-swap rule as `build_shore_wedge_geometry`.
            let corner = Vec2::new(world_x(cx as f32), world_z(cy as f32));
            let ew_mid = Vec2::new(world_x(owner.x as f32 + 0.5), world_z(cy as f32));
            let ns_mid = Vec2::new(world_x(cx as f32), world_z(owner.y as f32 + 0.5));
            let mut tri = [corner, ew_mid, ns_mid];
            if (owner.x == cx) != (owner.y == cy) {
                tri.swap(1, 2);
            }
            push_face(
                &mut positions,
                &mut normals,
                &mut indices,
                &[at(tri[0], floor), at(tri[1], floor), at(tri[2], floor)],
                Vec3::Y,
            );
            for &(p, q) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                push_face(
                    &mut positions,
                    &mut normals,
                    &mut indices,
                    &[at(p, floor), at(p, bottom), at(q, bottom), at(q, floor)],
                    side(p, q),
                );
            }
        }
    }
    (positions, normals, indices)
}

/// Marks the single merged bed-wedge mesh entity — the underwater
/// counterpart of `ShoreWedgeMesh`.
#[derive(Component)]
pub struct BedWedgeMesh;

/// Regenerate the merged bed-wedge mesh whenever the water changes — beds
/// are immutable, so in practice once, after the LDtk seeding (same trigger
/// and shape as `rebuild_shore_wedges`).
pub fn rebuild_bed_wedges(
    mut commands: Commands,
    water: Res<WaterMap>,
    assets: Res<GameAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut spawned: Local<bool>,
) {
    if !water.is_changed() {
        return;
    }
    let (positions, normals, indices) = build_bed_wedge_geometry(&water);
    if let Some(mesh) = meshes.get_mut(&assets.bed_wedge_mesh) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_indices(Indices::U32(indices));
    }
    if !*spawned {
        *spawned = true;
        commands.spawn((
            Mesh3d(assets.bed_wedge_mesh.clone()),
            MeshMaterial3d(assets.water_bed_material.clone()),
            Name::new("BedWedges"),
            BedWedgeMesh,
        ));
    }
}
