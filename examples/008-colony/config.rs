use bevy::prelude::*;

/// Must match the Ground layer dimensions in `assets/maps/colony.ldtk`.
pub const MAP_WIDTH: i32 = 64;
pub const MAP_HEIGHT: i32 = 64;

/// One LDtk grid cell in world units.
pub const TILE_SIZE: f32 = 1.0;
pub const TILE_THICKNESS: f32 = 0.1;

/// N/E/S/W map labels (`ui::spawn_compass_labels`): how many tiles beyond
/// each map edge the letters sit, clear of terrain.
pub const COMPASS_MARGIN_TILES: i32 = 3;

/// IntGrid values of the Ground layer in `colony.ldtk` (natural terrain).
/// Value 2 (`Wall`) used to live here; walls are a placed `Entity` on the
/// Entities layer now, not IntGrid terrain paint — see
/// `construction::spawn_walls_from_ldtk`. `STOCKPILE` below (value 3) used
/// to live here too, but is painted on its own `Zones` IntGrid layer now —
/// see `map::Stockpile`.
pub const DIRT: i32 = 1;
/// Stockpile zone paint on the dedicated `Zones` IntGrid layer in
/// `colony.ldtk` (walkable; hauled goods get dropped here) — a stockpile
/// cell's Ground-layer terrain is plain `Dirt`, see `map::Stockpile`.
pub const STOCKPILE: i32 = 3;
/// River water: walkable but slow (see the TERRAIN_COST_* consts).
pub const SHALLOW_WATER: i32 = 4;
pub const DEEP_WATER: i32 = 5;
/// Fertile land: same as dirt for movement, but plants grow much faster on
/// it (see the FERTILITY consts below).
pub const FERTILE: i32 = 6;
/// Grass seed paint: substrate stays dirt; this value seeds a full-coverage
/// grass `Flora` entity on the cell (see `cover.rs`).
pub const GRASS: i32 = 7;

pub const WALL_HEIGHT: f32 = 0.8;
pub const WALL_COLOR: Color = Color::srgb(0.35, 0.22, 0.18);
pub const STOCKPILE_COLOR: Color = Color::srgb(0.45, 0.48, 0.55);
pub const FERTILE_COLOR: Color = Color::srgb(0.29, 0.22, 0.12);
pub const GROWING_ZONE_COLOR: Color = Color::srgb(0.32, 0.5, 0.24);

/// How fast things grow on a tile, as a multiplier on the base growth rate.
/// Dirt grows everything, just slowly; fertile land is the real deal.
pub const DIRT_FERTILITY: f32 = 0.2;
pub const FERTILE_FERTILITY: f32 = 1.0;

/// Editable zones (stockpiles today; more kinds later). Every zone stays a
/// rectangle: Expand grows to the bounding box of old union drawn, Shrink
/// clips to old intersect drawn.
pub const ZONE_PREVIEW_ALPHA: f32 = 0.5;
pub const ZONE_OUTLINE_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
pub const ZONE_OUTLINE_HEIGHT: f32 = 0.12;

/// Day/night cycle, in seconds of running sim time (paused time doesn't
/// count). One full cycle = day + night.
pub const DAY_LENGTH_SECS: f32 = 180.0;
pub const NIGHT_LENGTH_SECS: f32 = 60.0;
pub const CYCLE_LENGTH_SECS: f32 = DAY_LENGTH_SECS + NIGHT_LENGTH_SECS;
/// Daylight fades out over this window at the end of the day (dusk) and back
/// in over the same window at the end of the night (dawn).
pub const DUSK_DAWN_FADE_SECS: f32 = 8.0;

/// Scene lighting at full day vs. dead of night; `daynight::apply_lighting`
/// lerps between them by the current daylight factor. Night keeps a minimal
/// cool-blue ambient so the map stays readable.
pub const DAY_AMBIENT_BRIGHTNESS: f32 = 250.0;
pub const NIGHT_AMBIENT_BRIGHTNESS: f32 = 35.0;
pub const DAY_AMBIENT_COLOR: Color = Color::WHITE;
pub const NIGHT_AMBIENT_COLOR: Color = Color::srgb(0.55, 0.65, 1.0);
pub const SUN_ILLUMINANCE: f32 = 8_000.0;

/// Golden hour: the sun's color and strength ramp with its height on the
/// arc — warm orange and dimmer at the horizon, near-white at noon, ambient
/// counter-tinted toward cool twilight blue while the direct light is warm
/// (skylight dominates when the sun is low). Driven by `sin(π·day_progress)`
/// rather than `daylight()`, which is already 1.0 at sunrise (its dawn fade
/// runs pre-06:00, during the night phase) and so can't carry a sunrise
/// tint. The horizon color is deliberately less saturated than the sky-art
/// `SKY_DUSK_COLOR`: a *light* color tints everything it touches.
pub const SUN_COLOR_HORIZON: Color = Color::srgb(1.0, 0.60, 0.30);
/// Slightly warm white (~5500 K), not pure white.
pub const SUN_COLOR_NOON: Color = Color::srgb(1.0, 0.98, 0.94);
/// Exponent on the sun height in the color mix; < 1 concentrates the
/// warmth near the horizon (real atmospheric color shifts fastest there).
pub const SUN_COLOR_CURVE: f32 = 0.6;
/// Horizon sun strength as a fraction of the noon strength.
pub const SUN_HORIZON_ILLUMINANCE_FACTOR: f32 = 0.55;
/// Ambient shifts toward this cool blue at sunrise/sunset, mixed in up to
/// `TWILIGHT_AMBIENT_STRENGTH` when the sun sits on the horizon.
pub const TWILIGHT_AMBIENT_COLOR: Color = Color::srgb(0.6, 0.68, 1.0);
pub const TWILIGHT_AMBIENT_STRENGTH: f32 = 0.35;

/// PCSS softness for the sun: the apparent size (world units) of the light
/// disc, which sets how much the penumbra widens with distance from the
/// caster. `shadow_depth_bias` is raised above Bevy's default (0.02) to
/// offset the extra acne PCSS's wider sampling tends to expose.
pub const SUN_SOFT_SHADOW_SIZE: f32 = 1.5;
pub const SUN_SHADOW_DEPTH_BIAS: f32 = 0.08;

/// 4096 instead of Bevy's default 2048: our single shadow cascade (see
/// `scene::setup`) covers the map's whole camera-space depth span, so
/// doubling the texel grid roughly halves the size of the 1-texel "swim"
/// the shadow snaps by each time the day/night cycle's continuously
/// rotating sun crosses a texel boundary.
pub const SUN_SHADOW_MAP_SIZE: u32 = 4096;

/// How far (camera-space depth, world units) the sun's single shadow
/// cascade reaches. Must cover the farthest ground corner at the worst
/// pan, or everything beyond it renders unshadowed and the cutoff shows as
/// a horizontal brightness seam across the screen (constant camera depth =
/// a horizontal line in the isometric view). From the (d, d, d) camera the
/// depth of ground point (x, 0, z) is the component distances over √3, so
/// the far corner (-half, 0, -half) sits at (3d + 2·half)/√3, plus up to
/// 2·CAMERA_PAN_LIMIT of pan — ~161 on the 64-map, scaling with any
/// resize. Widening the cascade spreads its texels thinner, so if texel
/// swim returns, SUN_SHADOW_MAP_SIZE is the lever.
pub const SUN_SHADOW_MAX_DISTANCE: f32 =
    (3.0 * CAMERA_START_DISTANCE + MAP_WIDTH as f32 * TILE_SIZE + 2.0 * CAMERA_PAN_LIMIT)
        / 1.732_051;

/// The fake in-game wall clock: the 180 s day spans sunrise..sunset, the
/// 60 s night wraps through midnight back to sunrise.
pub const CLOCK_SUNRISE_HOUR: f32 = 6.0;
pub const CLOCK_SUNSET_HOUR: f32 = 22.0;

/// Spring ambient range: the night (and sunrise/sunset) base, and the noon
/// peak of `temperature::ambient_temperature`'s sine hump.
pub const SPRING_NIGHT_TEMP_C: f32 = 12.0;
pub const SPRING_DAY_TEMP_C: f32 = 20.0;

/// Winter ambient range. The night sits exactly on `FREEZE_FULL_BELOW_C` so
/// a winter night guarantees fully frozen (walkable) water bodies; the day
/// stays sub-zero, so winter precipitation is always snow and melting only
/// happens after switching back to a warm season.
pub const WINTER_NIGHT_TEMP_C: f32 = -20.0;
pub const WINTER_DAY_TEMP_C: f32 = -4.0;

/// How fast an uninsulated indoor cell drifts toward ambient (fraction of
/// the remaining gap per second, scaled down by the room's insulation):
/// τ ≈ 20 s bare, ≈ 100 s behind 80% wooden walls — a lag you can watch
/// against the 60 s night.
pub const INDOOR_TEMP_RATE_PER_SECOND: f32 = 0.05;

/// What a room's boundary cells contribute to its insulation score (the
/// average over the ring — see `temperature::room_insulation`). The one
/// wall kind is wooden for now, insulating well; a door seals notably worse.
pub const WALL_INSULATION: f32 = 0.8;
pub const DOOR_INSULATION: f32 = 0.25;

/// Temperature debug overlay: 5 tints from cold blue to warm red, bucketed
/// across the active season's night..day ambient range.
pub const TEMP_OVERLAY_COLORS: [Color; 5] = [
    Color::srgb(0.20, 0.40, 1.00),
    Color::srgb(0.30, 0.70, 0.95),
    Color::srgb(0.55, 0.85, 0.55),
    Color::srgb(0.95, 0.70, 0.25),
    Color::srgb(0.95, 0.30, 0.20),
];
pub const TEMP_OVERLAY_ALPHA: f32 = 0.30;
/// Above the fuel overlay (0.005): stacked debug overlays never z-fight.
pub const TEMP_OVERLAY_OFFSET: f32 = 0.006;

/// The sun and moon ride a semicircular arc over the map (rising east,
/// peaking `SKY_ORBIT_HEIGHT` above the center, setting west). The balls are
/// on render layer 1 — invisible to the main camera (a sun 10 m off the
/// ground breaks the world's scale); only the sky widget's camera sees them.
/// The arc still matters to the world: the directional light aims from the
/// sun's position, and the moon's point light rides it for the water glint.
pub const SKY_ORBIT_RADIUS: f32 = 20.0;
pub const SKY_ORBIT_HEIGHT: f32 = 12.0;
/// The arc's plane is tilted toward +Z (the camera-facing side) so the sun
/// never passes through zenith: it tops out at `SUN_MAX_ELEVATION` at noon.
/// Vertical noon light flattens the scene — shadows collapse to nothing and
/// the water ripples (lighting-only normal perturbation) lose all contrast
/// because N·L stops varying — so midday keeps a raking angle instead, the
/// standard isometric-game treatment. Tilting toward +Z (not -Z) keeps
/// `daynight::sun_yaw`'s solar-panel sweep honest: noon azimuth genuinely
/// IS the camera-facing side now.
pub const SUN_MAX_ELEVATION: f32 = 55.0 * std::f32::consts::PI / 180.0;
pub const SUN_ARC_TILT: f32 = std::f32::consts::FRAC_PI_2 - SUN_MAX_ELEVATION;
/// Ball sizes are tuned for widget legibility, not world scale.
pub const SUN_RADIUS: f32 = 2.5;
pub const SUN_COLOR: Color = Color::srgb(1.0, 0.88, 0.3);
pub const MOON_RADIUS: f32 = 2.0;
pub const MOON_COLOR: Color = Color::srgb(0.85, 0.9, 1.0);
/// Black ring around the sun and moon so they read against any sky color,
/// independent of the viewer's color perception.
pub const SKY_BODY_OUTLINE_WIDTH: f32 = 0.45;
pub const SKY_BODY_OUTLINE_COLOR: Color = Color::BLACK;

/// Sky-status widget: a second camera renders the arc (layer 1) to a small
/// texture shown above the clock. Its clear color is the sky itself —
/// lerped night<->day with a warm dusk tint blended in mid-fade.
pub const SKY_WIDGET_WIDTH: f32 = 132.0;
pub const SKY_WIDGET_HEIGHT: f32 = 44.0;
/// Offscreen texture at 2x the widget size for retina crispness.
pub const SKY_TEXTURE_WIDTH: u32 = 264;
pub const SKY_TEXTURE_HEIGHT: u32 = 88;
/// World-units framing of the sky camera; same 3:1 aspect as the widget.
/// Wide/tall enough for the whole arc including the ball radii.
pub const SKY_VIEW_WIDTH: f32 = 48.0;
pub const SKY_VIEW_HEIGHT: f32 = 16.0;
pub const SKY_DAY_COLOR: Color = Color::srgb(0.3, 0.52, 0.96);
pub const SKY_NIGHT_COLOR: Color = Color::srgb(0.04, 0.05, 0.12);
pub const SKY_DUSK_COLOR: Color = Color::srgb(0.95, 0.55, 0.25);
/// How strongly the dusk tint takes over at the middle of a fade (0..1).
pub const SKY_DUSK_STRENGTH: f32 = 0.6;
/// The moon carries a point light (not directional: its per-fragment light
/// direction is what draws a localized glint on the glossy water).
pub const MOON_LIGHT_COLOR: Color = Color::srgb(0.6, 0.7, 1.0);
pub const MOON_LIGHT_INTENSITY: f32 = 5_000_000.0;
pub const MOON_LIGHT_RANGE: f32 = 60.0;

/// Water is glossy (low roughness, raised reflectance) so the moon's point
/// light paints a localized specular glint that travels with it. Cheaper
/// than real reflections (SSR needs deferred rendering) and reads well on a
/// stylized flat-color scene. The sun glints too — a free bonus.
pub const WATER_ROUGHNESS: f32 = 0.12;
pub const WATER_REFLECTANCE: f32 = 0.6;

/// Water renders as an opaque riverbed recessed below ground plus one
/// translucent surface quad per cell — depth reads through the water.
/// Vertical order (world units): beds -0.35/-0.15 < surface at the body's
/// live level (starts -0.05) < algae just above the surface < ground top 0
/// < flora +0.001 < the overlay ladder +0.0025..+0.006 (scorch/wet/snow/
/// wind/fuel/burning).
///
/// Authored bed recess below ground top (y = 0) per LDtk water class, world
/// units. Immutable geometry: the live water column on a cell is
/// `WaterMap::depth() = (body level + bed).max(0)`.
pub const WATER_BED_SHALLOW: f32 = 0.15;
pub const WATER_BED_DEEP: f32 = 0.35;
/// A live water column deeper than this (world units) reads as deep water —
/// surface tint, nav-cost class, tooltip label. At the starting level
/// (WATER_SURFACE_Y) a shallow bed carries a 0.10 column and a deep bed
/// 0.30: the same classes the LDtk paint authored.
pub const WATER_DEEP_MIN_DEPTH: f32 = 0.15;
/// The water level every body starts at (surface world-y): just below the
/// ground top so banks read as a slight drop even where the bed is shallow.
/// The water-level devtool (debug UI, Weather tab) moves it from here.
pub const WATER_SURFACE_Y: f32 = -0.05;
/// Water-level devtool: world-y the level moves per click, all bodies.
pub const WATER_LEVEL_STEP: f32 = 0.05;
/// Highest allowed body level: just under the ground top (y = 0) so the
/// surface quad never z-fights ground tiles. Water can't top the banks
/// anyway — cells with no authored bed never carry water.
pub const WATER_LEVEL_MAX: f32 = -0.02;
/// Lowest allowed body level: below even the deep bed (WATER_BED_DEEP),
/// i.e. fully drained everywhere, reachable in whole devtool steps.
pub const WATER_LEVEL_MIN: f32 = -0.4;
/// Surface opacity endpoints — shallow water lets more of the bed show
/// through. The merged surface mesh blends between them per vertex (see
/// WATER_TINT_DEEP_DEPTH), so mid-river reads as a continuous gradient.
pub const WATER_SHALLOW_ALPHA: f32 = 0.5;
pub const WATER_DEEP_ALPHA: f32 = 0.75;
/// Column depth (world units) at which the surface tint/alpha reaches the
/// full deep-water endpoint. Set to the default deep column
/// (WATER_BED_DEEP plus WATER_SURFACE_Y), so at the starting level the
/// channel centers show exactly DEEP_WATER_COLOR and the tint fades toward
/// SHALLOW_WATER_COLOR as the water thins. Visual only — the *class*
/// threshold for nav cost, tooltip and algae stays WATER_DEEP_MIN_DEPTH.
pub const WATER_TINT_DEEP_DEPTH: f32 = 0.30;
/// Surface alpha multiplier at the most exposed shore corners (unitless
/// 0..=1, blended toward 1.0 as more of a vertex's four neighboring cells
/// have beds). 1.0 = no fade; lower = the rim turns more transparent so
/// the bed shows through and the water's edge reads as shallowing rather
/// than a painted border. Raised from 0.55: at the old value the rim read
/// as noticeably paler/icier than open shallow water even with no snow
/// involved (SPEC "connected shore tiles" follow-up) — 0.7 keeps the
/// shallowing cue without washing the rim out past its neighbor.
pub const WATER_SHORE_FADE: f32 = 0.7;
/// Unconditional mix of the convex shore wedge's `dirt_tip` base
/// (`map::build_shore_wedge_geometry`) toward `SHALLOW_WATER_COLOR`, applied
/// before `map::snow_wet_tint`'s bucket logic. That tip fills a cell
/// tooltip-tagged "Shallow water" with the plain `DIRT_COLOR` checker (by
/// design, so the land silhouette runs through it) — without this floor the
/// tip reads as pure dirt until snow cover is deep enough to move it, and a
/// water-tagged cell should never read as fully dry (SPEC "connected shore
/// tiles" follow-up).
pub const SHORE_TIP_WATER_BLEND: f32 = 0.15;
/// Direction surface water visually flows (world XZ, normalized): north to
/// south and east to west per the on-screen compass (north = -Z, east = +X).
/// Purely visual — it scrolls the water shader's ripple pattern
/// (`map::WaterExtension`); no sim system reads it. Shader-side ripple
/// tuning (wavelengths, speeds, chop) lives in `water_surface.wgsl`, the
/// same split `wheat_sway.wgsl` uses.
pub const WATER_FLOW_DIRECTION: Vec2 = Vec2::new(
    -std::f32::consts::FRAC_1_SQRT_2,
    std::f32::consts::FRAC_1_SQRT_2,
);
/// Wet-sand riverbed color, seen through the translucent surface.
pub const WATER_BED_COLOR: Color = Color::srgb(0.45, 0.38, 0.28);
/// Ground/riverbed cuboid thickness (world units). Must exceed
/// WATER_BED_DEEP: the neighboring ground tiles' side faces are the
/// riverbanks, so they have to reach below the deepest bed. Overlay, roof,
/// flora and zone tiles keep the thin TILE_THICKNESS mesh.
pub const GROUND_THICKNESS: f32 = 0.5;

/// Debug gizmo polyline for computed paths.
pub const PATH_GIZMO_COLOR: Color = Color::srgb(1.0, 1.0, 0.2);

/// Middle-mouse orbit: radians of yaw per pixel of horizontal mouse motion.
pub const CAMERA_ORBIT_SPEED: f32 = 0.005;

/// Isometric start position: the camera sits at (d, d, d) looking at the
/// origin. World units; the 25/32 framing ratio was tuned on the original
/// 32-tile map and scales with it.
pub const CAMERA_START_DISTANCE: f32 = MAP_WIDTH as f32 * TILE_SIZE * 25.0 / 32.0;

/// Mouse-wheel zoom: fraction of orthographic scale changed per scroll tick.
pub const CAMERA_ZOOM_SPEED: f32 = 0.1;
/// Orthographic scale clamp (smaller = more zoomed in).
pub const CAMERA_ZOOM_MIN: f32 = 0.3;
pub const CAMERA_ZOOM_MAX: f32 = 2.0;

/// WASD viewport pan. Speed in world units/sec at zoom scale 1.0 (scaled by the
/// live zoom so it feels the same on screen at any zoom level).
pub const CAMERA_PAN_SPEED: f32 = 12.0;
/// The look-at point may leave the map center by at most this many world units
/// per axis: the map half-extent plus a small margin, so the edges stay
/// reachable but the view can't wander off into the void.
pub const CAMERA_PAN_LIMIT: f32 = MAP_WIDTH as f32 * TILE_SIZE / 2.0 + 0.5;
/// Default pan keybindings. Remappable at runtime via the `CameraControls`
/// resource (see `scene.rs`); these are just its defaults.
pub const CAMERA_PAN_FORWARD_KEY: KeyCode = KeyCode::KeyW;
pub const CAMERA_PAN_BACK_KEY: KeyCode = KeyCode::KeyS;
pub const CAMERA_PAN_LEFT_KEY: KeyCode = KeyCode::KeyA;
pub const CAMERA_PAN_RIGHT_KEY: KeyCode = KeyCode::KeyD;

/// F2 swaps between the isometric build view and a straight-down top-down
/// view (better for lining up rectangular rooms). See
/// `scene::toggle_camera_view`.
pub const CAMERA_TOPDOWN_TOGGLE_KEY: KeyCode = KeyCode::F2;
/// How long the animated swap between the two views takes, in real
/// (unpaused-sim-independent) seconds.
pub const CAMERA_VIEW_TRANSITION_SECS: f32 = 0.35;
/// Top-down `ScalingMode::AutoMin` extents: axis-aligned, so (unlike the
/// isometric view) no √2 diagonal factor is needed.
pub const CAMERA_TOPDOWN_MIN_WIDTH: f32 = MAP_WIDTH as f32 * TILE_SIZE + 4.0;
pub const CAMERA_TOPDOWN_MIN_HEIGHT: f32 = MAP_HEIGHT as f32 * TILE_SIZE + 4.0;

pub const LDTK_PROJECT_PATH: &str = "maps/colony.ldtk";

pub const CLEAR_COLOR: Color = Color::srgb(0.13, 0.07, 0.05);
pub const DIRT_COLOR_A: Color = Color::srgb(0.71, 0.38, 0.24);
pub const DIRT_COLOR_B: Color = Color::srgb(0.67, 0.35, 0.22);
pub const GRASS_COLOR_A: Color = Color::srgb(0.42, 0.6, 0.29);
pub const GRASS_COLOR_B: Color = Color::srgb(0.38, 0.56, 0.27);
pub const ALGAE_COLOR: Color = Color::srgb(0.22, 0.48, 0.34);

/// Ground cover (`cover.rs`): per-cell `Flora { kind, coverage }`. Substrate
/// fertility + this bonus x grass coverage = effective growing fertility
/// (dirt 0.2 + 0.4 at full grass = the old grass terrain's 0.6 exactly).
pub const FLORA_FERTILITY_BONUS: f32 = 0.4;
/// Trees root where grass coverage has reached at least this much.
pub const TREE_ROOT_MIN_COVER: f32 = 0.5;
/// Coverage regrowth per second, per kind.
pub const GRASS_REGROW_PER_SECOND: f32 = 0.01;
pub const ALGAE_REGROW_PER_SECOND: f32 = 0.005;
/// Cover this thick tries to seed a neighbor cell...
pub const FLORA_SPREAD_MIN: f32 = 0.8;
/// ...this often (per kind)...
pub const GRASS_SPREAD_SECS: f32 = 60.0;
pub const ALGAE_SPREAD_SECS: f32 = 90.0;
/// ...starting the new patch at this coverage.
pub const FLORA_SEED_COVERAGE: f32 = 0.1;
/// How many algae patches the river starts with.
pub const ALGAE_SEED_COUNT: usize = 3;

/// Algae's visual: a small cluster of thin reeds poking out of the water,
/// scaled up in height by coverage (see `cover::algae_reed_mesh`,
/// `cover::algae_scale`) — the water-plant analog of `CROP_*` below.
pub const ALGAE_REED_COUNT: usize = 5;
pub const ALGAE_REED_RADIUS: f32 = 0.012;
pub const ALGAE_REED_SEGMENTS: usize = 3;
pub const ALGAE_REED_SPREAD: f32 = 0.13;
/// Reed height at full coverage; scaled down to `ALGAE_MIN_SCALE` at 0%
/// coverage so a freshly seeded patch is a short stub, not invisible.
pub const ALGAE_REED_HEIGHT: f32 = 0.14;
pub const ALGAE_MIN_SCALE: f32 = 0.2;

pub const SHALLOW_WATER_COLOR: Color = Color::srgb(0.36, 0.55, 0.72);
pub const DEEP_WATER_COLOR: Color = Color::srgb(0.18, 0.35, 0.54);
pub const PAWN_COLOR: Color = Color::srgb(0.92, 0.83, 0.67);

/// Terrain traversal costs, the single source for A* step costs AND walk
/// speed (speed factor = TERRAIN_COST_DIRT / cost). Dirt is the 10-unit
/// baseline the octile heuristic assumes; never go below it.
pub const TERRAIN_COST_DIRT: u32 = 10;
/// A bit harder: ~0.67x walk speed.
pub const TERRAIN_COST_SHALLOW: u32 = 15;
/// Way harder: 0.25x walk speed.
pub const TERRAIN_COST_DEEP: u32 = 40;
/// Pushing through the woods: 0.5x walk speed on a tree's cell (grass
/// itself walks like dirt; the tree is what slows you).
pub const TERRAIN_COST_TREE: u32 = 20;
/// A fully frozen water body walks like dirt — that's the point of crossing
/// on the ice.
pub const TERRAIN_COST_ICE: u32 = 10;

/// Pawn mesh footprint (X/Z) and height (Y) — a tall rectangle rather than a
/// cube, both so it reads as a rough humanoid silhouette and so a sleeping
/// pawn's rotated pose (`director::sync_sleeping_pose`) is actually visible:
/// rotating a perfect cube 90 degrees looks identical to not rotating it.
pub const PAWN_WIDTH: f32 = 0.8;
pub const PAWN_HEIGHT: f32 = 1.4;
/// How deep a wading pawn's visual sinks while water is present (world
/// units): down to the bed, clamped here — a deep bed (0.35) still leaves
/// most of the pawn above the surface. On a *drained* bed the cap doesn't
/// apply: the pawn just stands on the lower ground.
pub const WADE_MAX_DEPTH: f32 = 0.25;
/// Walk speed in tiles per second (constant for now; terrain-type speed
/// multipliers are a planned extension).
pub const PAWN_SPEED: f32 = 3.0;

/// Sphere radius of a fully grown bush; scaled down to `BUSH_MIN_SCALE` at 0%.
pub const BUSH_RADIUS: f32 = 0.35;
pub const BUSH_MIN_SCALE: f32 = 0.35;
pub const BUSH_COLOR: Color = Color::srgb(0.24, 0.53, 0.28);
/// Harvestable bushes turn berry-red.
pub const BUSH_RIPE_COLOR: Color = Color::srgb(0.55, 0.27, 0.31);
/// Growth gained per second *at fertility 1.0* (scaled down by the bush's
/// tile fertility — see `Terrain::fertility`). On fertile land, 0% -> 100%
/// in ~100 s; on plain dirt (0.2x), ~500 s.
pub const BUSH_GROWTH_PER_SECOND: f32 = 0.05;
/// A bush becomes harvestable at this growth fraction.
pub const BUSH_HARVESTABLE_GROWTH: f32 = 0.8;
/// Harvest yield of a 100%-grown bush; prorated by growth below that.
pub const BUSH_MAX_YIELD: u32 = 20;

/// A crop tile is a cluster of tall thin stalks (`crops::wheat_cluster_mesh`),
/// each stacked from a few cuboid segments so the wind vertex shader can
/// curve them instead of shearing one long box.
pub const CROP_STALK_COUNT: usize = 4;
/// Stalk cross-section (world units, square).
pub const CROP_STALK_SIZE: f32 = 0.05;
/// Vertical segments per stalk — the shader's bend resolution.
pub const CROP_STALK_SEGMENTS: usize = 4;
/// Max XZ offset of a stalk from the tile center (world units).
pub const CROP_STALK_SPREAD: f32 = 0.14;
/// Stalk height of a fully grown crop; scaled down to `CROP_MIN_SCALE` at 0%
/// (a crop only becomes harvestable at 100%, unlike bushes' 80% threshold —
/// there's no prorated "early" wheat).
pub const CROP_HEIGHT: f32 = 0.5;
pub const CROP_MIN_SCALE: f32 = 0.15;
pub const CROP_COLOR: Color = Color::srgb(0.72, 0.62, 0.2);
/// Wheat's growth gained per second *at fertility 1.0* (scaled by tile
/// fertility, same model as bushes). On fertile land, 0% -> 100% in ~50 s;
/// on plain dirt (0.2x), ~250 s. Other crop kinds get their own constant,
/// dispatched by `CropKind::growth_per_second`.
pub const WHEAT_GROWTH_PER_SECOND: f32 = 0.02;
/// Harvest yield of a fully grown crop.
pub const CROP_MAX_YIELD: u32 = 15;

/// Wildfire fuel (`cover::fuel_at`): per-cell 0..=1, terrain-gated ground
/// fuel from grass cover plus the standing plant's contribution. Humidity
/// is deliberately NOT part of fuel — it gates ignition (fire phase).
pub const FUEL_GRASS: f32 = 0.6;
/// Standing-plant fuel at full growth, prorated by growth below that.
pub const FUEL_TREE: f32 = 1.0;
pub const FUEL_SHRUB: f32 = 0.7;
pub const FUEL_BUSH: f32 = 0.5;
pub const FUEL_CROP: f32 = 0.5;
/// Fuel debug overlay: amber tint, 4 alpha shades bucketed by fuel.
pub const FUEL_OVERLAY_COLOR: Color = Color::srgb(0.85, 0.55, 0.15);
pub const FUEL_OVERLAY_ALPHAS: [f32; 4] = [0.10, 0.18, 0.26, 0.34];
/// Above the wind overlay (0.004): stacked debug overlays never z-fight.
pub const FUEL_OVERLAY_OFFSET: f32 = 0.005;

/// Shrubs (`shrubs.rs`): decorative standing fuel — grows, spreads sparsely,
/// never harvestable; exists to burn once wildfires land. Deliberately
/// small and slow: a density cap (`SHRUB_MAX_NEIGHBORS`) keeps them from
/// ever blanketing the map.
pub const SHRUB_RADIUS: f32 = 0.26;
/// Y-squash baked into the mesh: a low dome, shape-distinct from the
/// bush's sphere and the tree's trunk+cone.
pub const SHRUB_FLATTEN: f32 = 0.45;
pub const SHRUB_MIN_SCALE: f32 = 0.25;
pub const SHRUB_COLOR: Color = Color::srgb(0.45, 0.52, 0.26);
/// Growth gained per second *at fertility 1.0* (scaled by tile fertility,
/// the shared plant model).
pub const SHRUB_GROWTH_PER_SECOND: f32 = 0.008;
/// Mature shrubs try to seed a neighbor this often — slower than trees.
pub const SHRUB_SPREAD_SECS: f32 = 180.0;
pub const SHRUB_SPREAD_RADIUS: i32 = 2;
/// A seed attempt only proceeds this often; the rest are no-ops, thinning
/// the reproduction rate further without touching the timer.
pub const SHRUB_SPREAD_CHANCE: f32 = 0.5;
/// Chebyshev radius used to count nearby shrubs for the density cap.
pub const SHRUB_NEIGHBOR_RADIUS: i32 = 2;
/// A candidate cell won't root if this many shrubs already stand within
/// `SHRUB_NEIGHBOR_RADIUS` — caps local density so patches stay sparse
/// instead of growing exponentially until the map is full.
pub const SHRUB_MAX_NEIGHBORS: usize = 2;
/// Shrubs root on thinner grass than trees (`TREE_ROOT_MIN_COVER` 0.5) —
/// hardy scrub.
pub const SHRUB_ROOT_MIN_COVER: f32 = 0.3;
/// World-start scatter onto LDtk grass cells, mostly-grown so the map
/// reads scrubby from minute one.
pub const SHRUB_SEED_COUNT: usize = 8;
pub const SHRUB_SEED_GROWTH: f32 = 0.7;

/// Wildfire (`fire.rs`): ground fire only for now — standing plants feed
/// spread with their fuel but survive (they burn in a later phase). Fire
/// advances in fixed ticks; spread is chance-based, driven by the target's
/// fuel, the source cell's local wind, and the target's humidity.
pub const FIRE_TICK_SECS: f32 = 0.5;
/// Below this fuel a cell can neither ignite nor be spread to.
pub const FIRE_MIN_FUEL: f32 = 0.05;
/// Ground wetter than this can't catch fire.
pub const FIRE_IGNITE_MAX_HUMIDITY: f32 = 0.5;
/// A burning cell soaked past this (rain) is doused.
pub const FIRE_EXTINGUISH_HUMIDITY: f32 = 0.5;
/// Per-tick spread chance to one neighbor, before fuel/wind/humidity
/// factors.
pub const FIRE_SPREAD_BASE_CHANCE: f32 = 0.12;
/// How strongly wind alignment scales spread, per unit of wind strength.
pub const FIRE_WIND_ALIGNMENT: f32 = 0.5;
/// Wind-factor floor: fire still creeps upwind/crosswind, slowly.
pub const FIRE_CREEP_FLOOR: f32 = 0.05;
/// Wind-factor cap: heavy gusting wind can't push past this multiplier.
pub const FIRE_WIND_FACTOR_MAX: f32 = 5.0;
/// A fire spreads only once it burns at least this hot.
pub const FIRE_SPREAD_MIN_INTENSITY: f32 = 0.3;
/// Intensity gained per second after ignition (0 -> 1 in 2 s).
pub const FIRE_RAMP_PER_SECOND: f32 = 0.5;
/// Fuel consumed per second at full intensity — a full-fuel cell burns
/// ~25 s.
pub const FIRE_CONSUME_PER_SECOND: f32 = 0.04;
/// Below this remaining fuel the flame dwindles (intensity capped at
/// fuel_left / this).
pub const FIRE_DWINDLE_FUEL: f32 = 0.15;
/// Scorch left by a doused fire is at least this — even a brief burn marks
/// the ground.
pub const SCORCH_MIN: f32 = 0.25;
/// Scorch healed per second (~500 s for a full burn, about two day/night
/// cycles).
pub const SCORCH_HEAL_PER_SECOND: f32 = 0.002;
/// Fertility multiplier at full scorch = 1 - this (burned land grows at
/// 20%).
pub const SCORCH_FERTILITY_PENALTY: f32 = 0.8;
/// Grass won't re-seed onto cells scorched above this — burned land
/// re-greens from the edges inward as it heals.
pub const SCORCH_REGROW_MAX: f32 = 0.5;
/// Burning-cell overlay: fire is game state, always shown (not a devtool).
pub const BURNING_OVERLAY_COLOR: Color = Color::srgb(1.0, 0.45, 0.1);
pub const BURNING_OVERLAY_ALPHAS: [f32; 4] = [0.15, 0.25, 0.35, 0.45];
/// Top of the overlay ladder (above fuel 0.005).
pub const BURNING_OVERLAY_OFFSET: f32 = 0.006;
/// Scorched-ground overlay, also always shown.
pub const SCORCH_OVERLAY_COLOR: Color = Color::srgb(0.05, 0.04, 0.03);
pub const SCORCH_OVERLAY_ALPHAS: [f32; 4] = [0.15, 0.3, 0.45, 0.6];
/// Deliberately UNDER the wet overlay (0.003): rain visually darkens
/// scorched ground, not the other way around.
pub const SCORCH_OVERLAY_OFFSET: f32 = 0.0025;
/// Flame/smoke particles, one small emitter per burning cell, drawn from a
/// fixed pool created at startup — bevy_hanabi crashes if effect instances
/// are created at runtime (buffer reallocation), and a pool also bounds
/// the worst case. A fire larger than the pool just shows this many
/// plumes.
pub const FIRE_EMITTER_POOL: usize = 48;
pub const FIRE_PARTICLE_CAPACITY: u32 = 64;
pub const FIRE_SPAWN_RATE: f32 = 24.0;
pub const FIRE_PARTICLE_LIFETIME: f32 = 1.4;
/// Particles rise at this speed (world-units/sec)...
pub const FIRE_RISE_SPEED: f32 = 1.2;
/// ...and drift laterally with the cell's local wind, scaled by this.
pub const FIRE_SMOKE_DRIFT: f32 = 0.5;
pub const FIRE_PARTICLE_SIZE: f32 = 0.12;
/// A fire ending with at least this much of its fuel burned kills the
/// standing plants (trees, shrubs) on its cell. Natural burnout is always
/// 1.0 and always kills; a douse (rain or devtool) arriving early enough
/// saves them.
pub const FIRE_PLANT_KILL_COMPLETENESS: f32 = 0.5;
/// Charred snag trunk — what a burnt tree keeps until it's cleared.
pub const TREE_TRUNK_CHARRED_COLOR: Color = Color::srgb(0.10, 0.08, 0.07);
/// Canopy of a tree whose cell is currently on fire...
pub const TREE_CANOPY_BURNING_COLOR: Color = Color::srgb(0.35, 0.12, 0.04);
/// ...and a shrub on a burning cell.
pub const SHRUB_BURNING_COLOR: Color = Color::srgb(0.30, 0.12, 0.05);
/// Emissive term shared by the burning-plant materials: the ember glow.
pub const BURNING_PLANT_EMISSIVE: LinearRgba = LinearRgba::rgb(2.0, 0.5, 0.1);

/// Trees: a trunk with a canopy, scaled from `TREE_MIN_SCALE` (sapling) to
/// full size by growth. Mature trees seed saplings on nearby free grass.
pub const TREE_TRUNK_SIZE: f32 = 0.22;
pub const TREE_TRUNK_HEIGHT: f32 = 0.9;
pub const TREE_TRUNK_COLOR: Color = Color::srgb(0.42, 0.29, 0.18);
/// The canopy is a cone (pine-like), centered on the trunk top: base radius,
/// and how tall the cone stands.
pub const TREE_CANOPY_RADIUS: f32 = 0.45;
pub const TREE_CANOPY_HEIGHT: f32 = 1.2;
pub const TREE_CANOPY_COLOR: Color = Color::srgb(0.16, 0.42, 0.2);
pub const TREE_MIN_SCALE: f32 = 0.15;
/// Growth gained per second *at fertility 1.0* (scaled by tile fertility,
/// same model as bushes/crops). On grass (0.6x): 0% -> 100% in ~2.8 min.
pub const TREE_GROWTH_PER_SECOND: f32 = 0.01;
/// A mature tree attempts to seed a sapling this often.
pub const TREE_SPREAD_SECS: f32 = 40.0;
/// Max Chebyshev distance of a sapling from its parent tree.
pub const TREE_SPREAD_RADIUS: i32 = 2;
/// A tree can be marked to cut from this growth on; below it there's no
/// trunk worth the axe (and no free wood from culling saplings).
pub const TREE_CUTTABLE_GROWTH: f32 = 0.5;
/// Wood a fully-grown tree yields; prorated by growth below 100%.
pub const TREE_WOOD_YIELD: u32 = 25;
/// Work time to fell a tree, the pawn standing next to it. Hardcoded for
/// now; a skill/tool formula can replace it when the economy gets balanced.
pub const TREE_CUT_SECS: f32 = 5.0;
/// Growing zones set to Trees plant on a spaced grid — one sapling every
/// this many tiles on both axes — so wood can't be farmed wall-to-wall.
pub const TREE_PLANT_SPACING: i32 = 3;

/// A job marked `Stuck` (unreachable target, nowhere to drop the yield)
/// retries after this long instead of being ignored forever.
pub const STUCK_RETRY_SECS: f32 = 5.0;

/// A pawn halted this long by another pawn occupying its next tile gives up
/// and re-plans around the blocker instead of waiting forever.
pub const BLOCKED_REPLAN_SECS: f32 = 0.75;

/// Director job priorities (lower = more urgent). Harvest, haul, and
/// farming are equal on purpose: pawn skill tiers break the tie.
pub const HARVEST_JOB_PRIORITY: u32 = 10;
pub const HAUL_JOB_PRIORITY: u32 = 10;
pub const FARM_JOB_PRIORITY: u32 = 10;
pub const CUT_JOB_PRIORITY: u32 = 10;
/// Construction work ranks with the other real work.
pub const SUPPLY_JOB_PRIORITY: u32 = 10;
pub const BUILD_JOB_PRIORITY: u32 = 10;
/// Demolition is construction work too (it shares the `build` allowance and
/// skill) — same band as `BUILD_JOB_PRIORITY`.
pub const DEMOLISH_JOB_PRIORITY: u32 = 10;
/// Roofing is finishing work: after the real work, before housekeeping.
pub const ROOF_JOB_PRIORITY: u32 = 15;
/// Stockpile consolidation is housekeeping: less urgent than real work.
pub const MERGE_JOB_PRIORITY: u32 = 20;
/// Recreation strolling: far below any real work.
pub const WALK_JOB_PRIORITY: u32 = 100;

/// Max Chebyshev distance of a random stroll destination.
pub const WANDER_RADIUS: i32 = 5;
/// Breather between two stroll legs.
pub const WANDER_PAUSE_SECS: f32 = 1.5;

/// Per-pawn completed-job log (`history::JobHistory`) keeps at most this many
/// entries, oldest dropped first. Recreation (`Walk`) jobs are never logged.
pub const MAX_JOB_HISTORY: usize = 500;

/// A ground stack holds at most this many of its item (any kind).
pub const STACK_CAP: u32 = 80;

/// Harvest yields and full-stockpile deliveries drop on the nearest usable
/// cell, searching outward up to this many tiles (Chebyshev rings) before
/// giving up — clogged surroundings spill further away instead of blocking.
pub const YIELD_DROP_RADIUS: i32 = 8;

/// Item stack dropped on the ground by a harvest or a haul; shared cuboid
/// shape for every `ItemKind`, colored per kind.
pub const ITEM_STACK_SIZE: f32 = 0.3;
pub const ITEM_STACK_HEIGHT: f32 = 0.15;
pub const BERRY_STACK_COLOR: Color = Color::srgb(0.78, 0.22, 0.32);
pub const WHEAT_STACK_COLOR: Color = Color::srgb(0.82, 0.68, 0.24);
pub const WOOD_STACK_COLOR: Color = Color::srgb(0.5, 0.36, 0.2);

/// Tile action buttons (RimWorld-style square orders next to the info
/// panel). Harvest uses the green family, Cancel the red one.
pub const ACTION_BUTTON_SIZE: f32 = 48.0;
pub const BUTTON_BACKGROUND: Color = Color::srgb(0.25, 0.35, 0.25);
pub const BUTTON_HOVER: Color = Color::srgb(0.32, 0.45, 0.32);
pub const BUTTON_PRESSED: Color = Color::srgb(0.18, 0.26, 0.18);
pub const CANCEL_BACKGROUND: Color = Color::srgb(0.4, 0.22, 0.2);
pub const CANCEL_HOVER: Color = Color::srgb(0.5, 0.28, 0.26);
pub const CANCEL_PRESSED: Color = Color::srgb(0.3, 0.16, 0.15);
/// Expand/Shrink zone orders: blue and amber families, distinct from the
/// green (Harvest) and red (Cancel/Delete) ones.
pub const EXPAND_BACKGROUND: Color = Color::srgb(0.22, 0.32, 0.42);
pub const EXPAND_HOVER: Color = Color::srgb(0.28, 0.4, 0.52);
pub const EXPAND_PRESSED: Color = Color::srgb(0.16, 0.24, 0.32);
pub const SHRINK_BACKGROUND: Color = Color::srgb(0.42, 0.36, 0.18);
pub const SHRINK_HOVER: Color = Color::srgb(0.52, 0.45, 0.24);
pub const SHRINK_PRESSED: Color = Color::srgb(0.32, 0.27, 0.14);

/// Left-center stock panel: one row per `StockCategory` summing stockpiled
/// items, expandable (click) into per-kind detail rows. Rows are transparent
/// on the panel background; hover/press use neutral grays (they're readouts,
/// not order buttons — no green/red family).
pub const STOCK_ROW_WIDTH: f32 = 130.0;
pub const STOCK_DETAIL_INDENT: f32 = 12.0;
pub const STOCK_ROW_HOVER: Color = Color::srgb(0.25, 0.25, 0.25);
pub const STOCK_ROW_PRESSED: Color = Color::srgb(0.35, 0.35, 0.35);

/// Allowance panel grid: a name column, then one column per work type with
/// a checkbox per pawn. Checked state is shape-encoded (a mark glyph), the
/// green/gray fill is only a secondary cue — colorblind-safe.
pub const NAME_COL_WIDTH: f32 = 56.0;
pub const WORK_COL_WIDTH: f32 = 56.0;
pub const CHECKBOX_SIZE: f32 = 16.0;
pub const CHECKBOX_ON_COLOR: Color = Color::srgb(0.3, 0.45, 0.3);
pub const CHECKBOX_ON_HOVER: Color = Color::srgb(0.38, 0.55, 0.38);
pub const CHECKBOX_OFF_COLOR: Color = Color::srgb(0.2, 0.2, 0.2);
pub const CHECKBOX_OFF_HOVER: Color = Color::srgb(0.3, 0.3, 0.3);
pub const CHECKBOX_BORDER: Color = Color::srgb(0.6, 0.58, 0.55);
/// The check-mark glyph: none of the repo fonts have U+2713 CHECK MARK, but
/// they all have U+00D7 (ballot-style cross).
pub const CHECKBOX_MARK: &str = "\u{d7}";
pub const CHECKBOX_MARK_FONT: &str = "fonts/FiraSans-Bold.ttf";

/// Screen-space selection indicator (corner brackets around the selection).
/// The square frames `SELECTION_WORLD_SIZE` world units around the selected
/// entity; the brackets themselves are sized in pixels.
pub const SELECTION_WORLD_SIZE: f32 = 1.2 * TILE_SIZE;
pub const SELECTION_FILL: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
pub const SELECTION_BRACKET: Color = Color::srgba(1.0, 1.0, 1.0, 0.9);
pub const BRACKET_ARM: f32 = 10.0;
pub const BRACKET_THICKNESS: f32 = 2.0;

/// Fixed width of the floating pawn name labels (text centered inside).
pub const LABEL_WIDTH: f32 = 120.0;

/// Top-center pawn selector: one chip per pawn (pawn-colored square with the
/// name's initial, name below). Click selects; clicking the already-selected
/// pawn centers the camera on it.
pub const PORTRAIT_SIZE: f32 = 40.0;
pub const PORTRAIT_BORDER: f32 = 2.0;
pub const PORTRAIT_SELECTED_BORDER: Color = Color::srgb(1.0, 1.0, 1.0);
pub const PORTRAIT_IDLE_BORDER: Color = Color::srgba(0.0, 0.0, 0.0, 0.35);
/// Dark glyph on the light pawn-colored chip.
pub const PORTRAIT_TEXT: Color = Color::srgb(0.25, 0.18, 0.12);

pub const PANEL_MARGIN: f32 = 12.0;
pub const PANEL_BACKGROUND: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
/// Same tint as `PANEL_BACKGROUND` but fully opaque, for panels that sit
/// over busy map content and shouldn't let it show through (the pawn tab
/// window).
pub const PANEL_BACKGROUND_OPAQUE: Color = Color::srgba(0.0, 0.0, 0.0, 1.0);
pub const PANEL_TEXT: Color = Color::srgb(0.92, 0.89, 0.84);

/// Pawn needs (`needs.rs`): 0-100 drives shown as gauges in the info panel.
/// `SLEEP_DECAY_PER_SEC` is tuned so a pawn awake for a full day (180s, see
/// `DAY_LENGTH_SECS`) runs from `SLEEP_START` down to about
/// `SLEEP_TIRED_THRESHOLD`. `SLEEP_TIRED_THRESHOLD`/`SLEEP_RESTED_THRESHOLD`
/// are the hysteresis band `needs::wants_sleep` falls asleep/wakes at.
/// `SLEEP_RECOVER_GROUND_PER_SEC` is deliberately slow relative to
/// `NIGHT_LENGTH_SECS` — one night on the ground won't fully refill a
/// thoroughly tired pawn, which is the point: it's what makes beds worth
/// building later.
pub const SLEEP_START: f32 = 90.0;
pub const SLEEP_DECAY_PER_SEC: f32 = 0.35;
pub const SLEEP_TIRED_THRESHOLD: f32 = 30.0;
pub const SLEEP_RESTED_THRESHOLD: f32 = 90.0;
pub const SLEEP_RECOVER_GROUND_PER_SEC: f32 = 0.8;
/// Resting in a built `Bed` recovers noticeably faster than the ground
/// fallback above — the whole reason to build one.
pub const SLEEP_RECOVER_BED_PER_SEC: f32 = 2.0;
/// `director::JobKind::Sleep` is personal/pre-assigned like Walk (never
/// scored in `assign_jobs`'s pool), so this priority is unused by scoring —
/// kept only because every `Job` carries a `JobPriority` (debug-tab display,
/// consistency).
pub const SLEEP_JOB_PRIORITY: u32 = WALK_JOB_PRIORITY;

/// Horizontal gauge widget (needs panel): a fixed-width track with a
/// percent-width fill, colored green-to-red by level.
pub const GAUGE_WIDTH: f32 = 120.0;
pub const GAUGE_HEIGHT: f32 = 10.0;
pub const GAUGE_TRACK_COLOR: Color = Color::srgb(0.2, 0.2, 0.2);
pub const GAUGE_FILL_HIGH_COLOR: Color = Color::srgb(0.35, 0.55, 0.3);
pub const GAUGE_FILL_LOW_COLOR: Color = Color::srgb(0.6, 0.25, 0.2);

/// Center-left pawn tab window (Job History / Skills / Needs, opened from the
/// info panel's tab bar): fixed width so it doesn't reflow, and a fixed
/// height for the scrollable Job History list specifically so it doesn't grow
/// as entries accumulate.
pub const HISTORY_PANEL_WIDTH: f32 = 240.0;
pub const HISTORY_PANEL_HEIGHT: f32 = 260.0;
/// Pixels of scroll per unit of accumulated mouse-wheel delta.
pub const HISTORY_SCROLL_SPEED: f32 = 24.0;

/// UI stacking layers (`GlobalZIndex`). Higher = rendered in front. HUD panels
/// use the implicit default of 0 (== `Z_HUD`, kept here for documentation —
/// no panel sets it explicitly); anything that must float above the HUD gets
/// an explicit `GlobalZIndex(Z_*)`. Gaps left for future tiers.
#[allow(dead_code)]
pub const Z_HUD: i32 = 0;
/// Below `Z_HUD`, so the always-on Stock panel never covers a pawn tab
/// window in the same left-center slot.
pub const Z_STOCK: i32 = -10;
/// Above `Z_HUD` (and the Stock panel), below the debug window: the pawn tab
/// window should always be reachable even while it overlaps the Stock panel.
pub const Z_PAWN_TABS: i32 = 50;
pub const Z_DEVTOOLS: i32 = 100;

/// Cursor tooltip offset (px, below-right so the pointer never covers it).
pub const TOOLTIP_OFFSET: Vec2 = Vec2::new(14.0, 18.0);

/// Wind (`weather::Wind`): permanent, blowing on the XZ plane with strength
/// gusting around a base value and a heading that slowly wanders around
/// this base direction. Strength unit = world-units/second of lateral
/// drift (what rain streaks inherit).
pub const WIND_DIRECTION: Vec2 = Vec2::new(1.0, 0.35);
/// Calm default — barely noticeable.
pub const WIND_BASE_STRENGTH: f32 = 1.5;
/// The Heavy Wind devtool's base — bends wheat flat and slants rain hard.
pub const WIND_HEAVY_STRENGTH: f32 = 6.0;
/// Gusts: two layered sines around 1.0x base. Amplitudes are fractions of
/// the base (their sum stays < 1 so strength never goes negative);
/// frequencies are in Hz and deliberately incommensurate so the pattern
/// doesn't visibly loop.
pub const WIND_GUST_AMPLITUDE_PRIMARY: f32 = 0.35;
pub const WIND_GUST_AMPLITUDE_SECONDARY: f32 = 0.2;
/// Slow swell.
pub const WIND_GUST_FREQUENCY_PRIMARY: f32 = 0.07;
/// Faster flutter on top.
pub const WIND_GUST_FREQUENCY_SECONDARY: f32 = 0.19;
/// Ambient heading wander around `WIND_DIRECTION`: two slow layered sines
/// (amplitudes in radians), the gust model an order of magnitude slower.
pub const WIND_WANDER_PRIMARY: f32 = 0.5;
pub const WIND_WANDER_SECONDARY: f32 = 0.25;
/// ~4 min swell.
pub const WIND_WANDER_FREQUENCY_PRIMARY: f32 = 0.004;
/// ~90 s flutter on top.
pub const WIND_WANDER_FREQUENCY_SECONDARY: f32 = 0.011;
/// Max rate the ACTUAL direction turns toward the desired one (radians/sec)
/// — what makes devtool shifts and wander swings smooth instead of snappy.
pub const WIND_TURN_SPEED: f32 = 0.25;
/// The Shift Wind devtool's jump per click (radians).
pub const WIND_SHIFT_STEP: f32 = std::f32::consts::FRAC_PI_2;

/// Wind exposure (`weather::WindExposureMap`): per-cell 0..=1 factor on the
/// ambient wind (cell wind = `Wind::current_vector()` x exposure), banded by
/// height (`weather::WindBand`) so a short blocker can't shelter something
/// taller than it. Blockers cast wind shadows on their DOWNWIND cells,
/// sampled this many cells upwind with linear falloff.
pub const WIND_SHELTER_RANGE: i32 = 4;
/// Shelter a solid built blocker (wall / door / turbine base) contributes to
/// the GROUND band at distance 1; falls off linearly to 1/RANGE of this at
/// max range. Every buildable today is short enough that none of them
/// shelter the ALTITUDE band at all (see `weather::terrain_shelter`).
pub const WIND_WALL_SHELTER: f32 = 1.0;
/// Max shelter of a fully grown tree (scaled by growth — a sapling barely
/// shelters): a porous canopy blocks less than masonry. Trees are tall
/// enough to shelter BOTH bands — trunk sheltering ground, canopy sheltering
/// altitude — so a forest can slow a turbine even though no built structure
/// can yet.
pub const WIND_TREE_SHELTER: f32 = 0.6;
/// Exposure floor for unroofed cells — wind eddies around anything, so no
/// open-air cell goes fully dead. Only roofed (indoor) cells are exactly 0.
/// Applies to both bands.
pub const WIND_MIN_EXPOSURE: f32 = 0.15;
/// Debug overlay (`weather::ShowWindOverlay`, GROUND band): SHELTERED cells
/// tint slate blue-grey, one of 4 alpha shades bucketed by shelter — open
/// field stays clean.
pub const WIND_OVERLAY_COLOR: Color = Color::srgb(0.45, 0.55, 0.75);
pub const WIND_OVERLAY_ALPHAS: [f32; 4] = [0.10, 0.18, 0.26, 0.34];
/// Above the wet overlay (`WET_OVERLAY_OFFSET` 0.003) so a cell that is
/// both wet and sheltered never z-fights.
pub const WIND_OVERLAY_OFFSET: f32 = 0.004;
/// Debug overlay (`weather::ShowAltitudeWindOverlay`, ALTITUDE band): a
/// warmer amber-brown so it reads apart from the ground overlay's
/// slate-blue at a glance when both are toggled on together. Since no
/// built structure shelters altitude yet, this overlay only ever shows
/// tree shadows.
pub const WIND_ALTITUDE_OVERLAY_COLOR: Color = Color::srgb(0.75, 0.55, 0.3);
/// Above `WIND_OVERLAY_OFFSET` (0.004) so the ground and altitude wind
/// overlays never z-fight when both are on at once.
pub const WIND_ALTITUDE_OVERLAY_OFFSET: f32 = 0.005;

/// Wind gusts (`weather::GustEffect`): always-on white "snakes" — each is a
/// CPU-moved emitter head gliding downwind on a flat serpentine path,
/// trailing a Hanabi ribbon whose tail fades and tapers away. Direction and
/// strength read without rain; heavy wind = faster heads = longer trails.
pub const GUST_COUNT: usize = 6;
/// Ribbon particle capacity per gust head (spawn rate x lifetime, plus
/// wiggle room).
pub const GUST_PARTICLE_CAPACITY: u32 = 100;
/// Trail particles spawned per second per head. Hanabi doesn't interpolate
/// spawn positions within a frame, so anything past the frame rate is
/// wasted (the ribbon example's own guidance).
pub const GUST_RIBBON_SPAWN_RATE: f32 = 60.0;
/// Seconds a trail particle lives = how far behind the head the tail
/// reaches (trail length ~= head speed x this).
pub const GUST_RIBBON_LIFETIME: f32 = 0.8;
/// Ribbon height at the head (world units); tapers to zero at the tail.
pub const GUST_RIBBON_WIDTH: f32 = 0.25;
/// Head drift speed = wind strength x this factor (raw strength reads too
/// slow over the map).
pub const GUST_SPEED_FACTOR: f32 = 2.5;
/// Vertical band the gusts glide in (world units above ground, below the
/// treetops). Each head keeps a fixed height — trails stay ground-parallel.
pub const GUST_BAND_MIN: f32 = 0.8;
pub const GUST_BAND_MAX: f32 = 1.6;
/// Serpentine sway: lateral speed amplitude (world-units/sec, perpendicular
/// to the wind) and its frequency in Hz — the "snake" in the trail.
pub const GUST_WIGGLE_SPEED: f32 = 1.6;
pub const GUST_WIGGLE_FREQUENCY: f32 = 0.35;
/// How far past the map edge a head glides before wrapping back upwind.
pub const GUST_EDGE_MARGIN: f32 = 1.5;
/// Trail color at the head; alpha fades linearly to zero along the tail.
pub const GUST_COLOR: Vec4 = Vec4::new(1.0, 1.0, 1.0, 0.22);

/// Weather (`weather.rs`). Rain is a single Hanabi GPU effect: drops spawn
/// uniformly over the map footprint at `RAIN_SPAWN_HEIGHT` (above the
/// tallest tree) and live exactly their fall time, so they die at ground
/// level without any collision test. The streaks' sideways slant comes
/// from the live `Wind` (see the Wind block above), not a constant here.
pub const RAIN_PARTICLE_CAPACITY: u32 = 16_384;
/// Drops spawned per second while raining; with the ~0.4 s fall that keeps
/// roughly 10k drops airborne over the 64x64 map (capacity must exceed it).
pub const RAIN_SPAWN_RATE: f32 = 24_000.0;
pub const RAIN_SPAWN_HEIGHT: f32 = 4.0;
pub const RAIN_FALL_SPEED: f32 = 10.0;
/// Streak quad size: length runs along the velocity (AlongVelocity maps
/// the quad's X axis onto it), width across it.
pub const RAIN_DROP_LENGTH: f32 = 0.3;
pub const RAIN_DROP_WIDTH: f32 = 0.02;
/// Linear RGBA of a drop (Hanabi takes raw Vec4 color).
pub const RAIN_COLOR: Vec4 = Vec4::new(0.6, 0.75, 0.95, 0.35);

/// Per-cell humidity 0..=1 (`weather::HumidityMap`): soaks at the rain
/// rate while rained on (~20 s to saturate), dries at the dry rate under a
/// clear sky (~100 s, about half an in-game day). Display-only for now.
pub const HUMIDITY_RAIN_PER_SECOND: f32 = 0.05;
pub const HUMIDITY_DRY_PER_SECOND: f32 = 0.01;
/// Wet ground darkens under a translucent overlay tile, one of 4 shades
/// bucketed by wetness (shared materials keep the map batchable).
pub const WET_OVERLAY_COLOR: Color = Color::srgb(0.08, 0.1, 0.2);
pub const WET_OVERLAY_ALPHAS: [f32; 4] = [0.10, 0.18, 0.26, 0.34];
/// Above the grass cover (+0.001) and zone tiles (+0.002): wetness tints
/// whatever the cell shows.
pub const WET_OVERLAY_OFFSET: f32 = 0.003;

/// Snow & ice (`snow.rs`). Snowfall is the rain effect's cold sibling:
/// billboarded white flakes drifting down slowly, sharing the map-wide
/// spawn box and the live wind slant.
pub const SNOW_PARTICLE_CAPACITY: u32 = 16_384;
/// Flakes per second; with the ~2 s fall that keeps ~8k flakes airborne
/// (capacity must exceed it).
pub const SNOW_SPAWN_RATE: f32 = 4_000.0;
pub const SNOW_FALL_SPEED: f32 = 2.0;
pub const SNOW_FLAKE_SIZE: f32 = 0.05;
/// Linear RGBA of a flake (Hanabi takes raw Vec4 color).
pub const SNOW_COLOR: Vec4 = Vec4::new(1.0, 1.0, 1.0, 0.85);

/// Per-cell snow cover 0..=1 (`snow::SnowMap`): ~50 s of snowfall to full
/// cover. Melt is proportional to the cell's °C above zero — the sun's
/// daily temperature hump IS the radiance coupling, so melt peaks at noon
/// (+20° spring noon ≈ 10 s, +5° ≈ 40 s) and refills humidity as it goes.
pub const SNOW_ACCUMULATE_PER_SECOND: f32 = 0.02;
pub const SNOW_MELT_PER_DEGREE_SECOND: f32 = 0.005;
/// Snow at least this deep hides the wet-ground sheen (white covers dark).
pub const SNOW_SUPPRESS_WET_MIN: f32 = 0.25;
/// Snow cover whitens the ground through 4 bucketed overlay shades, nearly
/// opaque at full cover.
pub const SNOW_OVERLAY_COLOR: Color = Color::srgb(0.93, 0.95, 1.0);
pub const SNOW_OVERLAY_ALPHAS: [f32; 4] = [0.25, 0.45, 0.65, 0.85];
/// Between the wet overlay (+0.003) and the wind overlay (+0.004) in the
/// overlay ladder (see the water block's vertical-order comment).
pub const SNOW_OVERLAY_OFFSET: f32 = 0.0035;
/// Nav penalty added per snow bucket: full snow turns dirt 10 into 22 —
/// pawns trudge at ~0.45x and paths route around deep drifts.
pub const SNOW_COST_PER_BUCKET: u32 = 3;

/// Water bodies freeze on a gradient. Sub-zero ice never melts, so the
/// freeze level only climbs while the ambient is below 0 °C — at full
/// speed at `FREEZE_FULL_BELOW_C` (one -20 °C night = solid, the
/// user-facing guarantee; the 60 s night x this rate = exactly 1.0) and
/// proportionally slower toward 0 °C. Thawing runs only above zero,
/// per-degree (a +20 °C spring noon clears solid ice in ~50 s).
pub const FREEZE_FULL_BELOW_C: f32 = -20.0;
pub const FREEZE_PER_SECOND_AT_FULL_COLD: f32 = 1.0 / NIGHT_LENGTH_SECS;
pub const ICE_THAW_PER_DEGREE_SECOND: f32 = 0.001;
/// At this freeze level the ice carries a pawn: bed cells cost
/// `TERRAIN_COST_ICE` and waders stop dipping.
pub const ICE_WALKABLE_MIN_FREEZE: f32 = 0.9;

/// Construction (`construction.rs`): walls and doors are ordered as ghost
/// blueprints, supplied with wood by batched hauling runs, then raised by a
/// timed Build job with the pawn standing next to the site.
pub const WALL_WOOD_COST: u32 = 5;
pub const DOOR_WOOD_COST: u32 = 5;
pub const WALL_BUILD_SECS: f32 = 4.0;
pub const DOOR_BUILD_SECS: f32 = 4.0;
/// One supply run carries at most this much wood, servicing several
/// blueprints per trip (leftovers go back to the stockpile).
pub const SUPPLY_LOAD_CAP: u32 = 40;
/// Demolish order (`director::JobKind::Demolish`): tear down a marked
/// structure standing next to it, timed like a Build job in reverse.
pub const DEMOLISH_SECS: f32 = 3.0;
/// Fraction of a structure's `wood_cost()` returned on demolition.
pub const DEMOLISH_REFUND_FRACTION: f32 = 0.8;
/// Built walls stand on a half-tile footprint — purely visual, the cell
/// still blocks pathfinding entirely.
pub const WALL_VISUAL_SIZE: f32 = 0.5;
/// Blueprint ghosts are the final silhouette at reduced alpha.
pub const BLUEPRINT_ALPHA: f32 = 0.35;
/// Squeezing through a doorway is a bit slower than open ground.
pub const TERRAIN_COST_DOOR: u32 = 15;
/// A door is two posts and a lintel — shape-distinct from a wall block, so
/// it never relies on color alone.
pub const DOOR_POST_SIZE: f32 = 0.12;
pub const DOOR_POST_OFFSET: f32 = 0.25;
pub const DOOR_LINTEL_LENGTH: f32 = 2.0 * DOOR_POST_OFFSET + DOOR_POST_SIZE;

/// Wind turbine (`construction::BuildableKind::Turbine`): free to build. The
/// nacelle yaws slowly to face upwind, the rotor spins with the live wind
/// strength, and (`power.rs`) it feeds the electricity grid at a rate scaled
/// by that same wind reading.
pub const TURBINE_WOOD_COST: u32 = 0;
pub const TURBINE_BUILD_SECS: f32 = 3.0;
/// Tower: a slim pole, taller than a wall, shorter than a grown pine.
pub const TURBINE_TOWER_HEIGHT: f32 = 1.8;
pub const TURBINE_TOWER_SIZE: f32 = 0.12;
pub const TURBINE_TOWER_COLOR: Color = Color::srgb(0.80, 0.82, 0.85);
/// Nacelle: the housing atop the tower, long axis along the wind.
pub const TURBINE_NACELLE_LENGTH: f32 = 0.3;
pub const TURBINE_NACELLE_SIZE: f32 = 0.16;
pub const TURBINE_NACELLE_COLOR: Color = Color::srgb(0.45, 0.48, 0.52);
/// Three blades on the upwind face, roots at the hub.
pub const TURBINE_BLADE_LENGTH: f32 = 0.55;
pub const TURBINE_BLADE_WIDTH: f32 = 0.09;
pub const TURBINE_BLADE_THICKNESS: f32 = 0.03;
pub const TURBINE_BLADE_COLOR: Color = Color::srgb(0.92, 0.93, 0.95);
/// Max nacelle yaw rate (radians/sec) — deliberately BELOW
/// `WIND_TURN_SPEED`, so turbines visibly lag and chase a swinging wind
/// instead of tracking it in lockstep.
pub const TURBINE_YAW_SPEED: f32 = 0.2;
/// Rotor spin (radians/sec) per unit of wind strength.
pub const TURBINE_SPIN_PER_STRENGTH: f32 = 0.9;

/// Solar panel (`construction::BuildableKind::SolarPanel`): the second
/// power-style building, a 2x2 footprint — the first building bigger than
/// one tile. A single-axis tracker — the base yaws (about vertical) to track
/// the sun's azimuth, with the cell array fixed at a tilt on top of it — and
/// (`power.rs`) feeds the electricity grid at a rate scaled by daylight and
/// sun elevation.
pub const SOLAR_PANEL_WOOD_COST: u32 = 10;
pub const SOLAR_PANEL_BUILD_SECS: f32 = 6.0;
/// Post: a short central mount, shorter than the turbine tower.
pub const SOLAR_POST_HEIGHT: f32 = 0.5;
pub const SOLAR_POST_SIZE: f32 = 0.14;
pub const SOLAR_POST_COLOR: Color = Color::srgb(0.32, 0.32, 0.35);
/// Cell array: a small grid of individual cells on a backing frame, inset
/// well within the 2-tile footprint so neighboring panels don't visually
/// merge.
pub const SOLAR_ARRAY_COLS: usize = 3;
pub const SOLAR_ARRAY_ROWS: usize = 2;
pub const SOLAR_CELL_SIZE: f32 = 0.32;
pub const SOLAR_CELL_GAP: f32 = 0.04;
pub const SOLAR_CELL_THICKNESS: f32 = 0.03;
pub const SOLAR_FRAME_THICKNESS: f32 = 0.04;
pub const SOLAR_PANEL_COLOR: Color = Color::srgb(0.08, 0.12, 0.30);
/// Fixed backward tilt of the array on its holder (radians) — the classic
/// single-axis tracker look; only the base below it yaws.
pub const SOLAR_ARRAY_TILT: f32 = 0.5;
/// Max yaw rate (radians/sec), same role as `TURBINE_YAW_SPEED`: the base
/// visibly chases the sun's azimuth instead of snapping to it.
pub const SOLAR_TRACK_SPEED: f32 = 0.3;

/// Lightpost (`construction::BuildableKind::Lightpost`): a cheap, walkable
/// building whose only job is light, drawing from the electricity grid
/// (`power.rs`) to do it. Unlike every other building it makes no `NavGrid`
/// change on completion (stays plain walkable terrain), so pawns pass
/// through its tile freely.
pub const LIGHTPOST_WOOD_COST: u32 = 2;
pub const LIGHTPOST_BUILD_SECS: f32 = 3.0;
/// Pole: slimmer and shorter than the turbine tower — just tall enough to
/// clear a standing pawn.
pub const LIGHTPOST_POLE_HEIGHT: f32 = 1.4;
pub const LIGHTPOST_POLE_SIZE: f32 = 0.08;
pub const LIGHTPOST_POLE_COLOR: Color = Color::srgb(0.30, 0.30, 0.33);
/// Lamp head: a small globe at the top of the pole.
pub const LIGHTPOST_LAMP_RADIUS: f32 = 0.14;
/// Base (unlit) lamp color, seen by day when the light is off.
pub const LIGHTPOST_LAMP_COLOR: Color = Color::srgb(0.85, 0.80, 0.60);
/// Warm lamp glow — contrasts the moon's cool blue (`MOON_LIGHT_COLOR`).
pub const LIGHTPOST_LIGHT_COLOR: Color = Color::srgb(1.0, 0.82, 0.5);
/// Much dimmer and shorter-range than the moon: a local pool of light, not
/// a map-wide wash.
pub const LIGHTPOST_LIGHT_INTENSITY: f32 = 400_000.0;
pub const LIGHTPOST_LIGHT_RANGE: f32 = 6.0;
/// Emissive the lamp head's material ramps toward as it lights up, in step
/// with the `PointLight` intensity (`construction::sync_lightposts`).
pub const LIGHTPOST_LAMP_EMISSIVE: LinearRgba = LinearRgba::rgb(3.0, 2.2, 0.8);

/// Electricity (`power.rs`): turbines and solar panels feed a single global
/// pool (`PowerGrid`); light posts (and batteries, below) draw from it. No
/// per-building wiring/circuits yet — one shared budget map-wide. Producer
/// totals are specified as a *daily energy budget* (units producible across
/// a full day/cycle at ideal conditions) and converted here to a peak
/// units/second rate, so "400/day", "1000 stored", and "1/sec" all stay
/// comparable at a glance: a full battery alone runs one light post for
/// about `BATTERY_CAPACITY / LIGHTPOST_POWER_DRAW` seconds.
pub const SOLAR_OUTPUT_PER_DAY: f32 = 400.0;
pub const TURBINE_OUTPUT_PER_DAY: f32 = 700.0;
/// Solar only ever produces by day, so its peak rate is spread over just the
/// day length; a turbine can produce around the clock, so its peak rate is
/// spread over the whole day+night cycle.
pub const SOLAR_PEAK_OUTPUT_PER_SECOND: f32 = SOLAR_OUTPUT_PER_DAY / DAY_LENGTH_SECS;
pub const TURBINE_PEAK_OUTPUT_PER_SECOND: f32 = TURBINE_OUTPUT_PER_DAY / CYCLE_LENGTH_SECS;
/// Live wind strength (`weather::Wind::current`, before per-cell exposure) a
/// turbine needs to spin up at all — a stalled rotor below this produces
/// nothing. Comfortably under `WIND_BASE_STRENGTH` so a turbine still turns
/// out something in calm ambient wind.
pub const POWER_WIND_CUTIN: f32 = 0.2;
/// Live wind strength (post-exposure, `weather::WindExposureMap::wind_at`'s
/// magnitude) at which a turbine reaches its peak output — sits between the
/// calm base (`WIND_BASE_STRENGTH` 1.5) and the Heavy Wind devtool
/// (`WIND_HEAVY_STRENGTH` 6.0), so ordinary gusts run a turbine under its cap
/// and only a strong blow maxes it out.
pub const POWER_WIND_FULL_STRENGTH: f32 = 3.5;
/// Battery (`construction::BuildableKind::Battery`): a one-tile store that
/// banks grid surplus and discharges it to cover a deficit. Free-standing —
/// same footprint/blocking shape as a turbine, no lightweight-walkable
/// exception like the lightpost.
pub const BATTERY_WOOD_COST: u32 = 15;
pub const BATTERY_BUILD_SECS: f32 = 5.0;
pub const BATTERY_CAPACITY: f32 = 1000.0;
/// Charge/discharge rate cap (units/sec) — even a huge instantaneous surplus
/// or deficit only moves a battery's charge this fast, so the readout ramps
/// visibly instead of snapping.
pub const BATTERY_MAX_FLOW: f32 = 50.0;
/// Boxy crate body plus a slim charge-indicator strip up the front.
pub const BATTERY_BODY_SIZE: f32 = 0.5;
pub const BATTERY_BODY_HEIGHT: f32 = 0.6;
pub const BATTERY_BODY_COLOR: Color = Color::srgb(0.28, 0.30, 0.32);
pub const BATTERY_STRIP_WIDTH: f32 = 0.08;
pub const BATTERY_STRIP_HEIGHT: f32 = 0.46;
pub const BATTERY_STRIP_THICKNESS: f32 = 0.03;
pub const BATTERY_STRIP_EMPTY_COLOR: Color = Color::srgb(0.35, 0.14, 0.12);
pub const BATTERY_STRIP_FULL_COLOR: Color = Color::srgb(0.2, 0.85, 0.35);
pub const LIGHTPOST_POWER_DRAW: f32 = 1.0;

/// Bed (`construction::BuildableKind::Bed`): a tired pawn's resting spot,
/// 2 tiles long — comfortably over a lying pawn's length (`PAWN_HEIGHT`).
/// Cheap and fast to build, like a door. Fully walkable, no `NavGrid` write,
/// like a lightpost — a sleeper must be able to stand on it.
pub const BED_WOOD_COST: u32 = 8;
pub const BED_BUILD_SECS: f32 = 4.0;
/// Visual dims: length along the footprint's long axis, width along the
/// short one, a small margin inside the 2-tile/1-tile grid cells so it reads
/// as furniture sitting on the floor rather than filling it edge to edge.
pub const BED_LENGTH: f32 = 2.0 * TILE_SIZE - 0.2;
pub const BED_WIDTH: f32 = TILE_SIZE - 0.2;
pub const BED_HEIGHT: f32 = 0.3;
pub const BED_COLOR: Color = Color::srgb(0.55, 0.35, 0.25);

/// Cooler (`construction::BuildableKind::Cooler`): a wall-segment climate
/// unit — blocks movement and encloses/roofs like a `Wall`, but insulates
/// perfectly (`COOLER_INSULATION`, see `WALL_INSULATION`/`DOOR_INSULATION`
/// above) and actively drives the room(s) it borders toward a player-set
/// `Cooler::target_c`, drawing from the electricity grid (`power.rs`)
/// proportional to how far the room sits from that target. Bidirectional:
/// it heats as readily as it cools.
pub const COOLER_WOOD_COST: u32 = 12;
pub const COOLER_BUILD_SECS: f32 = 5.0;
/// A cooler is the one boundary kind that seals completely — no draft at
/// all, unlike a wooden wall's 0.8 or a door's 0.25.
pub const COOLER_INSULATION: f32 = 1.0;
/// Squarish unit body, roughly wall-sized, with a small vent block on the
/// room-facing side so it reads as machinery rather than a plain wall panel.
pub const COOLER_BODY_COLOR: Color = Color::srgb(0.55, 0.58, 0.62);
pub const COOLER_VENT_COLOR: Color = Color::srgb(0.15, 0.55, 0.75);
pub const COOLER_VENT_SIZE: f32 = 0.3;
pub const COOLER_VENT_THICKNESS: f32 = 0.05;
/// Peak draw (units/sec, same scale as `LIGHTPOST_POWER_DRAW`) at a full
/// `CLIMATE_FULL_DELTA`-degree gap between the room and its target; scales
/// down to 0 as the room reaches the target.
pub const CLIMATE_POWER_DRAW: f32 = 4.0;
/// Room-to-target gap (°C) at which a cooler's power draw caps out.
pub const CLIMATE_FULL_DELTA: f32 = 15.0;
/// How fast a *fully powered* cooler pulls its room toward the target
/// (fraction of the remaining gap per second) — an order of magnitude past
/// `INDOOR_TEMP_RATE_PER_SECOND` so a powered cooler decisively drives its
/// room to the target within seconds, dominating both the ambient leak and
/// a partly-covered grid instead of just slowing the drift down.
pub const CLIMATE_RATE_PER_SECOND: f32 = 0.5;
/// The panel's +/- nudger step and clamp range for `Cooler::target_c`.
pub const TARGET_TEMP_STEP: f32 = 1.0;
pub const TARGET_TEMP_MIN: f32 = -20.0;
pub const TARGET_TEMP_MAX: f32 = 35.0;
/// A freshly built cooler starts targeting a comfortable room temperature.
pub const COOLER_DEFAULT_TARGET_C: f32 = 18.0;

/// Roofs (`map::RoofMap` + `construction.rs`): per-cell attribute, built
/// fast (no materials) on cells stamped with the Roof policy, removed on
/// cells stamped NoRoof. Panels float just above the walls.
pub const ROOF_BUILD_SECS: f32 = 1.5;
pub const ROOF_REMOVE_SECS: f32 = 2.0;
pub const ROOF_PANEL_Y: f32 = 0.85;
pub const ROOF_COLOR: Color = Color::srgb(0.55, 0.42, 0.30);
pub const ROOF_ALPHA: f32 = 0.30;
/// Planned-but-unbuilt roof cells show a fainter, inset ghost panel — the
/// smaller footprint is the "planned" cue, not just the alpha.
pub const ROOF_GHOST_ALPHA: f32 = 0.12;
pub const ROOF_GHOST_INSET: f32 = 0.7;

/// Dev-tooling debug window (`debug_ui.rs`), toggled with backtick.
pub const DEBUG_WINDOW_KEY: KeyCode = KeyCode::Backquote;
/// The window itself has no fixed width (it hugs its widest row, normally
/// the tab bar); this only bounds the free-form Jobs/Power tab text so it
/// wraps instead of stretching the window to fit one long line.
pub const DEBUG_WINDOW_WIDTH: f32 = 300.0;
/// The Jobs tab lists at most this many rows, then "... and N more".
pub const DEBUG_JOBS_MAX_LINES: usize = 25;
