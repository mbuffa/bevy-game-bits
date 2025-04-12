// Features an Infinite Runner prototype

// TODO:
// * Rewrite the custom State management with Bevy's states
// * Maybe run all physics-related systems on the FixedUpdate scheduler
// * Add progressive difficulty
// * Add more parallax background

use bevy::prelude::*;

use bevy_game_bits::jump;

mod actors;
mod collision;
mod colors;
mod game_state;
mod ui;

use actors::*;
use collision::{detect_collisions, CollisionEvent};
use game_state::*;
use ui::*;

const SCREEN_UNIT: f32 = 10.0;
const WINDOW_WIDTH: f32 = 800.0;
const WINDOW_HEIGHT: f32 = 600.0;

fn main() {
    App::new()
        .insert_resource(WindowSize(WINDOW_WIDTH, WINDOW_HEIGHT))
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0)))
        .insert_resource(GameState(GameStates::InsertCoin))
        .insert_resource(Score(0))
        .add_event::<GameStateEvent>()
        .add_event::<CollisionEvent>()
        .add_plugins(ActorsPlugin)
        .add_plugins(DefaultPlugins)
        .add_plugins(jump::JumpPlugin {
            screen_unit: SCREEN_UNIT,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (maybe_transit_to_play_state, maybe_hide_instructions_text)
                .run_if(resource_equals(GameState(GameStates::InsertCoin))),
        )
        .add_systems(
            Update,
            (
                jump::handle_jumping_state,
                jump::update_player_velocity,
                jump::update_player_transform,
            )
                .run_if(resource_equals(GameState(GameStates::Play))),
        )
        .add_systems(
            Update,
            (
                maybe_hide_instructions_text,
                spawn_scene_and_player,
                spawn_obstacles,
                spawn_background_elements,
                move_moving_elements,
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
