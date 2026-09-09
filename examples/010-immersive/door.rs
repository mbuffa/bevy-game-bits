//! `FuncDoor` runtime behavior: computes how far the door should travel from
//! its own collider bounds, then drives a Closed/Opening/Open/Closing state
//! machine in response to `Interacted` events.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_trenchbroom::physics::SceneCollidersReady;

use crate::classes::FuncDoor;
use crate::interact::Interacted;

#[derive(PartialEq, Clone, Copy)]
enum DoorPhase {
    Closed,
    Opening,
    Open,
    Closing,
}

#[derive(Component)]
pub struct DoorState {
    closed_pos: Vec3,
    open_pos: Vec3,
    phase: DoorPhase,
    wait_timer: Timer,
}

/// `insert_static_collider` only sets `RigidBody::Static` if none is
/// present yet (`insert_if_new`), and a static body never moves — so once
/// the door's collider exists we explicitly override it to `Kinematic` and
/// compute its travel distance from its own collider AABB, projected onto
/// `angle`'s direction (`2 * (hx|dx| + hy|dy| + hz|dz|)`, the standard
/// box support-function formula), minus `lip`.
///
/// This has to wait for `SceneCollidersReady` rather than `On<Add, FuncDoor>`
/// because the `ColliderAabb` doesn't exist until the physics backend has
/// run — see the bevy_trenchbroom manual's guidance on this trigger.
pub fn setup_doors(
    ready: On<SceneCollidersReady>,
    doors: Query<(&FuncDoor, &Transform, &ColliderAabb)>,
    mut commands: Commands,
) {
    for &entity in &ready.collider_entities {
        let Ok((door, transform, aabb)) = doors.get(entity) else {
            continue;
        };

        let angle = door.angle.to_radians();
        let dir = Vec3::new(angle.cos(), 0.0, -angle.sin());
        let half_extents = aabb.size() * 0.5;
        let width = 2.0
            * (half_extents.x * dir.x.abs()
                + half_extents.y * dir.y.abs()
                + half_extents.z * dir.z.abs());
        let travel = (width - door.lip).max(0.0);

        let closed_pos = transform.translation;
        let open_pos = closed_pos + dir * travel;

        commands.entity(entity).insert((
            RigidBody::Kinematic,
            DoorState {
                closed_pos,
                open_pos,
                phase: DoorPhase::Closed,
                wait_timer: Timer::from_seconds(door.wait.max(0.0), TimerMode::Once),
            },
        ));
    }
}

pub fn open_on_interact(trigger: On<Interacted>, mut doors: Query<&mut DoorState>) {
    if let Ok(mut state) = doors.get_mut(trigger.entity) {
        if state.phase == DoorPhase::Closed {
            state.phase = DoorPhase::Opening;
        }
    }
}

pub fn drive_doors(mut doors: Query<(&mut Transform, &mut DoorState, &FuncDoor)>, time: Res<Time>) {
    for (mut transform, mut state, door) in &mut doors {
        match state.phase {
            DoorPhase::Opening => {
                let target = state.open_pos;
                move_towards(
                    &mut transform.translation,
                    target,
                    door.speed * time.delta_secs(),
                );
                if transform.translation == target {
                    state.phase = DoorPhase::Open;
                    state.wait_timer.reset();
                }
            }
            DoorPhase::Open => {
                if door.wait >= 0.0 {
                    state.wait_timer.tick(time.delta());
                    if state.wait_timer.is_finished() {
                        state.phase = DoorPhase::Closing;
                    }
                }
            }
            DoorPhase::Closing => {
                let target = state.closed_pos;
                move_towards(
                    &mut transform.translation,
                    target,
                    door.speed * time.delta_secs(),
                );
                if transform.translation == target {
                    state.phase = DoorPhase::Closed;
                }
            }
            DoorPhase::Closed => {}
        }
    }
}

fn move_towards(current: &mut Vec3, target: Vec3, max_delta: f32) {
    let delta = target - *current;
    let dist = delta.length();
    if dist <= max_delta || dist < f32::EPSILON {
        *current = target;
    } else {
        *current += delta / dist * max_delta;
    }
}
