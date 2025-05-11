use bevy::prelude::*;

const PLAYER_SIZE: f32 = 24.0;
const SHIP_SPEED_FACTOR: f32 = 2.0;
const MISSILE_SPEED_FACTOR: f32 = 8.0;

#[derive(Component)]
struct Moving {
    speed: f32,
    acceleration: f32,
    speed_factor: f32,
    rotation_factor: f32,
}

#[derive(Component)]
struct Asteroid;

#[derive(Component)]
struct Ship;

#[derive(Component)]
struct Missile;

#[derive(Event)]
struct FireEvent;

#[derive(Resource, Deref)]
struct FireSound(Handle<AudioSource>);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_event::<FireEvent>()
        .add_systems(Startup, setup)
        // .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(
            FixedUpdate,
            (
                apply_acceleration,
                move_moving_objects,
                rotate_and_accelerate_ship,
            ),
        )
        .add_systems(Update, (fire_missile, play_fire_sound))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: Res<AssetServer>,
) {
    commands.spawn(Camera2d);

    let fire_sound = asset_server.load("sfx/jsfxr/click.wav");
    commands.insert_resource(FireSound(fire_sound));

    commands.spawn((
        Ship,
        Moving {
            speed: 0.0,
            acceleration: 0.0,
            speed_factor: SHIP_SPEED_FACTOR,
            rotation_factor: 0.0,
        },
        Transform::from_translation(Vec3::ZERO),
        Mesh2d(meshes.add(Triangle2d::new(
            Vec2::Y * PLAYER_SIZE * 0.5,
            Vec2::new(-PLAYER_SIZE * 0.5, -PLAYER_SIZE * 0.125),
            Vec2::new(PLAYER_SIZE * 0.5, -PLAYER_SIZE * 0.125),
        ))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
    ));

    // commands.spawn((
    //     Asteroid,
    //     Transform::from_xyz(0.0, 0.0, 0.0),
    //     Mesh2d(meshes.add(Circle::new(1.0))),
    //     MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
    // ));
}

fn apply_acceleration(mut query: Query<&mut Moving>, time: Res<Time>) {
    let dt = time.delta_secs();

    for mut moving in query.iter_mut() {
        moving.speed = moving.speed + moving.acceleration * dt;
    }
}

fn move_moving_objects(mut query: Query<(&mut Transform, &Moving)>, time: Res<Time>) {
    let dt = time.delta_secs();

    for (mut transform, moving) in query.iter_mut() {
        transform.rotate_z(moving.rotation_factor * f32::to_radians(360.0) * dt);

        let movement_direction = transform.rotation * Vec3::Y;
        let movement_distance = moving.speed * moving.speed_factor * dt;
        let translation_delta = movement_direction * movement_distance;
        transform.translation += translation_delta;
    }
}

// fn inflate(mut circle_q: Query<&mut Transform, With<Asteroid>>, time: Res<Time>) {
//     let elapsed = time.elapsed_secs();

//     for mut transform in circle_q.iter_mut() {
//         transform.scale.x = 1.0 + transform.scale.x * elapsed * 0.01;
//         transform.scale.y = 1.0 + transform.scale.y * elapsed * 0.01;
//     }
// }

fn rotate_and_accelerate_ship(
    ship_q: Single<&mut Moving, With<Ship>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    // time: Res<Time>,
) {
    let mut moving = ship_q.into_inner();
    // let dt = time.delta_secs();

    println!("{:?}", moving.acceleration);

    if keyboard.pressed(KeyCode::ArrowLeft) {
        moving.rotation_factor = 1.0;
    } else if keyboard.pressed(KeyCode::ArrowRight) {
        moving.rotation_factor = -1.0;
    } else {
        moving.rotation_factor = 0.0;
    }

    if keyboard.pressed(KeyCode::ArrowUp) {
        // if moving.acceleration < 2.0 {
        moving.acceleration = moving.acceleration + 0.2;
        // }
    }

    if keyboard.pressed(KeyCode::ArrowDown) {
        // if moving.acceleration < 0.0 {
        //     moving.acceleration = 0.0;
        // } else {
        moving.acceleration = moving.acceleration - 0.2;
        // }
    }
}

fn fire_missile(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    ship_q: Single<&Transform, With<Ship>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut events: EventWriter<FireEvent>,
) {
    let (transform) = ship_q.into_inner();

    if keyboard.just_pressed(KeyCode::Space) {
        let movement_direction = transform.rotation * Vec3::Y;
        let movement_distance = PLAYER_SIZE;
        let translation_delta = movement_direction * movement_distance;
        let missile_translation = transform.translation + translation_delta;

        commands.spawn((
            Missile,
            Moving {
                speed: 2.0,
                acceleration: 0.0,
                speed_factor: MISSILE_SPEED_FACTOR,
                rotation_factor: 0.0,
            },
            Transform {
                translation: missile_translation,
                rotation: transform.rotation,
                scale: Vec3::ONE,
            },
            Mesh2d(meshes.add(Rectangle::new(4.0, 12.0))),
            MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
        ));

        events.send(FireEvent);
    }
}

fn play_fire_sound(
    mut commands: Commands,
    mut events: EventReader<FireEvent>,
    sound: Res<FireSound>,
) {
    if !events.is_empty() {
        events.clear();
        commands.spawn((AudioPlayer(sound.clone()), PlaybackSettings::DESPAWN));
    }
}
