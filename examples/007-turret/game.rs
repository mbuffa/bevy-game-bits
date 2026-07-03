use bevy::prelude::*;

use crate::config::*;
use crate::enemy::{self, SimpleRng, WaveTimer};
use crate::{placement, scene, turret, weapons};

/// Damage requests, decoupling the weapons that deal damage from the
/// enemy systems that apply it (and trigger the hit flash).
#[derive(Message)]
pub struct DamageMessage {
    pub target: Entity,
    pub amount: f32,
}

/// Shared mesh/material handles, created once at startup. Enemies all share
/// one material handle; the hit flash swaps the component to `flash_material`
/// instead of mutating the asset, so flashing one enemy never affects others.
#[derive(Resource)]
pub struct GameAssets {
    pub enemy_mesh: Handle<Mesh>,
    pub enemy_material: Handle<StandardMaterial>,
    pub flash_material: Handle<StandardMaterial>,
    pub leg_mesh: Handle<Mesh>,
    pub leg_material: Handle<StandardMaterial>,
    pub hub_mesh: Handle<Mesh>,
    pub kinetic_base_material: Handle<StandardMaterial>,
    pub laser_base_material: Handle<StandardMaterial>,
    pub rock_mesh: Handle<Mesh>,
    pub rock_material_a: Handle<StandardMaterial>,
    pub rock_material_b: Handle<StandardMaterial>,
    pub barrel_mesh: Handle<Mesh>,
    pub barrel_material: Handle<StandardMaterial>,
    pub sensor_mesh: Handle<Mesh>,
    pub sensor_material: Handle<StandardMaterial>,
    pub projectile_mesh: Handle<Mesh>,
    pub projectile_material: Handle<StandardMaterial>,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamageMessage>()
            .insert_resource(WaveTimer(Timer::from_seconds(
                WAVE_INTERVAL,
                TimerMode::Repeating,
            )))
            .insert_resource(SimpleRng::from_time())
            .add_systems(Startup, (setup_assets, scene::setup).chain())
            .add_systems(
                Update,
                (
                    enemy::spawn_waves,
                    turret::sweep_sensors,
                    turret::acquire_and_validate_targets,
                    turret::aim_guns,
                    weapons::fire_kinetic,
                    weapons::fire_lasers,
                    enemy::apply_damage,
                    enemy::update_hit_flash,
                    enemy::despawn_dead,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    placement::place_turret_on_click,
                    turret::draw_sensor_cones,
                    weapons::draw_laser_beams,
                ),
            )
            .add_systems(
                FixedUpdate,
                (
                    enemy::move_enemies,
                    weapons::move_projectiles,
                    weapons::collide_projectiles,
                )
                    .chain(),
            );
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(GameAssets {
        enemy_mesh: meshes.add(Cuboid::new(ENEMY_SIZE.x, ENEMY_SIZE.y, ENEMY_SIZE.z)),
        enemy_material: materials.add(StandardMaterial {
            base_color: ENEMY_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        // Unlit so the flash reads as a pure white blink regardless of lighting.
        flash_material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        }),
        leg_mesh: meshes.add(Cuboid::new(
            TRIPOD_LEG_THICKNESS,
            (TRIPOD_APEX_HEIGHT * TRIPOD_APEX_HEIGHT + TRIPOD_FOOT_RADIUS * TRIPOD_FOOT_RADIUS)
                .sqrt(),
            TRIPOD_LEG_THICKNESS,
        )),
        leg_material: materials.add(StandardMaterial {
            base_color: LEG_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        hub_mesh: meshes.add(Sphere::new(0.3)),
        kinetic_base_material: materials.add(StandardMaterial {
            base_color: KINETIC_BASE_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        laser_base_material: materials.add(StandardMaterial {
            base_color: LASER_BASE_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        rock_mesh: meshes.add(Sphere::new(1.0)),
        rock_material_a: materials.add(StandardMaterial {
            base_color: ROCK_COLOR_A,
            perceptual_roughness: 1.0,
            ..default()
        }),
        rock_material_b: materials.add(StandardMaterial {
            base_color: ROCK_COLOR_B,
            perceptual_roughness: 1.0,
            ..default()
        }),
        barrel_mesh: meshes.add(Cuboid::new(0.25, 0.25, BARREL_LENGTH)),
        barrel_material: materials.add(StandardMaterial {
            base_color: BARREL_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        sensor_mesh: meshes.add(Sphere::new(0.25)),
        sensor_material: materials.add(StandardMaterial {
            base_color: SENSOR_COLOR,
            perceptual_roughness: 0.4,
            ..default()
        }),
        projectile_mesh: meshes.add(Sphere::new(PROJECTILE_RADIUS)),
        projectile_material: materials.add(StandardMaterial {
            base_color: PROJECTILE_COLOR,
            unlit: true,
            ..default()
        }),
    });
}
