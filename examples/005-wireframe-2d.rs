use bevy::{
    prelude::*,
    sprite::{Wireframe2d, Wireframe2dColor, Wireframe2dPlugin},
};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, Wireframe2dPlugin::default()))
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    // Native Wireframe, for debugging:
    commands.spawn((
        Transform::default(),
        Mesh2d(meshes.add(Rectangle::new(40.0, 40.0))),
        MeshMaterial2d(materials.add(Color::BLACK)),
        Wireframe2d,
        Wireframe2dColor {
            color: Color::WHITE.into(),
        },
    ));

    // "Homemade" wireframe, with two meshes overlapping:
    commands
        .spawn((
            Transform::from_xyz(50.0, 50.0, 0.0),
            Mesh2d(meshes.add(Rectangle::new(40.0, 40.0))),
            MeshMaterial2d(materials.add(Color::WHITE)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Transform::from_xyz(0.0, 0.0, 1.0),
                Mesh2d(meshes.add(Rectangle::new(38.0, 38.0))),
                MeshMaterial2d(materials.add(Color::BLACK)),
            ));
        });
}
