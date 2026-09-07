//! The `bevy_game_bits::world_map` overworld, on its own.
//!
//! A Fallout 1/2-style world map: a scrollable tile grid with a sidebar of
//! known places. Left-click anywhere to send the traveller there; it walks in
//! a straight line and slows to a crawl through forest, swamp and mountains,
//! so the fast route is the one *you* pick around them. `Space` cancels the
//! current trip. The arrow keys pan the map; while you're travelling the
//! camera follows the token until you pan away. Walk onto a town and a green
//! triangle appears over the token — click it to "enter" (this example just
//! logs it). Click a row in the sidebar to set course for that place.
//! Walk near the hidden ruin and it appears in the sidebar.
//!
//! The map, its terrain palette and its locations all come from
//! `assets/maps/wastes.worldmap.json` — the library never sees a hard-coded
//! tile. The clock in the sidebar (`WorldMapTimePlugin`) only advances while
//! the traveller is moving; a host with its own `GameTime` would leave that
//! plugin out and read `TravelProgress` instead.
//!
//! Everything specific to *this* map lives here: the camera, and the mapping
//! from `WorldMapAction` to log lines.

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
        .add_systems(Update, log_world_map_actions)
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, WorldMapCamera));
}

/// `WorldMapAction` -> a log line. The `match` is exhaustive on purpose: a new
/// variant should make this example stop compiling until it's handled.
fn log_world_map_actions(mut actions: MessageReader<WorldMapAction>, locations: Query<&Location>) {
    let name = |e: Entity| {
        locations
            .get(e)
            .map(|l| l.name.clone())
            .unwrap_or_else(|_| "somewhere".to_string())
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
                info!("discovered {}", name(*location))
            }
            WorldMapAction::LocationFocused { location, .. } => {
                info!("selected {}", name(*location))
            }
            WorldMapAction::EnterRequested { location, .. } => {
                info!("You enter {}.", name(*location))
            }
        }
    }
}
