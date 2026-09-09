//! TrenchBroom configuration and our custom class registration. Kept in one
//! file so every TrenchBroom-facing knob is visible together.

use bevy::prelude::*;
use bevy_trenchbroom::prelude::*;

use crate::{classes, config};

pub fn config() -> TrenchBroomConfig {
    TrenchBroomConfig::new(config::TB_GAME_NAME)
        .scale(config::TB_SCALE)
        // Every solid class gets a compound convex collider by default, plus
        // smoothed normals so curved brushwork doesn't look faceted. If we
        // ever author Q1 BSPs, this needs `qbsp -wrbrushesonly` or no
        // brushes (and thus no collision) make it into the compile.
        .default_solid_scene_hooks(|| {
            SceneHooks::new()
                .convex_collider()
                .smooth_by_default_angle()
        })
        .lightmap_exposure(Some(config::LIGHTMAP_EXPOSURE))
}

/// Registers our custom Quake classes. Built-in ones (`worldspawn`, `light`,
/// `info_player_start`, ...) are already registered by `TrenchBroomPlugins`
/// and don't need this.
pub fn register_classes(app: &mut App) {
    app.register_type::<classes::PlayerSpawn>()
        .register_type::<classes::Interactable>()
        .register_type::<classes::FuncDoor>()
        .register_type::<classes::FuncLadder>()
        .register_type::<classes::PropCrate>()
        .register_type::<classes::ItemPickup>();
}
