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
use bevy_game_bits::world_map::{
    interact_widget_center, tile_to_world, AsideRow, InteractMenuRow, InteractSubject,
    SecretLocation, TravelProgress,
};

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
        .run_system_once(move |q: Query<(Entity, &Traveler), With<PlayerTraveler>>| {
            q.iter()
                .find_map(|(e, t)| (t.map == map).then_some(e))
                .expect("player traveller exists")
        })
        .unwrap()
}

/// Spawns a host-driven party on `map` and returns its entity.
fn spawn_party(app: &mut App, map: Entity, tile: Vec2, speed: f32) -> Entity {
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands
                .spawn((
                    party_bundle(
                        map,
                        tile,
                        Party {
                            name: "Caravan".to_string(),
                            color: Color::WHITE,
                        },
                    ),
                    TravelSpeed(speed),
                ))
                .id()
        })
        .unwrap()
}

/// Pins `WorldClockHold` on the map so world time runs even while the player is
/// idle (the host's timed-action seam).
fn hold_clock(app: &mut App, map: Entity) {
    app.world_mut().entity_mut(map).insert(WorldClockHold);
}

fn world_time(app: &mut App, map: Entity) -> WorldMapTime {
    *app.world().get::<WorldMapTime>(map).unwrap()
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

fn set_cursor_world(app: &mut App, map: Entity, world: Vec2) {
    let (size, tile_px) = {
        let grid = app.world().get::<WorldMapGrid>(map).unwrap();
        (grid.size_px(), grid.tile_px())
    };
    let mut cursor = app.world_mut().get_mut::<WorldMapCursor>(map).unwrap();
    cursor.world = world;
    cursor.tile = world_to_tile(world, size, tile_px);
    cursor.valid = true;
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

    // A click on the interact-widget square (above the token) means "enter",
    // not "walk one over".
    let (size, tile_px) = {
        let g = app.world().get::<WorldMapGrid>(map).unwrap();
        (g.size_px(), g.tile_px())
    };
    let config = app.world().get::<WorldMapConfig>(map).unwrap().clone();
    let token_world = tile_to_world(tile_pos(&mut app, traveler), size, tile_px);
    let widget = interact_widget_center(token_world, &config);
    set_cursor_world(&mut app, map, widget);
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

/// The demo's `cache` — a secret given as the float point `(5.25, 0.25)`.
fn secret_of(app: &mut App) -> Entity {
    app.world_mut()
        .run_system_once(|q: Query<(Entity, &Location, Has<SecretLocation>)>| {
            q.iter()
                .find_map(|(e, l, secret)| (l.id == "cache" && secret).then_some(e))
                .expect("the demo's secret cache, marked SecretLocation")
        })
        .unwrap()
}

fn discoveries_of(actions: &[WorldMapAction], location: Entity) -> usize {
    actions
        .iter()
        .filter(|a| matches!(a, WorldMapAction::LocationDiscovered { location: l, .. } if *l == location))
        .count()
}

#[test]
fn walking_near_a_secret_does_not_reveal_it() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    let cache = secret_of(&mut app);

    // 0.4 tiles away — well inside the 1.6-tile ordinary reveal radius, far
    // outside the 0.15-tile secret one.
    set_tile_pos(&mut app, traveler, Vec2::new(5.25, 0.65));
    drain_actions(&mut app);
    for _ in 0..3 {
        app.update();
    }

    assert_eq!(discoveries_of(&drain_actions(&mut app), cache), 0);
    assert!(!app.world().get::<Discovered>(cache).unwrap().0);
}

#[test]
fn crossing_a_secret_reveals_it_once() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    let cache = secret_of(&mut app);

    // A straight line along y = 0.25 from x = 4 to x = 7, passing exactly
    // through the cache at (5.25, 0.25).
    set_tile_pos(&mut app, traveler, Vec2::new(4.0, 0.25));
    app.world_mut().get_mut::<TravelTarget>(traveler).unwrap().0 = Some(Vec2::new(7.0, 0.25));
    drain_actions(&mut app);

    // One deliberately long frame: the traveller steps from x≈4 to x≈6.4, so
    // the *end point* is 1.15 tiles from the cache — only the swept segment
    // catches it.
    advance(&mut app, 2.0);
    let after_jump = tile_pos(&mut app, traveler);
    assert!(
        after_jump.distance(Vec2::new(5.25, 0.25)) > 0.15,
        "the frame must overshoot the cache, landed at {after_jump}"
    );
    let mut total = discoveries_of(&drain_actions(&mut app), cache);

    for _ in 0..10 {
        advance(&mut app, 0.1);
        total += discoveries_of(&drain_actions(&mut app), cache);
    }
    assert_eq!(total, 1);
    assert!(app.world().get::<Discovered>(cache).unwrap().0);
}

#[test]
fn an_undiscovered_location_is_never_reached() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let traveler = traveler_of(&mut app, map);
    let cache = secret_of(&mut app);

    // Standing in the cache's own cell (5,0) but ~0.5 tiles from its point —
    // inside the cell, outside the reveal radius.
    set_tile_pos(&mut app, traveler, Vec2::new(5.6, 0.6));
    drain_actions(&mut app);
    app.update();

    assert!(!app.world().get::<Discovered>(cache).unwrap().0);
    assert_eq!(app.world().get::<AtLocation>(traveler).unwrap().0, None);
    assert!(
        !drain_actions(&mut app)
            .iter()
            .any(|a| matches!(a, WorldMapAction::LocationReached { .. })),
        "an unseen location is not somewhere you've reached"
    );
}

#[test]
fn a_click_does_not_steer_parties() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    let party = spawn_party(&mut app, map, Vec2::new(3.5, 3.5), 1.0);
    drain_actions(&mut app);

    set_cursor_tile(&mut app, map, Vec2::new(5.5, 5.5));
    click_left(&mut app);

    assert_eq!(target(&mut app, player), Some(Vec2::new(5.5, 5.5)));
    assert_eq!(
        target(&mut app, party),
        None,
        "the click must not touch a host-driven party"
    );
}

#[test]
fn a_party_walks_at_its_own_speed() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let party = spawn_party(&mut app, map, Vec2::new(0.5, 5.5), 0.6);
    // The player is idle, so hold the clock open — otherwise world time (and the
    // party) is frozen.
    hold_clock(&mut app, map);

    // Plains: the full TravelSpeed, independent of the config's 1.2 tiles/s.
    app.world_mut().get_mut::<TravelTarget>(party).unwrap().0 = Some(Vec2::new(5.5, 5.5));
    advance(&mut app, 0.05);
    let plains = progress(&mut app, party).tiles_this_frame;
    assert!((plains - 0.6 * 0.05).abs() < 1.0e-4, "plains step {plains}");

    // Water (speed 0.3 in the demo) scales it the same way it scales the player.
    set_tile_pos(&mut app, party, Vec2::new(2.5, 2.5));
    app.world_mut().get_mut::<TravelTarget>(party).unwrap().0 = Some(Vec2::new(2.5, 5.5));
    advance(&mut app, 0.05);
    let water = progress(&mut app, party).tiles_this_frame;
    assert!(
        (water / plains - 0.3).abs() < 1.0e-3,
        "ratio {}",
        water / plains
    );
}

#[test]
fn intercepting_a_party_fires_once_then_ends_on_separation() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    let party = spawn_party(&mut app, map, Vec2::new(2.5, 5.5), 1.0);
    set_tile_pos(&mut app, player, Vec2::new(2.5, 5.5));
    drain_actions(&mut app);

    app.update();
    let (mut intercepts, mut leaves) = (0, 0);
    for a in drain_actions(&mut app) {
        match a {
            WorldMapAction::PartyIntercepted { party: p, .. } if p == party => intercepts += 1,
            WorldMapAction::PartyLeft { .. } => leaves += 1,
            _ => {}
        }
    }
    assert_eq!((intercepts, leaves), (1, 0));
    assert_eq!(
        app.world().get::<Intercepting>(player).unwrap().0,
        vec![party]
    );

    // Holding contact doesn't re-fire.
    app.update();
    assert!(!drain_actions(&mut app)
        .iter()
        .any(|a| matches!(a, WorldMapAction::PartyIntercepted { .. })));

    // Separating fires PartyLeft.
    set_tile_pos(&mut app, party, Vec2::new(5.5, 5.5));
    app.update();
    assert!(drain_actions(&mut app)
        .iter()
        .any(|a| matches!(a, WorldMapAction::PartyLeft { party: p, .. } if *p == party)));
    assert!(app
        .world()
        .get::<Intercepting>(player)
        .unwrap()
        .0
        .is_empty());
}

#[test]
fn a_fast_crossing_still_intercepts() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    let party = spawn_party(&mut app, map, Vec2::new(5.5, 5.5), 2.0);

    set_tile_pos(&mut app, player, Vec2::new(0.5, 5.5));
    app.world_mut().get_mut::<TravelTarget>(player).unwrap().0 = Some(Vec2::new(5.5, 5.5));
    app.world_mut().get_mut::<TravelTarget>(party).unwrap().0 = Some(Vec2::new(0.5, 5.5));
    drain_actions(&mut app);

    // One big frame: they swap ends, passing through each other mid-step.
    advance(&mut app, 5.0);

    let (p, q) = (tile_pos(&mut app, player), tile_pos(&mut app, party));
    assert!(p.distance(q) > 4.0, "end points far apart: {p} vs {q}");
    assert!(
        drain_actions(&mut app).iter().any(
            |a| matches!(a, WorldMapAction::PartyIntercepted { party: pp, .. } if *pp == party)
        ),
        "the swept test catches a crossing the end points miss"
    );
}

#[test]
fn a_party_does_not_reveal_locations() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    set_tile_pos(&mut app, player, Vec2::new(0.5, 0.5));

    let ford = app
        .world_mut()
        .run_system_once(|q: Query<(Entity, &Location, &Discovered)>| {
            q.iter()
                .find_map(|(e, l, d)| (l.id == "ford" && !d.0).then_some(e))
                .expect("undiscovered Ford")
        })
        .unwrap();

    // Park the party right on the Ford — reveal only ever considers the player.
    let _party = spawn_party(&mut app, map, Vec2::new(4.5, 4.5), 1.0);
    drain_actions(&mut app);
    for _ in 0..3 {
        advance(&mut app, 0.1);
    }

    assert!(!app.world().get::<Discovered>(ford).unwrap().0);
    assert!(!drain_actions(&mut app).iter().any(
        |a| matches!(a, WorldMapAction::LocationDiscovered { location, .. } if *location == ford)
    ));
}

#[test]
fn time_freezes_while_the_player_is_idle() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let party = spawn_party(&mut app, map, Vec2::new(1.5, 5.5), 1.0);
    app.world_mut().get_mut::<TravelTarget>(party).unwrap().0 = Some(Vec2::new(5.5, 5.5));

    let before = tile_pos(&mut app, party);
    for _ in 0..5 {
        advance(&mut app, 0.5);
    }

    assert_eq!(
        tile_pos(&mut app, party),
        before,
        "a caravan doesn't move while the player stands still"
    );
    assert_eq!(world_time(&mut app, map).delta_secs, 0.0);
    assert_eq!(world_time(&mut app, map).elapsed_secs, 0.0);
}

#[test]
fn time_runs_while_the_player_travels() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    let party = spawn_party(&mut app, map, Vec2::new(1.5, 5.5), 1.0);

    set_tile_pos(&mut app, player, Vec2::new(0.5, 5.5));
    app.world_mut().get_mut::<TravelTarget>(player).unwrap().0 = Some(Vec2::new(4.5, 5.5));
    app.world_mut().get_mut::<TravelTarget>(party).unwrap().0 = Some(Vec2::new(5.5, 5.5));

    let before = tile_pos(&mut app, party);
    advance(&mut app, 0.3);

    assert!(
        tile_pos(&mut app, party).x > before.x + 1.0e-4,
        "the caravan moves while the player travels"
    );
    assert!(world_time(&mut app, map).elapsed_secs > 0.0);
}

#[test]
fn pause_time_when_idle_false_keeps_parties_moving() {
    let mut app = new_app();
    let source = WorldMapSource::Inline(Box::new(WorldMapData::demo()));
    let map = app
        .world_mut()
        .run_system_once(move |mut commands: Commands| {
            spawn_world_map(
                &mut commands,
                WorldMapSpec {
                    source: source.clone(),
                    config: WorldMapConfig {
                        pause_time_when_idle: false,
                        ..default()
                    },
                    ..default()
                },
            )
        })
        .unwrap();
    for _ in 0..4 {
        app.update();
    }

    let party = spawn_party(&mut app, map, Vec2::new(1.5, 5.5), 1.0);
    app.world_mut().get_mut::<TravelTarget>(party).unwrap().0 = Some(Vec2::new(5.5, 5.5));
    let before = tile_pos(&mut app, party);
    advance(&mut app, 0.3);

    assert!(
        tile_pos(&mut app, party).x > before.x + 1.0e-4,
        "with pause_time_when_idle=false the world runs even while the player is idle"
    );
}

/// World-space centre of the interact square over `traveler`.
fn widget_center(app: &mut App, map: Entity, traveler: Entity) -> Vec2 {
    let (size, tile_px) = {
        let g = app.world().get::<WorldMapGrid>(map).unwrap();
        (g.size_px(), g.tile_px())
    };
    let config = app.world().get::<WorldMapConfig>(map).unwrap().clone();
    let token_world = tile_to_world(tile_pos(app, traveler), size, tile_px);
    interact_widget_center(token_world, &config)
}

/// The open menu's rows for `map`, as `(row entity, subject)`.
fn menu_rows(app: &mut App, map: Entity) -> Vec<(Entity, InteractSubject)> {
    app.world_mut()
        .run_system_once(move |q: Query<(Entity, &InteractMenuRow)>| {
            q.iter()
                .filter(|(_, r)| r.map == map)
                .map(|(e, r)| (e, r.subject))
                .collect::<Vec<_>>()
        })
        .unwrap()
}

#[test]
fn one_square_pops_a_menu_when_two_subjects_are_in_reach() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    set_tile_pos(&mut app, player, Vec2::new(1.5, 1.5)); // Haven's cell, discovered
    let party = spawn_party(&mut app, map, Vec2::new(1.5, 1.5), 1.0);

    app.update();
    let haven = app
        .world()
        .get::<AtLocation>(player)
        .unwrap()
        .0
        .expect("standing on Haven");
    assert_eq!(
        app.world().get::<Intercepting>(player).unwrap().0,
        vec![party]
    );
    drain_actions(&mut app);

    // A click on the one square with two things in reach opens the menu and
    // acts on nothing yet.
    let center = widget_center(&mut app, map, player);
    set_cursor_world(&mut app, map, center);
    click_left(&mut app);
    app.update(); // let sync_interact_menu spawn the rows

    let acts = drain_actions(&mut app);
    assert!(!acts.iter().any(|a| matches!(
        a,
        WorldMapAction::EnterRequested { .. } | WorldMapAction::InteractRequested { .. }
    )));
    assert!(app.world().get::<InteractMenu>(map).unwrap().is_open());

    let rows = menu_rows(&mut app, map);
    assert_eq!(rows.len(), 2, "one row per subject");
    let (party_row, _) = rows
        .iter()
        .find(|(_, s)| matches!(s, InteractSubject::Party(p) if *p == party))
        .copied()
        .expect("a Hail row for the party");
    assert!(rows
        .iter()
        .any(|(_, s)| matches!(s, InteractSubject::Location(l) if *l == haven)));

    // Pressing the party's row (no UiPlugin, so drive Interaction by hand) hails
    // it and closes the menu.
    app.world_mut()
        .entity_mut(party_row)
        .insert(Interaction::Pressed);
    app.update();
    let acts = drain_actions(&mut app);
    assert!(acts
        .iter()
        .any(|a| matches!(a, WorldMapAction::InteractRequested { party: p, .. } if *p == party)));
    assert!(!app.world().get::<InteractMenu>(map).unwrap().is_open());
    assert!(menu_rows(&mut app, map).is_empty(), "rows despawn on close");
}

#[test]
fn two_parties_in_reach_are_both_intercepted() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    set_tile_pos(&mut app, player, Vec2::new(3.5, 5.5));
    let a = spawn_party(&mut app, map, Vec2::new(3.5, 5.5), 1.0);
    let b = spawn_party(&mut app, map, Vec2::new(3.5, 5.5), 1.0);
    drain_actions(&mut app);

    app.update();
    let intercepted = app.world().get::<Intercepting>(player).unwrap().0.clone();
    assert_eq!(intercepted.len(), 2);
    assert!(intercepted.contains(&a) && intercepted.contains(&b));
    let hits = drain_actions(&mut app)
        .iter()
        .filter(|x| matches!(x, WorldMapAction::PartyIntercepted { .. }))
        .count();
    assert_eq!(hits, 2, "one PartyIntercepted per party");

    // One walks off — exactly one PartyLeft, the other stays.
    set_tile_pos(&mut app, b, Vec2::new(0.5, 0.5));
    app.update();
    let leaves: Vec<Entity> = drain_actions(&mut app)
        .iter()
        .filter_map(|x| match x {
            WorldMapAction::PartyLeft { party, .. } => Some(*party),
            _ => None,
        })
        .collect();
    assert_eq!(leaves, vec![b]);
    assert_eq!(app.world().get::<Intercepting>(player).unwrap().0, vec![a]);
}

#[test]
fn an_outside_click_closes_the_menu_without_travelling() {
    let mut app = new_app();
    let map = spawn_map(&mut app, WorldMapData::demo());
    let player = traveler_of(&mut app, map);
    set_tile_pos(&mut app, player, Vec2::new(1.5, 1.5));
    let _party = spawn_party(&mut app, map, Vec2::new(1.5, 1.5), 1.0);
    app.update();

    // Open the menu.
    let center = widget_center(&mut app, map, player);
    set_cursor_world(&mut app, map, center);
    click_left(&mut app);
    app.update();
    assert!(app.world().get::<InteractMenu>(map).unwrap().is_open());
    drain_actions(&mut app);

    // A click on a far tile just closes it — no course set.
    set_cursor_tile(&mut app, map, Vec2::new(5.5, 5.5));
    click_left(&mut app);
    app.update();

    assert!(!app.world().get::<InteractMenu>(map).unwrap().is_open());
    assert_eq!(target(&mut app, player), None);
    assert!(!drain_actions(&mut app)
        .iter()
        .any(|a| matches!(a, WorldMapAction::TargetSet { .. })));
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
