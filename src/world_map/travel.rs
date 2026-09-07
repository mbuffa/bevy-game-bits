//! The travel model: the traveller token, its target, the step that moves it,
//! and the location bookkeeping — plus [`WorldMapAction`], the message this
//! module fires instead of playing a sound or opening a town itself (the same
//! division `vehicle::impact` draws: the library reports the fact, the game
//! decides what it means).
//!
//! Every fact lives on a component: [`TilePos`] is authoritative and the
//! traveller's `Transform` is a derived mirror (the reverse of most Bevy
//! setups, because all of this reasoning is in tile space).

use bevy::prelude::*;

use crate::world_map::config::WorldMapConfig;
use crate::world_map::data::{cell_of, tile_to_world, world_to_tile, WorldMapGrid};

/// Marker the host puts on its 2D camera so the module knows which one frames
/// the map. **Required** — an unmarked second camera would silently break the
/// `Single<&Camera>` the cursor and pan systems use.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct WorldMapCamera;

/// A token that moves across a map — the player, or a host-driven [`Party`].
/// `map` is the map entity it belongs to (its `ChildOf` parent too). Everything
/// that moves shares [`travel`], so terrain scaling and [`TravelProgress`]
/// reporting come for free.
#[derive(Component, Clone, Copy, Debug)]
pub struct Traveler {
    pub map: Entity,
}

/// Marker on the one traveller the module spawns for the player (in
/// [`resolve_map`](super::resolve_map)). Every system that means "the player"
/// and not "some caravan" filters on it — the click and `Space` handlers,
/// location reveal, the aside-row jump, the follow camera, and the coordinate
/// readout. One per map.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct PlayerTraveler;

/// A host-driven token: a caravan, a raiding party, a patrol. Build one with
/// [`party_bundle`]; the host owns where it goes (write its [`TravelTarget`])
/// and whether the player can see it (flip its [`Spotted`]). `color` is
/// per-faction, so it rides on the component rather than a global
/// `WorldMapTheme` — the same choice `TerrainData::color` makes.
///
/// Requires [`Spotted`] — `Spotted(true)` unless you spawn one explicitly
/// alongside, so `(party_bundle(..), Spotted(false))` is fine.
#[derive(Component, Clone, Debug)]
#[require(Spotted)]
pub struct Party {
    pub name: String,
    pub color: Color,
}

/// Whether the player can see this [`Party`]. Defaults to `true` (as a required
/// component of [`Party`]); the module ships **no rule that changes it** — the
/// host does, and this is the seam a "spot at a distance" or radio-overlap
/// feature plugs into. It does **not** gate interception: an unspotted party
/// still bumps into the player (that's the ambush).
#[derive(Component, Clone, Copy, Debug)]
pub struct Spotted(pub bool);

impl Default for Spotted {
    fn default() -> Self {
        Self(true)
    }
}

/// Per-traveller travel speed in tiles per second over `speed: 1.0` terrain,
/// overriding [`WorldMapConfig::travel_tiles_per_sec`] for this token only.
/// Standalone, not a [`Party`] field, so the player in a fast vehicle can carry
/// it too. Terrain still scales it.
#[derive(Component, Clone, Copy, Debug)]
pub struct TravelSpeed(pub f32);

/// Every [`Party`] the player is currently in contact with — those within
/// [`WorldMapConfig::intercept_radius_tiles`] this frame, **nearest first**.
/// Maintained by [`track_intercepts`]. A `Vec`, not an `Option`: two caravans
/// can share a spot, and the interact menu needs to offer both.
#[derive(Component, Clone, Debug, Default)]
pub struct Intercepting(pub Vec<Entity>);

impl Intercepting {
    /// The closest party in contact, if any.
    pub fn nearest(&self) -> Option<Entity> {
        self.0.first().copied()
    }
}

/// The traveller's position in **tile space** — the authoritative one. The
/// `Transform` is kept in step with it by
/// [`sync_traveler_transform`](super::sync_traveler_transform).
#[derive(Component, Clone, Copy, Debug)]
pub struct TilePos(pub Vec2);

/// Where the traveller is walking to, in tile space, or `None` when it's
/// standing still. Set by a left click, cleared by arrival, by `Space`, or by
/// stepping onto impassable terrain.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct TravelTarget(pub Option<Vec2>);

/// The location entity the traveller is currently standing on, if any.
/// Maintained by [`track_location`].
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AtLocation(pub Option<Entity>);

/// The host's read seam for time — rewritten every frame by [`travel`].
/// [`WorldMapTimePlugin`](super::WorldMapTimePlugin) reads it; a host with its
/// own clock reads it from a system ordered `.after(WorldMapSet::Travel)`
/// instead. Every field reads **zero for every traveller** on a frame where
/// [`WorldMapTime`] is paused (see [`tick_world_time`]).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct TravelProgress {
    /// Did the traveller move this frame?
    pub moving: bool,
    /// Tiles covered this frame.
    pub tiles_this_frame: f32,
    /// Straight-line tiles still between the traveller and its target (`0.0`
    /// with no target).
    pub tiles_remaining: f32,
    /// Where the traveller stood at the **start** of this frame — equal to the
    /// current [`TilePos`] when it didn't move. [`reveal_locations`] sweeps the
    /// `from → TilePos` segment so a single fast frame can't skip a tiny
    /// [`SecretLocation`].
    pub from: Vec2,
}

/// How much **world-map time** passed this frame — the budget [`travel`] moves
/// every traveller by, so parties and the player share one clock. A `Component`
/// on the map entity, rewritten each frame by [`tick_world_time`].
///
/// World time only runs while the player is travelling (or a [`WorldClockHold`]
/// is on the map, or [`WorldMapConfig::pause_time_when_idle`] is `false`) —
/// stop walking and the caravans freeze with you, which is what makes them
/// catchable. [`WorldMapClock`](super::WorldMapClock) is the optional
/// Day/HH:MM calendar layered on top of this.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct WorldMapTime {
    /// Seconds of world time this frame — `Res<Time>` delta while advancing,
    /// `0.0` while paused.
    pub delta_secs: f32,
    /// World seconds since the map loaded.
    pub elapsed_secs: f64,
}

/// Marker the **host** puts on the map entity to keep [`WorldMapTime`] running
/// while the player performs a non-travel timed action — resting, repairing, a
/// radio sweep. The library never adds or removes it. Time flows at the normal
/// rate while it's held; scale `Time<Virtual>` for a faster "rest".
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct WorldClockHold;

/// The open/closed state of the "interact" menu for one map — a `Component` on
/// the map entity. Opened by [`handle_click`] when the interact square is
/// clicked with **two or more** things in reach (one subject acts immediately,
/// no menu); closed by a pick, an outside click, `Escape`, or the subjects
/// going out of reach. A host that wants its own menu UI can read this plus
/// [`Intercepting`] and write [`WorldMapAction::EnterRequested`] /
/// [`WorldMapAction::InteractRequested`] itself.
#[derive(Component, Default, Clone, Debug)]
pub struct InteractMenu {
    /// The traveller the menu is open for; `None` when closed.
    pub traveler: Option<Entity>,
    /// What the rows currently stand for — re-checked against the live subjects
    /// each frame by [`sync_interact_menu`](super::sync_interact_menu), so a
    /// party walking out of reach drops its row.
    pub subjects: Vec<InteractSubject>,
}

impl InteractMenu {
    /// Is the menu open?
    pub fn is_open(&self) -> bool {
        self.traveler.is_some()
    }

    /// Reset to the closed state.
    pub fn close(&mut self) {
        self.traveler = None;
        self.subjects.clear();
    }
}

/// The cursor's position over one map, refreshed every frame by
/// [`track_cursor`]. A `Component` on the map entity.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct WorldMapCursor {
    /// Cursor position in world space.
    pub world: Vec2,
    /// Cursor position in tile space (may be off the map — check with
    /// [`WorldMapGrid::contains`]).
    pub tile: Vec2,
    /// Whether [`world`](Self::world) / [`tile`](Self::tile) are meaningful
    /// this frame (false when the pointer is outside the window).
    pub valid: bool,
}

/// One named place on the map. A `Component` on a child of the map entity,
/// spawned by [`resolve_map`](super::resolve_map) from a
/// [`LocationData`](super::LocationData).
#[derive(Component, Clone, Debug)]
pub struct Location {
    pub id: String,
    pub name: String,
    /// Where it sits, in **tile space** — a cell centre for a tile-anchored
    /// place, an arbitrary float point for a [`SecretLocation`].
    pub pos: Vec2,
    pub description: Option<String>,
}

impl Location {
    /// The cell [`pos`](Self::pos) falls in.
    pub fn cell(&self) -> IVec2 {
        cell_of(self.pos)
    }
}

/// Marker on a [`Location`] that only reveals when the traveller walks almost
/// exactly over [`Location::pos`] — [`WorldMapConfig::secret_reveal_radius_tiles`],
/// not the wide [`reveal_radius_tiles`](WorldMapConfig::reveal_radius_tiles) an
/// ordinary place is spotted from. Filter with `With<SecretLocation>` /
/// `Has<SecretLocation>`; discovery still arrives as
/// [`WorldMapAction::LocationDiscovered`].
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct SecretLocation;

/// Whether a [`Location`] is known yet. Starts from
/// [`LocationData::discovered`](super::LocationData::discovered); flipped to
/// `true` by [`reveal_locations`] when the traveller passes close.
#[derive(Component, Clone, Copy, Debug)]
pub struct Discovered(pub bool);

/// Everything the world map reports. One enum, every variant naming its `map`,
/// matching `InventoryAction`'s shape.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum WorldMapAction {
    /// The map file resolved and the grid, tiles and locations are up.
    Loaded { map: Entity },
    /// The map file failed to load or resolve; `error` is the reason.
    LoadFailed { map: Entity, error: String },
    /// A left click picked a new travel target.
    TargetSet {
        map: Entity,
        traveler: Entity,
        tile: Vec2,
    },
    /// The target was cancelled (`Space`, or a click that resolved to "enter"
    /// instead).
    TargetCleared { map: Entity, traveler: Entity },
    /// The traveller reached its target.
    Arrived {
        map: Entity,
        traveler: Entity,
        tile: Vec2,
    },
    /// The traveller walked into impassable terrain and stopped; the target
    /// was cleared.
    Blocked {
        map: Entity,
        traveler: Entity,
        tile: Vec2,
    },
    /// The traveller stepped onto a location's tile.
    LocationReached {
        map: Entity,
        traveler: Entity,
        location: Entity,
    },
    /// The traveller stepped off a location's tile.
    LocationLeft {
        map: Entity,
        traveler: Entity,
        location: Entity,
    },
    /// A previously-hidden location came into reveal range and is now shown.
    LocationDiscovered { map: Entity, location: Entity },
    /// The player came within [`WorldMapConfig::intercept_radius_tiles`] of a
    /// [`Party`] — edge-triggered, fires once per contact.
    PartyIntercepted {
        map: Entity,
        traveler: Entity,
        party: Entity,
    },
    /// The player and the [`Party`] it was in contact with have separated.
    PartyLeft {
        map: Entity,
        traveler: Entity,
        party: Entity,
    },
    /// The "interact" widget was clicked over an intercepted [`Party`] — the
    /// party-side twin of [`EnterRequested`](Self::EnterRequested), and the hook
    /// for a dialogue or event window.
    InteractRequested {
        map: Entity,
        traveler: Entity,
        party: Entity,
    },
    /// An aside row was clicked. Accompanied by a [`TargetSet`](Self::TargetSet)
    /// sending the traveller there — the row list is an input, not just a legend.
    LocationFocused { map: Entity, location: Entity },
    /// The "enter" widget was clicked while standing on a location — the hook
    /// for loading that place's own scene.
    EnterRequested {
        map: Entity,
        traveler: Entity,
        location: Entity,
    },
}

/// Project a tile-space position into world space for one grid.
pub(crate) fn traveler_world(grid: &WorldMapGrid, tile: Vec2) -> Vec2 {
    tile_to_world(tile, grid.size_px(), grid.tile_px())
}

/// Everything a host-driven [`Party`] needs to exist on `map` at `tile`. Compose
/// extra components on top — a faction tag, a [`TravelSpeed`], an [`AtLocation`]
/// if you want it tracked into towns:
///
/// ```ignore
/// commands.spawn((
///     party_bundle(map, Vec2::new(1.5, 8.5), Party { name: "Caravan".into(), color }),
///     TravelSpeed(0.8),
///     MyFaction::Merchants,
/// ));
/// ```
///
/// It starts [`Spotted(true)`](Spotted) (put `Spotted(false)` in the same spawn
/// to override) and with no [`TravelTarget`]; give it one to make it move.
pub fn party_bundle(map: Entity, tile: Vec2, party: Party) -> impl Bundle {
    (
        party,
        Traveler { map },
        TilePos(tile),
        TravelTarget::default(),
        TravelProgress::default(),
        Transform::default(),
        Visibility::default(),
        ChildOf(map),
    )
}

/// What one slot of the "interact" widget acts on when it's clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractSubject {
    /// Enter this location (fires [`WorldMapAction::EnterRequested`]).
    Location(Entity),
    /// Hail this party (fires [`WorldMapAction::InteractRequested`]).
    Party(Entity),
}

/// Everything the player can interact with from where they're standing — the
/// [`Location`] under the token first (if any), then every intercepted
/// [`Party`] nearest-first. The one place this list is built:
/// [`handle_click`], [`sync_interact_widgets`](super::sync_interact_widgets) and
/// [`sync_interact_menu`](super::sync_interact_menu) all call it.
pub fn interact_subjects(at: &AtLocation, intercepting: &Intercepting) -> Vec<InteractSubject> {
    let mut out = Vec::with_capacity(1 + intercepting.0.len());
    if let Some(location) = at.0 {
        out.push(InteractSubject::Location(location));
    }
    out.extend(intercepting.0.iter().map(|&p| InteractSubject::Party(p)));
    out
}

/// Where the single "interact" square sits over the token, in world space —
/// [`WorldMapConfig::interact_widget_offset_px`] straight above it. The one
/// place this geometry is written: [`handle_click`]'s hit test and
/// [`sync_interact_widgets`](super::sync_interact_widgets)'s drawing both call
/// it, so the square you click is always the square you see.
pub fn interact_widget_center(token_world: Vec2, config: &WorldMapConfig) -> Vec2 {
    token_world + Vec2::new(0.0, config.interact_widget_offset_px)
}

/// Is `p` inside the axis-aligned square of half-extent `half` centred on
/// `center`? The interact widget's hit test — a square, not the old disc.
pub fn in_interact_square(center: Vec2, half: f32, p: Vec2) -> bool {
    (p.x - center.x).abs() <= half && (p.y - center.y).abs() <= half
}

/// Refreshes every map's [`WorldMapCursor`] from the pointer and the
/// [`WorldMapCamera`].
pub fn track_cursor(
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<WorldMapCamera>>,
    mut maps: Query<(&WorldMapGrid, &mut WorldMapCursor)>,
) {
    let (camera, cam_tf) = *camera;
    let world = window
        .cursor_position()
        .and_then(|px| camera.viewport_to_world_2d(cam_tf, px).ok());
    for (grid, mut cursor) in &mut maps {
        match world {
            Some(world) => {
                cursor.world = world;
                cursor.tile = world_to_tile(world, grid.size_px(), grid.tile_px());
                cursor.valid = true;
            }
            None => cursor.valid = false,
        }
    }
}

/// Fire the [`WorldMapAction`] one interact subject stands for.
pub(crate) fn write_subject_action(
    actions: &mut MessageWriter<WorldMapAction>,
    map: Entity,
    traveler: Entity,
    subject: InteractSubject,
) {
    match subject {
        InteractSubject::Location(location) => {
            actions.write(WorldMapAction::EnterRequested {
                map,
                traveler,
                location,
            });
        }
        InteractSubject::Party(party) => {
            actions.write(WorldMapAction::InteractRequested {
                map,
                traveler,
                party,
            });
        }
    }
}

/// A left click interacts with the interact square over the player, dismisses an
/// open menu, or sets a new travel target. Clicks landing on the aside or the
/// open menu are ignored (the repo's `Query<&Interaction>` guard). Only the
/// [`PlayerTraveler`] is steered — a caravan is the host's to drive.
///
/// The square: with **one** thing in reach a click acts on it immediately; with
/// two or more it opens the [`InteractMenu`] for [`sync_interact_menu`](super::sync_interact_menu)
/// to draw and [`pick_interact_menu_row`](super::pick_interact_menu_row) to
/// resolve. A click anywhere else while the menu is open just closes it.
#[allow(clippy::type_complexity)]
pub fn handle_click(
    buttons: Res<ButtonInput<MouseButton>>,
    ui_nodes: Query<&Interaction>,
    mut maps: Query<(
        &WorldMapGrid,
        &WorldMapCursor,
        &WorldMapConfig,
        &mut InteractMenu,
    )>,
    mut travelers: Query<
        (
            Entity,
            &Traveler,
            &TilePos,
            &mut TravelTarget,
            &AtLocation,
            &Intercepting,
        ),
        With<PlayerTraveler>,
    >,
    mut actions: MessageWriter<WorldMapAction>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if ui_nodes.iter().any(|i| *i != Interaction::None) {
        return;
    }

    for (traveler_e, traveler, pos, mut target, at, intercepting) in &mut travelers {
        let Ok((grid, cursor, config, mut menu)) = maps.get_mut(traveler.map) else {
            continue;
        };
        if !cursor.valid {
            continue;
        }

        let here = traveler_world(grid, pos.0);
        let center = interact_widget_center(here, config);
        let on_square = in_interact_square(center, config.interact_widget_half_px, cursor.world);
        let subjects = interact_subjects(at, intercepting);

        if on_square && !subjects.is_empty() {
            // Acting on the square, not walking one tile over — drop any trip.
            if target.0.take().is_some() {
                actions.write(WorldMapAction::TargetCleared {
                    map: traveler.map,
                    traveler: traveler_e,
                });
            }
            if menu.is_open() {
                menu.close();
            } else if let [only] = subjects[..] {
                write_subject_action(&mut actions, traveler.map, traveler_e, only);
            } else {
                // Open it; `sync_interact_menu` fills in the rows from the live
                // subjects, so it stays the one place that list is built.
                menu.traveler = Some(traveler_e);
                menu.subjects.clear();
            }
            continue;
        }

        // A click off the square closes an open menu and does nothing else.
        if menu.is_open() {
            menu.close();
            continue;
        }

        let tile = grid.clamp_tile_pos(cursor.tile);
        target.0 = Some(tile);
        actions.write(WorldMapAction::TargetSet {
            map: traveler.map,
            traveler: traveler_e,
            tile,
        });
    }
}

/// `Space` cancels the player's current target; `Escape` closes an open
/// [`InteractMenu`]. A caravan's trip is the host's to interrupt.
pub fn cancel_target(
    keys: Res<ButtonInput<KeyCode>>,
    mut travelers: Query<(Entity, &Traveler, &mut TravelTarget), With<PlayerTraveler>>,
    mut menus: Query<&mut InteractMenu>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    let cancel = keys.just_pressed(KeyCode::Space);
    let dismiss = keys.just_pressed(KeyCode::Escape);
    if !cancel && !dismiss {
        return;
    }
    for (traveler_e, traveler, mut target) in &mut travelers {
        if dismiss {
            if let Ok(mut menu) = menus.get_mut(traveler.map) {
                if menu.is_open() {
                    menu.close();
                }
            }
        }
        if cancel && target.0.take().is_some() {
            actions.write(WorldMapAction::TargetCleared {
                map: traveler.map,
                traveler: traveler_e,
            });
        }
    }
}

/// Whether [`WorldMapTime`] advances this frame: always when the world doesn't
/// pause on idle or a [`WorldClockHold`] is set, otherwise only while the player
/// is travelling.
pub fn time_advancing(pause_when_idle: bool, hold: bool, player_travelling: bool) -> bool {
    !pause_when_idle || hold || player_travelling
}

/// Rewrites each map's [`WorldMapTime`] for this frame — `Res<Time>` delta while
/// the world is running, `0.0` while it's paused. Runs first in
/// [`WorldMapSet::Travel`](super::WorldMapSet::Travel), before [`travel`] reads
/// it.
pub fn tick_world_time(
    time: Res<Time>,
    mut maps: Query<(
        Entity,
        &mut WorldMapTime,
        &WorldMapConfig,
        Has<WorldClockHold>,
    )>,
    players: Query<(&Traveler, &TravelTarget), With<PlayerTraveler>>,
) {
    let real_dt = time.delta_secs();
    for (map, mut world_time, config, hold) in &mut maps {
        let travelling = players
            .iter()
            .any(|(t, target)| t.map == map && target.0.is_some());
        let advancing = time_advancing(config.pause_time_when_idle, hold, travelling);
        world_time.delta_secs = if advancing { real_dt } else { 0.0 };
        world_time.elapsed_secs += world_time.delta_secs as f64;
    }
}

/// Move each traveller toward its target by this frame's [`WorldMapTime`]
/// budget, scaled by the terrain under it. Impassable terrain stops it and
/// clears the target. A frame where world time is paused moves nobody.
///
/// Terrain is sampled once per frame at the traveller's *current* cell — good
/// enough at travel speeds, and it means the model never needs a path.
#[allow(clippy::type_complexity)]
pub fn travel(
    maps: Query<(&WorldMapGrid, &WorldMapConfig, &WorldMapTime)>,
    mut travelers: Query<(
        Entity,
        &Traveler,
        &mut TilePos,
        &mut TravelTarget,
        &mut TravelProgress,
        Option<&TravelSpeed>,
    )>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    for (traveler_e, traveler, mut pos, mut target, mut progress, speed) in &mut travelers {
        *progress = TravelProgress::default();
        progress.from = pos.0;

        let Ok((grid, config, world_time)) = maps.get(traveler.map) else {
            continue;
        };
        let dt = world_time.delta_secs;
        if dt <= 0.0 {
            continue;
        }
        let Some(goal) = target.0 else {
            continue;
        };

        let to_goal = goal - pos.0;
        let dist = to_goal.length();
        progress.tiles_remaining = dist;

        if dist <= config.arrive_epsilon_tiles {
            pos.0 = goal;
            target.0 = None;
            progress.tiles_remaining = 0.0;
            actions.write(WorldMapAction::Arrived {
                map: traveler.map,
                traveler: traveler_e,
                tile: goal,
            });
            continue;
        }

        let speed_mult = grid.speed_at(pos.0);
        if speed_mult <= 0.0 {
            target.0 = None;
            progress.tiles_remaining = 0.0;
            actions.write(WorldMapAction::Blocked {
                map: traveler.map,
                traveler: traveler_e,
                tile: pos.0,
            });
            continue;
        }

        let base_speed = speed.map_or(config.travel_tiles_per_sec, |s| s.0);
        let step = (base_speed * speed_mult * dt).min(dist);
        pos.0 = grid.clamp_tile_pos(pos.0 + to_goal / dist * step);
        progress.moving = true;
        progress.tiles_this_frame = step;
        progress.tiles_remaining = (dist - step).max(0.0);
    }
}

/// Distance from point `p` to the segment `a → b` (endpoints included). Used to
/// test the traveller's *whole path this frame* against a reveal radius, not
/// just where it ended up — at a tight [`SecretLocation`] radius a single fast
/// frame could otherwise step clean over the spot.
pub fn point_segment_distance(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= f32::EPSILON {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Reveals a hidden location when the **player's** path this frame passes within
/// its reveal radius — [`WorldMapConfig::reveal_radius_tiles`] for an ordinary
/// place, the far tighter [`WorldMapConfig::secret_reveal_radius_tiles`] for a
/// [`SecretLocation`]. Either radius being `None` disables that kind only. A
/// caravan wandering past does not uncover your map.
pub fn reveal_locations(
    travelers: Query<(&Traveler, &TilePos, &TravelProgress), With<PlayerTraveler>>,
    maps: Query<&WorldMapConfig>,
    mut locations: Query<(
        Entity,
        &Location,
        &ChildOf,
        &mut Discovered,
        Has<SecretLocation>,
    )>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    for (location_e, location, child_of, mut discovered, secret) in &mut locations {
        if discovered.0 {
            continue;
        }
        let map = child_of.parent();
        let Ok(config) = maps.get(map) else {
            continue;
        };
        let radius = if secret {
            config.secret_reveal_radius_tiles
        } else {
            config.reveal_radius_tiles
        };
        let Some(radius) = radius else {
            continue;
        };
        let near = travelers.iter().any(|(t, pos, progress)| {
            t.map == map && point_segment_distance(progress.from, pos.0, location.pos) <= radius
        });
        if near {
            discovered.0 = true;
            actions.write(WorldMapAction::LocationDiscovered {
                map,
                location: location_e,
            });
        }
    }
}

/// Maintains each traveller's [`AtLocation`], firing `LocationReached` /
/// `LocationLeft` on the edges. Only *discovered* locations count — an
/// undiscovered secret sharing your cell is not somewhere you've "reached" —
/// and when several share a cell the nearest to the traveller wins.
pub fn track_location(
    mut travelers: Query<(Entity, &Traveler, &TilePos, &mut AtLocation)>,
    locations: Query<(Entity, &Location, &ChildOf, &Discovered)>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    for (traveler_e, traveler, pos, mut at) in &mut travelers {
        let cell = cell_of(pos.0);
        let now = locations
            .iter()
            .filter(|(_, location, child_of, discovered)| {
                discovered.0 && child_of.parent() == traveler.map && location.cell() == cell
            })
            .min_by(|(_, a, _, _), (_, b, _, _)| {
                a.pos
                    .distance_squared(pos.0)
                    .total_cmp(&b.pos.distance_squared(pos.0))
            })
            .map(|(location_e, _, _, _)| location_e);
        if now == at.0 {
            continue;
        }
        if let Some(prev) = at.0 {
            actions.write(WorldMapAction::LocationLeft {
                map: traveler.map,
                traveler: traveler_e,
                location: prev,
            });
        }
        if let Some(curr) = now {
            actions.write(WorldMapAction::LocationReached {
                map: traveler.map,
                traveler: traveler_e,
                location: curr,
            });
        }
        at.0 = now;
    }
}

/// Maintains the player's [`Intercepting`] — every [`Party`] within
/// [`WorldMapConfig::intercept_radius_tiles`], nearest first — firing a
/// `PartyIntercepted` per party that entered contact and a `PartyLeft` per one
/// that left, [`track_location`]'s shape one row down.
///
/// The test is **swept and exact**: both tokens moved this frame, so two
/// closing on each other could pass clean through inside one step. The gap
/// between two points moving linearly over a frame is the distance from the
/// origin to the segment traced by their *difference* — one call to
/// [`point_segment_distance`], no new geometry, no tunnelling at any frame rate.
pub fn track_intercepts(
    mut players: Query<
        (
            Entity,
            &Traveler,
            &TilePos,
            &TravelProgress,
            &mut Intercepting,
        ),
        With<PlayerTraveler>,
    >,
    parties: Query<(Entity, &Traveler, &TilePos, &TravelProgress), With<Party>>,
    maps: Query<&WorldMapConfig>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    for (player_e, player_t, player_pos, player_prog, mut intercepting) in &mut players {
        let radius = maps
            .get(player_t.map)
            .ok()
            .and_then(|c| c.intercept_radius_tiles);
        let mut now: Vec<Entity> = Vec::new();
        if let Some(radius) = radius {
            let mut matched: Vec<(Entity, f32)> = parties
                .iter()
                .filter(|(_, pt, _, _)| pt.map == player_t.map)
                .filter_map(|(party_e, _, party_pos, party_prog)| {
                    let approach = point_segment_distance(
                        player_prog.from - party_prog.from,
                        player_pos.0 - party_pos.0,
                        Vec2::ZERO,
                    );
                    (approach <= radius).then_some((party_e, approach))
                })
                .collect();
            matched.sort_by(|a, b| a.1.total_cmp(&b.1));
            now = matched.into_iter().map(|(party_e, _)| party_e).collect();
        }
        // Same set (any order) — no edge, and leave the stored order alone.
        if now.len() == intercepting.0.len() && now.iter().all(|p| intercepting.0.contains(p)) {
            continue;
        }
        for &prev in &intercepting.0 {
            if !now.contains(&prev) {
                actions.write(WorldMapAction::PartyLeft {
                    map: player_t.map,
                    traveler: player_e,
                    party: prev,
                });
            }
        }
        for &curr in &now {
            if !intercepting.0.contains(&curr) {
                actions.write(WorldMapAction::PartyIntercepted {
                    map: player_t.map,
                    traveler: player_e,
                    party: curr,
                });
            }
        }
        intercepting.0 = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traveler_world_places_a_cell_centre_correctly() {
        // 6x6 @ 48 -> size 288, centred on origin.
        let data = crate::world_map::data::WorldMapData::demo();
        let grid = WorldMapGrid::from_data(&data).unwrap();
        // Cell (0,0) centre in tile space is (0.5, 0.5).
        let w = traveler_world(&grid, Vec2::splat(0.5));
        assert_eq!(w, Vec2::new(-144.0 + 24.0, 144.0 - 24.0));
    }

    #[test]
    fn point_segment_distance_projects_onto_the_segment() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(10.0, 0.0);
        // Foot of perpendicular lands inside the segment.
        assert!((point_segment_distance(a, b, Vec2::new(4.0, 3.0)) - 3.0).abs() < 1.0e-5);
        // A fast frame steps a → b clean over a point 0.2 off the line.
        assert!(point_segment_distance(a, b, Vec2::new(5.0, 0.2)) <= 0.2 + 1.0e-5);
        // Projection past b -> distance to b itself.
        assert!((point_segment_distance(a, b, Vec2::new(13.0, 4.0)) - 5.0).abs() < 1.0e-5);
        // Projection before a -> distance to a itself.
        assert!((point_segment_distance(a, b, Vec2::new(-3.0, 4.0)) - 5.0).abs() < 1.0e-5);
        // Degenerate segment -> plain point distance.
        assert!((point_segment_distance(a, a, Vec2::new(3.0, 4.0)) - 5.0).abs() < 1.0e-5);
    }

    #[test]
    fn relative_motion_catches_a_fast_crossing() {
        // Two tokens swap places along x in one frame: A 0->10, B 10->0. Both
        // end far from where they started, but they pass through each other.
        let (a_from, a_to) = (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0));
        let (b_from, b_to) = (Vec2::new(10.0, 0.0), Vec2::new(0.0, 0.0));
        let approach = point_segment_distance(a_from - b_from, a_to - b_to, Vec2::ZERO);
        assert!(approach < 1.0e-5, "closest approach ~0, got {approach}");
        // The naive end-point test would see 10 tiles and miss the contact.
        assert!(a_to.distance(b_to) > 9.0);
    }

    #[test]
    fn interact_widget_center_sits_above_the_token() {
        let config = WorldMapConfig::default();
        let token = Vec2::new(100.0, 50.0);
        let c = interact_widget_center(token, &config);
        assert!((c.x - token.x).abs() < 1.0e-5);
        assert!((c.y - (token.y + config.interact_widget_offset_px)).abs() < 1.0e-5);
    }

    #[test]
    fn interact_subjects_puts_the_location_first() {
        let loc = Entity::from_raw_u32(1).unwrap();
        let p0 = Entity::from_raw_u32(2).unwrap();
        let p1 = Entity::from_raw_u32(3).unwrap();

        // Nothing in reach -> empty.
        assert!(interact_subjects(&AtLocation(None), &Intercepting(vec![])).is_empty());

        // Location, then the parties in Intercepting order.
        let subjects = interact_subjects(&AtLocation(Some(loc)), &Intercepting(vec![p0, p1]));
        assert_eq!(
            subjects,
            vec![
                InteractSubject::Location(loc),
                InteractSubject::Party(p0),
                InteractSubject::Party(p1),
            ]
        );

        // Parties only, no location row.
        let subjects = interact_subjects(&AtLocation(None), &Intercepting(vec![p1, p0]));
        assert_eq!(
            subjects,
            vec![InteractSubject::Party(p1), InteractSubject::Party(p0)]
        );
    }

    #[test]
    fn time_advancing_truth_table() {
        // pause_when_idle = false -> world time always runs.
        assert!(time_advancing(false, false, false));
        // A held clock runs it even while idle.
        assert!(time_advancing(true, true, false));
        // Otherwise it follows the player.
        assert!(time_advancing(true, false, true));
        assert!(!time_advancing(true, false, false));
    }

    #[test]
    fn interact_square_includes_its_corner() {
        let c = Vec2::new(5.0, 5.0);
        assert!(in_interact_square(c, 2.0, c));
        assert!(in_interact_square(c, 2.0, Vec2::new(7.0, 7.0))); // corner
        assert!(!in_interact_square(c, 2.0, Vec2::new(7.01, 5.0))); // just outside
    }
}
