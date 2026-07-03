use bevy::camera::ScalingMode;
use bevy::prelude::*;

use crate::config::*;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.05, 0.0),
    ));

    commands.spawn((
        Text::new("LMB: kinetic turret  |  RMB: laser turret"),
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
