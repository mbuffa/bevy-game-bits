//! A tetris-style grid inventory: a board of large cells holding
//! axis-aligned rectangular items, click-to-select with a description panel,
//! and hold-to-drag/release-to-drop with a live placement preview.
//!
//! Everything about one board — its size, its skin, its layout, its
//! occupancy, its drag state, its open/closed flag — lives in **components on
//! the board entity**, so one app can hold several independent boards (a
//! player bag and a loot stash) at once.
//!
//! # Getting a board on screen
//!
//! The plugin spawns one board for you from its [`InventoryPlugin::board`]
//! field, and drops its entity into [`DefaultInventoryBoard`]:
//!
//! ```ignore
//! use bevy_game_bits::inventory::prelude::*;
//!
//! app.add_plugins(InventoryPlugin::default())
//!     .add_systems(Startup, (spawn_camera, spawn_loadout.after(InventorySet::Setup)));
//!
//! fn spawn_loadout(mut commands: Commands, board: Res<DefaultInventoryBoard>, mut inv: InventoryCommands) {
//!     inv.add(**board, InventoryItem::new("Pistol", "A worn sidearm.", UVec2::new(1, 1), Color::WHITE));
//! }
//! ```
//!
//! Or spawn boards yourself and skip the default one entirely:
//!
//! ```ignore
//! app.add_plugins(InventoryPlugin::headless());
//!
//! fn setup(mut commands: Commands) {
//!     let bag = spawn_inventory(&mut commands, InventoryBoardSpec::default());
//!     let stash = spawn_inventory(&mut commands, InventoryBoardSpec {
//!         layout: InventoryLayout { panel_side: PanelSide::Left, ..default() },
//!         window: InventoryWindow { open: false },
//!         ..default()
//!     });
//! }
//! ```
//!
//! # Adding, removing, and resizing at runtime
//!
//! [`InventoryCommands`] is the `SystemParam` for it: `inv.add(board, item)`
//! drops an item in the first free row-major spot (or fires
//! [`InventoryAction::AddRejected`] with the item back if it fits nowhere),
//! `inv.add_at` forces an origin, `inv.remove(item)` takes one out,
//! `inv.has_room_for` / `inv.free_cells` answer "can I even pick this up",
//! and `inv.resize(board, cols, rows)` re-shapes a board (applied at
//! [`InventorySet::Commands`]). `inv.transfer(item, board)` / `inv.transfer_at`
//! move an already-spawned item to another board without despawning it. For
//! systems that would rather not name a board type, the [`AddItem`] and
//! [`ResizeInventory`] messages do the same jobs.
//!
//! # The pieces
//!
//! - [`config`] — [`InventoryConfig`] (model numbers), [`InventoryTheme`]
//!   (skin), [`InventoryLayout`] (screen anchor + panel side), and
//!   [`InventoryBoardSpec`] bundling all three. Components on the board.
//! - [`grid`] — [`InventoryGrid`], the pure occupancy model, its placement
//!   math, and [`InventoryGrid::resize`]. No Bevy scheduling in this file.
//! - [`items`] — [`InventoryItem`], [`InventorySlot`], and [`spawn_item`] /
//!   [`despawn_item`], the checked way to keep a node and its grid entry from
//!   disagreeing.
//! - [`commands`] — [`InventoryCommands`] and the request messages.
//! - [`drag`] — press-to-select / hold-to-drag / release-to-drop, the
//!   double-click [`quick_transfer`], and [`InventoryAction`], the message
//!   this module fires instead of playing a sound itself.
//! - [`ui`] — [`spawn_inventory`], the board/panel layout, and the systems
//!   that keep it in sync with the model.
//!
//! # Scheduling
//!
//! Order your own systems against [`InventorySet`] rather than against the
//! system functions directly. Item-spawning through the `Startup` path
//! belongs `.after(InventorySet::Setup)`.
//!
//! # Showing and hiding a board
//!
//! This module owns [`InventoryWindow`] where the rest of this repo's library
//! modules deliberately don't touch state (`vehicle` never defines or reads a
//! `States` type). Open/closed isn't a hosting game's policy the way "is the
//! player allowed to drive" is — it's part of what an inventory *is*. It is a
//! **component**, not a `States` type, precisely because there can be more
//! than one board: a global state could not say "the bag is open and the
//! stash is closed". Flip `window.open` from your own keybinding — hiding the
//! board, stopping its interaction and sync work, and cancelling a drag that
//! was live at the moment you closed it all follow automatically.
//!
//! # Reach and transfers
//!
//! An item can be dragged from one board straight onto another — grid, slot,
//! `ChildOf` parentage, and selection all follow, and a
//! [`InventoryAction::Transferred`] reports it (plain in-board moves still
//! fire [`InventoryAction::Dropped`], unchanged). [`InventoryAccess`] is the
//! host's veto on that: a cross-board move needs `transfers` on **both**
//! boards, and a board with `interactive: false` is inert to the pointer
//! entirely. The worked case is a party game where each character's bag can
//! only exchange with a chest while that character stands next to it, but
//! every bag is always sortable on its own — which is why it's two bits, not
//! one. Write it every frame from whatever check you like; the module only
//! reads it.
//!
//! # Quick transfer
//!
//! A double-click also moves an item across — for when dragging across the
//! screen is more precision than the moment calls for. It always fires
//! [`InventoryAction::Activated`], the hook for "equip"/"use"/whatever else a
//! double-click should mean in your game. If that board's
//! [`InventoryTransferTarget`] names another *open* board that
//! [`InventoryAccess`] allows exchanging with and that has room, the item
//! lands there too ([`InventoryAction::Transferred`]; a named board with no
//! room instead fires [`InventoryAction::Rejected`]). Unlike `InventoryAccess`,
//! nothing about the destination is inferred — the library has no rule for
//! "closest board" or "first board found", because that's exactly the part a
//! host's reach check should own. `None` (the default) means a double-click
//! here only ever activates, never moves anything.
//!
//! # Limits
//!
//! A drag still starts and ends within one gesture — you can't park a half-
//! dragged item. [`InventoryLayout`] is read once at [`spawn_inventory`];
//! re-anchoring means respawning the board. Overlapping boards are the host's
//! problem: [`begin_drag`] consumes a press on the first board it finds the
//! cursor over and stops, and [`track_drag_target`] resolves a drop onto the
//! first board under the cursor.

pub mod commands;
pub mod config;
pub mod drag;
pub mod grid;
pub mod items;
pub mod quickbar;
pub mod ui;

use bevy::prelude::*;

pub use commands::{AddItem, InventoryCommands, ResizeInventory};
pub use config::{InventoryBoardSpec, InventoryConfig, InventoryLayout, InventoryTheme, PanelSide};
pub use drag::{
    begin_drag, end_drag, quick_transfer, reset_interaction, track_cursor, track_drag_target,
    update_drag, InventoryAction, InventoryClicks, InventoryCursor, InventoryDragState,
    InventoryPreview, InventorySelection,
};
pub use grid::{grab_offset, hovered_cell, target_origin, InventoryGrid, ResizeOutcome};
pub use items::{despawn_item, spawn_item, InventoryItem, InventorySlot};
pub use quickbar::{
    hovered_slot, ActiveSlot, Quickbar, QuickbarAction, QuickbarParts, QuickbarPlugin, QuickbarSet,
    QuickbarStyle,
};
pub use ui::{
    spawn_inventory, sync_description, sync_item_position, sync_placement_preview,
    sync_selection_outline, sync_window_visibility, InventoryBoard, InventoryCell,
    InventoryDescriptionSwatch, InventoryDescriptionText, InventoryParts, InventoryRoot,
    Z_ITEM_DRAGGED, Z_ITEM_IDLE,
};

/// Everything you need to build and drive an inventory, in one import.
pub mod prelude {
    pub use super::quickbar::{
        hovered_slot, ActiveSlot, Quickbar, QuickbarAction, QuickbarParts, QuickbarPlugin,
        QuickbarSet, QuickbarStyle,
    };
    pub use super::{
        despawn_item, grab_offset, hovered_cell, quick_transfer, spawn_inventory, spawn_item,
        target_origin, track_drag_target, AddItem, DefaultInventoryBoard, InventoryAccess,
        InventoryAction, InventoryBoard, InventoryBoardSpec, InventoryCell, InventoryClicks,
        InventoryCommands, InventoryConfig, InventoryCursor, InventoryDescriptionSwatch,
        InventoryDescriptionText, InventoryDragState, InventoryGrid, InventoryItem,
        InventoryLayout, InventoryParts, InventoryPlugin, InventoryPreview, InventoryRoot,
        InventorySelection, InventorySet, InventorySlot, InventoryTheme, InventoryTransferTarget,
        InventoryWindow, PanelSide, ResizeInventory,
    };
}

/// Ordering handles for the inventory systems. Order against these rather
/// than against the system functions, so a game keeps working if the
/// internals are re-split.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum InventorySet {
    /// `Startup`. [`spawn_inventory`] for the plugin's own default board.
    /// Order your own item-spawning `.after(InventorySet::Setup)` so
    /// [`DefaultInventoryBoard`] is readable.
    Setup,
    /// `Update`. [`sync_window_visibility`] and [`reset_interaction`] — the
    /// per-board work that has to run whether or not a board is open.
    Window,
    /// `Update`, after [`Window`](Self::Window), before
    /// [`Interaction`](Self::Interaction). Drains the [`AddItem`] and
    /// [`ResizeInventory`] messages — a message-driven add lands in the grid
    /// before that frame's drag handling, no extra frame of latency beyond
    /// Bevy's normal spawn flush.
    Commands,
    /// `Update`, after [`Commands`](Self::Commands). [`track_cursor`],
    /// [`track_drag_target`], [`begin_drag`], [`quick_transfer`],
    /// [`update_drag`], [`end_drag`], chained. Each skips a closed board
    /// internally.
    Interaction,
    /// `Update`, after [`Interaction`](Self::Interaction). The four `sync_*`
    /// systems that push model state back out to `Node`s and colors.
    Sync,
}

/// Whether one board is showing. A `Component` on the board entity — see the
/// module docs' "Showing and hiding a board" section for why this is a
/// component and not a `States` type.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct InventoryWindow {
    pub open: bool,
}

impl Default for InventoryWindow {
    fn default() -> Self {
        Self { open: true }
    }
}

/// What the player can currently *do* with one board. A `Component` on the
/// board entity, written by the host and only ever read by this module — flip
/// it from a distance check, a stun, a cutscene, whatever. See the module
/// docs' "Reach and transfers" section.
///
/// Neither bit gates clicking an item to read its description, and neither
/// gates [`InventoryCommands`] / [`AddItem`] — those are the host calling
/// directly and can check reach themselves.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct InventoryAccess {
    /// The pointer may drag items *within* this board (and out of it). `false`
    /// makes the board inert to the pointer: no dragging, no rearranging, and
    /// it is not a drop target — but its items still click-to-inspect.
    pub interactive: bool,
    /// This board may exchange items with *other* boards. A cross-board move
    /// needs this `true` on **both** endpoints; rearranging inside one board
    /// only needs that board's [`interactive`](Self::interactive).
    pub transfers: bool,
}

impl Default for InventoryAccess {
    fn default() -> Self {
        Self {
            interactive: true,
            transfers: true,
        }
    }
}

/// The board a double-click on *this* board sends items to. A `Component` on
/// the board entity, written by the host and only ever read by this module —
/// re-point it every frame from whatever reach check the game runs (or set it
/// once, for a fixed pairing like a player bag and its own stash). See the
/// module docs' "Quick transfer" section.
///
/// `None` (the [`Default`]) means a double-click here still fires
/// [`InventoryAction::Activated`], but moves nothing.
#[derive(Component, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct InventoryTransferTarget(pub Option<Entity>);

/// The entity of the board [`InventoryPlugin`] spawned for you. Absent if the
/// plugin was built with [`InventoryPlugin::headless`].
#[derive(Resource, Clone, Copy, Debug, Deref)]
pub struct DefaultInventoryBoard(pub Entity);

/// Registers the inventory systems and messages, and — unless built
/// [`headless`](Self::headless) — spawns one board from the [`board`](Self::board)
/// spec at `Startup`.
///
/// ```ignore
/// app.add_plugins(InventoryPlugin {
///     board: Some(InventoryBoardSpec {
///         config: InventoryConfig { cols: 6, rows: 10, ..default() },
///         ..default()
///     }),
/// });
/// ```
pub struct InventoryPlugin {
    /// The board to spawn at `Startup`, or `None` to spawn none and leave
    /// every board to the host. `Default` is `Some(InventoryBoardSpec::default())`.
    pub board: Option<InventoryBoardSpec>,
}

impl Default for InventoryPlugin {
    fn default() -> Self {
        Self {
            board: Some(InventoryBoardSpec::default()),
        }
    }
}

impl InventoryPlugin {
    /// Register the systems but spawn no board — every board is the host's to
    /// create with [`spawn_inventory`].
    pub fn headless() -> Self {
        Self { board: None }
    }
}

/// Holds [`InventoryPlugin::board`] between `build` and the `Startup` system
/// that consumes it.
#[derive(Resource)]
struct PendingDefaultBoard(InventoryBoardSpec);

fn spawn_default_board(mut commands: Commands, pending: Option<Res<PendingDefaultBoard>>) {
    let Some(pending) = pending else { return };
    let board = ui::spawn_inventory(&mut commands, pending.0.clone());
    commands.insert_resource(DefaultInventoryBoard(board));
    commands.remove_resource::<PendingDefaultBoard>();
}

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<InventoryAction>()
            .add_message::<AddItem>()
            .add_message::<ResizeInventory>()
            .configure_sets(
                Update,
                (
                    InventorySet::Window,
                    InventorySet::Commands,
                    InventorySet::Interaction,
                    InventorySet::Sync,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (ui::sync_window_visibility, drag::reset_interaction).in_set(InventorySet::Window),
            )
            .add_systems(
                Update,
                (
                    commands::apply_add_requests,
                    commands::apply_resize_requests,
                )
                    .in_set(InventorySet::Commands),
            )
            .add_systems(
                Update,
                (
                    drag::track_cursor,
                    drag::track_drag_target,
                    drag::begin_drag,
                    drag::quick_transfer,
                    drag::update_drag,
                    drag::end_drag,
                )
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

        if let Some(spec) = &self.board {
            app.insert_resource(PendingDefaultBoard(spec.clone()))
                .add_systems(Startup, spawn_default_board.in_set(InventorySet::Setup));
        }
    }
}
