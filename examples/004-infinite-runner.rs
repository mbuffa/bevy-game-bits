// Features an Infinite Runner prototype

// TODO:
// * Add instructions on the InsertCOin state.
// * Fix the cvrash when quitting the game and it's on Play State
//  * Occurs because some systems run with no window registered.
// * Check for a better way to handle state transitions
// * Check for a better way to handle entities spawn
// * Declare a Jump or Player Plugin
// * Run all physics-related systems on the FixedUpdate scheduler
// * Add progressive difficulty
// * Add some parallax background

use bevy::math::bounding::{Aabb2d, IntersectsVolume};
use bevy::prelude::*;

const SCREEN_UNIT: f32 = 22.0;
const PLAYER_WIDTH: f32 = 24.0;
const PLAYER_HEIGHT: f32 = 72.0;
const SCOREBOARD_TEXT_PADDING: Val = Val::Px(4.0);
const SCOREBOARD_FONT_SIZE: f32 = 16.0;

#[derive(Copy, Clone, Eq, PartialEq)]
enum GameStates {
    InsertCoin,
    Play,
    GameOver,
}

#[derive(Eq, PartialEq, Resource)]
struct GameState(GameStates);

#[derive(Event)]
struct GameStateEvent {
    from: GameStates,
    to: GameStates,
}

#[derive(Event, Default)]
struct CollisionEvent;

#[derive(Resource)]
struct Score(u32);

#[derive(Component)]
struct ScoreText;

enum JumpingStates {
    Idle,
    Airborne,
}

#[derive(Component)]
struct JumpingState {
    state: JumpingStates,
    jump_started_at: f32,
    current_velocity: f32,
}

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Obstacle;

#[derive(Component)]
struct GameOverText;

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0)))
        .add_event::<GameStateEvent>()
        .add_event::<CollisionEvent>()
        .insert_resource(Score(0))
        .insert_resource(GameState(GameStates::InsertCoin))
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, (setup, spawn_obstacles))
        .add_systems(
            Update,
            maybe_transit_to_play.run_if(resource_equals(GameState(GameStates::InsertCoin))),
        )
        .add_systems(
            Update,
            (
                handle_jumping_state,
                update_player_velocity,
                update_player_transform,
            )
                .run_if(resource_equals(GameState(GameStates::Play))),
        )
        .add_systems(
            Update,
            (
                spawn_entities,
                hide_game_over_text,
                move_obstacles,
                update_score_text,
                detect_collisions,
                maybe_transit_to_game_over,
            )
                .chain()
                .run_if(resource_equals(GameState(GameStates::Play))),
        )
        .add_systems(
            Update,
            (
                despawn_entities,
                display_game_over_text,
                maybe_transit_to_play,
            )
                .run_if(resource_equals(GameState(GameStates::GameOver))),
        )
        .run();
}

fn setup(
    mut window: Single<&mut Window>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    window.resolution.set(800.0, 600.0);

    // Camera
    commands.spawn(Camera2d);

    // Ground
    commands.spawn((
        Transform::from_xyz(0.0, 0.0 - (PLAYER_HEIGHT / 2.0), 0.0),
        Mesh2d(meshes.add(Rectangle::new(window.resolution.width(), 1.0))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
    ));

    // Score
    commands.spawn((
        Text::new("0"),
        TextFont {
            font_size: SCOREBOARD_FONT_SIZE,
            ..default()
        },
        TextColor(Color::WHITE),
        ScoreText,
        Node {
            position_type: PositionType::Absolute,
            top: SCOREBOARD_TEXT_PADDING,
            right: SCOREBOARD_TEXT_PADDING,
            ..default()
        },
    ));
}

fn spawn_entities(
    mut commands: Commands,
    window: Single<&mut Window>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut events: EventReader<GameStateEvent>,
) {
    if events.is_empty() {
        return;
    }

    let mut event = events.read();

    match event.next() {
        Some(event) => {
            match event.to {
                GameStates::Play => {
                    let window_width = window.resolution.width();

                    // Player
                    commands.spawn((
                        Player,
                        JumpingState {
                            state: JumpingStates::Idle,
                            jump_started_at: 0.0,
                            current_velocity: 0.0,
                        },
                        Transform {
                            translation: Vec3::new(
                                0.0 - (window_width / 2.0) + (window_width / 6.0),
                                0.0,
                                1.0,
                            ),
                            scale: Vec3::new(PLAYER_WIDTH, PLAYER_HEIGHT, 1.0),
                            ..Default::default()
                        },
                        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
                        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
                    ));
                }

                _ => {}
            }
        }

        _ => {}
    }
}

fn despawn_entities(
    mut commands: Commands,
    query: Query<Entity, Or<(With<Player>, With<Obstacle>)>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}

fn spawn_obstacles(
    window: Single<&mut Window>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let window_width = window.resolution.width();

    commands.spawn((
        Obstacle,
        Transform::from_xyz(window_width * 2., 0.0, 0.0),
        Mesh2d(meshes.add(Rectangle::new(PLAYER_WIDTH, PLAYER_HEIGHT))),
        MeshMaterial2d(materials.add(Color::srgb(0.7, 0.0, 0.0))),
    ));
}

fn move_obstacles(
    mut obstacles: Query<&mut Transform, With<Obstacle>>,
    window: Single<&Window>,
    mut score: ResMut<Score>,
) {
    let window_width = window.into_inner().width();

    for mut transform in obstacles.iter_mut() {
        if transform.translation.x < (-window_width / 2.) {
            transform.translation.x = window_width * 2.;
            score.0 += 100;
        } else {
            transform.translation.x -= 10.0;
        }
    }
}

fn update_score_text(score_text: Single<&mut Text, With<ScoreText>>, score: Res<Score>) {
    let mut text = score_text.into_inner();
    text.0 = score.0.to_string();
}

fn detect_collisions(
    player: Single<&Transform, With<Player>>,
    obstacles: Query<&Transform, With<Obstacle>>,
    mut events: EventWriter<CollisionEvent>,
) {
    let player_transform = player.into_inner();
    let player_bounding_box = Aabb2d::new(
        player_transform.translation.truncate(),
        player_transform.scale.truncate(),
    );

    for transform in obstacles.iter() {
        let obstacle_bounding_box =
            Aabb2d::new(transform.translation.truncate(), transform.scale.truncate());

        if player_bounding_box.intersects(&obstacle_bounding_box) {
            events.send_default();
        }
    }
}

fn maybe_transit_to_game_over(
    mut game_state_events: EventWriter<GameStateEvent>,
    mut game_state: ResMut<GameState>,
    mut events: EventReader<CollisionEvent>,
) {
    if !events.is_empty() {
        events.clear();

        game_state_events.send(GameStateEvent {
            from: game_state.0,
            to: GameStates::GameOver,
        });

        game_state.0 = GameStates::GameOver;
    }
}

fn maybe_transit_to_play(
    mut events: EventWriter<GameStateEvent>,
    mut game_state: ResMut<GameState>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut score: ResMut<Score>,
) {
    if keyboard.just_pressed(KeyCode::Space) {
        score.0 = 0;

        events.send(GameStateEvent {
            from: game_state.0,
            to: GameStates::Play,
        });

        game_state.0 = GameStates::Play;
    }
}

fn display_game_over_text(
    mut commands: Commands,
    window_query: Single<&Window>,
    mut game_state_events: EventReader<GameStateEvent>,
) {
    let window = window_query.into_inner();

    for event in game_state_events.read().into_iter() {
        match event.to {
            GameStates::GameOver => {
                commands.spawn((
                    Text2d::new("Game Over"),
                    TextLayout::new_with_justify(JustifyText::Center),
                    TextFont::from_font_size(48.0),
                    GameOverText,
                    Transform::from_xyz(
                        0.0,
                        0.0 + (window.width() / 2.) - (window.width() / 4.),
                        0.0,
                    ),
                ));
            }

            _ => {}
        }
    }
}

fn hide_game_over_text(
    mut commands: Commands,
    game_over_text_query: Query<Entity, With<GameOverText>>,
    mut game_state_events: EventReader<GameStateEvent>,
) {
    if game_over_text_query.is_empty() {
        return;
    }

    let game_over_text_entity = game_over_text_query.single();

    for event in game_state_events.read().into_iter() {
        match event.from {
            GameStates::GameOver => {
                commands.entity(game_over_text_entity).despawn();
            }

            _ => {}
        }
    }
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
