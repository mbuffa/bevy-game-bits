//! Wheels. A wheel is not a rigid body — it's a child entity of the chassis
//! carrying a [`Wheel`] component, whose mount point is where the suspension
//! raycast originates and whose transform is re-posed each frame to show the
//! suspension travelling, the fronts steering, and every wheel rolling.
//!
//! The physics lives in [`vehicle_controller`](super::vehicle_controller);
//! everything here is the state it writes and the visual sync that reads it.

use std::f32::consts::TAU;

use bevy::prelude::*;

use super::control::{ackermann_angle, Vehicle};
use super::spec::VehicleSpec;

/// One wheel of a vehicle, on a child entity of the chassis.
///
/// The first five fields are configuration, set once at spawn (see
/// [`wheel_bundle`]). The rest are per-step state the controller writes and
/// other systems read — visuals, skid marks, engine audio.
#[derive(Component, Debug)]
pub struct Wheel {
    /// Where this wheel is bolted to the chassis, in chassis-local space.
    /// The suspension raycast starts here and casts along chassis-down.
    pub mount_local: Vec3,
    /// Copied from [`VehicleSpec::wheel_radius`] at spawn — the raycast
    /// length and the visual spin rate both need it every tick, which is
    /// cheaper read from here than refetched off the chassis per wheel.
    pub radius: f32,
    /// This wheel's rest orientation. [`update_wheel_visuals`] composes steer
    /// and spin *on top of* it rather than replacing it, so a model with
    /// camber or toe baked into its wheel nodes carries that through
    /// automatically.
    pub rest_rotation: Quat,
    /// Whether this wheel turns with [`Vehicle::steer`] (Ackermann-split per
    /// side). Typically the front pair.
    pub steering: bool,
    /// Whether this wheel receives engine force. All four means AWD, which
    /// makes the launch engine-limited instead of rear-traction-limited; the
    /// friction circle still makes throttle cost cornering grip, just spread
    /// over both axles.
    pub driven: bool,

    /// Current suspension travel: 0 = fully compressed (mount-to-wheel
    /// distance zero), [`VehicleSpec::suspension_rest`] = fully extended or
    /// airborne. Purely cosmetic — the physics uses each step's raycast
    /// result directly.
    pub travel: f32,
    /// This wheel's ground-plane forward speed from the last physics step.
    /// Per-wheel, so inner and outer wheels visibly spin at different rates
    /// through a turn.
    pub ground_speed: f32,
    /// Accumulated visual roll angle around the axle (radians).
    pub spin: f32,
    /// Magnitude of this wheel's contact-patch slip against the ground (m/s;
    /// 0 when airborne): lateral slip for a rolling tire, the full planar
    /// velocity for a handbrake-locked one, since a locked patch drags in
    /// every direction. A wheel with high `slip_speed` is scrubbing rubber
    /// onto the floor — which is exactly what
    /// [`skidmarks`](super::skidmarks) keys off.
    pub slip_speed: f32,
    /// This step's suspension-ray hit point in world space, or `None` when
    /// the wheel found nothing under it.
    pub contact_world: Option<Vec3>,
}

impl Wheel {
    /// A wheel at rest: fully extended, stationary, airborne. `rest` is the
    /// wheel's pose in chassis-local space — its translation is the mount
    /// point, its rotation the rest orientation.
    pub fn new(spec: &VehicleSpec, rest: Transform, steering: bool, driven: bool) -> Self {
        Self {
            mount_local: rest.translation,
            radius: spec.wheel_radius,
            rest_rotation: rest.rotation,
            steering,
            driven,
            travel: spec.suspension_rest,
            ground_speed: 0.0,
            spin: 0.0,
            slip_speed: 0.0,
            contact_world: None,
        }
    }

    /// Whether this wheel found ground under it on the last physics step.
    pub fn grounded(&self) -> bool {
        self.contact_world.is_some()
    }
}

/// One wheel touching down hard enough to be worth reacting to — a dust puff,
/// a thud, a small camera shake.
///
/// Purely informational: the suspension has already absorbed the landing by
/// the time this is written, and nothing in this module reads it back. Written
/// in `FixedUpdate` by the controller, so read it in `FixedUpdate` too if you
/// need every one, or in `Update` if you're only driving cosmetics.
#[derive(Message, Debug)]
pub struct WheelLanding {
    /// Contact point in world space.
    pub position: Vec3,
    /// Closing speed (m/s) along the chassis's own up axis at touchdown —
    /// what to scale the reaction's intensity from.
    pub speed: f32,
}

/// One wheel child of a chassis: the [`Wheel`] and its rest [`Transform`].
///
/// `rest` is the wheel's pose in chassis-local space — pass the node
/// transform straight from a loaded model, or
/// `Transform::from_translation(mount)` for a procedural one. Add your own
/// `Mesh3d`/`MeshMaterial3d` alongside it; this module has no opinion on what
/// a wheel looks like:
///
/// ```ignore
/// chassis.spawn((
///     wheel_bundle(&spec, Transform::from_translation(mount), steering, true),
///     Mesh3d(wheel_mesh.clone()),
///     MeshMaterial3d(rubber.clone()),
/// ));
/// ```
pub fn wheel_bundle(
    spec: &VehicleSpec,
    rest: Transform,
    steering: bool,
    driven: bool,
) -> impl Bundle {
    (Wheel::new(spec, rest, steering, driven), rest)
}

/// Poses every wheel mesh: slides it down its mount by the suspension travel,
/// yaws steering wheels to their Ackermann angle, and rolls each one by its
/// own ground speed.
///
/// Registered in `Update`, not `FixedUpdate`, so wheels don't visibly stutter
/// between physics steps. It only writes `Transform`, so running it at render
/// rate costs nothing the physics cares about.
pub fn update_wheel_visuals(
    time: Res<Time>,
    vehicles: Query<&Vehicle>,
    mut wheels: Query<(&mut Wheel, &ChildOf, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (mut wheel, child_of, mut transform) in &mut wheels {
        let steer_angle = if wheel.steering {
            vehicles
                .get(child_of.parent())
                .map(|vehicle| {
                    ackermann_angle(
                        vehicle.steer,
                        wheel.mount_local.x,
                        vehicle.wheelbase,
                        vehicle.track_width,
                    )
                })
                .unwrap_or(0.0)
        } else {
            0.0
        };
        wheel.spin = (wheel.spin + wheel.ground_speed / wheel.radius * dt) % TAU;
        transform.translation = wheel.mount_local - Vec3::Y * wheel.travel;
        transform.rotation = Quat::from_rotation_y(steer_angle)
            * Quat::from_rotation_x(wheel.spin)
            * wheel.rest_rotation;
    }
}
