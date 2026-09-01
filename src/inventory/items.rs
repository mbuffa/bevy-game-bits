//! What an item is, and the two operations that must touch its node and the
//! occupancy grid together.

use std::borrow::Cow;

use bevy::prelude::*;

use crate::inventory::config::{InventoryConfig, InventoryTheme};
use crate::inventory::drag::InventorySelection;
use crate::inventory::grid::InventoryGrid;
use crate::inventory::ui::{place_node, Z_ITEM_IDLE};

/// An item's fixed facts: label, flavor text, footprint in cells, the color
/// standing in for its icon, and an optional real icon. Never mutated after
/// spawn.
///
/// `Cow<'static, str>` for the text, so a compile-time catalogue stays free
/// (build one with [`InventoryItem::borrowed`] in a `const`) while a
/// data-driven or localized one still works (build it with
/// [`InventoryItem::new`]). `Clone`, not `Copy`.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct InventoryItem {
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    /// Footprint in cells, (width, height). Always axis-aligned — this
    /// module doesn't support rotation.
    pub size: UVec2,
    /// The tile's fill color, and the description-panel swatch. Shows through
    /// wherever [`icon`](Self::icon) doesn't cover.
    pub color: Color,
    /// A real icon drawn inside the tile. `None` falls back to the flat
    /// [`color`](Self::color) plus (if [`InventoryConfig::show_item_labels`])
    /// the name.
    pub icon: Option<Handle<Image>>,
}

impl InventoryItem {
    /// A runtime-built item — text from anywhere that is `Into<Cow<'static, str>>`
    /// (a `String`, a `&'static str`, a localized lookup).
    pub fn new(
        name: impl Into<Cow<'static, str>>,
        description: impl Into<Cow<'static, str>>,
        size: UVec2,
        color: Color,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            size,
            color,
            icon: None,
        }
    }

    /// A compile-time item, usable in a `const` catalogue.
    pub const fn borrowed(
        name: &'static str,
        description: &'static str,
        size: UVec2,
        color: Color,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            description: Cow::Borrowed(description),
            size,
            color,
            icon: None,
        }
    }

    /// Builder: attach a real icon.
    pub fn with_icon(mut self, icon: Handle<Image>) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// The item's current top-left cell. [`InventoryGrid`] is the occupancy
/// index of the same fact, and the two are always written together —
/// writing this alone is how you get an item that's drawn but can't be
/// picked up.
#[derive(Component)]
pub struct InventorySlot(pub UVec2);

/// Spawn an item onto `board` and register it in that board's `grid`, in one
/// call.
///
/// This deliberately isn't the `<thing>_bundle() -> impl Bundle` shape the
/// rest of this repo uses (`chassis_bundle`, `wheel_bundle`). Those return a
/// bundle because the caller has more to add afterward. Here there's nothing
/// left for the caller to supply, and the second half of the job isn't a
/// component at all: the grid has to be told too. Splitting "build the node"
/// from "remember to call `grid.place`" is exactly how this board once ended
/// up fully drawn and entirely un-clickable. One call, or nothing.
///
/// Returns `None` (and logs a warning) if `origin` is off the board or
/// already occupied. Most callers want [`InventoryCommands`](super::InventoryCommands)
/// instead — it picks the origin and fires an [`InventoryAction`](super::InventoryAction).
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

    let size = item.size;
    let color = item.color;
    let label = item.name.clone();
    let icon = item.icon.clone();
    let show_label = config.show_item_labels;
    let label_font_size = theme.label_font_size(config);
    let label_color = theme.item_label_color;

    let mut node = Node {
        position_type: PositionType::Absolute,
        border: UiRect::all(Val::Px(config.item_border_px)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        padding: UiRect::all(Val::Px(theme.item_padding_px)),
        overflow: Overflow::clip(),
        ..default()
    };
    place_node(&mut node, origin, size, config);

    let mut entity_commands = commands.spawn((
        item,
        InventorySlot(origin),
        ZIndex(Z_ITEM_IDLE),
        node,
        BackgroundColor(color),
        BorderColor::all(Color::NONE),
        ChildOf(board),
    ));

    if show_label {
        entity_commands.with_child((
            Text::new(label),
            TextFont {
                font_size: label_font_size,
                ..default()
            },
            TextColor(label_color),
            TextLayout::new_with_justify(Justify::Center),
        ));
    }

    if let Some(icon) = icon {
        entity_commands.with_child((
            ImageNode::new(icon),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ));
    }

    let entity = entity_commands.id();
    grid.place(entity, origin, size);
    Some(entity)
}

/// The mirror of [`spawn_item`], and the more dangerous half to get wrong: a
/// despawned node whose cells were never freed leaves a ghost occupant that
/// blocks placement forever and can't be selected to diagnose.
///
/// Clears `selection` if this was the selected item — pass the board's
/// [`InventorySelection`] component.
pub fn despawn_item(
    commands: &mut Commands,
    grid: &mut InventoryGrid,
    selection: &mut InventorySelection,
    item: Entity,
) {
    grid.clear(item);
    if selection.0 == Some(item) {
        selection.0 = None;
    }
    commands.entity(item).despawn();
}
