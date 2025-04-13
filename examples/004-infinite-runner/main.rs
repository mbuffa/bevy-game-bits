// Features an Infinite Runner prototype

// TODO:
// * Maybe run all physics-related systems on the FixedUpdate scheduler

use bevy::prelude::*;

mod actors;
mod collision;
mod colors;
mod game_state;
mod ui;

use game_state::*;
use ui::WindowSize;

const WINDOW_WIDTH: f32 = 800.0;
const WINDOW_HEIGHT: f32 = 600.0;

fn main() {
    App::new()
        .insert_resource(WindowSize(WINDOW_WIDTH, WINDOW_HEIGHT))
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.0)))
        .add_plugins(DefaultPlugins)
        .add_plugins(GameStatePlugin)
        .run();
}
