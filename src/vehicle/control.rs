//! The driving model: one Dynamic chassis rigid body plus four non-physical
//! wheels that are raycasts.
//!
//! Each fixed step, every wheel casts a ray along chassis-down from its mount
//! and, if it finds ground (a surface facing sufficiently up), contributes a
//! spring/damper suspension force and slip-based tire forces. Tire forces are
//! capped twice: by the friction circle (`mu · N`, where `N` is that wheel's
//! *live* suspension load — so an unloaded wheel grips less, and throttle
//! spends lateral budget), and by the one-step impulse clamp (no force may
//! reverse its own slip within one tick), which is what keeps an explicit
//! fixed-step integrator of stiff tire forces from jittering. A wheel that
//! finds nothing under it contributes nothing, so the chassis free-falls under
//! gravity.
//!
//! What this model deliberately does *not* have: wheel angular dynamics. There
//! is no per-wheel spin state, no slip ratio, no wheelspin or lockup, no
//! differential. Longitudinal force is commanded directly. That's the
//! semi-sim trade — it costs realism at the traction limit and buys a model
//! that's stable at 64 Hz and tunable by feel.

use bevy::prelude::*;

use avian3d::prelude::*;

use super::spec::{VehicleGeometry, VehicleSpec, VehicleTuning};
use super::wheel::{Wheel, WheelLanding};

/// Driving *intent* for one chassis, consumed by [`vehicle_controller`] each
/// fixed step.
///
/// This is the seam between "who is driving" and "how the car responds":
/// write it from the keyboard (see [`input`](super::input)), from a gamepad,
/// from an AI, from a replay. The controller reads it and cannot tell the
/// sources apart.
///
/// Distinct from [`Vehicle`], which holds the *smoothed actuation* derived
/// from this. Note also that whatever writes this must not touch the
/// chassis's velocity components: [`Forces`] statically declares mutable
/// access to `LinearVelocity`/`AngularVelocity`, so a driver system that also
/// fetches them will conflict with the controller's query at runtime.
#[derive(Component, Default, Debug)]
pub struct DriveInput {
    /// Forward/backward pedal, -1..=1. Positive is "wants to go forward";
    /// whether that means throttle or brake depends on which way the car is
    /// currently moving.
    ///
    /// The magnitude is honoured rather than quantized, so a driver can lift
    /// partway off the throttle to spend more of the friction circle on
    /// cornering instead of driving a wide arc at full power.
    pub drive: f32,
    /// Steering, -1..=1. Positive turns toward chassis-local +X.
    pub steer: f32,
    pub handbrake: bool,
}

/// Scales the engine force this chassis can put down, 0..=1-ish.
///
/// Defaults to 1.0 (full power) and nothing in this module ever writes it —
/// it exists so a game can make a damaged engine limp, a cold engine start
/// weak, or a power-up boost, without the controller knowing anything about
/// damage or power-ups. Multiplies drive and reverse force only; braking and
/// the handbrake are unaffected.
#[derive(Component, Debug)]
pub struct DrivePower(pub f32);

impl Default for DrivePower {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Smoothed actuation state for one chassis.
///
/// `throttle` and `steer` are the values actually used for forces this step;
/// they ease toward [`DriveInput`]'s targets at
/// [`VehicleTuning::steer_lerp_rate`], so stomping a key doesn't produce an
/// instant force step. Read them for a HUD; the controller owns writing them.
#[derive(Component, Debug)]
pub struct Vehicle {
    pub throttle: f32,
    pub steer: f32,
    /// Raw handbrake state — no smoothing, because a handbrake is a binary
    /// yank. While true, the non-steering axle runs locked.
    pub handbrake: bool,
    /// Distance between front and rear axles, from [`VehicleGeometry`] at
    /// spawn. Kept here rather than recomputed so the steering model can
    /// never disagree with where the wheels actually are.
    pub wheelbase: f32,
    /// Distance between left and right wheels, same rationale.
    pub track_width: f32,
}

impl Vehicle {
    pub fn new(geometry: VehicleGeometry) -> Self {
        Self {
            throttle: 0.0,
            steer: 0.0,
            handbrake: false,
            wheelbase: geometry.wheelbase,
            track_width: geometry.track_width,
        }
    }
}

/// What the pedals mean this tick, resolved from [`DriveInput::drive`] against
/// the current signed forward speed: a backward pedal while rolling forward is
/// a brake, the same pedal from near rest is reverse — and symmetrically.
#[derive(Clone, Copy, PartialEq, Debug)]
enum DriveCmd {
    Drive,
    Reverse,
    Brake,
    Coast,
}

/// Everything a driveable chassis needs: the rigid body, its collider and
/// mass properties from the spec, and the control components.
///
/// Add your own visuals, name, and gameplay components alongside it, then
/// spawn [`wheel_bundle`](super::wheel_bundle) children:
///
/// ```ignore
/// let mut car = commands.spawn((
///     chassis_bundle(spec, &tuning, VehicleGeometry::from_mounts(&spec.wheel_mounts)),
///     Mesh3d(body_mesh.clone()),
///     MeshMaterial3d(paint.clone()),
///     Transform::from_xyz(0.0, spec.spawn_height, 0.0),
/// ));
/// car.with_children(|chassis| {
///     for mount in spec.wheel_mounts {
///         chassis.spawn(wheel_bundle(&spec, Transform::from_translation(mount), mount.z > 0.0, true));
///     }
/// });
/// ```
///
/// The bundle does *not* include a `Transform` — spawn the car where you want
/// it, at least [`VehicleSpec::spawn_height`] off the ground so its wheels
/// start within raycast range.
pub fn chassis_bundle(spec: VehicleSpec, tuning: &VehicleTuning, geometry: VehicleGeometry) -> impl Bundle {
    (
        Vehicle::new(geometry),
        spec,
        DriveInput::default(),
        DrivePower::default(),
        RigidBody::Dynamic,
        Collider::cuboid(spec.collider_size.x, spec.collider_size.y, spec.collider_size.z),
        Mass(spec.mass),
        CenterOfMass(spec.com_offset),
        LinearDamping(tuning.chassis_linear_damping),
        AngularDamping(tuning.chassis_angular_damping),
        Friction::new(tuning.chassis_friction),
        Restitution::new(tuning.chassis_restitution),
    )
}

/// The per-tick driving model — see the module docs for the shape of it.
///
/// Runs in `FixedUpdate` under [`VehicleSet::Control`](super::VehicleSet),
/// ahead of Avian's physics step in `FixedPostUpdate`, so the forces it
/// accumulates are the ones integrated this tick.
///
/// Deliberately ungated by any game state: a car needs its suspension every
/// tick to sit on its springs, including while nobody is allowed to drive it.
/// Stop the *input*, not the controller.
// The chassis query really is this wide — a `type` alias for it would only
// move the same tuple somewhere you have to go look it up.
#[allow(clippy::type_complexity)]
pub fn vehicle_controller(
    time: Res<Time>,
    tuning: Res<VehicleTuning>,
    spatial_query: SpatialQuery,
    // Velocity comes from `forces` itself (`linear_velocity()`,
    // `velocity_at_point()`) rather than separate `&LinearVelocity`/
    // `&AngularVelocity` fetches: `Forces` statically declares mutable access
    // to both (it needs it to support impulses), so an extra read-only fetch
    // of the same components in this query would panic as a conflicting
    // access, even though this system never calls an impulse method.
    mut chassis_q: Query<(
        Entity,
        &Transform,
        &mut Vehicle,
        &DriveInput,
        &DrivePower,
        &VehicleSpec,
        &Children,
        Forces,
    )>,
    mut wheels_q: Query<&mut Wheel>,
    mut landings: MessageWriter<WheelLanding>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (chassis_entity, transform, mut vehicle, input, power, spec, children, mut forces) in
        &mut chassis_q
    {
        let forward_pedal = input.drive > 0.1;
        let back_pedal = input.drive < -0.1;
        let handbrake = input.handbrake;

        vehicle.handbrake = handbrake;
        let chassis_up = transform.rotation * Vec3::Y;
        let chassis_forward = transform.rotation * Vec3::Z;
        let linvel = forces.linear_velocity();
        let forward_speed = linvel.dot(chassis_forward);

        let cmd = match (forward_pedal, back_pedal) {
            (true, false) if forward_speed < -tuning.reverse_threshold => DriveCmd::Brake,
            (true, false) => DriveCmd::Drive,
            (false, true) if forward_speed > tuning.reverse_threshold => DriveCmd::Brake,
            (false, true) => DriveCmd::Reverse,
            _ => DriveCmd::Coast,
        };
        let throttle_target = match cmd {
            DriveCmd::Drive => input.drive.min(1.0),
            DriveCmd::Reverse => input.drive.max(-1.0),
            _ => 0.0,
        };

        // Steering lock limited by lateral acceleration: at speed `v` a steer
        // angle `d` on wheelbase `L` asks for roughly `v² · tan(d) / L` of
        // lateral acceleration, so the lock is capped at the angle requesting
        // `spec.steer_accel_limit`.
        let planar_vel = linvel - chassis_up * linvel.dot(chassis_up);
        let planar_speed = planar_vel.length();
        // Under handbrake the speed-based cap is bypassed: it exists to stop
        // full-lock spins while gripping, but drift entry and counter-steer
        // both need the whole steering range — sliding is the point.
        let lock = if planar_speed > 1.0 && !handbrake {
            spec.max_steer_angle
                .min((spec.steer_accel_limit * vehicle.wheelbase / planar_speed.powi(2)).atan())
        } else {
            spec.max_steer_angle
        };
        let steer_target = input.steer * lock;

        let ease = (tuning.steer_lerp_rate * dt).clamp(0.0, 1.0);
        vehicle.throttle += (throttle_target - vehicle.throttle) * ease;
        vehicle.steer += (steer_target - vehicle.steer) * ease;

        let filter = SpatialQueryFilter::from_excluded_entities([chassis_entity]);
        // Cast along chassis-down, not world-down: the suspension is bolted to
        // the chassis, which is what keeps it working on ramps and banks.
        let ray_dir = Dir3::new(-chassis_up).unwrap_or(Dir3::NEG_Y);
        let com_world = transform.translation + transform.rotation * spec.com_offset;
        let corner_mass = spec.corner_mass();
        let spring_rate = spec.spring_rate(&tuning);
        let damping_rate = spec.damping_rate(&tuning);
        let max_spring_force = spec.max_spring_force(&tuning);
        let max_damper_force = spec.max_damper_force(&tuning);
        let tire_lat_stiffness = spec.tire_lat_stiffness(&tuning);
        // The one-step impulse clamp: the largest force that could zero
        // `slip` within this tick for one corner's share of the mass, times a
        // sub-1 safety factor (forces apply off-CoM, so they also rotate the
        // body). Slip-opposing forces above this overshoot and jitter.
        let clamp_slip = |force: f32, slip: f32| -> f32 {
            let cap = tuning.tire_max_impulse_factor * corner_mass * slip.abs() / dt;
            force.clamp(-cap, cap)
        };

        for child in children.iter() {
            let Ok(mut wheel) = wheels_q.get_mut(child) else {
                continue;
            };
            let max_ray = spec.suspension_rest + wheel.radius;
            let mount_world = transform.translation + transform.rotation * wheel.mount_local;

            let hit = spatial_query
                .cast_ray(mount_world, ray_dir, max_ray, true, &filter)
                // Only surfaces facing sufficiently up count as ground — a
                // wall grazing the ray is not something to stand on.
                .filter(|hit| hit.normal.dot(chassis_up) >= tuning.ground_normal_min);
            let Some(hit) = hit else {
                // Airborne: no ground under the wheel, no force from it.
                wheel.travel = spec.suspension_rest;
                wheel.slip_speed = 0.0;
                wheel.contact_world = None;
                continue;
            };

            wheel.travel = (hit.distance - wheel.radius).clamp(0.0, spec.suspension_rest);
            let compression = spec.suspension_rest - wheel.travel;
            let contact_point = mount_world + ray_dir * hit.distance;
            // First grounded tick after being airborne — a touchdown, not
            // ongoing ride contact. Captured before this tick's fields are
            // overwritten below.
            let just_landed = wheel.contact_world.is_none();

            // Velocity of the chassis material point at the contact, used for
            // both the suspension damper and tire slip.
            let point_vel = forces.velocity_at_point(contact_point);

            // Suspension: spring minus damper along the suspension axis.
            // Split into two caps, not one: the spring stores energy and can
            // fling the car, so it keeps a tight cap; the damper only
            // dissipates (zero the instant the wheel stops closing, and the
            // plain linear term on rebound), so it can never launch anything
            // and gets a much higher cap plus a velocity-progressive term
            // that only bites above the knee speed. That's what lets a hard
            // landing settle onto the springs instead of bottoming out into
            // the chassis collider, without changing ordinary ride feel,
            // where closing speeds never approach the knee.
            let closing = -point_vel.dot(chassis_up);
            let spring_force = (spring_rate * compression).min(max_spring_force);
            let damper_force = if closing > 0.0 {
                (damping_rate * closing * (1.0 + closing / tuning.suspension_damper_knee))
                    .min(max_damper_force)
            } else {
                damping_rate * closing
            };
            // The magnitude doubles as this wheel's load for the tire model.
            let load = (spring_force + damper_force).max(0.0);
            forces.apply_force_at_point(chassis_up * load, contact_point);

            if just_landed && closing >= tuning.landing_thump_min_speed {
                landings.write(WheelLanding { position: contact_point, speed: closing });
            }

            // Wheel basis: chassis orientation, steered for front wheels,
            // then projected onto the contact plane so tire forces never push
            // into or out of the ground.
            let steer_angle = if wheel.steering {
                ackermann_angle(
                    vehicle.steer,
                    wheel.mount_local.x,
                    vehicle.wheelbase,
                    vehicle.track_width,
                )
            } else {
                0.0
            };
            let steer_rot = Quat::from_axis_angle(Vec3::Y, steer_angle);
            let forward_dir = transform.rotation * steer_rot * Vec3::Z;
            let right_dir = transform.rotation * steer_rot * Vec3::X;
            let forward_dir =
                (forward_dir - hit.normal * forward_dir.dot(hit.normal)).normalize_or_zero();
            let right_dir =
                (right_dir - hit.normal * right_dir.dot(hit.normal)).normalize_or_zero();
            if forward_dir == Vec3::ZERO || right_dir == Vec3::ZERO {
                continue;
            }

            let v_lat = point_vel.dot(right_dir);
            let v_long = point_vel.dot(forward_dir);
            wheel.ground_speed = v_long;
            wheel.contact_world = Some(contact_point);

            // Handbrake locks the non-steering axle: sliding tires barely
            // grip sideways, the engine drops off them (the steering pair
            // keep pulling), and a locked-wheel drag scrubs speed.
            let handbraked = handbrake && !wheel.steering;

            // A rolling tire slips only sideways; a locked one drags its whole
            // contact patch, so all of its planar velocity is slip.
            wheel.slip_speed = if handbraked {
                v_lat.hypot(v_long)
            } else {
                v_lat.abs()
            };

            let lat_stiffness = if handbraked {
                tire_lat_stiffness * tuning.handbrake_lat_stiffness_scale
            } else {
                tire_lat_stiffness
            };
            let lat_force = clamp_slip(-v_lat * lat_stiffness, v_lat);

            let long_force = if handbraked {
                clamp_slip(-v_long.signum() * tuning.handbrake_drag_force, v_long)
            } else {
                match cmd {
                    DriveCmd::Drive if wheel.driven => {
                        vehicle.throttle.max(0.0) * spec.engine_force * power.0 / 4.0
                    }
                    DriveCmd::Reverse if wheel.driven => {
                        vehicle.throttle.min(0.0) * spec.reverse_force * power.0 / 4.0
                    }
                    DriveCmd::Brake => clamp_slip(-v_long.signum() * spec.brake_force, v_long),
                    _ => clamp_slip(-v_long.signum() * tuning.rolling_resistance, v_long),
                }
            };

            // Friction circle: lateral and longitudinal force share one grip
            // budget proportional to the wheel's live load. This is where load
            // transfer, throttle-on oversteer, and braking understeer all come
            // from.
            let mu = if wheel.steering { spec.mu_front } else { spec.mu_rear }
                * if handbraked { tuning.handbrake_mu_scale } else { 1.0 };
            let cap = mu * load;
            let mag = lat_force.hypot(long_force);
            let scale = if mag > cap && mag > 0.0 { cap / mag } else { 1.0 };

            forces.apply_force_at_point(forward_dir * long_force * scale, contact_point);
            // The lateral force applies part-way up toward CoM height: same
            // yaw torque, much less roll torque.
            let lift =
                (com_world - contact_point).dot(chassis_up).max(0.0) * tuning.tire_lat_force_height;
            forces.apply_force_at_point(
                right_dir * lat_force * scale,
                contact_point + chassis_up * lift,
            );
        }

        // Drag rises as (v / top_speed)^n, walling off hard at `top_speed`
        // while staying out of the way through the mid-range.
        let speed = linvel.length();
        let drag = spec.engine_force * (speed / spec.top_speed).powi(tuning.drag_exponent);
        forces.apply_force(-linvel.normalize_or_zero() * drag);

        flip_rescue(chassis_up, &tuning, &mut forces);
    }
}

/// Ackermann steering split: both front wheels steer around the same turn
/// center (on the rear axle line), so the inner wheel — closer to that center
/// — steers more than the outer one.
///
/// `steer` is the nominal bicycle-model angle and `mount_x` picks the side.
/// Positive `steer` turns toward +X, so the +X-side wheel is the inner one.
pub fn ackermann_angle(steer: f32, mount_x: f32, wheelbase: f32, track_width: f32) -> f32 {
    if steer.abs() < 1e-3 {
        return steer;
    }
    let radius = wheelbase / steer.abs().tan();
    let half_track = track_width / 2.0;
    let inner = mount_x.signum() == steer.signum();
    let wheel_radius = if inner {
        (radius - half_track).max(0.2)
    } else {
        radius + half_track
    };
    (wheelbase / wheel_radius).atan().copysign(steer)
}

/// Rescue torque for genuine rollovers only: zero below
/// [`VehicleTuning::flip_threshold`], ramping to full by twice that, righting
/// the chassis toward world-up. The damping term acts only on the angular
/// velocity component perpendicular to world up, so it never fights yaw spin.
fn flip_rescue(chassis_up: Vec3, tuning: &VehicleTuning, forces: &mut impl WriteRigidBodyForces) {
    let up_dot = chassis_up.dot(Vec3::Y).clamp(-1.0, 1.0);
    let tilt_angle = up_dot.acos();
    let ramp = ((tilt_angle - tuning.flip_threshold) / tuning.flip_threshold).clamp(0.0, 1.0);
    if ramp <= 0.0 {
        return;
    }

    let axis = chassis_up.cross(Vec3::Y);
    let Ok(axis) = Dir3::new(axis) else {
        // chassis_up is (anti)parallel to world up: either level (tilt_angle
        // ~0, so ramp is already 0) or perfectly upside down, where the
        // torque axis is undefined — let gravity and collisions break the tie.
        return;
    };

    let angular_velocity = forces.angular_velocity();
    let perp_angular_velocity = angular_velocity - Vec3::Y * angular_velocity.dot(Vec3::Y);

    let torque = axis * tilt_angle * tuning.flip_stiffness * ramp
        - perp_angular_velocity * tuning.flip_damping * ramp;
    forces.apply_torque(torque);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHEELBASE: f32 = 2.6;
    const TRACK: f32 = 1.8;

    /// The whole point of Ackermann: the wheel on the inside of the turn is
    /// closer to the turn center, so it must be cranked over further.
    #[test]
    fn the_inner_wheel_steers_more_than_the_outer() {
        // Positive steer turns toward +X, so +X is the inside of the turn.
        let inner = ackermann_angle(0.4, 0.9, WHEELBASE, TRACK);
        let outer = ackermann_angle(0.4, -0.9, WHEELBASE, TRACK);
        assert!(inner > outer, "inner {inner} should exceed outer {outer}");
        // ...and the nominal angle sits between them.
        assert!(inner > 0.4 && outer < 0.4);
    }

    /// Steering right must mirror steering left exactly, or the car pulls to
    /// one side.
    #[test]
    fn the_split_is_symmetric_across_the_center_line() {
        let left = ackermann_angle(0.4, 0.9, WHEELBASE, TRACK);
        let right = ackermann_angle(-0.4, -0.9, WHEELBASE, TRACK);
        assert!((left + right).abs() < 1e-6, "{left} vs {right}");
    }

    /// Near center the geometry degenerates (turn radius goes to infinity),
    /// so the input passes straight through instead of dividing by ~zero.
    #[test]
    fn near_zero_steering_passes_through_untouched() {
        assert_eq!(ackermann_angle(0.0, 0.9, WHEELBASE, TRACK), 0.0);
        assert_eq!(ackermann_angle(1e-4, 0.9, WHEELBASE, TRACK), 1e-4);
    }

    #[test]
    fn the_split_keeps_the_sign_of_the_input() {
        assert!(ackermann_angle(-0.4, 0.9, WHEELBASE, TRACK) < 0.0);
        assert!(ackermann_angle(-0.4, -0.9, WHEELBASE, TRACK) < 0.0);
    }

    /// At full lock the inner turn radius can fall inside the half-track,
    /// which would flip the angle's sign or blow it up. It's floored instead.
    #[test]
    fn full_lock_on_a_wide_track_stays_finite_and_signed() {
        let angle = ackermann_angle(1.4, 0.9, WHEELBASE, TRACK);
        assert!(angle.is_finite() && angle > 0.0, "{angle}");
    }
}
