//! Tunable constants for the immersive-sim prototype. Kept in one place, as
//! in 009-derby, so the feel (map path, player size, sensitivity, ...) can
//! be iterated on without hunting through the systems that use them.

use bevy::prelude::*;

// --- Map -------------------------------------------------------------------

/// Which map to load, relative to `assets/`. Swaps to a `.bsp` path in
/// Phase 5 once ericw-tools is wired up; the loader code itself doesn't
/// change, since `bevy_trenchbroom` treats both as a `SceneRoot` source.
pub const MAP_PATH: &str = "maps/immersive/test.map";

// --- TrenchBroom -------------------------------------------------------------------

/// The TrenchBroom game name this example registers itself as — shows up in
/// TrenchBroom's "New Map" dialog.
pub const TB_GAME_NAME: &str = "bevy_game_bits_immersive";

/// TrenchBroom units per Bevy meter. 39.37008 is bevy_trenchbroom's own
/// default (1 unit = 1 inch); named here so it's visible next to the other
/// tuning constants rather than an unlabeled default buried in a builder.
pub const TB_SCALE: f32 = 39.37008;

/// `StandardMaterial::lightmap_exposure` override for BSP-baked lightmaps.
/// Bevy's own default makes lightmaps invisible (see the bevy_trenchbroom
/// manual's BSP Tips section) — unused until Phase 5, harmless before then
/// since it only touches materials loaded from a `.bsp`.
pub const LIGHTMAP_EXPOSURE: f32 = 10_000.0;

// --- Player ------------------------------------------------------------------

/// Player collider dimensions, in meters. `bevy_ahoy`'s kinematic controller
/// "behaves best" with a cylinder per its own docs.
pub const PLAYER_RADIUS: f32 = 0.4;
pub const PLAYER_HEIGHT: f32 = 1.8;

/// Mouse-look sensitivity: degrees of yaw/pitch per pixel of mouse motion.
pub const MOUSE_SENSITIVITY: f32 = 0.07;

// --- Lighting ------------------------------------------------------------

/// Illuminance (lux) of the Phase 1 directional light. See `main.rs::spawn_light`
/// for why a directional light is used instead of the map's `light` entity.
pub const SUN_ILLUMINANCE: f32 = 3000.0;

// --- Interaction -----------------------------------------------------------

/// How far (meters) the "use" raycast reaches from the camera.
pub const INTERACT_RANGE: f32 = 2.5;

// --- Pickups -----------------------------------------------------------------

/// Edge length (meters) of the placeholder pickup cube.
pub const PICKUP_SIZE: f32 = 0.3;
pub const PICKUP_COLOR: Color = Color::srgb(1.0, 0.8, 0.15);
pub const PICKUP_EMISSIVE: LinearRgba = LinearRgba::rgb(1.5, 1.0, 0.1);

// --- Devtools ------------------------------------------------------------

/// A leg of the `IMMERSIVE_AUTOPILOT` script: hold this movement input and
/// turn at this yaw rate (degrees/sec) for `duration` seconds. See
/// `devtools.rs` for why this is the way this example can verify live
/// movement on a machine where synthetic keyboard/mouse input is blocked.
pub struct AutopilotStep {
    pub duration: f32,
    pub movement: bevy::math::Vec2,
    pub yaw_rate: f32,
}

/// Walk forward, turn toward the door, walk to it, turn to look around,
/// then hold still — enough to exercise movement, turning, and the
/// interact raycast without touching a keyboard.
pub const AUTOPILOT_SCRIPT: &[AutopilotStep] = &[
    AutopilotStep {
        duration: 1.5,
        movement: bevy::math::Vec2::new(0.0, 1.0),
        yaw_rate: 0.0,
    },
    AutopilotStep {
        duration: 1.0,
        movement: bevy::math::Vec2::ZERO,
        yaw_rate: 45.0,
    },
    AutopilotStep {
        duration: 2.0,
        movement: bevy::math::Vec2::new(0.0, 1.0),
        yaw_rate: 0.0,
    },
    AutopilotStep {
        duration: 2.0,
        movement: bevy::math::Vec2::ZERO,
        yaw_rate: 30.0,
    },
    AutopilotStep {
        duration: 3.0,
        movement: bevy::math::Vec2::ZERO,
        yaw_rate: 0.0,
    },
];

/// How often (seconds) `IMMERSIVE_TELEMETRY` logs player state.
pub const TELEMETRY_INTERVAL: f32 = 0.25;
