# Add a new building (constructible)

Worked example: `BuildableKind::Door` — it exercises every seam a new
building touches (own ghost silhouette, own terrain behavior, own nav cost).
For a building bigger than one tile, `BuildableKind::SolarPanel` (2x2) is
the worked example instead — see "Does it need a multi-cell footprint?"
below.

The pipeline you're plugging into, end to end:

```
GeneralAction button → arms a zones::Tool → construction::update_build_ghost
shows a follow-cursor silhouette at the landing point → drag_zone_tool
spawns Blueprint ghosts (walkable) → director: singleton Supply job batches
wood to them → once supplied, sync_construction_collision solidifies the
footprint (minus any cell a pawn stands on) AND a per-blueprint timed Build
job starts → BuildCommand → construction::complete_builds despawns the
ghost, mutates ConstructionMap + NavGrid, spawns the real visual, auto-roofs
```

Supply and Build jobs, wood delivery tracking, the info-panel wood readout,
Cancel-with-refund, and the follow-cursor ghost all work off
`Blueprint`/`BuildableKind`/`Footprint` generically — you don't touch the
director or `update_build_ghost` unless your building needs a new
*material* or isn't 1x1 (see below).

## Checklist

1. Consts in `config.rs`: wood cost, build seconds, visual dimensions/colors
   (copy the `DOOR_*` block).
2. `construction::BuildableKind` variant + arms in `label()`, `wood_cost()`,
   `build_secs()`, `blocks_movement()`.
3. Ghost + built silhouette: meshes/materials on `GameAssets` (`game.rs`),
   a child-spawning arm in `construction::spawn_blueprint` (ghost material
   is the shared `blueprint_material` at `BLUEPRINT_ALPHA`).
4. Placement tool: `zones::Tool` variant + `label()` arm; for a 1x1 building
   handle it in `drag_zone_tool`'s `Tool::PlaceWall | Tool::PlaceDoor |
   Tool::PlaceTurbine` arm (extend the mapping to your `BuildableKind`); a
   multi-cell building needs its own arm (see `Tool::PlaceSolarPanel` below).
   `ui::GeneralAction` variant + `ALL` + `label()` + arm in
   `run_general_actions`. Also decide `Tool::is_linear()`: a 1x1 wall-run
   structure (wall/door/turbine/lightpost/battery/cooler) drags as a
   one-cell-thick line along the drag's dominant axis by default (Shift
   restores the old rectangle fill); return `false` there if yours is meant
   to fill an area instead.
5. Completion: arm in `construction::complete_builds` — decide terrain and
   nav (see below), spawn the built visual.
6. Terrain: if the building behaves unlike `Wall`/`Door`, add a
   `map::Terrain` variant (arms in `label()`, `base_cost()`, `fertility()`)
   and audit the terrain predicates listed under Pitfalls.
7. Footprint: 1x1 by default — only touch this if your building spans more
   than one cell (see below).
8. `cargo check`, then run: order the building (drag a big enough area if
   it's multi-cell — see below), watch the follow-cursor ghost, watch a
   pawn supply and raise it, cancel one mid-supply and check the wood
   refund, click each of its cells to confirm selection.

## Key decisions

### Terrain and nav on completion (`complete_builds`)

This function is the **only** place that mutates `ConstructionMap` + `NavGrid`
for the *finished* structure — keep it that way. Ground `TerrainMap` is never
touched by construction (a built cell keeps its real ground geometry
underneath). Your arm must set `ConstructionMap` + `NavGrid` coherently, and
add your kind to `BuildableKind::blocks_movement()` — the single seam both
`complete_builds` and the Build job's entombment guard read:

- Blocks movement (wall-like): `blocks_movement() == true`,
  `construction.set(cell, Some(BuildableKind::X))` +
  `nav.set_walkable(cell, false)`.
- Walkable but slowed (door-like): `blocks_movement() == false`,
  `construction.set(...)` + `nav.set_cost(cell, TERRAIN_COST_X)` — never
  below `TERRAIN_COST_DIRT`, the A* heuristic's floor.

Then `auto_roof_around(*grid, &terrain, &mut roofs)` runs; see Pitfalls for
whether your terrain counts as enclosing.

### Collision while under construction (`sync_construction_collision`)

A `Blueprint` ghost is walkable while wood is still being delivered. The
moment it's `is_supplied()` (ready for a Build job), `sync_construction_collision`
(`construction.rs`, run every frame at the head of the movement chain in
`game.rs`) marks its whole footprint non-walkable — **every** kind, doors
included, regardless of `blocks_movement()` — so pawns route around an
active build site instead of walking across it. It skips any footprint cell
a pawn currently stands on (never solidify under a pawn — `find_path`
requires a walkable start, so that would strand it) and re-solidifies that
cell the frame after the pawn leaves. It reverts a cell to walkable once the
`Blueprint` entity is gone (built or canceled), unless the finished structure
left there is itself `blocks_movement()` — that final state belongs to
`complete_builds`, not this system. New building kinds need no changes
here — it works off `Blueprint`/`Footprint` generically, the same as Supply
and Cancel-with-refund.

### Does it need its own `Terrain` variant?

Yes if pathfinding, enclosure, roofing, placement, or the tooltip should
treat it differently from existing terrain. `Terrain::Door` is the template:
it was added precisely because a door encloses like a wall but stays
walkable. A purely decorative building on plain ground may need no terrain
change at all — then it's closer to a plant/prop than a building.

### Does it need a multi-cell footprint?

No by default — `BuildableKind::footprint()` returns `&[(0, 0)]` for every
1x1 building, meaning "just the anchor cell". `BuildableKind::SolarPanel`
(`&[(0, 0), (1, 0), (0, 1), (1, 1)]`, a fixed 2x2 block) and
`BuildableKind::Bed(BedOrientation)` (`&[(0,0),(1,0)]` horizontal or
`&[(0,0),(0,1)]` vertical — the first shape with more than one possible
footprint, picked by a payload on the enum variant) are the templates for a
bigger one. Every downstream seam reads the footprint off the `Footprint`
component (`cells: Vec<GridCoords>`), carried by both the `Blueprint` ghost
and the built entity — nothing keys off `GridCoords` alone for occupancy
anymore, so a 1x1 building works unchanged (its `Footprint` is just its one
`GridCoords`). If your building needs more than one cell:

- `BuildableKind::footprint()`: add your offsets. If your building can have
  more than one shape (like the bed's orientation), a payload variant
  (`Bed(BedOrientation)`) is cheaper than a whole extra `BuildableKind` per
  shape — every other method (`label()`, `wood_cost()`, ...) can just wildcard
  the payload (`Bed(_) => ...`) since only `footprint()` actually cares.
- `can_place_footprint`/`footprint_cells`/`footprint_center_world`: already
  generic — `spawn_blueprint` centers the ghost/blueprint parent on the
  footprint automatically. `construction::footprint_size(kind)` gives the
  tiling step (see below) as the footprint's own bounding box, so it can never
  drift out of sync with the shape.
- Placement: `drag_zone_tool`'s `Tool::PlaceWall | PlaceDoor | PlaceTurbine`
  arm fills every drawn cell 1:1, which only makes sense for a 1x1
  footprint. A multi-cell building needs its own arm — see the shared
  `Tool::PlaceSolarPanel | Tool::PlaceBed(_)` arm and
  `zones::snapped_footprint_anchors(a, b, step)`, which tiles blocks starting
  at the drag's own start cell instead (steps by `step` — pass
  `construction::footprint_size(kind)` — across the rectangle, so a plain
  click always lands one full unit, anchored exactly where you clicked). This
  means two drags with different starting parities can produce overlapping
  anchors; `can_place_footprint` at commit time still silently skips an
  anchor overlapping bad terrain, a built structure, a plant, or an item
  stack — but if the only obstruction is another unbuilt blueprint, it gets
  erased and replaced (see `construction::despawn_blueprint_with_refund`
  below) instead of blocking the new placement. A building with more than one
  possible shape (the bed) derives `kind` from the armed `Tool`'s payload
  first, then calls the same shared anchor/placement logic — no
  shape-specific branch needed beyond that.
- `complete_builds`: write `ConstructionMap`/`NavGrid` for every cell in
  `footprint.cells`, not just the anchor `*grid`.
- Demolish: `zones.rs`'s `drag_zone_tool` looks a building up by
  `Footprint::contains`, not `GridCoords` equality, via one shared query
  (`Or<(With<Wall>, With<Door>, ..., With<Battery>, With<Bed>)>`) — your new
  building's marker component must join that `Or<...>` set or none of the
  three demolish tools will find it. That one query backs: the instant,
  no-refund `Tool::Demolish` devtool (dedups by entity so dragging over
  several of a building's cells in one stroke demolishes it once); the
  player-facing `Tool::DesignateDemolish`/`Tool::CancelDemolish` order (marks/
  unmarks `construction::ToDemolish`, which `director::JobKind::Demolish`
  turns into a real timed job that salvages `DEMOLISH_REFUND_FRACTION` of
  `wood_cost()` via `construction::do_demolitions` on completion).
- `execute_jobs`' Build arm (`director.rs`): the stand spot
  (`footprint_stand_spot`, generalizing `nearest_stand_spot`), the adjacency
  gate (`adjacent_to_footprint` — beside the footprint, never *on* any of its
  cells, so a pawn can never be asked to build the floor it's standing on),
  and the entombment check all test every footprint cell, not just the
  anchor. (A job that rests *inside* a footprint instead — the bed's Sleep
  job — needs a different helper, `footprint_rest_spot`; see
  [08-add-a-need.md](08-add-a-need.md).)
- Selection (`selection::click_select`): matches on `Footprint::contains`
  too, so any of the building's cells selects it, not only the anchor.
- The follow-cursor ghost (`construction::update_build_ghost`) needs to be
  drag-aware too, not just the commit arm: reuse the same
  `zones::snapped_footprint_anchors(drag_anchor, hovered, step)` call (via the
  shared `zones::DragAnchor` resource) that the commit and the
  ground-highlight preview use, and spawn one ghost per anchor. Driving the
  ghost off a different computation than the commit is exactly how the solar
  panel's ghost/placement mismatch happened — don't reintroduce that split
  for a new multi-cell building.

## Pitfalls

- **Pause-gating**: `complete_builds` and friends are deliberately ungated
  (`game.rs` comment) because `BuildCommand` is a double-buffered message
  written by the sim-gated director. Don't "fix" this by gating them, and
  follow the same pattern for any new completion message.
- **Entombment check keys off `blocks_movement()`**: `execute_jobs`' Build arm
  refuses to finish any kind that `blocks_movement()` while a pawn/stack/plant
  stands on one of its footprint cells (`blocked = blueprint.kind.blocks_movement()
  && footprint.cells.iter().any(...)`). Give your building the right
  `blocks_movement()` answer and this backstop covers it automatically — it
  should rarely fire in practice now that `sync_construction_collision`
  keeps the footprint solid (minus any occupying pawn) for the whole
  supplied/under-construction window, but it stays as the safety net for a
  pawn/stack/plant that lands there in the one frame the cell reopens.
- **Enclosure + auto-roof**: `auto_roof_around` and `enclosed_interior`
  treat `Terrain::Wall | Terrain::Door` as boundary. Decide whether your
  terrain joins that predicate (it appears twice in `auto_roof_around`: the
  flood-fill boundary and the perimeter-roof stamping). Also check
  `is_roofable` / `wants_roof` (riverbeds — `WaterMap::has_bed`, drained or
  not — are the only non-roofable ground today).
- **Autotiling is wall-run specific**: `sync_wall_connections` picks among 16
  `connected_wall_mesh` variants for `Wall`-marked entities and rotates
  `Door`-marked ones via `wall_connect_mask` / `door_is_vertical`, driven by
  the `Terrain::Wall | Terrain::Door` connectivity predicate. A freestanding
  building skips all of this; a run-forming one needs a decision in that
  predicate too.
- **Placement rules**: `can_place_blueprint` allows walkable ground with no
  authored riverbed (`!WaterMap::has_bed` — live wetness would let you build
  in a drained bed and flood it later), unoccupied by
  blueprints/plants/stacks (pawns are fine — they move; the entombment check
  above is the backstop). Extend it if your building may sit elsewhere
  (e.g. a bridge on water would need its own predicate and a nav story).
- **Cancel refunds**: `cancel_blueprints` and drag-placement's erase-on-
  overlap (dragging a new blueprint over an existing unbuilt one replaces
  it, refunding the old one — see `drag_zone_tool`'s `Tool::PlaceWall |
  PlaceDoor | PlaceTurbine` and `Tool::PlaceSolarPanel` arms) both share
  `construction::despawn_blueprint_with_refund`, which pours
  `blueprint.delivered` wood back via `items::pour_yield`. Works for any
  kind automatically — but if you add non-wood costs, only that one
  function needs matching changes.
- **Selection**: the blueprint parent carries `Selectable` + `GridCoords` +
  `Footprint`; the built entity must too (plus the `Wall`/`Door`-style
  marker component if other systems need to find it). Both `spawn_blueprint`
  and your `complete_builds` arm spawn `Name::new(...)` — the info panel
  shows it.
- **A wall built from LDtk vs at runtime**: `construction::spawn_walls_from_ldtk`
  (the `Wall` entity bridge — walls are placed data on the Entities layer,
  not IntGrid terrain paint) and `complete_builds` spawn the same bundle on
  purpose. If you add an LDtk-placeable version of your building, keep the
  two spawn sites identical (or extract a helper as `spawn_door_parts` does
  for doors).
