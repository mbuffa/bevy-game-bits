use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;
use bevy_hanabi::HanabiPlugin;

use crate::config::*;
use crate::{
    construction, cover, crops, daynight, debug_ui, director, fire, flora, history, items, map,
    movement, nav, needs, power, scene, selection, shrubs, snow, temperature, trees, ui, units,
    weather, zones,
};

/// Whether the simulation is advancing. Toggled by Space (`toggle_pause`);
/// gates the "active" systems (movement, growth, the whole Director) so the
/// player can select/order/allow things without the world moving under
/// them. Selection, UI, and orders (they take effect on resume) stay live.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SimState {
    #[default]
    Running,
    Paused,
}

/// Space bar flips `SimState`.
pub fn toggle_pause(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<SimState>>,
    mut next: ResMut<NextState<SimState>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        next.set(match state.get() {
            SimState::Running => SimState::Paused,
            SimState::Paused => SimState::Running,
        });
    }
}

/// Shared mesh/material handles, created once at startup. Every tile shares
/// one mesh and one of two sand materials, so Bevy batches the whole map.
#[derive(Resource)]
pub struct GameAssets {
    pub tile_mesh: Handle<Mesh>,
    /// Thick cuboid for ground tiles and riverbeds: its side faces double as
    /// the riverbanks next to recessed water (see GROUND_THICKNESS).
    pub ground_mesh: Handle<Mesh>,
    /// Autotiled shore variants of `ground_mesh`, indexed by the cut-corner
    /// bitmask (see `map::shore_cut_mask` / `map::sync_shore_corners`);
    /// mask 0 stays the plain `ground_mesh`.
    pub ground_shore_meshes: [Handle<Mesh>; 16],
    /// Matching chamfered variants of `tile_mesh` for grass cover on shore
    /// cells, so the overlay doesn't overhang a cut corner
    /// (`cover::sync_shore_cover`).
    pub cover_shore_meshes: [Handle<Mesh>; 16],
    /// The merged shore-wedge mesh (dirt tips + bed floors along beveled
    /// shores), regenerated in place by `map::rebuild_shore_wedges`.
    pub shore_wedge_mesh: Handle<Mesh>,
    /// White + vertex colors: the wedge mesh carries the dirt checker/bed
    /// tints per vertex, so one material covers all wedges.
    pub shore_wedge_material: Handle<StandardMaterial>,
    /// The merged translucent water surface, regenerated in place by
    /// `map::rebuild_water_surface` (starts empty; LDtk beds arrive later).
    pub water_surface_mesh: Handle<Mesh>,
    pub dirt_material_a: Handle<StandardMaterial>,
    pub dirt_material_b: Handle<StandardMaterial>,
    pub fertile_material: Handle<StandardMaterial>,
    /// Flora overlay materials: 4 coverage buckets x the checker pair,
    /// mixed from the dirt colors toward the grass colors (bucket 3 == the
    /// full grass colors).
    pub grass_cover_materials: [[Handle<StandardMaterial>; 2]; 4],
    /// Algae's reed-cluster mesh (`cover::algae_reed_mesh`), scaled in Y by
    /// coverage on spawn/update.
    pub algae_reed_mesh: Handle<Mesh>,
    /// Wind-swaying reed material, same construction as `crop_material`
    /// (fed each frame by `weather::sync_wind_material`).
    pub algae_reed_material: Handle<weather::WindSwayMaterial>,
    /// The water surface: StandardMaterial translucency/gloss plus the
    /// ripple shader (`map::WaterExtension`), fed each frame by
    /// `weather::sync_water_material`. Base color is white — the surface
    /// mesh's vertex colors carry the depth tint and shore fade.
    pub water_material: Handle<map::WaterMaterial>,
    pub water_bed_material: Handle<StandardMaterial>,
    /// The merged bed-floor-wedge mesh (fills the shallow/deep bevel's cut
    /// corners), regenerated in place by `map::rebuild_bed_wedges`. Reuses
    /// `water_bed_material` — one uniform color regardless of depth, no
    /// vertex colors needed.
    pub bed_wedge_mesh: Handle<Mesh>,
    pub wall_mesh: Handle<Mesh>,
    /// Autotiled connected-wall meshes, indexed by neighbor bitmask (see
    /// `construction::connected_wall_mesh`). `wall_mesh` above stays the
    /// blueprint ghost's simple post.
    pub wall_meshes: [Handle<Mesh>; 16],
    pub wall_material: Handle<StandardMaterial>,
    pub stockpile_material: Handle<StandardMaterial>,
    pub growing_zone_material: Handle<StandardMaterial>,
    pub zone_preview_material: Handle<StandardMaterial>,
    pub pawn_mesh: Handle<Mesh>,
    pub pawn_material: Handle<StandardMaterial>,
    pub bush_mesh: Handle<Mesh>,
    pub bush_material: Handle<StandardMaterial>,
    pub bush_ripe_material: Handle<StandardMaterial>,
    /// Shrub: a Y-squashed sphere — a low dome, shape-distinct from the
    /// bush's full sphere.
    pub shrub_mesh: Handle<Mesh>,
    pub shrub_material: Handle<StandardMaterial>,
    /// Plants vs fire: ember-glow (emissive) swaps while the cell burns,
    /// and the charred trunk a burnt tree keeps as a snag.
    pub shrub_burning_material: Handle<StandardMaterial>,
    pub tree_canopy_burning_material: Handle<StandardMaterial>,
    pub tree_trunk_charred_material: Handle<StandardMaterial>,
    pub item_stack_mesh: Handle<Mesh>,
    pub berry_stack_material: Handle<StandardMaterial>,
    pub wheat_stack_material: Handle<StandardMaterial>,
    pub wood_stack_material: Handle<StandardMaterial>,
    pub crop_mesh: Handle<Mesh>,
    pub crop_material: Handle<weather::WindSwayMaterial>,
    pub tree_trunk_mesh: Handle<Mesh>,
    pub tree_trunk_material: Handle<StandardMaterial>,
    pub tree_canopy_mesh: Handle<Mesh>,
    pub tree_canopy_material: Handle<StandardMaterial>,
    /// Wet-ground overlay shades, one per humidity bucket (1..=4).
    pub wet_overlay_materials: [Handle<StandardMaterial>; 4],
    /// Snow-cover overlay shades, one per snow bucket (1..=4).
    pub snow_overlay_materials: [Handle<StandardMaterial>; 4],
    /// Wind-shadow debug-overlay shades (GROUND band), one per shelter
    /// bucket (1..=4).
    pub wind_overlay_materials: [Handle<StandardMaterial>; 4],
    /// Altitude-wind debug-overlay shades, one per shelter bucket (1..=4) —
    /// the `wind_overlay_materials` twin for `WindBand::Altitude`.
    pub altitude_wind_overlay_materials: [Handle<StandardMaterial>; 4],
    /// Fuel debug-overlay shades, one per fuel bucket (1..=4).
    pub fuel_overlay_materials: [Handle<StandardMaterial>; 4],
    /// Temperature debug overlay: one tint per cold→warm bucket.
    pub temp_overlay_materials: [Handle<StandardMaterial>; 5],
    /// Construction: blueprint ghosts share the wall color at low alpha;
    /// doors are two posts and a lintel (shape-distinct from walls).
    pub blueprint_material: Handle<StandardMaterial>,
    pub door_post_mesh: Handle<Mesh>,
    pub door_lintel_mesh: Handle<Mesh>,
    /// Wind turbine: tower pole, nacelle housing, and a blade whose root
    /// sits at the mesh origin so rotor children pivot around the hub.
    pub turbine_tower_mesh: Handle<Mesh>,
    pub turbine_nacelle_mesh: Handle<Mesh>,
    pub turbine_blade_mesh: Handle<Mesh>,
    pub turbine_tower_material: Handle<StandardMaterial>,
    pub turbine_nacelle_material: Handle<StandardMaterial>,
    pub turbine_blade_material: Handle<StandardMaterial>,
    /// Solar panel: a short central post, a backing frame, and a small grid
    /// of cells whose base yaws to track the sun's azimuth
    /// (`construction::SolarPanelPivot`) — a single-axis tracker, fixed-tilt
    /// on top. The first building bigger than one tile — centered on the
    /// panel's full 2x2 footprint.
    pub solar_post_mesh: Handle<Mesh>,
    pub solar_frame_mesh: Handle<Mesh>,
    pub solar_cell_mesh: Handle<Mesh>,
    pub solar_post_material: Handle<StandardMaterial>,
    pub solar_panel_material: Handle<StandardMaterial>,
    /// Lightpost: a slim pole and a lamp-head globe. The lamp material is
    /// shared and mutated live (`construction::sync_lightposts`) rather than
    /// swapped, so every built lightpost's glow fades together.
    pub lightpost_pole_mesh: Handle<Mesh>,
    pub lightpost_lamp_mesh: Handle<Mesh>,
    pub lightpost_pole_material: Handle<StandardMaterial>,
    pub lightpost_lamp_material: Handle<StandardMaterial>,
    /// Battery: a boxy body plus a charge-indicator strip on its front face.
    /// The strip's material swaps between 4 precomputed empty->full buckets
    /// (`construction::sync_batteries`), the same coverage-overlay idiom
    /// `grass_cover_materials`/`temp_overlay_materials` use — no material is
    /// ever created at runtime.
    pub battery_body_mesh: Handle<Mesh>,
    pub battery_strip_mesh: Handle<Mesh>,
    pub battery_body_material: Handle<StandardMaterial>,
    pub battery_strip_materials: [Handle<StandardMaterial>; 4],
    /// Bed: a flat 2-tile slab a tired pawn walks onto and rests atop, sized
    /// for a lying pawn's length (see `PAWN_HEIGHT`). One mesh per
    /// orientation (no runtime rotation of built structures) and one shared
    /// material — the simplest building after Lightpost.
    pub bed_mesh_horizontal: Handle<Mesh>,
    pub bed_mesh_vertical: Handle<Mesh>,
    pub bed_material: Handle<StandardMaterial>,
    /// Cooler: a wall-sized body (reuses `wall_mesh`'s exact dimensions —
    /// it stands in a wall run) plus a small vent block on its room-facing
    /// side so it reads as machinery, not a plain wall panel.
    pub cooler_vent_mesh: Handle<Mesh>,
    pub cooler_body_material: Handle<StandardMaterial>,
    pub cooler_vent_material: Handle<StandardMaterial>,
    /// Fire overlays (always shown — game state, not devtools): burning
    /// orange by intensity, scorch dark by severity.
    pub burning_overlay_materials: [Handle<StandardMaterial>; 4],
    pub scorch_overlay_materials: [Handle<StandardMaterial>; 4],
    /// Roof panels: one merged surface over all built roofs, a fainter
    /// merged surface of inset squares over all planned ones. Both meshes
    /// are empty `TriangleList`s regenerated by `construction::rebuild_roof_surfaces`
    /// (mirrors `water_surface_mesh`).
    pub roof_material: Handle<StandardMaterial>,
    pub roof_ghost_material: Handle<StandardMaterial>,
    pub roof_mesh: Handle<Mesh>,
    pub roof_ghost_mesh: Handle<Mesh>,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(LdtkPlugin)
            .add_plugins(HanabiPlugin)
            .add_plugins(MaterialPlugin::<weather::WindSwayMaterial>::default())
            .add_plugins(MaterialPlugin::<map::WaterMaterial>::default())
            .insert_resource(LevelSelection::index(0))
            .register_ldtk_int_cell::<map::DirtCellBundle>(DIRT)
            .register_ldtk_int_cell_for_layer::<zones::StockpileZoneCellBundle>("Zones", STOCKPILE)
            .register_ldtk_int_cell::<map::ShallowWaterCellBundle>(SHALLOW_WATER)
            .register_ldtk_int_cell::<map::DeepWaterCellBundle>(DEEP_WATER)
            .register_ldtk_int_cell::<map::FertileCellBundle>(FERTILE)
            .register_ldtk_int_cell::<map::GrassCellBundle>(GRASS)
            .init_resource::<nav::NavGrid>()
            .init_resource::<movement::Occupancy>()
            .init_resource::<map::Stockpile>()
            .init_resource::<map::TerrainMap>()
            .init_resource::<construction::ConstructionMap>()
            .init_resource::<construction::ConstructionBlocks>()
            .init_resource::<map::WaterMap>()
            .init_resource::<map::RoofMap>()
            .init_resource::<map::ShowSeams>()
            .init_resource::<cover::CoverMap>()
            .register_ldtk_entity::<units::PawnSpawnBundle>("Pawn")
            .register_ldtk_entity::<flora::BushSpawnBundle>("Bush")
            .register_ldtk_entity::<trees::TreeSpawnBundle>("Tree")
            .register_ldtk_entity::<construction::WallSpawnBundle>("Wall")
            .init_resource::<selection::SelectedEntity>()
            .init_resource::<director::WanderRng>()
            .init_resource::<zones::ActiveTool>()
            .init_resource::<zones::DragAnchor>()
            .init_resource::<ui::OpenCategory>()
            .init_resource::<scene::CameraControls>()
            .init_resource::<scene::CameraViewMode>()
            .init_resource::<daynight::GameClock>()
            .init_resource::<daynight::GameSpeed>()
            .init_resource::<daynight::ShowSkyBodies>()
            .init_resource::<weather::Weather>()
            .init_resource::<weather::Wind>()
            .init_resource::<weather::HumidityMap>()
            .init_resource::<temperature::ActiveSeason>()
            .init_resource::<temperature::TemperatureMap>()
            .init_resource::<temperature::ShowTemperatureOverlay>()
            .init_resource::<snow::SnowMap>()
            .init_resource::<snow::FreezeLevel>()
            .init_resource::<weather::WindExposureMap>()
            .init_resource::<weather::ShowWindOverlay>()
            .init_resource::<weather::ShowAltitudeWindOverlay>()
            .init_resource::<weather::ShowFuelOverlay>()
            .init_resource::<fire::FireMap>()
            .init_resource::<fire::ScorchMap>()
            .init_resource::<fire::FireTickTimer>()
            .init_resource::<power::PowerGrid>()
            .init_resource::<debug_ui::ActiveDebugTab>()
            .init_resource::<ui::StockUiState>()
            .init_resource::<ui::ActivePawnTab>()
            .init_state::<SimState>()
            .add_message::<flora::HarvestCommand>()
            .add_message::<trees::CutCommand>()
            .add_message::<fire::BurnCommand>()
            .add_message::<construction::BuildCommand>()
            .add_message::<construction::RoofCommand>()
            .add_message::<construction::DemolishCommand>()
            .add_message::<history::JobCompleted>()
            .add_systems(
                Startup,
                (
                    setup_assets,
                    scene::setup,
                    daynight::setup,
                    weather::setup,
                    weather::setup_gusts,
                    fire::setup,
                    ui::setup,
                    ui::spawn_compass_labels,
                    debug_ui::setup,
                    map::spawn_ldtk_world,
                )
                    .chain(),
            )
            .add_systems(OnEnter(SimState::Paused), weather::pause_rain)
            .add_systems(OnExit(SimState::Paused), weather::resume_rain)
            .add_systems(
                Update,
                (
                    toggle_pause,
                    // LDtk data -> 3D view bridges and lookup grids.
                    (
                        map::spawn_tile_visuals,
                        construction::spawn_walls_from_ldtk,
                        items::spawn_starter_wood,
                        // Beds first: the visuals and nav costs of the
                        // same-frame LDtk cells read them. Then the bodies
                        // (one-shot flood-fill once the cells exist) and
                        // the three level-followers — surfaces, nav costs,
                        // algae — which no-op unless the WaterMap changed
                        // (startup seeding or a water-level devtool click).
                        (
                            map::mark_water_map,
                            map::spawn_water_visuals,
                            nav::mark_water,
                            map::build_water_bodies,
                            map::rebuild_water_surface,
                            map::rebuild_shore_wedges,
                            map::rebuild_bed_wedges,
                            nav::sync_water_costs,
                            cover::sync_algae_to_water,
                            map::sync_shore_corners,
                            map::sync_bed_corners,
                            cover::sync_shore_cover,
                        )
                            .chain(),
                        map::spawn_fertile_visuals,
                        map::spawn_grass_visuals,
                        map::mark_terrain,
                        (
                            cover::seed_grass_from_ldtk,
                            cover::seed_algae,
                            shrubs::seed_shrubs,
                        ),
                        cover::sync_cover_map,
                        nav::mark_trees,
                        trees::spawn_tree_visual,
                        trees::arm_seed_trees,
                        units::spawn_pawn_visual,
                        flora::spawn_bush_visual,
                        zones::seed_zones_from_ldtk,
                        zones::rebuild_zone_visuals,
                        zones::sync_stockpile_cells,
                    ),
                    (
                        director::choose_objectives,
                        director::generate_jobs,
                        director::generate_walk_jobs,
                        director::generate_sleep_jobs,
                        director::cleanup_jobs,
                        director::enforce_allowances,
                        director::release_manual_pawns,
                        director::release_sleeping_pawns,
                        director::tick_stuck,
                        director::assign_jobs,
                        director::execute_jobs,
                        director::deliver_carried,
                        director::update_pawn_status,
                    )
                        .chain()
                        .run_if(in_state(SimState::Running)),
                    (director::sync_carry_visual, director::sync_sleeping_pose),
                    director::tick_wander_cooldowns.run_if(in_state(SimState::Running)),
                    (
                        construction::sync_construction_collision,
                        movement::rebuild_occupancy,
                        movement::plan_paths,
                        movement::move_along_path,
                        movement::sync_grid_coords,
                    )
                        .chain()
                        .run_if(in_state(SimState::Running)),
                    movement::debug_draw_paths,
                    (
                        flora::grow_bushes,
                        flora::harvest_plants,
                        crops::grow_crops,
                        trees::grow_trees,
                        trees::spread_trees,
                        shrubs::grow_shrubs,
                        shrubs::spread_shrubs,
                        trees::fell_trees,
                        cover::grow_cover,
                        cover::spread_cover,
                        // Freeze before snow (snow reads `is_frozen` to know
                        // what holds it), snow before humidity (melting
                        // cells soak the ground the same frame).
                        (
                            temperature::update_temperature,
                            snow::update_freeze,
                            snow::update_snow,
                            weather::update_humidity,
                        )
                            .chain(),
                        (
                            weather::update_wind,
                            weather::update_wind_exposure,
                            power::tick_power,
                            fire::tick_fire,
                        )
                            .chain(),
                        fire::heal_scorch,
                        needs::decay_needs,
                    )
                        .run_if(in_state(SimState::Running)),
                    (cover::update_cover_visuals, cover::sync_algae_scale),
                    (
                        weather::move_gust_heads,
                        construction::yaw_turbines,
                        construction::spin_turbines,
                        construction::aim_solar_panels,
                        construction::sync_lightposts.after(power::tick_power),
                        construction::sync_batteries.after(power::tick_power),
                    )
                        .run_if(in_state(SimState::Running)),
                    (
                        weather::sync_precipitation_effects,
                        weather::sync_rain_wind,
                        weather::sync_humidity_overlay,
                        (snow::sync_snow_overlay, snow::sync_snow_costs),
                        weather::sync_wind_material,
                        weather::sync_water_material,
                        weather::sync_wind_overlay,
                        weather::sync_altitude_wind_overlay,
                        weather::sync_fuel_overlay,
                        temperature::sync_temperature_overlay,
                        map::sync_seamless_dirt,
                        cover::sync_seamless_grass,
                        fire::sync_burning_overlay,
                        fire::sync_scorch_overlay,
                        fire::sync_fire_emitters,
                        fire::sync_fire_wind,
                        // Ungated on purpose: BurnCommands also come from the
                        // ungated Extinguish-all devtool, and a gated reader
                        // could drop them across a pause (the complete_builds
                        // rule). The material syncs just follow the FireMap.
                        trees::burn_trees,
                        shrubs::burn_shrubs,
                        trees::sync_tree_materials,
                        shrubs::sync_shrub_materials,
                    ),
                    // Ungated on purpose: completion messages are written by
                    // the sim-gated director, and an ungated reader can't
                    // drop one across a pause; cancels work while paused
                    // like zone edits do; the roof visuals just follow the
                    // RoofMap.
                    (
                        construction::complete_builds,
                        // The Demolish job's completion — same ungated
                        // reasoning as complete_builds: DemolishCommand is
                        // written by the sim-gated director.
                        construction::do_demolitions,
                        // Rooms follow the built layout (builds, demolition,
                        // the LDtk walls), so re-derive right after it moves.
                        temperature::sync_rooms
                            .after(construction::complete_builds)
                            .after(construction::do_demolitions),
                        construction::apply_roof_commands,
                        construction::cancel_blueprints,
                        construction::rebuild_roof_surfaces,
                        construction::sync_wall_connections,
                        history::record_history,
                    ),
                    (
                        daynight::apply_game_speed,
                        daynight::tick_clock.run_if(in_state(SimState::Running)),
                        daynight::apply_lighting,
                        daynight::move_sky_bodies,
                        daynight::paint_sky,
                        daynight::paint_sun_ball,
                        daynight::sync_sky_body_visibility,
                    ),
                    (zones::drag_zone_tool, construction::update_build_ghost).chain(),
                    (
                        // F2 top-down/isometric swap, then the controls it
                        // gates: orbit only makes sense isometric (locked
                        // north-up in top-down); zoom/pan work in both but
                        // freeze during the ~0.35s animated swap.
                        scene::toggle_camera_view,
                        scene::orbit_camera.run_if(scene::iso_view_active),
                        scene::zoom_camera.run_if(scene::camera_settled),
                        scene::pan_camera.run_if(scene::camera_settled),
                    ),
                    selection::click_select.run_if(zones::tool_inactive),
                    selection::click_move.run_if(zones::tool_inactive),
                    (
                        ui::sync_indicator,
                        ui::update_panel,
                        ui::update_actions,
                        ui::run_actions,
                        ui::run_general_actions,
                        ui::sync_zone_selection,
                        ui::update_job_panel,
                        ui::spawn_pawn_labels,
                        ui::sync_pawn_labels,
                        ui::sync_compass_labels,
                        ui::spawn_allowance_rows,
                        ui::toggle_allowance,
                        ui::sync_allowance_buttons,
                        ui::update_tooltip,
                        ui::sync_pause_banner,
                        (
                            ui::sync_grow_checkbox,
                            ui::toggle_grow_order,
                            ui::sync_plant_button,
                            ui::cycle_zone_plant,
                            ui::sync_manual_checkbox,
                            ui::toggle_manual_mode,
                            ui::sync_needs_panel,
                            ui::sync_target_temp_row,
                            ui::run_target_temp_buttons,
                            // Bundled here (rather than as their own
                            // top-level entries): the outer tuple is at the
                            // 20-element system-tuple cap.
                            ui::run_pawn_tab_buttons,
                            ui::sync_pawn_tabs,
                            ui::sync_job_history_tab,
                            ui::sync_skills_tab,
                            ui::sync_needs_tab,
                            ui::scroll_history,
                            ui::run_category_buttons,
                            ui::sync_category_menu,
                            ui::close_category_menu,
                        ),
                        // Bundled with the speed row: the outer tuple is at
                        // the 20-element system-tuple cap.
                        (
                            ui::update_clock_label,
                            ui::run_speed_buttons,
                            ui::run_speed_pause_button,
                            ui::sync_speed_buttons,
                        ),
                        (ui::run_stock_rows, ui::sync_stock_panel),
                        (ui::spawn_portraits, ui::run_portraits, ui::sync_portraits),
                        (
                            debug_ui::toggle_debug_window,
                            debug_ui::run_tab_buttons,
                            debug_ui::sync_tabs,
                            debug_ui::update_jobs_tab,
                            debug_ui::run_rain_toggle,
                            debug_ui::sync_rain_toggle,
                            (
                                debug_ui::run_water_level_buttons,
                                debug_ui::sync_water_level_label,
                                debug_ui::run_hour_buttons,
                                debug_ui::update_hour_clock_text,
                            ),
                            debug_ui::run_wind_toggle,
                            debug_ui::sync_wind_toggle,
                            debug_ui::run_wind_shift,
                            // Bundled: the outer tuple is at the 20-element
                            // system-tuple cap.
                            (
                                debug_ui::run_wind_overlay_toggle,
                                debug_ui::sync_wind_overlay_toggle,
                                debug_ui::run_altitude_wind_overlay_toggle,
                                debug_ui::sync_altitude_wind_overlay_toggle,
                            ),
                            // Bundled: the flat tuple is at the 20-element
                            // system-tuple cap.
                            (
                                debug_ui::run_fuel_overlay_toggle,
                                debug_ui::sync_fuel_overlay_toggle,
                                debug_ui::run_temp_overlay_toggle,
                                debug_ui::sync_temp_overlay_toggle,
                                debug_ui::run_season_toggle,
                                debug_ui::sync_season_toggle,
                            ),
                            debug_ui::run_ignite_button,
                            debug_ui::sync_ignite_button,
                            debug_ui::run_extinguish_all,
                            debug_ui::run_demolish_button,
                            debug_ui::sync_demolish_button,
                            (
                                debug_ui::run_sky_bodies_toggle,
                                debug_ui::sync_sky_bodies_toggle,
                                debug_ui::update_power_tab,
                                debug_ui::run_seamless_toggle,
                                debug_ui::sync_seamless_toggle,
                            ),
                        ),
                    ),
                ),
            );
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut sway_materials: ResMut<Assets<weather::WindSwayMaterial>>,
    mut water_materials: ResMut<Assets<map::WaterMaterial>>,
) {
    commands.insert_resource(GameAssets {
        tile_mesh: meshes.add(Cuboid::new(TILE_SIZE, TILE_THICKNESS, TILE_SIZE)),
        ground_mesh: meshes.add(Cuboid::new(TILE_SIZE, GROUND_THICKNESS, TILE_SIZE)),
        ground_shore_meshes: std::array::from_fn(|mask| {
            meshes.add(map::shore_dirt_mesh(mask as u8))
        }),
        cover_shore_meshes: std::array::from_fn(|mask| {
            meshes.add(map::shore_overlay_mesh(mask as u8))
        }),
        shore_wedge_mesh: meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )),
        shore_wedge_material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        }),
        water_surface_mesh: meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )),
        dirt_material_a: materials.add(StandardMaterial {
            base_color: DIRT_COLOR_A,
            perceptual_roughness: 1.0,
            ..default()
        }),
        dirt_material_b: materials.add(StandardMaterial {
            base_color: DIRT_COLOR_B,
            perceptual_roughness: 1.0,
            ..default()
        }),
        fertile_material: materials.add(StandardMaterial {
            base_color: FERTILE_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        grass_cover_materials: std::array::from_fn(|bucket| {
            let t = (bucket as f32 + 1.0) / 4.0;
            [(DIRT_COLOR_A, GRASS_COLOR_A), (DIRT_COLOR_B, GRASS_COLOR_B)].map(|(from, to)| {
                materials.add(StandardMaterial {
                    base_color: from.mix(&to, t),
                    perceptual_roughness: 1.0,
                    ..default()
                })
            })
        }),
        algae_reed_mesh: meshes.add(cover::algae_reed_mesh()),
        // Reeds sway like wheat: same `WindSwayMaterial` extension, fed each
        // frame by `weather::sync_wind_material`.
        algae_reed_material: sway_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: ALGAE_COLOR,
                perceptual_roughness: 0.6,
                ..default()
            },
            extension: weather::WindSwayExtension::default(),
        }),
        water_material: water_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: WATER_ROUGHNESS,
                reflectance: WATER_REFLECTANCE,
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
            extension: map::WaterExtension::default(),
        }),
        water_bed_material: materials.add(StandardMaterial {
            base_color: WATER_BED_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        bed_wedge_mesh: meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )),
        // Half-tile footprint: less blocky, and the cell still blocks
        // pathfinding entirely (the shrink is purely visual).
        wall_mesh: meshes.add(Cuboid::new(WALL_VISUAL_SIZE, WALL_HEIGHT, WALL_VISUAL_SIZE)),
        wall_meshes: std::array::from_fn(|mask| {
            meshes.add(construction::connected_wall_mesh(mask as u8))
        }),
        wall_material: materials.add(StandardMaterial {
            base_color: WALL_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        stockpile_material: materials.add(StandardMaterial {
            base_color: STOCKPILE_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        growing_zone_material: materials.add(StandardMaterial {
            base_color: GROWING_ZONE_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        zone_preview_material: materials.add(StandardMaterial {
            base_color: STOCKPILE_COLOR.with_alpha(ZONE_PREVIEW_ALPHA),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        }),
        pawn_mesh: meshes.add(Cuboid::new(PAWN_WIDTH, PAWN_HEIGHT, PAWN_WIDTH)),
        pawn_material: materials.add(StandardMaterial {
            base_color: PAWN_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        bush_mesh: meshes.add(Sphere::new(BUSH_RADIUS)),
        bush_material: materials.add(StandardMaterial {
            base_color: BUSH_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        bush_ripe_material: materials.add(StandardMaterial {
            base_color: BUSH_RIPE_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        shrub_mesh: meshes.add(Mesh::from(Sphere::new(SHRUB_RADIUS)).scaled_by(Vec3::new(
            1.0,
            SHRUB_FLATTEN,
            1.0,
        ))),
        shrub_material: materials.add(StandardMaterial {
            base_color: SHRUB_COLOR,
            perceptual_roughness: 0.95,
            ..default()
        }),
        shrub_burning_material: materials.add(StandardMaterial {
            base_color: SHRUB_BURNING_COLOR,
            emissive: BURNING_PLANT_EMISSIVE,
            perceptual_roughness: 0.95,
            ..default()
        }),
        item_stack_mesh: meshes.add(Cuboid::new(
            ITEM_STACK_SIZE,
            ITEM_STACK_HEIGHT,
            ITEM_STACK_SIZE,
        )),
        berry_stack_material: materials.add(StandardMaterial {
            base_color: BERRY_STACK_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        wheat_stack_material: materials.add(StandardMaterial {
            base_color: WHEAT_STACK_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        wood_stack_material: materials.add(StandardMaterial {
            base_color: WOOD_STACK_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        crop_mesh: meshes.add(crops::wheat_cluster_mesh()),
        // The one non-StandardMaterial in the game: wheat's wind-sway
        // extension keeps the same PBR base and adds a bending vertex
        // shader, fed each frame by `weather::sync_wind_material`.
        crop_material: sway_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: CROP_COLOR,
                perceptual_roughness: 0.9,
                ..default()
            },
            extension: weather::WindSwayExtension::default(),
        }),
        tree_trunk_mesh: meshes.add(Cuboid::new(
            TREE_TRUNK_SIZE,
            TREE_TRUNK_HEIGHT,
            TREE_TRUNK_SIZE,
        )),
        tree_trunk_material: materials.add(StandardMaterial {
            base_color: TREE_TRUNK_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        tree_canopy_mesh: meshes.add(Cone {
            radius: TREE_CANOPY_RADIUS,
            height: TREE_CANOPY_HEIGHT,
        }),
        tree_canopy_material: materials.add(StandardMaterial {
            base_color: TREE_CANOPY_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        tree_canopy_burning_material: materials.add(StandardMaterial {
            base_color: TREE_CANOPY_BURNING_COLOR,
            emissive: BURNING_PLANT_EMISSIVE,
            perceptual_roughness: 0.9,
            ..default()
        }),
        tree_trunk_charred_material: materials.add(StandardMaterial {
            base_color: TREE_TRUNK_CHARRED_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        }),
        wet_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: WET_OVERLAY_COLOR.with_alpha(WET_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        snow_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: SNOW_OVERLAY_COLOR.with_alpha(SNOW_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        wind_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: WIND_OVERLAY_COLOR.with_alpha(WIND_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        altitude_wind_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: WIND_ALTITUDE_OVERLAY_COLOR.with_alpha(WIND_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        fuel_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: FUEL_OVERLAY_COLOR.with_alpha(FUEL_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        temp_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: TEMP_OVERLAY_COLORS[bucket].with_alpha(TEMP_OVERLAY_ALPHA),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        burning_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: BURNING_OVERLAY_COLOR.with_alpha(BURNING_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        scorch_overlay_materials: std::array::from_fn(|bucket| {
            materials.add(StandardMaterial {
                base_color: SCORCH_OVERLAY_COLOR.with_alpha(SCORCH_OVERLAY_ALPHAS[bucket]),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 1.0,
                ..default()
            })
        }),
        blueprint_material: materials.add(StandardMaterial {
            base_color: WALL_COLOR.with_alpha(BLUEPRINT_ALPHA),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        }),
        door_post_mesh: meshes.add(Cuboid::new(DOOR_POST_SIZE, WALL_HEIGHT, DOOR_POST_SIZE)),
        door_lintel_mesh: meshes.add(Cuboid::new(
            DOOR_LINTEL_LENGTH,
            DOOR_POST_SIZE,
            DOOR_POST_SIZE,
        )),
        turbine_tower_mesh: meshes.add(Cuboid::new(
            TURBINE_TOWER_SIZE,
            TURBINE_TOWER_HEIGHT,
            TURBINE_TOWER_SIZE,
        )),
        turbine_nacelle_mesh: meshes.add(Cuboid::new(
            TURBINE_NACELLE_LENGTH,
            TURBINE_NACELLE_SIZE,
            TURBINE_NACELLE_SIZE,
        )),
        // Root at the origin: rotor children pivot blades around the hub.
        turbine_blade_mesh: meshes.add(
            Mesh::from(Cuboid::new(
                TURBINE_BLADE_THICKNESS,
                TURBINE_BLADE_LENGTH,
                TURBINE_BLADE_WIDTH,
            ))
            .translated_by(Vec3::Y * TURBINE_BLADE_LENGTH / 2.0),
        ),
        turbine_tower_material: materials.add(StandardMaterial {
            base_color: TURBINE_TOWER_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        turbine_nacelle_material: materials.add(StandardMaterial {
            base_color: TURBINE_NACELLE_COLOR,
            perceptual_roughness: 0.7,
            ..default()
        }),
        turbine_blade_material: materials.add(StandardMaterial {
            base_color: TURBINE_BLADE_COLOR,
            perceptual_roughness: 0.5,
            ..default()
        }),
        solar_post_mesh: meshes.add(Cuboid::new(
            SOLAR_POST_SIZE,
            SOLAR_POST_HEIGHT,
            SOLAR_POST_SIZE,
        )),
        solar_frame_mesh: meshes.add(Cuboid::new(
            SOLAR_ARRAY_COLS as f32 * (SOLAR_CELL_SIZE + SOLAR_CELL_GAP) + SOLAR_CELL_GAP,
            SOLAR_FRAME_THICKNESS,
            SOLAR_ARRAY_ROWS as f32 * (SOLAR_CELL_SIZE + SOLAR_CELL_GAP) + SOLAR_CELL_GAP,
        )),
        solar_cell_mesh: meshes.add(Cuboid::new(
            SOLAR_CELL_SIZE,
            SOLAR_CELL_THICKNESS,
            SOLAR_CELL_SIZE,
        )),
        solar_post_material: materials.add(StandardMaterial {
            base_color: SOLAR_POST_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        solar_panel_material: materials.add(StandardMaterial {
            base_color: SOLAR_PANEL_COLOR,
            perceptual_roughness: 0.3,
            reflectance: 0.4,
            ..default()
        }),
        lightpost_pole_mesh: meshes.add(Cuboid::new(
            LIGHTPOST_POLE_SIZE,
            LIGHTPOST_POLE_HEIGHT,
            LIGHTPOST_POLE_SIZE,
        )),
        lightpost_lamp_mesh: meshes.add(Sphere::new(LIGHTPOST_LAMP_RADIUS)),
        lightpost_pole_material: materials.add(StandardMaterial {
            base_color: LIGHTPOST_POLE_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        // Emissive starts at black (default); `sync_lightposts` ramps it up
        // toward `LIGHTPOST_LAMP_EMISSIVE` as the clock turns to night.
        lightpost_lamp_material: materials.add(StandardMaterial {
            base_color: LIGHTPOST_LAMP_COLOR,
            perceptual_roughness: 0.5,
            ..default()
        }),
        battery_body_mesh: meshes.add(Cuboid::new(
            BATTERY_BODY_SIZE,
            BATTERY_BODY_HEIGHT,
            BATTERY_BODY_SIZE,
        )),
        battery_strip_mesh: meshes.add(Cuboid::new(
            BATTERY_STRIP_WIDTH,
            BATTERY_STRIP_HEIGHT,
            BATTERY_STRIP_THICKNESS,
        )),
        battery_body_material: materials.add(StandardMaterial {
            base_color: BATTERY_BODY_COLOR,
            perceptual_roughness: 0.7,
            ..default()
        }),
        battery_strip_materials: std::array::from_fn(|bucket| {
            let t = (bucket as f32 + 1.0) / 4.0;
            materials.add(StandardMaterial {
                base_color: BATTERY_STRIP_EMPTY_COLOR.mix(&BATTERY_STRIP_FULL_COLOR, t),
                perceptual_roughness: 0.4,
                ..default()
            })
        }),
        // Length along X for horizontal, along Z for vertical — width/height
        // swapped between the two rather than rotating one mesh at runtime.
        bed_mesh_horizontal: meshes.add(Cuboid::new(BED_LENGTH, BED_HEIGHT, BED_WIDTH)),
        bed_mesh_vertical: meshes.add(Cuboid::new(BED_WIDTH, BED_HEIGHT, BED_LENGTH)),
        bed_material: materials.add(StandardMaterial {
            base_color: BED_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        cooler_vent_mesh: meshes.add(Cuboid::new(
            COOLER_VENT_SIZE,
            COOLER_VENT_SIZE,
            COOLER_VENT_THICKNESS,
        )),
        cooler_body_material: materials.add(StandardMaterial {
            base_color: COOLER_BODY_COLOR,
            perceptual_roughness: 0.5,
            metallic: 0.3,
            ..default()
        }),
        cooler_vent_material: materials.add(StandardMaterial {
            base_color: COOLER_VENT_COLOR,
            perceptual_roughness: 0.3,
            ..default()
        }),
        roof_material: materials.add(StandardMaterial {
            base_color: ROOF_COLOR.with_alpha(ROOF_ALPHA),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        }),
        roof_ghost_material: materials.add(StandardMaterial {
            base_color: ROOF_COLOR.with_alpha(ROOF_GHOST_ALPHA),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        }),
        roof_mesh: meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )),
        roof_ghost_mesh: meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )),
    });
}
