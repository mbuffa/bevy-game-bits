use bevy::prelude::*;

use bevy_game_bits::jump;

use crate::actors::*;
use crate::collision::{detect_collisions, CollisionEvent};
use crate::ui::*;

const SCREEN_UNIT: f32 = 10.0;

#[derive(States, Default, Debug, Clone, Eq, PartialEq, Hash)]
pub enum GameStates {
    #[default]
    InsertCoin,
    Play,
    GameOver,
}

pub struct GameStatePlugin;

impl Plugin for GameStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameStates>()
            .insert_resource(Score(0))
            .add_event::<CollisionEvent>()
            .add_plugins(ActorsPlugin)
            .add_plugins(jump::JumpPlugin {
                screen_unit: SCREEN_UNIT,
            })
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (maybe_transit_to_play_state).run_if(in_state(GameStates::InsertCoin)),
            )
            .add_systems(
                OnEnter(GameStates::Play),
                (maybe_hide_instructions_text, spawn_scene_and_player),
            )
            .add_systems(
                Update,
                (
                    maybe_transit_to_game_over,
                    spawn_obstacles,
                    spawn_background_elements,
                    update_score_text,
                )
                    .chain()
                    .run_if(in_state(GameStates::Play)),
            )
            .add_systems(
                FixedUpdate,
                (
                    jump::handle_jumping_state,
                    jump::update_player_velocity,
                    jump::update_player_transform,
                    move_moving_elements,
                    detect_collisions,
                )
                    .run_if(in_state(GameStates::Play)),
            )
            .add_systems(OnExit(GameStates::Play), despawn_entities)
            .add_systems(OnEnter(GameStates::GameOver), display_game_over_text)
            .add_systems(
                Update,
                (maybe_transit_to_play_state).run_if(in_state(GameStates::GameOver)),
            );
    }
}

fn setup(window_size: Res<WindowSize>, mut window: Single<&mut Window>, mut commands: Commands) {
    window.resolution.set(window_size.0, window_size.1);

    // Camera
    commands.spawn(Camera2d);

    commands.spawn((
        Text2d::new("Infinite Runner"),
        TextLayout::new_with_justify(JustifyText::Center),
        TextFont::from_font_size(72.0),
        TitleText,
        InstructionsText,
        Transform::from_xyz(
            0.0,
            0.0 + (window.height() / 2.) - (window.height() / 4.),
            0.0,
        ),
    ));

    add_instructions_text(&mut commands);
}

pub fn maybe_transit_to_play_state(
    mut next_state: ResMut<NextState<GameStates>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut score: ResMut<Score>,
) {
    if keyboard.just_pressed(KeyCode::Space) {
        score.0 = 0;
        next_state.set(GameStates::Play);
    }
}

pub fn maybe_transit_to_game_over(
    mut next_state: ResMut<NextState<GameStates>>,
    mut events: EventReader<CollisionEvent>,
) {
    if !events.is_empty() {
        events.clear();
        next_state.set(GameStates::GameOver)
    }
}
