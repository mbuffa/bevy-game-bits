use bevy::prelude::*;

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

const DIFFICULTY_MULTIPLIER: f32 = 0.98;

pub struct ActorsPlugin;

impl Plugin for ActorsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BackgroundElements {
            tiers: vec![
                // Small Clouds
                ElementTier::new(
                    0.0,
                    20.0,
                    0.6,
                    Vec2::new(-16.0, 1.0),
                    Vec2::new(28.0, 16.0),
                    COLOR_GRAY,
                ),
                // Large Clouds
                ElementTier::with_default_transform(
                    1.0,
                    20.0,
                    0.6,
                    Vec2::new(72.0, 36.0),
                    COLOR_DARKGREY,
                ),
            ],
        })
        .insert_resource(Obstacles {
            tiers: vec![
                // Cactus
                ElementTier::with_default_transform(
                    0.0,
                    4.0,
                    8.,
                    Vec2::new(OBSTACLE_WIDTH, OBSTACLE_HEIGHT),
                    COLOR_WHITE,
                ),
                // Stone
                ElementTier::with_default_transform(
                    5.0,
                    12.0,
                    8.,
                    Vec2::new(56.0, 56.0),
                    COLOR_GRAY,
                ),
                // Bike
                ElementTier::with_default_transform(
                    9.0,
                    18.0,
                    10.,
                    Vec2::new(72.0, 48.0),
                    COLOR_WHITE,
                ),
            ],
        });
    }
}

#[derive(Component)]
pub struct Velocity(f32);

#[derive(Debug)]
pub struct ElementTier {
    spawn_cooldown: f32,
    default_spawn_cooldown: f32,
    velocity: f32,
    transform: Vec2,
    size: Vec2,
    color: Color,
}

impl ElementTier {
    pub fn with_default_transform(
        spawn_cooldown: f32,
        default_spawn_cooldown: f32,
        velocity: f32,
        size: Vec2,
        color: Color,
    ) -> Self {
        Self {
            spawn_cooldown,
            default_spawn_cooldown,
            velocity,
            transform: Vec2::new(0.0, 0.0),
            size,
            color,
        }
    }

    pub fn new(
        spawn_cooldown: f32,
        default_spawn_cooldown: f32,
        velocity: f32,
        transform: Vec2,
        size: Vec2,
        color: Color,
    ) -> Self {
        Self {
            spawn_cooldown,
            default_spawn_cooldown,
            velocity,
            transform,
            size,
            color,
        }
    }
}

#[derive(Resource)]
pub struct BackgroundElements {
    tiers: Vec<ElementTier>,
}

#[derive(Resource)]
pub struct Obstacles {
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
        jump::JumpingState::default(),
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

    for tier in obstacles.tiers.iter_mut() {
        tier.spawn_cooldown -= elapsed_secs;
        if tier.spawn_cooldown < 0. {
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

            tier.default_spawn_cooldown = tier.default_spawn_cooldown * DIFFICULTY_MULTIPLIER;
            tier.spawn_cooldown = tier.default_spawn_cooldown;
        }
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

    for tier in elements.tiers.iter_mut() {
        tier.spawn_cooldown -= elapsed_secs;

        if tier.spawn_cooldown < 0. {
            commands.spawn((
                BackgroundElement,
                Kind(Kinds::BackgroundElement),
                Velocity(tier.velocity),
                Transform {
                    translation: Vec3::new(
                        window_size.0,
                        (window_size.1 / 3.) + tier.transform.x,
                        tier.transform.y,
                    ),
                    scale: Vec3::new(tier.size.x, tier.size.y, 1.),
                    ..Default::default()
                },
                Mesh2d(meshes.add(Rectangle::new(1., 1.))),
                MeshMaterial2d(materials.add(tier.color)),
            ));

            tier.spawn_cooldown = tier.default_spawn_cooldown;
        }
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
