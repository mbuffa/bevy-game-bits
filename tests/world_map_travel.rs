//! Headless behaviour tests for `bevy_game_bits::world_map`.
//!
//! `MinimalPlugins` only — no `AssetPlugin` (so maps come in as
//! [`WorldMapSource::Inline`]), no `UiPlugin`, no `InputPlugin`. The cursor is
//! written onto the map entity by hand (the way `track_cursor` would), the
//! mouse and keyboard `ButtonInput` resources are driven and `.clear()`-ed
//! between frames, and `Time<Virtual>` is paused so each `advance` + `update`
//! is one deterministic frame delta.

use std::time::Duration;

use bevy::app::PluginGroup;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::time::TimePlugin;

use bevy_game_bits::world_map::prelude::*;
use bevy_game_bits::world_map::{tile_to_world, AsideRow, TravelProgress};

/// `TimePlugin` is disabled so the generic `Time` resource is ours to drive:
/// its `time_system` would otherwise clobber a manual `advance_by` back to a
/// zero delta every frame (virtual time paused), and these tests need an
/// exact per-frame delta to check travel distances.
/// Accumulates every [`WorldMapAction`] across frames — a persistent
/// `MessageReader` in one registered system, so each message is seen exactly
/// once (a fresh `RunSystemOnce` reader would re-read messages still inside
/// the two-frame buffer window).
#[derive(Resource, Default)]
struct ActionLog(Vec<WorldMapAction>);

fn collect_actions(mut log: ResMut<ActionLog>, mut reader: MessageReader<WorldMapAction>) {
    log.0.extend(reader.read().cloned());
}

fn new_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.build().disable::<TimePlugin>())
        .add_plugins(WorldMapPlugin::headless())
        .init_resource::<Time>()
        .init_resource::<ActionLog>()
        .insert_resource(ButtonInput::<MouseButton>::default())
        .insert_resource(ButtonInput::<KeyCode>::default())
        .add_systems(Update, collect_actions.after(WorldMapSet::Sync));
    app.update();
    app
}

fn advance(app: &mut App, secs: f32) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(secs));
    app.update();
}

/// Spawns a map from inline data and runs frames until it has resolved.
fn spawn_map(app: &mut App, data: WorldMapData) -> Entity {
    let source = WorldMapSource::Inline(Box::new(data));
    let map = app
        .world_mut()
        .run_system_once(move |mut commands: Commands| {
            spawn_world_map(
                &mut commands,
                WorldMapSpec {
                    source: source.clone(),
                    ..default()
                },
            )
        })
        .unwrap();
    for _ in 0..4 {
        app.update();
    }
    assert!(
        app.world().get::<WorldMapGrid>(map).is_some(),
        "map should have resolved"
    );
    map
}

fn traveler_of(app: &mut App, map: Entity) -> Entity {
    app.world_mut()
        .run_system_once(move |q: Query<(Entity, &Traveler)>| {
            q.iter()
                .find_map(|(e, t)| (t.map == map).then_some(e))
                .expect("traveller exists")
        })
        .unwrap()
}

fn set_cursor_tile(app: &mut App, map: Entity, tile: Vec2) {
    let grid = app.world().get::<WorldMapGrid>(map).unwrap();
    let world = tile_to_world(tile, grid.size_px(), grid.tile_px());
    let mut cursor = app.world_mut().get_mut::<WorldMapCursor>(map).unwrap();
    cursor.world = world;
    cursor.tile = tile;
    cursor.valid = true;
}

fn set_tile_pos(app: &mut App, traveler: Entity, tile: Vec2) {
    app.world_mut().get_mut::<TilePos>(traveler).unwrap().0 = tile;
}

fn tile_pos(app: &mut App, traveler: Entity) -> Vec2 {
    app.world().get::<TilePos>(traveler).unwrap().0
}

fn target(app: &mut App, traveler: Entity) -> Option<Vec2> {
    app.world().get::<TravelTarget>(traveler).unwrap().0
}

fn progress(app: &mut App, traveler: Entity) -> TravelProgress {
    *app.world().get::<TravelProgress>(traveler).unwrap()
}

fn click_left(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    buttons.clear();
    buttons.release(MouseButton::Left);
}

fn press_space(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Space);
    app.update();
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.clear();
    keys.release(KeyCode::Space);
}

fn drain_actions(app: &mut App) -> Vec<WorldMapAction> {
    std::mem::take(&mut app.world_mut().resource_mut::<ActionLog>().0)
}

#[test]
fn inline_map_resolves_and_announces_itself() {
    let mut app = new_app();
    let source = WorldMapSource::Inline(Box::new(WorldMapData::demo()));
    let map = app
        .world_mut()
        .run_system_once(move |mut commands: Commands| {
            spawn_world_map(
                &mut commands,
                WorldMapSpec {
                    source: source.clone(),
                    ..default()
                },
            )
        })
        .unwrap();

    // First frame after spawn: resolve_map runs, grid + traveller appear.
    app.update();
    assert!(app.world().get::<WorldMapGrid>(map).is_some());
    let actions = drain_actions(&mut app);
    assert!(actions.contains(&WorldMapAction::Loaded { map }));
}

#[test]
fn a_click_sets_a_target() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    drain_actions(&mut app);

    set_cursor_tile(&mut app, map, Vec2::new(3.5, 5.5));
    click_left(&mut app);

    assert_eq!(target(&mut app, traveler), Some(Vec2::new(3.5, 5.5)));
    let actions = drain_actions(&mut app);
    assert!(actions.iter().any(|a| matches!(
        a,
        WorldMapAction::TargetSet { traveler: t, tile, .. }
            if *t == traveler && *tile == Vec2::new(3.5, 5.5)
    )));
}

#[test]
fn the_traveller_reaches_its_target_once() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    set_tile_pos(&mut app, traveler, Vec2::new(0.5, 5.5));

    set_cursor_tile(&mut app, map, Vec2::new(2.5, 5.5)); // 2 tiles east, all plains
    click_left(&mut app);
    drain_actions(&mut app);

    // Plenty of time to cover 2 tiles at ~1.2 tiles/s.
    let mut arrivals = 0;
    for _ in 0..40 {
        advance(&mut app, 0.1);
        for a in drain_actions(&mut app) {
            if matches!(a, WorldMapAction::Arrived { .. }) {
                arrivals += 1;
            }
        }
    }
    assert_eq!(arrivals, 1);
    assert_eq!(target(&mut app, traveler), None);
    assert!((tile_pos(&mut app, traveler) - Vec2::new(2.5, 5.5)).length() < 1.0e-3);
}

#[test]
fn slow_terrain_takes_proportionally_longer() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);

    // One frame of travel over plains (cell 0,5 -> speed 1.0).
    set_tile_pos(&mut app, traveler, Vec2::new(0.5, 5.5));
    app.world_mut().get_mut::<TravelTarget>(traveler).unwrap().0 = Some(Vec2::new(5.5, 5.5));
    advance(&mut app, 0.05);
    let plains_step = progress(&mut app, traveler).tiles_this_frame;

    // One frame of travel over water (cell 2,2 -> speed 0.3 in the demo).
    set_tile_pos(&mut app, traveler, Vec2::new(2.5, 2.5));
    app.world_mut().get_mut::<TravelTarget>(traveler).unwrap().0 = Some(Vec2::new(2.5, 5.5));
    advance(&mut app, 0.05);
    let water_step = progress(&mut app, traveler).tiles_this_frame;

    assert!(plains_step > 0.0 && water_step > 0.0);
    let ratio = water_step / plains_step;
    assert!((ratio - 0.3).abs() < 1.0e-3, "ratio was {ratio}");
}

#[test]
fn space_cancels_and_a_second_click_retargets() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    set_tile_pos(&mut app, traveler, Vec2::new(0.5, 5.5));

    set_cursor_tile(&mut app, map, Vec2::new(5.5, 5.5));
    click_left(&mut app);
    drain_actions(&mut app);
    advance(&mut app, 0.1);
    assert!(target(&mut app, traveler).is_some());

    press_space(&mut app);
    assert_eq!(target(&mut app, traveler), None);
    let actions = drain_actions(&mut app);
    assert!(actions
        .iter()
        .any(|a| matches!(a, WorldMapAction::TargetCleared { .. })));

    // Mid-journey retarget to a different, still-distant tile.
    set_cursor_tile(&mut app, map, Vec2::new(4.5, 5.5));
    click_left(&mut app);
    assert_eq!(target(&mut app, traveler), Some(Vec2::new(4.5, 5.5)));
}

#[test]
fn stepping_onto_a_location_is_reported_and_the_widget_can_be_entered() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);

    // Haven sits at cell (1,1) in the demo. Walk there.
    set_tile_pos(&mut app, traveler, Vec2::new(1.5, 2.5));
    set_cursor_tile(&mut app, map, Vec2::new(1.5, 1.5));
    click_left(&mut app);
    drain_actions(&mut app);

    let mut reached = None;
    for _ in 0..60 {
        advance(&mut app, 0.1);
        for a in drain_actions(&mut app) {
            if let WorldMapAction::LocationReached { location, .. } = a {
                reached = Some(location);
            }
        }
        if reached.is_some() {
            break;
        }
    }
    let location = reached.expect("should have reached Haven");
    assert_eq!(
        app.world().get::<AtLocation>(traveler).unwrap().0,
        Some(location)
    );

    // A click right on the token now means "enter", not "walk one over".
    let here = tile_pos(&mut app, traveler);
    set_cursor_tile(&mut app, map, here);
    click_left(&mut app);
    let actions = drain_actions(&mut app);
    assert!(actions.iter().any(|a| matches!(
        a,
        WorldMapAction::EnterRequested { location: l, .. } if *l == location
    )));
    assert_eq!(target(&mut app, traveler), None);

    // Walking off the tile reports LocationLeft.
    set_tile_pos(&mut app, traveler, Vec2::new(4.5, 4.5));
    app.update();
    let actions = drain_actions(&mut app);
    assert!(actions.iter().any(|a| matches!(
        a,
        WorldMapAction::LocationLeft { location: l, .. } if *l == location
    )));
}

#[test]
fn clicking_an_aside_row_sends_the_traveller_there() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    set_tile_pos(&mut app, traveler, Vec2::new(0.5, 5.5));
    drain_actions(&mut app);

    // Haven sits at cell (1,1) in the demo and is discovered, so it has a row.
    let row = app
        .world_mut()
        .run_system_once(
            move |rows: Query<(Entity, &AsideRow)>, locations: Query<&Location>| {
                rows.iter()
                    .find_map(|(e, r)| {
                        let is_haven = locations
                            .get(r.location)
                            .map(|l| l.id == "haven")
                            .unwrap_or(false);
                        (r.map == map && is_haven).then_some(e)
                    })
                    .expect("Haven has an aside row")
            },
        )
        .unwrap();

    // Press the row and click a *different* map tile in the same frame: the
    // click's ui-node guard must see the pressed row and bail, so the row wins.
    app.world_mut().entity_mut(row).insert(Interaction::Pressed);
    set_cursor_tile(&mut app, map, Vec2::new(5.5, 5.5));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();

    let haven = Vec2::new(1.5, 1.5);
    assert_eq!(target(&mut app, traveler), Some(haven));
    let actions = drain_actions(&mut app);
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(a, WorldMapAction::TargetSet { .. }))
            .count(),
        1,
        "exactly one TargetSet — handle_click bails on the pressed row"
    );
    assert!(actions
        .iter()
        .any(|a| matches!(a, WorldMapAction::LocationFocused { .. })));

    // And the traveller actually starts closing the distance.
    let before = tile_pos(&mut app, traveler).distance(haven);
    advance(&mut app, 0.5);
    assert!(tile_pos(&mut app, traveler).distance(haven) < before);
}

#[test]
fn passing_close_reveals_a_hidden_location_once() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);

    // The Ford sits at cell (4,4) in the demo, undiscovered.
    let ford = app
        .world_mut()
        .run_system_once(move |q: Query<(Entity, &Location, &Discovered)>| {
            q.iter()
                .find_map(|(e, l, d)| (l.id == "ford" && !d.0).then_some(e))
                .expect("undiscovered Ford")
        })
        .unwrap();

    set_tile_pos(&mut app, traveler, Vec2::new(4.5, 4.5));
    let mut discoveries = 0;
    for _ in 0..3 {
        app.update();
        for a in drain_actions(&mut app) {
            if matches!(a, WorldMapAction::LocationDiscovered { location, .. } if location == ford)
            {
                discoveries += 1;
            }
        }
    }
    assert_eq!(discoveries, 1);
    assert!(app.world().get::<Discovered>(ford).unwrap().0);
}
