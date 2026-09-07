//! Building one map on screen and keeping the view in step with the model.
//!
//! The map itself is world-space `Mesh2d` over primitive shapes — one shared
//! quad mesh, one `ColorMaterial` per terrain kind — matching every other 2D
//! example in this repo (there is no sprite or tilemap code anywhere in it).
//! The aside is plain `bevy_ui`.
//!
//! [`spawn_world_map`] returns immediately with the map entity; the tiles,
//! tokens and rows appear a frame or two later once [`resolve_map`] has the
//! map data.

use bevy::prelude::*;

use crate::world_map::asset::{WorldMapAsset, WorldMapSource};
use crate::world_map::config::{
    clamp_camera_center, visible_rect, WorldMapConfig, WorldMapLayout, WorldMapSpec, WorldMapTheme,
};
use crate::world_map::data::{tile_to_world, WorldMapData, WorldMapGrid};
use crate::world_map::time::WorldMapClock;
use crate::world_map::travel::{
    interact_widget_layout, traveler_world, AtLocation, Discovered, InteractSubject, Intercepting,
    Location, Party, PlayerTraveler, SecretLocation, Spotted, TilePos, TravelProgress,
    TravelTarget, Traveler, WorldMapAction, WorldMapCamera, WorldMapCursor, WorldMapTime,
};

/// Marks the root entity of one map — carries every per-map component and is
/// the handle every other API takes. Its `Mesh2d` children (backing, tiles,
/// tokens) hang off it in world space.
#[derive(Component)]
pub struct WorldMapRoot;

/// Runtime camera state for one map: whether the follow-cam is currently
/// engaged. A manual pan clears it; setting a new target restores it (to
/// [`WorldMapConfig::follow_traveler`]).
#[derive(Component, Clone, Copy, Debug)]
pub struct WorldMapView {
    pub follow: bool,
}

/// Set once resolving the map data fails, so [`resolve_map`] stops retrying.
#[derive(Component)]
pub struct WorldMapLoadFailed;

/// Set once [`build_map_visuals`] has drawn a map, so it draws each map once.
#[derive(Component)]
pub struct WorldMapVisualsBuilt;

/// The satellite entities [`spawn_world_map`] built for one map. A `Component`
/// on the map entity.
#[derive(Component, Clone, Copy, Debug)]
pub struct WorldMapParts {
    /// The absolutely-positioned aside panel.
    pub aside_root: Entity,
    /// The column the location rows are added to.
    pub aside_list: Entity,
    /// The one-line status text at the foot of the aside.
    pub status_text: Entity,
    /// The clock chip text. Empty and hidden unless a
    /// [`WorldMapClock`](super::WorldMapClock) exists.
    pub clock_label: Entity,
    /// The live `you … · cursor …` tile-coordinate line. Hidden when
    /// [`WorldMapConfig::show_coords`](super::WorldMapConfig::show_coords) is
    /// false.
    pub coords_label: Entity,
}

/// One terrain tile quad. `ChildOf` the map.
#[derive(Component)]
pub struct WorldMapTile;

/// The little disc that marks where the traveller is heading. One per
/// traveller; `.0` is the traveller entity.
#[derive(Component)]
pub struct TargetMarker(pub Entity);

/// One square of the clickable "interact" widget over the player token. Two per
/// player traveller: `slot` 0 acts on the location under the token, `slot` 1 on
/// an intercepted [`Party`]. Each shows only when its subject is present.
#[derive(Component)]
pub struct InteractWidget {
    pub traveler: Entity,
    pub slot: usize,
}

/// Set once [`build_party_visuals`] has drawn a [`Party`]'s token, so it draws
/// each once — even a party the host spawns mid-game.
#[derive(Component)]
pub struct PartyVisualsBuilt;

/// One aside row. `location` is the [`Location`] entity it stands for, `map`
/// the map it belongs to (rows aren't children of the map, so they carry the
/// link).
#[derive(Component, Clone, Copy)]
pub struct AsideRow {
    pub location: Entity,
    pub map: Entity,
}

/// Build one map: the aside panel and the map entity with all its per-map
/// components. Returns the **map entity**. Synchronous — the entity is usable
/// the moment this returns. Spawns no camera; the host does, tagged
/// [`WorldMapCamera`].
pub fn spawn_world_map(commands: &mut Commands, spec: WorldMapSpec) -> Entity {
    let WorldMapSpec {
        source,
        config,
        theme,
        layout,
    } = spec;

    // --- aside panel -------------------------------------------------------
    let aside_root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(layout.aside_width_px),
                left: match layout.aside_side {
                    crate::world_map::config::AsideSide::Left => Val::Px(0.0),
                    crate::world_map::config::AsideSide::Right => Val::Auto,
                },
                right: match layout.aside_side {
                    crate::world_map::config::AsideSide::Left => Val::Auto,
                    crate::world_map::config::AsideSide::Right => Val::Px(0.0),
                },
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(theme.aside_row_gap_px),
                padding: UiRect::all(Val::Px(theme.aside_padding_px)),
                ..default()
            },
            BackgroundColor(theme.aside_background),
            // A hover target, so `handle_click` can tell a click landed here.
            Interaction::default(),
        ))
        .id();

    if let Some(title) = &config.title {
        commands.spawn((
            Text::new(title.clone().into_owned()),
            TextFont {
                font_size: theme.aside_heading_font_size,
                ..default()
            },
            TextColor(theme.aside_heading_color),
            ChildOf(aside_root),
        ));
    }

    let aside_list = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(theme.aside_row_gap_px),
                flex_grow: 1.0,
                ..default()
            },
            ChildOf(aside_root),
        ))
        .id();

    let clock_label = commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: theme.clock_font_size,
                ..default()
            },
            TextColor(theme.clock_text_color),
            Visibility::Hidden,
            ChildOf(aside_root),
        ))
        .id();

    let coords_label = commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: theme.coords_font_size,
                ..default()
            },
            TextColor(theme.coords_text_color),
            if config.show_coords {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
            ChildOf(aside_root),
        ))
        .id();

    let status_text = commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: theme.status_font_size,
                ..default()
            },
            TextColor(theme.status_text_color),
            ChildOf(aside_root),
        ))
        .id();

    // --- map entity ------------------------------------------------------
    commands
        .spawn((
            WorldMapRoot,
            source,
            config,
            theme,
            layout,
            WorldMapCursor::default(),
            WorldMapTime::default(),
            Transform::default(),
            Visibility::default(),
            WorldMapParts {
                aside_root,
                aside_list,
                status_text,
                clock_label,
                coords_label,
            },
        ))
        .id()
}

/// Resolves each map's [`WorldMapSource`] and, once the data is in hand,
/// inserts [`WorldMapGrid`], spawns the traveller and location entities, and
/// fills the aside. Model only — no meshes, so it runs headlessly too.
/// [`build_map_visuals`] adds the drawing.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn resolve_map(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    assets: Option<Res<Assets<WorldMapAsset>>>,
    mut maps: Query<
        (Entity, &mut WorldMapSource, &WorldMapConfig, &WorldMapParts),
        (Without<WorldMapGrid>, Without<WorldMapLoadFailed>),
    >,
    mut actions: MessageWriter<WorldMapAction>,
) {
    for (map, mut source, config, parts) in &mut maps {
        // Get the raw data, or move on (still loading / no data yet).
        let data: WorldMapData = match &*source {
            WorldMapSource::Inline(data) => (**data).clone(),
            WorldMapSource::Path(path) => {
                let Some(server) = &asset_server else {
                    continue;
                };
                *source = WorldMapSource::Handle(server.load(path.to_string()));
                continue;
            }
            WorldMapSource::Handle(handle) => {
                let Some(assets) = &assets else { continue };
                match assets.get(handle) {
                    Some(asset) => asset.0.clone(),
                    None => {
                        let failed = asset_server
                            .as_ref()
                            .map(|s| s.load_state(handle).is_failed())
                            .unwrap_or(false);
                        if failed {
                            commands.entity(map).insert(WorldMapLoadFailed);
                            actions.write(WorldMapAction::LoadFailed {
                                map,
                                error: "asset failed to load".to_string(),
                            });
                        }
                        continue;
                    }
                }
            }
        };

        let grid = match WorldMapGrid::from_data(&data) {
            Ok(grid) => grid,
            Err(err) => {
                commands.entity(map).insert(WorldMapLoadFailed);
                actions.write(WorldMapAction::LoadFailed {
                    map,
                    error: err.to_string(),
                });
                continue;
            }
        };

        let size_px = grid.size_px();
        let tile_px = grid.tile_px();

        // Traveller.
        let start = grid.clamp_tile_pos(Vec2::from(data.start));
        let traveler = commands
            .spawn((
                Traveler { map },
                PlayerTraveler,
                TilePos(start),
                TravelTarget::default(),
                AtLocation::default(),
                Intercepting::default(),
                TravelProgress::default(),
                Transform::from_translation(traveler_world(&grid, start).extend(0.0)),
                Visibility::default(),
                ChildOf(map),
            ))
            .id();
        commands.spawn((TargetMarker(traveler), map_child(map)));
        commands.spawn((InteractWidget { traveler, slot: 0 }, map_child(map)));
        commands.spawn((InteractWidget { traveler, slot: 1 }, map_child(map)));

        // Locations. `from_data` succeeded above, so `tile_pos()` is `Some`.
        for location in &data.locations {
            let pos = location
                .tile_pos()
                .expect("validated by WorldMapGrid::from_data");
            let discovered = location.discovered();
            let center = tile_to_world(pos, size_px, tile_px);
            let location_entity = commands
                .spawn((
                    Location {
                        id: location.id.clone(),
                        name: location.name.clone(),
                        pos,
                        description: location.description.clone(),
                    },
                    Discovered(discovered),
                    Transform::from_translation(center.extend(0.0)),
                    if discovered {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    },
                    ChildOf(map),
                ))
                .id();
            if location.secret {
                commands.entity(location_entity).insert(SecretLocation);
            }

            commands.spawn((
                AsideRow {
                    location: location_entity,
                    map,
                },
                Button,
                Node {
                    padding: UiRect::all(Val::Px(6.0)),
                    // Starts collapsed; `sync_aside` opens it once discovered.
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ChildOf(parts.aside_list),
                children![(
                    Text::new(location.name.clone()),
                    TextFont::default(),
                    AsideRowText,
                )],
            ));
        }

        commands.entity(map).insert((
            grid,
            WorldMapView {
                follow: config.follow_traveler,
            },
        ));

        actions.write(WorldMapAction::Loaded { map });
    }
}

/// A row's name text — its font size is set from the theme by
/// [`style_aside_rows`] once the theme is known.
#[derive(Component)]
pub struct AsideRowText;

fn map_child(map: Entity) -> impl Bundle {
    (Transform::default(), Visibility::Hidden, ChildOf(map))
}

/// Adds the drawing for a freshly resolved map: the backing quad, the terrain
/// tiles, and the meshes on the traveller / markers / locations. Only added
/// when [`WorldMapPlugin::visuals`](super::WorldMapPlugin) is set.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn build_map_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    maps: Query<
        (Entity, &WorldMapGrid, &WorldMapConfig, &WorldMapTheme),
        (With<WorldMapView>, Without<WorldMapVisualsBuilt>),
    >,
    tokens: Query<(Entity, &Traveler), With<PlayerTraveler>>,
    markers: Query<(Entity, &TargetMarker)>,
    widgets: Query<(Entity, &InteractWidget)>,
    locations: Query<(Entity, &Location, &ChildOf, Has<SecretLocation>)>,
    rows: Query<(Entity, &AsideRow)>,
    row_texts: Query<(Entity, &ChildOf), With<AsideRowText>>,
    mut text_fonts: Query<&mut TextFont>,
) {
    for (map, grid, config, theme) in &maps {
        let size = grid.size_px();
        let tile_px = grid.tile_px();
        let inset = theme.tile_inset_px;

        // Backing quad — shows through the tile insets as grid seams.
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(size.x, size.y))),
            MeshMaterial2d(materials.add(ColorMaterial::from(theme.grid_color))),
            Transform::from_xyz(0.0, 0.0, theme.z_tiles - 0.1),
            ChildOf(map),
        ));

        // One shared inset quad, one material per terrain kind.
        let tile_mesh = meshes.add(Rectangle::new(tile_px - 2.0 * inset, tile_px - 2.0 * inset));
        let terrain_mats: Vec<Handle<ColorMaterial>> = grid
            .terrains()
            .iter()
            .map(|t| materials.add(ColorMaterial::from(t.color)))
            .collect();

        for row in 0..grid.rows() {
            for col in 0..grid.cols() {
                let cell = IVec2::new(col as i32, row as i32);
                let Some(idx) = grid.tile_terrain_index(cell) else {
                    continue;
                };
                let center =
                    tile_to_world(Vec2::new(col as f32 + 0.5, row as f32 + 0.5), size, tile_px);
                commands.spawn((
                    WorldMapTile,
                    Mesh2d(tile_mesh.clone()),
                    MeshMaterial2d(terrain_mats[idx].clone()),
                    Transform::from_xyz(center.x, center.y, theme.z_tiles),
                    ChildOf(map),
                ));
            }
        }

        // Traveller token.
        let traveler_mesh = meshes.add(Circle::new(theme.traveler_radius_px));
        let traveler_mat = materials.add(ColorMaterial::from(theme.traveler_color));
        for (entity, traveler) in &tokens {
            if traveler.map != map {
                continue;
            }
            commands.entity(entity).insert((
                Mesh2d(traveler_mesh.clone()),
                MeshMaterial2d(traveler_mat.clone()),
            ));
        }

        // Target marker.
        let marker_mesh = meshes.add(Circle::new(theme.target_marker_radius_px));
        let marker_mat = materials.add(ColorMaterial::from(theme.target_marker_color));
        for (entity, marker) in &markers {
            let Ok((_, traveler)) = tokens.get(marker.0) else {
                continue;
            };
            if traveler.map != map {
                continue;
            }
            commands.entity(entity).insert((
                Mesh2d(marker_mesh.clone()),
                MeshMaterial2d(marker_mat.clone()),
            ));
        }

        // Interact widget — one square mesh, a green material for the "enter a
        // location" slot and a cooler one for the "hail a party" slot.
        let square = meshes.add(Rectangle::new(
            2.0 * config.interact_widget_half_px,
            2.0 * config.interact_widget_half_px,
        ));
        let enter_mat = materials.add(ColorMaterial::from(theme.enter_widget_color));
        let party_mat = materials.add(ColorMaterial::from(theme.party_widget_color));
        for (entity, widget) in &widgets {
            let Ok((_, traveler)) = tokens.get(widget.traveler) else {
                continue;
            };
            if traveler.map != map {
                continue;
            }
            let mat = if widget.slot == 0 {
                &enter_mat
            } else {
                &party_mat
            };
            commands
                .entity(entity)
                .insert((Mesh2d(square.clone()), MeshMaterial2d(mat.clone())));
        }

        // Location circles + name labels. Secrets get their own smaller dot.
        let loc_mesh = meshes.add(Circle::new(theme.location_radius_px));
        let loc_mat = materials.add(ColorMaterial::from(theme.location_color));
        let secret_mesh = meshes.add(Circle::new(theme.secret_location_radius_px));
        let secret_mat = materials.add(ColorMaterial::from(theme.secret_location_color));
        for (entity, location, child_of, secret) in &locations {
            if child_of.parent() != map {
                continue;
            }
            let (mesh, mat, radius) = if secret {
                (
                    secret_mesh.clone(),
                    secret_mat.clone(),
                    theme.secret_location_radius_px,
                )
            } else {
                (loc_mesh.clone(), loc_mat.clone(), theme.location_radius_px)
            };
            commands
                .entity(entity)
                .insert((Mesh2d(mesh), MeshMaterial2d(mat)));
            let label_y =
                radius + theme.location_label_gap_px + theme.location_label_font_size * 0.5;
            commands.spawn((
                Text2d::new(location.name.clone()),
                TextFont {
                    font_size: theme.location_label_font_size,
                    ..default()
                },
                TextColor(theme.location_label_color),
                Transform::from_xyz(0.0, label_y, theme.z_labels - theme.z_locations),
                ChildOf(entity),
            ));
        }

        // Aside row font sizes from the theme.
        for (text_entity, child_of) in &row_texts {
            if rows
                .get(child_of.parent())
                .map(|(_, r)| r.map == map)
                .unwrap_or(false)
            {
                if let Ok(mut font) = text_fonts.get_mut(text_entity) {
                    font.font_size = theme.aside_row_font_size;
                }
            }
        }

        commands.entity(map).insert(WorldMapVisualsBuilt);
    }
}

/// Draws each [`Party`] token — a diamond in the party's own colour, with a
/// name label — the first frame it's seen. Runs every frame over
/// `Without<PartyVisualsBuilt>` (not once per map like [`build_map_visuals`]),
/// so a caravan the host spawns mid-game still gets drawn. `visuals`-gated.
pub fn build_party_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    themes: Query<&WorldMapTheme>,
    parties: Query<(Entity, &Traveler, &Party), Without<PartyVisualsBuilt>>,
) {
    for (entity, traveler, party) in &parties {
        let Ok(theme) = themes.get(traveler.map) else {
            continue;
        };
        let mesh = meshes.add(RegularPolygon::new(theme.party_radius_px, 4));
        let mat = materials.add(ColorMaterial::from(party.color));
        commands
            .entity(entity)
            .insert((Mesh2d(mesh), MeshMaterial2d(mat), PartyVisualsBuilt));
        let label_y = theme.party_radius_px
            + theme.location_label_gap_px
            + theme.location_label_font_size * 0.5;
        commands.spawn((
            Text2d::new(party.name.clone()),
            TextFont {
                font_size: theme.location_label_font_size,
                ..default()
            },
            TextColor(theme.party_label_color),
            Transform::from_xyz(0.0, label_y, theme.z_labels - theme.z_party),
            ChildOf(entity),
        ));
    }
}

/// Mirrors each [`Party`]'s [`Spotted`] flag onto its `Visibility`. An
/// idempotent every-frame mirror, **not** `Changed`-filtered: a party spawned
/// with `Spotted(false)` has to come up hidden on its very first frame, with no
/// transition to hook (inventory's `sync_window_visibility` lesson).
pub fn sync_party_visibility(mut parties: Query<(&Spotted, &mut Visibility), With<Party>>) {
    for (spotted, mut vis) in &mut parties {
        let wanted = if spotted.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != wanted {
            *vis = wanted;
        }
    }
}

/// Mirrors each traveller's [`TilePos`] onto its `Transform`, and holds it at
/// the right z-layer.
pub fn sync_traveler_transform(
    maps: Query<(&WorldMapGrid, &WorldMapTheme)>,
    mut travelers: Query<(&Traveler, &TilePos, &mut Transform, Has<Party>), Changed<TilePos>>,
) {
    for (traveler, pos, mut tf, is_party) in &mut travelers {
        let Ok((grid, theme)) = maps.get(traveler.map) else {
            continue;
        };
        let w = traveler_world(grid, pos.0);
        let z = if is_party {
            theme.z_party
        } else {
            theme.z_traveler
        };
        tf.translation = w.extend(z);
    }
}

/// Positions and shows/hides the target disc.
pub fn sync_target_marker(
    maps: Query<(&WorldMapGrid, &WorldMapTheme)>,
    travelers: Query<(&Traveler, &TravelTarget)>,
    mut markers: Query<(&TargetMarker, &mut Transform, &mut Visibility)>,
) {
    for (marker, mut tf, mut vis) in &mut markers {
        let Ok((traveler, target)) = travelers.get(marker.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        match target.0 {
            Some(tile) => {
                let Ok((grid, theme)) = maps.get(traveler.map) else {
                    continue;
                };
                tf.translation = traveler_world(grid, tile).extend(theme.z_target);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

/// Positions and shows/hides the two "interact" widget squares, reading the same
/// [`interact_widget_layout`](super::interact_widget_layout) the click hit test
/// does — so the square drawn is the square clicked. Slot 0 shows while the
/// player stands on a location, slot 1 while a [`Party`] is intercepted.
pub fn sync_interact_widgets(
    maps: Query<(&WorldMapGrid, &WorldMapConfig, &WorldMapTheme)>,
    travelers: Query<(&Traveler, &TilePos, &AtLocation, &Intercepting)>,
    mut widgets: Query<(&InteractWidget, &mut Transform, &mut Visibility)>,
) {
    for (widget, mut tf, mut vis) in &mut widgets {
        let Ok((traveler, pos, at, intercepting)) = travelers.get(widget.traveler) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let Ok((grid, config, theme)) = maps.get(traveler.map) else {
            continue;
        };
        let here = traveler_world(grid, pos.0);
        let subjects = [
            at.0.map(InteractSubject::Location),
            intercepting.0.map(InteractSubject::Party),
        ];
        match interact_widget_layout(here, subjects, config)[widget.slot] {
            Some((_, center)) => {
                tf.translation = center.extend(theme.z_enter_widget);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

/// Shows/hides a location's circle + label as it's discovered.
#[allow(clippy::type_complexity)]
pub fn sync_location_visibility(
    mut locations: Query<(&Discovered, &mut Visibility), (With<Location>, Changed<Discovered>)>,
) {
    for (discovered, mut vis) in &mut locations {
        *vis = if discovered.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Shows discovered rows (hides the rest) and highlights the row for the
/// location the traveller is standing on.
#[allow(clippy::type_complexity)]
pub fn sync_aside(
    travelers: Query<(&Traveler, &AtLocation)>,
    locations: Query<&Discovered, With<Location>>,
    themes: Query<&WorldMapTheme>,
    mut rows: Query<(&AsideRow, &mut Node, &mut BackgroundColor)>,
) {
    for (row, mut node, mut bg) in &mut rows {
        let Ok(theme) = themes.get(row.map) else {
            continue;
        };
        let discovered = locations.get(row.location).map(|d| d.0).unwrap_or(false);
        let wanted_display = if discovered {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != wanted_display {
            node.display = wanted_display;
        }
        let current = travelers
            .iter()
            .any(|(t, at)| t.map == row.map && at.0 == Some(row.location));
        let wanted_bg = if current {
            theme.aside_row_current_background
        } else {
            theme.aside_row_background
        };
        if bg.0 != wanted_bg {
            bg.0 = wanted_bg;
        }
    }
}

/// Fills the clock chip when a [`WorldMapClock`] exists; leaves it empty and
/// hidden otherwise.
pub fn sync_clock_label(
    clock: Option<Res<WorldMapClock>>,
    maps: Query<&WorldMapParts>,
    mut texts: Query<(&mut Text, &mut Visibility)>,
) {
    for parts in &maps {
        let Ok((mut text, mut vis)) = texts.get_mut(parts.clock_label) else {
            continue;
        };
        match &clock {
            Some(clock) => {
                let label = clock.label();
                if text.0 != label {
                    text.0 = label;
                }
                if *vis != Visibility::Inherited {
                    *vis = Visibility::Inherited;
                }
            }
            None => {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
        }
    }
}

/// Builds the `you … · cursor …` coordinate line — see [`coords_label`].
fn coords_label(you: Vec2, cursor: Option<Vec2>) -> String {
    match cursor {
        Some(c) => format!(
            "you {:.2}, {:.2}  ·  cursor {:.2}, {:.2}",
            you.x, you.y, c.x, c.y
        ),
        None => format!("you {:.2}, {:.2}", you.x, you.y),
    }
}

/// Keeps the aside's tile-coordinate line current — the traveller's tile
/// position and the cursor's (dropped while the pointer is off the map). Skips
/// a map whose [`WorldMapConfig::show_coords`](super::WorldMapConfig) is false,
/// leaving that line hidden.
pub fn sync_coords_label(
    maps: Query<(Entity, &WorldMapConfig, &WorldMapCursor, &WorldMapParts)>,
    travelers: Query<(&Traveler, &TilePos), With<PlayerTraveler>>,
    mut texts: Query<&mut Text>,
) {
    for (map, config, cursor, parts) in &maps {
        if !config.show_coords {
            continue;
        }
        let Ok(mut text) = texts.get_mut(parts.coords_label) else {
            continue;
        };
        let you = travelers
            .iter()
            .find_map(|(t, pos)| (t.map == map).then_some(pos.0))
            .unwrap_or(Vec2::ZERO);
        let label = coords_label(you, cursor.valid.then_some(cursor.tile));
        if text.0 != label {
            text.0 = label;
        }
    }
}

/// Writes the one-line status text from the latest [`WorldMapAction`]. Actions
/// carrying a `traveler` that isn't the [`PlayerTraveler`] (a caravan arriving,
/// say) are dropped — the status line is the player's.
pub fn sync_status_text(
    mut actions: MessageReader<WorldMapAction>,
    locations: Query<&Location>,
    secrets: Query<(), With<SecretLocation>>,
    players: Query<(), With<PlayerTraveler>>,
    parties: Query<&Party>,
    maps: Query<&WorldMapParts>,
    mut texts: Query<&mut Text>,
) {
    for action in actions.read() {
        let traveler = match action {
            WorldMapAction::TargetSet { traveler, .. }
            | WorldMapAction::TargetCleared { traveler, .. }
            | WorldMapAction::Arrived { traveler, .. }
            | WorldMapAction::Blocked { traveler, .. }
            | WorldMapAction::LocationReached { traveler, .. }
            | WorldMapAction::LocationLeft { traveler, .. }
            | WorldMapAction::EnterRequested { traveler, .. }
            | WorldMapAction::PartyIntercepted { traveler, .. }
            | WorldMapAction::PartyLeft { traveler, .. }
            | WorldMapAction::InteractRequested { traveler, .. } => Some(*traveler),
            WorldMapAction::Loaded { .. }
            | WorldMapAction::LoadFailed { .. }
            | WorldMapAction::LocationDiscovered { .. }
            | WorldMapAction::LocationFocused { .. } => None,
        };
        if traveler.is_some_and(|t| !players.contains(t)) {
            continue;
        }
        let party_name = |e: Entity| {
            parties
                .get(e)
                .map(|p| p.name.clone())
                .unwrap_or_else(|_| "a party".to_string())
        };
        let (map, line) = match action {
            WorldMapAction::Loaded { map } => (*map, "Map loaded.".to_string()),
            WorldMapAction::LoadFailed { map, error } => (*map, format!("Map failed: {error}")),
            WorldMapAction::TargetSet { map, .. } => (*map, "Travelling…".to_string()),
            WorldMapAction::TargetCleared { map, .. } => (*map, "Stopped.".to_string()),
            WorldMapAction::Arrived { map, .. } => (*map, "Arrived.".to_string()),
            WorldMapAction::Blocked { map, .. } => (*map, "Impassable — stopped.".to_string()),
            WorldMapAction::LocationReached { map, location, .. } => (
                *map,
                match locations.get(*location) {
                    Ok(l) => format!("At {}.", l.name),
                    Err(_) => "At a location.".to_string(),
                },
            ),
            WorldMapAction::LocationLeft { map, .. } => (*map, "Travelling…".to_string()),
            WorldMapAction::LocationDiscovered { map, location } => (
                *map,
                match locations.get(*location) {
                    Ok(l) if secrets.contains(*location) => format!("Found {}.", l.name),
                    Ok(l) => format!("Discovered {}.", l.name),
                    Err(_) => "Discovered a location.".to_string(),
                },
            ),
            WorldMapAction::LocationFocused { .. } => continue,
            WorldMapAction::EnterRequested { map, location, .. } => (
                *map,
                match locations.get(*location) {
                    Ok(l) => format!("Entering {}…", l.name),
                    Err(_) => "Entering…".to_string(),
                },
            ),
            WorldMapAction::PartyIntercepted { map, party, .. } => {
                (*map, format!("Intercepted {}.", party_name(*party)))
            }
            WorldMapAction::PartyLeft { map, .. } => (*map, "Travelling…".to_string()),
            WorldMapAction::InteractRequested { map, party, .. } => {
                (*map, format!("Hailing {}…", party_name(*party)))
            }
        };
        if let Ok(parts) = maps.get(map) {
            if let Ok(mut text) = texts.get_mut(parts.status_text) {
                text.0 = line;
            }
        }
    }
}

/// Arrow keys pan the camera and disengage the follow-cam.
pub fn pan_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut camera: Single<&mut Transform, With<WorldMapCamera>>,
    mut views: Query<(&WorldMapConfig, &mut WorldMapView)>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if dir == Vec2::ZERO {
        return;
    }
    let Some((config, mut view)) = views.iter_mut().next() else {
        return;
    };
    view.follow = false;
    let delta = dir.normalize() * config.pan_px_per_sec * time.delta_secs();
    camera.translation += delta.extend(0.0);
}

/// Restores the follow-cam when a fresh target is set.
pub fn refollow_on_new_target(
    mut actions: MessageReader<WorldMapAction>,
    mut views: Query<(&WorldMapConfig, &mut WorldMapView)>,
) {
    for action in actions.read() {
        if let WorldMapAction::TargetSet { map, .. } = action {
            if let Ok((config, mut view)) = views.get_mut(*map) {
                view.follow = config.follow_traveler;
            }
        }
    }
}

/// Sends the traveller to a location when its aside row is clicked — the same
/// outcome as clicking that tile on the map. Fires `LocationFocused` (the "a
/// row was clicked" host hook) alongside the `TargetSet`.
pub fn travel_to_aside_row(
    rows: Query<(&AsideRow, &Interaction), Changed<Interaction>>,
    locations: Query<(&Location, &Discovered)>,
    maps: Query<&WorldMapGrid>,
    mut travelers: Query<(Entity, &Traveler, &mut TravelTarget), With<PlayerTraveler>>,
    mut actions: MessageWriter<WorldMapAction>,
) {
    for (row, interaction) in &rows {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Ok((location, discovered)) = locations.get(row.location) else {
            continue;
        };
        if !discovered.0 {
            continue;
        }
        let Ok(grid) = maps.get(row.map) else {
            continue;
        };
        let tile = grid.clamp_tile_pos(location.pos);
        for (traveler_e, traveler, mut target) in &mut travelers {
            if traveler.map != row.map {
                continue;
            }
            target.0 = Some(tile);
            actions.write(WorldMapAction::TargetSet {
                map: row.map,
                traveler: traveler_e,
                tile,
            });
        }
        actions.write(WorldMapAction::LocationFocused {
            map: row.map,
            location: row.location,
        });
    }
}

/// Eases the follow-cam toward the moving traveller, then clamps the camera so
/// the map keeps covering the part of the view the aside doesn't hide.
pub fn follow_and_clamp_camera(
    time: Res<Time>,
    window: Single<&Window>,
    mut camera: Single<&mut Transform, With<WorldMapCamera>>,
    maps: Query<(
        Entity,
        &WorldMapGrid,
        &WorldMapConfig,
        &WorldMapLayout,
        &WorldMapView,
    )>,
    travelers: Query<(&Traveler, &TilePos, &TravelProgress), With<PlayerTraveler>>,
) {
    let Some((map, grid, config, layout, view)) = maps.iter().next() else {
        return;
    };
    let mut center = camera.translation.truncate();

    if view.follow && config.follow_traveler {
        if let Some((_, pos, _)) = travelers
            .iter()
            .find(|(t, _, progress)| t.map == map && progress.moving)
        {
            let target = traveler_world(grid, pos.0);
            let decay = (-config.follow_lerp * time.delta_secs()).exp();
            center = target + (center - target) * decay;
        }
    }

    let visible = visible_rect(window.size(), layout);
    center = clamp_camera_center(grid.size_px(), visible, center);
    camera.translation.x = center.x;
    camera.translation.y = center.y;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coords_label_formats_and_drops_an_invalid_cursor() {
        assert_eq!(
            coords_label(Vec2::new(1.5, 8.5), Some(Vec2::new(6.352, 7.809))),
            "you 1.50, 8.50  ·  cursor 6.35, 7.81"
        );
        assert_eq!(coords_label(Vec2::new(1.5, 8.5), None), "you 1.50, 8.50");
    }
}
