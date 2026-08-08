//! A semi-simulation raycast-suspension vehicle for Avian 3D.
//!
//! One Dynamic chassis rigid body carries four non-physical wheels that are
//! raycasts plus a spring/damper and a slip-based tire model. It's the
//! arcade-sim middle ground: enough load transfer, understeer, oversteer and
//! handbrake behaviour to feel like a car, without wheel angular dynamics or
//! a tire curve to fight.
//!
//! # Getting a car on the ground
//!
//! ```ignore
//! use bevy_game_bits::vehicle::prelude::*;
//!
//! app.add_plugins(PhysicsPlugins::default())
//!     .add_plugins(VehiclePlugin::default())
//!     .add_systems(Update, drive_from_keyboard.in_set(VehicleSet::Input));
//!
//! // ...then, in a spawn system:
//! let geometry = VehicleGeometry::from_mounts(&spec.wheel_mounts);
//! commands
//!     .spawn((
//!         chassis_bundle(spec, &tuning, geometry),
//!         KeyboardDriver::default(),
//!         Mesh3d(body), MeshMaterial3d(paint),
//!         Transform::from_xyz(0.0, spec.spawn_height, 0.0),
//!     ))
//!     .with_children(|chassis| {
//!         for mount in spec.wheel_mounts {
//!             chassis.spawn((
//!                 wheel_bundle(&spec, Transform::from_translation(mount), mount.z > 0.0, true),
//!                 Mesh3d(wheel.clone()), MeshMaterial3d(rubber.clone()),
//!             ));
//!         }
//!     });
//! ```
//!
//! # The pieces
//!
//! - [`spec`] — [`VehicleSpec`], one vehicle's own numbers, carried as a
//!   component; [`VehicleTuning`], how vehicles feel in this app.
//! - [`control`] — [`DriveInput`] (intent), [`Vehicle`] (smoothed actuation),
//!   [`chassis_bundle`], and [`vehicle_controller`], the model itself.
//! - [`wheel`] — [`Wheel`], [`wheel_bundle`], and the visual sync.
//! - [`input`] — an optional keyboard driver. Skip it and write
//!   [`DriveInput`] yourself from a gamepad or an AI.
//! - [`impact`] — an optional collision-damage pipeline that turns solver
//!   impulses into located, per-vehicle Δv, leaving what *breaks* to you.
//! - [`skidmarks`] — an optional decal system driven off wheel slip.
//!
//! Only [`VehiclePlugin`] is required. [`VehicleImpactPlugin`] and
//! [`SkidMarkPlugin`] are independent add-ons.
//!
//! # Scheduling
//!
//! [`vehicle_controller`] runs in `FixedUpdate`, which Bevy runs before
//! Avian's step in `FixedPostUpdate` — so the forces it accumulates are the
//! ones integrated that tick. Order your own systems against [`VehicleSet`]
//! rather than against the system functions directly.
//!
//! One trap worth stating plainly: whatever writes [`DriveInput`] must not
//! also fetch the chassis's `LinearVelocity`/`AngularVelocity`. The
//! controller's [`Forces`](avian3d::prelude::Forces) query item statically
//! declares mutable access to both, so an AI system that reads velocity off
//! the same entities will conflict at runtime. Read speed from
//! `Transform` deltas, or from a component you mirror it into.

pub mod control;
pub mod impact;
pub mod input;
pub mod skidmarks;
pub mod spec;
pub mod wheel;

use bevy::prelude::*;

pub use control::{
    ackermann_angle, chassis_bundle, vehicle_controller, DriveInput, DrivePower, Vehicle,
};
pub use impact::{
    detach_as_debris, detect_vehicle_impacts, side_index, ChassisMotion, DebrisConfig, ImpactSide,
    ImpactTuning, ImpactZone, SoftProp, VehicleImpact, VehicleImpactPlugin,
};
pub use input::{drive_from_keyboard, KeyboardDriver};
pub use skidmarks::{SkidMarkConfig, SkidMarkPlugin, SkidMarks};
pub use spec::{VehicleGeometry, VehicleSpec, VehicleTuning, GRAVITY};
pub use wheel::{update_wheel_visuals, wheel_bundle, Wheel, WheelLanding};

/// Everything you need to build and drive a vehicle, in one import.
pub mod prelude {
    pub use super::{
        ackermann_angle, chassis_bundle, detach_as_debris, drive_from_keyboard, side_index,
        wheel_bundle, ChassisMotion, DebrisConfig, DriveInput, DrivePower, ImpactSide, ImpactTuning,
        ImpactZone, KeyboardDriver, SkidMarkConfig, SkidMarkPlugin, SoftProp, Vehicle,
        VehicleGeometry, VehicleImpact, VehicleImpactPlugin, VehiclePlugin, VehicleSet, VehicleSpec,
        VehicleTuning, Wheel, WheelLanding,
    };
}

/// Ordering handles for the vehicle systems. Order against these rather than
/// against the system functions, so a game keeps working if the internals are
/// re-split.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VehicleSet {
    /// `FixedUpdate`, before [`Control`](Self::Control). Nothing is registered
    /// here by default — it's where you put whatever writes [`DriveInput`]:
    /// [`drive_from_keyboard`], a gamepad reader, an AI.
    Input,
    /// `FixedUpdate`. [`vehicle_controller`] — suspension, tires, steering,
    /// drag, flip rescue.
    Control,
    /// `FixedUpdate`, after [`Control`](Self::Control).
    /// [`detect_vehicle_impacts`], when [`VehicleImpactPlugin`] is added.
    /// Read [`VehicleImpact`] after this.
    Impact,
    /// `Update`. [`update_wheel_visuals`], and skid marks when
    /// [`SkidMarkPlugin`] is added.
    Visuals,
}

/// Registers the driving model. Required; everything else in this module is
/// optional on top of it.
///
/// ```ignore
/// app.add_plugins(VehiclePlugin {
///     tuning: VehicleTuning { suspension_freq_hz: 1.6, ..default() },
/// });
/// ```
#[derive(Default)]
pub struct VehiclePlugin {
    /// How vehicles feel in this app. [`VehicleTuning::default`] is a
    /// complete, playable arcade-sim setup.
    pub tuning: VehicleTuning,
}

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.tuning)
            .add_message::<WheelLanding>()
            .configure_sets(FixedUpdate, (VehicleSet::Input, VehicleSet::Control).chain())
            .add_systems(FixedUpdate, vehicle_controller.in_set(VehicleSet::Control))
            .add_systems(Update, update_wheel_visuals.in_set(VehicleSet::Visuals));
    }
}
