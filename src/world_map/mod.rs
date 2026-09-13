//! A Fallout 1/2-style overworld: a tile grid you click to send a token
//! walking across, terrain that slows it down, named places you can enter, and
//! a clock that only ticks while you travel.
//!
//! Everything about one map lives in **components on the map entity** — its
//! source, grid, config, skin, layout, cursor and camera state — so a host can
//! hold more than one map at a time.
//!
//! # Getting a map on screen
//!
//! ```ignore
//! use bevy_game_bits::world_map::prelude::*;
//!
//! app.add_plugins(DefaultPlugins)
//!     .add_plugins(WorldMapPlugin::default()) // loads maps/wastes.worldmap.json
//!     .add_plugins(WorldMapTimePlugin)        // optional in-game clock
//!     .add_systems(Startup, |mut commands: Commands| {
//!         commands.spawn((Camera2d, WorldMapCamera));
//!     });
//! ```
//!
//! The camera is the host's to spawn, and it **must** carry [`WorldMapCamera`]
//! — an unmarked second camera would silently break the `Single<&Camera>` the
//! cursor and pan systems rely on. It also **must stay at [`Camera::order`]
//! `0`** (Bevy's default): the map renders through a real
//! [`Camera::viewport`] cut to the region beside the aside (see
//! [`map_viewport`]), and the library spawns its *own* second camera — order
//! `1`, [`ClearColorConfig::None`] — to host the aside and interact-menu
//! `bevy_ui` roots full-window over top of it. Nothing clears the strip
//! outside the map viewport, so [`WorldMapTheme::aside_background`] is opaque
//! by default; a host that makes it translucent will see stale pixels there,
//! not a see-through map.
//!
//! Point [`WorldMapPlugin::map`] at your own spec to change the file, the
//! skin, or the layout, or spawn maps yourself with [`spawn_world_map`] after
//! [`WorldMapPlugin::headless`].
//!
//! # Time only moves when you do
//!
//! [`WorldMapTime`] (a component on the map entity) is the per-frame world-time
//! budget [`travel`] moves *every* traveller by — the player and every
//! [`Party`]. It runs while the player is travelling and pauses when they stop
//! ([`WorldMapConfig::pause_time_when_idle`], on by default), so caravans freeze
//! with you and can actually be intercepted. Drop a [`WorldClockHold`] on the
//! map to keep it running through a non-travel action (resting, repairing).
//!
//! # Bringing your own clock
//!
//! [`WorldMapTimePlugin`] is a separate, optional add-on — it owns
//! [`WorldMapClock`], the Day/HH:MM calendar layered on [`WorldMapTime`], and
//! draws the clock chip. Leave it out and read [`WorldMapTime`] /
//! [`TravelProgress`] from your own system ordered `.after(WorldMapSet::Travel)`.
//!
//! # The pieces
//!
//! - [`data`] — [`WorldMapData`] (the JSON format), [`WorldMapGrid`] (the
//!   resolved grid), and the [`tile_to_world`] / [`world_to_tile`] coordinate
//!   pair.
//! - [`asset`] — [`WorldMapAsset`], the `*.worldmap.json` [`AssetLoader`], and
//!   [`WorldMapSource`] (path / handle / inline).
//! - [`config`] — [`WorldMapConfig`] (model numbers), [`WorldMapTheme`] (skin),
//!   [`WorldMapLayout`], and [`WorldMapSpec`] bundling all three.
//! - [`travel`] — the traveller, its [`TravelTarget`], the movement step, the
//!   location bookkeeping, and [`WorldMapAction`], the message the module
//!   fires instead of acting itself.
//!
//! # Two kinds of hidden place
//!
//! An ordinary undiscovered location is spotted from a way off — anywhere
//! within [`WorldMapConfig::reveal_radius_tiles`]. A [`SecretLocation`] (a
//! buried cache, a wreck — the anchor of a treasure hunt) reveals only when
//! the traveller walks almost exactly over it, within the far tighter
//! [`WorldMapConfig::secret_reveal_radius_tiles`]. The check sweeps the whole
//! segment travelled each frame, so a fast frame can't skip the spot.
//!
//! # Parties
//!
//! Caravans, patrols and raiding parties are [`Party`] tokens the **host**
//! drives. Build one with [`party_bundle`]; it's a second [`Traveler`], so
//! [`travel`] moves it with terrain scaling and [`TravelProgress`] reports it
//! for free — and only while [`WorldMapTime`] runs, so a party freezes the
//! moment the player stops. The module ships no AI: the host writes its
//! [`TravelTarget`] and
//! flips its [`Spotted`] flag to say whether the player sees it (the seam a
//! spot-at-a-distance or radio feature plugs into). What it *does* own is
//! contact — [`track_intercepts`] keeps the player's [`Intercepting`] list of
//! *every* party within [`WorldMapConfig::intercept_radius_tiles`] (swept so a
//! fast frame can't tunnel through) and fires a
//! [`WorldMapAction::PartyIntercepted`] per one that enters it. The "enter"
//! affordance is a single square "interact" widget over the token: with one
//! thing in reach a click acts immediately (enter the town / hail the party),
//! with several it opens the [`InteractMenu`] to choose from. Only the
//! [`PlayerTraveler`] is steered by clicks — a caravan isn't. A host that wants
//! its own menu UI reads [`InteractMenu`] / [`Intercepting`] and writes
//! [`WorldMapAction::EnterRequested`] / [`WorldMapAction::InteractRequested`]
//! itself.
//!
//! - [`ui`] — [`spawn_world_map`], the tile/token drawing, the aside, and the
//!   camera.
//! - [`time`] — the optional [`WorldMapClock`] and [`WorldMapTimePlugin`].
//!
//! # Scheduling
//!
//! Order your own systems against [`WorldMapSet`], not the system functions.
//!
//! # Limits
//!
//! There's no pathfinding — routing around the mountains is the player's job.
//! Terrain is sampled once per frame at the traveller's current cell, so a
//! single fast frame could skim the corner of a slow tile. The interact square
//! sits just above the token, so a click on the very next tile up, close to the
//! token, can read as "interact" rather than "travel". The camera systems
//! assume one map is on screen at a time. A map opens framed on its
//! [`PlayerTraveler`] ([`snap_camera_to_traveler`]), then follows or holds per
//! [`WorldMapConfig::follow_traveler`].

pub mod asset;
pub mod config;
pub mod data;
pub mod time;
pub mod travel;
pub mod ui;

use bevy::prelude::*;

pub use asset::{WorldMapAsset, WorldMapLoadError, WorldMapLoader, WorldMapSource};
pub use config::{
    clamp_camera_center, map_viewport, visible_rect, AsideSide, WorldMapConfig, WorldMapLayout,
    WorldMapSpec, WorldMapTheme,
};
pub use data::{
    cell_of, tile_to_world, world_to_tile, LocationData, Terrain, TerrainData, WorldMapData,
    WorldMapError, WorldMapGrid,
};
pub use time::{advance_clock, WorldMapClock, WorldMapTimePlugin};
pub use travel::{
    cancel_target, handle_click, in_interact_square, interact_subjects, interact_widget_center,
    party_bundle, point_segment_distance, reveal_locations, tick_world_time, time_advancing,
    track_cursor, track_intercepts, track_location, travel, AtLocation, Discovered, InteractMenu,
    InteractSubject, Intercepting, Location, Party, PlayerTraveler, SecretLocation, Spotted,
    TilePos, TravelProgress, TravelSpeed, TravelTarget, Traveler, WorldClockHold, WorldMapAction,
    WorldMapCamera, WorldMapCursor, WorldMapTime,
};
pub use ui::{
    build_map_visuals, build_party_visuals, ensure_ui_camera, follow_and_clamp_camera, pan_camera,
    pick_interact_menu_row, refollow_on_new_target, resolve_map, snap_camera_to_traveler,
    spawn_world_map, sync_aside, sync_clock_label, sync_coords_label, sync_interact_menu,
    sync_interact_widgets, sync_location_visibility, sync_map_viewport, sync_party_visibility,
    sync_status_text, sync_target_marker, sync_traveler_transform, travel_to_aside_row, AsideRow,
    CameraSnapped, InteractMenuRow, InteractWidget, TargetMarker, WorldMapParts, WorldMapRoot,
    WorldMapTile, WorldMapView,
};

/// Everything you need to build and drive a world map, in one import.
pub mod prelude {
    pub use super::{
        cell_of, party_bundle, spawn_world_map, tile_to_world, world_to_tile, AsideSide,
        AtLocation, DefaultWorldMap, Discovered, InteractMenu, InteractSubject, Intercepting,
        Location, Party, PlayerTraveler, SecretLocation, Spotted, TilePos, TravelProgress,
        TravelSpeed, TravelTarget, Traveler, WorldClockHold, WorldMapAction, WorldMapCamera,
        WorldMapClock, WorldMapConfig, WorldMapCursor, WorldMapData, WorldMapGrid, WorldMapLayout,
        WorldMapPlugin, WorldMapSet, WorldMapSource, WorldMapSpec, WorldMapTheme, WorldMapTime,
        WorldMapTimePlugin, WorldMapView,
    };
}

/// Ordering handles for the world-map systems. Order against these, not the
/// system functions.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WorldMapSet {
    /// `Startup`. [`spawn_world_map`] for the plugin's own map. Order your own
    /// setup `.after(WorldMapSet::Setup)` to find [`DefaultWorldMap`].
    Setup,
    /// `Update`. [`resolve_map`] (and [`build_map_visuals`] when visuals are
    /// on) — the map data resolves and the entities come up here.
    Build,
    /// `Update`, after [`Build`](Self::Build). Cursor tracking, the click and
    /// `Space` handlers, and camera panning.
    Input,
    /// `Update`, after [`Input`](Self::Input). [`tick_world_time`], [`travel`],
    /// [`reveal_locations`], [`track_location`], [`track_intercepts`]. Read
    /// [`TravelProgress`] / [`WorldMapTime`] after this.
    Travel,
    /// `Update`, after [`Travel`](Self::Travel). The `sync_*` systems and the
    /// follow/clamp camera.
    Sync,
}

/// The entity of the map [`WorldMapPlugin`] spawned. Absent under
/// [`WorldMapPlugin::headless`].
#[derive(Resource, Clone, Copy, Debug, Deref)]
pub struct DefaultWorldMap(pub Entity);

/// Registers the world-map systems and the `*.worldmap.json` loader, and —
/// unless [`headless`](Self::headless) — spawns one map from [`map`](Self::map)
/// at `Startup`.
pub struct WorldMapPlugin {
    /// The map to spawn at `Startup`, or `None` to spawn none. `Default` is
    /// `Some(WorldMapSpec::default())` (the example's map file).
    pub map: Option<WorldMapSpec>,
    /// Draw the map. `false` registers only the model systems — no
    /// `Assets<Mesh>` access — so a headless test can drive the whole travel
    /// model under `MinimalPlugins`.
    pub visuals: bool,
}

impl Default for WorldMapPlugin {
    fn default() -> Self {
        Self {
            map: Some(WorldMapSpec::default()),
            visuals: true,
        }
    }
}

impl WorldMapPlugin {
    /// Register the model systems only: no map, no drawing.
    pub fn headless() -> Self {
        Self {
            map: None,
            visuals: false,
        }
    }
}

/// Carries [`WorldMapPlugin::map`] from `build` to the `Startup` system.
#[derive(Resource)]
struct PendingWorldMap(WorldMapSpec);

fn spawn_default_map(mut commands: Commands, pending: Option<Res<PendingWorldMap>>) {
    let Some(pending) = pending else { return };
    let map = ui::spawn_world_map(&mut commands, pending.0.clone());
    commands.insert_resource(DefaultWorldMap(map));
    commands.remove_resource::<PendingWorldMap>();
}

impl Plugin for WorldMapPlugin {
    fn build(&self, app: &mut App) {
        if app.is_plugin_added::<AssetPlugin>() {
            app.init_asset::<WorldMapAsset>()
                .register_asset_loader(WorldMapLoader);
        }

        app.add_message::<WorldMapAction>()
            .configure_sets(
                Update,
                (
                    WorldMapSet::Build,
                    WorldMapSet::Input,
                    WorldMapSet::Travel,
                    WorldMapSet::Sync,
                )
                    .chain(),
            )
            .add_systems(Update, ui::resolve_map.in_set(WorldMapSet::Build))
            .add_systems(
                Update,
                (
                    (
                        travel::track_cursor,
                        travel::handle_click,
                        ui::pick_interact_menu_row,
                        travel::cancel_target,
                        ui::travel_to_aside_row,
                        ui::refollow_on_new_target,
                    )
                        .chain(),
                    ui::pan_camera,
                )
                    .in_set(WorldMapSet::Input),
            )
            .add_systems(
                Update,
                (
                    travel::tick_world_time,
                    travel::travel,
                    travel::reveal_locations,
                    travel::track_location,
                    travel::track_intercepts,
                )
                    .chain()
                    .in_set(WorldMapSet::Travel),
            )
            .add_systems(
                Update,
                (
                    ui::sync_target_marker,
                    ui::sync_interact_widgets,
                    ui::sync_interact_menu,
                    ui::sync_location_visibility,
                    ui::sync_party_visibility,
                    ui::sync_aside,
                    ui::sync_clock_label,
                    ui::sync_coords_label,
                    ui::sync_status_text,
                    (
                        ui::sync_traveler_transform,
                        ui::snap_camera_to_traveler,
                        ui::follow_and_clamp_camera,
                    )
                        .chain(),
                )
                    .in_set(WorldMapSet::Sync),
            );

        if self.visuals {
            app.add_systems(
                Update,
                (
                    ui::build_map_visuals.after(ui::resolve_map),
                    ui::build_party_visuals,
                    ui::ensure_ui_camera,
                    ui::sync_map_viewport,
                )
                    .in_set(WorldMapSet::Build),
            );
        }

        if let Some(spec) = &self.map {
            app.insert_resource(PendingWorldMap(spec.clone()))
                .add_systems(Startup, spawn_default_map.in_set(WorldMapSet::Setup));
        }
    }
}
