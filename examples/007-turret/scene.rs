use bevy::camera::ScalingMode;
use bevy::prelude::*;
use std::f32::consts::TAU;

use crate::config::*;
use crate::enemy::SimpleRng;
use crate::game::GameAssets;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<GameAssets>,
    mut rng: ResMut<SimpleRng>,
) {
    // Orthographic camera in an angled 3/4 view (45 degrees over the field).
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: FIELD_WIDTH + 4.0,
                min_height: FIELD_DEPTH + 4.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 26.0, 26.0).looking_at(Vec3::ZERO, Vec3::Y),
        AmbientLight {
            brightness: 250.0,
            ..default()
        },
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, -0.6, -1.0)),
    ));

    // Ground slab; its top face sits at y = 0.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(FIELD_WIDTH, 0.1, FIELD_DEPTH))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: GROUND_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.05, 0.0),
    ));

    // Decorative rocks: squashed spheres in two darker shades of the sand.
    for i in 0..ROCK_COUNT {
        let x = rng.range_f32(-FIELD_WIDTH / 2.0 + 1.0, FIELD_WIDTH / 2.0 - 1.0);
        let z = rng.range_f32(-FIELD_DEPTH / 2.0 + 1.0, FIELD_DEPTH / 2.0 - 1.0);
        let scale = rng.range_f32(0.3, 1.1);
        let yaw = rng.range_f32(0.0, TAU);
        let material = if i % 2 == 0 {
            assets.rock_material_a.clone()
        } else {
            assets.rock_material_b.clone()
        };

        commands.spawn((
            Mesh3d(assets.rock_mesh.clone()),
            MeshMaterial3d(material),
            Transform {
                translation: Vec3::new(x, 0.0, z),
                rotation: Quat::from_rotation_y(yaw),
                scale: Vec3::new(scale, scale * 0.35, scale * 0.8),
            },
        ));
    }

    commands.spawn((
        Text::new("LMB: kinetic turret (20)  |  RMB: laser turret (40)  |  Space: start wave  |  R: restart"),
        TextFont::from_font_size(16.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
    ));
}
