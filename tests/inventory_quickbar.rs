//! Headless behaviour tests for `bevy_game_bits::inventory::quickbar`.
//!
//! Same shape as `tests/inventory_drag.rs`: `MinimalPlugins` only, no
//! `UiPlugin`/`InputPlugin`, `RelativeCursorPosition` written by hand and
//! `ButtonInput` driven and `.clear()`-ed between frames. The quickbar strip's
//! own `RelativeCursorPosition` (on `QuickbarParts::root`) is written directly
//! for the drag-onto-a-slot tests.

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::time::Virtual;
use bevy::ui::RelativeCursorPosition;

use bevy_game_bits::inventory::prelude::*;

fn pistol() -> InventoryItem {
    InventoryItem::new("Pistol", "test item", UVec2::new(1, 1), Color::WHITE)
}

fn rifle() -> InventoryItem {
    InventoryItem::new("Rifle", "test item", UVec2::new(2, 1), Color::WHITE)
}

fn new_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(InventoryPlugin::headless())
        .add_plugins(QuickbarPlugin)
        .insert_resource(ButtonInput::<MouseButton>::default())
        .insert_resource(ButtonInput::<KeyCode>::default());
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    app.update();
    app
}

/// A board with a 4-slot quickbar. Two updates: one to run `spawn_inventory`'s
/// deferred commands, one for `build_quickbar_ui` to raise the strip.
fn board_with_bar(app: &mut App, slots: usize) -> Entity {
    let board = app
        .world_mut()
        .run_system_once(move |mut commands: Commands| {
            let board = spawn_inventory(&mut commands, InventoryBoardSpec::default());
            commands.entity(board).insert((
                Quickbar::new(slots),
                ActiveSlot::default(),
                QuickbarStyle::default(),
            ));
            board
        })
        .unwrap();
    app.update();
    app.update();
    board
}

fn spawn_on_board(app: &mut App, board: Entity, item: InventoryItem, origin: UVec2) -> Entity {
    let entity =
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands,
                      mut boards: Query<(
                    &mut InventoryGrid,
                    &InventoryConfig,
                    &InventoryTheme,
                )>| {
                    let (mut grid, config, theme) = boards.get_mut(board).unwrap();
                    let (config, theme) = (config.clone(), theme.clone());
                    spawn_item(
                        &mut commands,
                        board,
                        &mut grid,
                        &config,
                        &theme,
                        item.clone(),
                        origin,
                    )
                    .expect("fixture fits")
                },
            )
            .unwrap();
    app.update();
    entity
}

fn quickbar(app: &App, board: Entity) -> Vec<Option<Entity>> {
    app.world().get::<Quickbar>(board).unwrap().slots.clone()
}

fn active(app: &App, board: Entity) -> Option<usize> {
    app.world().get::<ActiveSlot>(board).unwrap().0
}

fn press_key(app: &mut App, key: KeyCode) {
    {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.reset(key); // a prior press left `key` in `pressed`, suppressing `just_pressed`
        input.press(key);
    }
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn press_mouse(app: &mut App, button: MouseButton) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(button);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn release_mouse(app: &mut App, button: MouseButton) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(button);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn hover_board_cell(app: &mut App, board: Entity, cell: UVec2) {
    let config = app.world().get::<InventoryConfig>(board).unwrap().clone();
    let center = (cell.as_vec2() + Vec2::splat(0.5)) * config.cell_px;
    let normalized = center / config.board_size() - Vec2::splat(0.5);
    let mut rel = app
        .world_mut()
        .get_mut::<RelativeCursorPosition>(board)
        .unwrap();
    rel.cursor_over = true;
    rel.normalized = Some(normalized);
}

/// Cursor off the board (so no in-board drop happens) and over the strip's
/// `slot`.
fn hover_strip_slot(app: &mut App, board: Entity, slot: usize, slots: usize) {
    let mut board_rel = app
        .world_mut()
        .get_mut::<RelativeCursorPosition>(board)
        .unwrap();
    board_rel.cursor_over = false;
    board_rel.normalized = Some(Vec2::splat(5.0));

    let root = app.world().get::<QuickbarParts>(board).unwrap().root;
    let x = (slot as f32 + 0.5) / slots as f32 - 0.5;
    let mut rel = app
        .world_mut()
        .get_mut::<RelativeCursorPosition>(root)
        .unwrap();
    rel.cursor_over = true;
    rel.normalized = Some(Vec2::new(x, 0.0));
}

#[test]
fn a_new_board_item_auto_fills_the_first_free_slot() {
    let mut app = new_app();
    let board = board_with_bar(&mut app, 4);

    let crowbar = spawn_on_board(&mut app, board, pistol(), UVec2::new(0, 0));
    app.update();
    assert_eq!(quickbar(&app, board)[0], Some(crowbar));

    let second = spawn_on_board(&mut app, board, pistol(), UVec2::new(1, 0));
    app.update();
    assert_eq!(quickbar(&app, board)[1], Some(second));
}

#[test]
fn removing_an_item_clears_its_slot_and_frees_the_hands() {
    let mut app = new_app();
    let board = board_with_bar(&mut app, 4);
    let item = spawn_on_board(&mut app, board, pistol(), UVec2::new(0, 0));
    app.update();

    press_key(&mut app, KeyCode::Digit1);
    assert_eq!(active(&app, board), Some(0));

    app.world_mut()
        .run_system_once(move |mut inv: InventoryCommands| inv.remove(item))
        .unwrap();
    app.update();
    app.update();

    assert_eq!(quickbar(&app, board)[0], None);
    assert_eq!(active(&app, board), None);
    assert!(app.world().get_entity(item).is_err());
}

#[test]
fn number_keys_select_and_free_the_hands() {
    let mut app = new_app();
    let board = board_with_bar(&mut app, 4);
    spawn_on_board(&mut app, board, pistol(), UVec2::new(0, 0));
    spawn_on_board(&mut app, board, pistol(), UVec2::new(1, 0));
    app.update();

    press_key(&mut app, KeyCode::Digit2);
    assert_eq!(active(&app, board), Some(1));
    // Re-pressing the active slot's key frees the hands.
    press_key(&mut app, KeyCode::Digit2);
    assert_eq!(active(&app, board), None);
    // An empty slot's key never grabs.
    press_key(&mut app, KeyCode::Digit4);
    assert_eq!(active(&app, board), None);
}

#[test]
fn a_drag_released_over_a_slot_assigns_it() {
    let mut app = new_app();
    let board = board_with_bar(&mut app, 4);
    let item = spawn_on_board(&mut app, board, pistol(), UVec2::new(0, 0));
    app.update();
    // Auto-assigned to slot 0; move it to slot 3 by dragging.
    assert_eq!(quickbar(&app, board)[0], Some(item));

    hover_board_cell(&mut app, board, UVec2::new(0, 0));
    press_mouse(&mut app, MouseButton::Left);
    hover_strip_slot(&mut app, board, 3, 4);
    app.update(); // update_drag crosses the threshold -> Held { moved: true }
    release_mouse(&mut app, MouseButton::Left);

    let slots = quickbar(&app, board);
    assert_eq!(slots[3], Some(item));
    assert_eq!(slots[0], None);
    assert!(matches!(
        *app.world().get::<InventoryDragState>(board).unwrap(),
        InventoryDragState::Idle
    ));
    // The item never left the board.
    assert_eq!(app.world().get::<ChildOf>(item).unwrap().parent(), board);
    assert_eq!(
        app.world().get::<InventorySlot>(item).unwrap().0,
        UVec2::new(0, 0)
    );
}

#[test]
fn a_drag_onto_an_occupied_slot_swaps() {
    let mut app = new_app();
    let board = board_with_bar(&mut app, 4);
    let a = spawn_on_board(&mut app, board, pistol(), UVec2::new(0, 0));
    let b = spawn_on_board(&mut app, board, rifle(), UVec2::new(2, 0));
    app.update();
    assert_eq!(quickbar(&app, board)[0], Some(a));
    assert_eq!(quickbar(&app, board)[1], Some(b));

    // Drag `a` (slot 0) onto slot 1 (holds `b`): they swap.
    hover_board_cell(&mut app, board, UVec2::new(0, 0));
    press_mouse(&mut app, MouseButton::Left);
    hover_strip_slot(&mut app, board, 1, 4);
    app.update();
    release_mouse(&mut app, MouseButton::Left);

    let slots = quickbar(&app, board);
    assert_eq!(slots[0], Some(b));
    assert_eq!(slots[1], Some(a));
}

#[test]
fn a_double_click_activates_the_items_slot() {
    let mut app = new_app();
    let board = board_with_bar(&mut app, 4);
    let item = spawn_on_board(&mut app, board, pistol(), UVec2::new(0, 0));
    app.update();

    // A board double-click fires InventoryAction::Activated, which select_slot
    // promotes to activating the slot.
    app.world_mut()
        .write_message(bevy_game_bits::inventory::InventoryAction::Activated { board, item });
    app.update();

    assert_eq!(active(&app, board), Some(0));
}
