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

/// The traveller token. `map` is the map entity it belongs to (its `ChildOf`
/// parent too).
#[derive(Component, Clone, Copy, Debug)]
pub struct Traveler {
    pub map: Entity,
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
/// instead.
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

/// A left click either enters the place under the traveller, or sets a new
/// travel target. Clicks landing on the aside are ignored (the repo's
/// `Query<&Interaction>` guard).
pub fn handle_click(
    buttons: Res<ButtonInput<MouseButton>>,
    ui_nodes: Query<&Interaction>,
    maps: Query<(&WorldMapGrid, &WorldMapCursor, &WorldMapConfig)>,
    mut travelers: Query<(Entity, &Traveler, &TilePos, &mut TravelTarget, &AtLocation)>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if ui_nodes.iter().any(|i| *i != Interaction::None) {
        return;
    }

    for (traveler_e, traveler, pos, mut target, at) in &mut travelers {
        let Ok((grid, cursor, config)) = maps.get(traveler.map) else {
            continue;
        };
        if !cursor.valid {
            continue;
        }

        // Standing on a location: a click near the token means "enter", not
        // "walk one tile over". The radius is deliberately smaller than half a
        // tile so an adjacent-tile click still reads as travel.
        if let Some(location) = at.0 {
            let here = traveler_world(grid, pos.0);
            if cursor.world.distance(here) <= config.enter_widget_radius_px {
                if target.0.take().is_some() {
                    actions.write(WorldMapAction::TargetCleared {
                        map: traveler.map,
                        traveler: traveler_e,
                    });
                }
                actions.write(WorldMapAction::EnterRequested {
                    map: traveler.map,
                    traveler: traveler_e,
                    location,
                });
                continue;
            }
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

/// `Space` cancels the current target.
pub fn cancel_target(
    keys: Res<ButtonInput<KeyCode>>,
    mut travelers: Query<(Entity, &Traveler, &mut TravelTarget)>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    for (traveler_e, traveler, mut target) in &mut travelers {
        if target.0.take().is_some() {
            actions.write(WorldMapAction::TargetCleared {
                map: traveler.map,
                traveler: traveler_e,
            });
        }
    }
}

/// Move each traveller toward its target by this frame's distance budget,
/// scaled by the terrain under it. Impassable terrain stops it and clears the
/// target.
///
/// Terrain is sampled once per frame at the traveller's *current* cell — good
/// enough at travel speeds, and it means the model never needs a path.
pub fn travel(
    time: Res<Time>,
    maps: Query<(&WorldMapGrid, &WorldMapConfig)>,
    mut travelers: Query<(
        Entity,
        &Traveler,
        &mut TilePos,
        &mut TravelTarget,
        &mut TravelProgress,
    )>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    let dt = time.delta_secs();
    for (traveler_e, traveler, mut pos, mut target, mut progress) in &mut travelers {
        *progress = TravelProgress::default();
        progress.from = pos.0;

        let Ok((grid, config)) = maps.get(traveler.map) else {
            continue;
        };
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

        let step = (config.travel_tiles_per_sec * speed_mult * dt).min(dist);
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

/// Reveals a hidden location when a traveller's path this frame passes within
/// its reveal radius — [`WorldMapConfig::reveal_radius_tiles`] for an ordinary
/// place, the far tighter [`WorldMapConfig::secret_reveal_radius_tiles`] for a
/// [`SecretLocation`]. Either radius being `None` disables that kind only.
pub fn reveal_locations(
    travelers: Query<(&Traveler, &TilePos, &TravelProgress)>,
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
}
