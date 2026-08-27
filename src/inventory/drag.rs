//! Press-to-select, hold-to-drag, release-to-drop. A press that never moves
//! past [`InventoryConfig::drag_threshold_px`] is a plain click: it selects
//! the item (so `ui::sync_description` shows it) without ever entering the
//! held state.
//!
//! The held item is never removed from [`InventoryGrid`] mid-drag — it stays
//! placed at its old cells the whole time, and every occupancy check passes
//! `ignoring: Some(item)` so it doesn't block its own drop. That means a
//! crash or an early return mid-drag can never leave a hole in the grid.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::inventory::config::InventoryConfig;
use crate::inventory::grid::{self, InventoryGrid};
use crate::inventory::items::{InventoryItem, InventorySlot};
use crate::inventory::ui::{InventoryBoard, Z_ITEM_DRAGGED, Z_ITEM_IDLE};

/// The item shown in the description panel — set on every press, dragged or
/// not, which is what makes a plain click "select" for free.
#[derive(Resource, Default)]
pub struct InventorySelection(pub Option<Entity>);

/// Cursor position relative to the board's top-left corner, refreshed every
/// frame by [`track_cursor`].
#[derive(Resource, Default)]
pub struct InventoryCursor {
    /// Logical pixels. Extrapolated beyond the board (including negative)
    /// once a drag carries the cursor past an edge —
    /// [`grid::target_origin`] and [`InventoryGrid::fits`] are what reject
    /// that, not this.
    pub px: Option<Vec2>,
    /// True only while the raw cursor sits within the board's own rect.
    /// Starting a drag/selection requires this — `px` alone isn't enough,
    /// since it still extrapolates a value even when the click landed on
    /// the description panel instead.
    pub over_board: bool,
}

#[derive(Resource, Default)]
pub enum InventoryDragState {
    #[default]
    Idle,
    Held {
        item: Entity,
        /// Where to revert to if the drop is rejected, or if the window
        /// closes mid-drag.
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
    },
}

impl InventoryDragState {
    pub fn held_item(&self) -> Option<Entity> {
        match self {
            InventoryDragState::Held { item, .. } => Some(*item),
            InventoryDragState::Idle => None,
        }
    }

    /// The footprint the held item would land in if released right now —
    /// `None` before a press has turned into an actual drag.
    pub fn preview(
        &self,
        cursor: &InventoryCursor,
        grid: &InventoryGrid,
        config: &InventoryConfig,
        items: &Query<&InventoryItem>,
    ) -> Option<InventoryPreview> {
        let InventoryDragState::Held { item, grab_offset, moved, .. } = self else {
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
/// that's outside, which is what makes an overhanging footprint read red
/// without needing a separate "partly off the board" case.
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

/// What just happened on the board. This module plays no sound and shows no
/// animation of its own — the same way `vehicle::impact` leaves what
/// *breaks* to the game, this leaves what a pick-up *sounds like* to the
/// game.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryAction {
    Selected { item: Entity },
    PickedUp { item: Entity },
    Dropped { item: Entity, from: UVec2, to: UVec2 },
    Rejected { item: Entity, origin: UVec2 },
}

/// `RelativeCursorPosition` -> [`InventoryCursor`]. `ui_focus_system` (part
/// of `DefaultPlugins`) writes it before `Update` runs, so this always reads
/// the current frame's cursor.
///
/// `Option<Single<..>>`, not a bare `Single`: a bare `Single` silently skips
/// the whole system if the board doesn't exist yet or was despawned, which
/// is fine for a solo example but would leave a host with no error to find.
pub fn track_cursor(
    board: Option<Single<&RelativeCursorPosition, With<InventoryBoard>>>,
    config: Res<InventoryConfig>,
    mut cursor: ResMut<InventoryCursor>,
) {
    let Some(board) = board else {
        cursor.px = None;
        cursor.over_board = false;
        return;
    };
    let board_size = config.board_size();
    cursor.px = board.normalized.map(|normalized| (normalized + Vec2::splat(0.5)) * board_size);
    cursor.over_board = board.cursor_over;
}

/// A press over the board: selects whatever's under the cursor (or clears
/// the selection over an empty cell / outside the board) and, if it landed
/// on an item, arms a potential drag.
pub fn begin_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<InventoryCursor>,
    grid: Res<InventoryGrid>,
    config: Res<InventoryConfig>,
    mut drag: ResMut<InventoryDragState>,
    mut selection: ResMut<InventorySelection>,
    slots: Query<&InventorySlot>,
    mut actions: MessageWriter<InventoryAction>,
) {
    if !buttons.just_pressed(MouseButton::Left) || !matches!(*drag, InventoryDragState::Idle) {
        return;
    }
    if !cursor.over_board {
        selection.0 = None;
        return;
    }
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
    let Ok(InventorySlot(origin)) = slots.get(item) else { return };

    selection.0 = Some(item);
    let grab_px = px - origin.as_vec2() * config.cell_px;
    *drag = InventoryDragState::Held {
        item,
        origin: *origin,
        grab_offset: grid::grab_offset(grab_px, config.cell_px),
        grab_px,
        press_px: px,
        moved: false,
    };
    actions.write(InventoryAction::Selected { item });
}

/// While held: promotes a press to a real drag once it crosses
/// [`InventoryConfig::drag_threshold_px`], then keeps the grabbed point
/// glued to the cursor.
pub fn update_drag(
    cursor: Res<InventoryCursor>,
    config: Res<InventoryConfig>,
    mut drag: ResMut<InventoryDragState>,
    mut actions: MessageWriter<InventoryAction>,
    mut nodes: Query<(&mut Node, &mut ZIndex)>,
) {
    let InventoryDragState::Held { item, grab_px, press_px, moved, .. } = &mut *drag else {
        return;
    };
    let Some(px) = cursor.px else { return };

    if !*moved {
        if px.distance(*press_px) < config.drag_threshold_px {
            return;
        }
        *moved = true;
        actions.write(InventoryAction::PickedUp { item: *item });
        if let Ok((_, mut z)) = nodes.get_mut(*item) {
            *z = ZIndex(Z_ITEM_DRAGGED);
        }
    }

    if let Ok((mut node, _)) = nodes.get_mut(*item) {
        let top_left = px - *grab_px;
        node.left = Val::Px(top_left.x);
        node.top = Val::Px(top_left.y);
    }
}

/// Release: commits the move if the snapped target fits, otherwise reverts
/// the node back to `origin`. A press that never became a drag (`!moved`)
/// leaves the item exactly where it was — it was already selected on press.
pub fn end_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<InventoryCursor>,
    config: Res<InventoryConfig>,
    mut drag: ResMut<InventoryDragState>,
    mut grid: ResMut<InventoryGrid>,
    mut actions: MessageWriter<InventoryAction>,
    mut items: Query<(&InventoryItem, &mut InventorySlot, &mut Node, &mut ZIndex)>,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let InventoryDragState::Held { item, origin, grab_offset, moved, .. } = *drag else {
        return;
    };
    *drag = InventoryDragState::Idle;

    let Ok((item_data, mut slot, mut node, mut z)) = items.get_mut(item) else {
        return;
    };
    *z = ZIndex(Z_ITEM_IDLE);

    if !moved {
        return;
    }

    let target = cursor.px.map(|px| {
        let hovered = grid::hovered_cell(px, config.cell_px);
        grid::target_origin(hovered, grab_offset)
    });
    let placed = target.is_some_and(|target| grid.fits(target, item_data.size, Some(item)));

    if placed {
        let target = target.unwrap().as_uvec2();
        grid.clear(item);
        grid.place(item, target, item_data.size);
        // `Changed<InventorySlot>` picks this up in `ui::sync_item_position`.
        slot.0 = target;
        actions.write(InventoryAction::Dropped { item, from: origin, to: target });
    } else {
        // `slot.0` never changed, so nothing else will move this node back
        // — do it here.
        node.left = Val::Px(origin.x as f32 * config.cell_px + config.item_inset_px);
        node.top = Val::Px(origin.y as f32 * config.cell_px + config.item_inset_px);
        actions.write(InventoryAction::Rejected { item, origin });
    }
}

/// Puts interaction back to a known-good state on both edges of the window
/// opening and closing. Registered on `OnEnter` *and* `OnExit` of
/// [`InventoryWindowState::Open`](super::InventoryWindowState::Open).
///
/// Two things make this necessary rather than tidy. A drag that's live when
/// the window closes would otherwise be stranded: `sync_item_position` only
/// runs on `Changed<InventorySlot>`, and the slot never changes during a
/// drag, so nothing would ever move that node back. And `ui_focus_system`
/// stops writing `RelativeCursorPosition` entirely for a hidden node rather
/// than clearing it, so the last value from before the close survives — long
/// enough for the very click that reopens the window to be read as a click
/// on the board.
pub fn reset_interaction(
    mut drag: ResMut<InventoryDragState>,
    mut cursor: ResMut<InventoryCursor>,
    config: Res<InventoryConfig>,
    mut items: Query<(&mut Node, &mut ZIndex)>,
) {
    if let InventoryDragState::Held { item, origin, .. } = &*drag {
        if let Ok((mut node, mut z)) = items.get_mut(*item) {
            node.left = Val::Px(origin.x as f32 * config.cell_px + config.item_inset_px);
            node.top = Val::Px(origin.y as f32 * config.cell_px + config.item_inset_px);
            *z = ZIndex(Z_ITEM_IDLE);
        }
    }
    *drag = InventoryDragState::Idle;
    *cursor = InventoryCursor::default();
}
