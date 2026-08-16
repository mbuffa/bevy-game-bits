//! AI drivers for the fellow cars. Each AI car writes its own `DriveInput`
//! every fixed step — the same intent component the keyboard fills for the
//! player — and `vehicle_controller` consumes it identically, so the AI
//! drives the exact physics the player does.
//!
//! Perception is never cached: everything the AI reasons about (who's a
//! threat, what's ahead, where the wall is) is recomputed from the world
//! every tick. `AiDriver` only stores *decisions* that are deliberately
//! sticky — which car it committed to hunting, how long it's been stuck,
//! how long a retreat/recovery has left — so the component stays small and
//! the behavior stays honest to the current physics state.
//!
//! Five states:
//! - `Hunt` — chasing the committed target with lead pursuit, so contact
//!   lands nose-first (where the ram bar pays off).
//! - `Brace` — a hit is coming and the car can rotate to meet it nose-first
//!   in time: `SHIELD_ABSORB` eats most of a frontal hit, and
//!   `RAM_DAMAGE_BONUS` makes the counter-ram worth it.
//! - `Evade` — a hit is coming and the car *can't* rotate into it fast
//!   enough (or the ram bar is spent) — steer away and brake if it's
//!   coming from ahead.
//! - `Recover` — badly damaged: break off, run for a power-up or open
//!   space, and stay out of the fight for a while.
//! - `Retreat` — stuck against a wall/pillar/block; back out. Entered from
//!   *any* state (highest priority — a wedged car can't execute any other
//!   plan) and, unlike the others, keeps the exact reverse behavior from
//!   the first pass: Phase 8 verified 79/79 retreat cycles resolving under
//!   it, so it isn't touched by the newer arbitration.
//!
//! Arbitration policy (see `ai_drivers`): the few *hard* constraints —
//! being stuck, an imminent obstacle hit — override steering outright;
//! everything else (the wall escape, the obstacle whisker's soft nudge)
//! blends into a single `goal_bearing`. Full weighted-vector blending
//! across all influences was deliberately avoided: bearings are angles,
//! and averaging two escape angles either side of an obstacle averages to
//! "drive straight into it."
//!
//! Query-conflict trap: never add `Forces` or `&mut Damage` to this
//! system. `Forces` statically declares `&mut LinearVelocity` /
//! `&mut AngularVelocity` (needed for its impulse methods), which would
//! conflict with this system's own `cars` query the moment both exist —
//! the same reason `vehicle_controller` reads velocity through `Forces`
//! rather than a separate `&LinearVelocity` fetch (see `vehicle.rs`). This
//! system only ever *writes* `DriveInput`; `vehicle_controller` stays the
//! single authority on forces.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::config::*;
use crate::damage::{Damage, Immunity};
use crate::game::Wrecked;
use crate::powerups::PowerUp;
use bevy_game_bits::vehicle::{DriveInput, Vehicle, VehicleTuning};

// --- State & component -------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum AiState {
    /// Lead-pursuing the committed target at full throttle (lifting off
    /// to turn tighter past `AI_TURN_LIFT_BEARING`).
    Hunt,
    /// Rotating to meet an incoming threat nose-first.
    Brace,
    /// Steering away from an incoming threat it can't meet in time.
    Evade,
    /// Broken off to recover: heading for a power-up or open space.
    Recover { remaining: f32 },
    /// Backing out of a stuck position; counts down to re-entering `Hunt`.
    Retreat { remaining: f32 },
}

impl AiState {
    fn label(self) -> &'static str {
        match self {
            Self::Hunt => "hunt",
            Self::Brace => "brace",
            Self::Evade => "evade",
            Self::Recover { .. } => "recover",
            Self::Retreat { .. } => "retreat",
        }
    }
}

/// State machine + perception scratch for one AI-driven chassis.
#[derive(Component)]
pub struct AiDriver {
    state: AiState,
    /// Per-car xorshift64 stream, seeded once at spawn. Deliberately *not*
    /// a shared `Resource` like `powerups::DropRng`: a shared stream would
    /// couple every car's rolls to the order the query happens to visit
    /// cars in, so adding a fifth car would change the other four's
    /// history. A private stream per car keeps each driver's randomness
    /// independent of the field.
    rng: u64,
    /// The car this driver has committed to hunting. `None` until the
    /// first pick (immediate: `retarget_timer` starts at 0).
    target: Option<Entity>,
    /// Counts down to the next re-evaluation of `target`. Commitment is
    /// the point: recomputing "nearest" every tick made cars dither
    /// between two rivals equidistant off either shoulder.
    retarget_timer: f32,
    /// Seconds spent commanding forward drive while barely moving.
    stuck_timer: f32,
    /// Blocks re-entry into `Recover` for a while after one times out, so
    /// a car whose health can never come back doesn't hide for the rest
    /// of the round.
    recover_cooldown: f32,
}

impl AiDriver {
    /// `index` is the fellow car's spawn slot. The seed is mixed with a
    /// fixed odd constant so adjacent slots produce unrelated streams; the
    /// final `| 1` guards against a zero seed, xorshift's fixed point.
    pub fn new(index: usize) -> Self {
        let seed = (AI_RNG_SEED ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
        Self {
            state: AiState::Hunt,
            rng: seed,
            target: None,
            retarget_timer: 0.0,
            stuck_timer: 0.0,
            recover_cooldown: 0.0,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Uniform float in [0, 1) — the same top-53-bits trick as
    /// `powerups::DropRng::phase`.
    fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) as f32
    }

    /// Uniform index in `0..n`. Caller guarantees `n > 0`.
    fn pick(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Advances the state machine one tick and returns the state *before*
    /// this call, so the caller can log a transition. Mutates `state` and
    /// `recover_cooldown` in place.
    ///
    /// Priority, highest first: a live `Retreat` always self-manages (only
    /// the stuck-detector in `ai_drivers` enters or exits it, from
    /// *outside* this method); then a threat severe enough crosses
    /// straight into `Brace`/`Evade`; then `Recover` is entered/exited by
    /// condition; otherwise `Hunt`.
    fn decide(&mut self, threat: Option<&Threat>, can_brace: bool, condition: f32, dt: f32) -> AiState {
        let before = self.state;

        if self.recover_cooldown > 0.0 {
            self.recover_cooldown = (self.recover_cooldown - dt).max(0.0);
        }

        if let AiState::Retreat { remaining } = self.state {
            let remaining = remaining - dt;
            self.state = if remaining <= 0.0 {
                AiState::Hunt
            } else {
                AiState::Retreat { remaining }
            };
            return before;
        }

        // Schmitt trigger: entering Brace/Evade takes a higher severity
        // than leaving it, so severity oscillating near the boundary
        // doesn't flicker the state every tick.
        let defending = matches!(self.state, AiState::Brace | AiState::Evade);
        let threat_level = threat.map_or(0.0, |t| t.severity);
        let enter_threshold = if defending { AI_THREAT_EXIT } else { AI_THREAT_ENTER };
        if threat_level >= enter_threshold {
            self.state = if can_brace { AiState::Brace } else { AiState::Evade };
            return before;
        }

        self.state = match self.state {
            AiState::Recover { remaining } => {
                if condition >= AI_RECOVER_CONDITION + AI_RECOVER_HYSTERESIS {
                    AiState::Hunt
                } else {
                    let remaining = remaining - dt;
                    if remaining <= 0.0 {
                        self.recover_cooldown = AI_RECOVER_COOLDOWN;
                        AiState::Hunt
                    } else {
                        AiState::Recover { remaining }
                    }
                }
            }
            _ => {
                if condition <= AI_RECOVER_CONDITION && self.recover_cooldown <= 0.0 {
                    AiState::Recover {
                        remaining: AI_RECOVER_TIME,
                    }
                } else {
                    AiState::Hunt
                }
            }
        };
        before
    }
}

// --- Perception ----------------------------------------------------------

/// A snapshot of one car, taken once per tick. `position`/`velocity` are
/// full 3-D (world); callers planarize with `.with_y(0.0)` wherever the
/// math is 2-D, which is almost everywhere — this arena has no vertical
/// gameplay beyond the ramps.
#[derive(Clone, Copy)]
struct CarView {
    entity: Entity,
    position: Vec3,
    velocity: Vec3,
    rotation: Quat,
    condition: f32,
    shield: f32,
    immune: bool,
    /// This car's own class — `sense_obstacles` reads `class.spec()
    /// .chassis_size` off it, so the Buggy's whiskers spread from its own
    /// (smaller) nose instead of the Truck's.
    class: CarClass,
}

/// The single highest-severity incoming car, if any. Aim points are
/// precomputed here rather than re-derived from the threatening car's
/// `CarView` later, so `Brace`/`Evade` don't need a second lookup.
#[derive(Clone, Copy)]
struct Threat {
    severity: f32,
    /// Seconds to closest approach, clamped to `AI_THREAT_HORIZON`.
    t_ca: f32,
    /// The threat's predicted position at closest approach — the `Brace`
    /// aim point. Stable as `t_ca -> 0`, unlike normalizing the miss
    /// vector, which is degenerate exactly when severity peaks.
    lead: Vec3,
    /// Bearing from me to `lead`.
    bearing: f32,
    /// `threat_velocity - my_velocity`, planar — the direction `Evade`
    /// steers away from (the gradient of the miss distance).
    rel_v: Vec3,
    /// The threat's current planar position, for `Evade`'s "away from
    /// them" sign.
    position: Vec3,
}

/// What a forward obstacle whisker found. `None` from `sense_obstacles`
/// means nothing was in range on any of the three lanes.
#[derive(Clone, Copy)]
struct Blockage {
    /// Nearest hit distance (m) across the three whiskers.
    min_distance: f32,
    /// The range the whiskers were cast at.
    range: f32,
    /// Which way to turn to go around: +1 toward chassis +X, -1 away.
    turn_sign: f32,
    /// True once the nearest hit is close enough (in time at current
    /// speed) that avoidance must override steering *and* brake, not just
    /// nudge the racing line.
    hard: bool,
}

/// Bearing to a world point in chassis-local terms: positive = the point
/// is toward chassis +X, matching `DriveInput::steer`'s convention. Both
/// arguments should be in the same (typically planar) frame — mixing a
/// planar point with a full-height `from` would let any chassis pitch (on
/// a ramp) smear into the bearing.
fn bearing_to(rotation: Quat, from: Vec3, point: Vec3) -> f32 {
    let local = rotation.inverse() * (point - from);
    local.x.atan2(local.z)
}

/// Closing-geometry threat assessment against every other car: time to
/// closest approach and miss distance, combined multiplicatively with how
/// hard the closing speed is — all three have to hold at once for a
/// passing car to count as a threat at all. Keeps only the worst one.
fn assess_threat(me: &CarView, views: &[CarView]) -> Option<Threat> {
    let planar_pos = me.position.with_y(0.0);
    let planar_vel = me.velocity.with_y(0.0);
    let mut best: Option<Threat> = None;

    for other in views.iter().filter(|v| v.entity != me.entity) {
        let rel_p = other.position.with_y(0.0) - planar_pos;
        let rel_v = other.velocity.with_y(0.0) - planar_vel;
        let closing_speed = rel_v.length();
        if closing_speed < AI_THREAT_MIN_CLOSING {
            continue;
        }
        let speed_sq = rel_v.length_squared();
        let closing = -rel_p.dot(rel_v);
        let t_ca = (closing / speed_sq).clamp(0.0, AI_THREAT_HORIZON);
        let miss = rel_p + rel_v * t_ca;
        let d_miss = miss.length();
        if d_miss >= AI_THREAT_RADIUS {
            continue;
        }

        let proximity = 1.0 - d_miss / AI_THREAT_RADIUS;
        let urgency = 1.0 - t_ca / AI_THREAT_HORIZON;
        let force = (closing_speed / AI_THREAT_FULL_CLOSING).clamp(0.0, 1.0);
        let severity = proximity * urgency * force;

        if best.is_none_or(|b| severity > b.severity) {
            let lead = other.position.with_y(0.0) + other.velocity.with_y(0.0) * t_ca;
            best = Some(Threat {
                severity,
                t_ca,
                lead,
                bearing: bearing_to(me.rotation, planar_pos, lead),
                rel_v,
                position: other.position.with_y(0.0),
            });
        }
    }
    best
}

/// Three horizontal whiskers from the nose (chassis-center-height origin,
/// horizontal direction — a raw forward vector would fire into the floor
/// or the sky the moment the chassis pitches, e.g. on a ramp). A shapecast
/// was considered and rejected: the chassis is permanently touching the
/// ground, so a shapecast against the floor trimesh reports a contact
/// every tick, and it doesn't say *which side* is clear the way three
/// lanes do.
#[allow(clippy::too_many_arguments)]
fn sense_obstacles(
    spatial_query: &SpatialQuery,
    bodies: &Query<(&RigidBody, Option<&Mass>)>,
    filter: &SpatialQueryFilter,
    position: Vec3,
    heading: Vec3,
    right: Vec3,
    speed: f32,
    goal_bearing: f32,
    chassis_size: Vec3,
    // The same threshold the suspension uses to decide what counts as
    // drivable ground, read off `VehicleTuning` rather than duplicated here
    // — if one moves, both move.
    ground_normal_min: f32,
) -> Option<Blockage> {
    let Ok(direction) = Dir3::new(heading) else {
        return None; // airborne / perfectly vertical — nothing sensible to steer around
    };
    let range = (speed * AI_OBSTACLE_LOOKAHEAD).clamp(AI_OBSTACLE_MIN_RANGE, AI_OBSTACLE_MAX_RANGE);
    let nose = position + heading * (chassis_size.z * 0.5);

    // Static scenery is always an obstacle; a dynamic prop only counts if
    // it's too heavy to shove — "avoid what you can't shove", which
    // crucially lets the AI drive straight through loot crates (50 kg),
    // the only way their power-ups get into the arena.
    let avoidable = |entity: Entity| {
        bodies.get(entity).is_ok_and(|(body, mass)| match body {
            RigidBody::Static => true,
            RigidBody::Dynamic => mass.is_some_and(|m| m.0 >= AI_AVOID_MIN_MASS),
            _ => false,
        })
    };
    let cast = |lane: f32| {
        let origin = nose + right * lane * (chassis_size.x * 0.5);
        spatial_query
            .cast_ray_predicate(origin, direction, range, true, filter, &avoidable)
            // A surface facing up is drivable, not an obstacle: kills
            // shallow floor-trimesh hits and — crucially — reads a ramp's
            // sloped face (normal.y ~= 0.98) as clear while still avoiding
            // its vertical back/side faces, so the AI keeps launching off
            // ramps mid-chase.
            .filter(|hit| hit.normal.y < ground_normal_min)
            .map(|hit| hit.distance)
            .unwrap_or(range)
    };

    let d_plus = cast(1.0);
    let d_center = cast(0.0);
    let d_minus = cast(-1.0);
    let min_distance = d_plus.min(d_center).min(d_minus);
    if min_distance >= range {
        return None;
    }

    // Tie: go the way the current goal already points, so dodging an
    // obstacle doesn't cost the AI its target.
    let turn_sign = if (d_plus - d_minus).abs() < AI_OBSTACLE_TIE {
        if goal_bearing >= 0.0 { 1.0 } else { -1.0 }
    } else if d_plus > d_minus {
        1.0
    } else {
        -1.0
    };
    let hard = min_distance < speed * AI_OBSTACLE_BRAKE_TIME;
    Some(Blockage {
        min_distance,
        range,
        turn_sign,
        hard,
    })
}

/// Predictive wall avoidance: if the spot the car will occupy in
/// `AI_WALL_LOOKAHEAD` seconds leaves the safe radius, return an aim point
/// to steer for instead. Escapes along `-radial * AI_WALL_TURN_IN +
/// tangent`, not straight for the arena center — the center is exactly
/// where the r=3 pillar sits, so aiming there (the original bug) steers
/// the escape at an obstacle. The blended direction keeps the car's
/// tangential momentum while still guaranteeing a net inward drift, so it
/// spirals off the wall instead of grinding along it or orbiting forever.
fn wall_escape(position: Vec3, velocity: Vec3, heading: Vec3) -> Option<Vec3> {
    let planar_pos = position.with_y(0.0);
    let planar_vel = velocity.with_y(0.0);
    let ahead = planar_pos + planar_vel * AI_WALL_LOOKAHEAD;
    if ahead.length() <= ARENA_RADIUS - AI_WALL_MARGIN {
        return None;
    }

    let radial = planar_pos.normalize_or_zero();
    if radial == Vec3::ZERO {
        return None; // exactly at the arena center — no meaningful escape direction
    }
    let mut tangent = Vec3::Y.cross(radial).normalize_or_zero();
    if tangent.dot(heading) < 0.0 {
        tangent = -tangent;
    }
    let escape_dir = (-radial * AI_WALL_TURN_IN + tangent).normalize_or_zero();
    // Magnitude is arbitrary — only the bearing to this point is used.
    Some(planar_pos + escape_dir * 20.0)
}

// --- Decision --------------------------------------------------------------

/// Score for a `Hunt` target candidate inside the vision cone, or `None`
/// if it's outside the cone or range. Additive, unlike `assess_threat`'s
/// severity: these are *preferences* (a wounded car slightly off the nose
/// can outrank a healthy one dead ahead), not a conjunction of hard
/// conditions.
fn score_candidate(me: &CarView, other: &CarView) -> Option<f32> {
    let planar_pos = me.position.with_y(0.0);
    let other_pos = other.position.with_y(0.0);
    let bearing = bearing_to(me.rotation, planar_pos, other_pos);
    let dist = planar_pos.distance(other_pos);
    if bearing.abs() > AI_VISION_HALF_ANGLE || dist > AI_VISION_RANGE {
        return None;
    }

    let alignment = 1.0 - bearing.abs() / AI_VISION_HALF_ANGLE;
    let closeness = 1.0 - dist / AI_VISION_RANGE;
    let weakness = 1.0 - other.condition;
    let mut score =
        AI_SCORE_ALIGNMENT * alignment + AI_SCORE_CLOSENESS * closeness + AI_SCORE_WEAKNESS * weakness;
    if other.immune {
        // Hits do nothing to an immune target, and it still deals full
        // damage (plus its own ram bonus) back — attacking one is
        // strictly negative EV. Not a hard ban: still a last resort if
        // it's the only thing in the cone.
        score -= AI_SCORE_IMMUNE_PENALTY;
    }
    Some(score)
}

/// Best in-cone candidate by score, or a uniform random pick among every
/// other car if the cone is empty. Returns the target and whether it came
/// from the cone, for the `targets ... (cone|random)` log line.
fn select_target(me: &CarView, views: &[CarView], driver: &mut AiDriver) -> Option<(Entity, bool)> {
    let best = views
        .iter()
        .filter(|v| v.entity != me.entity)
        .filter_map(|v| score_candidate(me, v).map(|score| (v.entity, score)))
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
    if let Some((entity, _)) = best {
        return Some((entity, true));
    }

    // The random fallback is filtered to `AI_TARGET_DROP_RANGE` too: an
    // unfiltered pick can land on a car most of the arena away (two
    // fellow cars on opposite edge-spawn slots start over 45 m apart),
    // which is immediately far enough to fail the drop check again next
    // tick — re-picking every tick instead of every `AI_RETARGET_TIME`.
    // Falls back further to any other car only in the degenerate case
    // where nobody is within range at all.
    let planar_pos = me.position.with_y(0.0);
    let mut others: Vec<Entity> = views
        .iter()
        .filter(|v| {
            v.entity != me.entity && planar_pos.distance(v.position.with_y(0.0)) <= AI_TARGET_DROP_RANGE
        })
        .map(|v| v.entity)
        .collect();
    if others.is_empty() {
        others = views
            .iter()
            .filter(|v| v.entity != me.entity)
            .map(|v| v.entity)
            .collect();
    }
    if others.is_empty() {
        None
    } else {
        Some((others[driver.pick(others.len())], false))
    }
}

/// Lead-pursuit aim point for `Hunt`: where the target will be after the
/// time it'd take this car to close the current distance at its own
/// speed. This is what turns rams into *front-zone* rams — the whole
/// economic point of the ram bar.
fn lead_aim(me: &CarView, target: &CarView) -> Vec3 {
    let planar_pos = me.position.with_y(0.0);
    let target_pos = target.position.with_y(0.0);
    let dist = planar_pos.distance(target_pos);
    let own_speed = me.velocity.with_y(0.0).length();
    let t_lead = (dist / own_speed.max(AI_LEAD_MIN_SPEED)).min(AI_LEAD_TIME_MAX);
    target_pos + target.velocity.with_y(0.0) * t_lead
}

/// `Evade` aim point: `AI_EVADE_DISTANCE` out, perpendicular to the
/// threat's *relative* velocity (not to the position offset) — that's the
/// direction that grows the miss distance fastest — signed away from the
/// threat's current position.
fn evade_aim(me: &CarView, threat: &Threat) -> Vec3 {
    let planar_pos = me.position.with_y(0.0);
    let dir = threat.rel_v.normalize_or_zero();
    if dir == Vec3::ZERO {
        return planar_pos; // shouldn't happen: a Threat requires closing speed above AI_THREAT_MIN_CLOSING
    }
    let mut perp = Vec3::Y.cross(dir).normalize_or_zero();
    let away = planar_pos - threat.position;
    if perp.dot(away) < 0.0 {
        perp = -perp;
    }
    planar_pos + perp * AI_EVADE_DISTANCE
}

/// `Recover` aim point: the nearest power-up within
/// `AI_RECOVER_POWERUP_RANGE`, or — if none is close enough — a point away
/// from the field via inverse-square repulsion from every other car, so
/// the nearest rival dominates the escape direction (what "run for open
/// space" actually means, rather than fleeing the crowd's centroid).
fn recover_aim(me: &CarView, views: &[CarView], refuges: &[Vec3]) -> Vec3 {
    let planar_pos = me.position.with_y(0.0);

    let nearest_pickup = refuges
        .iter()
        .map(|&anchor| (anchor, planar_pos.distance(anchor.with_y(0.0))))
        .filter(|&(_, dist)| dist <= AI_RECOVER_POWERUP_RANGE)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((anchor, _)) = nearest_pickup {
        return anchor;
    }

    let mut repulse = Vec3::ZERO;
    for other in views.iter().filter(|v| v.entity != me.entity) {
        let delta = planar_pos - other.position.with_y(0.0);
        let dist_sq = delta.length_squared().max(0.01);
        repulse += delta / dist_sq;
    }
    let dir = repulse.normalize_or_zero();
    if dir == Vec3::ZERO {
        return planar_pos;
    }
    planar_pos + dir * AI_REFUGE_DISTANCE
}

/// The world-space point this tick's state wants the car aimed at.
fn goal_aim(
    state: AiState,
    me: &CarView,
    target: Option<&CarView>,
    threat: Option<&Threat>,
    views: &[CarView],
    refuges: &[Vec3],
) -> Vec3 {
    match state {
        AiState::Brace => threat.map_or(me.position, |t| t.lead),
        AiState::Evade => threat.map_or(me.position, |t| evade_aim(me, t)),
        AiState::Recover { .. } => recover_aim(me, views, refuges),
        AiState::Hunt | AiState::Retreat { .. } => target.map_or(me.position, |t| lead_aim(me, t)),
    }
}

// --- The system --------------------------------------------------------------

/// Writes each AI car's `DriveInput` from the world state. Runs in
/// `FixedUpdate` ahead of `vehicle_controller` (chained), so the intent it
/// writes is consumed the same tick.
pub fn ai_drivers(
    time: Res<Time>,
    tuning: Res<VehicleTuning>,
    spatial_query: SpatialQuery,
    cars: Query<(Entity, &Transform, &LinearVelocity, &Damage, Has<Immunity>, &CarClass), With<Vehicle>>,
    bodies: Query<(&RigidBody, Option<&Mass>)>,
    pickups: Query<&PowerUp>,
    // `Without<Wrecked>` is largely redundant with `check_wrecks` removing
    // `AiDriver` on KO (this query wouldn't match a wrecked car either
    // way) — kept as an explicit guarantee that a wrecked car never drives
    // again, independent of that removal happening to land first.
    mut ai_cars: Query<(Entity, &mut DriveInput, &mut AiDriver), Without<Wrecked>>,
    mut telemetry: Local<f32>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    // One snapshot of every car, built once — the AI queries share no
    // component with `ai_cars`, so nothing here conflicts with the
    // `&mut DriveInput`/`&mut AiDriver` fetch below (see the module doc's
    // `Forces` warning for the trap this avoids).
    let views: Vec<CarView> = cars
        .iter()
        .map(|(entity, transform, linvel, damage, immune, class)| CarView {
            entity,
            position: transform.translation,
            velocity: linvel.0,
            rotation: transform.rotation,
            condition: damage.condition(),
            shield: damage.shield,
            immune,
            class: *class,
        })
        .collect();
    let obstacle_filter = SpatialQueryFilter::from_excluded_entities(views.iter().map(|v| v.entity));
    let refuges: Vec<Vec3> = pickups.iter().map(|p| p.anchor).collect();

    *telemetry += dt;
    let emit_telemetry = *telemetry >= AI_TELEMETRY_INTERVAL;
    if emit_telemetry {
        *telemetry = 0.0;
    }

    for (entity, mut input, mut ai) in &mut ai_cars {
        let Some(&me) = views.iter().find(|v| v.entity == entity) else {
            continue;
        };
        let planar_pos = me.position.with_y(0.0);
        let planar_vel = me.velocity.with_y(0.0);
        let planar_speed = planar_vel.length();
        let heading = (me.rotation * Vec3::Z).with_y(0.0).normalize_or_zero();
        let right = (me.rotation * Vec3::X).with_y(0.0).normalize_or_zero();

        // 1. TARGET — re-pick on expiry, on the target despawning, or on
        // it running too far to chase while someone closer is available;
        // otherwise stay committed. `target_far` is deliberately gated on
        // `anyone_in_range`: without that gate, whenever *every* other car
        // is beyond `AI_TARGET_DROP_RANGE` — which is every car's exact
        // situation at spawn, `SPAWN_RING_RADIUS` (52) apart on 90° spawn
        // angles puts every pair at least 73 m apart — dropping the target
        // just makes `select_target`'s fallback pick another equally
        // out-of-range car, which fails the same check again next tick:
        // an every-tick reselect storm instead of a periodic one. Gating
        // on a nearer alternative existing keeps the "don't chase someone
        // who ran across the map" intent without the thrash.
        ai.retarget_timer -= dt;
        let target_gone = ai.target.is_some_and(|t| !views.iter().any(|v| v.entity == t));
        let anyone_in_range = views.iter().any(|v| {
            v.entity != entity && planar_pos.distance(v.position.with_y(0.0)) <= AI_TARGET_DROP_RANGE
        });
        let target_far = anyone_in_range
            && ai
                .target
                .and_then(|t| views.iter().find(|v| v.entity == t))
                .is_some_and(|t| planar_pos.distance(t.position.with_y(0.0)) > AI_TARGET_DROP_RANGE);
        if ai.retarget_timer <= 0.0 || target_gone || target_far {
            match select_target(&me, &views, &mut ai) {
                Some((picked, via_cone)) => {
                    ai.target = Some(picked);
                    info!(
                        "ai: car {entity} targets {picked} ({})",
                        if via_cone { "cone" } else { "random" }
                    );
                }
                None => ai.target = None,
            }
            let jitter = AI_RETARGET_JITTER * (2.0 * ai.unit() - 1.0);
            ai.retarget_timer = AI_RETARGET_TIME * (1.0 + jitter);
        }
        let target_view = ai.target.and_then(|t| views.iter().find(|v| v.entity == t));

        // 2. THREAT — skipped while immune: an incoming hit does nothing,
        // so bracing or dodging it would be pointless.
        let threat = if me.immune { None } else { assess_threat(&me, &views) };

        // 3. DECIDE
        let can_brace = threat.is_some_and(|t| {
            t.bearing.abs() <= AI_BRACE_YAW_RATE * t.t_ca
                && me.shield > AI_BRACE_MIN_SHIELD
                && me.condition > AI_BRACE_MIN_CONDITION
        });
        let before_kind = std::mem::discriminant(&ai.state);
        ai.decide(threat.as_ref(), can_brace, me.condition, dt);
        if std::mem::discriminant(&ai.state) != before_kind {
            info!(
                "ai: car {entity} -> {} (target {:?}, sev {:.2}, cond {:.2})",
                ai.state.label(),
                ai.target,
                threat.map_or(0.0, |t| t.severity),
                me.condition,
            );
        }

        // 4. AIM — goal bearing from the now-current state, used both for
        // steering and as the obstacle whiskers' tie-break.
        let aim = goal_aim(ai.state, &me, target_view, threat.as_ref(), &views, &refuges);
        let goal_bearing = bearing_to(me.rotation, planar_pos, aim.with_y(0.0));

        // 5. SENSE — obstacle whiskers and the predictive wall breach.
        let blockage = sense_obstacles(
            &spatial_query,
            &bodies,
            &obstacle_filter,
            me.position,
            heading,
            right,
            planar_speed,
            goal_bearing,
            me.class.spec().chassis_size,
            tuning.ground_normal_min,
        );
        let wall = wall_escape(me.position, me.velocity, heading);

        // 6. ARBITRATE — steering: hard constraints override in priority
        // order (obstacle over wall — a measured hit distance beats a
        // straight-line prediction that ignores current steering),
        // everything else blends. `Retreat` keeps its original, dedicated
        // behavior untouched.
        let retreating = matches!(ai.state, AiState::Retreat { .. });
        let steer_bearing = if retreating {
            -goal_bearing
        } else if let Some(b) = blockage.filter(|b| b.hard) {
            b.turn_sign * AI_OBSTACLE_HARD_BEARING
        } else if let Some(escape) = wall {
            bearing_to(me.rotation, planar_pos, escape)
        } else {
            let soft_nudge = blockage.map_or(0.0, |b| {
                AI_OBSTACLE_STEER * (1.0 - b.min_distance / b.range) * b.turn_sign
            });
            goal_bearing + soft_nudge
        };
        input.steer = (steer_bearing * AI_STEER_GAIN).clamp(-1.0, 1.0);

        // Throttle: lift off to turn (shares the tire's friction circle
        // between drive and lateral force with `vehicle_controller`, so
        // easing off genuinely tightens the turn); brake only into a
        // head-on threat (killing the Δv a head-on hit would deal), full
        // throttle away from a side threat (which needs the grip to
        // leave); a hard obstacle always brakes regardless of state.
        let mut drive = match ai.state {
            AiState::Hunt => {
                if steer_bearing.abs() > AI_TURN_LIFT_BEARING {
                    AI_THROTTLE_TURN
                } else {
                    1.0
                }
            }
            AiState::Brace => {
                let bearing = threat.map_or(0.0, |t| t.bearing);
                if bearing.abs() <= AI_BRACE_ALIGNED {
                    1.0
                } else {
                    AI_THROTTLE_TURN
                }
            }
            AiState::Evade => {
                let bearing = threat.map_or(0.0, |t| t.bearing);
                if bearing.abs() < AI_EVADE_BRAKE_BEARING {
                    -1.0
                } else {
                    1.0
                }
            }
            AiState::Recover { .. } => 1.0,
            AiState::Retreat { .. } => -1.0,
        };
        if !retreating && blockage.is_some_and(|b| b.hard) {
            drive = -1.0;
        }
        input.drive = drive;

        // Handbrake: a pure function of the final steer bearing and
        // speed, so every state gets the swing for free — a `Brace`
        // needing 90 degrees of rotation at speed yanks it, exactly the
        // move, and `vehicle_controller` already bypasses the
        // speed-limited steering lock while it's held.
        input.handbrake = !retreating
            && drive > 0.1
            && steer_bearing.abs() > AI_HANDBRAKE_BEARING
            && planar_speed > AI_HANDBRAKE_MIN_SPEED;

        // 7. STUCK — keyed on the drive command actually written this
        // tick, not on which state asked for it, so an `Evade` braking
        // into a corner still triggers a retreat. `Retreat` doesn't
        // re-trigger itself.
        if !retreating {
            if input.drive > 0.1 && planar_speed < AI_STUCK_SPEED {
                ai.stuck_timer += dt;
                if ai.stuck_timer >= AI_STUCK_TIME {
                    ai.stuck_timer = 0.0;
                    ai.state = AiState::Retreat {
                        remaining: AI_RETREAT_TIME,
                    };
                    info!("ai: car {entity} stuck, retreating");
                }
            } else {
                ai.stuck_timer = 0.0;
            }
        }

        if emit_telemetry {
            info!(
                "ai: car {entity} {} target {:?} cond {:.2} sev {:.2} v {planar_speed:.1} r {:.1} block {:?}",
                ai.state.label(),
                ai.target,
                me.condition,
                threat.map_or(0.0, |t| t.severity),
                planar_pos.length(),
                blockage.map(|b| b.min_distance),
            );
        }
    }
}
