//! Board and description-panel layout, and the `sync_*` systems that keep the
//! view matching the model. Plain `Node` UI — no egui, matching every example
//! in the repo this was extracted from.
//!
//! Each board's whole tree hangs off its own absolutely-positioned
//! [`InventoryRoot`] overlay, so it can sit on top of a host's own UI without
//! fighting their layout — and so hiding it ([`sync_window_visibility`]) can
//! take it fully out of that layout too.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::inventory::config::{InventoryBoardSpec, InventoryConfig, InventoryTheme};
use crate::inventory::drag::{
    InventoryClicks, InventoryCursor, InventoryDragState, InventorySelection,
};
use crate::inventory::grid::InventoryGrid;
use crate::inventory::items::{InventoryItem, InventorySlot};
use crate::inventory::InventoryWindow;

/// Marks the board node of one inventory. Carries [`RelativeCursorPosition`]
/// so `drag::track_cursor` can read the cursor over it every frame, even
/// while a dragged item node sits on top — `ui_focus_system` writes that
/// component for every node that has it, regardless of what's stacked above.
///
/// Every per-board component ([`InventoryConfig`], [`InventoryTheme`],
/// [`InventoryLayout`](super::InventoryLayout), [`InventoryGrid`],
/// [`InventoryCursor`], [`InventoryDragState`], [`InventorySelection`],
/// [`InventoryClicks`], [`InventoryWindow`],
/// [`InventoryAccess`](super::InventoryAccess),
/// [`InventoryTransferTarget`](super::InventoryTransferTarget),
/// [`InventoryParts`]) lives on this entity.
#[derive(Component)]
pub struct InventoryBoard;

/// The full-screen overlay one board's tree hangs off. Reparent it (it's a
/// normal entity) if you'd rather the inventory sat inside your own layout.
#[derive(Component)]
pub struct InventoryRoot;

/// One static background tile. `ChildOf` the board.
#[derive(Component)]
pub struct InventoryCell(pub UVec2);

#[derive(Component)]
pub struct InventoryDescriptionSwatch;

#[derive(Component)]
pub struct InventoryDescriptionText;

/// Sibling stacking order — `ZIndex`, not `GlobalZIndex`, because these only
/// need to out-rank other item nodes under the same board.
pub const Z_ITEM_IDLE: i32 = 0;
pub const Z_ITEM_DRAGGED: i32 = 1;

/// The satellite entities [`spawn_inventory`] built for one board — the
/// overlay root and, if [`InventoryConfig::show_description_panel`] was set,
/// the description panel and the two nodes inside it. A `Component` on the
/// board entity.
///
/// With more than one camera in your app, insert `UiTargetCamera` on `root`.
#[derive(Component, Clone, Copy, Debug)]
pub struct InventoryParts {
    pub root: Entity,
    pub description_panel: Option<Entity>,
    pub description_swatch: Option<Entity>,
    pub description_text: Option<Entity>,
}

/// Build one inventory board — overlay, board node, cell tiles, and (if
/// configured) description panel — and return the **board entity**, which
/// carries every per-board component and is the handle every other API takes.
///
/// Synchronous: the entity is usable the moment this returns, so a host never
/// has to order work `.after(InventorySet::Setup)` to find its board. Spawns
/// no camera — that's the host's job.
pub fn spawn_inventory(commands: &mut Commands, spec: InventoryBoardSpec) -> Entity {
    let InventoryBoardSpec {
        config,
        theme,
        layout,
        window,
        access,
        transfer_target,
    } = spec;

    let root = commands
        .spawn((
            InventoryRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: layout.panel_side.flex_direction(),
                justify_content: layout.horizontal,
                align_items: layout.vertical,
                column_gap: Val::Px(layout.panel_gap_px),
                row_gap: Val::Px(layout.panel_gap_px),
                padding: UiRect::all(Val::Px(layout.root_padding_px)),
                ..default()
            },
        ))
        .id();

    // Flex order is child-spawn order: for a panel on the left / above, it is
    // spawned before the board column; otherwise after.
    let panel_first = config.show_description_panel && layout.panel_side.panel_first();
    let panel_before = panel_first.then(|| spawn_description_panel(commands, root, &theme));

    let column = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(theme.board_row_gap_px),
                ..default()
            },
            ChildOf(root),
        ))
        .id();

    if let Some(title) = &config.title {
        commands.spawn((
            Text::new(title.clone()),
            TextFont {
                font_size: theme.title_font_size,
                ..default()
            },
            TextColor(theme.panel_text),
            ChildOf(column),
        ));
    }

    let board_size = config.board_size();
    let board = commands
        .spawn((
            InventoryBoard,
            RelativeCursorPosition::default(),
            Node {
                width: Val::Px(board_size.x),
                height: Val::Px(board_size.y),
                ..default()
            },
            BackgroundColor(theme.panel_background),
            ChildOf(column),
        ))
        .id();

    rebuild_cells(commands, board, &config, &theme, std::iter::empty());

    let panel_after = (config.show_description_panel && !panel_first)
        .then(|| spawn_description_panel(commands, root, &theme));
    let (description_panel, description_swatch, description_text) =
        match panel_before.or(panel_after) {
            Some((panel, swatch, text)) => (Some(panel), Some(swatch), Some(text)),
            None => (None, None, None),
        };

    commands.entity(board).insert((
        InventoryGrid::new(config.cols, config.rows),
        InventoryCursor::default(),
        InventoryDragState::default(),
        InventorySelection::default(),
        InventoryClicks::default(),
        window,
        access,
        transfer_target,
        layout,
        config,
        theme,
        InventoryParts {
            root,
            description_panel,
            description_swatch,
            description_text,
        },
    ));

    board
}

/// Write an item node's rect from its grid origin and footprint, in one
/// board's `config`. The single copy of the cell->pixel formula — spawn, the
/// per-frame sync, every drag revert, and a cross-board transfer (where the
/// receiving board's `cell_px` may differ) all go through here.
pub(crate) fn place_node(node: &mut Node, origin: UVec2, size: UVec2, config: &InventoryConfig) {
    node.left = Val::Px(origin.x as f32 * config.cell_px + config.item_inset_px);
    node.top = Val::Px(origin.y as f32 * config.cell_px + config.item_inset_px);
    node.width = Val::Px(size.x as f32 * config.cell_px - 2.0 * config.item_inset_px);
    node.height = Val::Px(size.y as f32 * config.cell_px - 2.0 * config.item_inset_px);
}

/// Despawn `old` cell tiles and spawn a fresh `cols`x`rows` grid of them as
/// children of `board`. Used by [`spawn_inventory`] and by resize, so the
/// tiles can't disagree with the grid.
pub(crate) fn rebuild_cells(
    commands: &mut Commands,
    board: Entity,
    config: &InventoryConfig,
    theme: &InventoryTheme,
    old: impl IntoIterator<Item = Entity>,
) {
    for entity in old {
        commands.entity(entity).despawn();
    }
    for y in 0..config.rows {
        for x in 0..config.cols {
            commands.spawn((
                InventoryCell(UVec2::new(x, y)),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x as f32 * config.cell_px),
                    top: Val::Px(y as f32 * config.cell_px),
                    width: Val::Px(config.cell_px),
                    height: Val::Px(config.cell_px),
                    border: UiRect::all(Val::Px(theme.cell_border_px)),
                    ..default()
                },
                BackgroundColor(theme.cell_background),
                BorderColor::all(theme.cell_border),
                ChildOf(board),
            ));
        }
    }
}

fn spawn_description_panel(
    commands: &mut Commands,
    parent: Entity,
    theme: &InventoryTheme,
) -> (Entity, Entity, Entity) {
    let panel = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(theme.description_panel_row_gap_px),
                width: Val::Px(theme.description_panel_width_px),
                padding: UiRect::all(Val::Px(theme.description_panel_padding_px)),
                ..default()
            },
            BackgroundColor(theme.panel_background),
            ChildOf(parent),
        ))
        .id();

    let swatch = commands
        .spawn((
            InventoryDescriptionSwatch,
            Node {
                width: Val::Px(theme.description_swatch_px),
                height: Val::Px(theme.description_swatch_px),
                border: UiRect::all(Val::Px(theme.description_swatch_border_px)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(theme.cell_border),
            ChildOf(panel),
        ))
        .id();

    let text = commands
        .spawn((
            InventoryDescriptionText,
            Text::new(theme.description_placeholder.clone()),
            TextFont {
                font_size: theme.description_font_size,
                ..default()
            },
            TextColor(theme.panel_text),
            ChildOf(panel),
        ))
        .id();

    (panel, swatch, text)
}

/// Mirrors each board's [`InventoryWindow`] onto its root's `Visibility` and
/// `Node.display`, every frame, writing only on an actual change.
///
/// Both, not one: `Visibility` is the honest "don't render" switch, but only
/// `display` takes the overlay out of layout — which matters the moment
/// somebody reparents the root into their own flex column. Neither blocks
/// clicks to whatever's underneath.
///
/// A mirror rather than `OnEnter`/`OnExit` because a board spawned mid-game
/// with `open: false` has to come up hidden on its very first frame, with no
/// transition to hook.
pub fn sync_window_visibility(
    boards: Query<(&InventoryWindow, &InventoryParts), With<InventoryBoard>>,
    mut roots: Query<(&mut Node, &mut Visibility), With<InventoryRoot>>,
) {
    for (window, parts) in &boards {
        let Ok((mut node, mut visibility)) = roots.get_mut(parts.root) else {
            continue;
        };
        let wanted_display = if window.open {
            Display::Flex
        } else {
            Display::None
        };
        let wanted_visibility = if window.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if node.display != wanted_display {
            node.display = wanted_display;
        }
        if *visibility != wanted_visibility {
            *visibility = wanted_visibility;
        }
    }
}

/// Keeps an item's `Node` in step with its [`InventorySlot`] — except while
/// it's being dragged, when `drag` drives its position directly from the
/// cursor instead. Reads the parent from `ChildOf`, so it also lands a
/// transferred item in its new board's cells the frame the reparent flushes.
pub fn sync_item_position(
    boards: Query<(&InventoryDragState, &InventoryConfig), With<InventoryBoard>>,
    mut items: Query<
        (Entity, &ChildOf, &InventorySlot, &InventoryItem, &mut Node),
        Changed<InventorySlot>,
    >,
) {
    for (entity, child_of, slot, item, mut node) in &mut items {
        let Ok((drag, config)) = boards.get(child_of.parent()) else {
            continue;
        };
        if drag.held_item() == Some(entity) {
            continue;
        }
        place_node(&mut node, slot.0, item.size, config);
    }
}

/// Colored border on each board's selected item, cleared everywhere else.
/// `Changed<InventorySelection>` on the board query is the efficiency guard —
/// it fires again the frame a selection changes even while the window is
/// closed, so the hidden board's borders are already right when it reopens.
#[allow(clippy::type_complexity)]
pub fn sync_selection_outline(
    boards: Query<
        (Entity, &InventorySelection, &InventoryTheme),
        (With<InventoryBoard>, Changed<InventorySelection>),
    >,
    mut items: Query<(Entity, &ChildOf, &mut BorderColor), With<InventoryItem>>,
) {
    for (board, selection, theme) in &boards {
        for (item, child_of, mut border) in &mut items {
            if child_of.parent() != board {
                continue;
            }
            let wanted = if selection.0 == Some(item) {
                theme.selected_border
            } else {
                Color::NONE
            };
            if border.top != wanted {
                *border = BorderColor::all(wanted);
            }
        }
    }
}

/// Fills each board's description panel from its [`InventorySelection`], or
/// shows the placeholder when nothing is selected. A no-op for a board whose
/// [`InventoryConfig::show_description_panel`] was `false`.
#[allow(clippy::type_complexity)]
pub fn sync_description(
    boards: Query<
        (&InventorySelection, &InventoryParts, &InventoryTheme),
        (With<InventoryBoard>, Changed<InventorySelection>),
    >,
    items: Query<&InventoryItem>,
    mut swatches: Query<&mut BackgroundColor, With<InventoryDescriptionSwatch>>,
    mut texts: Query<&mut Text, With<InventoryDescriptionText>>,
) {
    for (selection, parts, theme) in &boards {
        let (Some(swatch_e), Some(text_e)) = (parts.description_swatch, parts.description_text)
        else {
            continue;
        };
        let (Ok(mut swatch), Ok(mut text)) = (swatches.get_mut(swatch_e), texts.get_mut(text_e))
        else {
            continue;
        };
        match selection.0.and_then(|e| items.get(e).ok()) {
            Some(item) => {
                swatch.0 = item.color;
                text.0 = format!(
                    "{}\n{}x{}\n\n{}",
                    item.name, item.size.x, item.size.y, item.description
                );
            }
            None => {
                swatch.0 = Color::NONE;
                text.0 = theme.description_placeholder.to_string();
            }
        }
    }
}

/// Recomputed every frame while a drag is live: paints the would-be footprint
/// green (fits) or red (doesn't) on **the board the drag is targeting**, which
/// may not be the one it started on — everything else goes back to the plain
/// cell background.
#[allow(clippy::type_complexity)]
pub fn sync_placement_preview(
    boards: Query<
        (
            Entity,
            &InventoryDragState,
            &InventoryCursor,
            &InventoryGrid,
            &InventoryConfig,
            &InventoryTheme,
            &InventoryWindow,
        ),
        With<InventoryBoard>,
    >,
    items: Query<&InventoryItem>,
    mut cells: Query<(&ChildOf, &InventoryCell, &mut BackgroundColor)>,
) {
    // The one live drag's source state, and the board it points at. The
    // preview is computed against that target board's own cursor/grid/config.
    let active: Option<(Entity, &InventoryDragState)> =
        boards
            .iter()
            .find_map(|(_, drag, _, _, _, _, window)| match drag {
                InventoryDragState::Held { target, .. } if window.open => Some((*target, drag)),
                _ => None,
            });

    for (board, _, cursor, grid, config, theme, window) in &boards {
        let preview = match active {
            Some((target, drag)) if target == board && window.open => {
                drag.preview(cursor, grid, config, &items)
            }
            _ => None,
        };
        for (child_of, InventoryCell(cell), mut background) in &mut cells {
            if child_of.parent() != board {
                continue;
            }
            let wanted = match preview {
                Some(p) if p.covers(*cell) => {
                    if p.fits {
                        theme.preview_ok
                    } else {
                        theme.preview_bad
                    }
                }
                _ => theme.cell_background,
            };
            if background.0 != wanted {
                background.0 = wanted;
            }
        }
    }
}
