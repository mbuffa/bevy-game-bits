//! Immersive-sim prototype: roam a TrenchBroom-authored Quake map with a
//! first-person kinematic controller (bevy_ahoy). Levels are authored in
//! TrenchBroom (game "bevy_game_bits_immersive") and loaded as `.map` files
//! via bevy_trenchbroom; see SPEC.md for the phased build-out and why Q1/Q2
//! BSP was chosen over Quake 3.
//!
//! Controls: WASD move, mouse look, Space jump, Ctrl crouch, click to grab
//! the cursor, Escape to release it.

mod classes;
mod config;
mod devtools;
mod door;
mod input;
mod interact;
mod pickup;
mod player;
mod trenchbroom;
mod ui;

use avian3d::prelude::*;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_trenchbroom::physics::TrenchBroomPhysicsPlugin;
use bevy_trenchbroom::prelude::*;
use bevy_trenchbroom_avian::AvianPhysicsBackend;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Immersive".into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(PhysicsPlugins::default())
    .add_plugins(EnhancedInputPlugin)
    .add_plugins(AhoyPlugins::default())
    .add_plugins((
        TrenchBroomPlugins(trenchbroom::config()),
        TrenchBroomPhysicsPlugin::new(AvianPhysicsBackend),
    ))
    .add_input_context::<input::PlayerInput>()
    .init_resource::<interact::InteractionFocus>()
    .init_resource::<pickup::Inventory>()
    .add_observer(player::spawn_player)
    .add_observer(door::setup_doors)
    .add_observer(door::open_on_interact)
    .add_observer(interact::fire_interact)
    .add_observer(pickup::spawn_visuals)
    .add_observer(pickup::collect_on_interact)
    .add_systems(Startup, (spawn_map, spawn_light, ui::setup_hud))
    .add_systems(
        Update,
        (
            player::capture_cursor.run_if(input_just_pressed(MouseButton::Left)),
            player::release_cursor.run_if(input_just_pressed(KeyCode::Escape)),
            interact::update_focus,
            door::drive_doors,
            ui::update_prompt,
            ui::update_inventory,
        ),
    );

    if devtools::env_flag("IMMERSIVE_AUTOPILOT") {
        app.add_systems(Update, devtools::autopilot_drive);
    }
    if devtools::env_flag("IMMERSIVE_TELEMETRY") {
        app.add_systems(Update, devtools::telemetry);
    }
    if devtools::env_flag("IMMERSIVE_SHOTS") {
        app.add_systems(Update, devtools::take_screenshot);
    }

    trenchbroom::register_classes(&mut app);

    app.run();
}

fn spawn_map(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(SceneRoot(
        asset_server.load(format!("{}#Scene", config::MAP_PATH)),
    ));
}

/// A directional (sun-style) light rather than the map's own `light` point
/// entity: a real-time `PointLight` in this room, at any intensity, blows
/// the whole frame out to solid white (confirmed intensity-independent —
/// 300,000 and 40,000 lm both white, 0 lights renders correctly, and
/// swapping to a `DirectionalLight` renders correctly at the same position).
/// Root cause not yet isolated; see SPEC.md risks. Revisit before Phase 5/6.
fn spawn_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: config::SUN_ILLUMINANCE,
            shadows_enabled: true,
            ..default()
        },
        Transform::default().looking_at(Vec3::new(-0.3, -1.0, -0.5), Vec3::Y),
    ));
}
