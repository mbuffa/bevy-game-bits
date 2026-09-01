//! What one board is built out of (`InventoryConfig`), what it looks like
//! (`InventoryTheme`), where it sits on screen (`InventoryLayout`), and the
//! [`InventoryBoardSpec`] that bundles all three plus an initial open/closed
//! state into the one argument [`spawn_inventory`](super::spawn_inventory)
//! takes.
//!
//! All three are **components** on the board entity, not resources — that's
//! what lets one app hold several independent boards (a player bag and a loot
//! stash) at once. [`InventoryPlugin`](super::InventoryPlugin) copies them out
//! of its [`InventoryBoardSpec`] field onto the board it spawns; a host that
//! spawns its own boards hands a spec straight to
//! [`spawn_inventory`](super::spawn_inventory).
//!
//! The `Config` / `Theme` split is deliberate: `InventoryConfig` is the
//! dependency of the placement *model* — `grid.rs`'s pure functions,
//! hit-testing, every unit test — while `InventoryTheme` is the dependency of
//! the *view*. A re-skin that only touches `InventoryTheme` provably cannot
//! move an item, and `grid.rs`'s tests never have to name a colour.

use std::borrow::Cow;

use bevy::prelude::*;

use super::{InventoryAccess, InventoryTransferTarget, InventoryWindow};

/// The numbers one board's *model* is built out of: how big it is, and where
/// the line between a click and a drag falls.
///
/// A `Component` on the board entity. Read at [`spawn_inventory`](super::spawn_inventory)
/// (to size [`InventoryGrid`](super::InventoryGrid) and the board node) and
/// again at every [`spawn_item`](super::spawn_item) call. `cols`/`rows` are
/// kept in step with the grid by
/// [`InventoryCommands::resize`](super::InventoryCommands::resize); editing
/// them by hand does **not** resize the board.
#[derive(Component, Clone, Debug)]
pub struct InventoryConfig {
    /// Board width in cells.
    pub cols: u32,
    /// Board height in cells.
    pub rows: u32,
    /// One cell's edge length in logical pixels.
    pub cell_px: f32,
    /// Item tiles are drawn `cell_px - 2 * item_inset_px` across, so
    /// neighbouring items always show a sliver of cell border between them.
    pub item_inset_px: f32,
    /// Border width on an item tile — what the selection outline is drawn in.
    pub item_border_px: f32,
    /// A press that never moves the cursor past this many pixels is a click,
    /// not a drag. Without it, an in-place click starts (and immediately
    /// cancels) a drag every time.
    pub drag_threshold_px: f32,
    /// Two non-drag presses on the same item within this many seconds is a
    /// double-click: it fires [`InventoryAction::Activated`](super::InventoryAction::Activated)
    /// and, if [`InventoryTransferTarget`] names another board, a
    /// [`quick_transfer`](super::quick_transfer) to it. `None` turns
    /// double-click detection off entirely (no `Activated`, no quick
    /// transfer) — the two presses are just two ordinary clicks.
    pub double_click_secs: Option<f32>,
    /// Draw each item's name inside its tile. Turn this off if you're going
    /// to insert your own `ImageNode` on the entity [`spawn_item`](super::spawn_item)
    /// returns — or just set [`InventoryItem::icon`](super::InventoryItem::icon).
    pub show_item_labels: bool,
    /// Spawn the description panel beside the board.
    pub show_description_panel: bool,
    /// Heading above the board. `None` spawns no heading row at all.
    pub title: Option<Cow<'static, str>>,
}

impl InventoryConfig {
    /// The board's pixel size — what `track_cursor` un-normalizes
    /// `RelativeCursorPosition` against, and what `spawn_inventory` sizes the
    /// board node to. One derivation, so those two can't disagree.
    pub fn board_size(&self) -> Vec2 {
        Vec2::new(
            self.cols as f32 * self.cell_px,
            self.rows as f32 * self.cell_px,
        )
    }
}

impl Default for InventoryConfig {
    fn default() -> Self {
        Self {
            cols: 7,
            rows: 8,
            cell_px: 64.0,
            item_inset_px: 3.0,
            item_border_px: 2.0,
            drag_threshold_px: 4.0,
            double_click_secs: Some(0.35),
            show_item_labels: true,
            show_description_panel: true,
            title: Some(Cow::Borrowed("INVENTORY")),
        }
    }
}

/// Every colour, type size, and remaining pixel constant one board is drawn
/// with. A `Component` on the board entity. Nothing in here is read by the
/// placement model — changing it cannot move an item.
#[derive(Component, Clone, Debug)]
pub struct InventoryTheme {
    pub panel_background: Color,
    pub panel_text: Color,
    pub cell_background: Color,
    pub cell_border: Color,
    /// Border width on an empty cell tile, in logical pixels.
    pub cell_border_px: f32,
    /// Placement preview on cells the held item would legally land on.
    pub preview_ok: Color,
    /// ...and on cells it wouldn't.
    pub preview_bad: Color,
    pub selected_border: Color,
    /// `GlobalZIndex` a held item is lifted to for the length of a drag, so it
    /// rides above *other* boards' whole subtrees and any host HUD between
    /// them — a plain sibling `ZIndex` only clears its own board. Reset when
    /// the drag ends.
    pub dragged_global_z: i32,
    pub item_label_color: Color,
    /// `None` derives the label size from [`InventoryConfig::cell_px`] — see
    /// [`InventoryTheme::label_font_size`].
    pub item_label_font_size: Option<f32>,
    /// Padding inside an item tile, between its border and its label/icon.
    pub item_padding_px: f32,
    pub title_font_size: f32,
    /// Gap between the title row and the board.
    pub board_row_gap_px: f32,
    pub description_font_size: f32,
    pub description_panel_width_px: f32,
    /// Padding inside the description panel.
    pub description_panel_padding_px: f32,
    /// Gap between the swatch and the text inside the description panel.
    pub description_panel_row_gap_px: f32,
    pub description_swatch_px: f32,
    /// Border width on the description-panel colour swatch.
    pub description_swatch_border_px: f32,
    /// Shown when nothing is selected.
    pub description_placeholder: Cow<'static, str>,
}

impl InventoryTheme {
    /// Item labels have to fit inside a one-cell-wide tile, and text can't
    /// break mid-word — so the usable size is a fraction of the cell, not a
    /// constant. Derived from `cell_px` by default precisely so shrinking the
    /// board doesn't silently start overflowing every label.
    pub fn label_font_size(&self, config: &InventoryConfig) -> f32 {
        self.item_label_font_size.unwrap_or(config.cell_px * 0.14)
    }
}

impl Default for InventoryTheme {
    fn default() -> Self {
        Self {
            panel_background: Color::srgba(0.0, 0.0, 0.0, 0.6),
            panel_text: Color::srgb(0.92, 0.89, 0.84),
            cell_background: Color::srgba(1.0, 1.0, 1.0, 0.06),
            cell_border: Color::srgba(1.0, 1.0, 1.0, 0.25),
            cell_border_px: 1.0,
            preview_ok: Color::srgba(0.2, 0.9, 0.3, 0.45),
            preview_bad: Color::srgba(0.9, 0.2, 0.2, 0.45),
            selected_border: Color::srgb(1.0, 0.85, 0.2),
            dragged_global_z: 1000,
            item_label_color: Color::BLACK,
            item_label_font_size: None,
            item_padding_px: 2.0,
            title_font_size: 20.0,
            board_row_gap_px: 8.0,
            description_font_size: 14.0,
            description_panel_width_px: 280.0,
            description_panel_padding_px: 14.0,
            description_panel_row_gap_px: 10.0,
            description_swatch_px: 48.0,
            description_swatch_border_px: 2.0,
            description_placeholder: Cow::Borrowed("Click an item to inspect it."),
        }
    }
}

/// Which side of the board the description panel sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelSide {
    Left,
    Right,
    Above,
    Below,
}

impl PanelSide {
    /// The overlay's main axis: `Row` for a panel beside the board, `Column`
    /// for one above or below. Never a `*Reverse` — the panel lands on the
    /// right side by [`spawn_inventory`](super::spawn_inventory) spawning it
    /// before or after the board (see [`panel_first`](Self::panel_first)), so
    /// [`InventoryLayout::horizontal`]/[`vertical`](InventoryLayout::vertical)
    /// keep their plain meaning.
    pub fn flex_direction(self) -> FlexDirection {
        match self {
            PanelSide::Left | PanelSide::Right => FlexDirection::Row,
            PanelSide::Above | PanelSide::Below => FlexDirection::Column,
        }
    }

    /// Whether the panel node is spawned before the board (so it ends up on
    /// the left / above) rather than after it.
    pub fn panel_first(self) -> bool {
        matches!(self, PanelSide::Left | PanelSide::Above)
    }
}

/// Where one board's board+panel block sits on the full-screen overlay, and
/// which side the panel takes. A `Component` on the board entity, read once at
/// [`spawn_inventory`](super::spawn_inventory) — re-anchoring at runtime means
/// respawning the board.
#[derive(Component, Clone, Copy, Debug)]
pub struct InventoryLayout {
    /// `justify_content` on the overlay — horizontal placement of the block.
    pub horizontal: JustifyContent,
    /// `align_items` on the overlay — vertical placement of the block.
    pub vertical: AlignItems,
    /// Which side of the board the description panel sits on.
    pub panel_side: PanelSide,
    /// Padding between the block and the screen edge, in logical pixels.
    pub root_padding_px: f32,
    /// Gap between the board and the description panel, in logical pixels.
    pub panel_gap_px: f32,
}

impl Default for InventoryLayout {
    fn default() -> Self {
        Self {
            horizontal: JustifyContent::FlexStart,
            vertical: AlignItems::FlexStart,
            panel_side: PanelSide::Right,
            root_padding_px: 16.0,
            panel_gap_px: 16.0,
        }
    }
}

/// Everything [`spawn_inventory`](super::spawn_inventory) needs to build one
/// board: its model numbers, its skin, its screen anchor, and whether it
/// starts open. `Default` gives the same 7x8 top-left board
/// `InventoryPlugin::default()` has always produced.
///
/// ```ignore
/// let bag = spawn_inventory(&mut commands, InventoryBoardSpec {
///     config: InventoryConfig { cols: 6, rows: 10, ..default() },
///     window: InventoryWindow { open: false },
///     ..default()
/// });
/// ```
#[derive(Clone, Default)]
pub struct InventoryBoardSpec {
    pub config: InventoryConfig,
    pub theme: InventoryTheme,
    pub layout: InventoryLayout,
    pub window: InventoryWindow,
    /// The host's veto on dragging (`Default` allows everything). Usually
    /// rewritten every frame once the board is up — see [`InventoryAccess`].
    pub access: InventoryAccess,
    /// The board a double-click here sends items to (`Default` is `None` —
    /// no quick transfer). Usually rewritten every frame alongside `access` —
    /// see [`InventoryTransferTarget`].
    pub transfer_target: InventoryTransferTarget,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_size_multiplies_out() {
        let config = InventoryConfig {
            cols: 7,
            rows: 8,
            cell_px: 64.0,
            ..default()
        };
        assert_eq!(config.board_size(), Vec2::new(448.0, 512.0));

        let config = InventoryConfig {
            cols: 3,
            rows: 5,
            cell_px: 32.0,
            ..default()
        };
        assert_eq!(config.board_size(), Vec2::new(96.0, 160.0));
    }

    #[test]
    fn derived_label_size_matches_the_hand_tuned_value_at_the_default_cell() {
        let config = InventoryConfig::default();
        let theme = InventoryTheme::default();
        // The example this was extracted from hand-tuned 9.0 at cell_px 64.0;
        // the derivation should land close enough to be indistinguishable.
        assert!((theme.label_font_size(&config) - 9.0).abs() < 0.5);
    }

    #[test]
    fn an_explicit_label_size_overrides_the_derivation() {
        let config = InventoryConfig::default();
        let theme = InventoryTheme {
            item_label_font_size: Some(20.0),
            ..default()
        };
        assert_eq!(theme.label_font_size(&config), 20.0);
    }

    #[test]
    fn panel_side_maps_to_axis_and_order() {
        assert_eq!(PanelSide::Right.flex_direction(), FlexDirection::Row);
        assert_eq!(PanelSide::Left.flex_direction(), FlexDirection::Row);
        assert_eq!(PanelSide::Below.flex_direction(), FlexDirection::Column);
        assert_eq!(PanelSide::Above.flex_direction(), FlexDirection::Column);

        assert!(PanelSide::Left.panel_first());
        assert!(PanelSide::Above.panel_first());
        assert!(!PanelSide::Right.panel_first());
        assert!(!PanelSide::Below.panel_first());
    }
}
