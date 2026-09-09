//! Verification harness for running this example on a machine where
//! synthetic keyboard/mouse input is blocked (no macOS Accessibility grant —
//! see `.claude/skills/verify/SKILL.md`). `bevy_ahoy`'s `AccumulatedInput`
//! fields are all `pub`, and its own input observers only *write* them, so
//! driving the same fields from here composes cleanly with real input still
//! attached — this isn't a replacement input path, just another writer. It
//! mirrors the real path's fidelity too: a zero-movement leg writes *nothing*
//! to `last_movement` (leaving it `None`), exactly as a released dead-zoned
//! axis does, so `ladder::stash_input` can tell "let go" from "still holding".
//! The
//! `interact` and `jump` legs go one step further and press the real
//! `KeyCode::KeyE` / `KeyCode::Space` in `ButtonInput`, so the whole binding →
//! `Press` -> `fire_interact` path (E) and binding -> ahoy `Jump` /
//! `Start<Jump>` path (Space) are under test — those *are* the real input paths
//! end to end, which is how this harness can catch an `Interact` action that
//! fires every frame instead of once per press, or a jump pressed *before* a
//! grab wrongly knocking the player off a ladder.
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
use crate::ladder::Climbing;

pub fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// One human-length key press per script leg. Once `start` is true on a leg,
/// holds `key` down in `ButtonInput` for `config::AUTOPILOT_TAP_SECS`, then
/// releases — so the next leg's press is a fresh released→pressed edge for a
/// `Press` condition to fire on. Idempotent `press`/`release` calls, so driving
/// it every frame is fine.
#[derive(Default)]
pub struct Tap {
    for_step: Option<usize>,
    held: f32,
}

impl Tap {
    fn drive(
        &mut self,
        keys: &mut ButtonInput<KeyCode>,
        key: KeyCode,
        step: usize,
        start: bool,
        dt: f32,
    ) {
        if self.for_step == Some(step) {
            if self.held < config::AUTOPILOT_TAP_SECS {
                self.held += dt;
                keys.press(key);
            } else {
                keys.release(key);
            }
        } else if start {
            self.for_step = Some(step);
            self.held = 0.0;
            keys.press(key);
        } else {
            keys.release(key);
        }
    }
}

/// Drives the player through `config::AUTOPILOT_SCRIPT` on a loop: writes
/// `AccumulatedInput.last_movement` (same field ahoy's own WASD observer
/// writes), rotates the camera's `Transform` directly (ahoy copies camera
/// rotation into `CharacterLook`, which steers movement, every
/// `RunFixedMainLoop`), taps the real `KeyCode::KeyE` on `interact` legs, and
/// *holds* the real `KeyCode::Space` for the whole of any `jump` leg (so a run
/// of consecutive `jump` legs is one continuous hold — a jump pressed well
/// before the leg that grabs the ladder, which is the reported gesture). Runs
/// in `PreUpdate`, after `bevy::input::InputSystems` and before
/// `EnhancedInputSystems::Update`, so the key writes are seen by
/// bevy_enhanced_input the same frame.
pub fn autopilot_drive(
    mut inputs: Query<&mut AccumulatedInput, With<CharacterController>>,
    mut cameras: Query<&mut Transform, With<CharacterControllerCamera>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    focus: Res<InteractionFocus>,
    time: Res<Time>,
    mut step_idx: Local<usize>,
    mut step_elapsed: Local<f32>,
    mut use_tap: Local<Tap>,
) {
    if config::AUTOPILOT_SCRIPT.is_empty() || inputs.is_empty() {
        // Don't start (or advance) the script until the player exists — the
        // scene loads asynchronously, and letting the clock run during the load
        // would make which leg is "current" when the player appears depend on
        // disk speed, which the `jump` legs (fired at a leg's first frame)
        // can't tolerate.
        return;
    }

    *step_elapsed += time.delta_secs();
    let mut step = &config::AUTOPILOT_SCRIPT[*step_idx % config::AUTOPILOT_SCRIPT.len()];
    if *step_elapsed >= step.duration {
        *step_elapsed = 0.0;
        *step_idx += 1;
        step = &config::AUTOPILOT_SCRIPT[*step_idx % config::AUTOPILOT_SCRIPT.len()];
    }
    let AutopilotStep {
        movement,
        yaw_rate,
        interact,
        jump,
        ..
    } = *step;
    let dt = time.delta_secs();

    for mut input in &mut inputs {
        // A real release writes *nothing*: ahoy's `Movement` observer only
        // fires on a non-zero, dead-zoned vector, so `last_movement` stays
        // `None` for the frame. Publishing `Some(ZERO)` here would hide the very
        // bug the stop-mid-climb leg exists to catch (`ladder::stash_input`
        // reads `None` as "let go").
        if movement != Vec2::ZERO {
            input.last_movement = Some(movement);
        }
    }
    for mut transform in &mut cameras {
        let (mut yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        yaw += yaw_rate.to_radians() * dt;
        transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    }

    // Press the real keys, not the actions: this puts the whole binding →
    // `Press` → `fire_interact` (E) and binding → ahoy `Jump` / `Start<Jump>`
    // (Space) paths under test. Drop `Press` from `Interact` and the E hold
    // toggles the ladder every frame; write `AccumulatedInput.jumped` directly
    // instead of via Space and you'd miss that the ladder must key off the
    // press *edge*, not the buffer.
    //
    // E is a tap fired the frame the raycast focuses something (so the grab
    // lands the instant the ladder is in range). Space is *held* for the whole
    // of a `jump` leg — so a run of `jump` legs ending on the grab leg means
    // Space went down seconds before the grab and is still down as it lands:
    // the "run at the ladder, jump, then press E" gesture, and the case
    // `let_go_on_jump` must NOT treat as a request to let go (the press
    // predates the climb, so it raises no new edge while climbing).
    use_tap.drive(
        &mut keys,
        KeyCode::KeyE,
        *step_idx,
        interact && focus.0.is_some(),
        dt,
    );
    if jump {
        keys.press(KeyCode::Space);
    } else {
        keys.release(KeyCode::Space);
    }
}

pub fn telemetry(
    player: Query<
        (&Transform, &CharacterControllerState, Option<&Climbing>),
        With<CharacterController>,
    >,
    cameras: Query<&Transform, With<CharacterControllerCameraOf>>,
    focus: Res<InteractionFocus>,
    time: Res<Time>,
    mut since_last: Local<f32>,
) {
    *since_last += time.delta_secs();
    if *since_last < config::TELEMETRY_INTERVAL {
        return;
    }
    *since_last = 0.0;

    let cam = cameras.single().ok().map(|t| {
        let (y, p, _) = t.rotation.to_euler(EulerRot::YXZ);
        (y.to_degrees(), p.to_degrees())
    });
    for (transform, state, climbing) in &player {
        info!(
            "telemetry: pos={:?} grounded={} climbing={} focus={:?} cam_yaw_pitch={:?}",
            transform.translation,
            state.grounded.is_some(),
            climbing.is_some(),
            focus.0.as_ref().map(|(_, prompt)| prompt.as_str()),
            cam,
        );
    }
}

/// Three shots, each fired once. One a fixed couple of seconds in (the
/// warehouse and its ladder in view during the approach). One the first frame
/// the player is locked on and clear of the floor (`Climbing` + `y > 2.5`) —
/// the acceptance shot for the ladder standoff: the rungs must fill the frame
/// at arm's length, which telemetry can't show. One the first frame the player
/// is standing on a platform after the climb. The last two are keyed on state,
/// not a frame number, since the climb's duration drifts with FPS.
pub fn take_screenshot(
    mut commands: Commands,
    player: Query<
        (&Transform, &CharacterControllerState, Option<&Climbing>),
        With<CharacterController>,
    >,
    mut frame: Local<u32>,
    mut lock_view_done: Local<bool>,
    mut on_platform_done: Local<bool>,
) {
    *frame += 1;
    if *frame == 120 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/devtools-autopilot.png",
            ));
    }
    let Ok((transform, state, climbing)) = player.single() else {
        return;
    };
    if !*lock_view_done && climbing.is_some() && transform.translation.y > 2.5 {
        *lock_view_done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260908-ladder-lock-view.png",
            ));
    }
    if !*on_platform_done && state.grounded.is_some() && transform.translation.y > 3.0 {
        *on_platform_done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260908-on-platform-a.png",
            ));
    }
}
