use bevy::prelude::*;

mod config;
mod enemy;
mod game;
mod placement;
mod scene;
mod turret;
mod weapons;

use config::CLEAR_COLOR;
use game::GamePlugin;

fn main() {
    App::new()
        .insert_resource(ClearColor(CLEAR_COLOR))
        .add_plugins(DefaultPlugins)
        .add_plugins(GamePlugin)
        .run();
}
