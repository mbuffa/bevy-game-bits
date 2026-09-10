//! An optional hotbar for a [`crate::inventory`] board — a fixed strip of
//! numbered slots along the bottom of the screen, each a *pointer* to an item
//! that lives on the board.
//!
//! # It is not storage
//!
//! A slot holds an [`Entity`] only because that entity is already an item on
//! the board. Picking something up puts it in the board's grid first
//! ([`InventoryCommands::add`](super::InventoryCommands::add) enforces the
//! room check and returns `None` when the board is full); the quickbar then
//! records the entity in its first free slot. Nothing is ever *in* the
//! quickbar that isn't also in the pack.
//!
//! # What drives a slot
//!
//! - **Auto-assign** — any item on the board with no slot yet drops into the
//!   first free one ([`prune_and_assign_slots`]).
//! - **Drag** — dragging a board tile onto a slot moves (or swaps) the
//!   pointer there ([`quickbar_drop`], which claims the release so the drag
//!   never reads as a rejected off-board drop).
//! - **Prune** — a slot whose item has left the board (despawned, transferred
//!   away, evicted) clears itself. This is why a slot can never hold a
//!   dangling entity, and it's the only correct route:
//!   [`InventoryAction::Removed`](super::InventoryAction::Removed) and friends
//!   carry the item *data*, not its entity, so they can't identify a slot.
//!
//! # Selecting
//!
//! Number keys `1`–`9` and `0` set [`ActiveSlot`]. Pressing an empty slot's
//! key, or the already-active slot's key, clears it ("free hands"). A
//! double-click on the board ([`InventoryAction::Activated`](super::InventoryAction::Activated))
//! also activates the item's slot, assigning one first if it has none.
//!
//! Everything the module does, it also reports through [`QuickbarAction`] —
//! the "fire a message, let the game decide what it means" seam, the same as
//! [`InventoryAction`](super::InventoryAction) and `vehicle::impact`.
//!
//! # Wiring
//!
//! Add [`QuickbarPlugin`] beside [`InventoryPlugin`](super::InventoryPlugin)
//! (it needs the inventory systems registered — it orders against
//! [`InventorySet`](super::InventorySet)), then put a [`Quickbar`],
//! [`ActiveSlot`] and [`QuickbarStyle`] on whichever board should have a
//! strip. Colours come from that board's
//! [`InventoryTheme`](super::InventoryTheme); [`QuickbarStyle`] carries only
//! the pixel geometry.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::inventory::config::{InventoryConfig, InventoryTheme};
use crate::inventory::drag::{InventoryAction, InventoryDragState};
use crate::inventory::items::{InventoryItem, InventorySlot};
use crate::inventory::ui::{place_node, InventoryBoard, Z_ITEM_IDLE};
use crate::inventory::{InventorySet, InventoryWindow};

/// The slot pointers for one board, in strip order. A `Component` on the
/// board entity — the module's standing rule that everything about a board
/// lives on the board.
#[derive(Component, Clone, Debug)]
pub struct Quickbar {
    pub slots: Vec<Option<Entity>>,
}

impl Quickbar {
    /// A quickbar with `slots` empty slots.
    pub fn new(slots: usize) -> Self {
        Self {
            slots: vec![None; slots],
        }
    }

    /// The slot `item` currently sits in, if any.
    pub fn slot_of(&self, item: Entity) -> Option<usize> {
        self.slots.iter().position(|s| *s == Some(item))
    }

    /// The lowest empty slot, if the bar isn't full.
    pub fn first_free(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }
}

/// Which slot is "in hand", or `None` for free hands. A `Component` on the
/// board entity.
#[derive(Component, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ActiveSlot(pub Option<usize>);

/// The strip's pixel geometry. A `Component` on the board entity. Colours are
/// *not* here — they come from the board's
/// [`InventoryTheme`](super::InventoryTheme), so a re-skin of the pack
/// re-skins the strip for free.
#[derive(Component, Clone, Debug)]
pub struct QuickbarStyle {
    pub cell_px: f32,
    pub gap_px: f32,
    /// Distance from the bottom edge of the screen to the strip.
    pub bottom_px: f32,
    pub border_px: f32,
    pub index_font_size: f32,
    pub label_font_size: f32,
    /// Draw each slot's item name under its index.
    pub show_labels: bool,
}

impl Default for QuickbarStyle {
    fn default() -> Self {
        Self {
            cell_px: 48.0,
            gap_px: 6.0,
            bottom_px: 24.0,
            border_px: 2.0,
            index_font_size: 11.0,
            label_font_size: 10.0,
            show_labels: false,
        }
    }
}

/// The strip's satellite entities — the [`InventoryParts`](super::InventoryParts)
/// pattern. A `Component` on the board entity, added by [`build_quickbar_ui`].
#[derive(Component, Clone, Debug)]
pub struct QuickbarParts {
    /// The absolutely-positioned strip node. Carries [`RelativeCursorPosition`]
    /// so [`quickbar_drop`] can hit-test a release over it.
    pub root: Entity,
    /// One node per slot, strip order.
    pub cells: Vec<Entity>,
}

/// Marks the name text under a slot's index, so [`sync_quickbar_ui`] can find
/// it without walking `Children`.
#[derive(Component)]
pub struct QuickbarCellLabel;

/// What just happened on a quickbar. Fired, never acted on — a host turns it
/// into a sound, a viewmodel swap, or nothing. Every variant carries its
/// `board`.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum QuickbarAction {
    /// `item` now points from `slot` (auto-assign, drag, or double-click).
    Assigned {
        board: Entity,
        slot: usize,
        item: Entity,
    },
    /// `slot` is now the active one.
    Activated {
        board: Entity,
        slot: usize,
        item: Entity,
    },
    /// The board has free hands now.
    Deactivated { board: Entity },
    /// `slot`'s item left the board; the pointer was cleared.
    Cleared { board: Entity, slot: usize },
}

/// Ordering handles for the quickbar systems, chained in `Update`
/// `.after(InventorySet::Sync)`. [`quickbar_drop`] is the exception — it runs
/// inside [`InventorySet::Interaction`](super::InventorySet::Interaction),
/// between the inventory's own `update_drag` and `end_drag`.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuickbarSet {
    /// [`build_quickbar_ui`].
    Build,
    /// [`prune_and_assign_slots`].
    Assign,
    /// [`select_slot`].
    Input,
    /// [`sync_quickbar_ui`].
    Sync,
}

/// Which slot a strip-relative cursor position falls in. `normalized` is
/// centre-relative (`-0.5..0.5` on each axis), the shape
/// [`RelativeCursorPosition`] reports. `None` past any edge or for an empty
/// bar — a release there just isn't a slot assignment.
pub fn hovered_slot(normalized: Vec2, slots: usize) -> Option<usize> {
    if slots == 0 {
        return None;
    }
    if !(-0.5..=0.5).contains(&normalized.y) {
        return None;
    }
    let x = normalized.x + 0.5;
    if !(0.0..=1.0).contains(&x) {
        return None;
    }
    Some(((x * slots as f32).floor() as usize).min(slots - 1))
}

/// Add beside [`InventoryPlugin`](super::InventoryPlugin). Drives every board
/// that carries a [`Quickbar`].
#[derive(Default)]
pub struct QuickbarPlugin;

impl Plugin for QuickbarPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<QuickbarAction>()
            .configure_sets(
                Update,
                (
                    QuickbarSet::Build,
                    QuickbarSet::Assign,
                    QuickbarSet::Input,
                    QuickbarSet::Sync,
                )
                    .chain()
                    .after(InventorySet::Sync),
            )
            .add_systems(Update, build_quickbar_ui.in_set(QuickbarSet::Build))
            .add_systems(
                Update,
                prune_and_assign_slots.in_set(QuickbarSet::Assign),
            )
            .add_systems(Update, select_slot.in_set(QuickbarSet::Input))
            .add_systems(Update, sync_quickbar_ui.in_set(QuickbarSet::Sync))
            .add_systems(
                Update,
                quickbar_drop
                    .in_set(InventorySet::Interaction)
                    .after(crate::inventory::update_drag)
                    .before(crate::inventory::end_drag),
            );
    }
}

/// Builds the strip for any board that has a [`Quickbar`] but no
/// [`QuickbarParts`] yet — per frame, not once at `Startup`, so a board that
/// gains a quickbar mid-game still gets its strip (the
/// `world_map::build_party_visuals` pattern).
#[allow(clippy::type_complexity)]
pub fn build_quickbar_ui(
    mut commands: Commands,
    boards: Query<
        (Entity, &Quickbar, &QuickbarStyle),
        (With<InventoryBoard>, Without<QuickbarParts>),
    >,
) {
    for (board, quickbar, style) in &boards {
        let n = quickbar.slots.len();
        let width = n as f32 * style.cell_px + n.saturating_sub(1) as f32 * style.gap_px;

        let root = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(style.bottom_px),
                    left: Val::Percent(50.0),
                    margin: UiRect::left(Val::Px(-width / 2.0)),
                    width: Val::Px(width),
                    height: Val::Px(style.cell_px),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(style.gap_px),
                    ..default()
                },
                RelativeCursorPosition::default(),
            ))
            .id();

        let mut cells = Vec::with_capacity(n);
        for i in 0..n {
            let digit = if i == 9 { 0 } else { i + 1 };
            let cell = commands
                .spawn((
                    Node {
                        width: Val::Px(style.cell_px),
                        height: Val::Px(style.cell_px),
                        border: UiRect::all(Val::Px(style.border_px)),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(2.0)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderColor::all(Color::NONE),
                    ChildOf(root),
                ))
                .with_children(|cell| {
                    cell.spawn((
                        Text::new(digit.to_string()),
                        TextFont::from_font_size(style.index_font_size),
                        TextColor(Color::WHITE),
                    ));
                    cell.spawn((
                        QuickbarCellLabel,
                        Text::new(""),
                        TextFont::from_font_size(style.label_font_size),
                        TextColor(Color::WHITE),
                    ));
                })
                .id();
            cells.push(cell);
        }

        commands
            .entity(board)
            .insert(QuickbarParts { root, cells });
    }
}

/// Clears slots whose item has left the board, then drops any still-loose
/// board item into the first free slot. One idempotent pass — the only place
/// a slot is cleared by the item leaving, and the reason a slot never holds a
/// dangling entity.
pub fn prune_and_assign_slots(
    mut boards: Query<(Entity, &mut Quickbar, &mut ActiveSlot), With<InventoryBoard>>,
    items: Query<(Entity, &ChildOf), With<InventoryItem>>,
    mut actions: MessageWriter<QuickbarAction>,
) {
    for (board, mut quickbar, mut active) in &mut boards {
        for slot in 0..quickbar.slots.len() {
            let Some(item) = quickbar.slots[slot] else {
                continue;
            };
            let still_here = matches!(items.get(item), Ok((_, parent)) if parent.parent() == board);
            if still_here {
                continue;
            }
            quickbar.slots[slot] = None;
            actions.write(QuickbarAction::Cleared { board, slot });
            if active.0 == Some(slot) {
                active.0 = None;
                actions.write(QuickbarAction::Deactivated { board });
            }
        }

        for (item, parent) in &items {
            if parent.parent() != board || quickbar.slot_of(item).is_some() {
                continue;
            }
            let Some(free) = quickbar.first_free() else {
                break;
            };
            quickbar.slots[free] = Some(item);
            actions.write(QuickbarAction::Assigned {
                board,
                slot: free,
                item,
            });
        }
    }
}

/// Number keys `1`–`9`/`0` set [`ActiveSlot`]; an empty or already-active
/// slot's key frees the hands. Also promotes a board double-click
/// ([`InventoryAction::Activated`]) into activating that item's slot.
pub fn select_slot(
    keys: Res<ButtonInput<KeyCode>>,
    mut inventory_actions: MessageReader<InventoryAction>,
    mut boards: Query<(Entity, &mut Quickbar, &mut ActiveSlot), With<InventoryBoard>>,
    mut actions: MessageWriter<QuickbarAction>,
) {
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
    ];
    let pressed = DIGITS.iter().position(|k| keys.just_pressed(*k));

    let equips: Vec<(Entity, Entity)> = inventory_actions
        .read()
        .filter_map(|a| match a {
            InventoryAction::Activated { board, item } => Some((*board, *item)),
            _ => None,
        })
        .collect();

    for (board, mut quickbar, mut active) in &mut boards {
        if let Some(slot) = pressed {
            if slot < quickbar.slots.len() {
                let target = if active.0 == Some(slot) || quickbar.slots[slot].is_none() {
                    None
                } else {
                    Some(slot)
                };
                set_active(board, &mut active, target, &quickbar, &mut actions);
            }
        }

        for &(equip_board, item) in &equips {
            if equip_board != board {
                continue;
            }
            let slot = if let Some(slot) = quickbar.slot_of(item) {
                Some(slot)
            } else if let Some(free) = quickbar.first_free() {
                quickbar.slots[free] = Some(item);
                actions.write(QuickbarAction::Assigned {
                    board,
                    slot: free,
                    item,
                });
                Some(free)
            } else {
                None
            };
            if let Some(slot) = slot {
                set_active(board, &mut active, Some(slot), &quickbar, &mut actions);
            }
        }
    }
}

fn set_active(
    board: Entity,
    active: &mut ActiveSlot,
    target: Option<usize>,
    quickbar: &Quickbar,
    actions: &mut MessageWriter<QuickbarAction>,
) {
    if active.0 == target {
        return;
    }
    active.0 = target;
    match target.and_then(|slot| quickbar.slots.get(slot).copied().flatten().map(|i| (slot, i))) {
        Some((slot, item)) => actions.write(QuickbarAction::Activated { board, slot, item }),
        None => actions.write(QuickbarAction::Deactivated { board }),
    };
}

/// A drag released over the strip: move (empty target) or swap (occupied
/// target) the pointer, then **claim the release** — revert the tile to its
/// own cell, drop the drag state to `Idle` — so the inventory's `end_drag`
/// finds nothing and fires no spurious `Rejected`.
#[allow(clippy::type_complexity)]
pub fn quickbar_drop(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    strips: Query<&RelativeCursorPosition>,
    mut boards: Query<
        (
            Entity,
            &mut InventoryDragState,
            &InventoryConfig,
            &InventoryWindow,
            &mut Quickbar,
            &mut ActiveSlot,
            &QuickbarParts,
        ),
        With<InventoryBoard>,
    >,
    mut items: Query<(&InventoryItem, &InventorySlot, &mut Node, &mut ZIndex)>,
    mut actions: MessageWriter<QuickbarAction>,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    for (board, mut drag, config, window, mut quickbar, mut active, parts) in &mut boards {
        if !window.open {
            continue;
        }
        let InventoryDragState::Held { item, moved, .. } = *drag else {
            continue;
        };
        if !moved {
            continue;
        }
        let Ok(rel) = strips.get(parts.root) else {
            continue;
        };
        if !rel.cursor_over {
            continue;
        }
        let Some(slot) = rel
            .normalized
            .and_then(|n| hovered_slot(n, quickbar.slots.len()))
        else {
            continue;
        };

        let from = quickbar.slot_of(item);
        let displaced = quickbar.slots[slot];
        quickbar.slots[slot] = Some(item);
        if let Some(from) = from {
            quickbar.slots[from] = displaced;
        }
        // The active pointer follows whichever item moved.
        if active.0 == from {
            active.0 = Some(slot);
        } else if active.0 == Some(slot) {
            active.0 = from;
        }
        actions.write(QuickbarAction::Assigned { board, slot, item });

        // Claim the release: the item never left its board, so put its node
        // back where the grid still has it.
        if let Ok((data, item_slot, mut node, mut z)) = items.get_mut(item) {
            place_node(&mut node, item_slot.0, data.size, config);
            *z = ZIndex(Z_ITEM_IDLE);
        }
        commands.entity(item).remove::<GlobalZIndex>();
        *drag = InventoryDragState::Idle;
    }
}

/// Every-frame mirror: each slot's fill is its item's colour (or the theme's
/// empty-cell colour), its border marks the active slot, its label is the
/// index plus (optionally) the item name. The `lights::sync_*` rule.
#[allow(clippy::type_complexity)]
pub fn sync_quickbar_ui(
    boards: Query<
        (
            &Quickbar,
            &ActiveSlot,
            &QuickbarParts,
            &QuickbarStyle,
            &InventoryTheme,
        ),
        With<InventoryBoard>,
    >,
    items: Query<&InventoryItem>,
    mut cells: Query<(&mut BackgroundColor, &mut BorderColor)>,
    mut labels: Query<(&ChildOf, &mut Text), With<QuickbarCellLabel>>,
) {
    for (quickbar, active, parts, style, theme) in &boards {
        for (i, cell) in parts.cells.iter().enumerate() {
            let item = quickbar.slots.get(i).copied().flatten();
            let data = item.and_then(|e| items.get(e).ok());

            if let Ok((mut background, mut border)) = cells.get_mut(*cell) {
                let wanted_bg = data.map(|d| d.color).unwrap_or(theme.cell_background);
                if background.0 != wanted_bg {
                    background.0 = wanted_bg;
                }
                let wanted_border = if active.0 == Some(i) {
                    theme.selected_border
                } else {
                    theme.cell_border
                };
                if border.top != wanted_border {
                    *border = BorderColor::all(wanted_border);
                }
            }

            for (parent, mut text) in &mut labels {
                if parent.parent() != *cell {
                    continue;
                }
                let wanted = if style.show_labels {
                    data.map(|d| d.name.to_string()).unwrap_or_default()
                } else {
                    String::new()
                };
                if text.0 != wanted {
                    text.0 = wanted;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..n).map(|_| world.spawn_empty().id()).collect()
    }

    #[test]
    fn first_free_and_slot_of_track_occupancy() {
        let e = ids(2);
        let mut bar = Quickbar::new(3);
        assert_eq!(bar.first_free(), Some(0));
        bar.slots[0] = Some(e[0]);
        bar.slots[1] = Some(e[1]);
        assert_eq!(bar.first_free(), Some(2));
        assert_eq!(bar.slot_of(e[1]), Some(1));
        assert_eq!(bar.slot_of(e[0]), Some(0));
        bar.slots[2] = Some(ids(1)[0]);
        assert_eq!(bar.first_free(), None);
    }

    #[test]
    fn hovered_slot_splits_the_strip_evenly() {
        assert_eq!(hovered_slot(Vec2::new(-0.5, 0.0), 10), Some(0));
        assert_eq!(hovered_slot(Vec2::new(-0.45, 0.0), 10), Some(0));
        assert_eq!(hovered_slot(Vec2::new(0.0, 0.0), 10), Some(5));
        assert_eq!(hovered_slot(Vec2::new(0.49, 0.0), 10), Some(9));
        // Exactly the right edge clamps to the last slot rather than 10.
        assert_eq!(hovered_slot(Vec2::new(0.5, 0.0), 10), Some(9));
    }

    #[test]
    fn hovered_slot_rejects_off_strip() {
        assert_eq!(hovered_slot(Vec2::new(-0.6, 0.0), 10), None);
        assert_eq!(hovered_slot(Vec2::new(0.6, 0.0), 10), None);
        assert_eq!(hovered_slot(Vec2::new(0.0, 0.9), 10), None);
        assert_eq!(hovered_slot(Vec2::new(0.0, 0.0), 0), None);
    }
}
