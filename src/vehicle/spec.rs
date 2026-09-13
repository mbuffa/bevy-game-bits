//! What a vehicle *is* (`VehicleSpec`, per chassis) and how vehicles in
//! general *feel* (`VehicleTuning`, one per app).
//!
//! The split is deliberate. `VehicleSpec` holds the numbers that distinguish
//! one car from another — mass, geometry, engine force, grip — and rides on
//! the chassis entity as a component, so a game can have as many distinct
//! vehicles as it likes without any class registry or lookup table.
//! `VehicleTuning` holds the numbers that describe the *model itself*:
//! suspension frequency, tire saturation slip, the impulse-clamp safety
//! factor. Those are shared by every vehicle in a game, and changing one
//! changes how all of them feel.
//!
//! Stiffnesses are derived, not tabled. A spring rate written down per class
//! can silently disagree with that class's mass; `spring_rate()` computed from
//! `m·(2πf)²` cannot. Heavier vehicles automatically get stiffer springs,
//! lighter ones softer ones, at the same felt frequency.

use std::f32::consts::TAU;

use bevy::prelude::*;

/// Standard gravity (m/s²), used only to turn `VehicleTuning`'s design-target
/// multiples-of-g into force numbers. Avian's own default gravity matches
/// this, so a vehicle's static suspension compression comes out where the
/// derivation expects.
pub const GRAVITY: f32 = 9.81;

/// One vehicle's physical spec, carried on its chassis entity.
///
/// `Copy` and entirely `const`-constructible, so a game can keep a plain
/// `const` array of these with no startup cost — but nothing in this module
/// requires that. The controller reads the spec off the entity, so two
/// vehicles in the same world can differ in every field.
///
/// Wheelbase and track width are deliberately *not* fields: they're
/// [`VehicleGeometry`], passed separately at spawn, because a game loading its
/// wheel positions from an artist-authored model needs them derived from that
/// model rather than from a second source that could disagree with it.
#[derive(Component, Clone, Copy, Debug)]
pub struct VehicleSpec {
    // --- Chassis ---
    /// Visual body box (x = width, y = height, z = length). The controller
    /// never reads this; it's here so gameplay code (AI whiskers, nose
    /// offsets, spawn spacing) has one place to ask how big the car is.
    pub chassis_size: Vec3,
    /// Physics collider half-extents' source box — usually a little smaller
    /// than `chassis_size`, since the ray suspension alone carries the car in
    /// normal driving and the collider is for wall hits, landings and
    /// car-vs-car contact.
    pub collider_size: Vec3,
    pub mass: f32,
    /// Center of mass in chassis-local space, normally offset *below* the
    /// body center — the single biggest rollover-resistance lever.
    pub com_offset: Vec3,
    /// Chassis-center height to spawn at. Must keep every wheel mount within
    /// suspension raycast range, or the car free-falls onto its springs and
    /// bounces instead of settling.
    pub spawn_height: f32,

    // --- Wheels ---
    pub wheel_radius: f32,
    pub wheel_width: f32,
    /// Mount points in chassis-local space. Positive Z is forward, positive X
    /// is the side positive steer turns toward. The controller reads mounts
    /// off each [`Wheel`](super::Wheel) entity rather than from here, so this
    /// is the authoring source (and what [`VehicleGeometry::from_mounts`]
    /// measures); a game whose wheel poses come from a model can leave it as
    /// whatever it exported from.
    pub wheel_mounts: [Vec3; 4],
    /// Fully-extended suspension travel: the distance from mount to wheel
    /// center with no load on it.
    pub suspension_rest: f32,

    // --- Tires ---
    pub mu_front: f32,
    pub mu_rear: f32,

    // --- Drive ---
    /// Total engine force, split across the driven wheels and capped by their
    /// friction circles. Setting it just under the driven axle's static-load
    /// friction cap makes a standing launch grip-limited rather than leaving
    /// grip unused — what makes it feel like a car and not a rocket.
    pub engine_force: f32,
    pub brake_force: f32,
    pub reverse_force: f32,
    /// Straight-line top speed (m/s): the speed at which the undamaged engine
    /// force and the aero drag curve cancel. Expressed as a target speed
    /// rather than a drag coefficient because the target is the thing you
    /// actually want to tune.
    pub top_speed: f32,

    // --- Steering ---
    /// Maximum front-wheel angle (radians) at a standstill.
    pub max_steer_angle: f32,
    /// Lateral acceleration (m/s²) the steering lock is allowed to *request*
    /// at speed. Set just above what the tires can hold, so full lock at pace
    /// slides controllably instead of being either impossible or an instant
    /// spin.
    pub steer_accel_limit: f32,

    /// Multiplier on impact damage this vehicle takes, on top of the mass
    /// effect already implicit in `Δv = impulse / mass` — a lighter car takes
    /// more Δv from the same collision for free. Read by
    /// [`impact`](super::impact); the controller ignores it.
    pub damage_scale: f32,
}

impl VehicleSpec {
    /// One corner's share of the mass — the reference load every derived
    /// suspension and tire rate is scaled from.
    pub fn corner_mass(&self) -> f32 {
        self.mass / 4.0
    }

    /// `k = m·(2πf)²` — the spring rate that puts this corner at the tuning's
    /// natural frequency for its own share of the mass.
    pub fn spring_rate(&self, tuning: &VehicleTuning) -> f32 {
        self.corner_mass() * (TAU * tuning.suspension_freq_hz).powi(2)
    }

    /// `c = 2ζ√(k·m)` — damper rate at the tuning's fraction of critical.
    pub fn damping_rate(&self, tuning: &VehicleTuning) -> f32 {
        2.0 * tuning.suspension_damping_ratio
            * (self.spring_rate(tuning) * self.corner_mass()).sqrt()
    }

    /// Static corner load × `suspension_max_g`. A tight cap: the spring stores
    /// energy and can fling the car if it's allowed to push too hard.
    pub fn max_spring_force(&self, tuning: &VehicleTuning) -> f32 {
        self.corner_mass() * GRAVITY * tuning.suspension_max_g
    }

    /// Static corner load × `suspension_damper_max_g` — the compression
    /// damper's own, much higher cap. Safe to raise where the spring's isn't,
    /// because a damper only ever dissipates: it's zero the instant the wheel
    /// stops closing, so a high cap can soak up a hard landing faster but can
    /// never launch anything.
    pub fn max_damper_force(&self, tuning: &VehicleTuning) -> f32 {
        self.corner_mass() * GRAVITY * tuning.suspension_damper_max_g
    }

    /// `mu_front · corner load / saturation slip` — how many newtons of
    /// lateral force one metre-per-second of sideways slip asks for. The
    /// friction circle clamps well before this ever runs away, so it sets
    /// *how sharp* the tire feels, not a real limit.
    pub fn tire_lat_stiffness(&self, tuning: &VehicleTuning) -> f32 {
        self.mu_front * self.corner_mass() * GRAVITY / tuning.tire_saturation_slip
    }
}

/// Axle geometry, measured rather than tabled.
///
/// Kept out of [`VehicleSpec`] so a game whose wheel poses come from an
/// artist-authored model derives these from that model — one source of truth
/// for where the wheels actually are. Games with procedural wheels can call
/// [`VehicleGeometry::from_mounts`] on the spec's own mounts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleGeometry {
    /// Distance between the front and rear axles.
    pub wheelbase: f32,
    /// Distance between the left and right wheels.
    pub track_width: f32,
}

impl VehicleGeometry {
    /// Measures wheelbase and track from four mount points, in any order:
    /// the full Z spread and the full X spread of the set.
    pub fn from_mounts(mounts: &[Vec3; 4]) -> Self {
        let spread = |axis: fn(&Vec3) -> f32| {
            let (min, max) = mounts
                .iter()
                .fold((f32::MAX, f32::MIN), |(min, max), mount| {
                    (min.min(axis(mount)), max.max(axis(mount)))
                });
            max - min
        };
        Self {
            wheelbase: spread(|mount| mount.z),
            track_width: spread(|mount| mount.x),
        }
    }
}

/// How vehicles in this app feel: the constants of the driving *model*, as
/// opposed to [`VehicleSpec`]'s per-vehicle numbers.
///
/// [`Default`] is a complete, playable arcade-sim setup — the tuning the
/// derby prototype this module was extracted from shipped with. Override
/// fields via [`VehiclePlugin::tuning`](super::VehiclePlugin::tuning).
#[derive(Resource, Clone, Copy, Debug)]
pub struct VehicleTuning {
    // --- Suspension ---
    /// Natural frequency (Hz) every vehicle's spring rate is derived from.
    /// 2 Hz is sporty but not karty.
    pub suspension_freq_hz: f32,
    /// Damping ratio (fraction of critical). Below ~0.5 a landing bounces;
    /// near 1.0 it slams. 0.68 settles in one overshoot.
    pub suspension_damping_ratio: f32,
    /// Spring force cap in multiples of a corner's static load, so a
    /// bottomed-out landing can't launch the car regardless of its mass.
    pub suspension_max_g: f32,
    /// Compression-damper force cap, same units. Deliberately far higher than
    /// `suspension_max_g` — see [`VehicleSpec::max_damper_force`].
    pub suspension_damper_max_g: f32,
    /// Closing speed (m/s) at which the compression damper's force has
    /// roughly doubled over its linear `c·v` value. Ordinary ride motion
    /// (corners, potholes) sits well under this and barely notices it; only a
    /// hard landing's closing speed reaches it. Rebound stays plain linear.
    pub suspension_damper_knee: f32,
    /// A suspension raycast hit only counts as ground if its normal · the
    /// chassis's up axis is at least this — 0.5 is roughly a 60° slope limit,
    /// so wall faces aren't drivable surface.
    pub ground_normal_min: f32,

    // --- Tires ---
    /// Slip (m/s) at which a wheel's lateral force would reach `mu_front`
    /// times its static load if stiffness scaled linearly forever. Lower
    /// feels twitchy, higher feels numb.
    pub tire_saturation_slip: f32,
    /// Safety factor on the "zero this wheel's slip within one tick" force
    /// clamp. Slightly under 1 because forces apply off-center and so also
    /// rotate the body; this is the anti-jitter guard that lets an explicit
    /// fixed-step integrator carry stiff tire forces at all.
    pub tire_max_impulse_factor: f32,
    /// Where lateral tire force is applied along the suspension axis: 0 = at
    /// the ground contact (physical, maximum roll torque), 1 = at CoM height
    /// (no roll torque at all). Raising it trades body roll for stability
    /// while keeping the yaw torque — the standard raycast-car trade.
    pub tire_lat_force_height: f32,

    // --- Drive ---
    /// Forward speed (m/s) above which a backward pedal means brake; below
    /// it, the pedal reverses (and symmetrically while rolling backwards).
    pub reverse_threshold: f32,
    /// Constant force per wheel opposing its rolling direction when coasting,
    /// so the vehicle rolls to a stop instead of gliding forever. Impulse-
    /// clamped near rest, so it can't push the car backwards.
    pub rolling_resistance: f32,
    /// Drag curve shape: `drag = engine_force · (v / top_speed)^n`. Quadratic
    /// (n = 2) spends half the engine fighting drag at only 70% of top speed,
    /// which makes acceleration feel soggy through the mid-range; n = 4 keeps
    /// drag out of the way and then walls off hard right at `top_speed`.
    pub drag_exponent: i32,

    // --- Handbrake ---
    /// While the handbrake is held, the non-steering axle's friction-circle
    /// μ is scaled by this. Lower = looser tail, easier drift entry.
    pub handbrake_mu_scale: f32,
    /// Rear lateral stiffness scale while held — a locked, sliding tire
    /// barely resists sideways motion.
    pub handbrake_lat_stiffness_scale: f32,
    /// Locked-wheel drag per rear wheel (impulse-clamped, and it shares the
    /// reduced friction circle), so the handbrake also scrubs speed.
    pub handbrake_drag_force: f32,

    // --- Steering ---
    /// How fast steer and throttle approach their targets, in "per second"
    /// terms fed to an exponential smoothing lerp. Stops a stomped key from
    /// producing an instant force step.
    pub steer_lerp_rate: f32,

    // --- Flip rescue ---
    /// Tilt off world-up (radians) past which the rescue torque switches on,
    /// ramping to full by twice this. ~1.0 rad (57°) is well clear of any
    /// cornering roll, and a car headed for its roof sails past it.
    pub flip_threshold: f32,
    /// Proportional (spring) term of the rescue torque, at full ramp.
    pub flip_stiffness: f32,
    /// Derivative (damping) term, applied only against the angular velocity
    /// component perpendicular to world up, so it never fights yaw spin.
    pub flip_damping: f32,

    // --- Landings ---
    /// Closing speed (m/s) at touchdown below which no
    /// [`WheelLanding`](super::WheelLanding) is emitted — ordinary ride
    /// contact isn't a thump.
    pub landing_thump_min_speed: f32,

    // --- Chassis rigid body ---
    /// Zero by default: straight-line top speed comes from the aero drag
    /// curve, not from a linear damping that would also fight low-speed
    /// handling.
    pub chassis_linear_damping: f32,
    /// Mild and uniform. Yaw authority comes from the tire model; this just
    /// bleeds residual tumbling after airtime.
    pub chassis_angular_damping: f32,
    pub chassis_friction: f32,
    pub chassis_restitution: f32,
}

impl Default for VehicleTuning {
    fn default() -> Self {
        Self {
            suspension_freq_hz: 2.0,
            suspension_damping_ratio: 0.68,
            suspension_max_g: 7.0,
            suspension_damper_max_g: 14.0,
            suspension_damper_knee: 2.5,
            ground_normal_min: 0.5,
            tire_saturation_slip: 0.31,
            tire_max_impulse_factor: 0.9,
            tire_lat_force_height: 0.6,
            reverse_threshold: 0.8,
            rolling_resistance: 90.0,
            drag_exponent: 4,
            handbrake_mu_scale: 0.5,
            handbrake_lat_stiffness_scale: 0.35,
            handbrake_drag_force: 1_500.0,
            steer_lerp_rate: 10.0,
            flip_threshold: 1.0,
            flip_stiffness: 3_000.0,
            flip_damping: 700.0,
            landing_thump_min_speed: 2.5,
            chassis_linear_damping: 0.0,
            chassis_angular_damping: 0.3,
            chassis_friction: 0.6,
            chassis_restitution: 0.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec the derived-rate expectations below are calibrated against —
    /// a 350 kg arcade truck.
    fn truck() -> VehicleSpec {
        VehicleSpec {
            chassis_size: Vec3::new(1.8, 0.6, 3.6),
            collider_size: Vec3::new(1.7, 0.5, 3.4),
            mass: 350.0,
            com_offset: Vec3::new(0.0, -0.25, 0.0),
            spawn_height: 0.9,
            wheel_radius: 0.35,
            wheel_width: 0.3,
            wheel_mounts: [
                Vec3::new(-0.9, -0.1, 1.3),
                Vec3::new(0.9, -0.1, 1.3),
                Vec3::new(-0.9, -0.1, -1.3),
                Vec3::new(0.9, -0.1, -1.3),
            ],
            suspension_rest: 0.40,
            mu_front: 1.10,
            mu_rear: 0.95,
            engine_force: 3_200.0,
            brake_force: 1_200.0,
            reverse_force: 1_500.0,
            top_speed: 21.0,
            max_steer_angle: 0.55,
            steer_accel_limit: 13.0,
            damage_scale: 1.0,
        }
    }

    /// Asserts `actual` is within `tolerance` (as a fraction) of `expected`.
    fn assert_within(actual: f32, expected: f32, tolerance: f32, what: &str) {
        let error = (actual - expected).abs() / expected;
        assert!(
            error <= tolerance,
            "{what}: {actual} is {:.1}% off {expected}, tolerance {:.1}%",
            error * 100.0,
            tolerance * 100.0,
        );
    }

    /// The regression guard for making the suspension/tire constants
    /// configurable: under the default tuning, the truck's derived rates must
    /// still land on the hand-tuned flat values they replaced.
    #[test]
    fn default_tuning_reproduces_the_hand_tuned_flat_rates() {
        let spec = truck();
        let tuning = VehicleTuning::default();
        assert_within(spec.spring_rate(&tuning), 14_000.0, 0.02, "spring rate");
        assert_within(spec.damping_rate(&tuning), 1_500.0, 0.02, "damping rate");
        assert_within(
            spec.max_spring_force(&tuning),
            6_000.0,
            0.02,
            "max spring force",
        );
        assert_within(
            spec.tire_lat_stiffness(&tuning),
            3_000.0,
            0.02,
            "tire lateral stiffness",
        );
    }

    /// The damper's cap is the one that may be loose, because a damper can
    /// only dissipate. If these two ever converge, hard landings will start
    /// bottoming out into the collider again.
    #[test]
    fn the_damper_cap_stays_far_above_the_spring_cap() {
        let spec = truck();
        let tuning = VehicleTuning::default();
        assert!(spec.max_damper_force(&tuning) > spec.max_spring_force(&tuning) * 1.5);
    }

    /// Every derived rate scales with mass, so a lighter class is softly
    /// sprung and a heavier one stiffly, at the same felt frequency.
    #[test]
    fn derived_rates_scale_with_mass() {
        let tuning = VehicleTuning::default();
        let heavy = truck();
        let light = VehicleSpec {
            mass: 230.0,
            ..truck()
        };
        let ratio = light.mass / heavy.mass;
        assert_within(
            light.spring_rate(&tuning),
            heavy.spring_rate(&tuning) * ratio,
            1e-4,
            "spring rate",
        );
        assert_within(
            light.tire_lat_stiffness(&tuning),
            heavy.tire_lat_stiffness(&tuning) * ratio,
            1e-4,
            "tire lateral stiffness",
        );
    }

    #[test]
    fn geometry_measures_the_full_spread_of_the_mounts() {
        let geometry = VehicleGeometry::from_mounts(&truck().wheel_mounts);
        assert_within(geometry.wheelbase, 2.6, 1e-5, "wheelbase");
        assert_within(geometry.track_width, 1.8, 1e-5, "track width");
    }

    /// Mount order is an authoring detail, not a contract — the measurement
    /// must not depend on it.
    #[test]
    fn geometry_is_independent_of_mount_order() {
        let mounts = truck().wheel_mounts;
        let shuffled = [mounts[3], mounts[0], mounts[2], mounts[1]];
        assert_eq!(
            VehicleGeometry::from_mounts(&mounts),
            VehicleGeometry::from_mounts(&shuffled),
        );
    }
}
