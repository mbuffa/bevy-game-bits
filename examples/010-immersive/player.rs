//! Turns a `PlayerSpawn` map entity into an actual playable character:
//! ahoy's kinematic controller, a collider, our input bindings, and a child
//! camera. Also owns cursor grab/release, since that's a per-player concern.

use avian3d::prelude::*;
use bevy::prelude::*;
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
            Name::new("Player"),
        ))
        .insert(input::player_input_bundle());

    commands.spawn((
        Camera3d::default(),
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
