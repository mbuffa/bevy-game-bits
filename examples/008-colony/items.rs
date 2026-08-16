use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::game::GameAssets;
use crate::map;
use crate::selection::Selectable;

/// What an `ItemStack` holds. New resources (produce, ore, ...) add a
/// variant here plus arms in `label`/`material`/`category`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ItemKind {
    Berries,
    Wheat,
    Wood,
}

/// Summary bucket on the stock panel: kinds roll up into a few abstract
/// rows (Anno-style) so the HUD stays compact as resources multiply.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StockCategory {
    RawFood,
    Materials,
}

impl StockCategory {
    /// Panel row order.
    pub const ALL: [StockCategory; 2] = [StockCategory::RawFood, StockCategory::Materials];

    pub fn label(&self) -> &'static str {
        match self {
            StockCategory::RawFood => "Raw Food",
            StockCategory::Materials => "Materials",
        }
    }
}

impl ItemKind {
    /// Detail-row order under each stock-panel category.
    pub const ALL: [ItemKind; 3] = [ItemKind::Berries, ItemKind::Wheat, ItemKind::Wood];

    pub fn label(&self) -> &'static str {
        match self {
            ItemKind::Berries => "Berries",
            ItemKind::Wheat => "Wheat",
            ItemKind::Wood => "Wood",
        }
    }

    /// Stock-panel bucket; exhaustive so a new kind must pick one.
    pub fn category(&self) -> StockCategory {
        match self {
            ItemKind::Berries | ItemKind::Wheat => StockCategory::RawFood,
            ItemKind::Wood => StockCategory::Materials,
        }
    }

    fn material(&self, assets: &GameAssets) -> Handle<StandardMaterial> {
        match self {
            ItemKind::Berries => assets.berry_stack_material.clone(),
            ItemKind::Wheat => assets.wheat_stack_material.clone(),
            ItemKind::Wood => assets.wood_stack_material.clone(),
        }
    }
}

/// A stack of one resource kind lying on the ground, dropped by a harvest or
/// a haul. Holds at most `STACK_CAP`; stacks of different kinds never merge
/// (see the Haul/Merge/deliver systems in `director.rs`).
#[derive(Component)]
pub struct ItemStack {
    pub kind: ItemKind,
    pub amount: u32,
}

impl ItemStack {
    /// How many more of its kind fit in this stack.
    pub fn space(&self) -> u32 {
        STACK_CAP.saturating_sub(self.amount)
    }
}

/// Stockpiled total per kind, from every stack as `(kind, amount,
/// in_stockpile)`. Loose ground piles and carried loads are excluded on
/// purpose: the stock panel only counts what's been hauled home.
pub fn stock_totals(
    stacks: impl IntoIterator<Item = (ItemKind, u32, bool)>,
) -> HashMap<ItemKind, u32> {
    let mut totals = HashMap::new();
    for (kind, amount, in_stockpile) in stacks {
        if in_stockpile {
            *totals.entry(kind).or_insert(0) += amount;
        }
    }
    totals
}

/// Nearest cell around `origin` that can absorb items of `kind`, scanning
/// outward ring by ring (Chebyshev distance) up to `radius`: clogged
/// surroundings spill further away instead of blocking the drop. Within a
/// ring, pouring into a started same-kind stack beats opening a fresh cell;
/// a fresh cell must be standable and (beyond the origin itself, where the
/// dropping pawn stands) free of other occupants. `stacks` is every stack as
/// `(kind, cell, amount)`; `occupied` every `Selectable`'s cell.
pub fn find_drop_cell(
    kind: ItemKind,
    origin: GridCoords,
    radius: i32,
    stacks: &[(ItemKind, GridCoords, u32)],
    occupied: &HashSet<GridCoords>,
    standable: impl Fn(GridCoords) -> bool,
) -> Option<GridCoords> {
    for d in 0..=radius {
        let ring: Vec<GridCoords> = (-d..=d)
            .flat_map(|dx| (-d..=d).map(move |dy| (dx, dy)))
            .filter(|(dx, dy)| dx.abs().max(dy.abs()) == d)
            .map(|(dx, dy)| GridCoords::new(origin.x + dx, origin.y + dy))
            .filter(|cell| (0..MAP_WIDTH).contains(&cell.x) && (0..MAP_HEIGHT).contains(&cell.y))
            .collect();
        let merge = ring.iter().find(|cell| {
            matches!(
                stacks.iter().find(|(_, grid, _)| grid == *cell),
                Some((stack_kind, _, amount)) if *stack_kind == kind && *amount < STACK_CAP
            )
        });
        if let Some(cell) = merge {
            return Some(*cell);
        }
        let fresh = ring.iter().find(|cell| {
            stacks.iter().all(|(_, grid, _)| grid != *cell)
                && standable(**cell)
                && (d == 0 || !occupied.contains(cell))
        });
        if let Some(cell) = fresh {
            return Some(*cell);
        }
    }
    None
}

/// Pour `amount` of `kind` onto the ground around `origin`: nearest usable
/// cells first (`find_drop_cell`), topping up started same-kind stacks
/// before spawning fresh ones, spilling outward as stacks hit the cap.
/// Anything that finds no cell within `YIELD_DROP_RADIUS` is lost (with a
/// warning) — only a truly walled-in origin gets there. Shared by plant
/// harvesting and tree felling.
#[allow(clippy::too_many_arguments)]
pub fn pour_yield(
    commands: &mut Commands,
    assets: &GameAssets,
    kind: ItemKind,
    amount: u32,
    origin: GridCoords,
    stacks: &mut Query<(&mut ItemStack, &GridCoords)>,
    occupied: &HashSet<GridCoords>,
    standable: impl Fn(GridCoords) -> bool,
) {
    // The local mirror of the stack list tracks this loop's own pours and
    // spawns, which the query can't see yet (spawns are deferred commands).
    let mut stack_list: Vec<(ItemKind, GridCoords, u32)> = stacks
        .iter()
        .map(|(stack, stack_grid)| (stack.kind, *stack_grid, stack.amount))
        .collect();
    let mut remaining = amount;
    while remaining > 0 {
        let cell = find_drop_cell(
            kind,
            origin,
            YIELD_DROP_RADIUS,
            &stack_list,
            occupied,
            &standable,
        );
        let Some(cell) = cell else {
            warn!(
                "pour: nowhere within {YIELD_DROP_RADIUS} tiles of {origin:?} to drop, \
                 {remaining} {} lost",
                kind.label()
            );
            break;
        };
        if let Some((mut stack, _)) = stacks
            .iter_mut()
            .find(|(stack, stack_grid)| **stack_grid == cell && stack.kind == kind)
        {
            let added = stack.space().min(remaining);
            stack.amount += added;
            remaining -= added;
            if let Some(entry) = stack_list
                .iter_mut()
                .find(|(entry_kind, grid, _)| *grid == cell && *entry_kind == kind)
            {
                entry.2 += added;
            }
        } else {
            let dropped = remaining.min(STACK_CAP);
            spawn_item_stack(commands, assets, kind, cell, dropped);
            remaining -= dropped;
            stack_list.push((kind, cell, dropped));
        }
    }
}

pub fn spawn_item_stack(
    commands: &mut Commands,
    assets: &GameAssets,
    kind: ItemKind,
    grid: GridCoords,
    amount: u32,
) {
    debug_assert!(amount <= STACK_CAP);
    commands.spawn((
        Mesh3d(assets.item_stack_mesh.clone()),
        MeshMaterial3d(kind.material(assets)),
        Transform::from_translation(map::grid_to_world(&grid) + Vec3::Y * ITEM_STACK_HEIGHT / 2.0),
        Name::new(kind.label()),
        ItemStack { kind, amount },
        grid,
        Selectable,
    ));
}

/// Devtool aid: drop a couple of wood stacks near the stockpile on startup so
/// construction can be tested without felling trees first. Gated like
/// `construction::spawn_walls_from_ldtk`'s siblings: LDtk cells arrive a few
/// frames after startup, so this waits for `DirtCell`s to exist rather than
/// running at `Startup`. Cells verified against the raw LDtk data: plain
/// Dirt, unzoned, and clear of other entities.
pub fn spawn_starter_wood(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut done: Local<bool>,
    ready: Query<(), With<map::DirtCell>>,
) {
    if *done || ready.is_empty() {
        return;
    }
    *done = true;
    for cell in [GridCoords::new(12, 20), GridCoords::new(13, 20)] {
        spawn_item_stack(&mut commands, &assets, ItemKind::Wood, cell, 50);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: i32 = 8;

    fn grid(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    #[test]
    fn stock_totals_count_only_stockpiled_stacks() {
        let totals = stock_totals([
            (ItemKind::Berries, 30, true),
            (ItemKind::Berries, 50, true),
            (ItemKind::Wheat, 20, false),
            (ItemKind::Wood, 10, true),
        ]);
        assert_eq!(totals.get(&ItemKind::Berries), Some(&80));
        assert_eq!(totals.get(&ItemKind::Wheat), None);
        assert_eq!(totals.get(&ItemKind::Wood), Some(&10));
    }

    #[test]
    fn free_origin_wins_immediately() {
        let cell = find_drop_cell(
            ItemKind::Berries,
            grid(5, 5),
            R,
            &[],
            &HashSet::new(),
            |_| true,
        );
        assert_eq!(cell, Some(grid(5, 5)));
    }

    #[test]
    fn occupied_origin_is_fine_but_occupied_neighbors_are_not() {
        // The dropping pawn stands on the origin: still a valid drop.
        let occupied: HashSet<GridCoords> = [grid(5, 5)].into();
        let cell = find_drop_cell(ItemKind::Berries, grid(5, 5), R, &[], &occupied, |_| true);
        assert_eq!(cell, Some(grid(5, 5)));
        // A stack on the origin pushes the drop outward, and occupied
        // ring-1 cells are skipped in favor of a free ring-1 cell.
        let stacks = [(ItemKind::Wheat, grid(5, 5), 10)];
        let occupied: HashSet<GridCoords> = [grid(5, 5), grid(4, 4), grid(4, 5)].into();
        let cell = find_drop_cell(ItemKind::Berries, grid(5, 5), R, &stacks, &occupied, |_| {
            true
        })
        .unwrap();
        assert_eq!((cell.x - 5).abs().max((cell.y - 5).abs()), 1);
        assert!(!occupied.contains(&cell));
    }

    #[test]
    fn merge_beats_fresh_in_the_same_ring() {
        // Origin blocked by a full stack; ring 1 has both free cells and a
        // half-full same-kind stack — the stack wins.
        let stacks = [
            (ItemKind::Berries, grid(5, 5), STACK_CAP),
            (ItemKind::Berries, grid(6, 6), 10),
        ];
        let cell = find_drop_cell(
            ItemKind::Berries,
            grid(5, 5),
            R,
            &stacks,
            &HashSet::new(),
            |_| true,
        );
        assert_eq!(cell, Some(grid(6, 6)));
    }

    #[test]
    fn walled_in_spills_to_farther_rings() {
        // Origin + all 8 ring-1 cells hold full stacks: land on ring 2.
        let mut stacks = vec![(ItemKind::Berries, grid(5, 5), STACK_CAP)];
        for dx in -1..=1 {
            for dy in -1..=1 {
                if (dx, dy) != (0, 0) {
                    stacks.push((ItemKind::Berries, grid(5 + dx, 5 + dy), STACK_CAP));
                }
            }
        }
        let cell = find_drop_cell(
            ItemKind::Berries,
            grid(5, 5),
            R,
            &stacks,
            &HashSet::new(),
            |_| true,
        )
        .unwrap();
        assert_eq!((cell.x - 5).abs().max((cell.y - 5).abs()), 2);
    }

    #[test]
    fn foreign_kind_stacks_never_merge() {
        let stacks = [
            (ItemKind::Wheat, grid(5, 5), 1),
            (ItemKind::Wheat, grid(6, 5), 1),
        ];
        // Everything unstandable except the wheat cells: berries can't land.
        let wheat_cells: HashSet<GridCoords> = [grid(5, 5), grid(6, 5)].into();
        let cell = find_drop_cell(
            ItemKind::Berries,
            grid(5, 5),
            R,
            &stacks,
            &HashSet::new(),
            |c| wheat_cells.contains(&c),
        );
        assert_eq!(cell, None);
    }

    #[test]
    fn exhausted_radius_gives_none_and_stays_in_bounds() {
        // Nothing standable at all: None, even near the map corner where
        // most ring cells are out of bounds.
        let cell = find_drop_cell(
            ItemKind::Berries,
            grid(0, 0),
            R,
            &[],
            &HashSet::new(),
            |_| false,
        );
        assert_eq!(cell, None);
    }
}
