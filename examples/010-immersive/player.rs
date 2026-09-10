//! Turns a `PlayerSpawn` map entity into an actual playable character:
//! ahoy's kinematic controller, a collider, our input bindings, and a child
//! camera. Also owns the cursor grab/release and the look/move freeze while
//! the pack is open, since those are per-player concerns.

use avian3d::prelude::*;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_game_bits::inventory::prelude::InventoryWindow;

use crate::{classes::PlayerSpawn, config, footsteps::Footsteps, input};

pub fn spawn_player(add: On<Add, PlayerSpawn>, mut commands: Commands) {
    let player = add.entity;
    commands
        .entity(player)
        .insert((
            // Slower than ahoy's Quake-sprint default, with a visible wind-up
            // and skid and less snap in the air. Everything unnamed stays at
            // the crate's default. See `config`'s `Movement feel` block —
            // including the three knobs deliberately *not* touched.
            CharacterController {
                speed: config::MOVE_SPEED,
                acceleration_hz: config::MOVE_ACCEL_HZ,
                friction_hz: config::MOVE_FRICTION_HZ,
                stop_speed: config::MOVE_STOP_SPEED,
                air_acceleration_hz: config::AIR_ACCEL_HZ,
                ..default()
            },
            Collider::cylinder(config::PLAYER_RADIUS, config::PLAYER_HEIGHT),
            // Not a weight — avian ignores a kinematic body's mass. This is
            // only the shove strength bevy_ahoy applies to dynamic props the
            // player walks into. See `config::PLAYER_PUSH_MASS`.
            Mass(config::PLAYER_PUSH_MASS),
            Footsteps::default(),
            Name::new("Player"),
        ))
        .insert(input::player_input_bundle());

    commands.spawn((
        Camera3d::default(),
        // The warehouse starts near-black (`lights.rs`); the switch's red
        // indicator and the lamp lenses are HDR emissives, and bloom is what
        // makes a 9 cm glowing square legible from across the room. `Bloom`
        // needs an `Hdr` camera in Bevy 0.18.
        Hdr,
        Bloom::NATURAL,
        CharacterControllerCameraOf::new(player),
    ));
}

/// Set true by `Escape` (with no board open) to release the cursor for a
/// desktop-style "look away"; cleared by a click back into the window. When
/// true — or any inventory board is open — the player is not in control.
#[derive(Resource, Default)]
pub struct CursorReleased(pub bool);

/// Raw-key cursor / pack management, all `ButtonInput` (it must keep working
/// while [`sync_cursor_mode`] has the `PlayerInput` context switched off):
/// `Tab` toggles the pack, `Escape` closes an open board or else releases the
/// cursor, a left-click with the cursor released and no board open re-grabs.
pub fn player_cursor_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut released: ResMut<CursorReleased>,
    mut windows: Query<&mut InventoryWindow>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        for mut window in &mut windows {
            window.open = !window.open;
        }
    }

    if keys.just_pressed(KeyCode::Escape) {
        if windows.iter().any(|w| w.open) {
            for mut window in &mut windows {
                window.open = false;
            }
        } else {
            released.0 = true;
        }
    }

    if mouse.just_pressed(MouseButton::Left) && released.0 && !windows.iter().any(|w| w.open) {
        released.0 = false;
    }
}

/// Idempotent every-frame mirror (the `lights::sync_*` rule): the cursor is
/// locked+hidden and the `PlayerInput` context is active **iff** the player is
/// in control — no board open and the cursor not deliberately released.
///
/// Switching the whole `PlayerInput` context off is the entire look/move
/// freeze: `Movement` / `Jump` / `Crouch` / `RotateCamera` / `Interact` /
/// `Grab` / `Use` all live in it (`input.rs`), so `AccumulatedInput` simply
/// stops being written and ahoy clears it every frame regardless. The old
/// plan's `RunFixedMainLoop`-slot + `PostUpdate`-camera-hold freeze isn't
/// needed. `ContextActivity` is `#[component(immutable)]`, so re-insert only
/// on a real change or bevy_enhanced_input resets every action each frame.
pub fn sync_cursor_mode(
    windows: Query<&InventoryWindow>,
    released: Res<CursorReleased>,
    mut cursor: Single<&mut CursorOptions>,
    player: Single<(Entity, &ContextActivity<input::PlayerInput>)>,
    mut commands: Commands,
) {
    let in_control = !released.0 && !windows.iter().any(|w| w.open);

    let wanted_grab = if in_control {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    if cursor.grab_mode != wanted_grab {
        cursor.grab_mode = wanted_grab;
        cursor.visible = !in_control;
    }

    let (player, activity) = *player;
    if **activity != in_control {
        commands
            .entity(player)
            .insert(ContextActivity::<input::PlayerInput>::new(in_control));
    }
}
