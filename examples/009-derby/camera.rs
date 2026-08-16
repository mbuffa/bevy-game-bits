//! A perspective third-person chase camera: sits behind and above the car
//! and eases toward that pose every frame. Its target direction comes from
//! the chassis's *yaw only* — if the car flips or tips (free rotation, no
//! upright lock), the camera doesn't tumble with it.

use bevy::prelude::*;

use crate::config::*;
use crate::game::CameraTarget;
use bevy_game_bits::vehicle::Vehicle;

#[derive(Component)]
pub struct FollowCamera;

pub fn setup_camera(mut commands: Commands) {
    // Placeholder pose before any car exists — `follow_camera` takes over
    // the instant a `CameraTarget` spawns, so the exact height only matters
    // for the single pre-spawn frame; the default class's is a fine guess.
    let spawn_height = CarClass::default().spec().spawn_height;
    commands.spawn((
        Camera3d::default(),
        FollowCamera,
        Transform::from_xyz(0.0, spawn_height + CAMERA_UP, -CAMERA_BACK)
            .looking_at(Vec3::Y * spawn_height, Vec3::Y),
        // Per-view ambient light, attached to the camera rather than a
        // global resource.
        AmbientLight {
            brightness: 300.0,
            ..default()
        },
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, -0.6, -1.0)),
    ));
}

/// Chases whichever chassis `game::CameraTarget` marks — the player,
/// unless they've wrecked mid-match and `game::hand_off_camera` moved it to
/// a survivor. Targets a point `CAMERA_BACK` behind (along the chassis's
/// flattened forward) and `CAMERA_UP` above it, and eases the camera's
/// position toward that point each frame. `flat_forward` is persisted in a
/// `Local` so that a chassis forward pointing straight up or down
/// (mid-flip) — where the flattened direction collapses toward zero —
/// doesn't snap the camera to a degenerate heading; it just keeps the last
/// good one until the car rights itself.
pub fn follow_camera(
    time: Res<Time>,
    // Separate from `time` above: `relative_speed()` is specific to the
    // `Virtual` clock and isn't exposed on the generic `Time` (which in
    // `Update` already mirrors it for `delta_secs()`, so the easing below is
    // unaffected either way).
    virtual_time: Res<Time<Virtual>>,
    // `Option<Single<..>>`: the car model loads asynchronously
    // (`model.rs`), so there's no `CameraTarget` yet for the first several
    // frames — a bare `Single` would panic on that count mismatch.
    car: Option<Single<&Transform, (With<CameraTarget>, Without<FollowCamera>)>>,
    camera: Single<&mut Transform, (With<FollowCamera>, Without<Vehicle>)>,
    mut flat_forward: Local<Vec3>,
) {
    let Some(car) = car else { return };
    let car_transform = car.into_inner();
    let mut camera_transform = camera.into_inner();

    if *flat_forward == Vec3::ZERO {
        *flat_forward = Vec3::Z;
    }
    // Matches vehicle.rs's convention: the chassis's local +Z is "forward".
    let forward_world = car_transform.rotation * Vec3::Z;
    let flat = Vec3::new(forward_world.x, 0.0, forward_world.z);
    if flat.length_squared() > 0.01 {
        *flat_forward = flat.normalize();
    }

    // Pulls the camera in during `juice`'s slow-motion — reading the time
    // scale directly rather than a dedicated resource, so this file doesn't
    // need to know slow-motion's trigger logic, only that it exists.
    let slowmo_t = ((1.0 - virtual_time.relative_speed()) / (1.0 - SLOWMO_SCALE)).clamp(0.0, 1.0);
    let camera_back = CAMERA_BACK * (1.0 - slowmo_t * (1.0 - SLOWMO_CAMERA_PULL));

    let look_target = car_transform.translation + Vec3::Y * 0.5;
    let target_pos =
        car_transform.translation - *flat_forward * camera_back + Vec3::Y * CAMERA_UP;

    let ease = (CAMERA_LERP_RATE * time.delta_secs()).clamp(0.0, 1.0);
    camera_transform.translation = camera_transform.translation.lerp(target_pos, ease);
    camera_transform.look_at(look_target, Vec3::Y);
}
