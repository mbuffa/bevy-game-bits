# Add a new zone kind, or a new plantable

Two recipes: a **zone kind** (like Stockpile/Growing — a player-drawn area
that drives behavior) and a **plantable** (what a growing zone plants —
`ZonePlant::Wheat`/`Trees` — or a whole new plant species).

## Zones: how they're built

A zone is an entity: `Zone { kind: ZoneKind }` + **`ZoneRegion`** — a
free-form `HashSet<GridCoords>` (any shape, holes and islands allowed; all
consumers work cell-by-cell). Kind-specific state rides along as extra
components: growing zones carry `GrowOrder` and `ZonePlant`, stockpiles
nothing, rooms `temperature::Room`. Overlay tiles are rebuilt from scratch
by `rebuild_zone_visuals` whenever the region changes; Delete/Expand/Shrink
come free via the generic `TileAction`s on anything `With<Zone>`.

**Derivation, not duplication**: `zones::sync_stockpile_cells` derives
`map::Stockpile.cells` from the live stockpile zones every frame. Never
write it directly — create/edit the zone and let the sync own the
downstream state. A new zone kind that needs a fast lookup should copy this
pattern (its own derived resource + sync system), not push cells into
someone else's.

**Auto-managed zones**: `ZoneKind::Room` is the first zone the *simulation*
draws — `temperature::sync_rooms` despawns and re-derives every Room zone
whenever `ConstructionMap` changes (see `temperature::detect_rooms`). Rooms
are exempt from the one-zone-per-tile `occupied` set in `drag_zone_tool`
(both directions: they overlap player zones and never block them), spawn no
overlay tiles (`rebuild_zone_visuals` skips them), are never `Selectable`,
and `Tool::Expand` refuses them. A future auto-managed kind should follow
the same exemptions — the checklist below is written for player-drawn
kinds.

## Checklist: new zone kind

1. `zones::ZoneKind` variant + `label()` arm.
2. Overlay material: color const in `config.rs`, handle on `GameAssets` +
   creation in `setup_assets`, arm in the material match inside
   `rebuild_zone_visuals`.
3. Placement: `zones::Tool` variant + `label()`; `ui::GeneralAction` variant
   + `ALL` + `label()` + arm in `run_general_actions`; spawn arm in
   `drag_zone_tool` (copy `Tool::PlaceGrowing`: filter the drawn rect through
   an eligibility predicate + the "no cell belongs to two zones" `occupied`
   set, spawn `Zone` + `ZoneRegion::from_cells` + your extra components +
   `Name`).
4. Eligibility predicate beside `is_buildable` (stockpiles: walkable plain
   `Dirt`) / `is_growable` (walkable `Dirt | Fertile`). `drag_zone_tool`'s
   `Tool::Expand` arm must also map your kind to it.
5. Behavior: if the zone drives work, generate jobs from it in
   `director::generate_jobs` (the Farming block iterates
   `Query<(&ZoneRegion, &GrowOrder, &ZonePlant)>` — the query's component
   set is what scopes it to growing zones) and void them in `cleanup_jobs`
   when the cell leaves the zone.
6. Zone-scoped panel UI (a checkbox like Grow, a cycle button like Plant):
   see guide 05; `sync_grow_checkbox`/`toggle_grow_order` are the template —
   visibility keys off "does the selected entity have my component".

Pitfall: `seed_zones_from_ldtk` converts the LDtk-painted stockpile into a
normal zone and backfills dirt tiles under it (LDtk cells hold one IntGrid
value, so those cells have no ground of their own). An LDtk-seeded zone of a
new kind needs the same backfill treatment.

## Checklist: new `ZonePlant` choice (existing species)

1. `zones::ZonePlant` variant + arms in `label()`, `next()` (the panel cycle
   button), `accepts_cell()` (density rule — wheat packs every cell, trees
   keep a `TREE_PLANT_SPACING` grid).
2. Planting arm in `director::execute_jobs`' Farming match
   (`ZonePlant::Wheat => crops::spawn_crop`, `Trees => trees::plant_sapling`
   — add yours).
3. Job voiding is generic: `cleanup_jobs` voids Farming jobs whose zone
   switched plants (`*zone_plant == plant` capture check).

## Checklist: new plant species

Decide which template fits:

- **Crop-like** (planted, grows, consumed on harvest): extend `crops.rs`.
  `CropKind` variant + `label()` + `growth_per_second()` (const in
  `config.rs`); parameterize `spawn_crop` — it currently hardcodes
  `CropKind::Wheat`, its `Name`, and the shared `crop_mesh` (a merged
  stalk-cluster built by `crops::wheat_cluster_mesh`) / `crop_material`
  (a `weather::WindSwayMaterial`, not a `StandardMaterial` — its vertex
  shader bends the stalks under wind, so a new crop wanting sway should
  reuse it).
  Growth (`grow_crops`), 100%-triggers-`ToHarvest`, and the info panel are
  kind-generic already.
- **Bush-like** (persists, regrows after harvest): mirror `flora.rs`
  (`BerryBush`, `Harvestable` at threshold, `ToHarvest` at 100% or on
  order, reset in `harvest_plants`).
- **Tree-like** (timed forestry work, spreads): mirror `trees.rs` (`ToCut`
  designation, `WorkProgress`-timed Cut job, `SpreadTimer`, nav-cost
  interplay via `nav::mark_trees` and the revert in `fell_trees`).
- **Standing-fuel / decorative** (no jobs, no yield): mirror `shrubs.rs`
  (grow + spread only, procedural world-start seeding via the `seed_algae`
  one-shot pattern instead of an LDtk entity).

Every species needs: mesh/material in `GameAssets` + consts in `config.rs`;
a growth system scaling by `cover::fertility_at` (substrate fertility +
grass-cover bonus — the shared model for bushes, crops, and trees); a spawn
fn with `GridCoords` + `Selectable` + `Name`; a yield poured through
`items::pour_yield` (unless the species yields nothing); and a `fuel()`
method feeding `cover::fuel_at` (see the `FUEL_*` consts).

## Pitfalls

- **Harvest item kind is inferred from the component**, not stored:
  `director::execute_jobs`' Harvest arm and `flora::harvest_plants` decide
  `Berries` vs `Wheat` by `Has<BerryBush>`. A new species yielding a third
  `ItemKind` must extend both spots (and guide 01 for the item itself).
  Consider moving the kind onto the plant component if this grows further.
- **Plant queries enumerate species explicitly.** Occupancy and target
  queries use `Or<(With<Tree>, With<BerryBush>, With<Crop>, With<Shrub>)>` —
  they appear in `zones::drag_zone_tool` (blueprint placement),
  `trees::spread_trees`, `shrubs::spread_shrubs`,
  `director::{assign_jobs, execute_jobs}`, and `debug_ui::update_jobs_tab`.
  Grep for `With<Crop>` and audit each `Or<(...)>` when adding a species, or
  it will be invisible to placement/occupancy checks.
- **`generate_jobs`' planted-set** (`crop_cells` ∪ `tree_cells`) is what
  stops Farming jobs on occupied cells — include your species there too.
- **Growth must be sim-gated**: register grow/spread systems in the
  `run_if(in_state(SimState::Running))` growth group in `game.rs`.
- **Fertility 0 means no growth**: `cover::fertility_at` is zero on wet
  cells (`WaterMap::is_wet`) and built cells — a plant spawned there never
  grows. `accepts_cell` + the zone eligibility predicate (which also
  rejects riverbeds via `has_bed`) are what keep planting on sane ground.
