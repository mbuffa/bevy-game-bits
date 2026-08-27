//! Headless behaviour tests for `bevy_game_bits::inventory`.
//!
//! The unit tests inside the module cover pure grid/config math; these drive
//! the actual scheduled systems — press, drag, release, and the window
//! state — because the thing most worth protecting is exactly the bug this
//! extraction was born fixing: an item drawn on screen but never registered
//! in the occupancy grid, so nothing was ever selectable.
//!
//! No `UiPlugin`/`InputPlugin`. `RelativeCursorPosition` is written by hand
//! onto the board entity (`normalized` is centre-relative, `-0.5..0.5`)
//! instead of relying on `ui_focus_system` and a real window; and
//! `ButtonInput<MouseButton>` is driven by hand and `.clear()`-ed between
//! frames ourselves — `InputPlugin`'s own `PreUpdate` systems would
//! otherwise wipe a manually-set `just_pressed` before `Update` reads it.

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::ui::RelativeCursorPosition;

use bevy_game_bits::inventory::prelude::*;

fn pistol() -> InventoryItem {
    InventoryItem { name: "Pistol", description: "test item", size: UVec2::new(1, 1), color: Color::WHITE }
}

fn medkit() -> InventoryItem {
    InventoryItem { name: "Medkit", description: "test item", size: UVec2::new(2, 2), color: Color::WHITE }
}

/// A minimal, headless app with the inventory plugin already through
/// `Startup` — `InventoryUi`, the board, and its cell tiles all exist.
fn new_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(StatesPlugin)
        .add_plugins(InventoryPlugin::default())
        .insert_resource(ButtonInput::<MouseButton>::default());
    app.update();
    app
}

/// Spawns `item` at `origin` through the real `spawn_item` entry point —
/// this is the exact call a host makes, and the exact call that was missing
/// its `grid.place` half in the bug this test suite guards against.
fn spawn_test_item(app: &mut App, item: InventoryItem, origin: UVec2) -> Entity {
    app.world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  ui: Res<InventoryUi>,
                  mut grid: ResMut<InventoryGrid>,
                  config: Res<InventoryConfig>,
                  theme: Res<InventoryTheme>| {
                spawn_item(&mut commands, ui.board, &mut grid, &config, &theme, item, origin)
                    .expect("test fixtures always fit")
            },
        )
        .unwrap()
}

/// Points the synthetic cursor at the center of `cell`, as if the OS cursor
/// were really there.
fn hover_cell(app: &mut App, cell: UVec2) {
    let config = *app.world().resource::<InventoryConfig>();
    let board = app.world().resource::<InventoryUi>().board;
    let center_px = (cell.as_vec2() + Vec2::splat(0.5)) * config.cell_px;
    let normalized = center_px / config.board_size() - Vec2::splat(0.5);
    let mut rel = app.world_mut().get_mut::<RelativeCursorPosition>(board).unwrap();
    rel.cursor_over = true;
    rel.normalized = Some(normalized);
}

/// One frame with `button` freshly pressed, then clears the "just" flag —
/// mimicking exactly one real input frame.
fn press(app: &mut App, button: MouseButton) {
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(button);
    app.update();
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().clear();
}

/// One frame with `button` freshly released, same "just" semantics as
/// [`press`].
fn release(app: &mut App, button: MouseButton) {
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().release(button);
    app.update();
    app.world_mut().resource_mut::<ButtonInput<MouseButton>>().clear();
}

fn selection(app: &App) -> Option<Entity> {
    app.world().resource::<InventorySelection>().0
}

#[test]
fn a_spawned_item_is_selectable_where_it_was_drawn() {
    let mut app = new_app();
    let item = spawn_test_item(&mut app, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    // No movement in between: this is a plain click.
    release(&mut app, MouseButton::Left);

    assert_eq!(selection(&app), Some(item));
    assert!(matches!(*app.world().resource::<InventoryDragState>(), InventoryDragState::Idle));
}

#[test]
fn a_drag_to_a_free_footprint_moves_both_the_slot_and_the_grid() {
    let mut app = new_app();
    let item = spawn_test_item(&mut app, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, UVec2::new(3, 3));
    app.update(); // update_drag: crosses the threshold, node follows the cursor
    release(&mut app, MouseButton::Left);

    assert_eq!(app.world().get::<InventorySlot>(item).unwrap().0, UVec2::new(3, 3));
    let grid = app.world().resource::<InventoryGrid>();
    assert_eq!(grid.at(UVec2::new(3, 3)), Some(item));
    assert_eq!(grid.at(UVec2::new(0, 0)), None);
}

#[test]
fn a_drag_onto_an_occupied_footprint_reverts_and_leaves_the_grid_alone() {
    let mut app = new_app();
    let moving = spawn_test_item(&mut app, pistol(), UVec2::new(0, 0));
    let blocker = spawn_test_item(&mut app, medkit(), UVec2::new(3, 3));

    hover_cell(&mut app, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, UVec2::new(3, 3)); // lands inside the medkit's 2x2 footprint
    app.update();
    release(&mut app, MouseButton::Left);

    assert_eq!(app.world().get::<InventorySlot>(moving).unwrap().0, UVec2::new(0, 0));
    let grid = app.world().resource::<InventoryGrid>();
    assert_eq!(grid.at(UVec2::new(0, 0)), Some(moving));
    assert_eq!(grid.at(UVec2::new(3, 3)), Some(blocker));
}

#[test]
fn closing_the_window_cancels_a_live_drag_and_stops_the_systems() {
    let mut app = new_app();
    let item = spawn_test_item(&mut app, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, UVec2::new(3, 3));
    app.update(); // now mid-drag: DragState::Held { moved: true, .. }
    assert!(matches!(*app.world().resource::<InventoryDragState>(), InventoryDragState::Held { .. }));

    app.world_mut().resource_mut::<NextState<InventoryWindowState>>().set(InventoryWindowState::Closed);
    app.update(); // OnExit(Open) -> reset_interaction

    assert!(matches!(*app.world().resource::<InventoryDragState>(), InventoryDragState::Idle));
    let node = app.world().get::<Node>(item).unwrap();
    let config = *app.world().resource::<InventoryConfig>();
    assert_eq!(node.left, Val::Px(0.0 * config.cell_px + config.item_inset_px));
    assert_eq!(node.top, Val::Px(0.0 * config.cell_px + config.item_inset_px));

    // The button is still up from `release` never having been called, and
    // the window is closed, so a fresh press should do nothing at all —
    // `InventorySet::Interaction` isn't running.
    let before = selection(&app);
    hover_cell(&mut app, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);
    assert_eq!(selection(&app), before);
}
