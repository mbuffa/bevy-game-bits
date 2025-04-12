use bevy::prelude::*;

use crate::collision::CollisionEvent;
use crate::ui::Score;

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum GameStates {
    InsertCoin,
    Play,
    GameOver,
}

#[derive(Eq, PartialEq, Resource)]
pub struct GameState(pub GameStates);

#[derive(Event)]
pub struct GameStateEvent {
    #[allow(dead_code)]
    pub from: GameStates,
    pub to: GameStates,
}

pub fn maybe_transit_to_game_over(
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

pub fn maybe_transit_to_play_state(
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
