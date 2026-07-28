//! All TrenchBroom entity classes for this example, and nothing else — meant
//! to be read as a single reference for what a level author can place.

use bevy::prelude::*;
use bevy_trenchbroom::prelude::*;

/// Where the player starts. The entity spawned from the map *becomes* the
/// player: `player::spawn_player` observes `On<Add, PlayerSpawn>` and bolts
/// the character controller onto it. The origin is the capsule's vertical
/// center, matching how `Collider::cylinder` is centered on `Transform`.
#[point_class(base(Transform), color(0 255 0), size(-16 -16 -36, 16 16 36))]
pub struct PlayerSpawn;

/// Base class for anything the player's "use" raycast can target.
/// `interact.rs` matches on this to know what to show a prompt for.
#[base_class]
#[derive(Default)]
pub struct Interactable {
    /// Text shown in the centered prompt, e.g. "Open door".
    pub prompt: String,
}

/// A brush door that slides `lip`-short of its own bounds along `angle`
/// when used, waits `wait` seconds, then slides back. Quake `func_door`
/// semantics.
#[solid_class(base(Interactable))]
pub struct FuncDoor {
    /// Yaw in degrees the door travels toward (0 = +X, 90 = +Y, ...).
    pub angle: f32,
    /// Travel speed in meters per second.
    pub speed: f32,
    /// How much of the door stays visible when fully open, in meters.
    pub lip: f32,
    /// Seconds open before auto-closing. Negative means it stays open.
    pub wait: f32,
}
impl Default for FuncDoor {
    fn default() -> Self {
        Self {
            angle: 0.0,
            speed: 2.0,
            lip: 0.2,
            wait: 3.0,
        }
    }
}

/// A collectable. `pickup::spawn_visuals` gives it a placeholder emissive-cube
/// mesh + sensor collider; swap for a glTF model later via the `model(...)`
/// class attribute.
#[point_class(base(Transform, Interactable))]
#[derive(Default)]
pub struct ItemPickup {
    /// Inventory key, e.g. "keycard_blue".
    pub item: String,
}
