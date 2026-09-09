//! Turns a `PlayerSpawn` map entity into an actual playable character:
//! ahoy's kinematic controller, a collider, our input bindings, and a child
//! camera. Also owns cursor grab/release, since that's a per-player concern.

use avian3d::prelude::*;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy_ahoy::prelude::*;

use crate::{classes::PlayerSpawn, config, input};

pub fn spawn_player(add: On<Add, PlayerSpawn>, mut commands: Commands) {
    let player = add.entity;
    commands
        .entity(player)
        .insert((
            CharacterController::default(),
            Collider::cylinder(config::PLAYER_RADIUS, config::PLAYER_HEIGHT),
            // Not a weight — avian ignores a kinematic body's mass. This is
            // only the shove strength bevy_ahoy applies to dynamic props the
            // player walks into. See `config::PLAYER_PUSH_MASS`.
            Mass(config::PLAYER_PUSH_MASS),
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

pub fn capture_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
}

pub fn release_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.visible = true;
    cursor.grab_mode = CursorGrabMode::None;
}
