// Features a very basic Jump mechanic. No physics engine used, just a parabola.

use bevy::prelude::*;

const SCREEN_UNIT: f32 = 22.0;

enum JumpingStates {
    Idle,
    Airborne,
}

#[derive(Component)]
struct Player;

#[derive(Component)]
struct JumpingState {
    state: JumpingStates,
    jump_started_at: f32,
    current_velocity: f32,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_jumping_state,
                update_player_velocity,
                update_player_transform,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    commands.spawn((
        Player,
        JumpingState {
            state: JumpingStates::Idle,
            jump_started_at: 0.0,
            current_velocity: 0.0,
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
        Mesh2d(meshes.add(Rectangle::new(100.0, 100.0))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
    ));
}

fn handle_jumping_state(
    mut query: Query<&mut JumpingState, With<Player>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    if query.is_empty() {
        return;
    }

    if keyboard.just_pressed(KeyCode::Space) {
        let mut jumping_state = query.single_mut();

        match jumping_state.state {
            JumpingStates::Idle => {
                jumping_state.state = JumpingStates::Airborne;
                jumping_state.jump_started_at = time.elapsed_secs();
            }
            _ => {}
        }
    }
}

fn update_player_velocity(mut query: Query<&mut JumpingState, With<Player>>, time: Res<Time>) {
    if query.is_empty() {
        return;
    }

    let tt = time.elapsed_secs();

    let mut jumping_state = query.single_mut();

    if jumping_state.current_velocity < 0.0 {
        jumping_state.state = JumpingStates::Idle;
    }

    match jumping_state.state {
        JumpingStates::Airborne => {
            let x: f32 = tt - jumping_state.jump_started_at;
            // h\ +\ v\cdot x-\frac{1}{2}\cdot g\cdot x^{2}
            // h + v * x - 1/2 g * x²
            let y: f32 = 0.0 + (70.0 * x) - 0.5 * 160.0 * x.powi(2);
            jumping_state.current_velocity = y;
        }

        _ => {
            jumping_state.current_velocity = 0.0;
            jumping_state.jump_started_at = 0.0;
        }
    }
}

fn update_player_transform(mut query: Query<(&mut Transform, &JumpingState), With<Player>>) {
    if query.is_empty() {
        return;
    }

    let (mut transform, jumping_state) = query.single_mut();

    if transform.translation.y < 0.0 {
        transform.translation.y = 0.0;
    } else {
        transform.translation.y = jumping_state.current_velocity * SCREEN_UNIT;
    }
}
