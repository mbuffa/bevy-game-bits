//! Colony sim prototype base: an LDtk-edited map rendered in 3D.
//!
//! The map data lives in `assets/maps/colony.ldtk` (editable with the LDtk
//! editor). bevy_ecs_ldtk loads it data-only — its 2D renderer is disabled —
//! and our own systems turn the grid into 3D tiles and units.

mod config;
mod construction;
mod cover;
mod crops;
mod daynight;
mod debug_ui;
mod director;
mod fire;
mod flora;
mod game;
mod history;
mod items;
mod map;
mod movement;
mod nav;
mod needs;
mod power;
mod scene;
mod selection;
mod shrubs;
mod snow;
mod temperature;
mod trees;
mod ui;
mod units;
mod weather;
mod zones;

use bevy::prelude::*;

use crate::config::CLEAR_COLOR;
use crate::game::GamePlugin;

fn main() {
    App::new()
        .insert_resource(ClearColor(CLEAR_COLOR))
        .add_plugins(DefaultPlugins)
        .add_plugins(GamePlugin)
        .run();
}
