# 008-colony — Phased spec: Walls & NavGrid, Pathfinding, Float Movement, AI Director

Status: **All 4 phases done — 2026-07-07.** Walls + NavGrid, A* pathfinding, float movement, and the AI Director are implemented and verified.

Post-spec milestones shipped since: pawn identity (names, per-pawn status, floating labels, right-side job panel), the stockpile/hauling loop (`JobKind::Haul`, `Carrying`, 80-per-stack cap, skill tiers S-D deciding who prefers which job kind), objectives + recreation strolling, the Allowance work-toggle panel, the ToHarvest queue-at-100% model, **terrain costs** (river with shallow/deep water, `NavGrid` per-cell costs feeding both A* step costs and walk speed, hover tooltip), and **farming** (`Terrain::Fertile` land driving growth speed for bushes and crops via a shared fertility multiplier; typed `ItemStack`/`ItemKind` so Berries and Wheat haul/stockpile/merge independently; `Crop`/`CropKind::Wheat` grown from a generalized `JobKind::Harvest` shared with bushes; `ZoneKind::Growing` with a per-zone `GrowOrder` toggle and a `JobKind::Farming` job planting on its cells, gated by a new Farming pawn allowance/skill), and the **day/night cycle** (`daynight.rs`: a `GameClock` resource — 180 s day / 60 s night, frozen by pause — driving faded ambient/directional lighting, a visible sun ball riding a sky arc with the shadow-casting directional light aimed from it, a moon ball carrying a cool `PointLight` at night whose glint reads on the glossy water materials, and a corner "Day N — HH:MM" clock chip; the sun/moon balls live on render layer 1 — invisible to the main camera for scale believability, lighting only — and a second offscreen camera renders their arc to a texture shown as a live sky-status widget above the clock, its clear color lerping night/day with a dusk tint; the world camera carries a `MainCamera` marker every cursor/viewport query must filter on), and **director robustness + pawn portraits** (haul jobs are only generated while `stockpile_capacity` can absorb that item kind — fixing a full-stockpile livelock where a pawn forever re-hauled the stack it just dropped at its feet; `Stuck` jobs now carry a retry timer instead of being ignored forever; yields spill outward — `items::find_drop_cell` searches Chebyshev rings up to `YIELD_DROP_RADIUS` for the nearest cell that can absorb (merge into started same-kind stacks first), shared by harvest placement, the pre-harvest guard, and the full-stockpile delivery fallback, so clogged surroundings drop further away instead of blocking; a top-center portrait bar shows one chip per pawn — click selects, click-again centers the camera; a backtick-toggled **debug window** (`debug_ui.rs`, tabbed via the `DebugTab` enum — Jobs tab lists the live job pool with priority/target/assignee/stuck-retry; jobs marked `Stuck` also shed their `AssignedTo` so the retry timer actually returns them to the pool), and the **living forest** (`Terrain::Grass` IntGrid 7 — fertility 0.6, buildable/growable for zones — as a blob in the NW corner; `trees.rs`: LDtk-seeded mature `Tree`s grow fertility-scaled and periodically seed 0-growth saplings on nearby free grass via `can_root` — grass-only rooting bounds the forest to its pocket; tree cells cost `TERRAIN_COST_TREE` 20 so pawns push through the woods at half speed), and the **substrate/cover split** (`cover.rs`: `Terrain` is substrate-only again — `Terrain::Grass` removed; vegetation is per-cell `Flora { kind: Grass|Algae, coverage 0..1 }` entities that regrow, creep to neighbors their kind roots on — grass slowly greens plain dirt, algae spreads along shallow water — and render as coverage-bucketed overlay tiles; effective fertility = substrate + `FLORA_FERTILITY_BONUS` x grass coverage, reproducing the old grass terrain's 0.6 exactly; trees root on grass coverage >= `TREE_ROOT_MIN_COVER`; the LDtk `GRASS` paint now just seeds full-coverage grass on dirt substrate; future wildfire/grazing consume coverage). Implement one phase at a time, in order; each phase is independently verifiable.

## Context

The colony sim currently has: an LDtk map rendered in 3D (data-only bevy_ecs_ldtk, bridge systems spawn the 3D view entities), selectable pawns and berry bushes, a bush growth model (harvestable at >= 80%, prorated yield), and a devmode Harvest button that fires a `HarvestCommand` message. This spec covers the next milestone group: pawns acting on the world themselves.

Decisions already made:

- Build **bottom-up**: walls/nav -> pathfinding -> movement -> director, so every phase lands testable.
- **Hand-rolled A\*** (learning project, no new dependency, full control for later terrain costs).
- **8-way movement with no corner cutting**.
- Movement is **float-based**: pawn positions are not snapped to cells. Speed is constant for now; terrain-type speed multipliers come later.

## Architecture overview

```
LDtk walls ──> NavGrid (walkable cells) ──> A* find_path ──> Path component
                                                                  │
Director: Harvestable bushes ──> Job entities ──> assign to idle pawn
                                                                  │
             move_along_path (float, Transform-authoritative) <───┘
                        │ on arrival next to bush
                        └──> HarvestCommand { bush, drop_at: pawn's tile }
```

Key shift in Phase 3: for pawns, `Transform` becomes the authoritative position (float, non-integer); `GridCoords` becomes a derived cache kept in sync for selection and the nav layer. Bushes/stacks stay grid-locked. All tuning stays in `config.rs`.

---

## Phase 1 — Walls + NavGrid

**Goal**: obstacles exist on the map and the game knows which cells are traversable.

- **Map**: add IntGrid value 2 "Wall" to the Ground layer (edit in the LDtk editor, or regenerate with the map generator script). Layout: an L-shaped wall between the pawn start (16,16) and bush C's area, plus a small 3-sided enclosure with a one-tile gap — enough to force real detours. Keep every bush reachable.
- **Code**: `WALL: i32 = 2` const; `WallCellBundle` registered with `register_ldtk_int_cell` (same pattern as `RedSandCellBundle` in `map.rs`); wall visuals = taller cuboid (`TILE_SIZE x WALL_HEIGHT x TILE_SIZE`, `WALL_HEIGHT ~ 0.8`, dark brown `WALL_COLOR`) spawned by an `Added<WallCell>` bridge system; `GameAssets` += `wall_mesh`, `wall_material`.
- **New module `nav.rs`**: `NavGrid` resource — `walkable: Vec<bool>` (`MAP_WIDTH` x `MAP_HEIGHT`), initialized all-walkable, cells flipped false by an `Added<WallCell>` system. `is_walkable(GridCoords) -> bool` returns false out of bounds. (bevy_ecs_ldtk spawns entities over a few frames after asset load; the grid fills in as they arrive — fine for the prototype since nothing paths before the map exists.)
- **Verify**: screenshot shows walls standing on the map; log clean; fmt/clippy clean.

## Phase 2 — Pathfinding (hand-rolled A*)

**Goal**: `nav::find_path(from, to) -> Option<Vec<GridCoords>>` that routes around walls.

- A* over `NavGrid` with **integer costs** (cardinal 10, diagonal 14 — avoids float `Ord` headaches in `BinaryHeap<Reverse<...>>`), octile heuristic `14*min(dx,dy) + 10*(max(dx,dy) - min(dx,dy))`.
- **8-way neighbors, no corner cutting**: a diagonal step is legal only if *both* flanking cardinal cells are walkable.
- Returns waypoints from start to goal (start cell excluded); `None` if unreachable.
- **Verify with unit tests** (`#[cfg(test)]` in `nav.rs`, run via `cargo test --example 008-colony`): straight path on open ground; detour around a wall; diagonal corner-cut rejected; unreachable (enclosed goal) -> `None`; out-of-bounds -> `None`. This is the first logic in the project that's cleanly unit-testable — use it.
- Optional in-app check: temp debug system pathing pawn -> bush C, drawn with `Gizmos` polyline, screenshot.

## Phase 3 — Float movement

**Goal**: pawns glide smoothly along paths; positions are floats, not cells.

- **New module `movement.rs`**:
  - `MoveOrder { goal: GridCoords }` — intent, inserted by dev tool or Director.
  - `Path { waypoints: Vec<Vec3>, next: usize }` — computed from `MoveOrder` via `nav::find_path`, waypoints at cell centers (`map::grid_to_world`, pawn's Y preserved). Unreachable -> log, drop order.
  - `PAWN_SPEED` const (tiles/sec, start ~3.0) — **constant for now**; terrain-type speed multipliers are a planned later extension (NavGrid grows a per-cell cost, A* is already integer-cost-ready).
  - `move_along_path`: advance `Transform.translation` toward the next waypoint by `PAWN_SPEED * dt`, looping through waypoints while the frame's distance budget lasts (no per-frame stalls at corners); snap + remove `Path` at the end.
  - `sync_grid_coords`: update the pawn's `GridCoords` from `world_to_grid(translation)` (already in `selection.rs`) whenever the cell changes — keeps click-selection and future nav queries honest.
- **Dev tool**: with a pawn selected, **right-click** a tile -> `MoveOrder`. Factor the cursor->ray->cell code out of `click_select` into a shared helper (`cursor_to_cell`).
- Debug: `Gizmos` polyline of the active path.
- **Verify**: temp debug system issues a `MoveOrder` across the wall; screenshots a second apart show the pawn at non-integer positions detouring around the wall and arriving; clicking it mid-walk still selects it.

## Phase 4 — AI Director

**Goal**: pawns find work themselves; priorities decide what gets done first.

- **New module `director.rs`**, jobs as entities:
  - `Job` component with a kind enum — only `Harvest { bush: Entity }` at first — plus `JobPriority(u32)` (lower = more urgent) and `AssignedTo(Option<Entity>)`.
  - **Generation**: a scan system spawns one Harvest job per `Harvestable` bush that doesn't already have one; job despawns when the bush stops being harvestable (or is gone).
  - **Assignment**: idle pawns (no `Path`, no `CurrentJob`) claim the best unassigned job: priority first, straight-line distance as tiebreak. Sets `CurrentJob(job)` on the pawn, `AssignedTo` on the job.
  - **Execution**: pawn with a Harvest job gets a `MoveOrder` to the nearest walkable neighbor of the bush; on arrival (no `Path`, adjacent), it writes `HarvestCommand` — **extended with `drop_at: Option<GridCoords>`** set to the pawn's tile ("stack appears beneath the pawn, 1 tile from the bush"). `None` keeps the neighbor-scan behavior so the devmode button still works.
  - Failure paths: bush unreachable or no longer harvestable on arrival -> drop job, back to idle. Idle pawns stand still (no wandering).
- **Map**: add a second Pawn instance so the Director visibly arbitrates between workers.
- **Verify**: long screenshot run — a pawn walks unprompted to the ripe bush around the walls, harvests (stack beneath it), goes idle, then walks to bush B when it crosses 80%; two pawns don't claim the same job.

---

## Cross-cutting

- New consts in `config.rs`: `WALL`, `WALL_HEIGHT`, `WALL_COLOR`, `PAWN_SPEED`, path-gizmo color.
- New modules per phase: `nav.rs` (1-2), `movement.rs` (3), `director.rs` (4); registered in `main.rs`/`game.rs` like the existing ones.
- Every phase ends with: fmt + clippy clean, screenshot verification, and this file's Status line updated.

---

## Resolved — connected shore tiles read as their bare base color at low snow%

Landed 2026-07-21 (first pass): snow/wet overlays now follow the beveled
shore instead of overhanging it as a square (`map::decal_shore_mask`), the
shore wedge (`map::build_shore_wedge_geometry`) and the water surface's
concave-corner triangle (`map::build_water_surface_geometry`) tint toward
snow/wet, bucket-aligned with the flat `SnowOverlay`/`WetOverlay` tiles
(`map::snow_wet_tint`), and the ice shader
(`assets/shaders/water_surface.wgsl`) reads as one smooth sheet instead of a
grid. A freeze-based tint on the wedge/water-triangle was tried and reverted
— it had no counterpart on flat land tiles (never track `snow::FreezeLevel`)
and double-applied on the water triangle (already shader-tinted via the
shared `water_material`); connected tiles now track snow/wet only, like their
flat neighbors.

Follow-up (also 2026-07-21): at a light dusting (~7% snow, still below
`SNOW_SUPPRESS_WET_MIN`), two connected-tile spots read closer to their bare
pre-shore color than to their neighbors, measured live via screenshot color
sampling — the "inland shallow water too red" / "outward shallow water too
white/blue icy" complaint. Both were older design choices getting more
scrutiny with the squares/grid/spike bugs gone, not regressions from the
first pass. Fixed by picking lever 1 from each candidate list below (the
other lever considered and rejected):

- **Convex wedge tip** (`dirt_tip` branch, bed cell wrapped by 3 land) read
  as `#824e3b` vs plain dirt `#86584b` — nearly identical (Δ 4/10/16). Its
  base was the raw `DIRT_COLOR_A`/`DIRT_COLOR_B` checker by original design
  ("land silhouette runs through the tip"), so at bucket 1
  (`SNOW_OVERLAY_ALPHAS[0] = 0.25`) the 25% snow mix barely moved it off
  dirt's own palette, even though the cell is tooltip-tagged "Shallow
  water." Fixed with a new unconditional floor blend,
  `config::SHORE_TIP_WATER_BLEND = 0.15`: the checker mixes toward
  `SHALLOW_WATER_COLOR` by that fraction *before* `snow_wet_tint`'s bucket
  logic runs, so a water-tagged tip never reads as pure dirt regardless of
  snow/wet state, while bucket-alignment with the flat tiles is unaffected
  on top of it. (Rejected: steepening bucket 1 for connected tiles only —
  would trade the "matches the flat tile bucket-for-bucket" invariant the
  first pass established for "visibly reacts to a first dusting," risking
  the transition-seam bugs rounds 2-3 fixed.)
- **Water surface shore fade** (`water_vertex_color`'s `WATER_SHORE_FADE`,
  pre-dates the first pass) read as `#5e5657` (diagonal/near-shore shallow
  water) vs `#4a464c` (open shallow water) — same cool palette, just lighter
  (94 vs 74 avg) from the alpha fade, unrelated to snow. Fixed by raising
  `config::WATER_SHORE_FADE` from `0.55` to `0.7`: the rim stays less
  transparent (less riverbed showing through) so it reads closer to open
  shallow water, everywhere, not only in snowy weather.

Verify by repro: Debug → Weather → Season Winter, Rain On, let snow sit
around 5-10%, hover-compare tooltip + screenshot-sampled hex color between a
connected tile and its flat neighbor. Also spot-check a normal-weather
open-water shore to confirm the raised `WATER_SHORE_FADE` floor still reads
as shallowing rather than a hard painted border.
