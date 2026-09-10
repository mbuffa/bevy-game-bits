//! `PropWoodCrate` — a wooden crate that holds an item and breaks on a hard
//! enough impact (or, Phase 16, a crowbar swing). Throwing it off platform B
//! onto the concrete is the intended opener; `carry.rs` lifts and throws it
//! with no changes, because its grab gate is body-type + mass and never checks
//! for `PropCrate`.
//!
//! | system / observer | when | job |
//! |---|---|---|
//! | `setup_breakable_assets` | `Startup` | shared wood material + debris mesh |
//! | `spawn_wood_crates` | `On<Add, PropWoodCrate>` | dynamic body, collider, `Mass`, `Breakable`, plank mesh |
//! | `damage_on_impact` | `Update` | read Avian contact impulses → subtract `damage_from_delta_v` |
//! | `shatter` | `Update`, after `damage_on_impact` | `health <= 0` → debris + spawn `contains` as an `ItemPickup` + despawn |
//! | `despawn_after` | `Update` | tick `DespawnAfter` timers (debris cleanup) |
//!
//! The impulse→damage read mirrors `src/vehicle/impact.rs`: contact normal
//! impulse over the body's own mass is the Δv the hit cost, and only Δv past a
//! threshold does anything — so setting the crate down, bumping it, or it
//! resting on the floor (a tiny support impulse every step) are all free.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::classes::{Interactable, ItemPickup, PropWoodCrate};
use crate::config;
use crate::items;

/// On each wooden crate: remaining health. `damage_on_impact` subtracts from
/// it; `shatter` fires at `<= 0`.
#[derive(Component)]
pub struct Breakable {
    pub health: f32,
}

/// Despawn this entity when the timer finishes. Debris cubes carry one.
#[derive(Component)]
pub struct DespawnAfter(pub Timer);

/// Shared wood material and debris mesh, built once (the `carry::CrateAssets`
/// pattern). The crate mesh is per-entity — sized from `PropWoodCrate::size`.
#[derive(Resource)]
pub struct BreakableAssets {
    wood: Handle<StandardMaterial>,
    debris_mesh: Handle<Mesh>,
}

pub fn setup_breakable_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(BreakableAssets {
        wood: materials.add(StandardMaterial {
            base_color: config::WOOD_COLOR,
            perceptual_roughness: 0.85,
            ..default()
        }),
        debris_mesh: meshes.add(Cuboid::new(
            config::DEBRIS_SIZE,
            config::DEBRIS_SIZE,
            config::DEBRIS_SIZE,
        )),
    });
}

/// `PropWoodCrate` from the map → a real dynamic crate. Same body/friction/
/// damping shape as `carry::spawn_crates`, plus `Breakable`.
pub fn spawn_wood_crates(
    add: On<Add, PropWoodCrate>,
    crates: Query<&PropWoodCrate>,
    assets: Res<BreakableAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let Ok(wc) = crates.get(add.entity) else {
        return;
    };
    let s = wc.size;
    commands.entity(add.entity).insert((
        Mesh3d(meshes.add(wood_crate_mesh(s))),
        MeshMaterial3d(assets.wood.clone()),
        RigidBody::Dynamic,
        Collider::cuboid(s, s, s),
        Mass(wc.mass),
        Friction::new(config::CRATE_FRICTION).with_combine_rule(CoefficientCombine::Max),
        LinearDamping(config::CRATE_LINEAR_DAMPING),
        AngularDamping(config::CRATE_ANGULAR_DAMPING),
        Breakable { health: wc.health },
        Name::new("PropWoodCrate"),
    ));
}

/// Damage a hit of `delta_v` (m/s) deals: linear past `BREAK_MIN_DELTA_V`,
/// nothing below it. Pure — unit-tested.
pub fn damage_from_delta_v(delta_v: f32) -> f32 {
    (delta_v - config::BREAK_MIN_DELTA_V).max(0.0) * config::BREAK_DAMAGE_PER_DELTA_V
}

/// Reads last step's contact impulses off Avian's contact graph and chips
/// health off every `Breakable` in a hard enough manifold. One physics tick
/// stale, which nobody can tell (the `vehicle::impact` note).
pub fn damage_on_impact(
    collisions: Collisions,
    mut breakables: Query<(&mut Breakable, &ComputedMass)>,
) {
    for pair in collisions.iter() {
        for entity in [pair.collider1, pair.collider2] {
            let Ok((mut breakable, mass)) = breakables.get_mut(entity) else {
                continue;
            };
            if breakable.health <= 0.0 {
                continue;
            }
            let impulse: f32 = pair
                .manifolds
                .iter()
                .map(|m| m.total_normal_impulse())
                .sum();
            if impulse <= 0.0 {
                continue;
            }
            let delta_v = impulse / mass.value().max(1.0e-3);
            let damage = damage_from_delta_v(delta_v);
            if damage > 0.0 {
                breakable.health -= damage;
            }
        }
    }
}

/// `health <= 0` → throw debris, drop the contained item, despawn the crate.
pub fn shatter(
    crates: Query<(Entity, &Breakable, &GlobalTransform, &PropWoodCrate)>,
    assets: Res<BreakableAssets>,
    mut commands: Commands,
) {
    for (entity, breakable, global, wc) in &crates {
        if breakable.health > 0.0 {
            continue;
        }
        let center = global.translation();
        let base_y = center.y - wc.size * 0.5;

        for i in 0..config::DEBRIS_COUNT {
            let a = i as f32 / config::DEBRIS_COUNT as f32 * std::f32::consts::TAU;
            let dir = Vec3::new(a.cos(), 0.7, a.sin()).normalize();
            commands.spawn((
                Name::new("Debris"),
                Mesh3d(assets.debris_mesh.clone()),
                MeshMaterial3d(assets.wood.clone()),
                RigidBody::Dynamic,
                Collider::cuboid(
                    config::DEBRIS_SIZE,
                    config::DEBRIS_SIZE,
                    config::DEBRIS_SIZE,
                ),
                Mass(0.4),
                Transform::from_translation(center + dir * (wc.size * 0.3)),
                LinearVelocity(dir * config::DEBRIS_SPEED),
                AngularVelocity(Vec3::new(a, a * 1.7, a * 0.5)),
                DespawnAfter(Timer::from_seconds(
                    config::DEBRIS_LIFETIME,
                    TimerMode::Once,
                )),
            ));
        }

        // The contained item, resting just above where the crate's floor was.
        // `pickup::spawn_visuals` (On<Add, ItemPickup>) fills in the model,
        // collider and prompt.
        commands.spawn((
            ItemPickup {
                item: wc.contains.clone(),
            },
            Interactable {
                prompt: items::prompt_for(&wc.contains),
            },
            Transform::from_translation(Vec3::new(center.x, base_y + 0.06, center.z)),
        ));

        commands.entity(entity).despawn();
    }
}

pub fn despawn_after(
    time: Res<Time>,
    mut timers: Query<(Entity, &mut DespawnAfter)>,
    mut commands: Commands,
) {
    for (entity, mut after) in &mut timers {
        if after.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// A wooden crate: an inset body slab plus a raised plank frame on every edge,
/// merged into one mesh (the `ladder::ladder_mesh` idiom). Centred on the
/// origin, extents `size` on every axis.
fn wood_crate_mesh(size: f32) -> Mesh {
    let h = size * 0.5;
    let plank = size * 0.12; // edge-frame plank cross-section
    let inset = size - plank; // body slab, recessed behind the frame
    let off = h - plank * 0.5; // frame-bar centre, so the bars sit flush, not proud

    let mut mesh = Mesh::from(Cuboid::new(inset, inset, inset));

    // Twelve edge planks, one per cube edge: for each axis, four bars running
    // the full length along it at the four corners of the other two axes.
    let bar_long = Cuboid::new(size, plank, plank); // along X
    let bar_up = Cuboid::new(plank, size, plank); // along Y
    let bar_deep = Cuboid::new(plank, plank, size); // along Z
    for &(sy, sz) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        mesh.merge(&Mesh::from(bar_long).translated_by(Vec3::new(0.0, sy * off, sz * off)))
            .unwrap();
    }
    for &(sx, sz) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        mesh.merge(&Mesh::from(bar_up).translated_by(Vec3::new(sx * off, 0.0, sz * off)))
            .unwrap();
    }
    for &(sx, sy) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        mesh.merge(&Mesh::from(bar_deep).translated_by(Vec3::new(sx * off, sy * off, 0.0)))
            .unwrap();
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::primitives::MeshAabb;

    #[test]
    fn a_gentle_bump_does_no_damage() {
        assert_eq!(damage_from_delta_v(0.0), 0.0);
        assert_eq!(damage_from_delta_v(config::BREAK_MIN_DELTA_V), 0.0);
        assert_eq!(damage_from_delta_v(config::BREAK_MIN_DELTA_V - 2.0), 0.0);
    }

    #[test]
    fn a_platform_b_drop_shatters_a_full_health_crate_in_one_hit() {
        // deck top 4.88 m, ahoy gravity 29.0 → v = sqrt(2·g·h) at the floor.
        let impact_speed = (2.0 * 29.0 * 4.88_f32).sqrt();
        assert!(
            damage_from_delta_v(impact_speed) >= config::WOOD_CRATE_HEALTH,
            "a full drop ({impact_speed:.1} m/s) should one-shot the crate"
        );
    }

    #[test]
    fn a_crowbar_takes_more_than_one_swing_but_not_many() {
        assert!(config::CROWBAR_DAMAGE < config::WOOD_CRATE_HEALTH);
        assert!(config::CROWBAR_DAMAGE * 3.0 >= config::WOOD_CRATE_HEALTH);
    }

    #[test]
    fn crate_mesh_fills_its_declared_size() {
        let aabb = wood_crate_mesh(0.6).compute_aabb().expect("positions");
        let size = Vec3::from(aabb.half_extents) * 2.0;
        assert!(
            size.abs_diff_eq(Vec3::splat(0.6), 1.0e-4),
            "crate mesh size {size:?} != 0.6³"
        );
    }
}
