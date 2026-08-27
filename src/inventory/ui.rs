//! Board and description-panel layout. Plain `Node` UI — no egui, matching
//! every example in the repo this was extracted from.
//!
//! The whole tree hangs off one absolutely-positioned [`InventoryRoot`]
//! overlay, so it can sit on top of a host's own UI without fighting their
//! layout — and so hiding it (`sync_window_visibility`) can take it fully out
//! of that layout too.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::inventory::config::{InventoryConfig, InventoryTheme};
use crate::inventory::drag::{InventoryCursor, InventoryDragState, InventorySelection};
use crate::inventory::grid::InventoryGrid;
use crate::inventory::items::{InventoryItem, InventorySlot};
use crate::inventory::InventoryWindowState;

/// The board. Carries [`RelativeCursorPosition`] so `drag::track_cursor` can
/// read the cursor over it every frame, even while a dragged item node sits
/// on top of the cursor — `ui_focus_system` writes that component for every
/// node that has it, regardless of what's stacked above.
#[derive(Component)]
pub struct InventoryBoard;

/// The full-screen overlay everything else hangs off.
/// [`sync_window_visibility`] is the only thing that touches it directly;
/// reparent it (it's a normal entity) if you'd rather the inventory sat
/// inside your own layout.
#[derive(Component)]
pub struct InventoryRoot;

/// One static background tile, at board coordinate `.0`.
#[derive(Component)]
pub struct InventoryCell(pub UVec2);

#[derive(Component)]
pub struct InventoryDescriptionSwatch;

#[derive(Component)]
pub struct InventoryDescriptionText;

/// Sibling stacking order — `ZIndex`, not `GlobalZIndex`, because these only
/// need to out-rank other item nodes under the same board, not escape it.
pub const Z_ITEM_IDLE: i32 = 0;
pub const Z_ITEM_DRAGGED: i32 = 1;

/// The entities [`setup_inventory_ui`] spawned. Inserted by that system, not
/// by [`InventoryPlugin::build`](super::InventoryPlugin) — it doesn't exist
/// until `InventorySet::Setup` has run, so order anything that reads it
/// `.after(InventorySet::Setup)`. That ordering also gets you the command
/// flush that makes the resource visible.
///
/// With more than one camera in your app, insert `UiTargetCamera` on `root`.
#[derive(Resource, Clone, Copy, Debug)]
pub struct InventoryUi {
    pub root: Entity,
    pub board: Entity,
    pub description_panel: Option<Entity>,
}

/// Builds the root overlay, the board, its empty cell tiles, and the
/// description panel. Spawns no camera — that's the host's job, since a
/// library plugin can't assume it's the only thing on screen.
pub fn setup_inventory_ui(mut commands: Commands, config: Res<InventoryConfig>, theme: Res<InventoryTheme>) {
    let root = commands
        .spawn((
            InventoryRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: Val::Px(theme.panel_margin_px),
                padding: UiRect::all(Val::Px(theme.panel_margin_px)),
                ..default()
            },
        ))
        .id();

    let board = spawn_board_panel(&mut commands, root, &config, &theme);
    let description_panel =
        config.show_description_panel.then(|| spawn_description_panel(&mut commands, root, &theme));

    commands.insert_resource(InventoryUi { root, board, description_panel });
}

fn spawn_board_panel(commands: &mut Commands, parent: Entity, config: &InventoryConfig, theme: &InventoryTheme) -> Entity {
    let column = commands
        .spawn((
            Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(8.0), ..default() },
            ChildOf(parent),
        ))
        .id();

    if let Some(title) = config.title {
        commands.spawn((
            Text::new(title),
            TextFont { font_size: theme.title_font_size, ..default() },
            TextColor(theme.panel_text),
            ChildOf(column),
        ));
    }

    let size = config.board_size();
    let board = commands
        .spawn((
            InventoryBoard,
            RelativeCursorPosition::default(),
            Node { width: Val::Px(size.x), height: Val::Px(size.y), ..default() },
            BackgroundColor(theme.panel_background),
            ChildOf(column),
        ))
        .id();

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
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(theme.cell_background),
                BorderColor::all(theme.cell_border),
                ChildOf(board),
            ));
        }
    }

    board
}

fn spawn_description_panel(commands: &mut Commands, parent: Entity, theme: &InventoryTheme) -> Entity {
    let panel = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                width: Val::Px(theme.description_panel_width_px),
                padding: UiRect::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(theme.panel_background),
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        InventoryDescriptionSwatch,
        Node {
            width: Val::Px(theme.description_swatch_px),
            height: Val::Px(theme.description_swatch_px),
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        BorderColor::all(theme.cell_border),
        ChildOf(panel),
    ));

    commands.spawn((
        InventoryDescriptionText,
        Text::new(theme.description_placeholder),
        TextFont { font_size: theme.description_font_size, ..default() },
        TextColor(theme.panel_text),
        ChildOf(panel),
    ));

    panel
}

/// Mirrors [`InventoryWindowState`] onto the root's `Visibility` *and*
/// `Node.display`, every frame, writing only on an actual change.
///
/// Both, not one: `Visibility` is the honest "don't render" switch, but only
/// `display` takes the overlay out of layout — which matters the moment
/// somebody reparents the root into their own flex column, where a merely
/// invisible full-screen node would still leave a hole. Neither blocks
/// clicks to whatever's underneath: `ui_focus_system` force-resets
/// `Interaction` on an invisible node, and a `Display::None` node has a zero
/// rect either way.
///
/// A mirror rather than `OnEnter`/`OnExit` because the very first state
/// transition runs *before* `PreStartup` — a transition hook would fire
/// before this root exists, and an app configured to start closed would
/// open anyway.
pub fn sync_window_visibility(
    state: Res<State<InventoryWindowState>>,
    root: Option<Single<(&mut Node, &mut Visibility), With<InventoryRoot>>>,
) {
    let Some(root) = root else { return };
    let (mut node, mut visibility) = root.into_inner();

    let open = *state.get() == InventoryWindowState::Open;
    let wanted_display = if open { Display::Flex } else { Display::None };
    let wanted_visibility = if open { Visibility::Visible } else { Visibility::Hidden };

    if node.display != wanted_display {
        node.display = wanted_display;
    }
    if *visibility != wanted_visibility {
        *visibility = wanted_visibility;
    }
}

/// Keeps an item's `Node` in step with its [`InventorySlot`] — except while
/// it's being dragged, when `drag` drives its position directly from the
/// cursor instead (that's how the grabbed cell stays glued to the cursor).
pub fn sync_item_position(
    drag: Res<InventoryDragState>,
    config: Res<InventoryConfig>,
    mut items: Query<(Entity, &InventorySlot, &InventoryItem, &mut Node), Changed<InventorySlot>>,
) {
    let held = drag.held_item();
    for (entity, slot, item, mut node) in &mut items {
        if Some(entity) == held {
            continue;
        }
        node.left = Val::Px(slot.0.x as f32 * config.cell_px + config.item_inset_px);
        node.top = Val::Px(slot.0.y as f32 * config.cell_px + config.item_inset_px);
        node.width = Val::Px(item.size.x as f32 * config.cell_px - 2.0 * config.item_inset_px);
        node.height = Val::Px(item.size.y as f32 * config.cell_px - 2.0 * config.item_inset_px);
    }
}

/// Colored border on the selected item, cleared everywhere else.
///
/// The `is_changed()` guard below is tick-based against this system's own
/// last run — so being gated off while the window is closed doesn't lose the
/// change, it correctly fires again on the very frame the window reopens.
pub fn sync_selection_outline(
    selection: Res<InventorySelection>,
    theme: Res<InventoryTheme>,
    mut items: Query<(Entity, &mut BorderColor), With<InventoryItem>>,
) {
    if !selection.is_changed() {
        return;
    }
    for (entity, mut border) in &mut items {
        let wanted = if selection.0 == Some(entity) { theme.selected_border } else { Color::NONE };
        if border.top != wanted {
            *border = BorderColor::all(wanted);
        }
    }
}

/// Fills the description panel from [`InventorySelection`], or shows the
/// placeholder when nothing is selected. A no-op if
/// [`InventoryConfig::show_description_panel`] was `false` at setup.
pub fn sync_description(
    selection: Res<InventorySelection>,
    theme: Res<InventoryTheme>,
    items: Query<&InventoryItem>,
    swatch: Option<Single<&mut BackgroundColor, With<InventoryDescriptionSwatch>>>,
    text: Option<Single<&mut Text, With<InventoryDescriptionText>>>,
) {
    if !selection.is_changed() {
        return;
    }
    let (Some(swatch), Some(text)) = (swatch, text) else { return };
    let mut swatch = swatch.into_inner();
    let mut text = text.into_inner();

    match selection.0.and_then(|e| items.get(e).ok()) {
        Some(item) => {
            swatch.0 = item.color;
            text.0 = format!("{}\n{}x{}\n\n{}", item.name, item.size.x, item.size.y, item.description);
        }
        None => {
            swatch.0 = Color::NONE;
            text.0 = theme.description_placeholder.to_string();
        }
    }
}

/// Recomputed every frame while a drag is live: paints the held item's
/// would-be footprint green (fits) or red (doesn't), everything else back to
/// the plain cell background.
pub fn sync_placement_preview(
    drag: Res<InventoryDragState>,
    cursor: Res<InventoryCursor>,
    grid: Res<InventoryGrid>,
    config: Res<InventoryConfig>,
    theme: Res<InventoryTheme>,
    items: Query<&InventoryItem>,
    mut cells: Query<(&InventoryCell, &mut BackgroundColor)>,
) {
    let preview = drag.preview(&cursor, &grid, &config, &items);
    for (InventoryCell(cell), mut background) in &mut cells {
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
