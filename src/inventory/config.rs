//! What the board is built out of (`InventoryConfig`) and what it looks like
//! (`InventoryTheme`).
//!
//! The split is deliberate: `InventoryConfig` is the dependency of the
//! placement *model* — `grid.rs`'s pure functions, hit-testing, every unit
//! test — while `InventoryTheme` is the dependency of the *view*. A re-skin
//! that only touches `InventoryTheme` provably cannot move an item, and
//! `grid.rs`'s tests never have to name a colour.

use bevy::prelude::*;

/// The numbers the inventory *model* is built out of: how big the board is,
/// and where the line between a click and a drag falls.
///
/// Read once at plugin build (to size [`InventoryGrid`](super::InventoryGrid))
/// and again at every [`spawn_item`](super::spawn_item) call. Mutating the
/// resource at runtime does not resize an existing board. Override fields via
/// [`InventoryPlugin::config`](super::InventoryPlugin::config).
#[derive(Resource, Clone, Copy, Debug)]
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
    /// Draw each item's name inside its tile. Turn this off if you're going
    /// to insert your own `ImageNode` on the entity [`spawn_item`](super::spawn_item)
    /// returns.
    pub show_item_labels: bool,
    /// Spawn the description panel beside the board.
    pub show_description_panel: bool,
    /// Heading above the board. `None` spawns no heading row at all.
    pub title: Option<&'static str>,
}

impl InventoryConfig {
    /// The board's pixel size — what `track_cursor` un-normalizes
    /// `RelativeCursorPosition` against, and what `setup_inventory_ui` sizes
    /// the board node to. One derivation, so those two can't disagree.
    pub fn board_size(&self) -> Vec2 {
        Vec2::new(self.cols as f32 * self.cell_px, self.rows as f32 * self.cell_px)
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
            show_item_labels: true,
            show_description_panel: true,
            title: Some("INVENTORY"),
        }
    }
}

/// Every colour and type size the board is drawn with. Nothing in here is
/// read by the placement model — changing it cannot move an item. Override
/// fields via [`InventoryPlugin::theme`](super::InventoryPlugin::theme).
#[derive(Resource, Clone, Copy, Debug)]
pub struct InventoryTheme {
    pub panel_background: Color,
    pub panel_text: Color,
    pub panel_margin_px: f32,
    pub cell_background: Color,
    pub cell_border: Color,
    /// Placement preview on cells the held item would legally land on.
    pub preview_ok: Color,
    /// ...and on cells it wouldn't.
    pub preview_bad: Color,
    pub selected_border: Color,
    pub item_label_color: Color,
    /// `None` derives the label size from [`InventoryConfig::cell_px`] — see
    /// [`InventoryTheme::label_font_size`].
    pub item_label_font_size: Option<f32>,
    pub title_font_size: f32,
    pub description_font_size: f32,
    pub description_panel_width_px: f32,
    pub description_swatch_px: f32,
    /// Shown when nothing is selected.
    pub description_placeholder: &'static str,
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
            panel_margin_px: 16.0,
            cell_background: Color::srgba(1.0, 1.0, 1.0, 0.06),
            cell_border: Color::srgba(1.0, 1.0, 1.0, 0.25),
            preview_ok: Color::srgba(0.2, 0.9, 0.3, 0.45),
            preview_bad: Color::srgba(0.9, 0.2, 0.2, 0.45),
            selected_border: Color::srgb(1.0, 0.85, 0.2),
            item_label_color: Color::BLACK,
            item_label_font_size: None,
            title_font_size: 20.0,
            description_font_size: 14.0,
            description_panel_width_px: 280.0,
            description_swatch_px: 48.0,
            description_placeholder: "Click an item to inspect it.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_size_multiplies_out() {
        let config = InventoryConfig { cols: 7, rows: 8, cell_px: 64.0, ..default() };
        assert_eq!(config.board_size(), Vec2::new(448.0, 512.0));

        let config = InventoryConfig { cols: 3, rows: 5, cell_px: 32.0, ..default() };
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
        let theme = InventoryTheme { item_label_font_size: Some(20.0), ..default() };
        assert_eq!(theme.label_font_size(&config), 20.0);
    }
}
