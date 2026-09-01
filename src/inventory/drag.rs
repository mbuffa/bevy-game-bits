//! Press-to-select, hold-to-drag, release-to-drop. A press that never moves
//! past [`InventoryConfig::drag_threshold_px`] is a plain click: it selects
//! the item (so `ui::sync_description` shows it) without ever entering the
//! held state.
//!
//! Every system here is per-board — a `Query` over `With<InventoryBoard>` that
//! skips any board whose [`InventoryWindow`] is closed. The [`InventoryDragState`]
//! stays on the board the press landed on (that's where the revert
//! information lives), but its `target` field tracks whichever board the
//! cursor is currently over, so a release can drop the item onto a *different*
//! board — see [`track_drag_target`] and [`InventoryAction::Transferred`].
//! [`InventoryAccess`] is the host's veto on both dragging and receiving.
//!
//! The held item is never removed from that board's [`InventoryGrid`]
//! mid-drag — it stays placed at its old cells the whole time, and every
//! occupancy check passes `ignoring: Some(item)` so it doesn't block its own
//! drop. A crash or an early return mid-drag can never leave a hole in the
//! grid.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::inventory::config::{InventoryConfig, InventoryTheme};
use crate::inventory::grid::{self, InventoryGrid};
use crate::inventory::items::{InventoryItem, InventorySlot};
use crate::inventory::ui::{place_node, InventoryBoard, Z_ITEM_DRAGGED, Z_ITEM_IDLE};
use crate::inventory::{InventoryAccess, InventoryWindow};

/// The item shown in one board's description panel — set on every press,
/// dragged or not, which is what makes a plain click "select" for free. A
/// `Component` on the board entity.
#[derive(Component, Default)]
pub struct InventorySelection(pub Option<Entity>);

/// Cursor position relative to one board's top-left corner, refreshed every
/// frame by [`track_cursor`]. A `Component` on the board entity.
#[derive(Component, Default)]
pub struct InventoryCursor {
    /// Logical pixels. Extrapolated beyond the board (including negative)
    /// once a drag carries the cursor past an edge —
    /// [`grid::target_origin`] and [`InventoryGrid::fits`] are what reject
    /// that, not this.
    pub px: Option<Vec2>,
    /// True only while the raw cursor sits within this board's own rect.
    pub over_board: bool,
}

/// One board's drag state. A `Component` on the board entity.
#[derive(Component, Default)]
pub enum InventoryDragState {
    #[default]
    Idle,
    Held {
        item: Entity,
        /// Where to revert to if the drop is rejected, or if the window
        /// closes mid-drag. A cell on the **source** board.
        origin: UVec2,
        /// Which of the item's own cells was grabbed (0..size on each axis).
        grab_offset: UVec2,
        /// Cursor offset from the item's top-left corner, in logical pixels
        /// — kept constant for the whole drag so the grabbed point never
        /// drifts out from under the cursor.
        grab_px: Vec2,
        /// Cursor position at the press, for the click-vs-drag threshold.
        press_px: Vec2,
        moved: bool,
        /// The board a release would drop onto, re-resolved every frame by
        /// [`track_drag_target`]. The source board itself for an ordinary
        /// in-board drag, and whenever the cursor is over no eligible board —
        /// which is what makes "drag past the edge, footprint still fits, drop
        /// lands" keep working.
        target: Entity,
    },
}

impl InventoryDragState {
    pub fn held_item(&self) -> Option<Entity> {
        match self {
            InventoryDragState::Held { item, .. } => Some(*item),
            InventoryDragState::Idle => None,
        }
    }

    /// The board a release would currently drop onto — the source board for a
    /// plain in-board drag. `None` when nothing is held.
    pub fn target(&self) -> Option<Entity> {
        match self {
            InventoryDragState::Held { target, .. } => Some(*target),
            InventoryDragState::Idle => None,
        }
    }

    /// The footprint the held item would land in if released right now —
    /// `None` before a press has turned into an actual drag. `cursor`, `grid`,
    /// and `config` are the **target** board's (the one being dropped onto),
    /// which is not necessarily the board this state lives on.
    pub fn preview(
        &self,
        cursor: &InventoryCursor,
        grid: &InventoryGrid,
        config: &InventoryConfig,
        items: &Query<&InventoryItem>,
    ) -> Option<InventoryPreview> {
        let InventoryDragState::Held {
            item,
            grab_offset,
            moved,
            ..
        } = self
        else {
            return None;
        };
        if !moved {
            return None;
        }
        let hovered = grid::hovered_cell(cursor.px?, config.cell_px);
        let size = items.get(*item).ok()?.size;
        let origin = grid::target_origin(hovered, *grab_offset);
        let fits = grid.fits(origin, size, Some(*item));
        Some(InventoryPreview { fits, origin, size })
    }
}

/// A drag's would-be landing spot. `origin` is signed and `size` may overhang
/// the board — `fits` is already `false` in that case, and
/// [`covers`](Self::covers) simply reports no on-board cell for the part
/// that's outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventoryPreview {
    pub fits: bool,
    pub origin: IVec2,
    pub size: UVec2,
}

impl InventoryPreview {
    pub fn covers(&self, cell: UVec2) -> bool {
        let cell = cell.as_ivec2();
        cell.x >= self.origin.x
            && cell.x < self.origin.x + self.size.x as i32
            && cell.y >= self.origin.y
            && cell.y < self.origin.y + self.size.y as i32
    }
}

/// What just happened on a board. This module plays no sound and shows no
/// animation of its own — the same way `vehicle::impact` leaves what *breaks*
/// to the game, this leaves what a pick-up *sounds like* to the game. Every
/// variant carries the `board` it happened on.
///
/// The `Add*` / `Removed` / `Evicted` variants carry the [`InventoryItem`]
/// *data*, not its entity — an added-then-rejected item has no entity, and a
/// removed one's entity is already despawned, so a host that wants to drop
/// the loot on the floor instead still has everything it needs.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum InventoryAction {
    Selected {
        board: Entity,
        item: Entity,
    },
    PickedUp {
        board: Entity,
        item: Entity,
    },
    Dropped {
        board: Entity,
        item: Entity,
        from: UVec2,
        to: UVec2,
    },
    /// An item dragged off one board and dropped onto another. `from` is a
    /// cell on `from_board`, `to` a cell on `to_board`; the item's `ChildOf`,
    /// grid entry, and (if it was selected) the description panel have all
    /// moved to `to_board`.
    Transferred {
        from_board: Entity,
        to_board: Entity,
        item: Entity,
        from: UVec2,
        to: UVec2,
    },
    Rejected {
        board: Entity,
        item: Entity,
        origin: UVec2,
    },
    Added {
        board: Entity,
        item: InventoryItem,
        origin: UVec2,
    },
    AddRejected {
        board: Entity,
        item: InventoryItem,
    },
    Removed {
        board: Entity,
        item: InventoryItem,
    },
    Evicted {
        board: Entity,
        item: InventoryItem,
    },
    Resized {
        board: Entity,
        cols: u32,
        rows: u32,
    },
}

/// `RelativeCursorPosition` -> each board's [`InventoryCursor`].
/// `ui_focus_system` (part of `DefaultPlugins`) writes it before `Update`
/// runs, so this always reads the current frame's cursor. A closed board's
/// cursor is forced empty.
pub fn track_cursor(
    mut boards: Query<
        (
            &RelativeCursorPosition,
            &InventoryConfig,
            &mut InventoryCursor,
            &InventoryWindow,
        ),
        With<InventoryBoard>,
    >,
) {
    for (rel, config, mut cursor, window) in &mut boards {
        if !window.open {
            if cursor.px.is_some() || cursor.over_board {
                cursor.px = None;
                cursor.over_board = false;
            }
            continue;
        }
        let board_size = config.board_size();
        cursor.px = rel
            .normalized
            .map(|normalized| (normalized + Vec2::splat(0.5)) * board_size);
        cursor.over_board = rel.cursor_over;
    }
}

/// Re-points every live drag's `target` at whichever board the cursor is over
/// this frame — the source board itself when that's where the cursor is, or
/// when it's over nothing, or over a board the transfer policy forbids.
///
/// The policy lives **only here**: a cross-board target is allowed only when
/// the hovered board and the source board both have
/// [`InventoryAccess::transfers`]. [`end_drag`] then trusts `target` without
/// re-checking, so the two can't drift.
pub fn track_drag_target(
    mut boards: Query<
        (
            Entity,
            &InventoryCursor,
            &InventoryWindow,
            &InventoryAccess,
            &mut InventoryDragState,
        ),
        With<InventoryBoard>,
    >,
) {
    // The board under the cursor, and whether it will exchange with others.
    let hovered: Option<(Entity, bool)> = boards
        .iter()
        .find(|(_, cursor, window, access, _)| {
            window.open && access.interactive && cursor.over_board
        })
        .map(|(entity, _, _, access, _)| (entity, access.transfers));

    for (board, _, _, access, mut drag) in &mut boards {
        let InventoryDragState::Held { target, .. } = &mut *drag else {
            continue;
        };
        let wanted = match hovered {
            Some((hovered, _)) if hovered == board => board,
            Some((hovered, hovered_transfers)) if hovered_transfers && access.transfers => hovered,
            // Over nothing, or over a board one of the two ends won't exchange
            // with: the drop stays with the source board.
            _ => board,
        };
        if *target != wanted {
            *target = wanted;
        }
    }
}

/// A press over a board: selects whatever's under the cursor (or clears that
/// board's selection over an empty cell) and, if it landed on an item and the
/// board is [`interactive`](InventoryAccess::interactive), arms a potential
/// drag. The press is consumed by the first open board the cursor is over —
/// even a non-interactive one, so a board stacked behind it doesn't also
/// react; overlapping boards are the host's problem.
#[allow(clippy::type_complexity)]
pub fn begin_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    mut boards: Query<
        (
            Entity,
            &InventoryCursor,
            &InventoryGrid,
            &InventoryConfig,
            &InventoryAccess,
            &mut InventoryDragState,
            &mut InventorySelection,
            &InventoryWindow,
        ),
        With<InventoryBoard>,
    >,
    slots: Query<&InventorySlot>,
    mut actions: MessageWriter<InventoryAction>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    for (board, cursor, grid, config, access, mut drag, mut selection, window) in &mut boards {
        if !window.open || !matches!(*drag, InventoryDragState::Idle) || !cursor.over_board {
            continue;
        }
        // This board owns the press from here on — every path returns.
        let Some(px) = cursor.px else { return };
        let cell = grid::hovered_cell(px, config.cell_px);
        if cell.x < 0 || cell.y < 0 {
            selection.0 = None;
            return;
        }
        let cell = cell.as_uvec2();
        let Some(item) = grid.at(cell) else {
            selection.0 = None;
            return;
        };
        let Ok(InventorySlot(origin)) = slots.get(item) else {
            return;
        };

        // Selecting to inspect is never gated — you can read a crate's
        // contents from across the room.
        selection.0 = Some(item);
        actions.write(InventoryAction::Selected { board, item });

        if !access.interactive {
            return;
        }

        let grab_px = px - origin.as_vec2() * config.cell_px;
        *drag = InventoryDragState::Held {
            item,
            origin: *origin,
            grab_offset: grid::grab_offset(grab_px, config.cell_px),
            grab_px,
            press_px: px,
            moved: false,
            target: board,
        };
        return;
    }
}

/// While held: promotes a press to a real drag once it crosses
/// [`InventoryConfig::drag_threshold_px`], then keeps the grabbed point glued
/// to the cursor.
#[allow(clippy::type_complexity)]
pub fn update_drag(
    mut commands: Commands,
    mut boards: Query<
        (
            Entity,
            &InventoryCursor,
            &InventoryConfig,
            &InventoryTheme,
            &InventoryAccess,
            &mut InventoryDragState,
            &InventoryWindow,
        ),
        With<InventoryBoard>,
    >,
    mut nodes: Query<(&mut Node, &mut ZIndex, &InventoryItem)>,
    mut actions: MessageWriter<InventoryAction>,
) {
    for (board, cursor, config, theme, access, mut drag, window) in &mut boards {
        if !window.open {
            continue;
        }

        // Source board went out of reach mid-drag: snap the held item home
        // and drop the state. `track_drag_target` handles losing `transfers`
        // for free (the target just falls back to the source).
        if !access.interactive {
            if let InventoryDragState::Held { item, origin, .. } = *drag {
                if let Ok((mut node, mut z, data)) = nodes.get_mut(item) {
                    place_node(&mut node, origin, data.size, config);
                    *z = ZIndex(Z_ITEM_IDLE);
                }
                commands.entity(item).remove::<GlobalZIndex>();
                *drag = InventoryDragState::Idle;
                actions.write(InventoryAction::Rejected {
                    board,
                    item,
                    origin,
                });
            }
            continue;
        }

        let InventoryDragState::Held {
            item,
            grab_px,
            press_px,
            moved,
            ..
        } = &mut *drag
        else {
            continue;
        };
        let Some(px) = cursor.px else { continue };

        if !*moved {
            if px.distance(*press_px) < config.drag_threshold_px {
                continue;
            }
            *moved = true;
            actions.write(InventoryAction::PickedUp { board, item: *item });
            if let Ok((_, mut z, _)) = nodes.get_mut(*item) {
                *z = ZIndex(Z_ITEM_DRAGGED);
            }
            // Sibling `ZIndex` only ranks the item among its own board; a
            // `GlobalZIndex` is what carries it over another board's subtree.
            commands
                .entity(*item)
                .insert(GlobalZIndex(theme.dragged_global_z));
        }

        if let Ok((mut node, _, _)) = nodes.get_mut(*item) {
            let top_left = px - *grab_px;
            node.left = Val::Px(top_left.x);
            node.top = Val::Px(top_left.y);
        }
    }
}

/// Release. `moved == false` was a plain click — the press already selected
/// the item, nothing to do. Otherwise the item lands on its `target` board
/// (which [`track_drag_target`] keeps pointed at whatever's under the cursor):
///
/// - `target == source`, footprint fits → in-board move, [`InventoryAction::Dropped`].
/// - `target != source`, footprint fits → the grid entry, `ChildOf`, and
///   selection all move across; [`InventoryAction::Transferred`].
/// - doesn't fit → the node snaps back to `origin` on the source board;
///   [`InventoryAction::Rejected`].
#[allow(clippy::type_complexity)]
pub fn end_drag(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    mut boards: Query<
        (
            Entity,
            &InventoryCursor,
            &InventoryConfig,
            &mut InventoryGrid,
            &mut InventoryDragState,
            &mut InventorySelection,
            &InventoryWindow,
        ),
        With<InventoryBoard>,
    >,
    mut items: Query<(&InventoryItem, &mut InventorySlot, &mut Node, &mut ZIndex)>,
    mut actions: MessageWriter<InventoryAction>,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }

    // Collect first — a transfer mutates two board rows at once, so we can't
    // be holding a `&mut boards` iterator when we do it. One entry in
    // practice (one mouse), but a `Vec` keeps a stuck second drag from
    // quietly breaking the borrow shape.
    let mut drops: Vec<PendingDrop> = Vec::new();
    for (board, _, _, _, mut drag, _, window) in &mut boards {
        if !window.open {
            continue;
        }
        if let InventoryDragState::Held {
            item,
            origin,
            grab_offset,
            moved,
            target,
            ..
        } = *drag
        {
            drops.push(PendingDrop {
                source: board,
                item,
                origin,
                grab_offset,
                moved,
                target,
            });
            *drag = InventoryDragState::Idle;
        }
    }

    for drop in drops {
        let source = drop.source;

        if let Ok((_, _, _, mut z)) = items.get_mut(drop.item) {
            *z = ZIndex(Z_ITEM_IDLE);
        }
        commands.entity(drop.item).remove::<GlobalZIndex>();

        if !drop.moved {
            continue;
        }

        let size = match items.get(drop.item) {
            Ok((data, ..)) => data.size,
            Err(_) => continue,
        };

        // Landing origin is resolved in the TARGET board's cell space.
        let landing = {
            let Ok((_, cursor, config, ..)) = boards.get(drop.target) else {
                continue;
            };
            cursor.px.map(|px| {
                grid::target_origin(grid::hovered_cell(px, config.cell_px), drop.grab_offset)
            })
        };

        if drop.target == source {
            let Ok((_, _, config, mut grid, _, _, _)) = boards.get_mut(source) else {
                continue;
            };
            if landing.is_some_and(|o| grid.fits(o, size, Some(drop.item))) {
                let to = landing.unwrap().as_uvec2();
                grid.clear(drop.item);
                grid.place(drop.item, to, size);
                if let Ok((_, mut slot, mut node, _)) = items.get_mut(drop.item) {
                    slot.0 = to;
                    place_node(&mut node, to, size, config);
                }
                actions.write(InventoryAction::Dropped {
                    board: source,
                    item: drop.item,
                    from: drop.origin,
                    to,
                });
            } else {
                if let Ok((_, _, mut node, _)) = items.get_mut(drop.item) {
                    place_node(&mut node, drop.origin, size, config);
                }
                actions.write(InventoryAction::Rejected {
                    board: source,
                    item: drop.item,
                    origin: drop.origin,
                });
            }
            continue;
        }

        // Cross-board: two rows at once.
        let Ok([src_row, tgt_row]) = boards.get_many_mut([source, drop.target]) else {
            continue;
        };
        let (_, _, source_config, mut source_grid, _, mut source_sel, _) = src_row;
        let (_, _, target_config, mut target_grid, _, mut target_sel, _) = tgt_row;

        if landing.is_some_and(|o| target_grid.fits(o, size, Some(drop.item))) {
            let to = landing.unwrap().as_uvec2();
            source_grid.clear(drop.item);
            target_grid.place(drop.item, to, size);
            if source_sel.0 == Some(drop.item) {
                source_sel.0 = None;
            }
            target_sel.0 = Some(drop.item);
            if let Ok((_, mut slot, mut node, _)) = items.get_mut(drop.item) {
                slot.0 = to;
                place_node(&mut node, to, size, target_config);
            }
            commands.entity(drop.item).insert(ChildOf(drop.target));
            actions.write(InventoryAction::Transferred {
                from_board: source,
                to_board: drop.target,
                item: drop.item,
                from: drop.origin,
                to,
            });
        } else {
            if let Ok((_, _, mut node, _)) = items.get_mut(drop.item) {
                place_node(&mut node, drop.origin, size, source_config);
            }
            actions.write(InventoryAction::Rejected {
                board: source,
                item: drop.item,
                origin: drop.origin,
            });
        }
    }
}

/// A drag captured at release, before its board's state is cleared.
struct PendingDrop {
    /// The board the press landed on and the Held state lived on.
    source: Entity,
    item: Entity,
    origin: UVec2,
    grab_offset: UVec2,
    moved: bool,
    /// Where [`track_drag_target`] last pointed the drag — `source` for a
    /// plain in-board move.
    target: Entity,
}

/// Puts a board's interaction back to a known-good state whenever its
/// [`InventoryWindow`] changes — which covers opening, closing, and the frame
/// the component is first added (a harmless reset of an already-idle board).
///
/// Two things make this necessary rather than tidy. A drag that's live when
/// the window closes would otherwise be stranded: `sync_item_position` only
/// runs on `Changed<InventorySlot>`, and the slot never changes during a
/// drag. And `ui_focus_system` stops writing `RelativeCursorPosition`
/// entirely for a hidden node rather than clearing it, so the last value from
/// before the close survives — long enough for the very click that reopens
/// the window to be read as a click on the board.
#[allow(clippy::type_complexity)]
pub fn reset_interaction(
    mut commands: Commands,
    mut boards: Query<
        (
            &InventoryConfig,
            &mut InventoryDragState,
            &mut InventoryCursor,
        ),
        (With<InventoryBoard>, Changed<InventoryWindow>),
    >,
    mut items: Query<(&mut Node, &mut ZIndex, &InventoryItem)>,
) {
    for (config, mut drag, mut cursor) in &mut boards {
        if let InventoryDragState::Held { item, origin, .. } = &*drag {
            if let Ok((mut node, mut z, data)) = items.get_mut(*item) {
                place_node(&mut node, *origin, data.size, config);
                *z = ZIndex(Z_ITEM_IDLE);
            }
            commands.entity(*item).remove::<GlobalZIndex>();
        }
        *drag = InventoryDragState::Idle;
        *cursor = InventoryCursor::default();
    }
}
