//! Verification harness for running this example on a machine where
//! synthetic keyboard/mouse input is blocked (no macOS Accessibility grant —
//! see `.claude/skills/verify/SKILL.md`). `bevy_ahoy`'s `AccumulatedInput`
//! fields are all `pub`, and its own input observers only *write* them, so
//! driving the same fields from here composes cleanly with real input still
//! attached — this isn't a replacement input path, just another writer.
//!
//! All of this is opt-in via environment variables, checked once at
//! startup in `main.rs`, so a normal `cargo run` behaves exactly as before:
//! - `IMMERSIVE_AUTOPILOT=1` — walk `config::AUTOPILOT_SCRIPT` automatically.
//! - `IMMERSIVE_TELEMETRY=1` — log position/speed/grounded/focus periodically.
//! - `IMMERSIVE_SHOTS=1` — take in-app screenshots at scripted checkpoints.

use bevy::prelude::*;
use bevy::render::view::window::screenshot::{save_to_disk, Screenshot};
use bevy_ahoy::input::AccumulatedInput;
use bevy_ahoy::prelude::*;

use crate::config::{self, AutopilotStep};
use crate::interact::InteractionFocus;

pub fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// Drives the player through `config::AUTOPILOT_SCRIPT` on a loop: writes
/// `AccumulatedInput.last_movement` (same field ahoy's own WASD observer
/// writes) and rotates the camera's `Transform` directly (ahoy copies
/// camera rotation into `CharacterLook`, which steers movement, every
/// `RunFixedMainLoop`).
pub fn autopilot_drive(
    mut inputs: Query<&mut AccumulatedInput, With<CharacterController>>,
    mut cameras: Query<&mut Transform, With<CharacterControllerCamera>>,
    time: Res<Time>,
    mut step_idx: Local<usize>,
    mut step_elapsed: Local<f32>,
) {
    if config::AUTOPILOT_SCRIPT.is_empty() {
        return;
    }

    *step_elapsed += time.delta_secs();
    let mut step = &config::AUTOPILOT_SCRIPT[*step_idx % config::AUTOPILOT_SCRIPT.len()];
    if *step_elapsed >= step.duration {
        *step_elapsed = 0.0;
        *step_idx += 1;
        step = &config::AUTOPILOT_SCRIPT[*step_idx % config::AUTOPILOT_SCRIPT.len()];
    }
    let AutopilotStep { movement, yaw_rate, .. } = *step;

    for mut input in &mut inputs {
        input.last_movement = Some(movement);
    }
    for mut transform in &mut cameras {
        let (mut yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        yaw += yaw_rate.to_radians() * time.delta_secs();
        transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    }
}

pub fn telemetry(
    player: Query<(&Transform, &CharacterControllerState), With<CharacterController>>,
    focus: Res<InteractionFocus>,
    time: Res<Time>,
    mut since_last: Local<f32>,
) {
    *since_last += time.delta_secs();
    if *since_last < config::TELEMETRY_INTERVAL {
        return;
    }
    *since_last = 0.0;

    for (transform, state) in &player {
        info!(
            "telemetry: pos={:?} grounded={} focus={:?}",
            transform.translation,
            state.grounded.is_some(),
            focus.0.as_ref().map(|(_, prompt)| prompt.as_str())
        );
    }
}

/// Fires once, a couple of seconds in — late enough for the autopilot to
/// have walked away from spawn, giving a screenshot with something to show.
pub fn take_screenshot(mut commands: Commands, mut frame: Local<u32>) {
    *frame += 1;
    if *frame == 120 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("screenshots/010-immersive/devtools-autopilot.png"));
    }
}
