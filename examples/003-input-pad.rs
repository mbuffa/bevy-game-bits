// Features a simple WASD keypad turning red when an input is pressed,
// similar to software used for speedrunning.

use bevy::{
    color::palettes::css::*,
    prelude::*,
    text::{LineBreak, TextBounds},
};

#[derive(Resource)]
struct MyMaterials {
    idle: Option<Handle<ColorMaterial>>,
    pressed: Option<Handle<ColorMaterial>>,
}

#[derive(Component)]
struct Key {
    code: KeyCode,
}

const FONT_SIZE: f32 = 16.0;
const KEY_SIZE: f32 = 32.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(MyMaterials {
            idle: None,
            pressed: None,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, is_key_pressed)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut my_materials: ResMut<MyMaterials>,
    asset_server: Res<AssetServer>,
) {
    let font = asset_server.load("fonts/FiraSans-Bold.ttf");
    let text_font = TextFont {
        font: font.clone(),
        font_size: FONT_SIZE,
        ..default()
    };

    let idle_material = materials.add(Color::srgb(1.0, 1.0, 1.0));
    my_materials.idle = Some(materials.add(Color::srgb(1.0, 1.0, 1.0)));
    my_materials.pressed = Some(materials.add(Color::srgb(1.0, 0.0, 0.0)));

    commands.spawn(Camera2d);

    let key_size = Vec2::new(KEY_SIZE, KEY_SIZE);

    for (key_code, label, (x, y, z)) in [
        (KeyCode::KeyW, "W", (0.0, KEY_SIZE, 0.0)),
        (KeyCode::KeyA, "A", (-KEY_SIZE, 0.0, 0.0)),
        (KeyCode::KeyS, "S", (0.0, 0.0, 0.0)),
        (KeyCode::KeyD, "D", (KEY_SIZE, 0.0, 0.0)),
    ] {
        commands
            .spawn((
                Key { code: key_code },
                Transform::from_xyz(x, y, z),
                Mesh2d(meshes.add(Rectangle::from_size(key_size))),
                MeshMaterial2d(idle_material.clone()),
            ))
            .with_children(|builder| {
                builder.spawn((
                    Text2d::new(label),
                    text_font.clone(),
                    TextColor(Color::Srgba(BLACK)),
                    TextLayout::new(JustifyText::Left, LineBreak::AnyCharacter),
                    TextBounds::from(key_size),
                    Transform::from_translation(Vec3::Z),
                ));
            });
    }
}

// Keeping the function below as a reminder on how to pass Bevy's "resources" around in functions:
// fn maybe_switch_material(
//     code: KeyCode,
//     keyboard: &ButtonInput<KeyCode>,
//     my_materials: &MyMaterials,
//     material: &mut MeshMaterial2d<ColorMaterial>,
// ) {
//     ...
// }

fn is_key_pressed(
    mut query: Query<(&mut Key, &mut MeshMaterial2d<ColorMaterial>)>,
    keyboard: Res<ButtonInput<KeyCode>>,
    my_materials: Res<MyMaterials>,
) {
    if my_materials.idle == None || my_materials.pressed == None {
        panic!("Materials not initialized!");
    }

    for (key, mut material) in query.iter_mut() {
        // See above :)
        // maybe_switch_material(
        //     key.code,
        //     keyboard.as_ref(),
        //     my_materials.as_ref(),
        //     material.as_mut(),
        // );

        // Hopefully, we're just cloning the handles here, not the materials themselves.
        // I'm not sure how to check though, maybe with one of the bevy GUIs!
        if keyboard.just_pressed(key.code) {
            material.0 = my_materials.pressed.as_ref().unwrap().clone();
        }

        if keyboard.just_released(key.code) {
            material.0 = my_materials.idle.as_ref().unwrap().clone();
        }
    }
}
