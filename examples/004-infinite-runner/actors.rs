use bevy::prelude::*;
use rand::seq::IndexedRandom;

use bevy_game_bits::jump;

use crate::colors::*;
use crate::ui::{
    InstructionsText, Score, ScoreText, WindowSize, SCOREBOARD_FONT_SIZE, SCOREBOARD_TEXT_PADDING,
};

const PLAYER_WIDTH: f32 = 32.0;
const PLAYER_HEIGHT: f32 = 32.0;
const OBSTACLE_WIDTH: f32 = 8.0;
const OBSTACLE_HEIGHT: f32 = 72.0;

const HORIZON_HEIGHT: f32 = 32.0;

pub struct ActorsPlugin;

impl Plugin for ActorsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BackgroundElements {
            spawn_cooldown: 0.,
            default_spawn_cooldown: 8.,
            tiers: vec![
                // Clouds
                ElementTier::new(1.2, Vec2::new(28.0, 16.0), COLOR_GRAY),
            ],
        })
        .insert_resource(Obstacles {
            spawn_cooldown: 0.,
            default_spawn_cooldown: 2.,
            tiers: vec![
                // Cactus
                ElementTier::new(8., Vec2::new(OBSTACLE_WIDTH, OBSTACLE_HEIGHT), COLOR_WHITE),
                // Stone
                ElementTier::new(8., Vec2::new(56.0, 56.0), COLOR_GRAY),
                // Bike
                ElementTier::new(10., Vec2::new(72.0, 48.0), COLOR_WHITE),
            ],
        });
    }
}

#[derive(Component)]
pub struct Velocity(f32);

#[derive(Debug)]
pub struct ElementTier {
    velocity: f32,
    size: Vec2,
    color: Color,
}

impl ElementTier {
    pub fn new(velocity: f32, size: Vec2, color: Color) -> Self {
        Self {
            velocity,
            size,
            color,
        }
    }
}

#[derive(Resource)]
pub struct BackgroundElements {
    spawn_cooldown: f32,
    default_spawn_cooldown: f32,
    tiers: Vec<ElementTier>,
}

#[derive(Resource)]
pub struct Obstacles {
    spawn_cooldown: f32,
    default_spawn_cooldown: f32,
    tiers: Vec<ElementTier>,
}

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct Obstacle;

#[derive(Component)]
pub struct BackgroundElement;

enum Kinds {
    BackgroundElement,
    Obstacle,
}

#[derive(Component)]
pub struct Kind(Kinds);

pub fn spawn_scene_and_player(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // Player
    commands.spawn((
        Player,
        jump::JumpingState {
            state: jump::JumpingStates::Idle,
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

    // Horiwon
    commands.spawn((
        Transform::from_xyz(0.0, HORIZON_HEIGHT, -1.0),
        Mesh2d(meshes.add(Rectangle::new(window_size.0, 1.0))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
    ));

    // Score
    commands.spawn((
        ScoreText,
        InstructionsText,
        Text::new("0"),
        TextFont {
            font_size: SCOREBOARD_FONT_SIZE,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: SCOREBOARD_TEXT_PADDING,
            right: SCOREBOARD_TEXT_PADDING,
            ..default()
        },
    ));
}

pub fn spawn_obstacles(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    time: Res<Time>,
    obstacles_resource: ResMut<Obstacles>,
) {
    let elapsed_secs = time.delta().as_secs_f32();

    let obstacles = obstacles_resource.into_inner();

    obstacles.spawn_cooldown -= elapsed_secs;
    if obstacles.spawn_cooldown < 0. {
        match obstacles.tiers.choose(&mut rand::rng()) {
            Some(tier) => {
                commands.spawn((
                    Obstacle,
                    Kind(Kinds::Obstacle),
                    Velocity(tier.velocity),
                    Transform {
                        translation: Vec3::new(
                            window_size.0 * 2.,
                            (tier.size.y - PLAYER_HEIGHT) / 2.,
                            0.0,
                        ),
                        scale: Vec3::new(tier.size.x, tier.size.y, 1.),
                        ..Default::default()
                    },
                    Mesh2d(meshes.add(Rectangle::new(1., 1.))),
                    MeshMaterial2d(materials.add(tier.color)),
                ));
            }
            None => {}
        }

        obstacles.spawn_cooldown = obstacles.default_spawn_cooldown;
    }
}

pub fn spawn_background_elements(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    time: Res<Time>,
    elements_resource: ResMut<BackgroundElements>,
) {
    let elapsed_secs = time.delta().as_secs_f32();

    let elements = elements_resource.into_inner();

    elements.spawn_cooldown -= elapsed_secs;
    if elements.spawn_cooldown < 0. {
        match elements.tiers.choose(&mut rand::rng()) {
            Some(tier) => {
                commands.spawn((
                    BackgroundElement,
                    Kind(Kinds::BackgroundElement),
                    Velocity(tier.velocity),
                    Transform {
                        translation: Vec3::new(window_size.0, window_size.1 / 3., 0.0),
                        scale: Vec3::new(tier.size.x, tier.size.y, 1.),
                        ..Default::default()
                    },
                    Mesh2d(meshes.add(Rectangle::new(1., 1.))),
                    MeshMaterial2d(materials.add(tier.color)),
                ));
            }
            None => {}
        }

        elements.spawn_cooldown = elements.default_spawn_cooldown;
    }
}

pub fn despawn_entities(
    mut commands: Commands,
    query: Query<Entity, Or<(With<Player>, With<Obstacle>, With<BackgroundElement>)>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}

pub fn move_moving_elements(
    mut commands: Commands,
    window_size: Res<WindowSize>,
    mut obstacles: Query<(Entity, &Kind, &mut Velocity, &mut Transform)>,
    mut score: ResMut<Score>,
) {
    for (entity, kind, velocity, mut transform) in obstacles.iter_mut() {
        if transform.translation.x < (-window_size.0 / 2.) - OBSTACLE_WIDTH / 2. {
            match kind.0 {
                Kinds::Obstacle => {
                    score.0 += 100;
                }
                Kinds::BackgroundElement => {}
            }
            commands.entity(entity).despawn();
        } else {
            transform.translation.x -= velocity.0;
        }
    }
}
