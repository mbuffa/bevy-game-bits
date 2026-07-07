use bevy::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::*;
use crate::game::{DamageMessage, EnemyResolved, GameAssets};
use crate::waves::{EnemyArchetype, WAVES};

#[derive(Component)]
pub struct Enemy {
    pub radius: f32,
    /// Base HP lost if this enemy leaks past the near edge.
    pub leak_cost: u32,
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

/// Playback state of the current wave's spawn schedule, plus the spawned /
/// resolved tally that decides when the wave is over. Reset by `start_wave`
/// on entering `WaveActive`.
#[derive(Resource, Default)]
pub struct ActiveWave {
    pub elapsed: f32,
    /// Enemies spawned so far per group, parallel to `WAVES[wave].groups`.
    pub cursors: Vec<u32>,
    pub spawned: u32,
    pub resolved: u32,
}

impl ActiveWave {
    pub fn all_spawned(&self, wave: usize) -> bool {
        WAVES[wave]
            .groups
            .iter()
            .zip(&self.cursors)
            .all(|(group, cursor)| *cursor >= group.count)
    }
}

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

    pub fn range_f32(&mut self, low: f32, high: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        low + unit * (high - low)
    }
}

/// Play back the current wave's spawn schedule. The while-loop catches up
/// after frame hitches and supports `interval: 0.0` (whole group at once).
pub fn spawn_wave_enemies(
    mut commands: Commands,
    time: Res<Time>,
    mut wave: ResMut<ActiveWave>,
    current: Res<crate::game::CurrentWave>,
    mut rng: ResMut<SimpleRng>,
    assets: Res<GameAssets>,
) {
    wave.elapsed += time.delta_secs();

    for (i, group) in WAVES[current.0].groups.iter().enumerate() {
        while wave.cursors[i] < group.count
            && wave.elapsed >= group.start_delay + wave.cursors[i] as f32 * group.interval
        {
            let x = rng.range_f32(-FIELD_WIDTH / 2.0 + 1.0, FIELD_WIDTH / 2.0 - 1.0);
            spawn_enemy(&mut commands, &assets, group.archetype, x);
            wave.cursors[i] += 1;
            wave.spawned += 1;
        }
    }
}

fn spawn_enemy(commands: &mut Commands, assets: &GameAssets, archetype: EnemyArchetype, x: f32) {
    let stats = archetype.stats();
    let material = match archetype {
        EnemyArchetype::Grunt => assets.enemy_material.clone(),
        EnemyArchetype::Runner => assets.runner_material.clone(),
        EnemyArchetype::Brute => assets.brute_material.clone(),
    };

    commands.spawn((
        Enemy {
            radius: ENEMY_RADIUS * stats.scale,
            leak_cost: stats.leak_cost,
        },
        Health { current: stats.hp },
        Velocity(Vec3::Z * stats.speed),
        Mesh3d(assets.enemy_mesh.clone()),
        MeshMaterial3d(material),
        Transform {
            translation: Vec3::new(x, ENEMY_SIZE.y / 2.0 * stats.scale, -FIELD_DEPTH / 2.0),
            scale: Vec3::splat(stats.scale),
            ..default()
        },
    ));
}

pub fn move_enemies(
    time: Res<Time>,
    mut enemies: Query<(&mut Transform, &Velocity), With<Enemy>>,
) {
    for (mut transform, velocity) in &mut enemies {
        transform.translation += velocity.0 * time.delta_secs();
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

/// The single place enemies leave the field: dead first, then leaked. Exactly
/// one `EnemyResolved` per enemy keeps the wave-end tally correct — never
/// despawn enemies anywhere else.
pub fn resolve_enemies(
    mut commands: Commands,
    mut writer: MessageWriter<EnemyResolved>,
    enemies: Query<(Entity, &Health, &Transform, &Enemy)>,
) {
    for (entity, health, transform, enemy) in &enemies {
        if health.current <= 0.0 {
            commands.entity(entity).despawn();
            writer.write(EnemyResolved {
                leaked: false,
                leak_cost: 0,
            });
        } else if transform.translation.z > FIELD_DEPTH / 2.0 + enemy.radius {
            commands.entity(entity).despawn();
            writer.write(EnemyResolved {
                leaked: true,
                leak_cost: enemy.leak_cost,
            });
        }
    }
}
