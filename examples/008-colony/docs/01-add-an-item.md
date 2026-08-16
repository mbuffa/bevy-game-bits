# Add a new item / resource kind

Worked example: `ItemKind::Wood` (added for forestry + construction).
Everything downstream of the enum — hauling, stockpiling, merging, stack
caps, carry visuals, the info panel — is kind-generic, so a new resource is
mostly one enum variant plus a material.

## Checklist

1. Color const in `config.rs` (next to `WOOD_STACK_COLOR`).
2. Material handle on `GameAssets` + creation in `setup_assets` (`game.rs`).
3. Variant on `items::ItemKind` + arms in `label()`, `material()`, and
   `category()` (its stock-panel bucket) + entry in `ItemKind::ALL`.
4. Arm in `director::sync_carry_visual` (the cube riding on a carrying pawn).
5. A producer: something that calls `items::pour_yield` with your kind.
6. `cargo check --example 008-colony` — exhaustive matches catch stragglers.

## Steps

### 1–2. Color and material

Add `pub const ORE_STACK_COLOR: Color = …;` in `config.rs` beside the other
`*_STACK_COLOR`s, an `ore_stack_material: Handle<StandardMaterial>` field on
`GameAssets`, and its `materials.add(...)` in `setup_assets`, copying the
`wood_stack_material` block. All item stacks share `item_stack_mesh`; only
the material differs per kind.

### 3. The enum (`items.rs`)

```rust
pub enum ItemKind { Berries, Wheat, Wood, Ore }
```

Add arms in `ItemKind::label()`, `ItemKind::material()`, and
`ItemKind::category()` — the last picks which `StockCategory` row of the
stock panel the kind rolls up into (Raw Food, Materials, ...; add a new
category variant + `StockCategory::ALL` entry if none fits). All matches are
exhaustive — the compiler will insist. Also append the kind to
`ItemKind::ALL`, which orders the panel's detail rows.

### 4. Carry visual (`director.rs`)

`sync_carry_visual` matches `carrying.kind` to pick the material of the cube
shown on a hauling pawn. Exhaustive match; add your arm.

### 5. Produce it

Items enter the world through **`items::pour_yield`** — the shared drop
helper used by `flora::harvest_plants` (berries/wheat), `trees::fell_trees`
(wood), and `construction::cancel_blueprints` (wood refunds). It tops up
started same-kind stacks first, spills outward ring by ring
(`find_drop_cell`, Chebyshev distance up to `YIELD_DROP_RADIUS`), and spawns
fresh stacks via `spawn_item_stack`, capped at `STACK_CAP` (80) each.

Call it from whatever produces your resource (a new harvest source, a mining
job, …) — don't spawn `ItemStack` entities directly, or you'll stack two
piles on one cell and skip the merge-first logic. Typical call site shape
(from `fell_trees`):

```rust
let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
items::pour_yield(&mut commands, &assets, ItemKind::Ore, amount, origin,
    &mut stacks, &occupied_cells, |cell| nav.is_walkable(cell));
```

### What you get for free

- **Hauling**: `director::generate_jobs` spawns a Haul job for any stack
  outside the stockpile, `deliver_carried` walks it home and deposits
  merge-first. No changes needed.
- **Stockpile capacity gating**: hauls are only generated while
  `stockpile_capacity(kind) > 0` — prevents the full-stockpile livelock.
- **Merging**: the singleton Merge job consolidates small stockpiled stacks.
- **Selection panel**: `ui::update_panel` prints `"{label}: {amount} / 80"`
  for any `ItemStack`.
- **Stock panel**: the left-center HUD (`ui::sync_stock_panel`) sums
  stockpiled stacks under the kind's `category()` row, with a per-kind
  detail row when expanded.

## Pitfalls

- **Different kinds never merge.** A stockpile cell holding another kind
  counts as full for yours (`stockpile_capacity`, `deliver_carried`). One
  small stockpile + many kinds = spillover on the ground around it.
- **Harvest item kinds are currently inferred, not stored.** The plant
  harvest path decides `Berries` vs `Wheat` by `Has<BerryBush>`
  (`director::execute_jobs` Harvest arm, and `flora::harvest_plants`). If
  your new item comes from a new *plant*, that inference needs restructuring
  — see guide 04.
- **`pour_yield` can lose surplus** (with a `warn!`) if nothing within
  `YIELD_DROP_RADIUS` can take it. Only a truly walled-in origin triggers it;
  the director pre-checks `find_drop_cell` before harvesting/cutting and
  marks the job `Stuck` instead.
- If the new item is a **construction material**, note that the Supply/Build
  economy is hardwired to `ItemKind::Wood` (`generate_jobs`' supply block,
  `execute_jobs`' Supply arm, `Blueprint`/`wood_cost`). A second material is
  a real construction-system change, not an item change (guide 02).
