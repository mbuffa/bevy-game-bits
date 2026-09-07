//! The `bevy_game_bits::world_map` overworld, on its own.
//!
//! A Fallout 1/2-style world map: a scrollable tile grid with a sidebar of
//! known places. Left-click anywhere to send the traveller there; it walks in
//! a straight line and slows to a crawl through forest, swamp and mountains,
//! so the fast route is the one *you* pick around them. `Space` cancels the
//! current trip. The arrow keys pan the map; while you're travelling the
//! camera follows the token until you pan away. Walk onto a town and a green
//! square appears over the token — click it to "enter" (this example just
//! logs it). When more than one thing is in reach — a town *and* a caravan
//! standing on it — that one square pops a small menu to pick from; `Escape`
//! closes it. Click a row in the sidebar to set course for that place.
//! Walk near the hidden ruin and it appears in the sidebar.
//!
//! Two of the map's places are *secret* — a wrecked convoy near `6.35, 7.80`
//! and a spring near `3.15, 1.60`. A real game would hand those coordinates to
//! you as a quest; here they're written above. You only uncover one by walking
//! almost exactly over the point, so aim with the live `you … · cursor …`
//! readout in the sidebar and click the spot precisely.
//!
//! Two parties roam the map — a slow **Water Caravan** shuttling between Shady
//! Sands and Junktown, and fast **Raiders** cycling the middle. They're the
//! *host's* to drive: this example gives each a hand-rolled `Patrol` and a
//! `spot_parties` stand-in for the eventual "you spot them at a distance" rule
//! (they stay hidden until you're within 3 tiles). Walk into one and the
//! interact square's menu gains a "Hail" row for it. You start on Shady Sands
//! with the Water Caravan on top of you, so the very first click on the square
//! already pops a two-row menu.
//!
//! **The world only moves while you do.** Stop, and the caravans freeze with
//! you (and the clock stops) — so line one up and step forward to intercept it.
//! Hold **W** to "wait": that drops a `WorldClockHold` on the map so time keeps
//! flowing while you stand still, and you can watch the caravans move.
//!
//! The map, its terrain palette and its locations all come from
//! `assets/maps/wastes.worldmap.json` — the library never sees a hard-coded
//! tile. The clock in the sidebar (`WorldMapTimePlugin`) rides on `WorldMapTime`;
//! a host with its own `GameTime` would leave that plugin out and read
//! `WorldMapTime` / `TravelProgress` instead.
//!
//! Everything specific to *this* map lives here: the camera, the party AI, the
//! wait key, and the mapping from `WorldMapAction` to log lines.

use bevy::prelude::*;
use bevy_game_bits::world_map::prelude::*;

const CLEAR_COLOR: Color = Color::srgb(0.03, 0.04, 0.05);

fn main() {
    App::new()
        .insert_resource(ClearColor(CLEAR_COLOR))
        .add_plugins(DefaultPlugins)
        .add_plugins(WorldMapPlugin::default())
        .add_plugins(WorldMapTimePlugin)
        .add_systems(Startup, spawn_camera)
        .add_systems(
            Update,
            (
                spawn_parties,
                steer_parties,
                spot_parties,
                wait_key,
                log_world_map_actions,
            ),
        )
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, WorldMapCamera));
}

/// A hand-rolled patrol route — the entirety of this example's "AI". The
/// library ships none; a real game's factions would live here.
#[derive(Component)]
struct Patrol {
    waypoints: Vec<Vec2>,
    next: usize,
}

/// Once the map is up, drop two host-driven parties onto it. They start
/// unspotted so `spot_parties` has something to reveal.
fn spawn_parties(mut actions: MessageReader<WorldMapAction>, mut commands: Commands) {
    for action in actions.read() {
        let WorldMapAction::Loaded { map } = action else {
            continue;
        };
        let map = *map;

        commands.spawn((
            party_bundle(
                map,
                Vec2::new(1.5, 8.5),
                Party {
                    name: "Water Caravan".to_string(),
                    color: Color::srgb(0.4, 0.75, 0.95),
                },
            ),
            Spotted(false),
            TravelSpeed(0.8),
            Patrol {
                waypoints: vec![Vec2::new(1.5, 8.5), Vec2::new(8.5, 1.5)],
                next: 1,
            },
        ));

        commands.spawn((
            party_bundle(
                map,
                Vec2::new(5.0, 5.0),
                Party {
                    name: "Raiders".to_string(),
                    color: Color::srgb(0.9, 0.4, 0.3),
                },
            ),
            Spotted(false),
            TravelSpeed(1.5),
            Patrol {
                waypoints: vec![
                    Vec2::new(5.0, 5.0),
                    Vec2::new(7.5, 6.5),
                    Vec2::new(4.5, 7.5),
                    Vec2::new(3.0, 4.5),
                ],
                next: 1,
            },
        ));
    }
}

/// Hand a party its next waypoint whenever it's standing still (`travel`
/// clears `TravelTarget` on arrival).
fn steer_parties(mut parties: Query<(&mut TravelTarget, &mut Patrol)>) {
    for (mut target, mut patrol) in &mut parties {
        if target.0.is_none() {
            target.0 = Some(patrol.waypoints[patrol.next]);
            patrol.next = (patrol.next + 1) % patrol.waypoints.len();
        }
    }
}

/// Stand-in for the real "your party spots theirs at a distance" rule: reveal a
/// party once you're within 3 tiles of it. This is exactly where that logic
/// will live.
fn spot_parties(
    players: Query<(&Traveler, &TilePos), With<PlayerTraveler>>,
    mut parties: Query<(&Traveler, &TilePos, &mut Spotted), With<Party>>,
) {
    for (party_traveler, party_pos, mut spotted) in &mut parties {
        if spotted.0 {
            continue;
        }
        for (player_traveler, player_pos) in &players {
            if player_traveler.map == party_traveler.map
                && player_pos.0.distance(party_pos.0) <= 3.0
            {
                spotted.0 = true;
            }
        }
    }
}

/// Hold `W` to keep world time running while standing still — this drops a
/// `WorldClockHold` on the map entity, the library's timed-action seam (a real
/// game would set it while resting, repairing, running a radio sweep…).
fn wait_key(
    keys: Res<ButtonInput<KeyCode>>,
    map: Option<Res<DefaultWorldMap>>,
    held: Query<(), With<WorldClockHold>>,
    mut commands: Commands,
) {
    let Some(map) = map else {
        return;
    };
    let want = keys.pressed(KeyCode::KeyW);
    match (want, held.contains(map.0)) {
        (true, false) => {
            commands.entity(map.0).insert(WorldClockHold);
        }
        (false, true) => {
            commands.entity(map.0).remove::<WorldClockHold>();
        }
        _ => {}
    }
}

/// `WorldMapAction` -> a log line. The `match` is exhaustive on purpose: a new
/// variant should make this example stop compiling until it's handled.
fn log_world_map_actions(
    mut actions: MessageReader<WorldMapAction>,
    locations: Query<&Location>,
    secrets: Query<(), With<SecretLocation>>,
    parties: Query<&Party>,
) {
    let name = |e: Entity| {
        locations
            .get(e)
            .map(|l| l.name.clone())
            .unwrap_or_else(|_| "somewhere".to_string())
    };
    let party_name = |e: Entity| {
        parties
            .get(e)
            .map(|p| p.name.clone())
            .unwrap_or_else(|_| "a party".to_string())
    };
    for action in actions.read() {
        match action {
            WorldMapAction::Loaded { .. } => info!("world map loaded"),
            WorldMapAction::LoadFailed { error, .. } => error!("world map failed to load: {error}"),
            WorldMapAction::TargetSet { tile, .. } => {
                info!("travelling to ({:.1}, {:.1})", tile.x, tile.y)
            }
            WorldMapAction::TargetCleared { .. } => info!("trip cancelled"),
            WorldMapAction::Arrived { .. } => info!("arrived"),
            WorldMapAction::Blocked { .. } => info!("blocked by impassable terrain"),
            WorldMapAction::LocationReached { location, .. } => {
                info!("reached {}", name(*location))
            }
            WorldMapAction::LocationLeft { location, .. } => info!("left {}", name(*location)),
            WorldMapAction::LocationDiscovered { location, .. } => {
                if secrets.contains(*location) {
                    info!("found the secret at {}!", name(*location))
                } else {
                    info!("discovered {}", name(*location))
                }
            }
            WorldMapAction::LocationFocused { location, .. } => {
                info!("selected {}", name(*location))
            }
            WorldMapAction::EnterRequested { location, .. } => {
                info!("You enter {}.", name(*location))
            }
            WorldMapAction::PartyIntercepted { party, .. } => {
                info!("party intercepted: {}", party_name(*party))
            }
            WorldMapAction::PartyLeft { party, .. } => {
                info!("broke contact with {}", party_name(*party))
            }
            WorldMapAction::InteractRequested { party, .. } => {
                info!("You hail {}.", party_name(*party))
            }
        }
    }
}
