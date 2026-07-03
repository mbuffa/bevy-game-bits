use bevy::prelude::*;

use crate::config::*;
use crate::enemy::Enemy;
use crate::game::{DamageMessage, GameAssets};
use crate::turret::{yaw_direction, Gun, TurretAi, TurretParts};

#[derive(Component)]
pub struct KineticWeapon {
    pub shot_timer: Timer,
    /// Total rounds fired, drives the every-Nth tracer cadence.
    pub rounds_fired: u32,
}

impl Default for KineticWeapon {
    fn default() -> Self {
        Self {
            shot_timer: Timer::from_seconds(MINIGUN_SHOT_INTERVAL, TimerMode::Repeating),
            rounds_fired: 0,
        }
    }
}

#[derive(Component)]
pub struct LaserWeapon {
    pub dps: f32,
    /// Target the beam is currently burning; drives the beam gizmo.
    pub firing_at: Option<Entity>,
}

#[derive(Component)]
pub struct Projectile {
    pub velocity: Vec3,
    pub damage: f32,
    pub radius: f32,
}

pub fn fire_kinetic(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<GameAssets>,
    mut turrets: Query<(&Transform, &TurretParts, &mut KineticWeapon)>,
    guns: Query<&Gun>,
) {
    for (root_transform, parts, mut weapon) in &mut turrets {
        let Ok(gun) = guns.get(parts.gun) else {
            continue;
        };

        // Minigun: continuous stream while aimed; the stream pauses with the aim.
        if !gun.aligned {
            continue;
        }

        if weapon.shot_timer.tick(time.delta()).just_finished() {
            let tracer = weapon.rounds_fired % TRACER_EVERY == 0;
            spawn_projectile(
                &mut commands,
                &assets,
                root_transform.translation,
                gun.yaw,
                tracer,
            );
            weapon.rounds_fired += 1;
        }
    }
}

fn spawn_projectile(
    commands: &mut Commands,
    assets: &GameAssets,
    root_pos: Vec3,
    yaw: f32,
    tracer: bool,
) {
    let dir = yaw_direction(yaw);
    let muzzle = root_pos + Vec3::Y * GUN_HEIGHT + dir * BARREL_LENGTH;

    let mut round = commands.spawn((
        Projectile {
            velocity: dir * PROJECTILE_SPEED,
            damage: MINIGUN_DAMAGE,
            radius: PROJECTILE_RADIUS,
        },
        Transform::from_translation(muzzle),
    ));

    // Most rounds are invisible; every Nth is a glowing tracer carrying a light.
    if tracer {
        round.insert((
            Mesh3d(assets.projectile_mesh.clone()),
            MeshMaterial3d(assets.projectile_material.clone()),
            PointLight {
                color: TRACER_LIGHT_COLOR,
                intensity: 20_000.0,
                range: 6.0,
                shadows_enabled: false,
                ..default()
            },
        ));
    }
}

pub fn fire_lasers(
    time: Res<Time>,
    mut writer: MessageWriter<DamageMessage>,
    mut turrets: Query<(&TurretAi, &TurretParts, &mut LaserWeapon)>,
    guns: Query<&Gun>,
) {
    for (ai, parts, mut laser) in &mut turrets {
        laser.firing_at = None;

        let TurretAi::TargetAcquired(target) = ai else {
            continue;
        };
        let Ok(gun) = guns.get(parts.gun) else {
            continue;
        };

        if gun.aligned {
            // Damage-per-second, frame-rate independent.
            writer.write(DamageMessage {
                target: *target,
                amount: laser.dps * time.delta_secs(),
            });
            laser.firing_at = Some(*target);
        }
    }
}

pub fn draw_laser_beams(
    mut gizmos: Gizmos,
    turrets: Query<(&Transform, &TurretParts, &LaserWeapon)>,
    guns: Query<&Gun>,
    enemies: Query<&Transform, With<Enemy>>,
) {
    for (root_transform, parts, laser) in &turrets {
        let Some(target) = laser.firing_at else {
            continue;
        };
        let (Ok(gun), Ok(enemy_transform)) = (guns.get(parts.gun), enemies.get(target)) else {
            continue;
        };

        let muzzle =
            root_transform.translation + Vec3::Y * GUN_HEIGHT + yaw_direction(gun.yaw) * BARREL_LENGTH;
        gizmos.line(muzzle, enemy_transform.translation, LASER_BEAM_COLOR);
    }
}

pub fn move_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    mut projectiles: Query<(Entity, &mut Transform, &Projectile)>,
) {
    for (entity, mut transform, projectile) in &mut projectiles {
        transform.translation += projectile.velocity * time.delta_secs();

        let pos = transform.translation;
        if pos.x.abs() > FIELD_WIDTH / 2.0 + 2.0 || pos.z.abs() > FIELD_DEPTH / 2.0 + 2.0 {
            commands.entity(entity).despawn();
        }
    }
}

pub fn collide_projectiles(
    mut commands: Commands,
    mut writer: MessageWriter<DamageMessage>,
    projectiles: Query<(Entity, &Transform, &Projectile)>,
    enemies: Query<(Entity, &Transform, &Enemy)>,
) {
    for (projectile_entity, projectile_transform, projectile) in &projectiles {
        for (enemy_entity, enemy_transform, enemy) in &enemies {
            let offset = projectile_transform.translation - enemy_transform.translation;
            let planar_distance = Vec2::new(offset.x, offset.z).length();

            if planar_distance <= projectile.radius + enemy.radius {
                writer.write(DamageMessage {
                    target: enemy_entity,
                    amount: projectile.damage,
                });
                commands.entity(projectile_entity).despawn();
                break;
            }
        }
    }
}
