use bevy::prelude::*;
use std::f32::consts::{PI, TAU};

use crate::config::*;
use crate::enemy::Enemy;
use crate::game::GameAssets;
use crate::weapons::{KineticWeapon, LaserWeapon};

#[derive(Component, Clone, Copy)]
pub enum TurretKind {
    Kinetic,
    Laser,
}

#[derive(Component)]
pub enum TurretAi {
    LookingForTarget,
    TargetAcquired(Entity),
}

/// Direct handles to a turret's part entities, so AI systems can address
/// the gun and sensor without walking the `Children` hierarchy.
#[derive(Component)]
pub struct TurretParts {
    pub gun: Entity,
    pub sensor: Entity,
}

#[derive(Component)]
pub struct Gun {
    pub yaw: f32,
    pub aligned: bool,
}

#[derive(Component)]
pub struct Sensor {
    pub yaw: f32,
    pub sweep_dir: f32,
}

/// Turret forward is -Z at yaw 0 (facing the spawn edge); yaw rotates around +Y.
pub fn yaw_direction(yaw: f32) -> Vec3 {
    Vec3::new(-yaw.sin(), 0.0, -yaw.cos())
}

pub fn direction_to_yaw(direction: Vec3) -> f32 {
    f32::atan2(-direction.x, -direction.z)
}

fn wrap_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// Smallest t >= 0 with |origin + t * dir - center|^2 = radius^2 (dir normalized).
pub fn ray_sphere_intersect(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let c = oc.length_squared() - radius * radius;
    let discriminant = b * b - c;

    if discriminant < 0.0 {
        return None;
    }

    let sqrt_d = discriminant.sqrt();
    let t = -b - sqrt_d;
    if t >= 0.0 {
        return Some(t);
    }

    let t = -b + sqrt_d;
    (t >= 0.0).then_some(t)
}

pub fn spawn_turret(commands: &mut Commands, assets: &GameAssets, kind: TurretKind, position: Vec3) {
    let base_material = match kind {
        TurretKind::Kinetic => assets.kinetic_base_material.clone(),
        TurretKind::Laser => assets.laser_base_material.clone(),
    };

    let mut gun = Entity::PLACEHOLDER;
    let mut sensor = Entity::PLACEHOLDER;

    let mut root = commands.spawn((
        kind,
        TurretAi::LookingForTarget,
        Transform::from_translation(position),
        Visibility::default(),
    ));

    root.with_children(|parent| {
        // Tripod: three legs from evenly spaced feet up to a hub under the gun.
        // The base never rotates, so child pivots' local yaw == world yaw.
        let apex = Vec3::Y * TRIPOD_APEX_HEIGHT;
        for i in 0..3 {
            let angle = i as f32 * TAU / 3.0;
            let foot = Vec3::new(
                angle.sin() * TRIPOD_FOOT_RADIUS,
                0.0,
                angle.cos() * TRIPOD_FOOT_RADIUS,
            );
            parent.spawn((
                Mesh3d(assets.leg_mesh.clone()),
                MeshMaterial3d(assets.leg_material.clone()),
                Transform {
                    translation: (foot + apex) / 2.0,
                    rotation: Quat::from_rotation_arc(Vec3::Y, (apex - foot).normalize()),
                    ..default()
                },
            ));
        }

        // Kind-colored hub where the legs meet.
        parent.spawn((
            Mesh3d(assets.hub_mesh.clone()),
            MeshMaterial3d(base_material),
            Transform::from_translation(apex),
        ));

        gun = parent
            .spawn((
                Gun {
                    yaw: 0.0,
                    aligned: false,
                },
                Transform::from_xyz(0.0, GUN_HEIGHT, 0.0),
                Visibility::default(),
            ))
            .with_children(|pivot| {
                pivot.spawn((
                    Mesh3d(assets.barrel_mesh.clone()),
                    MeshMaterial3d(assets.barrel_material.clone()),
                    Transform::from_xyz(0.0, 0.0, -BARREL_LENGTH / 2.0),
                ));
            })
            .id();

        sensor = parent
            .spawn((
                Sensor {
                    yaw: 0.0,
                    sweep_dir: 1.0,
                },
                Transform::from_xyz(0.0, SENSOR_HEIGHT, 0.0),
                Visibility::default(),
            ))
            .with_children(|pivot| {
                pivot.spawn((
                    Mesh3d(assets.sensor_mesh.clone()),
                    MeshMaterial3d(assets.sensor_material.clone()),
                    Transform::default(),
                ));
            })
            .id();
    });

    root.insert(TurretParts { gun, sensor });

    match kind {
        TurretKind::Kinetic => root.insert(KineticWeapon::default()),
        TurretKind::Laser => root.insert(LaserWeapon {
            dps: LASER_DPS,
            firing_at: None,
        }),
    };
}

pub fn sweep_sensors(
    time: Res<Time>,
    turrets: Query<(&TurretAi, &TurretParts)>,
    mut sensors: Query<(&mut Sensor, &mut Transform)>,
) {
    for (ai, parts) in &turrets {
        if !matches!(ai, TurretAi::LookingForTarget) {
            continue;
        }
        let Ok((mut sensor, mut transform)) = sensors.get_mut(parts.sensor) else {
            continue;
        };

        sensor.yaw += sensor.sweep_dir * SWEEP_SPEED * time.delta_secs();
        if sensor.yaw.abs() > SWEEP_LIMIT {
            sensor.yaw = sensor.yaw.clamp(-SWEEP_LIMIT, SWEEP_LIMIT);
            sensor.sweep_dir = -sensor.sweep_dir;
        }

        transform.rotation = Quat::from_rotation_y(sensor.yaw);
    }
}

#[allow(clippy::type_complexity)]
pub fn acquire_and_validate_targets(
    time: Res<Time>,
    mut turrets: Query<(&Transform, &mut TurretAi, &TurretParts), Without<Sensor>>,
    mut sensors: Query<(&mut Sensor, &mut Transform), (Without<TurretAi>, Without<Enemy>)>,
    enemies: Query<(Entity, &Transform, &Enemy)>,
) {
    for (root_transform, mut ai, parts) in &mut turrets {
        let root_pos = root_transform.translation;
        let sensor_origin = root_pos + Vec3::Y * SENSOR_HEIGHT;

        // Drop targets that died, despawned or left sensor range.
        if let TurretAi::TargetAcquired(target) = *ai {
            let still_valid = enemies.get(target).is_ok_and(|(_, enemy_transform, _)| {
                planar_distance(root_pos, enemy_transform.translation) <= SENSOR_RANGE
            });
            if !still_valid {
                *ai = TurretAi::LookingForTarget;
            }
        }

        let Ok((mut sensor, mut sensor_transform)) = sensors.get_mut(parts.sensor) else {
            continue;
        };

        if matches!(*ai, TurretAi::LookingForTarget) {
            let forward = yaw_direction(sensor.yaw);

            // All enemies inside the vision cone, nearest first.
            let mut candidates: Vec<(Entity, Vec3, f32)> = enemies
                .iter()
                .filter_map(|(entity, enemy_transform, _)| {
                    let to_enemy = enemy_transform.translation - root_pos;
                    let planar = Vec3::new(to_enemy.x, 0.0, to_enemy.z);
                    let distance = planar.length();
                    if distance < f32::EPSILON || distance > SENSOR_RANGE {
                        return None;
                    }
                    let angle = forward.angle_between(planar / distance);
                    (angle <= CONE_HALF_ANGLE).then_some((
                        entity,
                        enemy_transform.translation,
                        distance,
                    ))
                })
                .collect();
            candidates.sort_by(|a, b| a.2.total_cmp(&b.2));

            // Nearest candidate with a clear line of sight from the sensor.
            for (entity, center, _) in candidates {
                if has_line_of_sight(sensor_origin, entity, center, &enemies) {
                    *ai = TurretAi::TargetAcquired(entity);
                    break;
                }
            }
        }

        // Track the acquired target: the cone follows it.
        if let TurretAi::TargetAcquired(target) = *ai {
            if let Ok((_, enemy_transform, _)) = enemies.get(target) {
                let to_enemy = enemy_transform.translation - root_pos;
                let target_yaw = direction_to_yaw(Vec3::new(to_enemy.x, 0.0, to_enemy.z));
                let step = SWEEP_SPEED * time.delta_secs();
                let delta = wrap_angle(target_yaw - sensor.yaw);
                sensor.yaw = wrap_angle(sensor.yaw + delta.clamp(-step, step));
                sensor_transform.rotation = Quat::from_rotation_y(sensor.yaw);
            }
        }
    }
}

fn planar_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

/// Raycast from the sensor to the target center; any *other* enemy sphere
/// intersecting the ray before the target blocks line of sight.
fn has_line_of_sight(
    origin: Vec3,
    target: Entity,
    target_center: Vec3,
    enemies: &Query<(Entity, &Transform, &Enemy)>,
) -> bool {
    let to_target = target_center - origin;
    let distance = to_target.length();
    if distance < f32::EPSILON {
        return true;
    }
    let dir = to_target / distance;

    for (other, other_transform, other_enemy) in enemies.iter() {
        if other == target {
            continue;
        }
        if let Some(t) = ray_sphere_intersect(origin, dir, other_transform.translation, other_enemy.radius)
        {
            if t < distance - 0.01 {
                return false;
            }
        }
    }

    true
}

#[allow(clippy::type_complexity)]
pub fn aim_guns(
    time: Res<Time>,
    turrets: Query<(&Transform, &TurretAi, &TurretParts)>,
    mut guns: Query<(&mut Gun, &mut Transform), (Without<TurretAi>, Without<Enemy>)>,
    enemies: Query<&Transform, With<Enemy>>,
) {
    for (root_transform, ai, parts) in &turrets {
        let Ok((mut gun, mut gun_transform)) = guns.get_mut(parts.gun) else {
            continue;
        };

        let target_transform = match ai {
            TurretAi::TargetAcquired(target) => enemies.get(*target).ok(),
            TurretAi::LookingForTarget => None,
        };
        let Some(enemy_transform) = target_transform else {
            gun.aligned = false;
            continue;
        };

        let to_enemy = enemy_transform.translation - root_transform.translation;
        let target_yaw = direction_to_yaw(Vec3::new(to_enemy.x, 0.0, to_enemy.z));

        let step = GUN_TURN_SPEED * time.delta_secs();
        let delta = wrap_angle(target_yaw - gun.yaw);
        gun.yaw = wrap_angle(gun.yaw + delta.clamp(-step, step));
        gun.aligned = wrap_angle(target_yaw - gun.yaw).abs() < ALIGN_THRESHOLD;

        gun_transform.rotation = Quat::from_rotation_y(gun.yaw);
    }
}

pub fn draw_sensor_cones(
    mut gizmos: Gizmos,
    turrets: Query<(&Transform, &TurretAi, &TurretParts)>,
    sensors: Query<&Sensor>,
) {
    const ARC_SEGMENTS: usize = 16;

    for (root_transform, ai, parts) in &turrets {
        let Ok(sensor) = sensors.get(parts.sensor) else {
            continue;
        };

        let apex = root_transform.translation + Vec3::Y * 0.15;
        let color = match ai {
            TurretAi::LookingForTarget => CONE_SEARCH_COLOR,
            TurretAi::TargetAcquired(_) => CONE_LOCKED_COLOR,
        };

        let arc = (0..=ARC_SEGMENTS).map(|i| {
            let angle = sensor.yaw - CONE_HALF_ANGLE
                + 2.0 * CONE_HALF_ANGLE * i as f32 / ARC_SEGMENTS as f32;
            apex + yaw_direction(angle) * SENSOR_RANGE
        });
        gizmos.linestrip(arc, color);

        for edge in [-CONE_HALF_ANGLE, CONE_HALF_ANGLE] {
            gizmos.line(
                apex,
                apex + yaw_direction(sensor.yaw + edge) * SENSOR_RANGE,
                color,
            );
        }
    }
}
