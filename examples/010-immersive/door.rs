//! Two doors:
//!
//! * [`FuncDoor`] — the Quake `func_door` slider. `setup_doors` computes how
//!   far it should travel from its own collider bounds, then `drive_doors`
//!   runs a Closed/Opening/Open/Closing machine that auto-closes after `wait`.
//! * [`PropDoor`] — a hinged "regular" door (Phase 10). `spawn_swing_doors`
//!   builds the leaf + handle + lock plate as children and `swing_doors`
//!   rotates the whole entity about its hinge. **E toggles** it (no auto-close
//!   — a corridor door that shuts behind you is a nuisance), and it refuses
//!   while `locked`.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_trenchbroom::physics::SceneCollidersReady;

use crate::classes::{FuncDoor, Interactable, PropDoor};
use crate::config;
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

// --- PropDoor: the hinged door ------------------------------------------------

#[derive(PartialEq, Clone, Copy, Debug)]
enum SwingPhase {
    Closed,
    Opening,
    Open,
    Closing,
}

/// Runtime state of a hinged [`PropDoor`]. `closed_yaw` / `open_yaw` are
/// absolute +Y rotations (radians); `speed` is radians/sec.
#[derive(Component)]
pub struct DoorSwing {
    closed_yaw: f32,
    open_yaw: f32,
    speed: f32,
    phase: SwingPhase,
    /// Mirrored from [`PropDoor::locked`] at spawn; Phase 16's lockpick clears
    /// it. `sync_lock_plates` pushes it to the plate material.
    pub locked: bool,
}

impl DoorSwing {
    /// Whether the leaf is open or on its way there — for telemetry / tests.
    pub fn is_open(&self) -> bool {
        matches!(self.phase, SwingPhase::Open | SwingPhase::Opening)
    }
}

/// The leaf's small square lock indicator (material swapped red/green by
/// `sync_lock_plates`).
#[derive(Component)]
pub struct LockPlate;

/// Shared door materials, built once at `Startup` (the `lights::LampAssets`
/// pattern). Leaf meshes are per-door — sized from `PropDoor`'s fields.
#[derive(Resource)]
pub struct DoorAssets {
    leaf: Handle<StandardMaterial>,
    handle: Handle<StandardMaterial>,
    lock_locked: Handle<StandardMaterial>,
    lock_open: Handle<StandardMaterial>,
}

pub fn setup_door_assets(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let lock = |emissive| StandardMaterial {
        base_color: Color::BLACK,
        emissive,
        ..default()
    };
    commands.insert_resource(DoorAssets {
        leaf: materials.add(StandardMaterial {
            base_color: config::DOOR_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        handle: materials.add(StandardMaterial {
            base_color: config::DOOR_HANDLE_COLOR,
            metallic: 0.6,
            perceptual_roughness: 0.3,
            ..default()
        }),
        lock_locked: materials.add(lock(config::DOOR_LOCK_LOCKED_EMISSIVE)),
        lock_open: materials.add(lock(config::DOOR_LOCK_OPEN_EMISSIVE)),
    });
}

/// `PropDoor` from the map → a hinged door. Rotates the entity's `Transform`
/// to `face_yaw` (the closed pose) and hangs the leaf, two handle knobs and a
/// lock plate off it as children. The leaf carries the solid `Collider`; the
/// entity carries `RigidBody::Kinematic`, so the child collider gets a
/// `ColliderOf` and blocks the player (a collider with no `RigidBody`
/// ancestor is silently invisible to ahoy's KCC — the `ladder.rs` lesson).
pub fn spawn_swing_doors(
    add: On<Add, PropDoor>,
    mut doors: Query<(&PropDoor, &mut Transform)>,
    assets: Res<DoorAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let Ok((door, mut transform)) = doors.get_mut(add.entity) else {
        return;
    };
    let closed_yaw = door.face_yaw.to_radians();
    let open_yaw = closed_yaw + door.swing.to_radians();
    transform.rotation = Quat::from_rotation_y(closed_yaw);

    let (w, h, t) = (door.width, door.height, door.thickness);
    // The leaf is built `2·rebate` wider and `rebate` taller than the nominal
    // opening and shifted `rebate` toward the hinge, so it laps its frame on
    // three sides (a doorstop) and no sub-unit drift between the metres leaf
    // and the integer-TB-unit hole can show as a gap. See `config::DOOR_REBATE`.
    let r = config::DOOR_REBATE;
    let (lw, lh) = (w + 2.0 * r, h + r);
    let leaf_mesh = meshes.add(door_leaf_mesh(lw, lh, t));
    let knob_mesh = meshes.add(Cuboid::new(0.035, 0.035, 0.09));
    let plate_mesh = meshes.add(Cuboid::new(0.08, 0.12, 0.02));
    let lock_mat = if door.locked.0 {
        assets.lock_locked.clone()
    } else {
        assets.lock_open.clone()
    };

    commands
        .entity(add.entity)
        .insert((
            RigidBody::Kinematic,
            DoorSwing {
                closed_yaw,
                open_yaw,
                speed: door.speed.to_radians(),
                phase: SwingPhase::Closed,
                locked: door.locked.0,
            },
        ))
        .with_children(|d| {
            // Leaf: pivot (entity origin) at local x=0 = the hinge jamb; the
            // leaf spans local x ∈ [-r, w+r] (laps both jambs) and y ∈ [0, h+r]
            // (bottom on the floor, laps the head).
            d.spawn((
                Mesh3d(leaf_mesh),
                MeshMaterial3d(assets.leaf.clone()),
                Collider::cuboid(lw, lh, t),
                Transform::from_xyz(w * 0.5, lh * 0.5, 0.0),
                Name::new("DoorLeaf"),
            ));
            // A lever knob on each face, near the latch edge, at hand height.
            for z in [t * 0.5 + 0.05, -(t * 0.5 + 0.05)] {
                d.spawn((
                    Mesh3d(knob_mesh.clone()),
                    MeshMaterial3d(assets.handle.clone()),
                    Transform::from_xyz(w - 0.10, 1.02, z),
                ));
            }
            // Lock plate, latch side, just above the knob, front face.
            d.spawn((
                LockPlate,
                Mesh3d(plate_mesh),
                MeshMaterial3d(lock_mat),
                Transform::from_xyz(w - 0.07, 1.24, t * 0.5 + 0.011),
            ));
        });
}

pub fn swing_doors(mut doors: Query<(&mut Transform, &mut DoorSwing)>, time: Res<Time>) {
    for (mut transform, mut swing) in &mut doors {
        let (target, arrived_phase) = match swing.phase {
            SwingPhase::Opening => (swing.open_yaw, SwingPhase::Open),
            SwingPhase::Closing => (swing.closed_yaw, SwingPhase::Closed),
            SwingPhase::Closed | SwingPhase::Open => continue,
        };
        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        let next = approach(yaw, target, swing.speed * time.delta_secs());
        transform.rotation = Quat::from_rotation_y(next);
        if next == target {
            swing.phase = arrived_phase;
        }
    }
}

/// E on a hinged door: toggle it, unless it's locked (then just swap the
/// prompt to say so).
pub fn toggle_swing_on_interact(
    trigger: On<Interacted>,
    mut doors: Query<(&mut DoorSwing, &mut Interactable)>,
) {
    let Ok((mut swing, mut interactable)) = doors.get_mut(trigger.entity) else {
        return;
    };
    if swing.locked {
        interactable.prompt = config::DOOR_PROMPT_LOCKED.to_string();
        return;
    }
    let (next, prompt) = match swing.phase {
        SwingPhase::Closed | SwingPhase::Closing => {
            (SwingPhase::Opening, config::DOOR_PROMPT_CLOSE)
        }
        SwingPhase::Open | SwingPhase::Opening => (SwingPhase::Closing, config::DOOR_PROMPT_OPEN),
    };
    swing.phase = next;
    interactable.prompt = prompt.to_string();
}

/// Idempotent mirror of `DoorSwing::locked` onto the lock-plate material — so
/// Phase 16's unlock (which just flips the bool) turns the plate green with no
/// transition to hook (the `lights::sync_*` rule).
pub fn sync_lock_plates(
    doors: Query<(&DoorSwing, &Children), Changed<DoorSwing>>,
    assets: Res<DoorAssets>,
    mut plates: Query<&mut MeshMaterial3d<StandardMaterial>, With<LockPlate>>,
) {
    for (swing, children) in &doors {
        for &child in children {
            if let Ok(mut mat) = plates.get_mut(child) {
                mat.0 = if swing.locked {
                    assets.lock_locked.clone()
                } else {
                    assets.lock_open.clone()
                };
            }
        }
    }
}

/// Move `current` toward `target` by at most `max_step`, snapping exactly on
/// arrival (so a door settles at its rest yaw with no residual jitter —
/// `move_towards`'s scalar twin).
fn approach(current: f32, target: f32, max_step: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_step {
        target
    } else {
        current + delta.signum() * max_step
    }
}

/// A door leaf: a slab plus two raised rectangular panel frames on each face,
/// merged into one mesh (the `ladder::ladder_mesh` idiom). Centred on the
/// origin, extents `(width, height, thickness)`.
fn door_leaf_mesh(width: f32, height: f32, thickness: f32) -> Mesh {
    let mut mesh = Mesh::from(Cuboid::new(width, height, thickness));

    let margin = (width.min(height) * 0.14).min(0.12);
    let mid = height * 0.5 - margin * 0.5;
    let rail = 0.03;
    let proud = thickness * 0.5 + 0.006;
    let pw = width - 2.0 * margin;
    let ph = height * 0.5 - 1.5 * margin;

    for &panel_y in &[mid * 0.5, -mid * 0.5] {
        for &face in &[proud, -proud] {
            // Top/bottom rails of this panel's frame.
            for &edge_y in &[panel_y + ph * 0.5, panel_y - ph * 0.5] {
                mesh.merge(
                    &Mesh::from(Cuboid::new(pw, rail, 0.012))
                        .translated_by(Vec3::new(0.0, edge_y, face)),
                )
                .unwrap();
            }
            // Left/right rails.
            for &edge_x in &[pw * 0.5, -pw * 0.5] {
                mesh.merge(
                    &Mesh::from(Cuboid::new(rail, ph, 0.012))
                        .translated_by(Vec3::new(edge_x, panel_y, face)),
                )
                .unwrap();
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::primitives::MeshAabb;

    #[test]
    fn approach_snaps_exactly_on_arrival() {
        assert_eq!(approach(0.0, 1.5, 0.4), 0.4);
        assert_eq!(approach(0.4, 1.5, 0.4), 0.8);
        // Overshoot lands exactly on target, not past it.
        assert_eq!(approach(1.3, 1.5, 0.4), 1.5);
        assert_eq!(approach(1.5, 1.5, 0.4), 1.5);
    }

    #[test]
    fn approach_works_downward_too() {
        assert_eq!(approach(1.5, 0.0, 0.4), 1.1);
        assert_eq!(approach(0.2, 0.0, 0.4), 0.0);
    }

    #[test]
    fn leaf_mesh_fills_its_declared_box() {
        let aabb = door_leaf_mesh(0.95, 2.1, 0.08)
            .compute_aabb()
            .expect("positions");
        let size = Vec3::from(aabb.half_extents) * 2.0;
        // The slab sets width/height; the proud panels only stick out ~6 mm on Z.
        assert!((size.x - 0.95).abs() < 1.0e-4, "width {}", size.x);
        assert!((size.y - 2.1).abs() < 1.0e-4, "height {}", size.y);
        assert!(
            size.z >= 0.08 && size.z < 0.08 + 0.03,
            "thickness {}",
            size.z
        );
    }
}
