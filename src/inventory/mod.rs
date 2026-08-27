//! A tetris-style grid inventory: a board of large cells holding
//! axis-aligned rectangular items, click-to-select with a description panel,
//! and hold-to-drag/release-to-drop with a live placement preview.
//!
//! # Getting a board on screen
//!
//! ```ignore
//! use bevy_game_bits::inventory::prelude::*;
//!
//! app.add_plugins(InventoryPlugin::default())
//!     .add_systems(Startup, (spawn_camera, spawn_loadout.after(InventorySet::Setup)));
//!
//! const LOADOUT: &[(InventoryItem, UVec2)] = &[
//!     (InventoryItem { name: "Pistol", description: "...", size: UVec2::new(1, 1), color: Color::WHITE }, UVec2::new(0, 0)),
//! ];
//!
//! fn spawn_loadout(
//!     mut commands: Commands,
//!     ui: Res<InventoryUi>,
//!     mut grid: ResMut<InventoryGrid>,
//!     config: Res<InventoryConfig>,
//!     theme: Res<InventoryTheme>,
//! ) {
//!     for (item, origin) in LOADOUT {
//!         spawn_item(&mut commands, ui.board, &mut grid, &config, &theme, *item, *origin);
//!     }
//! }
//! ```
//!
//! # The pieces
//!
//! - [`config`] — [`InventoryConfig`] (board size and interaction
//!   thresholds, the model's dependency) and [`InventoryTheme`] (colors and
//!   type sizes, the view's dependency). Split so a re-skin provably can't
//!   move an item.
//! - [`grid`] — [`InventoryGrid`], the pure occupancy model, and its
//!   placement math (`hovered_cell`, `grab_offset`, `target_origin`). No
//!   Bevy scheduling in this file at all — it's plain data, cleanly
//!   unit-testable.
//! - [`items`] — [`InventoryItem`], [`InventorySlot`], and [`spawn_item`] /
//!   [`despawn_item`], which are the checked way to keep a node and its grid
//!   entry from disagreeing.
//! - [`drag`] — press-to-select / hold-to-drag / release-to-drop, and
//!   [`InventoryAction`], the message this module fires instead of playing a
//!   sound itself.
//! - [`ui`] — the board/panel layout and the systems that keep it in sync
//!   with the model.
//!
//! # Scheduling
//!
//! Order your own systems against [`InventorySet`] rather than against the
//! system functions directly, so a game keeps working if the internals are
//! re-split. Item-spawning belongs `.after(InventorySet::Setup)`.
//!
//! # Showing and hiding the window
//!
//! Unlike this repo's other library modules (`vehicle` deliberately never
//! defines or reads a `States` type — see `vehicle::input`'s module docs),
//! this one owns [`InventoryWindowState`]. Open/closed isn't a hosting
//! game's policy the way "is the player allowed to drive" is; it's part of
//! what an inventory *is*. Flip it with `NextState` from your own
//! keybinding — hiding the board, stopping [`InventorySet::Interaction`] and
//! [`InventorySet::Sync`], and cancelling a drag that was live at the moment
//! you closed it all follow automatically.
//!
//! # Limits
//!
//! `InventoryWindowState` is a global state, so there is exactly one
//! inventory window per app. Item text is `&'static str` — fine for a
//! compile-time catalogue, not for text loaded at runtime. There's no
//! rotation: an item's `size` is fixed for its lifetime. And this module
//! must be added after `DefaultPlugins` (or at least after
//! `bevy::state::app::StatesPlugin`) — `insert_state` panics if the
//! `StateTransition` schedule doesn't exist yet.

pub mod config;
pub mod drag;
pub mod grid;
pub mod items;
pub mod ui;

use bevy::prelude::*;

pub use config::{InventoryConfig, InventoryTheme};
pub use drag::{
    begin_drag, end_drag, reset_interaction, track_cursor, update_drag, InventoryAction, InventoryCursor,
    InventoryDragState, InventoryPreview, InventorySelection,
};
pub use grid::{grab_offset, hovered_cell, target_origin, InventoryGrid};
pub use items::{despawn_item, spawn_item, InventoryItem, InventorySlot};
pub use ui::{
    sync_description, sync_item_position, sync_placement_preview, sync_selection_outline, sync_window_visibility,
    InventoryBoard, InventoryCell, InventoryDescriptionSwatch, InventoryDescriptionText, InventoryRoot, InventoryUi,
    Z_ITEM_DRAGGED, Z_ITEM_IDLE,
};

/// Everything you need to build and drive an inventory, in one import.
pub mod prelude {
    pub use super::{
        despawn_item, grab_offset, hovered_cell, spawn_item, target_origin, InventoryAction, InventoryBoard,
        InventoryCell, InventoryConfig, InventoryCursor, InventoryDescriptionSwatch, InventoryDescriptionText,
        InventoryDragState, InventoryGrid, InventoryItem, InventoryPlugin, InventoryPreview, InventoryRoot,
        InventorySelection, InventorySet, InventorySlot, InventoryTheme, InventoryUi, InventoryWindowState,
    };
}

/// Ordering handles for the inventory systems. Order against these rather
/// than against the system functions, so a game keeps working if the
/// internals are re-split.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum InventorySet {
    /// `Startup`. [`ui::setup_inventory_ui`]. Order your own item-spawning
    /// `.after(InventorySet::Setup)` — that's what makes [`InventoryUi`]
    /// readable, and a name that survives internals being re-split.
    Setup,
    /// `Update`, before [`Interaction`](Self::Interaction). Deliberately
    /// *not* gated on [`InventoryWindowState`] — [`sync_window_visibility`]
    /// lives here, and it has to keep running while the window is closed in
    /// order to notice it being reopened.
    Window,
    /// `Update`, after [`Window`](Self::Window), gated on
    /// [`InventoryWindowState::Open`]. [`track_cursor`], [`begin_drag`],
    /// [`update_drag`], [`end_drag`], chained in that order.
    Interaction,
    /// `Update`, after [`Interaction`](Self::Interaction), gated on
    /// [`InventoryWindowState::Open`]. The four `sync_*` systems that push
    /// model state back out to `Node`s and colors.
    Sync,
}

/// Whether the inventory window is showing. See the module docs' "Showing
/// and hiding the window" section for why this module owns a `States` type
/// when the rest of this repo's library modules deliberately don't.
///
/// A global state, so there is exactly one inventory window per app.
#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum InventoryWindowState {
    #[default]
    Open,
    Closed,
}

/// Registers the whole inventory: board, drag-and-drop, description panel,
/// and the open/closed window state. Everything is configured through the
/// fields below — this module inserts no resource you're expected to
/// pre-supply.
///
/// Must be added after `DefaultPlugins` (or after
/// `bevy::state::app::StatesPlugin`) — see the module docs' "Limits"
/// section.
///
/// ```ignore
/// app.add_plugins(InventoryPlugin {
///     config: InventoryConfig { cols: 6, rows: 10, ..default() },
///     ..default()
/// });
/// ```
#[derive(Default)]
pub struct InventoryPlugin {
    /// Board dimensions and interaction thresholds. Read once, at build and
    /// at spawn — changing the resource at runtime will not resize the
    /// board.
    pub config: InventoryConfig,
    /// Colors and type sizes. Nothing in here can change behavior.
    pub theme: InventoryTheme,
    /// Whether the window starts open. Defaults to
    /// [`InventoryWindowState::Open`], so `InventoryPlugin::default()` puts
    /// something on screen immediately.
    pub window: InventoryWindowState,
}

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config)
            .insert_resource(self.theme)
            .insert_resource(InventoryGrid::new(self.config.cols, self.config.rows))
            .init_resource::<InventoryCursor>()
            .init_resource::<InventoryDragState>()
            .init_resource::<InventorySelection>()
            .add_message::<InventoryAction>()
            .insert_state(self.window)
            .configure_sets(Update, (InventorySet::Window, InventorySet::Interaction, InventorySet::Sync).chain())
            .configure_sets(Update, InventorySet::Interaction.run_if(in_state(InventoryWindowState::Open)))
            .configure_sets(Update, InventorySet::Sync.run_if(in_state(InventoryWindowState::Open)))
            .add_systems(Startup, ui::setup_inventory_ui.in_set(InventorySet::Setup))
            // Not `OnEnter`/`OnExit`: the first state transition runs before
            // `PreStartup`, so a transition hook would fire before the root
            // exists and an app configured to start closed would open
            // anyway. An idempotent mirror can't get that wrong.
            .add_systems(Update, ui::sync_window_visibility.in_set(InventorySet::Window))
            // These two *are* transitions: cancelling a live drag and
            // clearing a stale cursor are events, not state to mirror.
            .add_systems(OnEnter(InventoryWindowState::Open), drag::reset_interaction)
            .add_systems(OnExit(InventoryWindowState::Open), drag::reset_interaction)
            .add_systems(
                Update,
                (drag::track_cursor, drag::begin_drag, drag::update_drag, drag::end_drag)
                    .chain()
                    .in_set(InventorySet::Interaction),
            )
            .add_systems(
                Update,
                (
                    ui::sync_item_position,
                    ui::sync_selection_outline,
                    ui::sync_description,
                    ui::sync_placement_preview,
                )
                    .in_set(InventorySet::Sync),
            );
    }
}
