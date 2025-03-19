// Features a very basic Jump mechanic. No physics engine used, just a parabola.

use bevy::prelude::*;

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
        .add_systems(Update, handle_jumping_state)
        .add_systems(Update, update_player_velocity)
        .add_systems(Update, update_player_transform_and_handle_ground_collision)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    window: Query<&Window>,
) {
    let window = window.single();
    let width = window.resolution.width();

    commands.spawn(Camera2d);

    commands.spawn((
        Player,
        JumpingState {
            state: JumpingStates::Idle,
            jump_started_at: 0.0,
            current_velocity: 0.0,
        },
        Mesh2d(meshes.add(Rectangle::new(100.0, 100.0))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
        Transform::from_xyz(-width / 2.0 + 50.0, 0.0, 0.0),
    ));
}

fn handle_jumping_state(
    mut query: Query<&mut JumpingState, With<Player>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    if keyboard.just_pressed(KeyCode::Space) {
        for mut jumping_state in &mut query {
            match jumping_state.state {
                JumpingStates::Idle => {
                    // println!("jump!");
                    jumping_state.state = JumpingStates::Airborne;
                    jumping_state.jump_started_at = time.elapsed_secs();
                }
                _ => {}
            }
        }
    }
}

fn update_player_velocity(mut query: Query<&mut JumpingState, With<Player>>, time: Res<Time>) {
    let tt = time.elapsed_secs();

    for mut jumping_state in &mut query {
        match jumping_state.state {
            JumpingStates::Airborne => {
                // println!("airborne!");
                let x = tt - jumping_state.jump_started_at;

                let ax2 = -50.0 * (x - 0.25).powi(2);
                let bx = 2.0 * x;
                let c = 3.0 - x;

                jumping_state.current_velocity = ax2 + bx + c;
            }

            _ => {}
        }
    }
}

fn update_player_transform_and_handle_ground_collision(
    mut query: Query<(&mut Transform, &mut JumpingState), With<Player>>,
) {
    for (mut transform, mut jumping_state) in &mut query {
        // Touching the ground; we reset velocity and state.
        if transform.translation.y < 0.0 {
            jumping_state.current_velocity = 0.0;
            jumping_state.state = JumpingStates::Idle;
            jumping_state.jump_started_at = 0.0;
            transform.translation.y = 0.0;
        }

        transform.translation.y = jumping_state.current_velocity * 100.0;

        // println!("y = {:?}", transform.translation.y);
    }
}
