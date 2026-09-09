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
//! - `IMMERSIVE_AUTOPILOT=1` / `=crates` — walk a scripted route automatically.
//! - `IMMERSIVE_TELEMETRY=1` — log position/speed/grounded/focus/lights periodically.
//! - `IMMERSIVE_SHOTS=1` — take in-app screenshots at scripted checkpoints.
//! - `IMMERSIVE_LIGHTS=on` — every fixture/switch comes up energised.
//! - `IMMERSIVE_LIGHTS=toggle` — fire `Interacted` at the wall switch once,
//!   ~2.5 s in (the switch is only reachable by a hand-built crate stack).

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy::render::view::window::screenshot::{save_to_disk, Screenshot};
use bevy_ahoy::input::AccumulatedInput;
use bevy_ahoy::prelude::*;

use crate::carry::{Carried, Carrying, ThrowCharge};
use crate::classes::PropCrate;
use crate::config::{self, AutopilotStep};
use crate::interact::InteractionFocus;
use crate::ladder::Climbing;

pub fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// `IMMERSIVE_LIGHTS=on` — every `LightFixture` / `FuncLightSwitch` comes up
/// energised regardless of its `start_on`, so the lit warehouse can be
/// iterated on without first building a crate stack to reach the switch.
pub fn force_lights_on() -> bool {
    std::env::var("IMMERSIVE_LIGHTS")
        .map(|v| v == "on")
        .unwrap_or(false)
}

/// `IMMERSIVE_LIGHTS=toggle` — fire `Interacted` straight at the wall switch
/// once, a couple of seconds in. The switch sits where only a hand-built
/// crate stack reaches, which no autopilot can do; the "use" raycast that
/// finds it is the same `SpatialQuery::cast_ray` the ladder already proves,
/// so the harness skips straight to the event and exercises the part that's
/// new: `lights::toggle_on_interact` → `Powered` → the `sync_*` mirrors →
/// `GlobalAmbientLight`. Read with `IMMERSIVE_TELEMETRY=1`: `lights=4/12`
/// (the 4 always-on pillar brackets) must step to `lights=12/12 switches_on=1`
/// exactly once and stay, and the 4 brackets are unaffected.
pub fn auto_toggle_switch(
    switches: Query<Entity, With<crate::lights::SwitchState>>,
    mut commands: Commands,
    mut frame: Local<u32>,
    mut done: Local<bool>,
) {
    *frame += 1;
    if *done || *frame < 150 {
        return;
    }
    if let Ok(switch) = switches.single() {
        info!("devtools: auto-toggling light switch {switch}");
        commands.trigger(crate::interact::Interacted { entity: switch });
        *done = true;
    }
}

/// Which scripted run `IMMERSIVE_AUTOPILOT` selects: `=1` the ladder
/// regression walk (`config::AUTOPILOT_SCRIPT`), `=crates` the crate
/// grab/place/throw/weight-gate walk (`config::AUTOPILOT_SCRIPT_CRATES`).
#[derive(Resource, Clone, Copy)]
pub struct AutopilotScript(pub &'static [AutopilotStep]);

/// The absolute view orientation (radians) the current leg wants. Written by
/// `autopilot_drive` in `PreUpdate`, applied to the camera by `autopilot_look`
/// in `PostUpdate` — after ahoy's `copy_camera_to_character_look` (which runs
/// in `RunFixedMainLoop`, *before* `Update`) would otherwise clobber anything
/// written into `CharacterLook`, and after a crate bump has nudged the body.
/// Forcing it late and absolutely is the only stable way. Same "write the
/// camera late" trick as `ladder::turn_to_ladder`.
#[derive(Resource, Default)]
pub struct AutopilotAim {
    /// Yaw to add to the camera's current facing this frame (`yaw_rate * dt`).
    pub yaw_delta: f32,
    /// Absolute pitch (radians) to hold the view at.
    pub pitch: f32,
}

pub fn autopilot_script() -> Option<AutopilotScript> {
    match std::env::var("IMMERSIVE_AUTOPILOT").ok().as_deref() {
        Some("1") => Some(AutopilotScript(config::AUTOPILOT_SCRIPT)),
        Some("crates") => Some(AutopilotScript(config::AUTOPILOT_SCRIPT_CRATES)),
        _ => None,
    }
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

/// Drives the player through the selected `AutopilotScript` on a loop: writes
/// `AccumulatedInput.last_movement` (same field ahoy's own WASD observer
/// writes), hands the view orientation to `autopilot_look` via `AutopilotAim`,
/// taps the real `KeyCode::KeyE` on `interact` legs, and *holds* the real
/// `KeyCode::Space` / `MouseButton::Right` for the whole of any `jump` / `grab`
/// leg (so a run of such legs is one continuous hold — Space pressed well
/// before a ladder grab, RMB held to charge a throw). Runs in `PreUpdate`,
/// after `bevy::input::InputSystems` and before `EnhancedInputSystems::Update`,
/// so the key/mouse writes are seen by bevy_enhanced_input the same frame.
pub fn autopilot_drive(
    script: Res<AutopilotScript>,
    mut inputs: Query<&mut AccumulatedInput, With<CharacterController>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut aim: ResMut<AutopilotAim>,
    focus: Res<InteractionFocus>,
    time: Res<Time>,
    mut step_idx: Local<usize>,
    mut step_elapsed: Local<f32>,
    mut use_tap: Local<Tap>,
) {
    let steps = script.0;
    if steps.is_empty() || inputs.is_empty() {
        // Don't start (or advance) the script until the player exists — the
        // scene loads asynchronously, and letting the clock run during the load
        // would make which leg is "current" when the player appears depend on
        // disk speed, which the `jump` legs (fired at a leg's first frame)
        // can't tolerate.
        return;
    }

    // Walk the script once and then hold on the last leg (both scripts end on
    // a STILL leg). Not a loop — a second pass would start from a world the
    // first pass rearranged (a thrown crate underfoot), making the telemetry
    // impossible to read.
    *step_elapsed += time.delta_secs();
    let mut step = &steps[(*step_idx).min(steps.len() - 1)];
    if *step_elapsed >= step.duration && *step_idx < steps.len() - 1 {
        *step_elapsed = 0.0;
        *step_idx += 1;
        step = &steps[*step_idx];
    }
    let AutopilotStep {
        movement,
        yaw_rate,
        interact,
        jump,
        grab,
        pitch_deg,
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
    // Hand the view orientation to `autopilot_look` (PostUpdate): it keeps the
    // camera's own yaw, adds this leg's `yaw_rate` and forces the pitch. Both
    // scripts here leave `yaw_rate` 0 and rely on the spawn facing, so in
    // practice this only ever sets the pitch — needed because a floor crate is
    // below an eye-level ray. Writing `CharacterLook` from here is pointless:
    // ahoy's `copy_camera_to_character_look` overwrites it from the camera in
    // `RunFixedMainLoop`, before `Update` applies it and before the KCC runs.
    aim.yaw_delta = yaw_rate.to_radians() * dt;
    aim.pitch = pitch_deg.to_radians();

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

    // RMB is *held* for the whole of a `grab` leg, like Space — so a leg's
    // duration is the throw charge, and a run of `grab` legs is one continuous
    // press (grab, then keep holding to charge). Exercises the real
    // binding → `Start<Grab>` / `Complete<Grab>` path in `carry.rs`.
    if grab {
        mouse.press(MouseButton::Right);
    } else {
        mouse.release(MouseButton::Right);
    }
}

/// Apply the current leg's pitch to the camera, after ahoy's own camera sync.
/// `PostUpdate`, before transform propagation — the `ladder::turn_to_ladder`
/// slot and reasoning.
pub fn autopilot_look(
    aim: Res<AutopilotAim>,
    mut cameras: Query<&mut Transform, With<CharacterControllerCameraOf>>,
) {
    for mut transform in &mut cameras {
        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw + aim.yaw_delta, aim.pitch, 0.0);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn telemetry(
    player: Query<
        (
            &Transform,
            &CharacterControllerState,
            Option<&Climbing>,
            Option<&Carrying>,
            Option<&ThrowCharge>,
        ),
        With<CharacterController>,
    >,
    cameras: Query<&Transform, With<CharacterControllerCameraOf>>,
    crates: Query<(&Transform, Option<&LinearVelocity>, &PropCrate), Without<Carried>>,
    fixtures: Query<&crate::lights::Powered>,
    switches: Query<&crate::lights::SwitchState>,
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
        (
            (t.translation.x, t.translation.y, t.translation.z),
            y.to_degrees(),
            p.to_degrees(),
        )
    });
    // *Liftable* crates only (mass <= CARRY_MAX_MASS) — the carry subjects.
    // Skips the pre-placed heavy crates (400 kg autopilot, 800 kg base), so
    // `max_y`/`moving` mean something: a walked-into crate stays at 0 moving,
    // a thrown one shows as 1, a stack lifts `max_y`.
    let mut count = 0;
    let mut max_y = f32::MIN;
    let mut moving = 0;
    for (transform, vel, prop) in &crates {
        if prop.mass > config::CARRY_MAX_MASS {
            continue;
        }
        count += 1;
        max_y = max_y.max(transform.translation.y);
        if vel.is_some_and(|v| v.0.length() > 0.15) {
            moving += 1;
        }
    }
    let lamps_on = fixtures.iter().filter(|p| p.0).count();
    let lamps_total = fixtures.iter().count();
    let switches_on = switches.iter().filter(|s| s.0).count();
    for (transform, state, climbing, carrying, charge) in &player {
        info!(
            "telemetry: pos={:?} grounded={} climbing={} carrying={} charge={:.2} \
             crates(n={} max_y={:.2} moving={}) lights={}/{} switches_on={} focus={:?} cam_yaw_pitch={:?}",
            transform.translation,
            state.grounded.is_some(),
            climbing.is_some(),
            carrying.is_some(),
            charge.map_or(0.0, |c| c.0),
            count,
            if count > 0 { max_y } else { 0.0 },
            moving,
            lamps_on,
            lamps_total,
            switches_on,
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
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn take_screenshot(
    mut commands: Commands,
    player: Query<
        (
            &Transform,
            &CharacterControllerState,
            Option<&Climbing>,
            Option<&Carrying>,
        ),
        With<CharacterController>,
    >,
    switches: Query<&crate::lights::SwitchState>,
    mut frame: Local<u32>,
    mut lock_view_done: Local<bool>,
    mut on_platform_done: Local<bool>,
    mut carrying_done: Local<bool>,
    mut platform_b_done: Local<bool>,
    mut lights_dark_done: Local<bool>,
    mut lights_lit_done: Local<bool>,
) {
    *frame += 1;
    if *frame == 120 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/devtools-autopilot.png",
            ));
    }
    // Phase 9: the dark room (a fixed early frame) and the lit room (the first
    // frame any switch reads on).
    if !*lights_dark_done && *frame == 90 {
        *lights_dark_done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-warehouse-dark.png",
            ));
    }
    if !*lights_lit_done && switches.iter().any(|s| s.0) {
        *lights_lit_done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-warehouse-lit.png",
            ));
    }
    let Ok((transform, state, climbing, carrying)) = player.single() else {
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
    // Phase 8: the ghost crate at arm's length + charge slider.
    if !*carrying_done && carrying.is_some() {
        *carrying_done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-carrying-crate.png",
            ));
    }
    // Phase 8: standing on platform B (deck top 4.88 m) after a crate stack.
    if !*platform_b_done && state.grounded.is_some() && transform.translation.y > 4.5 {
        *platform_b_done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-on-platform-b.png",
            ));
    }
}
