//! Arena props: pushable dynamic obstacles scattered around the floor.
//! "Static" in the sense of no behavior — but everything is a rigid body,
//! so light crates fly when rammed, balls roll away, and the heavy blocks
//! shrug and barely move. Layout is hand-placed (tables below), clear of
//! the arena furniture: the center pillar, the ramp pinwheel on the axes
//! at radius ~19, the potholes (arena.rs), and the spawn ring at 40.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::config::*;
use bevy_game_bits::vehicle::SoftProp;
use crate::powerups::{CrateBroken, CrateOrigin, LootCrate};

/// Light wooden crates: a loose mid-field cluster, begging to be
/// scattered. (x, z)
const CRATES: [(f32, f32); 6] = [
    (-5.2, 33.8),
    (2.0, 36.4),
    (-1.3, 40.3),
    (5.2, 33.1),
    (8.0, 38.0),
    (-8.0, 36.0),
];

/// Heavy concrete blocks: mid-ring hazards. (x, z)
const BLOCKS: [(f32, f32); 4] = [(-31.2, 15.6), (28.6, -23.4), (13.0, -36.4), (-13.0, -37.0)];

/// Balls: scattered rollers. (x, z)
const BALLS: [(f32, f32); 6] = [
    (-18.2, -5.2),
    (15.6, 28.6),
    (-26.0, -33.8),
    (33.8, 7.8),
    (0.0, -45.0),
    (45.0, 20.0),
];

/// Shared mesh/material for crates, kept alive so a broken crate can be
/// rebuilt in place — `respawn_crates` reuses these instead of re-adding
/// duplicate GPU assets every time a crate returns.
#[derive(Resource)]
pub struct CrateAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// One crate bundle, identical whether it's part of the initial layout or
/// a respawn — `LootCrate` marks it breakable (`powerups::break_crates`),
/// `CrateOrigin` is where it returns to if it breaks.
fn spawn_crate(commands: &mut Commands, assets: &CrateAssets, x: f32, z: f32) {
    commands.spawn((
        Name::new("Crate"),
        LootCrate,
        CrateOrigin(Vec3::new(x, 0.0, z)),
        RigidBody::Dynamic,
        Collider::cuboid(CRATE_SIZE, CRATE_SIZE, CRATE_SIZE),
        Mass(CRATE_MASS),
        Friction::new(OBSTACLE_FRICTION),
        Restitution::new(BOX_RESTITUTION),
        AngularDamping(OBSTACLE_ANGULAR_DAMPING),
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.material.clone()),
        Transform::from_xyz(x, CRATE_SIZE / 2.0 + 0.1, z),
    ));
}

pub fn setup_obstacles(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let crate_assets = CrateAssets {
        mesh: meshes.add(Cuboid::new(CRATE_SIZE, CRATE_SIZE, CRATE_SIZE)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.65, 0.45, 0.2),
            perceptual_roughness: 0.9,
            ..default()
        }),
    };
    for &(x, z) in CRATES.iter() {
        spawn_crate(&mut commands, &crate_assets, x, z);
    }
    commands.insert_resource(crate_assets);

    let block_mesh = meshes.add(Cuboid::new(BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE));
    let block_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.55, 0.58),
        perceptual_roughness: 0.95,
        ..default()
    });
    let ball_mesh = meshes.add(Sphere::new(BALL_RADIUS));
    let ball_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.75, 0.15, 0.15),
        perceptual_roughness: 0.4,
        ..default()
    });

    for &(x, z) in BLOCKS.iter() {
        commands.spawn((
            Name::new("Block"),
            RigidBody::Dynamic,
            Collider::cuboid(BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE),
            Mass(BLOCK_MASS),
            Friction::new(OBSTACLE_FRICTION),
            Restitution::new(BOX_RESTITUTION),
            AngularDamping(OBSTACLE_ANGULAR_DAMPING),
            Mesh3d(block_mesh.clone()),
            MeshMaterial3d(block_material.clone()),
            Transform::from_xyz(x, BLOCK_SIZE / 2.0 + 0.1, z),
        ));
    }
    for &(x, z) in BALLS.iter() {
        commands.spawn((
            Name::new("Ball"),
            RigidBody::Dynamic,
            Collider::sphere(BALL_RADIUS),
            Mass(BALL_MASS),
            Friction::new(BALL_FRICTION),
            // `Min` outranks the chassis's default `Average` combine rule
            // (avian's priority is Max > Multiply > Min > GeometricMean >
            // Average), so a car-vs-ball contact uses 0.05 instead of the
            // 0.2 the old 0.3/Average pairing gave — no more springing off
            // the nose when shoved.
            Restitution::new(BALL_RESTITUTION).with_combine_rule(CoefficientCombine::Min),
            AngularDamping(BALL_ANGULAR_DAMPING),
            LinearDamping(BALL_LINEAR_DAMPING),
            // Soft when parked, full-strength when punted — see
            // damage.rs's `prop_softness`.
            SoftProp,
            Mesh3d(ball_mesh.clone()),
            MeshMaterial3d(ball_material.clone()),
            Transform::from_xyz(x, BALL_RADIUS + 0.1, z),
        ));
    }
}

/// Brings a broken crate back at its original spot after
/// `CRATE_RESPAWN_DELAY`. Pending respawns live in a plain `Local` list
/// (the same lightweight pattern `ui.rs`'s HUD accel smoothing uses for
/// per-call state) rather than marker entities — there's nothing to render
/// or collide with while the timer runs.
pub fn respawn_crates(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<CrateAssets>,
    mut broken: MessageReader<CrateBroken>,
    mut pending: Local<Vec<(Vec3, Timer)>>,
) {
    for event in broken.read() {
        pending.push((event.origin, Timer::from_seconds(CRATE_RESPAWN_DELAY, TimerMode::Once)));
    }
    pending.retain_mut(|(origin, timer)| {
        if timer.tick(time.delta()).is_finished() {
            spawn_crate(&mut commands, &assets, origin.x, origin.z);
            false
        } else {
            true
        }
    });
}
