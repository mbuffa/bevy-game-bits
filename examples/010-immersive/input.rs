//! `bevy_enhanced_input` context and action bindings for the player.
//! `bevy_ahoy` provides the `Movement`/`Jump`/`Crouch`/`RotateCamera` action
//! types; `Interact` is ours, consumed by `interact.rs` (Phase 2).

use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::*;
// Both `bevy::prelude` (a picking event) and `bevy_enhanced_input::prelude`
// export `Press`; name the one we mean explicitly (an explicit import beats a
// glob), as the crate's own doc comment instructs.
use bevy_enhanced_input::prelude::Press;

use crate::config;

/// Marker component identifying the `bevy_enhanced_input` context the
/// player's actions are bound under. Registered via `add_input_context` in
/// `main.rs`.
#[derive(Component, Default)]
pub struct PlayerInput;

#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Interact;

/// RMB: grab / carry / throw a `PropCrate`, consumed by `carry.rs`.
#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Grab;

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
                // Deliberately unconditioned, like ahoy's own `minimal.rs`:
                // holding Space is meant to re-jump the frame you land, and
                // ahoy's jump buffer is fed by the every-frame `Fire<Jump>`.
                // Anything that wants a jump *press* (edge) instead — e.g.
                // `ladder::let_go_on_jump` — observes `Start<Jump>` rather than
                // adding a `Press` here, so ahoy's buffering keeps working.
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
                // Edge-triggered. Without a condition a bevy_enhanced_input
                // action is `Down`-like and its `Fire` event triggers *every
                // frame* the key is held — which turns any toggling
                // `Interacted` consumer (`ladder::attach_on_interact`'s
                // lock/unlock) into a coin flip. `door`/`pickup` only survive
                // that by being idempotent.
                Press::default(),
                bindings![KeyCode::KeyE, GamepadButton::West],
            ),
            (
                // Deliberately *unconditioned*, unlike `Interact`. `carry.rs`
                // never reads `Fire<Grab>`: it grabs/charges on the `Start<Grab>`
                // press edge and places/throws on the `Complete<Grab>` release,
                // with the hold duration in between as the throw charge. A
                // `Press` here would collapse the hold and swallow the release.
                // Same `Start`/`Complete` pair as ahoy's unconditioned `Jump`.
                Action::<Grab>::new(),
                bindings![MouseButton::Right, GamepadButton::RightTrigger2],
            ),
        ]),
    )
}
