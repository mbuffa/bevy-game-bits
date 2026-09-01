//! Adding, removing, and resizing at runtime — the API a game reaches for
//! when the player picks something up, drops something, or upgrades their
//! pack.
//!
//! Two shapes, one implementation each. [`InventoryCommands`] is a
//! `SystemParam` for systems that hold a board [`Entity`]: `add` / `add_at` /
//! `remove` take effect immediately (and `add*` hand back the spawned
//! entity), while `resize` enqueues a [`ResizeInventory`] applied at
//! [`InventorySet::Commands`](super::InventorySet::Commands). The [`AddItem`]
//! and [`ResizeInventory`] messages do the same jobs for systems that would
//! rather not name a board type.
//!
//! Every path reports its outcome through [`InventoryAction`] — `Added`,
//! `AddRejected` (with the item data handed back, so a host can drop the loot
//! on the floor instead), `Removed`, `Evicted`, `Resized`.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::inventory::config::{InventoryConfig, InventoryTheme};
use crate::inventory::drag::{InventoryAction, InventoryDragState, InventorySelection};
use crate::inventory::grid::InventoryGrid;
use crate::inventory::items::{spawn_item, InventoryItem, InventorySlot};
use crate::inventory::ui::{rebuild_cells, InventoryBoard, InventoryCell};

/// Request to add an item to a board. `origin: None` means "first free
/// row-major spot, or reject". Fire it from any system; drained at
/// [`InventorySet::Commands`](super::InventorySet::Commands).
#[derive(Message, Clone, Debug)]
pub struct AddItem {
    pub board: Entity,
    pub item: InventoryItem,
    pub origin: Option<UVec2>,
}

/// Request to re-shape a board to `cols`x`rows`. Items that no longer fit in
/// place are relocated with a row-major first-fit; ones that fit nowhere are
/// evicted. Drained at [`InventorySet::Commands`](super::InventorySet::Commands).
#[derive(Message, Clone, Copy, Debug)]
pub struct ResizeInventory {
    pub board: Entity,
    pub cols: u32,
    pub rows: u32,
}

/// The `SystemParam` for runtime inventory changes. See the module docs.
#[derive(SystemParam)]
pub struct InventoryCommands<'w, 's> {
    commands: Commands<'w, 's>,
    boards: Query<
        'w,
        's,
        (
            &'static mut InventoryGrid,
            &'static InventoryConfig,
            &'static InventoryTheme,
            &'static mut InventorySelection,
        ),
        With<InventoryBoard>,
    >,
    child_of: Query<'w, 's, &'static ChildOf>,
    item_data: Query<'w, 's, &'static InventoryItem>,
    resize_writer: MessageWriter<'w, ResizeInventory>,
    actions: MessageWriter<'w, InventoryAction>,
}

impl InventoryCommands<'_, '_> {
    /// Add `item` to `board` in the first free row-major spot. Returns the
    /// spawned entity, or `None` (with an [`InventoryAction::AddRejected`]
    /// carrying `item` back) if it fits nowhere.
    pub fn add(&mut self, board: Entity, item: InventoryItem) -> Option<Entity> {
        self.place(board, item, None)
    }

    /// Add `item` to `board` with its top-left corner forced to `origin`.
    /// Returns `None` (and an [`InventoryAction::AddRejected`]) if that spot
    /// is off the board or occupied.
    pub fn add_at(&mut self, board: Entity, item: InventoryItem, origin: UVec2) -> Option<Entity> {
        self.place(board, item, Some(origin))
    }

    fn place(
        &mut self,
        board: Entity,
        item: InventoryItem,
        origin: Option<UVec2>,
    ) -> Option<Entity> {
        let Ok((mut grid, config, theme, _)) = self.boards.get_mut(board) else {
            return None;
        };
        place_item(
            &mut self.commands,
            board,
            &mut grid,
            config,
            theme,
            item,
            origin,
            &mut self.actions,
        )
    }

    /// Remove `item` from whichever board holds it, freeing its cells and
    /// despawning its node. Fires [`InventoryAction::Removed`] with the item
    /// data. A no-op if `item` isn't on a board.
    pub fn remove(&mut self, item: Entity) {
        let Ok(board) = self.child_of.get(item).map(ChildOf::parent) else {
            return;
        };
        let data = self.item_data.get(item).ok().cloned();
        if let Ok((mut grid, _, _, mut selection)) = self.boards.get_mut(board) {
            grid.clear(item);
            if selection.0 == Some(item) {
                selection.0 = None;
            }
        }
        self.commands.entity(item).despawn();
        if let Some(data) = data {
            self.actions
                .write(InventoryAction::Removed { board, item: data });
        }
    }

    /// Would a `size`-shaped item fit anywhere on `board` right now?
    pub fn has_room_for(&self, board: Entity, size: UVec2) -> bool {
        self.boards
            .get(board)
            .map(|(grid, ..)| grid.has_room_for(size))
            .unwrap_or(false)
    }

    /// How many empty cells `board` has. `0` for an unknown board.
    pub fn free_cells(&self, board: Entity) -> u32 {
        self.boards
            .get(board)
            .map(|(grid, ..)| grid.free_cells())
            .unwrap_or(0)
    }

    /// Re-shape `board` to `cols`x`rows`, applied at
    /// [`InventorySet::Commands`](super::InventorySet::Commands).
    pub fn resize(&mut self, board: Entity, cols: u32, rows: u32) {
        self.resize_writer
            .write(ResizeInventory { board, cols, rows });
    }
}

/// The one placement path both [`InventoryCommands`] and [`apply_add_requests`]
/// route through.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_item(
    commands: &mut Commands,
    board: Entity,
    grid: &mut InventoryGrid,
    config: &InventoryConfig,
    theme: &InventoryTheme,
    item: InventoryItem,
    origin: Option<UVec2>,
    actions: &mut MessageWriter<InventoryAction>,
) -> Option<Entity> {
    let origin = match origin.or_else(|| grid.first_fit(item.size)) {
        Some(origin) => origin,
        None => {
            actions.write(InventoryAction::AddRejected { board, item });
            return None;
        }
    };
    match spawn_item(commands, board, grid, config, theme, item.clone(), origin) {
        Some(entity) => {
            actions.write(InventoryAction::Added {
                board,
                item,
                origin,
            });
            Some(entity)
        }
        None => {
            actions.write(InventoryAction::AddRejected { board, item });
            None
        }
    }
}

/// Drains [`AddItem`].
pub fn apply_add_requests(
    mut commands: Commands,
    mut requests: MessageReader<AddItem>,
    mut boards: Query<
        (&mut InventoryGrid, &InventoryConfig, &InventoryTheme),
        With<InventoryBoard>,
    >,
    mut actions: MessageWriter<InventoryAction>,
) {
    for AddItem {
        board,
        item,
        origin,
    } in requests.read()
    {
        let Ok((mut grid, config, theme)) = boards.get_mut(*board) else {
            continue;
        };
        place_item(
            &mut commands,
            *board,
            &mut grid,
            config,
            theme,
            item.clone(),
            *origin,
            &mut actions,
        );
    }
}

/// Drains [`ResizeInventory`]: cancels any live drag on the board, re-shapes
/// the grid, rewrites `config.cols`/`rows`, rebuilds the cell tiles, resizes
/// the board node, and reports relocations/evictions.
#[allow(clippy::type_complexity)]
pub fn apply_resize_requests(
    mut commands: Commands,
    mut requests: MessageReader<ResizeInventory>,
    mut boards: Query<
        (
            &mut InventoryGrid,
            &mut InventoryConfig,
            &InventoryTheme,
            &mut InventoryDragState,
            &mut InventorySelection,
            &mut Node,
            &Children,
        ),
        With<InventoryBoard>,
    >,
    item_q: Query<(&InventorySlot, &InventoryItem)>,
    cell_q: Query<(), With<InventoryCell>>,
    mut actions: MessageWriter<InventoryAction>,
) {
    for &ResizeInventory { board, cols, rows } in requests.read() {
        let Ok((mut grid, mut config, theme, mut drag, mut selection, mut node, children)) =
            boards.get_mut(board)
        else {
            continue;
        };

        // A held item's true origin is its drag origin, not wherever the node
        // has drifted to. Fold that in, and end the drag.
        let held = match *drag {
            InventoryDragState::Held { item, origin, .. } => {
                *drag = InventoryDragState::Idle;
                Some((item, origin))
            }
            InventoryDragState::Idle => None,
        };

        let mut occupants: Vec<(Entity, UVec2, UVec2)> = Vec::new();
        let mut data: Vec<(Entity, InventoryItem)> = Vec::new();
        for child in children.iter() {
            let Ok((slot, item)) = item_q.get(child) else {
                continue;
            };
            let origin = match held {
                Some((held_item, held_origin)) if held_item == child => held_origin,
                _ => slot.0,
            };
            occupants.push((child, origin, item.size));
            data.push((child, item.clone()));
        }

        let outcome = grid.resize(cols, rows, &occupants);
        config.cols = cols;
        config.rows = rows;

        let size = config.board_size();
        node.width = Val::Px(size.x);
        node.height = Val::Px(size.y);

        let old_cells: Vec<Entity> = children
            .iter()
            .filter(|entity| cell_q.get(*entity).is_ok())
            .collect();
        rebuild_cells(&mut commands, board, &config, theme, old_cells);

        let data_of = |entity: Entity| {
            data.iter()
                .find(|(e, _)| *e == entity)
                .map(|(_, d)| d.clone())
        };

        for &evicted in &outcome.evicted {
            if selection.0 == Some(evicted) {
                selection.0 = None;
            }
            commands.entity(evicted).despawn();
            if let Some(item) = data_of(evicted) {
                actions.write(InventoryAction::Evicted { board, item });
            }
        }

        // Snap every surviving item's node back to its grid cell: relocated
        // ones moved, and a previously-held one needs its drifted node reset
        // even if it stayed put. Re-inserting the slot forces the `Changed`
        // filter in `ui::sync_item_position`.
        for &(item, origin, _) in &occupants {
            if outcome.evicted.contains(&item) {
                continue;
            }
            let final_origin = outcome
                .relocated
                .iter()
                .find(|(e, _)| *e == item)
                .map(|(_, o)| *o)
                .unwrap_or(origin);
            commands.entity(item).insert(InventorySlot(final_origin));
        }

        actions.write(InventoryAction::Resized { board, cols, rows });
    }
}
