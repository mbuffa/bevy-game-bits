//! Headless behaviour tests for `bevy_game_bits::inventory`.
//!
//! The unit tests inside the module cover pure grid/config math; these drive
//! the actual scheduled systems — press, drag, release, the window flag, the
//! runtime add/resize API, and two boards side by side — because the thing
//! most worth protecting is exactly the bug this extraction was born fixing:
//! an item drawn on screen but never registered in the occupancy grid.
//!
//! No `UiPlugin`/`InputPlugin`, and no `StatesPlugin` (the window is a plain
//! component now). `RelativeCursorPosition` is written by hand onto each board
//! entity (`normalized` is centre-relative, `-0.5..0.5`); and
//! `ButtonInput<MouseButton>` is driven by hand and `.clear()`-ed between
//! frames ourselves — `InputPlugin`'s own `PreUpdate` systems would otherwise
//! wipe a manually-set `just_pressed` before `Update` reads it.

use std::time::Duration;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::time::Virtual;
use bevy::ui::RelativeCursorPosition;

use bevy_game_bits::inventory::prelude::*;

fn pistol() -> InventoryItem {
    InventoryItem::new("Pistol", "test item", UVec2::new(1, 1), Color::WHITE)
}

fn medkit() -> InventoryItem {
    InventoryItem::new("Medkit", "test item", UVec2::new(2, 2), Color::WHITE)
}

/// A minimal, headless app with the inventory systems registered but no
/// board — every test spawns the board(s) it wants.
///
/// Virtual time is paused, freezing `Res<Time>::elapsed_secs()` so any two
/// presses in a test are "simultaneous" and the double-click window always
/// holds unless [`advance`] walks it forward on purpose.
fn new_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(InventoryPlugin::headless())
        .insert_resource(ButtonInput::<MouseButton>::default());
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    app.update();
    app
}

/// Walks paused virtual time forward, for tests that need two presses to
/// fall outside [`InventoryConfig::double_click_secs`].
fn advance(app: &mut App, secs: f32) {
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(Duration::from_secs_f32(secs));
}

fn spawn_board(app: &mut App, spec: InventoryBoardSpec) -> Entity {
    let board = app
        .world_mut()
        .run_system_once(move |mut commands: Commands| spawn_inventory(&mut commands, spec.clone()))
        .unwrap();
    app.update();
    board
}

fn default_board(app: &mut App) -> Entity {
    spawn_board(app, InventoryBoardSpec::default())
}

/// Spawns `item` at `origin` through the real `spawn_item` entry point.
fn spawn_test_item(app: &mut App, board: Entity, item: InventoryItem, origin: UVec2) -> Entity {
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
                    .expect("test fixtures always fit")
                },
            )
            .unwrap();
    app.update();
    entity
}

/// Runs `InventoryCommands::add` and returns what it placed, plus every
/// `InventoryAction` it fired.
fn add_item(
    app: &mut App,
    board: Entity,
    item: InventoryItem,
) -> (Option<Entity>, Vec<InventoryAction>) {
    let placed = app
        .world_mut()
        .run_system_once(move |mut inv: InventoryCommands| inv.add(board, item.clone()))
        .unwrap();
    let actions = drain_actions(app);
    app.update();
    (placed, actions)
}

fn drain_actions(app: &mut App) -> Vec<InventoryAction> {
    app.world_mut()
        .run_system_once(|mut reader: MessageReader<InventoryAction>| {
            reader.read().cloned().collect::<Vec<_>>()
        })
        .unwrap()
}

/// Points the synthetic cursor at the center of `cell` on `board`.
fn hover_cell(app: &mut App, board: Entity, cell: UVec2) {
    let config = app.world().get::<InventoryConfig>(board).unwrap().clone();
    let center_px = (cell.as_vec2() + Vec2::splat(0.5)) * config.cell_px;
    let normalized = center_px / config.board_size() - Vec2::splat(0.5);
    let mut rel = app
        .world_mut()
        .get_mut::<RelativeCursorPosition>(board)
        .unwrap();
    rel.cursor_over = true;
    rel.normalized = Some(normalized);
}

/// Points the synthetic cursor at `cell` on `board` and marks every *other*
/// board as not-hovered — but leaves those boards a far-off-screen
/// `normalized`, the way `ui_focus_system` keeps extrapolating a position for
/// every node regardless of which one the cursor is actually over. That
/// extrapolated value is what carries a drag's `press_px` distance past the
/// threshold once the cursor leaves the board it started on.
fn hover_cell_exclusive(app: &mut App, board: Entity, cell: UVec2) {
    let config = app.world().get::<InventoryConfig>(board).unwrap().clone();
    let center_px = (cell.as_vec2() + Vec2::splat(0.5)) * config.cell_px;
    let normalized = center_px / config.board_size() - Vec2::splat(0.5);
    app.world_mut()
        .run_system_once(
            move |mut boards: Query<(Entity, &mut RelativeCursorPosition), With<InventoryBoard>>| {
                for (entity, mut rel) in &mut boards {
                    if entity == board {
                        rel.cursor_over = true;
                        rel.normalized = Some(normalized);
                    } else {
                        rel.cursor_over = false;
                        rel.normalized = Some(Vec2::splat(5.0));
                    }
                }
            },
        )
        .unwrap();
}

fn set_access(app: &mut App, board: Entity, interactive: bool, transfers: bool) {
    *app.world_mut().get_mut::<InventoryAccess>(board).unwrap() = InventoryAccess {
        interactive,
        transfers,
    };
}

/// Points `board`'s double-click destination at `target`.
fn link(app: &mut App, board: Entity, target: Entity) {
    app.world_mut()
        .get_mut::<InventoryTransferTarget>(board)
        .unwrap()
        .0 = Some(target);
}

/// One frame with `button` freshly pressed, then clears the "just" flag.
fn press(app: &mut App, button: MouseButton) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(button);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

/// One frame with `button` freshly released, same "just" semantics.
fn release(app: &mut App, button: MouseButton) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(button);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn selection(app: &App, board: Entity) -> Option<Entity> {
    app.world().get::<InventorySelection>(board).unwrap().0
}

fn grid_at(app: &App, board: Entity, cell: UVec2) -> Option<Entity> {
    app.world().get::<InventoryGrid>(board).unwrap().at(cell)
}

fn parent_of(app: &App, item: Entity) -> Entity {
    app.world().get::<ChildOf>(item).unwrap().parent()
}

fn slot_of(app: &App, item: Entity) -> UVec2 {
    app.world().get::<InventorySlot>(item).unwrap().0
}

/// press on `source` at `from`, drag across to `target` at `to`, release.
fn drag_between(app: &mut App, source: Entity, from: UVec2, target: Entity, to: UVec2) {
    hover_cell(app, source, from);
    press(app, MouseButton::Left);
    hover_cell_exclusive(app, target, to);
    app.update();
    release(app, MouseButton::Left);
    app.update(); // flush the ChildOf reparent
}

/// Two quick, in-place presses on `cell` of `board` — a double-click. No
/// trailing `app.update()`: `quick_transfer`'s `ChildOf` reparent already
/// flushes within the second press's own update (before `InventorySet::Sync`
/// reads it), and an extra frame here would age the `InventoryAction`
/// messages out of their two-frame window before a test can drain them.
fn double_click(app: &mut App, board: Entity, cell: UVec2) {
    hover_cell(app, board, cell);
    press(app, MouseButton::Left);
    release(app, MouseButton::Left);
    press(app, MouseButton::Left);
    release(app, MouseButton::Left);
}

fn cell_count(app: &mut App, board: Entity) -> usize {
    app.world_mut()
        .run_system_once(move |cells: Query<&ChildOf, With<InventoryCell>>| {
            cells.iter().filter(|c| c.parent() == board).count()
        })
        .unwrap()
}

#[test]
fn a_spawned_item_is_selectable_where_it_was_drawn() {
    let mut app = new_app();
    let board = default_board(&mut app);
    let item = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, board, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);

    assert_eq!(selection(&app, board), Some(item));
    assert!(matches!(
        *app.world().get::<InventoryDragState>(board).unwrap(),
        InventoryDragState::Idle
    ));
}

#[test]
fn a_drag_to_a_free_footprint_moves_both_the_slot_and_the_grid() {
    let mut app = new_app();
    let board = default_board(&mut app);
    let item = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, board, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, board, UVec2::new(3, 3));
    app.update();
    release(&mut app, MouseButton::Left);

    assert_eq!(
        app.world().get::<InventorySlot>(item).unwrap().0,
        UVec2::new(3, 3)
    );
    assert_eq!(grid_at(&app, board, UVec2::new(3, 3)), Some(item));
    assert_eq!(grid_at(&app, board, UVec2::new(0, 0)), None);
}

#[test]
fn a_drag_onto_an_occupied_footprint_reverts_and_leaves_the_grid_alone() {
    let mut app = new_app();
    let board = default_board(&mut app);
    let moving = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 0));
    let blocker = spawn_test_item(&mut app, board, medkit(), UVec2::new(3, 3));

    hover_cell(&mut app, board, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, board, UVec2::new(3, 3));
    app.update();
    release(&mut app, MouseButton::Left);

    assert_eq!(
        app.world().get::<InventorySlot>(moving).unwrap().0,
        UVec2::new(0, 0)
    );
    assert_eq!(grid_at(&app, board, UVec2::new(0, 0)), Some(moving));
    assert_eq!(grid_at(&app, board, UVec2::new(3, 3)), Some(blocker));
}

#[test]
fn closing_the_window_cancels_a_live_drag_and_stops_the_systems() {
    let mut app = new_app();
    let board = default_board(&mut app);
    let item = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, board, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, board, UVec2::new(3, 3));
    app.update();
    assert!(matches!(
        *app.world().get::<InventoryDragState>(board).unwrap(),
        InventoryDragState::Held { .. }
    ));

    app.world_mut()
        .get_mut::<InventoryWindow>(board)
        .unwrap()
        .open = false;
    app.update();

    assert!(matches!(
        *app.world().get::<InventoryDragState>(board).unwrap(),
        InventoryDragState::Idle
    ));
    let node = app.world().get::<Node>(item).unwrap();
    let config = app.world().get::<InventoryConfig>(board).unwrap();
    assert_eq!(node.left, Val::Px(config.item_inset_px));
    assert_eq!(node.top, Val::Px(config.item_inset_px));

    let before = selection(&app, board);
    hover_cell(&mut app, board, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);
    assert_eq!(selection(&app, board), before);
}

#[test]
fn add_puts_an_item_in_the_first_free_spot() {
    let mut app = new_app();
    let board = default_board(&mut app);
    spawn_test_item(&mut app, board, medkit(), UVec2::new(0, 0)); // fills (0,0)..(1,1)

    let (placed, actions) = add_item(&mut app, board, pistol());
    let placed = placed.expect("the board has room");

    // First free row-major origin after a 2x2 at the corner is (2, 0).
    assert_eq!(
        app.world().get::<InventorySlot>(placed).unwrap().0,
        UVec2::new(2, 0)
    );
    assert_eq!(grid_at(&app, board, UVec2::new(2, 0)), Some(placed));
    assert!(actions.iter().any(
        |a| matches!(a, InventoryAction::Added { origin, .. } if *origin == UVec2::new(2, 0))
    ));
}

#[test]
fn add_to_a_full_board_is_rejected_and_hands_the_item_back() {
    let mut app = new_app();
    let board = spawn_board(
        &mut app,
        InventoryBoardSpec {
            config: InventoryConfig {
                cols: 2,
                rows: 2,
                ..default()
            },
            ..default()
        },
    );
    spawn_test_item(&mut app, board, medkit(), UVec2::new(0, 0)); // 2x2 fills it

    let (placed, actions) = add_item(&mut app, board, pistol());
    assert_eq!(placed, None);
    assert!(actions
        .iter()
        .any(|a| matches!(a, InventoryAction::AddRejected { item, .. } if *item == pistol())));
}

#[test]
fn the_add_message_and_the_system_param_place_identically() {
    let mut param_app = new_app();
    let param_board = default_board(&mut param_app);
    spawn_test_item(&mut param_app, param_board, medkit(), UVec2::new(0, 0));
    let (via_param, _) = add_item(&mut param_app, param_board, pistol());
    let via_param = via_param.unwrap();

    let mut msg_app = new_app();
    let msg_board = default_board(&mut msg_app);
    spawn_test_item(&mut msg_app, msg_board, medkit(), UVec2::new(0, 0));
    msg_app.world_mut().write_message(AddItem {
        board: msg_board,
        item: pistol(),
        origin: None,
    });
    msg_app.update();
    let via_msg = msg_app
        .world_mut()
        .run_system_once(|items: Query<(Entity, &InventoryItem)>| {
            items
                .iter()
                .find(|(_, i)| i.name == "Pistol")
                .map(|(e, _)| e)
        })
        .unwrap()
        .expect("the message add spawned a pistol");

    assert_eq!(
        param_app.world().get::<InventorySlot>(via_param).unwrap().0,
        msg_app.world().get::<InventorySlot>(via_msg).unwrap().0,
    );
}

#[test]
fn two_boards_drag_independently() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let bag_item = spawn_test_item(&mut app, bag, pistol(), UVec2::new(0, 0));
    let stash_item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(1, 1));

    // A press over the bag must not touch the stash's selection or grid.
    hover_cell(&mut app, bag, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, bag, UVec2::new(4, 4));
    app.update();
    release(&mut app, MouseButton::Left);

    assert_eq!(selection(&app, bag), Some(bag_item));
    assert_eq!(selection(&app, stash), None);
    assert_eq!(
        app.world().get::<InventorySlot>(bag_item).unwrap().0,
        UVec2::new(4, 4)
    );
    assert_eq!(
        app.world().get::<InventorySlot>(stash_item).unwrap().0,
        UVec2::new(1, 1)
    );
    assert_eq!(grid_at(&app, stash, UVec2::new(1, 1)), Some(stash_item));
}

#[test]
fn closing_one_board_leaves_the_other_interactive() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash_item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    app.world_mut()
        .get_mut::<InventoryWindow>(bag)
        .unwrap()
        .open = false;
    app.update();

    hover_cell(&mut app, stash, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);

    assert_eq!(selection(&app, stash), Some(stash_item));
}

#[test]
fn resize_relocates_what_fits_and_evicts_the_rest() {
    let mut app = new_app();
    let board = spawn_board(
        &mut app,
        InventoryBoardSpec {
            config: InventoryConfig {
                cols: 2,
                rows: 3,
                ..default()
            },
            ..default()
        },
    );
    let top = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 0));
    let mid = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 1));
    let bottom = spawn_test_item(&mut app, board, pistol(), UVec2::new(1, 2));

    assert_eq!(cell_count(&mut app, board), 6);

    app.world_mut().write_message(ResizeInventory {
        board,
        cols: 2,
        rows: 2,
    });
    app.update();
    let actions = drain_actions(&mut app);

    // top and mid still fit where they are; bottom's (1,2) is gone but (1,0)
    // is free, so it relocates rather than being evicted.
    assert_eq!(
        app.world().get::<InventorySlot>(top).unwrap().0,
        UVec2::new(0, 0)
    );
    assert_eq!(
        app.world().get::<InventorySlot>(mid).unwrap().0,
        UVec2::new(0, 1)
    );
    assert_eq!(
        app.world().get::<InventorySlot>(bottom).unwrap().0,
        UVec2::new(1, 0)
    );
    assert_eq!(cell_count(&mut app, board), 4);
    assert_eq!(app.world().get::<InventoryConfig>(board).unwrap().rows, 2);
    assert!(actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Resized { rows: 2, .. })));

    // Now shrink to a single row: only (0,0) and (1,0) survive.
    app.world_mut().write_message(ResizeInventory {
        board,
        cols: 2,
        rows: 1,
    });
    app.update();
    let actions = drain_actions(&mut app);
    let evicted: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            InventoryAction::Evicted { item, .. } => Some(item.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(evicted, vec![pistol()]);
    // top (0,0) and bottom (1,0) survive the single row; mid (0,1) is gone.
    assert!(app.world().get_entity(top).is_ok());
    assert!(app.world().get_entity(bottom).is_ok());
    assert!(app.world().get_entity(mid).is_err());
    assert_eq!(cell_count(&mut app, board), 2);
}

#[test]
fn a_drag_from_one_board_to_another_moves_it() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    drag_between(&mut app, stash, UVec2::new(0, 0), bag, UVec2::new(2, 2));

    assert_eq!(parent_of(&app, item), bag);
    assert_eq!(slot_of(&app, item), UVec2::new(2, 2));
    assert_eq!(grid_at(&app, bag, UVec2::new(2, 2)), Some(item));
    assert_eq!(grid_at(&app, stash, UVec2::new(0, 0)), None);

    let actions = drain_actions(&mut app);
    assert!(actions.iter().any(|a| matches!(
        a,
        InventoryAction::Transferred { from_board, to_board, from, to, .. }
            if *from_board == stash && *to_board == bag
                && *from == UVec2::new(0, 0) && *to == UVec2::new(2, 2)
    )));
}

#[test]
fn a_transfer_onto_an_occupied_footprint_reverts_to_the_source_board() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let mover = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));
    let blocker = spawn_test_item(&mut app, bag, medkit(), UVec2::new(2, 2));

    drag_between(&mut app, stash, UVec2::new(0, 0), bag, UVec2::new(2, 2));

    assert_eq!(parent_of(&app, mover), stash);
    assert_eq!(slot_of(&app, mover), UVec2::new(0, 0));
    assert_eq!(grid_at(&app, stash, UVec2::new(0, 0)), Some(mover));
    assert_eq!(grid_at(&app, bag, UVec2::new(2, 2)), Some(blocker));

    let actions = drain_actions(&mut app);
    assert!(actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Rejected { board, .. } if *board == stash)));
}

#[test]
fn a_non_interactive_board_can_still_be_inspected_but_not_dragged_from() {
    let mut app = new_app();
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));
    set_access(&mut app, stash, false, false);

    hover_cell(&mut app, stash, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, stash, UVec2::new(3, 3));
    app.update();
    release(&mut app, MouseButton::Left);

    assert_eq!(selection(&app, stash), Some(item));
    assert_eq!(slot_of(&app, item), UVec2::new(0, 0));
    assert!(matches!(
        *app.world().get::<InventoryDragState>(stash).unwrap(),
        InventoryDragState::Idle
    ));
}

#[test]
fn a_board_that_refuses_transfers_can_still_be_rearranged_internally() {
    let mut app = new_app();
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));
    set_access(&mut app, stash, true, false);

    hover_cell(&mut app, stash, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, stash, UVec2::new(2, 2));
    app.update();
    release(&mut app, MouseButton::Left);

    assert_eq!(slot_of(&app, item), UVec2::new(2, 2));
    assert_eq!(grid_at(&app, stash, UVec2::new(2, 2)), Some(item));
}

#[test]
fn a_transfer_needs_the_bit_on_both_ends() {
    for (src_transfers, tgt_transfers) in [(true, true), (true, false), (false, true), (false, false)]
    {
        let mut app = new_app();
        let bag = spawn_board(&mut app, InventoryBoardSpec::default());
        let stash = spawn_board(&mut app, InventoryBoardSpec::default());
        let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));
        set_access(&mut app, stash, true, src_transfers);
        set_access(&mut app, bag, true, tgt_transfers);

        drag_between(&mut app, stash, UVec2::new(0, 0), bag, UVec2::new(2, 2));

        if src_transfers && tgt_transfers {
            assert_eq!(parent_of(&app, item), bag, "{src_transfers},{tgt_transfers}");
            assert_eq!(slot_of(&app, item), UVec2::new(2, 2));
        } else {
            assert_eq!(parent_of(&app, item), stash, "{src_transfers},{tgt_transfers}");
            assert_eq!(slot_of(&app, item), UVec2::new(0, 0));
        }
    }
}

#[test]
fn losing_interactive_mid_drag_snaps_the_item_back() {
    let mut app = new_app();
    let board = default_board(&mut app);
    let item = spawn_test_item(&mut app, board, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, board, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, board, UVec2::new(3, 3));
    app.update();
    assert!(matches!(
        *app.world().get::<InventoryDragState>(board).unwrap(),
        InventoryDragState::Held { .. }
    ));

    set_access(&mut app, board, false, true);
    app.update();

    assert!(matches!(
        *app.world().get::<InventoryDragState>(board).unwrap(),
        InventoryDragState::Idle
    ));
    assert_eq!(slot_of(&app, item), UVec2::new(0, 0));
    assert_eq!(grid_at(&app, board, UVec2::new(0, 0)), Some(item));
}

#[test]
fn a_transfer_moves_the_selection_to_the_receiving_board() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    drag_between(&mut app, stash, UVec2::new(0, 0), bag, UVec2::new(2, 2));

    assert_eq!(selection(&app, stash), None);
    assert_eq!(selection(&app, bag), Some(item));
}

#[test]
fn a_double_click_sends_the_item_to_the_named_board() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    link(&mut app, stash, bag);
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    double_click(&mut app, stash, UVec2::new(0, 0));

    assert_eq!(parent_of(&app, item), bag);
    assert_eq!(slot_of(&app, item), UVec2::new(0, 0));
    assert_eq!(grid_at(&app, bag, UVec2::new(0, 0)), Some(item));
    assert_eq!(grid_at(&app, stash, UVec2::new(0, 0)), None);

    let actions = drain_actions(&mut app);
    assert!(actions.iter().any(
        |a| matches!(a, InventoryAction::Activated { board, item: i } if *board == stash && *i == item)
    ));
    assert!(actions.iter().any(|a| matches!(
        a,
        InventoryAction::Transferred { from_board, to_board, .. }
            if *from_board == stash && *to_board == bag
    )));
}

#[test]
fn a_double_click_with_no_named_board_only_reports_activation() {
    let mut app = new_app();
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    double_click(&mut app, stash, UVec2::new(0, 0));

    assert_eq!(parent_of(&app, item), stash);
    assert_eq!(slot_of(&app, item), UVec2::new(0, 0));

    let actions = drain_actions(&mut app);
    assert!(actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Activated { .. })));
    assert!(!actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Transferred { .. })));
}

#[test]
fn a_double_click_needs_transfers_on_both_ends() {
    for (src_transfers, tgt_transfers) in [(true, true), (true, false), (false, true), (false, false)]
    {
        let mut app = new_app();
        let bag = spawn_board(&mut app, InventoryBoardSpec::default());
        let stash = spawn_board(&mut app, InventoryBoardSpec::default());
        link(&mut app, stash, bag);
        let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));
        set_access(&mut app, stash, true, src_transfers);
        set_access(&mut app, bag, true, tgt_transfers);

        double_click(&mut app, stash, UVec2::new(0, 0));

        if src_transfers && tgt_transfers {
            assert_eq!(parent_of(&app, item), bag, "{src_transfers},{tgt_transfers}");
        } else {
            assert_eq!(
                parent_of(&app, item),
                stash,
                "{src_transfers},{tgt_transfers}"
            );
            assert_eq!(slot_of(&app, item), UVec2::new(0, 0));
        }
    }
}

#[test]
fn a_double_click_to_a_closed_board_does_nothing() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    link(&mut app, stash, bag);
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));
    app.world_mut()
        .get_mut::<InventoryWindow>(bag)
        .unwrap()
        .open = false;
    app.update();

    double_click(&mut app, stash, UVec2::new(0, 0));

    assert_eq!(parent_of(&app, item), stash);
    let actions = drain_actions(&mut app);
    assert!(!actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Transferred { .. })));
}

#[test]
fn a_double_click_to_a_full_board_is_rejected() {
    let mut app = new_app();
    let bag = spawn_board(
        &mut app,
        InventoryBoardSpec {
            config: InventoryConfig {
                cols: 1,
                rows: 1,
                ..default()
            },
            ..default()
        },
    );
    spawn_test_item(&mut app, bag, pistol(), UVec2::new(0, 0)); // fills the 1x1
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    link(&mut app, stash, bag);
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    double_click(&mut app, stash, UVec2::new(0, 0));

    assert_eq!(parent_of(&app, item), stash);
    assert_eq!(grid_at(&app, stash, UVec2::new(0, 0)), Some(item));
    let actions = drain_actions(&mut app);
    assert!(actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Rejected { board, .. } if *board == stash)));
}

#[test]
fn two_slow_clicks_are_not_a_double_click() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    link(&mut app, stash, bag);
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, stash, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);
    advance(&mut app, 1.0);
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);

    assert_eq!(parent_of(&app, item), stash);
    let actions = drain_actions(&mut app);
    assert!(!actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Activated { .. })));
}

#[test]
fn a_press_that_became_a_drag_does_not_arm_a_double_click() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    link(&mut app, stash, bag);
    spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    hover_cell(&mut app, stash, UVec2::new(0, 0));
    press(&mut app, MouseButton::Left);
    hover_cell(&mut app, stash, UVec2::new(2, 2));
    app.update(); // update_drag crosses the threshold, clears InventoryClicks
    release(&mut app, MouseButton::Left);

    hover_cell(&mut app, stash, UVec2::new(2, 2));
    press(&mut app, MouseButton::Left);
    release(&mut app, MouseButton::Left);

    let actions = drain_actions(&mut app);
    assert!(!actions
        .iter()
        .any(|a| matches!(a, InventoryAction::Activated { .. })));
}

#[test]
fn a_landed_quick_transfer_leaves_no_drag_armed() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    link(&mut app, stash, bag);
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    double_click(&mut app, stash, UVec2::new(0, 0));

    assert!(matches!(
        *app.world().get::<InventoryDragState>(stash).unwrap(),
        InventoryDragState::Idle
    ));
    assert!(matches!(
        *app.world().get::<InventoryDragState>(bag).unwrap(),
        InventoryDragState::Idle
    ));

    let before = slot_of(&app, item);
    release(&mut app, MouseButton::Left);
    assert_eq!(slot_of(&app, item), before);
}

#[test]
fn transfer_moves_an_item_between_boards_without_respawning_it() {
    let mut app = new_app();
    let bag = spawn_board(&mut app, InventoryBoardSpec::default());
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    let moved = app
        .world_mut()
        .run_system_once(move |mut inv: InventoryCommands| inv.transfer(item, bag))
        .unwrap();
    let actions = drain_actions(&mut app);

    assert_eq!(moved, Some(UVec2::new(0, 0)));
    assert_eq!(parent_of(&app, item), bag);
    assert_eq!(grid_at(&app, bag, UVec2::new(0, 0)), Some(item));
    assert_eq!(grid_at(&app, stash, UVec2::new(0, 0)), None);
    assert!(app.world().get_entity(item).is_ok());
    assert!(actions.iter().any(|a| matches!(
        a,
        InventoryAction::Transferred { from_board, to_board, .. }
            if *from_board == stash && *to_board == bag
    )));
}

#[test]
fn transfer_to_a_full_board_leaves_it_put() {
    let mut app = new_app();
    let bag = spawn_board(
        &mut app,
        InventoryBoardSpec {
            config: InventoryConfig {
                cols: 1,
                rows: 1,
                ..default()
            },
            ..default()
        },
    );
    spawn_test_item(&mut app, bag, pistol(), UVec2::new(0, 0)); // fills the 1x1
    let stash = spawn_board(&mut app, InventoryBoardSpec::default());
    let item = spawn_test_item(&mut app, stash, pistol(), UVec2::new(0, 0));

    let moved = app
        .world_mut()
        .run_system_once(move |mut inv: InventoryCommands| inv.transfer(item, bag))
        .unwrap();

    assert_eq!(moved, None);
    assert_eq!(parent_of(&app, item), stash);
    assert_eq!(grid_at(&app, stash, UVec2::new(0, 0)), Some(item));
}
