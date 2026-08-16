# Add an LDtk map element

The map is `assets/maps/colony.ldtk` (open it in the LDtk editor; the level
is 64×64 cells — `MAP_WIDTH`/`MAP_HEIGHT` must match the Ground layer).
bevy_ecs_ldtk loads it **data-only**: registered bundles put marker
components + `GridCoords` on plain data entities, and our `Added<…>`-driven
bridge systems build the real 3D world from them (see the architecture
guide). Two element shapes exist:

- **IntGrid value** — painted per cell, on one of two IntGrid layers:
  - **Ground** — natural terrain (Dirt, water, Fertile, Grass; the water
    paints author `WaterMap` beds on Dirt terrain, see the checklist note).
  - **Zones** — player-editable zone paint, today just Stockpile (see
    `zones::StockpileZoneCell`). A cell on this layer keeps its own genuine
    Ground terrain underneath; the zone is tracked separately (`map::Stockpile`
    / the `Zone`/`ZoneRegion` entities), never mutating `TerrainMap`.
- **Entity** on the Entities layer — placed things, with fields (Pawn,
  Bush) or without (Tree, Wall).

**Pitfall — no unpainted Ground cells**: every cell must carry a Ground
value, including cells under entities. A Wall entity stands *on* natural
terrain (`construction::spawn_walls_from_ldtk` does no ground backfill), so
a wall cell left at 0 renders as a hole in the ground. When rebuilding or
enlarging the map, fill the whole Ground layer first, then place entities.

## Checklist: new IntGrid value (terrain paint)

Worked example: `FERTILE`.

1. **LDtk editor**: add the value to the Ground layer's IntGrid definition
   and paint some cells. Note its integer.
2. Const in `config.rs` (`pub const FERTILE: i32 = 6;`) with a doc comment
   saying what the paint means.
3. Marker + bundle in `map.rs`:
   ```rust
   #[derive(Default, Component)]
   pub struct FertileCell;

   #[derive(Default, Bundle, LdtkIntCell)]
   pub struct FertileCellBundle { cell: FertileCell }
   ```
4. `register_ldtk_int_cell::<map::FertileCellBundle>(FERTILE)` in
   `game.rs`.
5. Bridge systems on `Query<&GridCoords, Added<FertileCell>>`, registered in
   the bridge tuple in `game.rs`:
   - **Visual**: spawn the ground tile (`map::spawn_fertile_visuals`; reuse
     `spawn_dirt_tile` if the ground is plain dirt).
   - **Terrain**: an arm in `map::mark_terrain` setting the `TerrainMap`
     entry — usually with a new `Terrain` variant (arms in `label()`,
     `base_cost()`, `fertility()`; audit the predicates listed in guide 02
     if it interacts with construction/roofing). Wetness is `WaterMap`'s
     job, never a `Terrain` variant: the water paints (`SHALLOW_WATER`/
     `DEEP_WATER`) leave their cells' terrain as plain `Dirt` and instead
     author a per-cell bed via `map::mark_water_map` — anything
     water-adjacent should read `WaterMap::depth()/is_wet()/has_bed()`.
   - **Nav**: a `nav::mark_*` system if walkability/cost differ from dirt.
6. Check the tooltip: `ui::update_tooltip` reads `Terrain::label()` — a new
   variant shows up automatically.

**Shore bevels come free with beds.** Shorelines are autotiled visually:
`map::shore_cut_mask` (land corner wrapped by 3 beds → the ground tile swaps
to a `ground_shore_meshes` variant with the corner prism cut away, grass
overlays likewise via `cover_shore_meshes`) and `map::water_clip_mask` (bed
corner wrapped by 3 land cells → the surface polygon's corner is clipped)
turn L-shaped shore steps into 45° diagonals, and the merged wedge mesh
(`map::rebuild_shore_wedges`) fills clipped tips with full-height dirt
prisms and floors cut corners with bed-colored triangles — a diagonal
staircase of paint renders as one straight shoreline. All of it reads
`WaterMap::has_bed`, so any new water-like paint that calls `set_bed` in
`mark_water_map` gets beveled shores automatically. Purely visual: nav,
terrain and water semantics stay square per cell.

Two special precedents worth knowing before inventing a new value:

- **Seed paint, not terrain**: `GRASS` cells are plain dirt terrain — the
  paint just seeds a full-coverage `cover::Flora` entity
  (`cover::seed_grass_from_ldtk`). If your element is "something ON the
  ground", follow that split instead of adding terrain.
- **Converted to runtime state**: the LDtk `STOCKPILE` block is turned into
  a normal editable `Zone` by `zones::seed_zones_from_ldtk`. Because
  Stockpile is painted on its own **Zones** IntGrid layer rather than on
  Ground, the cell keeps its real natural terrain underneath — no dirt
  backfill needed (an LDtk cell holds one value *per layer*, so a second
  IntGrid layer is how you paint an area without displacing the terrain
  value on Ground). Any paint that seeds player-editable state should
  follow the same shape: its own IntGrid layer (not Ground) + one-shot
  seeding (a `Local<bool>` guard) into the runtime state.

## Checklist: new entity type (placed thing)

Worked example: `Bush` (has a field), `Tree` (fieldless).

1. **LDtk editor**: define the entity (name, size, fields) on the Entities
   layer and place instances.
2. In the owning module: a `XSpawn` marker + an `LdtkEntity` bundle:
   ```rust
   #[derive(Default, Bundle, LdtkEntity)]
   pub struct BushSpawnBundle {
       marker: BushSpawn,
       #[grid_coords]
       grid_coords: GridCoords,
       #[with(berry_bush_from_field)]
       berry_bush: BerryBush,
   }
   ```
   `#[grid_coords]` fills the cell address; `#[with(fn)]` extractors read
   fields off the `EntityInstance`
   (`entity_instance.get_int_field("Growth")`, `get_string_field`, …).
3. `register_ldtk_entity::<XSpawnBundle>("Name")` in `game.rs` — the string
   must match the LDtk entity identifier exactly.
4. A visual bridge on `Added<XSpawn>` (`flora::spawn_bush_visual` is the
   template) spawning the **3D game entity**: mesh/material from
   `GameAssets`, `Transform` from `map::grid_to_world`, `Name`, gameplay
   components, `GridCoords`, `Selectable`. Gameplay state lives on this
   entity — the LDtk data entity only carries initial values.
5. If the entity affects nav (like trees), a `nav::mark_*` system keyed on
   the game entity's `Added<X>`.

## Pitfalls

- **Undeclared fields fail silently.** The extractors use
  `unwrap_or(default)`, so a field the code reads but the LDtk project never
  defines just yields the default — no error, no log. This is live today:
  `units::skills_from_fields` reads `FarmSkill` and `ForestrySkill`, but
  `colony.ldtk` only declares `Id`, `PawnName`, `Growth`,
  `HarvestSkill`/`HaulSkill`/`BuildSkill` — every pawn silently gets tier C
  farm/forestry. When adding a field, change **both** the extractor and the
  LDtk entity definition, then verify the value shows in-game (pawn info
  panel, tooltip, …).
- **Cells arrive frames after startup.** Never read LDtk-derived state in a
  Startup system; bridge with `Added` filters, and use a `Local<bool>` +
  "is the query non-empty yet" guard for one-shot seeding
  (`cover::seed_algae`). Same reason neighbor-mask autotiling (shore bevels,
  wall connections) must never pick a mesh at spawn time — the neighbors may
  not exist yet. Spawn with the plain mesh and let a sync system
  (`map::sync_shore_corners`, `construction::sync_wall_connections`) re-pick
  handles on `is_changed()` / `Added` triggers.
- **Dimensions are duplicated on purpose**: `MAP_WIDTH`/`MAP_HEIGHT` in
  `config.rs` size every map resource (`TerrainMap`, `NavGrid`, `RoofMap`,
  `CoverMap`). Resizing the level means updating them together.
- **The LDtk file is JSON** — mergeable but noisy. Prefer editing in the
  LDtk editor; hand-editing the entity/IntGrid *definitions* is error-prone
  (instances reference definition uids).
