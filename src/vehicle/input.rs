//! An optional keyboard driver.
//!
//! Entirely a convenience: it writes [`DriveInput`] and nothing else, so a
//! game with its own bindings, a gamepad, or an AI can ignore this module
//! completely and write [`DriveInput`] itself.
//!
//! [`drive_from_keyboard`] is deliberately *not* registered by
//! [`VehiclePlugin`](super::VehiclePlugin) — when the player is allowed to
//! drive is a game's decision, and it usually wants its own run conditions:
//!
//! ```ignore
//! app.add_systems(
//!     FixedUpdate,
//!     drive_from_keyboard
//!         .in_set(VehicleSet::Input)
//!         .run_if(in_state(GameState::Playing)),
//! );
//! ```

use bevy::prelude::*;

use super::control::DriveInput;

/// Drives this chassis from the keyboard. Remove it to take control away
/// (a cutscene, a wrecked car, a finished race) and the vehicle keeps its
/// last [`DriveInput`] — zero that too if you want it to coast.
///
/// Multiple entities can carry one, each with its own bindings, for
/// same-keyboard split-screen.
#[derive(Component, Clone, Copy, Debug)]
pub struct KeyboardDriver {
    pub forward: KeyCode,
    pub back: KeyCode,
    /// Steers toward chassis-local +X — the same sign as
    /// [`DriveInput::steer`].
    pub steer_left: KeyCode,
    pub steer_right: KeyCode,
    pub handbrake: KeyCode,
}

impl Default for KeyboardDriver {
    fn default() -> Self {
        Self {
            forward: KeyCode::KeyW,
            back: KeyCode::KeyS,
            steer_left: KeyCode::KeyA,
            steer_right: KeyCode::KeyD,
            handbrake: KeyCode::Space,
        }
    }
}

/// Samples the keyboard into every [`KeyboardDriver`]'s [`DriveInput`].
///
/// Writes full-magnitude ±1.0 — a keyboard has no partial throttle. Schedule
/// it in [`VehicleSet::Input`](super::VehicleSet::Input) so it lands ahead of
/// the controller in the same tick.
pub fn drive_from_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut drivers: Query<(&KeyboardDriver, &mut DriveInput)>,
) {
    for (driver, mut input) in &mut drivers {
        input.drive = axis(&keys, driver.forward, driver.back);
        input.steer = axis(&keys, driver.steer_left, driver.steer_right);
        input.handbrake = keys.pressed(driver.handbrake);
    }
}

/// -1, 0 or 1 from a pair of keys. With both held the positive key wins
/// rather than cancelling — mashing accelerate and brake together should
/// still move the car, not silently coast.
fn axis(keys: &ButtonInput<KeyCode>, positive: KeyCode, negative: KeyCode) -> f32 {
    if keys.pressed(positive) {
        1.0
    } else if keys.pressed(negative) {
        -1.0
    } else {
        0.0
    }
}
