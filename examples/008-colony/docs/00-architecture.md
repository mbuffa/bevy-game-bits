# Architecture: the lay of the land

Everything lives flat in `examples/008-colony/` — no submodule directories.
`main.rs` declares the modules and adds `game::GamePlugin`, which wires the
entire app. `SPEC.md` is the running design log (why things are the way they
are); this file is the map of *what is where*.

## Module map

| File | What it owns |
| --- | --- |
| `main.rs` | Entry point: module list, `ClearColor`, `DefaultPlugins` + `GamePlugin`. |
| `game.rs` | `GamePlugin` (all wiring), `SimState` pause state, `GameAssets` resource, `setup_assets` (every mesh/material). |
| `config.rs` | Every tuning constant: map dims, IntGrid values, colors, costs, growth rates, priorities, UI sizes. |
| `map.rs` | LDtk IntGrid cell bundles/markers, `Terrain` enum (dry-land kinds only), `TerrainMap`, `WaterMap` — the single wetness authority: authored per-cell *beds* + flood-filled water *bodies* each holding a *level*; the live column is `depth() = (level + bed).max(0)`, placement gates on `has_bed()`. Water renders as recessed per-cell bed cuboids plus ONE merged translucent surface mesh (`rebuild_water_surface` regenerates it on `water.is_changed()`): one polygon per wet cell — convex water tips clipped to 45° between edge midpoints (`water_clip_mask`), concave land corners covered by an extra welded triangle — with vertices at the body's level, per-vertex colors blending the depth tint (`WATER_TINT_DEEP_DEPTH`) and shore alpha fade (`WATER_SHORE_FADE`) CPU-side — the GPU interpolation is what smooths shallow↔deep and bank edges. The shallow/deep tint band itself is bent onto the same 45° diagonals as the shoreline: `deep_boundary_tip_mask` (live `depth()`-driven, unlike the immutable-bed shore masks) flags corners fully wrapped by the opposite class and cuts them into their own welded triangle, while `edge_is_mixed` inserts the shared midpoint on any shallow/deep edge so both flanking cells weld to the identical vertex — no T-junctions. Land tiles at concave shore corners swap to beveled `ground_shore_meshes` variants (`shore_cut_mask`/`sync_shore_corners`); the merged wedge mesh (`rebuild_shore_wedges`) fills convex tips with full-height dirt prisms and floors cut corners with bed-colored triangles, so a diagonal LDtk staircase renders as one straight shoreline. The surface uses `WaterMaterial` (StandardMaterial + `WaterExtension`, `assets/shaders/water_surface.wgsl`): a fragment shader tilting the lighting normal with waves scrolling along `WATER_FLOW_DIRECTION`, fed by `weather::sync_water_material`; the mesh's vertex colors flow through it untouched (white base color). A second uniform (`ice`, binding 101) carries `snow::FreezeLevel`: freezing flattens the waves and lerps the surface toward an opaque matte ice sheet, all shader-side — no mesh rebuild. `RoofMap`/`RoofPolicy`, `Stockpile` resource (what `zones.rs` populates), `grid_to_world`/`world_to_grid`, tile-visual bridges. Unit-tested. |
| `nav.rs` | `NavGrid` (walkable + per-cell base cost + an additive snow-penalty layer summed in `cost()` — base writers and `snow::sync_snow_costs` never stomp each other), A* `find_path`, `water_cost` (depth → step cost), `ice_or_water_cost` (frozen bed → `TERRAIN_COST_ICE`), `mark_water`/`mark_trees`, `sync_water_costs` (re-derives bed-cell costs on level changes or the ice threshold flipping). Unit-tested. |
| `movement.rs` | `MoveOrder`/`Path`, `plan_paths`, `move_along_path` (float movement, terrain-scaled speed), `Wader`/`wade_dip(bed, depth)` (pawns walk the bed, capped while fording; a drained bed is just lower ground; solid ice carries the pawn — no dip), `sync_grid_coords`. |
| `director.rs` | The AI director: `Job`/`JobKind`, generate/assign/execute/cleanup, `Carrying`, `deliver_carried`, `WorkProgress`, `Stuck`. |
| `units.rs` | `Pawn`, `PawnSpawnBundle` (LDtk), `Tier`, `Skills`, `Allowance`, `ManualMode` (drafted — off AI control). |
| `items.rs` | `ItemKind`, `ItemStack`, `STACK_CAP`, `find_drop_cell`, `pour_yield`, `spawn_item_stack`. Unit-tested. |
| `flora.rs` | Berry bushes: `BerryBush`, `Harvestable`/`ToHarvest`, `HarvestCommand`, growth + harvest. |
| `crops.rs` | Planted crops: `Crop`/`CropKind`, `spawn_crop`, `grow_crops`. |
| `trees.rs` | Trees: `Tree`, `ToCut`, `CutCommand`, growth/spread/fell/plant; a wildfire chars them into snags (`Tree.burnt`: no growth/fuel/wood, still cuttable so the player clears them via the normal Cut flow). |
| `cover.rs` | Ground cover on top of terrain: `Flora`/`FloraKind` (grass/algae), `CoverMap`, `fertility_at`, `fuel_at` (per-cell wildfire fuel: terrain-gated grass cover + standing-plant `fuel()` contributions via `collect_plant_fuel`/`plant_fuel_at`). |
| `shrubs.rs` | Shrubs: decorative standing fuel — grows, spreads slowly onto grass, never harvestable; exists to burn, and does: a wildfire that consumes its cell erases it. |
| `fire.rs` | Wildfire: `FireMap` (per-cell burning state + devtool ignition queue) / `ScorchMap` (healing burn marks debuffing fertility and blocking grass re-seed), fixed-tick spread driven by fuel x local wind (`wind_at`) x humidity, rain douses, burnout consumes grass cover; burning/scorch overlays (always shown) + one Hanabi flame/smoke emitter per burning cell drifting with the wind. `BurnCommand` fires when a cell's fire ends; if it burned most of its fuel (`burn_kills_plants`) trees char to snags and shrubs burn away (ember-glow material swaps while burning). Bushes/crops are still fireproof and pawns still ignore fire. Unit-tested. |
| `construction.rs` | `Blueprint`/`BuildableKind`, `ConstructionMap` (what's built where — a layer on top of natural `Terrain`, never replacing it), Supply/Build flow, `complete_builds` (runtime construction+nav mutation), the `Wall` LDtk entity bridge (`spawn_walls_from_ldtk`), roofs, wall autotiling, `enclosed_interior`, wind turbines (free prototype building: nacelle yaws upwind, rotor spins with strength), `ToDemolish`/`DemolishCommand`/`do_demolitions` (the player-facing Demolish order's completion: despawns the structure, reverts its footprint via `demolish_at`, and pours `DEMOLISH_REFUND_FRACTION` of its wood cost back — contrast the instant, no-refund `Tool::Demolish` devtool). Unit-tested. |
| `zones.rs` | `Zone`/`ZoneKind`/`ZoneRegion`, `GrowOrder`, `ZonePlant`, `Tool`/`ActiveTool`, `drag_zone_tool` (all drag placement), the `StockpileZoneCell` LDtk bridge (Zones IntGrid layer -> `Zone`/`map::Stockpile`). Unit-tested. |
| `selection.rs` | `Selectable`, `SelectedEntity`, `cursor_to_cell`, click-select (cycling) and right-click move (`click_move` — only acts on a drafted `ManualMode` pawn). |
| `scene.rs` | Orthographic camera, orbit/zoom/pan, `center_camera_on`. Two views: the classic isometric build view and a straight-down top-down view (F2, `toggle_camera_view`) for lining up rectangular rooms — `CameraViewMode` tracks which is active/animating and the saved isometric orbit yaw to restore; `iso_view_active`/`camera_settled` gate orbit (locked north-up in top-down) and zoom/pan (frozen mid-swap) respectively. Cursor picking (`selection::cursor_to_cell`) is projection-agnostic, so no other system needed to change. |
| `ui.rs` | All HUD: selection panel + `TileAction` buttons, `GeneralAction` bar, allowance grid (`ToggleWork`), portraits, tooltip, clock, pause banner, zone panels. |
| `debug_ui.rs` | Backtick debug window: `DebugTab` (Jobs / Weather / Fire / Build), job-pool listing, rain/season(Spring↔Winter)/heavy-wind/shift-wind + overlay toggles (ground wind, altitude wind, fuel, temperature), water-level raise/lower buttons (nudge every body's level by `WATER_LEVEL_STEP`; the map/nav/cover followers react on their own), ignite tool + extinguish-all devtools, instant demolish devtool (`zones::Tool::Demolish`, despawns any built structure on the dragged cells and reverts its terrain/nav/roof state via `construction::demolish_at`, no material refund — contrast the player-facing Demolish order, a real timed job that salvages wood, armed from `ui.rs`'s Demolish category instead). |
| `daynight.rs` | `GameClock`, day/night lighting, sun/moon sky-widget rendering. |
| `temperature.rs` | `Season`/`ActiveSeason` (Spring and Winter, flipped by the Weather-tab season devtool until a calendar exists; Winter runs −20→−4 °C so nights guarantee full freeze), `ambient_temperature` (sine hump over the day: night base at sunrise, seasonal peak at noon; flat base all night — deliberately not `daylight()`, whose short lighting fade would square-wave the curve), `TemperatureMap` (per-cell °C, flat-grid model: outdoor cells ARE the ambient, roofed cells drift toward it at `INDOOR_TEMP_RATE_PER_SECOND` scaled by insulation), auto-managed Room zones (`sync_rooms` re-derives `ZoneKind::Room` + `Room { insulation }` entities from `ConstructionMap` via `detect_rooms`, one shared-visited connected-components pass; `room_insulation` averages `WALL_INSULATION`/`DOOR_INSULATION` over the interior's 8-way ring), cold-blue→warm-red debug overlay (`ShowTemperatureOverlay`, Weather tab). Consumed by `snow.rs` (melt rate, freeze level) and `weather.rs` (precipitation form); `fire.rs` is the intended future heat source, rain coupling deferred. Unit-tested. |
| `snow.rs` | Snow and ice: `SnowMap` (per-cell cover 0..=1, flat-grid model — accumulates while snowing on unroofed cells, melts ∝ the cell's °C above zero, so the sun's daily hump is the radiance coupling and melt peaks at noon; melting cells soak the `HumidityMap` via `weather::update_humidity`), `FreezeLevel` (global 0..=1 scalar: climbs whenever ambient ≤ 0 °C, faster the colder — one `FREEZE_FULL_BELOW_C` (−20 °C) night freezes a river solid — and thaws only above 0 °C per-degree, so sub-zero ice never melts back; `is_frozen()` at `ICE_WALKABLE_MIN_FREEZE` gates walkable ice, the wade dip, snow-on-ice, and the "Ice" tooltip label), white bucketed snow overlay (always shown; suppresses the wet sheen via `weather::overlay_bucket`), `sync_snow_costs` (mirrors snow buckets into `NavGrid`'s additive penalty layer). Pure fns (`next_snow`, `freeze_target`, `next_freeze`, `snow_cost_penalty`) unit-tested. |
| `weather.rs` | `Weather` + `Wind` resources (permanent gusting, slowly wandering wind; a devtool shifts its heading, always swinging smoothly); `Weather.rain` means "precipitation on" and `precipitation_form(ambient)` picks rain vs snow at 0 °C, with two Hanabi effects (rain streaks, slow billboarded snow flakes — both authored in `setup`, both slanted by the live wind via the shared `"wind_velocity"` property; `sync_precipitation_effects` runs exactly one), always-on gust "snakes" (ribbon trails behind wind-gliding emitter heads), `HumidityMap` + wet-ground overlay (rain — and actively melting snow — soaks, clear sky dries, snow cover hides the sheen), `WindExposureMap` (per-cell wind exposure 0..=1, banded by height via `WindBand`: short blockers — walls/doors/turbine bases — only shelter `Ground`, growth-scaled trees shelter both `Ground` and `Altitude` (only thing tall enough to slow a turbine today), roofed cells are becalmed in both bands; `wind_at(cell, wind, band)` is the consumer API; separate ground/altitude debug overlay toggles), `WindSwayMaterial` (`assets/shaders/wheat_sway.wgsl`, wheat's vertex sway; `map::WaterMaterial` is the repo's other custom shader — both follow the same pattern: a `MaterialExtension` over StandardMaterial plus an ungated `sync_*_material` system mirroring sim state into the uniform, with sim time passed through the uniform so a pause freezes the animation). |

Assets live outside the example: the map at `assets/maps/colony.ldtk`
(path const `LDTK_PROJECT_PATH` in `config.rs`) and the checkbox font at
`assets/fonts/FiraSans-Bold.ttf` (`CHECKBOX_MARK_FONT`).

## GamePlugin wiring (`game.rs`)

One plugin does everything, in this order inside `impl Plugin for GamePlugin`:

1. **Plugins**: `LdtkPlugin`, `HanabiPlugin`,
   `MaterialPlugin::<weather::WindSwayMaterial>` (the wheat-sway shader),
   `MaterialPlugin::<map::WaterMaterial>` (the water-ripple shader).
2. **LDtk registrations**: `register_ldtk_int_cell::<map::…CellBundle>(VALUE)`
   for each Ground-layer value, `register_ldtk_entity::<…SpawnBundle>("Name")`
   for Pawn/Bush/Tree. New map content registers here (guide 06).
3. **Resources**: `init_resource` for `NavGrid`, `TerrainMap`, `RoofMap`,
   `Stockpile`, `CoverMap`, `SelectedEntity`, `ActiveTool`, `GameClock`,
   `Weather`, `Wind`, `HumidityMap`, `ActiveSeason`, etc.
4. **State**: `init_state::<SimState>()`.
5. **Messages**: `add_message` for `HarvestCommand`, `CutCommand`,
   `BuildCommand`, `RoofCommand`. New "the sim decided X happened" signals
   follow this pattern.
6. **Startup chain** (`.chain()`, order matters — everything needs
   `GameAssets` first): `setup_assets → scene::setup → daynight::setup →
   weather::setup → ui::setup → debug_ui::setup → map::spawn_ldtk_world`.
7. **Update systems**, grouped as described below.

## Conventions every recipe relies on

### All tuning lives in `config.rs`

Colors, sizes, costs, rates, priorities, key bindings. No magic numbers in
systems. A new feature adds its constants there, with a doc comment saying
what the number means (and its unit).

### All meshes/materials are procedural, created once

There are no external models. `setup_assets` (`game.rs`) builds every mesh
from Bevy primitives (`Cuboid`, `Sphere`, `Cone`) and every material as a
flat-color `StandardMaterial`, storing the handles in the **`GameAssets`**
resource. World objects clone handles from it — shared handles are what let
Bevy batch the whole map. A new world object means: color/size consts in
`config.rs` → handle field(s) on `GameAssets` → creation in `setup_assets` →
spawn with `Mesh3d(assets.x.clone())` + `MeshMaterial3d(...)`.

Autotiled shapes are pre-baked **16-mesh families indexed by a 4-bit
neighbor mask**, with a sync system re-picking handles after the fact:
`wall_meshes` (edge mask, `construction::wall_connect_mask` /
`sync_wall_connections`) and the shore bevels `ground_shore_meshes` /
`cover_shore_meshes` (corner mask, `map::shore_cut_mask` /
`map::sync_shore_corners` / `cover::sync_shore_cover`). The bevels' corner
wedges (full-height dirt prisms filling convex water tips, bed-colored
floors under cut land corners) live in one merged vertex-colored mesh
(`shore_wedge_mesh` + white `shore_wedge_material`,
`map::rebuild_shore_wedges`) — a second merged-mesh entity alongside the
water surface. The shore bevels are purely visual —
`NavGrid`/`TerrainMap`/`WaterMap` stay square per cell (a wading pawn may
visually clip a cut corner's wedge; accepted).

### `SimState` pause gating

`SimState` (`game.rs`) is `Running`/`Paused`, toggled by Space
(`toggle_pause`). The rule of thumb:

- **Sim systems** (director pipeline, movement, growth, clock, humidity,
  temperature) run
  under `.run_if(in_state(SimState::Running))`.
- **UI, selection, camera, orders** stay ungated: the player can inspect and
  queue orders while paused; they take effect on resume.
- **Message readers whose messages are written by sim-gated systems must
  stay ungated.** Bevy messages are double-buffered; a gated reader skipping
  frames across a pause would silently drop them. That is why
  `construction::complete_builds`, `apply_roof_commands`, and
  `cancel_blueprints` run ungated (see the comment in `game.rs`). Same
  reasoning made blueprint cancel a `ToCancel` *component* instead of a
  message. When you add a completion signal, decide: component (pause-proof,
  queryable) or message (transient) — and if message, keep the reader ungated.

### The bridge-system pattern (LDtk is data-only)

bevy_ecs_ldtk's 2D renderer is unused; it spawns pure *data* entities (grid
coords + marker components in a pixel-scale 2D hierarchy). Standalone systems
with `Added<XCell>` / `Added<XSpawn>` filters then build the real 3D world:
visuals (`map::spawn_tile_visuals`, `units::spawn_pawn_visual`, …), terrain
(`map::mark_terrain`; water is separate: `map::mark_water_map` records the
authored beds, chained ahead of the water visual/nav bridges so they read
same-frame depths, then `map::build_water_bodies` one-shot flood-fills the
beds into leveled bodies and the followers —
`map::rebuild_water_surface`, `nav::sync_water_costs`,
`cover::sync_algae_to_water`, plus the shore-bevel mesh syncs
`map::sync_shore_corners`/`cover::sync_shore_cover` — react whenever the
`WaterMap` changes), nav
(`nav::mark_water`/`mark_trees`,
`construction::spawn_walls_from_ldtk` for walls), cover, zones
(`zones::seed_zones_from_ldtk` reads the dedicated Zones IntGrid layer to
seed the stockpile as a normal `Zone`). The 3D game
entity — not the LDtk data entity — carries `Selectable`, `GridCoords`, and
gameplay components. LDtk cells arrive a few frames after startup, so bridges
run every frame on `Added` filters rather than once at Startup.

### Grid vs world, and the two parallel maps

- `GridCoords` (from bevy_ecs_ldtk) is the cell address; `map::grid_to_world`
  / `map::world_to_grid` convert to/from XZ-plane world positions (the map is
  centered on the origin; grid +y is world −Z).
- **`TerrainMap`** answers "what is this cell" (`Terrain` enum — dry-land
  kinds only); **`WaterMap`** answers "how much water stands on it" (a river
  cell's terrain is plain `Dirt` shaped into a bed); **`NavGrid`** answers
  "can I walk here and how expensive is it". They are parallel arrays and
  must be **mutated together**. At load, the bridge systems fill all three.
  At runtime the mutators are — copy those:
  `construction::complete_builds` (wall: `construction.set` +
  `set_walkable(false)`; door: `construction.set` +
  `set_cost(TERRAIN_COST_DOOR)` — natural terrain never changes when
  something is built on it), `trees::fell_trees` (cost reverts to
  `Terrain::base_cost()`), and the water-level devtool
  (`WaterMap::shift_all_levels`, with `nav::sync_water_costs` re-deriving
  every bed cell's cost from the new depths).
- Terrain cost is the single source for A* step cost **and** walk speed
  (`movement::move_along_path` scales speed by
  `TERRAIN_COST_DIRT / nav.cost(cell)`). Never set a cost below
  `TERRAIN_COST_DIRT` — the A* octile heuristic assumes it as the floor.
  `NavGrid::cost()` returns base + the snow-penalty layer
  (`snow::sync_snow_costs` owns that vec exclusively); penalties only add,
  so the floor holds.

### Jobs are entities; the director is a chained pipeline

Work is an entity holding `Job { kind: JobKind }` + `JobPriority`, optionally
`AssignedTo`, `Stuck`, `WorkProgress`. Pawns point back with `CurrentJob`.
The pipeline order in `game.rs` (one `.chain()`, sim-gated):

```
choose_objectives → generate_jobs → generate_walk_jobs → cleanup_jobs
→ enforce_allowances → release_manual_pawns → tick_stuck → assign_jobs
→ execute_jobs → deliver_carried → update_pawn_status
```

Designations are components on the target (`ToHarvest`, `ToCut`) or derived
from map state (roof jobs from `RoofMap` policy-vs-roofed gaps) —
`generate_jobs` turns them into job entities, `cleanup_jobs` despawns jobs
whose reason disappeared. Guide 03 has the full recipe.

### Manual mode (drafting a pawn)

`units::ManualMode` is a marker a pawn either has or doesn't — present means
"drafted," off AI control. The player toggles it via the Manual checkbox in
the selected pawn's info panel (`ui::sync_manual_checkbox` /
`toggle_manual_mode`). Drafting drops whatever the pawn was doing at once
(`director::release_manual_pawns`, mirroring `enforce_allowances`) and the
pawn is excluded from `assign_jobs`, `generate_walk_jobs`, `execute_jobs`, and
`deliver_carried` (`Without<ManualMode>` on each). The only thing that still
moves a drafted pawn is `selection::click_move` (right-click), which — the
other direction — only acts on a pawn that *has* `ManualMode`. Un-drafting
does nothing special; the pawn simply becomes eligible for the pipeline
again next frame.

### Run conditions beyond pause

`selection::click_select` / `click_move` run under
`.run_if(zones::tool_inactive)` so drag-placing a zone never also selects.
UI buttons/panels guard against click-through by putting an
`Interaction::default()` component on panel roots (systems check "is any UI
node hovered" before handling map clicks).
