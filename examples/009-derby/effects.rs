//! Engine steam and smoke via bevy_hanabi. Two shared `EffectAsset`s are
//! authored once at startup; every car gets one steam and one smoke emitter
//! child parked over its hood, both spawning inactive. A per-frame system
//! flips each emitter from its car's engine health: white steam pours below
//! `STEAM_THRESHOLD`, turning to black smoke below `SMOKE_THRESHOLD`.
//! Particles simulate in world space, so a limping car drags a trail.

use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use crate::config::*;
use crate::damage::{Damage, ImpactBurst};
use crate::game::Wrecked;
use bevy_game_bits::vehicle::WheelLanding;

/// Which engine-state effect an emitter child renders.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum EngineEmitter {
    Steam,
    Smoke,
}

/// Shared effect handles, built by `setup_effects` before the cars spawn
/// (`spawn_vehicle` clones them onto every car's emitter children).
#[derive(Resource)]
pub struct EngineEffects {
    pub steam: Handle<EffectAsset>,
    pub smoke: Handle<EffectAsset>,
}

/// One-shot effects thrown at collision points: spark showers in two sizes
/// (`light` for scratches under `BURST_HEAVY_DAMAGE` part health removed,
/// `heavy` for real hits) and a `dust` puff for a hard wheel landing — the
/// latter carries no damage at all (`vehicle::WheelLanding`,
/// `spawn_landing_dust`), so it's visually distinct: cool and settling
/// rather than hot and bursting.
#[derive(Resource)]
pub struct BurstEffects {
    pub light: Handle<EffectAsset>,
    pub heavy: Handle<EffectAsset>,
    pub dust: Handle<EffectAsset>,
}

/// Tags a transient burst-emitter entity; despawned once the timer (sized
/// to outlast the longest spark) runs out.
#[derive(Component)]
pub struct BurstLifetime(Timer);

pub fn setup_effects(mut commands: Commands, mut effects: ResMut<Assets<EffectAsset>>) {
    // Steam: small bright puffs rising briskly and fading fast.
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.9, 0.9, 0.9, 0.35));
    color.add_key(0.4, Vec4::new(0.95, 0.95, 0.95, 0.2));
    color.add_key(1.0, Vec4::new(1.0, 1.0, 1.0, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.15));
    size.add_key(1.0, Vec3::splat(0.55));
    let steam = effects.add(engine_effect(
        "engine-steam",
        STEAM_PARTICLE_CAPACITY,
        STEAM_SPAWN_RATE,
        STEAM_PARTICLE_LIFETIME,
        Vec3::new(-0.4, 1.2, -0.4),
        Vec3::new(0.4, 2.2, 0.4),
        color,
        size,
    ));

    // Smoke: bigger, near-black, slower, longer-lived.
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.02, 0.02, 0.02, 0.75));
    color.add_key(0.5, Vec4::new(0.05, 0.05, 0.05, 0.5));
    color.add_key(1.0, Vec4::new(0.08, 0.08, 0.08, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.25));
    size.add_key(1.0, Vec3::splat(1.1));
    let smoke = effects.add(engine_effect(
        "engine-smoke",
        SMOKE_PARTICLE_CAPACITY,
        SMOKE_SPAWN_RATE,
        SMOKE_PARTICLE_LIFETIME,
        Vec3::new(-0.5, 0.8, -0.5),
        Vec3::new(0.5, 1.6, 0.5),
        color,
        size,
    ));

    commands.insert_resource(EngineEffects { steam, smoke });
    commands.insert_resource(BurstEffects {
        light: effects.add(spark_burst("impact-burst-light", BURST_LIGHT_COUNT)),
        heavy: effects.add(spark_burst("impact-burst-heavy", BURST_HEAVY_COUNT)),
        dust: effects.add(landing_dust()),
    });
}

/// A one-shot spark shower: `count` hot orange particles fired radially
/// with an upward bias from the impact point, pulled down by gravity,
/// shrinking and cooling to transparent within a second.
fn spark_burst(name: &str, count: f32) -> EffectAsset {
    let spawner = SpawnerSettings::once(count.into());

    let writer = ExprWriter::new();
    let init_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        writer
            .lit(Vec3::splat(-0.15))
            .uniform(writer.lit(Vec3::splat(0.15)))
            .expr(),
    );
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        writer
            .lit(Vec3::new(-4.0, 0.5, -4.0))
            .uniform(writer.lit(Vec3::new(4.0, 5.0, 4.0)))
            .expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer
            .lit(BURST_LIFETIME_MIN)
            .uniform(writer.lit(BURST_LIFETIME_MAX))
            .expr(),
    );
    let gravity = AccelModifier::new(writer.lit(Vec3::new(0.0, -9.8, 0.0)).expr());
    let module = writer.finish();

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(3.0, 2.8, 1.8, 1.0));
    color.add_key(0.3, Vec4::new(2.5, 1.2, 0.3, 0.9));
    color.add_key(1.0, Vec4::new(1.0, 0.2, 0.05, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.09));
    size.add_key(1.0, Vec3::splat(0.02));

    EffectAsset::new(BURST_PARTICLE_CAPACITY, spawner, module)
        .with_name(name)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .update(gravity)
        .render(SizeOverLifetimeModifier {
            gradient: size,
            ..default()
        })
        .render(ColorOverLifetimeModifier::new(color))
}

/// A one-shot dust puff: `LANDING_DUST_COUNT` grey-tan particles fired low
/// and outward from a wheel's touchdown point, growing and fading as they
/// settle — the inverse curve of `spark_burst`'s hot-and-shrinking sparks,
/// since dust billows out rather than burning down.
fn landing_dust() -> EffectAsset {
    let spawner = SpawnerSettings::once(LANDING_DUST_COUNT.into());

    let writer = ExprWriter::new();
    let init_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        writer
            .lit(Vec3::new(-0.2, 0.0, -0.2))
            .uniform(writer.lit(Vec3::new(0.2, 0.1, 0.2)))
            .expr(),
    );
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        writer
            .lit(Vec3::new(-2.0, 0.2, -2.0))
            .uniform(writer.lit(Vec3::new(2.0, 1.2, 2.0)))
            .expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer
            .lit(LANDING_DUST_LIFETIME_MIN)
            .uniform(writer.lit(LANDING_DUST_LIFETIME_MAX))
            .expr(),
    );
    // Gentle drag rather than gravity: dust hangs and drifts instead of
    // falling like the sparks do.
    let drag = LinearDragModifier::new(writer.lit(1.5).expr());
    let module = writer.finish();

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.55, 0.5, 0.4, 0.5));
    color.add_key(0.5, Vec4::new(0.6, 0.56, 0.48, 0.35));
    color.add_key(1.0, Vec4::new(0.65, 0.62, 0.55, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.1));
    size.add_key(1.0, Vec3::splat(0.6));

    EffectAsset::new(LANDING_DUST_PARTICLE_CAPACITY, spawner, module)
        .with_name("landing-dust")
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .update(drag)
        .render(SizeOverLifetimeModifier { gradient: size, ..default() })
        .render(ColorOverLifetimeModifier::new(color))
}

/// Spawns a transient one-shot dust emitter at each hard wheel touchdown —
/// mirrors `spawn_impact_bursts` exactly, reusing the same `BurstLifetime`
/// reaper. Purely cosmetic: `vehicle::WheelLanding` carries no damage.
pub fn spawn_landing_dust(
    mut commands: Commands,
    effects: Res<BurstEffects>,
    mut landings: MessageReader<WheelLanding>,
) {
    for landing in landings.read() {
        commands.spawn((
            Name::new("LandingDust"),
            ParticleEffect::new(effects.dust.clone()),
            Transform::from_translation(landing.position),
            BurstLifetime(Timer::from_seconds(BURST_ENTITY_AGE, TimerMode::Once)),
        ));
    }
}

/// Spawns a transient one-shot emitter at each registered impact — the
/// `once` spawner fires the whole burst on the entity's first frame.
pub fn spawn_impact_bursts(
    mut commands: Commands,
    effects: Res<BurstEffects>,
    mut bursts: MessageReader<ImpactBurst>,
) {
    for burst in bursts.read() {
        let handle = if burst.heavy {
            effects.heavy.clone()
        } else {
            effects.light.clone()
        };
        commands.spawn((
            Name::new("ImpactBurst"),
            ParticleEffect::new(handle),
            Transform::from_translation(burst.position),
            BurstLifetime(Timer::from_seconds(BURST_ENTITY_AGE, TimerMode::Once)),
        ));
    }
}

/// Reaps burst emitters once every spark they fired has expired.
pub fn despawn_finished_bursts(
    time: Res<Time>,
    mut commands: Commands,
    mut bursts: Query<(Entity, &mut BurstLifetime)>,
) {
    for (entity, mut lifetime) in &mut bursts {
        if lifetime.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// One hood-plume effect: particles spawn in a small box over the emitter,
/// pick a velocity uniformly between `vel_min` and `vel_max` (mostly up,
/// some spread), and fade out per the gradients.
#[allow(clippy::too_many_arguments)]
fn engine_effect(
    name: &str,
    capacity: u32,
    rate: f32,
    lifetime: f32,
    vel_min: Vec3,
    vel_max: Vec3,
    color: bevy_hanabi::Gradient<Vec4>,
    size: bevy_hanabi::Gradient<Vec3>,
) -> EffectAsset {
    let spawner = SpawnerSettings::rate(rate.into()).with_starts_active(false);

    let writer = ExprWriter::new();
    let init_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        writer
            .lit(Vec3::new(-0.2, 0.0, -0.2))
            .uniform(writer.lit(Vec3::new(0.2, 0.1, 0.2)))
            .expr(),
    );
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        writer.lit(vel_min).uniform(writer.lit(vel_max)).expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(lifetime).expr());
    let module = writer.finish();

    EffectAsset::new(capacity, spawner, module)
        .with_name(name)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(SizeOverLifetimeModifier {
            gradient: size,
            ..default()
        })
        .render(ColorOverLifetimeModifier::new(color))
}

/// Drives each emitter's active flag from its car's engine health. The
/// `EffectSpawner` is `Option` because Hanabi inserts it a frame after the
/// `ParticleEffect` spawns (same guard as 008-colony's fire). A wrecked car
/// smokes regardless of its engine's own health — it's an obstacle now, and
/// should read as dead even if what killed it was cosmetic damage.
pub fn update_engine_emitters(
    cars: Query<(&Damage, Has<Wrecked>)>,
    mut emitters: Query<(&EngineEmitter, &ChildOf, Option<&mut EffectSpawner>)>,
) {
    for (emitter, child_of, spawner) in &mut emitters {
        let Ok((damage, wrecked)) = cars.get(child_of.parent()) else {
            continue;
        };
        let Some(mut spawner) = spawner else {
            continue;
        };
        let active = match emitter {
            EngineEmitter::Steam => {
                !wrecked && damage.engine <= STEAM_THRESHOLD && damage.engine > SMOKE_THRESHOLD
            }
            EngineEmitter::Smoke => wrecked || damage.engine <= SMOKE_THRESHOLD,
        };
        if spawner.active != active {
            spawner.active = active;
        }
    }
}
