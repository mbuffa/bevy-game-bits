//! Immersive-sim prototype: roam a TrenchBroom-authored Quake map with a
//! first-person kinematic controller (bevy_ahoy). Levels are authored in
//! TrenchBroom (game "bevy_game_bits_immersive") and loaded as `.map` files
//! via bevy_trenchbroom; see SPEC.md for the phased build-out and why Q1/Q2
//! BSP was chosen over Quake 3.
//!
//! Controls: WASD move, mouse look, Space jump, Ctrl crouch, left-click to
//! grab the cursor, Escape to release it. Aim at a ladder and press E to lock
//! on — W/S climb up/down, E or Space lets go. Aim at a crate and press RMB to
//! carry it; RMB again to place it, or hold RMB to charge a throw (slider
//! under the crosshair) and release to fling it. Stack crates to reach
//! platform B.

mod carry;
mod classes;
mod config;
mod devtools;
mod door;
mod input;
mod interact;
mod ladder;
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
    .insert_resource(GlobalAmbientLight {
        brightness: config::AMBIENT_BRIGHTNESS,
        ..default()
    })
    .init_resource::<interact::InteractionFocus>()
    .init_resource::<pickup::Inventory>()
    .add_observer(player::spawn_player)
    .add_observer(door::setup_doors)
    .add_observer(door::open_on_interact)
    .add_observer(interact::fire_interact)
    .add_observer(pickup::spawn_visuals)
    .add_observer(pickup::collect_on_interact)
    .add_observer(ladder::setup_ladders)
    .add_observer(ladder::attach_on_interact)
    .add_observer(ladder::let_go_on_jump)
    .add_observer(carry::spawn_crates)
    .add_observer(carry::start_grab_or_charge)
    .add_observer(carry::release_prop)
    .add_systems(
        Startup,
        (
            spawn_map,
            spawn_light,
            carry::setup_crate_assets,
            ui::setup_hud,
        ),
    )
    .add_systems(
        Update,
        (
            player::capture_cursor.run_if(input_just_pressed(MouseButton::Left)),
            player::release_cursor.run_if(input_just_pressed(KeyCode::Escape)),
            interact::update_focus,
            carry::advance_charge,
            door::drive_doors,
            ui::update_prompt,
            ui::update_inventory,
            ui::update_charge_bar,
        ),
    )
    // The yaw ease after an E-grab runs here, not in Update, so ahoy's
    // Update-schedule camera sync can't clobber it. See `ladder.rs`.
    .add_systems(
        PostUpdate,
        (ladder::turn_to_ladder, carry::hold_prop).before(TransformSystems::Propagate),
    )
    // Ladder climbing brackets bevy_ahoy's controller: read intent before it
    // runs, overwrite the result after. See `ladder.rs`.
    //
    // `stash_input` runs once per *frame* (not per fixed step): it `take`s
    // `AccumulatedInput::last_movement` and treats a missing value as
    // "released". `AccumulatedInput` has a one-frame lifetime — ahoy clears it
    // in `AfterFixedMainLoop` — so taking it here feeds every substep the same
    // intent, exactly as ahoy reads it. In `FixedPostUpdate` a second substep
    // would see `None` and stall the climb.
    .add_systems(
        RunFixedMainLoop,
        ladder::stash_input.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
    )
    .add_systems(
        FixedPostUpdate,
        ladder::climb
            .after(AhoySystems::MoveCharacters)
            .before(PhysicsSystems::First),
    );

    if let Some(script) = devtools::autopilot_script() {
        // In PreUpdate, after bevy samples the keyboard/mouse and before
        // bevy_enhanced_input reads it, so the autopilot's held `KeyE` /
        // `MouseButton::Right` are seen the same frame. See
        // `devtools::autopilot_drive`. `IMMERSIVE_AUTOPILOT=1` runs the ladder
        // walk, `=crates` the crate grab/place/throw walk.
        app.insert_resource(script)
            .init_resource::<devtools::AutopilotAim>()
            .add_systems(
                PreUpdate,
                devtools::autopilot_drive
                    .after(bevy::input::InputSystems)
                    .before(EnhancedInputSystems::Update),
            )
            .add_systems(
                PostUpdate,
                devtools::autopilot_look.before(TransformSystems::Propagate),
            );
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
