//! Ground cover: per-cell `Flora { kind, coverage }` living ON the substrate
//! (grass on dirt, algae on shallow water), as opposed to `map::Terrain`
//! which is only what the ground IS. Cover regrows, spreads to neighboring
//! cells its kind can root on (grass slowly creeps over plain dirt — a
//! deliberate world-sim choice), boosts the cell's growing fertility, and is
//! the substrate-vs-cover split that later mechanics (wildfire fuel,
//! grazing) build on. The LDtk `GRASS` IntGrid paint is seed data: those
//! cells start at full coverage.

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::construction::ConstructionMap;
use crate::director::WanderRng;
use crate::game::GameAssets;
use crate::map::{self, GrassCell, ShallowWaterCell, ShowSeams, Terrain, TerrainMap};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FloraKind {
    Grass,
    Algae,
}

impl FloraKind {
    pub fn label(&self) -> &'static str {
        match self {
            FloraKind::Grass => "grass",
            FloraKind::Algae => "algae",
        }
    }

    /// Which substrates this kind can live on — the whole rooting rule.
    /// `built` gates out cells with a wall/door/turbine on them: ground
    /// terrain no longer changes when something is built there (see
    /// `construction::ConstructionMap`), so without this gate a built cell
    /// would read as plain `Dirt` and grass would happily root on it.
    /// `depth` is the cell's live water column (`map::WaterMap::depth`):
    /// grass needs dry ground (a drained riverbed will green over in
    /// time), algae needs standing shallow water. A stockpile zone is NOT
    /// gated here — it's a purely logical area (where hauled goods get
    /// dropped), not a physical surface, so grass grows under one exactly
    /// like it would on bare dirt.
    pub fn roots_on(&self, substrate: Terrain, built: bool, depth: f32) -> bool {
        if built {
            return false;
        }
        match self {
            FloraKind::Grass => {
                matches!(substrate, Terrain::Dirt | Terrain::Fertile) && depth <= 0.0
            }
            FloraKind::Algae => depth > 0.0 && !map::is_deep(depth),
        }
    }

    fn regrow_per_second(&self) -> f32 {
        match self {
            FloraKind::Grass => GRASS_REGROW_PER_SECOND,
            FloraKind::Algae => ALGAE_REGROW_PER_SECOND,
        }
    }

    fn spread_secs(&self) -> f32 {
        match self {
            FloraKind::Grass => GRASS_SPREAD_SECS,
            FloraKind::Algae => ALGAE_SPREAD_SECS,
        }
    }
}

/// A patch of cover on one cell, 0.0..=1.0 thick.
#[derive(Component)]
pub struct Flora {
    pub kind: FloraKind,
    pub coverage: f32,
}

/// Countdown to a thick patch's next attempt to seed a neighbor.
#[derive(Component)]
pub struct CoverSpreadTimer(pub Timer);

/// Per-cell cover lookup, rebuilt each frame from the `Flora` entities —
/// the cheap read for fertility, tree rooting, tooltips, and future fire.
#[derive(Resource)]
pub struct CoverMap {
    cells: Vec<Option<(FloraKind, f32)>>,
}

impl Default for CoverMap {
    fn default() -> Self {
        Self {
            cells: vec![None; (MAP_WIDTH * MAP_HEIGHT) as usize],
        }
    }
}

impl CoverMap {
    fn in_bounds(cell: GridCoords) -> bool {
        (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y)
    }

    pub fn get(&self, cell: GridCoords) -> Option<(FloraKind, f32)> {
        Self::in_bounds(cell)
            .then(|| self.cells[(cell.y * MAP_WIDTH + cell.x) as usize])
            .flatten()
    }

    pub(crate) fn set(&mut self, cell: GridCoords, cover: Option<(FloraKind, f32)>) {
        if Self::in_bounds(cell) {
            self.cells[(cell.y * MAP_WIDTH + cell.x) as usize] = cover;
        }
    }
}

pub fn sync_cover_map(mut cover: ResMut<CoverMap>, flora: Query<(&Flora, &GridCoords)>) {
    cover.cells.fill(None);
    for (patch, grid) in &flora {
        cover.set(*grid, Some((patch.kind, patch.coverage)));
    }
}

/// Effective growing fertility of a cell: the substrate's base plus the
/// grass bonus (algae doesn't feed land plants). Dirt at full grass = the
/// old grass terrain's 0.6 exactly. A wet cell grows nothing — land plants
/// don't live underwater (wetness is `WaterMap`'s call, not terrain's).
pub fn fertility_at(
    cell: GridCoords,
    terrain: &TerrainMap,
    cover: &CoverMap,
    water: &map::WaterMap,
) -> f32 {
    if water.is_wet(cell) {
        return 0.0;
    }
    let base = terrain.get(cell).map_or(1.0, |t| t.fertility());
    let bonus = match cover.get(cell) {
        Some((FloraKind::Grass, coverage)) => FLORA_FERTILITY_BONUS * coverage,
        _ => 0.0,
    };
    base + bonus
}

/// Wildfire fuel of a cell, 0..=1: gated to real, dry land (submerged
/// cells are inert no matter what sits on them — grass that rooted on a
/// drained bed stops being fuel the moment the water returns), ground fuel
/// from grass cover, plus whatever standing plant the caller found on the
/// cell (`plant_fuel`, see `plant_fuel_at`/`collect_plant_fuel`). Humidity
/// is deliberately NOT part of fuel: fuel is what CAN burn; wetness gates
/// ignition in the fire sim. Built cells (walls/doors/turbines) are inert
/// too — checked separately from terrain, since ground terrain no longer
/// changes when something is built on it (see
/// `construction::ConstructionMap`). A stockpile zone is NOT gated here:
/// it's a logic area, not a physical surface, so whatever grass and plants
/// are actually on the cell burn normally.
pub fn fuel_at(
    cell: GridCoords,
    terrain: &TerrainMap,
    construction: &ConstructionMap,
    cover: &CoverMap,
    water: &map::WaterMap,
    plant_fuel: f32,
) -> f32 {
    if construction.get(cell).is_some() || terrain.get(cell).is_none() || water.is_wet(cell) {
        return 0.0;
    }
    let ground = match cover.get(cell) {
        Some((FloraKind::Grass, coverage)) => FUEL_GRASS * coverage,
        _ => 0.0,
    };
    (ground + plant_fuel).clamp(0.0, 1.0)
}

/// Fold `(cell, fuel)` pairs from plant queries into a flat per-cell vec
/// (the `update_wind_exposure` scratch-vec model), keeping the max where
/// plants ever share a cell. Callers chain their species queries:
/// `trees.iter().map(|(t, g)| (*g, t.fuel())).chain(...)`.
pub fn collect_plant_fuel(plants: impl IntoIterator<Item = (GridCoords, f32)>) -> Vec<f32> {
    let mut fuel = vec![0.0f32; (MAP_WIDTH * MAP_HEIGHT) as usize];
    for (cell, plant) in plants {
        if CoverMap::in_bounds(cell) {
            let index = (cell.y * MAP_WIDTH + cell.x) as usize;
            fuel[index] = fuel[index].max(plant);
        }
    }
    fuel
}

/// One cell's standing-plant fuel from chained `(cell, fuel)` pairs — the
/// single-cell sibling of `collect_plant_fuel` for tooltip-style lookups.
pub fn plant_fuel_at(cell: GridCoords, plants: impl IntoIterator<Item = (GridCoords, f32)>) -> f32 {
    plants
        .into_iter()
        .filter(|(grid, _)| *grid == cell)
        .map(|(_, plant)| plant)
        .fold(0.0, f32::max)
}

/// Coverage bucket for the overlay visual (4 steps of greening).
fn coverage_bucket(coverage: f32) -> usize {
    ((coverage * 4.0) as usize).min(3)
}

/// `seamless` flattens the checker to index 0 for both parities. Callers
/// pass `!ShowSeams` — the devtool's polarity is "show the checker", the
/// opposite of this flag — mirroring `map::spawn_dirt_tile`'s dirt checker.
fn flora_material(
    assets: &GameAssets,
    grid: &GridCoords,
    coverage: f32,
    seamless: bool,
) -> Handle<StandardMaterial> {
    let bucket = coverage_bucket(coverage);
    let checker = if seamless {
        0
    } else {
        ((grid.x + grid.y) % 2) as usize
    };
    assets.grass_cover_materials[bucket][checker].clone()
}

/// Grass sits a hair above the ground tile (and below zone tiles at +0.002).
const GRASS_OFFSET: f32 = 0.001;

/// Visual Y-scale of an algae reed cluster at the given coverage: a short
/// stub at 0%, full `ALGAE_REED_HEIGHT` at 100% (the `crop_scale` analog —
/// see `crops::crop_scale`).
pub fn algae_scale(coverage: f32) -> f32 {
    ALGAE_MIN_SCALE + (1.0 - ALGAE_MIN_SCALE) * coverage
}

/// The algae visual: a small cluster of thin reeds poking out of the water,
/// merged into one mesh (one draw, one entity) like `crops::wheat_cluster_mesh`.
/// Each reed is a stack of `ALGAE_REED_SEGMENTS` cylinders sharing boundary
/// vertices, so the wind sway shader bends it into a curve. Base of every
/// reed is at y=0; the tallest reaches `ALGAE_REED_HEIGHT`.
pub fn algae_reed_mesh() -> Mesh {
    // (XZ offset in units of ALGAE_REED_SPREAD, height factor): hand-placed
    // so the cluster reads organic, with an uneven top line.
    const REEDS: [(Vec2, f32); ALGAE_REED_COUNT] = [
        (Vec2::new(-0.6, -0.3), 1.0),
        (Vec2::new(0.5, -0.5), 0.8),
        (Vec2::new(-0.4, 0.5), 0.9),
        (Vec2::new(0.5, 0.4), 0.7),
        (Vec2::new(0.0, 0.0), 0.85),
    ];
    let mut mesh: Option<Mesh> = None;
    for (offset, height_factor) in REEDS {
        let segment_height = ALGAE_REED_HEIGHT * height_factor / ALGAE_REED_SEGMENTS as f32;
        for segment in 0..ALGAE_REED_SEGMENTS {
            let piece = Mesh::from(Cylinder::new(ALGAE_REED_RADIUS, segment_height)).translated_by(
                Vec3::new(
                    offset.x * ALGAE_REED_SPREAD,
                    (segment as f32 + 0.5) * segment_height,
                    offset.y * ALGAE_REED_SPREAD,
                ),
            );
            match &mut mesh {
                Some(mesh) => mesh.merge(&piece).unwrap(),
                None => mesh = Some(piece),
            }
        }
    }
    mesh.unwrap()
}

/// Plant cover at `cell`: the entity IS the overlay visual. Grass is a flat
/// tinted tile; algae is a reed cluster scaled by coverage (different mesh
/// AND material type — `WindSwayMaterial`, not `StandardMaterial` — so the
/// two kinds spawn along separate branches).
pub fn spawn_flora(
    commands: &mut Commands,
    assets: &GameAssets,
    kind: FloraKind,
    grid: GridCoords,
    coverage: f32,
    seamless: bool,
) {
    match kind {
        FloraKind::Grass => {
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(flora_material(assets, &grid, coverage, seamless)),
                Transform::from_translation(
                    map::grid_to_world(&grid) - Vec3::Y * TILE_THICKNESS / 2.0
                        + Vec3::Y * GRASS_OFFSET,
                ),
                Name::new("Flora"),
                Flora { kind, coverage },
                grid,
            ));
        }
        FloraKind::Algae => {
            commands.spawn((
                Mesh3d(assets.algae_reed_mesh.clone()),
                MeshMaterial3d(assets.algae_reed_material.clone()),
                Transform::from_translation(
                    map::grid_to_world(&grid) - Vec3::Y * TILE_THICKNESS / 2.0
                        + Vec3::Y * (WATER_SURFACE_Y + 0.001),
                )
                .with_scale(Vec3::new(1.0, algae_scale(coverage), 1.0)),
                Name::new("Flora"),
                Flora { kind, coverage },
                grid,
            ));
        }
    }
}

/// The LDtk `GRASS` paint seeds full-coverage grass; the substrate itself
/// is plain dirt (`map::mark_terrain`, `map::spawn_grass_visuals`).
pub fn seed_grass_from_ldtk(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowSeams>,
    seeds: Query<&GridCoords, Added<GrassCell>>,
) {
    for grid in &seeds {
        spawn_flora(
            &mut commands,
            &assets,
            FloraKind::Grass,
            *grid,
            1.0,
            !show.0,
        );
    }
}

/// Start the river with a few algae patches once its cells exist (LDtk
/// spawns them a few frames in, so this waits rather than running at
/// Startup).
pub fn seed_algae(
    mut commands: Commands,
    assets: Res<GameAssets>,
    show: Res<ShowSeams>,
    mut rng: ResMut<WanderRng>,
    mut done: Local<bool>,
    shallows: Query<&GridCoords, With<ShallowWaterCell>>,
) {
    if *done {
        return;
    }
    let cells: Vec<&GridCoords> = shallows.iter().collect();
    if cells.is_empty() {
        return;
    }
    *done = true;
    for _ in 0..ALGAE_SEED_COUNT {
        let cell = cells[rng.range(0, cells.len() as i32 - 1) as usize];
        spawn_flora(
            &mut commands,
            &assets,
            FloraKind::Algae,
            *cell,
            FLORA_SEED_COVERAGE,
            !show.0,
        );
    }
}

/// Coverage thickens wherever the kind can (still) root; something built on
/// the cell freezes it instead (see `roots_on`) — a stockpile zone does not.
pub fn grow_cover(
    time: Res<Time>,
    terrain: Res<TerrainMap>,
    construction: Res<ConstructionMap>,
    water: Res<map::WaterMap>,
    mut flora: Query<(&mut Flora, &GridCoords)>,
) {
    for (mut patch, grid) in &mut flora {
        if patch.coverage >= 1.0 {
            continue;
        }
        let built = construction.get(*grid).is_some();
        if terrain
            .get(*grid)
            .is_some_and(|t| patch.kind.roots_on(t, built, water.depth(*grid)))
        {
            patch.coverage =
                (patch.coverage + patch.kind.regrow_per_second() * time.delta_secs()).min(1.0);
        }
    }
}

/// Thick cover creeps: patches at `FLORA_SPREAD_MIN`+ carry a staggered
/// timer; on expiry, roll one 8-neighbor and seed it if the kind roots on
/// its substrate, nothing covers it yet, and the ground isn't too scorched
/// — burned land re-greens from the edges inward as the scorch heals.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn spread_cover(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<GameAssets>,
    terrain: Res<TerrainMap>,
    construction: Res<ConstructionMap>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    scorch: Res<crate::fire::ScorchMap>,
    show: Res<ShowSeams>,
    mut rng: ResMut<WanderRng>,
    mut flora: Query<(Entity, &Flora, &GridCoords, Option<&mut CoverSpreadTimer>)>,
) {
    let mut taken: Vec<GridCoords> = Vec::new();
    for (entity, patch, grid, timer) in &mut flora {
        if patch.coverage < FLORA_SPREAD_MIN {
            continue;
        }
        let Some(mut timer) = timer else {
            let offset = rng.range(0, patch.kind.spread_secs() as i32) as f32;
            commands
                .entity(entity)
                .insert(CoverSpreadTimer(Timer::from_seconds(
                    patch.kind.spread_secs() + offset,
                    TimerMode::Once,
                )));
            continue;
        };
        if !timer.0.tick(time.delta()).is_finished() {
            continue;
        }
        timer.0 = Timer::from_seconds(patch.kind.spread_secs(), TimerMode::Once);
        let candidate = GridCoords::new(grid.x + rng.range(-1, 1), grid.y + rng.range(-1, 1));
        let built = construction.get(candidate).is_some();
        let rootable = terrain.get(candidate).is_some_and(|substrate| {
            patch
                .kind
                .roots_on(substrate, built, water.depth(candidate))
        });
        if rootable
            && cover.get(candidate).is_none()
            && scorch.get(candidate) <= SCORCH_REGROW_MAX
            && !taken.contains(&candidate)
        {
            info!("cover: {} spreads to {candidate:?}", patch.kind.label());
            spawn_flora(
                &mut commands,
                &assets,
                patch.kind,
                candidate,
                FLORA_SEED_COVERAGE,
                !show.0,
            );
            taken.push(candidate);
        }
    }
}

/// Keep algae riding its body's water level: the patch floats at the live
/// surface and hides while its cell is drained (it stops thickening and
/// spreading too, via `roots_on`, but the patch survives to re-float when
/// the water returns). Runs on level changes, plus whenever new flora
/// appeared so late-seeded or spread algae get placed against a moved
/// level right away.
pub fn sync_algae_to_water(
    water: Res<map::WaterMap>,
    mut flora: Query<(&Flora, &GridCoords, &mut Transform, &mut Visibility)>,
    fresh: Query<(), Added<Flora>>,
) {
    if !water.is_changed() && fresh.is_empty() {
        return;
    }
    for (patch, grid, mut transform, mut visibility) in &mut flora {
        if patch.kind != FloraKind::Algae {
            continue;
        }
        let level = water.surface_level(*grid).unwrap_or(WATER_SURFACE_Y);
        transform.translation.y = -TILE_THICKNESS / 2.0 + level + 0.001;
        *visibility = if water.is_wet(*grid) {
            Visibility::default()
        } else {
            Visibility::Hidden
        };
    }
}

/// Keep grass overlays on shore cells chamfered like the ground tile under
/// them (`map::sync_shore_corners`), so the square slab doesn't overhang a
/// cut corner as a floating green triangle. Same triggers as the tile sync,
/// plus fresh flora (grass spreads onto shore cells at runtime). Algae
/// keeps its own reed-cluster mesh — it floats on the water, not on a cut.
pub fn sync_shore_cover(
    assets: Res<GameAssets>,
    water: Res<map::WaterMap>,
    fresh: Query<(), Added<Flora>>,
    mut flora: Query<(&Flora, &GridCoords, &mut Mesh3d)>,
) {
    if !water.is_changed() && fresh.is_empty() {
        return;
    }
    for (patch, grid, mut mesh) in &mut flora {
        if patch.kind != FloraKind::Grass {
            continue;
        }
        let mask = map::shore_cut_mask(*grid, &water);
        let want = if mask == 0 {
            assets.tile_mesh.clone()
        } else {
            assets.cover_shore_meshes[mask as usize].clone()
        };
        if mesh.0 != want {
            mesh.0 = want;
        }
    }
}

/// Keep each grass patch's overlay material on its coverage bucket. Algae no
/// longer has a `MeshMaterial3d<StandardMaterial>` (its reed material is a
/// `WindSwayMaterial`), so this query naturally only ever matches grass.
pub fn update_cover_visuals(
    assets: Res<GameAssets>,
    show: Res<ShowSeams>,
    mut flora: Query<(&Flora, &GridCoords, &mut MeshMaterial3d<StandardMaterial>), Changed<Flora>>,
) {
    for (patch, grid, mut material) in &mut flora {
        debug_assert_eq!(patch.kind, FloraKind::Grass);
        let wanted = flora_material(&assets, grid, patch.coverage, !show.0);
        if material.0 != wanted {
            material.0 = wanted;
        }
    }
}

/// Reconciles every grass patch's material against `ShowSeams` when the
/// devtool flips — `update_cover_visuals` above only reacts to
/// `Changed<Flora>`, which the toggle itself never touches, so this is the
/// full-sweep counterpart (mirrors `map::sync_seamless_dirt`).
pub fn sync_seamless_grass(
    assets: Res<GameAssets>,
    show: Res<ShowSeams>,
    mut flora: Query<(&Flora, &GridCoords, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    if !show.is_changed() {
        return;
    }
    for (patch, grid, mut material) in &mut flora {
        if patch.kind != FloraKind::Grass {
            continue;
        }
        let wanted = flora_material(&assets, grid, patch.coverage, !show.0);
        if material.0 != wanted {
            material.0 = wanted;
        }
    }
}

/// Keep each algae reed cluster's height scaled to its coverage — the reeds
/// grow taller as the patch thickens (`crops::grow_crops`'s scale-update is
/// the analog for crops).
pub fn sync_algae_scale(mut flora: Query<(&Flora, &mut Transform), Changed<Flora>>) {
    for (patch, mut transform) in &mut flora {
        if patch.kind != FloraKind::Algae {
            continue;
        }
        transform.scale.y = algae_scale(patch.coverage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construction::BuildableKind;

    /// The live columns beds carry at the starting level.
    const SHALLOW_COLUMN: f32 = WATER_SURFACE_Y + WATER_BED_SHALLOW;
    const DEEP_COLUMN: f32 = WATER_SURFACE_Y + WATER_BED_DEEP;

    /// A WaterMap with standing water on the one cell.
    fn wet(cell: GridCoords) -> map::WaterMap {
        let mut water = map::WaterMap::default();
        water.set_bed(cell, WATER_BED_SHALLOW);
        water.rebuild_bodies();
        water
    }

    #[test]
    fn rooting_rules_per_kind() {
        assert!(FloraKind::Grass.roots_on(Terrain::Dirt, false, 0.0));
        assert!(FloraKind::Grass.roots_on(Terrain::Fertile, false, 0.0));
        // Grass needs dry ground; any standing water blocks it.
        assert!(!FloraKind::Grass.roots_on(Terrain::Dirt, false, SHALLOW_COLUMN));
        // Built cells never root, regardless of the ground underneath.
        assert!(!FloraKind::Grass.roots_on(Terrain::Dirt, true, 0.0));
        assert!(FloraKind::Algae.roots_on(Terrain::Dirt, false, SHALLOW_COLUMN));
        // Algae needs standing water — a drained bed is dry ground —
        // and shallow at that.
        assert!(!FloraKind::Algae.roots_on(Terrain::Dirt, false, 0.0));
        assert!(!FloraKind::Algae.roots_on(Terrain::Dirt, false, DEEP_COLUMN));
    }

    #[test]
    fn fertility_composes_substrate_plus_grass() {
        let cell = GridCoords::new(3, 3);
        let terrain = TerrainMap::default(); // all dirt
        let water = map::WaterMap::default(); // all dry
        let mut cover = CoverMap::default();
        // Bare dirt: base only.
        assert_eq!(fertility_at(cell, &terrain, &cover, &water), DIRT_FERTILITY);
        // Full grass on dirt reproduces the old grass terrain's 0.6.
        cover.set(cell, Some((FloraKind::Grass, 1.0)));
        let full = fertility_at(cell, &terrain, &cover, &water);
        assert!((full - 0.6).abs() < 1e-6);
        // Half coverage, half bonus.
        cover.set(cell, Some((FloraKind::Grass, 0.5)));
        let half = fertility_at(cell, &terrain, &cover, &water);
        assert!((half - (DIRT_FERTILITY + FLORA_FERTILITY_BONUS * 0.5)).abs() < 1e-6);
        // Algae feeds nothing on land rules.
        cover.set(cell, Some((FloraKind::Algae, 1.0)));
        assert_eq!(fertility_at(cell, &terrain, &cover, &water), DIRT_FERTILITY);
        // Underwater nothing grows, whatever the substrate says.
        cover.set(cell, Some((FloraKind::Grass, 1.0)));
        assert_eq!(fertility_at(cell, &terrain, &cover, &wet(cell)), 0.0);
    }

    #[test]
    fn fuel_is_zero_on_wet_or_built_cells() {
        let cell = GridCoords::new(3, 3);
        let terrain = TerrainMap::default();
        let construction = ConstructionMap::default();
        let mut cover = CoverMap::default();
        cover.set(cell, Some((FloraKind::Grass, 1.0)));
        // Even with full grass AND a standing plant: submerged is inert.
        assert_eq!(
            fuel_at(cell, &terrain, &construction, &cover, &wet(cell), 1.0),
            0.0
        );
        let dry = map::WaterMap::default();
        // Off-map too.
        assert_eq!(
            fuel_at(
                GridCoords::new(-1, 0),
                &terrain,
                &construction,
                &cover,
                &dry,
                1.0
            ),
            0.0
        );
        // A built cell over otherwise-flammable dirt is inert too.
        let mut construction = ConstructionMap::default();
        construction.set(cell, Some(BuildableKind::Wall));
        assert_eq!(
            fuel_at(cell, &terrain, &construction, &cover, &dry, 1.0),
            0.0
        );
    }

    #[test]
    fn fuel_scales_with_grass_coverage() {
        let cell = GridCoords::new(3, 3);
        let terrain = TerrainMap::default(); // all dirt
        let construction = ConstructionMap::default();
        let water = map::WaterMap::default();
        let mut cover = CoverMap::default();
        let fuel =
            |cover: &CoverMap, plant| fuel_at(cell, &terrain, &construction, cover, &water, plant);
        assert_eq!(fuel(&cover, 0.0), 0.0); // bare dirt
        cover.set(cell, Some((FloraKind::Grass, 1.0)));
        assert!((fuel(&cover, 0.0) - FUEL_GRASS).abs() < 1e-6);
        cover.set(cell, Some((FloraKind::Grass, 0.5)));
        assert!((fuel(&cover, 0.0) - FUEL_GRASS * 0.5).abs() < 1e-6);
        // Algae carries no fuel.
        cover.set(cell, Some((FloraKind::Algae, 1.0)));
        assert_eq!(fuel(&cover, 0.0), 0.0);
    }

    #[test]
    fn plant_fuel_adds_and_clamps() {
        let cell = GridCoords::new(3, 3);
        let terrain = TerrainMap::default();
        let construction = ConstructionMap::default();
        let water = map::WaterMap::default();
        let mut cover = CoverMap::default();
        // A plant on bare dirt is fuel by itself.
        assert!((fuel_at(cell, &terrain, &construction, &cover, &water, 0.7) - 0.7).abs() < 1e-6);
        // Full grass + a big plant clamps at 1.0.
        cover.set(cell, Some((FloraKind::Grass, 1.0)));
        assert_eq!(
            fuel_at(cell, &terrain, &construction, &cover, &water, 1.0),
            1.0
        );
    }

    #[test]
    fn plant_fuel_folds_pick_the_cell_max() {
        let a = GridCoords::new(2, 2);
        let b = GridCoords::new(5, 5);
        let plants = [(a, 0.4), (a, 0.9), (b, 0.2), (GridCoords::new(-3, 0), 1.0)];
        assert!((plant_fuel_at(a, plants) - 0.9).abs() < 1e-6);
        assert!((plant_fuel_at(b, plants) - 0.2).abs() < 1e-6);
        assert_eq!(plant_fuel_at(GridCoords::new(9, 9), plants), 0.0);
        let folded = collect_plant_fuel(plants);
        assert!((folded[(a.y * MAP_WIDTH + a.x) as usize] - 0.9).abs() < 1e-6);
        assert!((folded[(b.y * MAP_WIDTH + b.x) as usize] - 0.2).abs() < 1e-6);
        // The out-of-bounds plant was ignored, not a panic.
        assert_eq!(folded.len(), (MAP_WIDTH * MAP_HEIGHT) as usize);
    }

    #[test]
    fn coverage_buckets_quantize_cleanly() {
        assert_eq!(coverage_bucket(0.0), 0);
        assert_eq!(coverage_bucket(0.24), 0);
        assert_eq!(coverage_bucket(0.25), 1);
        assert_eq!(coverage_bucket(0.6), 2);
        assert_eq!(coverage_bucket(0.99), 3);
        assert_eq!(coverage_bucket(1.0), 3);
    }

    #[test]
    fn algae_scale_ranges_from_min_to_full() {
        assert!((algae_scale(0.0) - ALGAE_MIN_SCALE).abs() < 1e-6);
        assert!((algae_scale(1.0) - 1.0).abs() < 1e-6);
        // Monotonically increasing in between.
        assert!(algae_scale(0.5) > algae_scale(0.25));
        assert!(algae_scale(0.75) > algae_scale(0.5));
    }

    fn reed_positions() -> Vec<[f32; 3]> {
        let mesh = algae_reed_mesh();
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            bevy::mesh::VertexAttributeValues::Float32x3(positions) => positions.clone(),
            other => panic!("unexpected position format: {other:?}"),
        }
    }

    #[test]
    fn reed_cluster_sits_on_the_water_and_tops_out_at_reed_height() {
        let ys: Vec<f32> = reed_positions().iter().map(|p| p[1]).collect();
        let min = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let max = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(min.abs() < 1e-6, "reed bases should sit at y=0, got {min}");
        assert!(
            (max - ALGAE_REED_HEIGHT).abs() < 1e-6,
            "tallest reed should reach ALGAE_REED_HEIGHT, got {max}"
        );
    }

    #[test]
    fn reed_cluster_stays_inside_the_reed_spread() {
        let reach = ALGAE_REED_SPREAD + ALGAE_REED_RADIUS;
        for p in reed_positions() {
            assert!(p[0].abs() <= reach + 1e-6);
            assert!(p[2].abs() <= reach + 1e-6);
        }
    }
}
