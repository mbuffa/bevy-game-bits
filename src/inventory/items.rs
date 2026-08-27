//! What an item is, and the two operations that must touch its node and the
//! occupancy grid together.

use bevy::prelude::*;

use crate::inventory::config::{InventoryConfig, InventoryTheme};
use crate::inventory::drag::InventorySelection;
use crate::inventory::grid::InventoryGrid;
use crate::inventory::ui::Z_ITEM_IDLE;

/// An item's fixed facts: label, flavor text, footprint in cells, and the
/// color standing in for its icon. Never mutated after spawn.
///
/// `&'static str` and `Copy`, so a game can keep its catalogue in a plain
/// `const` array with no startup cost — the library never sees that
/// catalogue, it only ever sees the items you hand [`spawn_item`].
///
/// For a real icon, insert an `ImageNode` on the entity `spawn_item` returns
/// and set [`InventoryConfig::show_item_labels`] to `false`.
#[derive(Component, Clone, Copy, Debug)]
pub struct InventoryItem {
    pub name: &'static str,
    pub description: &'static str,
    /// Footprint in cells, (width, height). Always axis-aligned — this
    /// module doesn't support rotation.
    pub size: UVec2,
    pub color: Color,
}

/// The item's current top-left cell. [`InventoryGrid`] is the occupancy
/// index of the same fact, and the two are always written together —
/// writing this alone is how you get an item that's drawn but can't be
/// picked up.
#[derive(Component)]
pub struct InventorySlot(pub UVec2);

/// Spawn an item onto the board and register it in the grid, in one call.
///
/// This deliberately isn't the `<thing>_bundle() -> impl Bundle` shape the
/// rest of this repo uses (`chassis_bundle`, `wheel_bundle`). Those return a
/// bundle because the caller has more to add afterward — a mesh, a
/// transform, wheel children whose mounts vary per car. Here there's nothing
/// left for the caller to supply, and the second half of the job isn't a
/// component at all: the grid has to be told too. Splitting "build the node"
/// from "remember to call `grid.place`" is exactly how this board once ended
/// up fully drawn and entirely un-clickable — every item was there on
/// screen, but the occupancy grid had never heard of any of them. One call,
/// or nothing.
///
/// Returns `None` (and logs a warning) if `origin` is off the board or
/// already occupied, so a bad catalogue entry can't leave the grid
/// disagreeing with the screen.
///
/// ```ignore
/// fn spawn_loadout(
///     mut commands: Commands,
///     ui: Res<InventoryUi>,
///     mut grid: ResMut<InventoryGrid>,
///     config: Res<InventoryConfig>,
///     theme: Res<InventoryTheme>,
/// ) {
///     for (item, origin) in STARTING_LOADOUT {
///         spawn_item(&mut commands, ui.board, &mut grid, &config, &theme, *item, *origin);
///     }
/// }
/// ```
pub fn spawn_item(
    commands: &mut Commands,
    board: Entity,
    grid: &mut InventoryGrid,
    config: &InventoryConfig,
    theme: &InventoryTheme,
    item: InventoryItem,
    origin: UVec2,
) -> Option<Entity> {
    if !grid.fits(origin.as_ivec2(), item.size, None) {
        warn!(
            "inventory: \"{}\" at ({}, {}) doesn't fit (off the board, or already occupied) — not spawned",
            item.name, origin.x, origin.y
        );
        return None;
    }

    let mut entity_commands = commands.spawn((
        item,
        InventorySlot(origin),
        ZIndex(Z_ITEM_IDLE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(origin.x as f32 * config.cell_px + config.item_inset_px),
            top: Val::Px(origin.y as f32 * config.cell_px + config.item_inset_px),
            width: Val::Px(item.size.x as f32 * config.cell_px - 2.0 * config.item_inset_px),
            height: Val::Px(item.size.y as f32 * config.cell_px - 2.0 * config.item_inset_px),
            border: UiRect::all(Val::Px(config.item_border_px)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::all(Val::Px(2.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(item.color),
        BorderColor::all(Color::NONE),
        ChildOf(board),
    ));

    if config.show_item_labels {
        entity_commands.with_child((
            Text::new(item.name),
            TextFont { font_size: theme.label_font_size(config), ..default() },
            TextColor(theme.item_label_color),
            TextLayout::new_with_justify(Justify::Center),
        ));
    }

    let entity = entity_commands.id();
    grid.place(entity, origin, item.size);
    Some(entity)
}

/// The mirror of [`spawn_item`], and the more dangerous half to get wrong: a
/// despawned node whose cells were never freed leaves a ghost occupant that
/// blocks placement forever and can't be selected to diagnose, since nothing
/// is drawn there to click on anymore.
///
/// Clears [`InventorySelection`] if this was the selected item.
pub fn despawn_item(commands: &mut Commands, grid: &mut InventoryGrid, selection: &mut InventorySelection, item: Entity) {
    grid.clear(item);
    if selection.0 == Some(item) {
        selection.0 = None;
    }
    commands.entity(item).despawn();
}
