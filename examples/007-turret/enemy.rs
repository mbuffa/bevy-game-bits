use bevy::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::*;
use crate::game::{DamageMessage, GameAssets};

#[derive(Component)]
pub struct Enemy {
    pub radius: f32,
}

#[derive(Component)]
pub struct Health {
    pub current: f32,
}

#[derive(Component)]
pub struct Velocity(pub Vec3);

/// While present, the enemy renders with the shared flash material;
/// `original` restores its normal material when the timer runs out.
#[derive(Component)]
pub struct HitFlash {
    pub timer: Timer,
    pub original: MeshMaterial3d<StandardMaterial>,
}

#[derive(Resource)]
pub struct WaveTimer(pub Timer);

/// Minimal xorshift64* PRNG, so the example needs no extra dependency.
#[derive(Resource)]
pub struct SimpleRng(u64);

impl SimpleRng {
    pub fn from_time() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0xB5AD4ECEDA1CE2A9);
        Self(nanos | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    pub fn range_u32(&mut self, low: u32, high_inclusive: u32) -> u32 {
        low + (self.next_u64() >> 33) as u32 % (high_inclusive - low + 1)
    }
}

pub fn spawn_waves(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<WaveTimer>,
    mut rng: ResMut<SimpleRng>,
    assets: Res<GameAssets>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let count = rng.range_u32(WAVE_MIN_ENEMIES, WAVE_MAX_ENEMIES);

    for i in 0..count {
        let x = -FIELD_WIDTH / 2.0 + FIELD_WIDTH * (i as f32 + 0.5) / count as f32;

        commands.spawn((
            Enemy {
                radius: ENEMY_RADIUS,
            },
            Health {
                current: ENEMY_MAX_HP,
            },
            Velocity(Vec3::Z * ENEMY_SPEED),
            Mesh3d(assets.enemy_mesh.clone()),
            MeshMaterial3d(assets.enemy_material.clone()),
            Transform::from_xyz(x, ENEMY_RADIUS, -FIELD_DEPTH / 2.0),
        ));
    }
}

pub fn move_enemies(
    mut commands: Commands,
    time: Res<Time>,
    mut enemies: Query<(Entity, &mut Transform, &Velocity), With<Enemy>>,
) {
    for (entity, mut transform, velocity) in &mut enemies {
        transform.translation += velocity.0 * time.delta_secs();

        if transform.translation.z > FIELD_DEPTH / 2.0 + ENEMY_RADIUS {
            commands.entity(entity).despawn();
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn apply_damage(
    mut commands: Commands,
    mut messages: MessageReader<DamageMessage>,
    assets: Res<GameAssets>,
    mut enemies: Query<
        (
            &mut Health,
            &MeshMaterial3d<StandardMaterial>,
            Option<&mut HitFlash>,
        ),
        With<Enemy>,
    >,
) {
    for message in messages.read() {
        let Ok((mut health, material, hit_flash)) = enemies.get_mut(message.target) else {
            continue;
        };

        health.current -= message.amount;

        match hit_flash {
            // Already flashing: keep the stored original material, extend the blink.
            Some(mut hit_flash) => hit_flash.timer.reset(),
            None => {
                commands.entity(message.target).insert((
                    HitFlash {
                        timer: Timer::from_seconds(FLASH_DURATION, TimerMode::Once),
                        original: material.clone(),
                    },
                    MeshMaterial3d(assets.flash_material.clone()),
                ));
            }
        }
    }
}

pub fn update_hit_flash(
    mut commands: Commands,
    time: Res<Time>,
    mut flashing: Query<(Entity, &mut HitFlash)>,
) {
    for (entity, mut hit_flash) in &mut flashing {
        if hit_flash.timer.tick(time.delta()).is_finished() {
            commands
                .entity(entity)
                .insert(hit_flash.original.clone())
                .remove::<HitFlash>();
        }
    }
}

pub fn despawn_dead(mut commands: Commands, enemies: Query<(Entity, &Health), With<Enemy>>) {
    for (entity, health) in &enemies {
        if health.current <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}
