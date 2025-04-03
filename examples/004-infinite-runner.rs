// Features an Infinite Runner prototype

// TODO:
// * Check for a better way to handle state transitions
// * Check for a better way to handle entities spawn
// * Declare a Jump or Player Plugin
// * Run all physics-related systems on the FixedUpdate scheduler
// * Add progressive difficulty
// * Add some parallax background

use bevy::math::bounding::{Aabb2d, IntersectsVolume};
use bevy::prelude::*;

const SCREEN_UNIT: f32 = 10.0;
const WINDOW_WIDTH: f32 = 800.0;
const WINDOW_HEIGHT: f32 = 600.0;
const HORIZON_HEIGHT: f32 = 32.0;
const PLAYER_WIDTH: f32 = 32.0;
const PLAYER_HEIGHT: f32 = 32.0;
const OBSTACLE_WIDTH: f32 = 8.0;
const OBSTACLE_HEIGHT: f32 = 72.0;
const SCOREBOARD_TEXT_PADDING: Val = Val::Px(4.0);
const SCOREBOARD_FONT_SIZE: f32 = 48.0;

#[derive(Resource)]
struct WindowSize(f32, f32);

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

#[derive(Resource)]
struct SpawnCooldowns {
    tier1: f32,
}

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

#[derive(Component)]
struct HitSpaceText;

#[derive(Component)]
struct InstructionsText;

fn main() {
    App::new()
        .insert_resource(WindowSize(WINDOW_WIDTH, WINDOW_HEIGHT))
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0)))
        .insert_resource(GameState(GameStates::InsertCoin))
        .insert_resource(Score(0))
        .insert_resource(SpawnCooldowns { tier1: 0.0 })
        .add_event::<GameStateEvent>()
        .add_event::<CollisionEvent>()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (maybe_transit_to_play_state, maybe_hide_instructions_text)
                .run_if(resource_equals(GameState(GameStates::InsertCoin))),
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
                maybe_hide_instructions_text,
                spawn_player,
                spawn_obstacles,
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
                maybe_transit_to_play_state,
            )
                .run_if(resource_equals(GameState(GameStates::GameOver))),
        )
        .run();
}

fn setup(
    window_size: Res<WindowSize>,
    mut window: Single<&mut Window>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    window.resolution.set(window_size.0, window_size.1);

    // Camera
    commands.spawn(Camera2d);

    add_instructions_text(&mut commands, &window);

    // Horiwon
    commands.spawn((
        Transform::from_xyz(0.0, HORIZON_HEIGHT, -1.0),
        Mesh2d(meshes.add(Rectangle::new(window_size.0, 1.0))),
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

fn spawn_player(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut events: EventReader<GameStateEvent>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if events.is_empty() {
        return;
    }

    let mut event = events.read();

    match event.next() {
        Some(event) => {
            match event.to {
                GameStates::Play => {
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
                                0.0 - (window_size.0 / 2.0) + (window_size.0 / 6.0),
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

fn spawn_obstacles(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    time: Res<Time>,
    mut cooldowns: ResMut<SpawnCooldowns>,
) {
    cooldowns.tier1 -= time.delta().as_secs_f32();

    if cooldowns.tier1 < 0.0 {
        cooldowns.tier1 = 2.0;

        commands.spawn((
            Obstacle,
            Transform {
                translation: Vec3::new(
                    window_size.0 * 2.,
                    (OBSTACLE_HEIGHT - PLAYER_HEIGHT) / 2.,
                    0.0,
                ),
                scale: Vec3::new(OBSTACLE_WIDTH, OBSTACLE_HEIGHT, 1.),
                ..Default::default()
            },
            Mesh2d(meshes.add(Rectangle::new(1., 1.))),
            MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
        ));
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

fn move_obstacles(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut obstacles: Query<(Entity, &mut Transform), With<Obstacle>>,
    mut score: ResMut<Score>,
) {
    for (entity, mut transform) in obstacles.iter_mut() {
        if transform.translation.x < (-window_size.0 / 2.) - OBSTACLE_WIDTH / 2. {
            score.0 += 100;
            commands.entity(entity).despawn();
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
        player_transform.scale.truncate() / 2.,
    );

    for transform in obstacles.iter() {
        let obstacle_bounding_box = Aabb2d::new(
            transform.translation.truncate(),
            transform.scale.truncate() / 2.,
        );

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

fn maybe_transit_to_play_state(
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
                    InstructionsText,
                    Transform::from_xyz(
                        0.0,
                        0.0 + (window.width() / 2.) - (window.width() / 4.),
                        0.0,
                    ),
                ));

                add_instructions_text(&mut commands, &window)
            }

            _ => {}
        }
    }
}

fn add_instructions_text(commands: &mut Commands, window: &Window) {
    commands.spawn((
        Text2d::new("Hit Space to Play"),
        TextLayout::new_with_justify(JustifyText::Center),
        TextFont::from_font_size(48.0),
        HitSpaceText,
        InstructionsText,
        Transform::from_xyz(0.0, 0.0 - (window.width() / 4.), 0.0),
    ));
}

fn maybe_hide_instructions_text(
    mut commands: Commands,
    ui_elements_query: Query<Entity, With<InstructionsText>>,
    mut game_state_events: EventReader<GameStateEvent>,
) {
    if ui_elements_query.is_empty() {
        return;
    }

    for event in game_state_events.read().into_iter() {
        match event.to {
            GameStates::Play => {
                for entity in ui_elements_query.iter() {
                    commands.entity(entity).despawn();
                }
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
