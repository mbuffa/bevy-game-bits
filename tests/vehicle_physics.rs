//! Headless behaviour tests for `bevy_game_bits::vehicle`.
//!
//! The unit tests inside the module cover the pure helpers; these drive the
//! actual controller against a real Avian world, because the things most worth
//! protecting — that a car settles onto its springs at the right ride height,
//! that drag really does wall off at `top_speed`, that steering yaws the way
//! the sign says — only exist once forces are being integrated.
//!
//! Time is stepped manually (`TimeUpdateStrategy::ManualDuration` at the fixed
//! timestep), so every run is deterministic and takes milliseconds rather than
//! wall-clock seconds.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use bevy_game_bits::vehicle::prelude::*;

const HZ: f64 = 64.0;
const GROUND_SIZE: f32 = 400.0;

/// A 350 kg arcade truck — the spec `examples/009-derby` ships, so the ride
/// height asserted below is the one that prototype is tuned around.
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

/// A world with flat ground, the vehicle plugin, and one car at `height`.
fn world_with_car(spec: VehicleSpec, height: f32) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(TransformPlugin)
        // Avian needs both even with nothing to render: `Assets<Mesh>` for
        // mesh-derived colliders, and `SceneSpawner` for its
        // collider-constructor hierarchies. Neither is in `MinimalPlugins`.
        .add_plugins(AssetPlugin::default())
        .init_asset::<Mesh>()
        .add_plugins(bevy::scene::ScenePlugin)
        .add_plugins(PhysicsPlugins::default())
        .add_plugins(VehiclePlugin::default())
        .insert_resource(Time::<Fixed>::from_hz(HZ))
        // One `app.update()` == exactly one fixed step, so every test below
        // is deterministic and finishes in milliseconds.
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / HZ,
        )));
    // `App::update()` does not call these the way `App::run()` would, and
    // several of Avian's resources are registered in plugin `finish` hooks —
    // without this the first update panics on a missing resource.
    app.finish();
    app.cleanup();

    // Top face at y = 0.
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(GROUND_SIZE, 1.0, GROUND_SIZE),
        Friction::new(0.9),
        Transform::from_xyz(0.0, -0.5, 0.0),
    ));

    let tuning = *app.world().resource::<VehicleTuning>();
    let geometry = VehicleGeometry::from_mounts(&spec.wheel_mounts);
    let car = app
        .world_mut()
        .spawn((
            chassis_bundle(spec, &tuning, geometry),
            Transform::from_xyz(0.0, height, 0.0),
        ))
        .id();
    for mount in spec.wheel_mounts {
        app.world_mut().spawn((
            wheel_bundle(&spec, Transform::from_translation(mount), mount.z > 0.0, true),
            ChildOf(car),
        ));
    }

    (app, car)
}

fn step(app: &mut App, seconds: f32) {
    for _ in 0..(seconds as f64 * HZ) as usize {
        app.update();
    }
}

fn set_input(app: &mut App, car: Entity, drive: f32, steer: f32, handbrake: bool) {
    let mut input = app.world_mut().get_mut::<DriveInput>(car).unwrap();
    input.drive = drive;
    input.steer = steer;
    input.handbrake = handbrake;
}

fn position(app: &App, car: Entity) -> Vec3 {
    app.world().get::<Transform>(car).unwrap().translation
}

fn velocity(app: &App, car: Entity) -> Vec3 {
    app.world().get::<LinearVelocity>(car).unwrap().0
}

/// Static ride height, from first principles: at rest each corner carries
/// `m/4 · g`, and the spring rate is `m/4 · (2πf)²`, so the compression is
/// `g / (2πf)²` — independent of mass, which is the whole point of deriving
/// the rate from a frequency. The chassis then sits at
/// `(rest - compression) + wheel_radius - mount_y`.
fn expected_ride_height(spec: &VehicleSpec, tuning: &VehicleTuning) -> f32 {
    let compression = 9.81 / (std::f32::consts::TAU * tuning.suspension_freq_hz).powi(2);
    let travel = spec.suspension_rest - compression;
    travel + spec.wheel_radius - spec.wheel_mounts[0].y
}

/// The most basic thing a raycast car must do, and the one most likely to
/// break silently in a refactor: sit still at the right height, on its
/// springs, without sinking, jittering or slowly climbing.
#[test]
fn a_car_settles_onto_its_springs_at_the_derived_ride_height() {
    let spec = truck();
    let (mut app, car) = world_with_car(spec, spec.spawn_height);
    let expected = expected_ride_height(&spec, &VehicleTuning::default());

    step(&mut app, 3.0);

    let height = position(&app, car).y;
    assert!(
        (height - expected).abs() < 0.01,
        "settled at {height:.4} m, expected {expected:.4} m",
    );
    // Genuinely at rest, not still oscillating through the right value.
    assert!(velocity(&app, car).length() < 0.05, "still moving: {:?}", velocity(&app, car));
}

/// A wheel with nothing under it must contribute nothing, so the chassis
/// free-falls — the airborne branch is easy to get wrong in a way that
/// silently holds the car up.
#[test]
fn a_car_dropped_from_height_falls_then_settles() {
    let spec = truck();
    let (mut app, car) = world_with_car(spec, 8.0);

    step(&mut app, 0.25);
    // Free fall: v = g·t, and nothing should be damping it yet.
    let falling = velocity(&app, car).y;
    assert!(falling < -2.0, "should be free-falling, vy = {falling:.2}");

    step(&mut app, 5.0);
    let expected = expected_ride_height(&spec, &VehicleTuning::default());
    let height = position(&app, car).y;
    assert!(
        (height - expected).abs() < 0.02,
        "settled at {height:.4} m after a drop, expected {expected:.4} m",
    );
}

/// Drag is expressed as a target speed, so the car must actually converge on
/// it — close from below, and never meaningfully past it.
#[test]
fn full_throttle_converges_on_top_speed_without_exceeding_it() {
    let spec = truck();
    let (mut app, car) = world_with_car(spec, spec.spawn_height);
    step(&mut app, 1.0);
    set_input(&mut app, car, 1.0, 0.0, false);

    step(&mut app, 4.0);
    let mid = velocity(&app, car).length();
    assert!(mid > 8.0, "should be well underway after 4 s, got {mid:.1} m/s");

    step(&mut app, 20.0);
    let terminal = velocity(&app, car).length();
    assert!(
        terminal > spec.top_speed * 0.95 && terminal <= spec.top_speed * 1.02,
        "terminal speed {terminal:.2} m/s should sit at top_speed {:.1}",
        spec.top_speed,
    );
}

/// Positive steer turns toward +X — the sign convention `DriveInput::steer`
/// and `ackermann_angle` both document, and which a game's AI depends on.
#[test]
fn positive_steer_turns_the_car_toward_positive_x() {
    let spec = truck();
    let (mut app, car) = world_with_car(spec, spec.spawn_height);
    step(&mut app, 1.0);

    set_input(&mut app, car, 1.0, 1.0, false);
    step(&mut app, 3.0);

    let position = position(&app, car);
    assert!(position.x > 0.5, "should have arced toward +X, at x = {:.2}", position.x);
    let (yaw, ..) = app
        .world()
        .get::<Transform>(car)
        .unwrap()
        .rotation
        .to_euler(EulerRot::YXZ);
    assert!(yaw < -0.1, "yaw should have turned toward +X, got {yaw:.3} rad");
}

/// Braking has to actually haul the car down, not merely stop driving it —
/// so it's measured against a coasting control run from the same speed.
///
/// Note what this does *not* assert: that the car ends up stopped. Holding
/// the pedal past a stop is reverse, by design (`reverse_threshold`), so a
/// long enough brake becomes a reverse. That's the model working.
#[test]
fn braking_hauls_a_car_down_far_harder_than_coasting() {
    let spec = truck();

    let run = |brake: bool| {
        let (mut app, car) = world_with_car(spec, spec.spawn_height);
        step(&mut app, 1.0);
        set_input(&mut app, car, 1.0, 0.0, false);
        step(&mut app, 5.0);
        let cruising = velocity(&app, car).z;
        assert!(cruising > 5.0, "expected to be rolling forward, got {cruising:.2} m/s");

        set_input(&mut app, car, if brake { -1.0 } else { 0.0 }, 0.0, false);
        step(&mut app, 1.5);
        (cruising, velocity(&app, car).z)
    };

    let (cruising, braked) = run(true);
    let (_, coasted) = run(false);

    assert!(braked < coasted, "braking {braked:.2} should beat coasting {coasted:.2} m/s");
    assert!(
        braked < cruising * 0.25,
        "1.5 s of brakes should scrub most of {cruising:.2} m/s, left {braked:.2}",
    );
    // Coasting is rolling resistance and drag only, so it must still be
    // clearly rolling forward — otherwise "braking beat coasting" proves
    // nothing.
    assert!(coasted > cruising * 0.5, "coasting fell off too fast: {coasted:.2} m/s");
}

/// The handbrake's job is to break rear grip. Held through a turn at speed,
/// the rear wheels must report far more contact-patch slip than the fronts —
/// that difference is what drifting *is*, and what skid marks key off.
#[test]
fn the_handbrake_makes_the_rear_wheels_slip_more_than_the_front() {
    let spec = truck();
    let (mut app, car) = world_with_car(spec, spec.spawn_height);
    step(&mut app, 1.0);
    set_input(&mut app, car, 1.0, 0.0, false);
    step(&mut app, 5.0);

    set_input(&mut app, car, 1.0, 1.0, true);
    step(&mut app, 1.0);

    let mut front = 0.0_f32;
    let mut rear = 0.0_f32;
    for wheel in app.world_mut().query::<&Wheel>().iter(app.world()) {
        if wheel.steering {
            front = front.max(wheel.slip_speed);
        } else {
            rear = rear.max(wheel.slip_speed);
        }
    }
    assert!(rear > front, "rear slip {rear:.2} should exceed front {front:.2}");
    assert!(rear > 1.5, "rear should be properly sliding, got {rear:.2} m/s");
}

/// A hard landing has to announce itself, and an ordinary settle must not —
/// otherwise every car puffs dust just spawning in.
#[test]
fn a_hard_landing_emits_a_wheel_landing_but_a_gentle_settle_does_not() {
    let spec = truck();

    let (mut app, _) = world_with_car(spec, spec.spawn_height);
    step(&mut app, 2.0);
    let quiet = app.world().resource::<Messages<WheelLanding>>().iter_current_update_messages().count();
    assert_eq!(quiet, 0, "settling onto the grid should be silent");

    let (mut app, _) = world_with_car(spec, 6.0);
    let mut hard = Vec::new();
    for _ in 0..(2.0 * HZ) as usize {
        app.update();
        hard.extend(
            app.world()
                .resource::<Messages<WheelLanding>>()
                .iter_current_update_messages()
                .map(|landing| landing.speed),
        );
    }
    assert_eq!(hard.len(), 4, "all four wheels should report the touchdown");
    let tuning = VehicleTuning::default();
    assert!(
        hard.iter().all(|speed| *speed >= tuning.landing_thump_min_speed),
        "every reported landing must clear the threshold: {hard:?}",
    );
}
