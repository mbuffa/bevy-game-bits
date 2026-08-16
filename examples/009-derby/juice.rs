//! Collision "juice": presentation-only feedback that isn't part of the
//! simulation. Two independent effects sharing this module because both are
//! reactions to the same collision events:
//!
//! - **Screenshake** — every `ImpactBurst` (`damage.rs`) nudges a trauma
//!   accumulator; the camera's rotation gets a small sine-driven shake
//!   proportional to `trauma²`, decaying on real time so it stays crisp even
//!   during slow-motion.
//! - **Cinematic slow-motion** — a deterministic near-miss detector watches
//!   the camera's target against every other live car using the same
//!   closing-geometry math `ai.rs::assess_threat` uses for its own
//!   `Brace`/`Evade` decisions. An approach that gets genuinely close and
//!   dangerous, then separates *without* an actual collision, briefly slows
//!   `Time<Virtual>` (which is what drives both avian's fixed timestep and
//!   bevy_hanabi's particle simulation, so physics and effects slow with it
//!   for free) before easing back to normal speed.

use std::f32::consts::TAU;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::camera::FollowCamera;
use crate::config::*;
use crate::damage::ImpactBurst;
use crate::game::CameraTarget;
use bevy_game_bits::vehicle::{ImpactTuning, Vehicle, VehicleTuning, WheelLanding};

// --- Screenshake -----------------------------------------------------------

/// Standard trauma model: accumulate on impact, decay over time, shake by
/// `trauma²` so small hits barely register and big ones ramp fast.
#[derive(Resource, Default)]
pub struct CameraShake {
    trauma: f32,
}

/// Drains this frame's `ImpactBurst`s and `WheelLanding`s into
/// `CameraShake::trauma`, decays it, and applies a small sine-driven
/// rotational offset to the chase camera. Runs `.after(camera::
/// follow_camera)` (`main.rs`) so it can safely post-multiply onto a
/// rotation `follow_camera` already set fresh this frame via `look_at` — a
/// rotational offset can never feed back into the eased chase pose the way a
/// positional one would.
pub fn shake_camera(
    real_time: Res<Time<Real>>,
    // Both shake ramps start where the driving model stops ignoring a hit,
    // so they read those two floors off the library's tuning rather than
    // keeping copies that could drift out of step with it.
    vehicle_tuning: Res<VehicleTuning>,
    impact_tuning: Res<ImpactTuning>,
    mut shake: ResMut<CameraShake>,
    mut bursts: MessageReader<ImpactBurst>,
    mut landings: MessageReader<WheelLanding>,
    camera: Single<&mut Transform, With<FollowCamera>>,
) {
    let mut camera_transform = camera.into_inner();

    for burst in bursts.read() {
        let distance = camera_transform.translation.distance(burst.position);
        let falloff = (1.0 - distance / SHAKE_RANGE).clamp(0.0, 1.0);
        let min_delta_v = impact_tuning.min_delta_v;
        let intensity =
            ((burst.delta_v - min_delta_v) / (SHAKE_FULL_DELTA_V - min_delta_v)).clamp(0.0, 1.0);
        shake.trauma = (shake.trauma + SHAKE_PER_IMPACT * intensity * falloff).min(1.0);
    }
    // A landing's own, much gentler contribution — up to four can fire the
    // same tick, so this is scaled down rather than treated like a crash.
    for landing in landings.read() {
        let distance = camera_transform.translation.distance(landing.position);
        let falloff = (1.0 - distance / SHAKE_RANGE).clamp(0.0, 1.0);
        let min_speed = vehicle_tuning.landing_thump_min_speed;
        let intensity = ((landing.speed - min_speed) / (LANDING_THUMP_FULL_SPEED - min_speed))
            .clamp(0.0, 1.0);
        shake.trauma = (shake.trauma + SHAKE_PER_LANDING * intensity * falloff).min(1.0);
    }
    shake.trauma = (shake.trauma - SHAKE_DECAY * real_time.delta_secs()).max(0.0);
    if shake.trauma <= 0.0 {
        return;
    }

    let amount = shake.trauma * shake.trauma * SHAKE_MAX_ANGLE;
    let t = real_time.elapsed_secs();
    let yaw = (t * SHAKE_FREQS[0] * TAU).sin() * amount;
    let pitch = (t * SHAKE_FREQS[1] * TAU).sin() * amount;
    let roll = (t * SHAKE_FREQS[2] * TAU).sin() * amount;
    camera_transform.rotation *= Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll);
}

// --- Near-miss slow-motion ---------------------------------------------------

/// One in-progress or resolved close approach between the watched car and
/// another, tracked across ticks while the pair is closing.
struct Approach {
    other: Entity,
    /// Highest severity seen while this pair was closing.
    peak_severity: f32,
    /// Highest closing speed (m/s) seen while closing.
    peak_closing: f32,
    /// Smallest actual centre-to-centre planar distance observed.
    min_distance: f32,
    /// Whether avian ever reported an actual contact between the pair while
    /// this record was open — an exact test, not a distance guess.
    touched: bool,
}

/// Tracks in-progress approaches for whichever car the camera is currently
/// watching. Cleared whenever the watched car changes (e.g. `game::
/// hand_off_camera` moves it after a KO), so a stale approach can never
/// resolve against a car nobody is looking at.
#[derive(Resource, Default)]
pub struct NearMissWatch {
    watched: Option<Entity>,
    approaches: Vec<Approach>,
    cooldown: f32,
}

/// Deterministic near-miss detection: for the watched car against every
/// other live car, projects closing geometry each tick (mirrors `ai.rs::
/// assess_threat`'s severity model) and opens an "approach" record once
/// severity crosses `NEAR_MISS_ARM`. The record resolves the tick the pair
/// starts separating; if its peak severity, minimum distance and peak
/// closing speed all cleared their thresholds, no cooldown is active, and
/// avian's contact graph never actually saw a hit between the pair, it
/// counts as an avoided near miss and triggers slow-motion. Every resolution
/// is logged (`near-miss: …`) whether or not it fires, so the thresholds can
/// be tuned from the log instead of by feel.
pub fn watch_near_misses(
    time: Res<Time>,
    collisions: Collisions,
    mut watch: ResMut<NearMissWatch>,
    mut slowmo: ResMut<SlowMotion>,
    watched_q: Query<Entity, With<CameraTarget>>,
    cars: Query<(Entity, &Transform, &LinearVelocity), With<Vehicle>>,
) {
    let dt = time.delta_secs();
    watch.cooldown = (watch.cooldown - dt).max(0.0);

    let Ok(watched) = watched_q.single() else {
        watch.approaches.clear();
        watch.watched = None;
        return;
    };
    if watch.watched != Some(watched) {
        watch.approaches.clear();
        watch.watched = Some(watched);
    }
    let Ok((_, watched_transform, watched_vel)) = cars.get(watched) else {
        return;
    };
    let watched_pos = watched_transform.translation.with_y(0.0);
    let watched_vel = watched_vel.0.with_y(0.0);

    for (other, other_transform, other_vel) in cars.iter().filter(|(e, ..)| *e != watched) {
        let rel_p = other_transform.translation.with_y(0.0) - watched_pos;
        let rel_v = other_vel.0.with_y(0.0) - watched_vel;
        let closing = -rel_p.dot(rel_v);
        let approaching = closing > 0.0;
        let index = watch.approaches.iter().position(|a| a.other == other);

        if approaching {
            let speed_sq = rel_v.length_squared();
            let severity = if speed_sq > 0.0 {
                let t_ca = (closing / speed_sq).clamp(0.0, NEAR_MISS_HORIZON);
                let miss = rel_p + rel_v * t_ca;
                let proximity = (1.0 - miss.length() / NEAR_MISS_RADIUS).max(0.0);
                let urgency = 1.0 - t_ca / NEAR_MISS_HORIZON;
                let force = (rel_v.length() / NEAR_MISS_FULL_CLOSING).min(1.0);
                proximity * urgency * force
            } else {
                0.0
            };
            let touched = collisions.contains(watched, other);
            let distance = rel_p.length();

            match index {
                Some(i) => {
                    let approach = &mut watch.approaches[i];
                    approach.peak_severity = approach.peak_severity.max(severity);
                    approach.peak_closing = approach.peak_closing.max(rel_v.length());
                    approach.min_distance = approach.min_distance.min(distance);
                    approach.touched |= touched;
                }
                None if severity >= NEAR_MISS_ARM => {
                    watch.approaches.push(Approach {
                        other,
                        peak_severity: severity,
                        peak_closing: rel_v.length(),
                        min_distance: distance,
                        touched,
                    });
                }
                None => {}
            }
        } else if let Some(i) = index {
            let approach = watch.approaches.swap_remove(i);
            let fires = approach.peak_severity >= NEAR_MISS_SEVERITY
                && !approach.touched
                && approach.min_distance <= NEAR_MISS_DISTANCE
                && approach.peak_closing >= NEAR_MISS_MIN_CLOSING
                && watch.cooldown <= 0.0;
            info!(
                "near-miss: car {} peak_severity {:.2} min_distance {:.1} peak_closing {:.1} touched {} -> {}",
                approach.other,
                approach.peak_severity,
                approach.min_distance,
                approach.peak_closing,
                approach.touched,
                if fires { "SLOWMO" } else { "no trigger" },
            );
            if fires {
                slowmo.trigger();
                watch.cooldown = NEAR_MISS_COOLDOWN;
            }
        }
    }
}

/// Drives `Time<Virtual>`'s relative speed through an ease-in / hold /
/// ease-out envelope after `NearMissWatch::trigger`s it. Ticks on
/// `Time<Real>` so the countdown to normal speed can't be slowed by its own
/// effect.
#[derive(Resource, Default)]
pub struct SlowMotion {
    elapsed: f32,
    active: bool,
}

impl SlowMotion {
    fn trigger(&mut self) {
        self.elapsed = 0.0;
        self.active = true;
    }
}

pub fn tick_slow_motion(
    real_time: Res<Time<Real>>,
    mut slowmo: ResMut<SlowMotion>,
    mut virtual_time: ResMut<Time<Virtual>>,
) {
    if !slowmo.active {
        return;
    }
    slowmo.elapsed += real_time.delta_secs();
    let total = 2.0 * SLOWMO_EASE + SLOWMO_DURATION;
    if slowmo.elapsed >= total {
        slowmo.active = false;
        virtual_time.set_relative_speed(1.0);
        return;
    }
    let scale = if slowmo.elapsed < SLOWMO_EASE {
        let t = slowmo.elapsed / SLOWMO_EASE;
        1.0 + (SLOWMO_SCALE - 1.0) * t
    } else if slowmo.elapsed < SLOWMO_EASE + SLOWMO_DURATION {
        SLOWMO_SCALE
    } else {
        let t = (slowmo.elapsed - SLOWMO_EASE - SLOWMO_DURATION) / SLOWMO_EASE;
        SLOWMO_SCALE + (1.0 - SLOWMO_SCALE) * t
    };
    virtual_time.set_relative_speed(scale);
}

/// `OnExit(MatchState::Fighting)`: forces normal speed so the results screen
/// (or a fresh countdown) never opens mid-slow-motion.
pub fn reset_slow_motion(mut slowmo: ResMut<SlowMotion>, mut virtual_time: ResMut<Time<Virtual>>) {
    slowmo.active = false;
    slowmo.elapsed = 0.0;
    virtual_time.set_relative_speed(1.0);
}
