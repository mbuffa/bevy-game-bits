//! `bevy_enhanced_input` context and action bindings for the player.
//! `bevy_ahoy` provides the `Movement`/`Jump`/`Crouch`/`RotateCamera` action
//! types; `Interact` is ours, consumed by `interact.rs` (Phase 2).

use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::config;

/// Marker component identifying the `bevy_enhanced_input` context the
/// player's actions are bound under. Registered via `add_input_context` in
/// `main.rs`.
#[derive(Component, Default)]
pub struct PlayerInput;

#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Interact;

/// `PlayerInput` plus every action binding, inserted onto the player entity
/// when it spawns. Movement/camera bindings mirror bevy_ahoy's own
/// `minimal.rs` example; `Interact` is appended for our own use-raycast.
pub fn player_input_bundle() -> impl Bundle {
    (
        PlayerInput,
        actions!(PlayerInput[
            (
                Action::<Movement>::new(),
                // Normalize the input vector.
                DeadZone::default(),
                Bindings::spawn((Cardinal::wasd_keys(), Axial::left_stick())),
            ),
            (
                Action::<Jump>::new(),
                bindings![KeyCode::Space, GamepadButton::South],
            ),
            (
                Action::<Crouch>::new(),
                bindings![KeyCode::ControlLeft, GamepadButton::LeftTrigger2],
            ),
            (
                Action::<RotateCamera>::new(),
                Bindings::spawn((
                    Spawn((Binding::mouse_motion(), Scale::splat(config::MOUSE_SENSITIVITY))),
                    Axial::right_stick().with((Scale::splat(4.0), DeadZone::default())),
                )),
            ),
            (
                Action::<Interact>::new(),
                bindings![KeyCode::KeyE, GamepadButton::West],
            ),
        ]),
    )
}
