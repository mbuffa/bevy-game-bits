//! This example's sound catalogue. The playback plumbing — the decode guard,
//! the deferred-request queue, the one-shot spawn — lives in
//! `bevy_game_bits::audio` now (010-immersive is the second consumer). All
//! that's left here is *which four sounds this example has* and how loud each
//! one is; the mapping from an `InventoryAction` to one of them stays in
//! `main.rs`, where the game's decisions belong.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use bevy_game_bits::audio::{PlaySfx, SfxPlugin};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sfx {
    /// Pressing down on an item (also covers a plain click-to-select).
    Select,
    /// The press turned into an actual drag (crossed `DRAG_THRESHOLD_PX`).
    PickUp,
    /// Released over a free spot: the item moved.
    Drop,
    /// Released over an occupied or off-grid spot: the item snapped back.
    Invalid,
}

impl Sfx {
    pub const ALL: [Sfx; 4] = [Sfx::Select, Sfx::PickUp, Sfx::Drop, Sfx::Invalid];

    pub const fn path(self) -> &'static str {
        match self {
            Sfx::Select => "sfx/jsfxr/click.wav",
            Sfx::PickUp => "sfx/jsfxr/pickup.wav",
            Sfx::Drop => "sfx/jsfxr/click2.wav",
            Sfx::Invalid => "sfx/jsfxr/impact.wav",
        }
    }

    pub const fn volume(self) -> f32 {
        match self {
            Sfx::Invalid => 0.5,
            _ => 0.8,
        }
    }
}

/// Clip handles, loaded once at `Startup`.
#[derive(Resource)]
pub struct SfxAssets {
    handles: HashMap<Sfx, Handle<AudioSource>>,
}

impl SfxAssets {
    /// The [`PlaySfx`] request for one catalogue entry, at its recorded volume.
    pub fn play(&self, sfx: Sfx) -> PlaySfx {
        PlaySfx::new(self.handles[&sfx].clone()).with_volume(sfx.volume())
    }
}

pub fn load_sfx(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handles = Sfx::ALL
        .into_iter()
        .map(|sfx| (sfx, asset_server.load(sfx.path())))
        .collect();
    commands.insert_resource(SfxAssets { handles });
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SfxPlugin)
            .add_systems(Startup, load_sfx);
    }
}
