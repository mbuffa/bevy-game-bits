//! The `bevy_game_bits::vehicle` driving model, on its own.
//!
//! One procedurally-meshed car on a flat plane with a few ramps: no glTF
//! rigs, no car classes, no match state, no AI, no damage. It exists to show
//! what the library actually needs — a `VehicleSpec`, a `chassis_bundle`, four
//! `wheel_bundle` children — and to be the thing you copy when starting a new
//! project. `examples/009-derby` is the same model with a whole game on top.
//!
//! Controls: W/S accelerate/brake-reverse, A/D steer, Space handbrake,
//! R respawn.

use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::prelude::*;

use bevy_game_bits::vehicle::prelude::*;

/// The one car. Not a table of classes — a single `VehicleSpec` is all the
/// library needs, and a game with several just keeps several of these.
///
/// These numbers are a light, grippy arcade car: 260 kg, 1.35/1.20 μ, and a
/// low center of mass, which is the biggest thing keeping it off its roof.
const CAR: VehicleSpec = VehicleSpec {
    chassis_size: Vec3::new(1.6, 0.55, 3.1),
    // A little smaller than the visual box: the ray suspension carries the car
    // in normal driving, so the collider is only for walls, landings and
    // car-vs-car.
    collider_size: Vec3::new(1.5, 0.46, 2.9),
    mass: 260.0,
    com_offset: Vec3::new(0.0, -0.22, 0.0),
    spawn_height: 0.8,
    wheel_radius: 0.32,
    wheel_width: 0.28,
    // Positive Z is forward, so the first two are the steering pair.
    wheel_mounts: [
        Vec3::new(-0.78, -0.09, 1.15),
        Vec3::new(0.78, -0.09, 1.15),
        Vec3::new(-0.78, -0.09, -1.15),
        Vec3::new(0.78, -0.09, -1.15),
    ],
    suspension_rest: 0.34,
    // Front grippier than rear, so the car understeers at the limit rather
    // than snapping into a spin.
    mu_front: 1.35,
    mu_rear: 1.20,
    engine_force: 2_800.0,
    brake_force: 1_000.0,
    reverse_force: 1_200.0,
    top_speed: 24.0,
    max_steer_angle: 0.60,
    steer_accel_limit: 15.0,
    // Nothing in this example reads damage, but the field is part of the spec.
    damage_scale: 1.0,
};

const GROUND_SIZE: f32 = 120.0;
/// Where `R` puts the car back.
const SPAWN: Vec3 = Vec3::new(0.0, CAR.spawn_height, -20.0);

/// Marks the chase camera.
#[derive(Component)]
struct ChaseCamera;

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Vehicle".into(), ..default() }),
            ..default()
        }))
        .add_plugins(PhysicsPlugins::default())
        // The whole integration: the driving model, plus skid marks because
        // they cost one line and make the handling legible.
        .add_plugins(VehiclePlugin::default())
        .add_plugins(SkidMarkPlugin::default())
        .add_systems(Startup, (setup_scene, spawn_car, setup_hud))
        // Input belongs in `FixedUpdate` alongside the controller it feeds —
        // `VehicleSet::Input` is already ordered before `VehicleSet::Control`.
        .add_systems(FixedUpdate, drive_from_keyboard.in_set(VehicleSet::Input))
        .add_systems(Update, (respawn, follow_camera, update_hud))
        .run();
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ground = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.23, 0.25),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Name::new("Ground"),
        RigidBody::Static,
        Collider::cuboid(GROUND_SIZE, 1.0, GROUND_SIZE),
        Friction::new(0.9),
        Restitution::new(0.05),
        Mesh3d(meshes.add(Cuboid::new(GROUND_SIZE, 1.0, GROUND_SIZE))),
        MeshMaterial3d(ground.clone()),
        // Top face at y = 0.
        Transform::from_xyz(0.0, -0.5, 0.0),
    ));

    // Four ramps facing the middle, so a lap round the plane always has
    // something to launch off — the landing damper is the most interesting
    // thing in the model to feel.
    let ramp_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.34, 0.22),
        perceptual_roughness: 0.9,
        ..default()
    });
    let ramp_mesh = meshes.add(Cuboid::new(5.0, 1.2, 7.0));
    for (i, angle) in (0..4).map(|i| (i, i as f32 * std::f32::consts::FRAC_PI_2)) {
        let (sin, cos) = angle.sin_cos();
        commands.spawn((
            Name::new(format!("Ramp {i}")),
            RigidBody::Static,
            Collider::cuboid(5.0, 1.2, 7.0),
            Friction::new(0.9),
            Mesh3d(ramp_mesh.clone()),
            MeshMaterial3d(ramp_material.clone()),
            // Tipped 10° about X and sunk so the low edge meets the ground —
            // a wedge without needing a wedge mesh.
            Transform::from_xyz(22.0 * sin, -0.35, 22.0 * cos)
                .with_rotation(Quat::from_rotation_y(angle) * Quat::from_rotation_x(0.17)),
        ));
    }

    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, shadows_enabled: true, ..default() },
        Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        ChaseCamera,
        Camera3d::default(),
        Transform::from_xyz(0.0, 6.0, -32.0).looking_at(SPAWN, Vec3::Y),
        // Ambient light is per-view in Bevy 0.18, so it rides on the camera
        // rather than being a global resource.
        AmbientLight { brightness: 220.0, ..default() },
    ));
}

/// The whole of building a driveable car.
fn spawn_car(
    mut commands: Commands,
    tuning: Res<VehicleTuning>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let paint = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.25, 0.2),
        perceptual_roughness: 0.4,
        metallic: 0.3,
        ..default()
    });
    let rubber = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        perceptual_roughness: 0.95,
        ..default()
    });
    let wheel_mesh = meshes.add(Cylinder::new(CAR.wheel_radius, CAR.wheel_width));

    commands
        .spawn((
            Name::new("Car"),
            // Rigid body, collider, mass properties, `Vehicle`, `DriveInput`,
            // `DrivePower`.
            chassis_bundle(CAR, &tuning, VehicleGeometry::from_mounts(&CAR.wheel_mounts)),
            // What makes W/A/S/D reach this car's `DriveInput`. Drop it and
            // write `DriveInput` from an AI instead — the controller can't
            // tell the difference.
            KeyboardDriver::default(),
            Mesh3d(meshes.add(Cuboid::new(
                CAR.chassis_size.x,
                CAR.chassis_size.y,
                CAR.chassis_size.z,
            ))),
            MeshMaterial3d(paint),
            Transform::from_translation(SPAWN),
        ))
        .with_children(|chassis| {
            for mount in CAR.wheel_mounts {
                chassis.spawn((
                    Name::new("Wheel"),
                    wheel_bundle(
                        &CAR,
                        // Bevy's cylinder stands on Y; rolling it onto X makes
                        // it an axle. `update_wheel_visuals` composes steer and
                        // spin on top of this rest pose rather than replacing
                        // it, so the alignment survives.
                        Transform::from_translation(mount)
                            .with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
                        // Front pair steers; all four are driven (AWD).
                        mount.z > 0.0,
                        true,
                    ),
                    Mesh3d(wheel_mesh.clone()),
                    MeshMaterial3d(rubber.clone()),
                ));
            }
        });
}

/// `R` drops the car back on the spawn point, upright and stationary — the
/// flip rescue handles ordinary rollovers, this is for wedging it somewhere
/// silly.
fn respawn(
    keys: Res<ButtonInput<KeyCode>>,
    mut car: Single<(&mut Transform, &mut LinearVelocity, &mut AngularVelocity), With<Vehicle>>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    let (transform, linear, angular) = &mut *car;
    **transform = Transform::from_translation(SPAWN);
    linear.0 = Vec3::ZERO;
    angular.0 = Vec3::ZERO;
}

/// Yaw-only chase camera: sits behind the car's heading, eased, and always
/// looking at it. Deliberately doesn't inherit pitch or roll — a camera that
/// copies the chassis through a landing is unreadable.
fn follow_camera(
    time: Res<Time>,
    car: Single<&Transform, (With<Vehicle>, Without<ChaseCamera>)>,
    mut camera: Single<&mut Transform, With<ChaseCamera>>,
) {
    let (yaw, ..) = car.rotation.to_euler(EulerRot::YXZ);
    let behind = Quat::from_rotation_y(yaw) * Vec3::new(0.0, 3.2, -8.5);
    let wanted = car.translation + behind;
    // Frame-rate independent exponential ease.
    let t = 1.0 - (-6.0 * time.delta_secs()).exp();
    camera.translation = camera.translation.lerp(wanted, t);
    camera.look_at(car.translation + Vec3::Y * 0.8, Vec3::Y);
}

/// Marks the speed readout.
#[derive(Component)]
struct SpeedText;

fn setup_hud(mut commands: Commands) {
    commands.spawn((
        Text::new("W/S drive  ·  A/D steer  ·  Space handbrake  ·  R respawn"),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(12),
            ..default()
        },
    ));
    commands.spawn((
        SpeedText,
        Text::new("0 km/h"),
        TextFont { font_size: 28.0, ..default() },
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            left: px(12),
            ..default()
        },
    ));
}

fn update_hud(
    car: Single<(&LinearVelocity, &Vehicle)>,
    mut text: Single<&mut Text, With<SpeedText>>,
) {
    let (velocity, vehicle) = *car;
    let kmh = velocity.0.length() * 3.6;
    let handbrake = if vehicle.handbrake { "  HANDBRAKE" } else { "" };
    text.0 = format!("{kmh:.0} km/h{handbrake}");
}
