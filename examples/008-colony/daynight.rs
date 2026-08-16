//! Day/night cycle: a `GameClock` resource tracking sim time (frozen while
//! paused, like everything else), and a lighting system fading the scene
//! between full day and a minimal cool-blue night. The visible sun and moon
//! ride on this clock too.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::render_resource::{Face, TextureFormat};

use crate::config::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DayPhase {
    Day,
    Night,
}

/// Sim-time day/night clock. `elapsed` only advances while the sim runs, so
/// pausing freezes the sky mid-sweep along with the pawns.
#[derive(Resource, Default)]
pub struct GameClock {
    pub elapsed: f32,
}

impl GameClock {
    /// Seconds into the current day/night cycle (0..CYCLE_LENGTH_SECS).
    fn time_in_cycle(&self) -> f32 {
        self.elapsed.rem_euclid(CYCLE_LENGTH_SECS)
    }

    pub fn phase(&self) -> DayPhase {
        if self.time_in_cycle() < DAY_LENGTH_SECS {
            DayPhase::Day
        } else {
            DayPhase::Night
        }
    }

    /// 1-based day counter ("Day 1" starts at elapsed 0).
    pub fn day_count(&self) -> u32 {
        (self.elapsed / CYCLE_LENGTH_SECS) as u32 + 1
    }

    /// How far through the day we are (0..1); clamps to 1 during the night.
    pub fn day_progress(&self) -> f32 {
        (self.time_in_cycle() / DAY_LENGTH_SECS).min(1.0)
    }

    /// How far through the night we are (0..1); clamps to 0 during the day.
    pub fn night_progress(&self) -> f32 {
        ((self.time_in_cycle() - DAY_LENGTH_SECS) / NIGHT_LENGTH_SECS).clamp(0.0, 1.0)
    }

    /// Daylight factor for lighting: 1.0 through the day, 0.0 through the
    /// night, with a linear fade over `DUSK_DAWN_FADE_SECS` at dusk (end of
    /// day) and dawn (end of night).
    pub fn daylight(&self) -> f32 {
        let t = self.time_in_cycle();
        if t < DAY_LENGTH_SECS - DUSK_DAWN_FADE_SECS {
            1.0
        } else if t < DAY_LENGTH_SECS {
            (DAY_LENGTH_SECS - t) / DUSK_DAWN_FADE_SECS
        } else if t < CYCLE_LENGTH_SECS - DUSK_DAWN_FADE_SECS {
            0.0
        } else {
            1.0 - (CYCLE_LENGTH_SECS - t) / DUSK_DAWN_FADE_SECS
        }
    }

    /// "Day 2 — 14:30" for the corner clock. The day maps to the fake
    /// sunrise..sunset hours, the night wraps through midnight.
    pub fn clock_label(&self) -> String {
        let hour = match self.phase() {
            DayPhase::Day => {
                CLOCK_SUNRISE_HOUR + (CLOCK_SUNSET_HOUR - CLOCK_SUNRISE_HOUR) * self.day_progress()
            }
            DayPhase::Night => {
                CLOCK_SUNSET_HOUR
                    + (24.0 - CLOCK_SUNSET_HOUR + CLOCK_SUNRISE_HOUR) * self.night_progress()
            }
        };
        let minutes = (hour * 60.0).round() as u32 % (24 * 60);
        format!(
            "Day {} \u{2014} {:02}:{:02}",
            self.day_count(),
            minutes / 60,
            minutes % 60
        )
    }

    /// Continuous hours since Day 1 06:00 (`elapsed == 0`). Inverse of
    /// `elapsed_for_abs_hour`; keep both in sync with `clock_label`'s hour
    /// mapping if the day/night span constants ever change.
    fn abs_hour(&self) -> f32 {
        let cycle_index = (self.elapsed / CYCLE_LENGTH_SECS).floor();
        let t = self.time_in_cycle();
        let hour_in_cycle = if t < DAY_LENGTH_SECS {
            CLOCK_SUNRISE_HOUR + (CLOCK_SUNSET_HOUR - CLOCK_SUNRISE_HOUR) * (t / DAY_LENGTH_SECS)
        } else {
            CLOCK_SUNSET_HOUR
                + (24.0 - CLOCK_SUNSET_HOUR + CLOCK_SUNRISE_HOUR)
                    * ((t - DAY_LENGTH_SECS) / NIGHT_LENGTH_SECS)
        };
        cycle_index * 24.0 + (hour_in_cycle - CLOCK_SUNRISE_HOUR)
    }

    /// Inverse of `abs_hour`: the `elapsed` value for a continuous hour
    /// count since Day 1 06:00. Negative input clamps to 0 (Day 1 06:00).
    fn elapsed_for_abs_hour(abs_hour: f32) -> f32 {
        let abs_hour = abs_hour.max(0.0);
        let cycle_index = (abs_hour / 24.0).floor();
        let hour_in_cycle = (abs_hour - cycle_index * 24.0) + CLOCK_SUNRISE_HOUR;
        let t = if hour_in_cycle < CLOCK_SUNSET_HOUR {
            (hour_in_cycle - CLOCK_SUNRISE_HOUR) / (CLOCK_SUNSET_HOUR - CLOCK_SUNRISE_HOUR)
                * DAY_LENGTH_SECS
        } else {
            DAY_LENGTH_SECS
                + (hour_in_cycle - CLOCK_SUNSET_HOUR)
                    / (24.0 - CLOCK_SUNSET_HOUR + CLOCK_SUNRISE_HOUR)
                    * NIGHT_LENGTH_SECS
        };
        cycle_index * CYCLE_LENGTH_SECS + t
    }

    /// Jump the clock by whole hours (Time debug tab's +1h/-1h buttons).
    /// Mirrors `clock_label`'s hour mapping so "+1 hour" always advances the
    /// displayed clock by exactly 60 minutes, day or night; clamps at Day 1
    /// 06:00 (`elapsed = 0`) rather than going negative.
    pub fn shift_hours(&mut self, delta_hours: i32) {
        self.elapsed = Self::elapsed_for_abs_hour(self.abs_hour() + delta_hours as f32);
    }
}

/// Sim-speed multiplier (HUD speed selector: 0.5x/1x/2x/3x). Applied to
/// `Time<Virtual>`'s relative speed by `apply_game_speed`, so it scales the
/// whole sim uniformly — the clock, pawn movement, growth, work progress —
/// with no per-system plumbing. Orthogonal to `SimState::Paused`, which
/// gates individual systems off entirely rather than zeroing this out.
#[derive(Resource)]
pub struct GameSpeed(pub f32);

impl Default for GameSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Keep `Time<Virtual>`'s relative speed following the `GameSpeed` resource.
pub fn apply_game_speed(speed: Res<GameSpeed>, mut virtual_time: ResMut<Time<Virtual>>) {
    if !speed.is_changed() {
        return;
    }
    virtual_time.set_relative_speed(speed.0);
}

/// The sun ball (render layer 1: sky widget only, invisible in the world).
/// The scene's `DirectionalLight` is a separate entity (directions matter
/// for it, positions don't); `move_sky_bodies` keeps it aimed from the ball
/// toward the map center so shadows sweep with the sun.
#[derive(Component)]
pub struct Sun;

/// The moon ball (render layer 1, like the sun).
#[derive(Component)]
pub struct Moon;

/// The moon's `PointLight`, on a mesh-less child of the ball: it must stay
/// on render layer 0 to light the world (a light only lights its own
/// layers), while the ball itself lives on layer 1. Transform propagation
/// keeps it riding the arc for free.
#[derive(Component)]
pub struct MoonLight;

/// The offscreen camera rendering the sky arc to the widget texture.
#[derive(Component)]
pub struct SkyCamera;

/// Marker on every mesh making up the sun/moon balls (the balls themselves
/// and their outline-ring children) — `RenderLayers` doesn't propagate to
/// children, so `sync_sky_body_visibility` needs to reach all of them
/// directly. Toggled by the Sky debug tab's "Show Sun/Moon" switch.
#[derive(Component)]
pub struct SkyBodyVisual;

/// Devtool switch (Sky debug tab): render the sun/moon balls in the main
/// view too, not just the sky widget, so trajectories are checkable by eye.
#[derive(Resource, Default)]
pub struct ShowSkyBodies(pub bool);

/// Handle of the widget's render-target texture, for `ui::setup`.
#[derive(Resource)]
pub struct SkyWidgetImage(pub Handle<Image>);

/// Where on the sky arc a body sits at `progress` (0 = rising, 1 = setting):
/// a semicircle over the map, rising in the east (+X, screen right with the
/// default camera — north at the top) and setting in the west, like the
/// real thing. The semicircle's plane is tilted about the X axis by
/// `SUN_ARC_TILT` toward +Z, so at noon the body peaks at
/// `SUN_MAX_ELEVATION` instead of passing through zenith (see the constant's
/// doc for why vertical light is undesirable).
fn sky_position(progress: f32) -> Vec3 {
    let angle = std::f32::consts::PI * progress;
    Vec3::new(
        angle.cos() * SKY_ORBIT_RADIUS,
        angle.sin() * SKY_ORBIT_HEIGHT * SUN_ARC_TILT.cos(),
        angle.sin() * SKY_ORBIT_HEIGHT * SUN_ARC_TILT.sin(),
    )
}

/// Normalized sun height for the golden-hour ramps: 0 at the horizon
/// (sunrise/sunset), 1 at noon. `sin(π·progress)` is a proxy for the true
/// elevation on the tilted arc — monotonic with it, exact at the endpoints
/// — which is all the color/strength curves (and `power::tick_power`'s solar
/// output ramp) need.
pub fn sun_height(progress: f32) -> f32 {
    (std::f32::consts::PI * progress).sin().clamp(0.0, 1.0)
}

/// Sunlight color at `progress` through the day: horizon orange blending to
/// near-white noon, with the warmth concentrated near the horizon by
/// `SUN_COLOR_CURVE`. Shared by the scene's directional light and the sky
/// widget's sun ball.
pub fn sun_color(progress: f32) -> Color {
    SUN_COLOR_HORIZON.mix(&SUN_COLOR_NOON, sun_height(progress).powf(SUN_COLOR_CURVE))
}

/// Illuminance multiplier at `progress`: the horizon sun is dimmer as well
/// as warmer. Multiplies with the `daylight()` dusk/dawn fade.
fn sun_illuminance_scale(progress: f32) -> f32 {
    SUN_HORIZON_ILLUMINANCE_FACTOR + (1.0 - SUN_HORIZON_ILLUMINANCE_FACTOR) * sun_height(progress)
}

/// The base yaw (radians, about world +Y) a single-axis solar tracker should
/// hold at `progress` (0..1 through the day, see `GameClock::day_progress`)
/// to face the sun's azimuth. The sun's arc sits in a fixed plane tilted
/// toward +Z (see `sky_position`), so this sweep — east (`progress` 0)
/// through the camera-facing side (`progress` 0.5) to west (`progress` 1) —
/// matches the true azimuth exactly at the ends and at noon, and only
/// approximates the in-between hours; same map-wide-approximation spirit as
/// `weather::Wind`'s uniform direction. Used by
/// `construction::aim_solar_panels`.
///
/// The panel's rest-forward (yaw = 0) reference axis is **`Vec3::Z`**, not
/// `Vec3::X` — `construction::spawn_solar_parts` tilts the array about
/// local X (`SOLAR_ARRAY_TILT`), which tips the flat plate's +Y normal
/// toward +Z, not +X (work through `Quat::from_rotation_x`'s matrix and the
/// rest-normal ends up `(0, cosφ, sinφ)`). So `Quat::from_rotation_y(yaw) *
/// Vec3::Z` — not `* Vec3::X` — is the panel's true ground-projected facing,
/// `(sin(yaw), cos(yaw))`. Solve for the desired east/south/west facings
/// against that axis to get this formula; if the tilt axis in
/// `spawn_solar_parts` ever changes, this phase must be re-derived too.
pub fn sun_yaw(progress: f32) -> f32 {
    std::f32::consts::PI * (0.5 - progress)
}

/// Spawn the sun and moon balls (unlit — they're light sources, not lit
/// surfaces — and layer-1, so only the sky widget's camera sees them), the
/// moon's world-side point light, and the widget's render-to-texture camera.
/// Each ball carries an inverted-hull outline child: a slightly larger black
/// sphere culling front faces, so only its back shows — a ring around the
/// ball from any angle. `RenderLayers` doesn't propagate, so the children
/// carry layer 1 themselves; `Visibility` does, so show/hide stays free.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let outline_material = materials.add(StandardMaterial {
        base_color: SKY_BODY_OUTLINE_COLOR,
        unlit: true,
        cull_mode: Some(Face::Front),
        ..default()
    });
    commands
        .spawn((
            Mesh3d(meshes.add(Sphere::new(SUN_RADIUS))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: SUN_COLOR,
                unlit: true,
                ..default()
            })),
            Transform::from_translation(sky_position(0.0)),
            Name::new("Sun"),
            Sun,
            SkyBodyVisual,
            RenderLayers::layer(1),
            NotShadowCaster,
            NotShadowReceiver,
        ))
        .with_child((
            Mesh3d(meshes.add(Sphere::new(SUN_RADIUS + SKY_BODY_OUTLINE_WIDTH))),
            MeshMaterial3d(outline_material.clone()),
            SkyBodyVisual,
            RenderLayers::layer(1),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    commands
        .spawn((
            Mesh3d(meshes.add(Sphere::new(MOON_RADIUS))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: MOON_COLOR,
                unlit: true,
                ..default()
            })),
            Transform::from_translation(sky_position(0.0)),
            Visibility::Hidden,
            Name::new("Moon"),
            Moon,
            SkyBodyVisual,
            RenderLayers::layer(1),
            NotShadowCaster,
            NotShadowReceiver,
        ))
        .with_child((
            Mesh3d(meshes.add(Sphere::new(MOON_RADIUS + SKY_BODY_OUTLINE_WIDTH))),
            MeshMaterial3d(outline_material),
            SkyBodyVisual,
            RenderLayers::layer(1),
            NotShadowCaster,
            NotShadowReceiver,
        ))
        .with_child((
            PointLight {
                color: MOON_LIGHT_COLOR,
                intensity: 0.0,
                range: MOON_LIGHT_RANGE,
                shadows_enabled: false,
                ..default()
            },
            MoonLight,
        ));

    // The sky widget: an offscreen camera framing the whole arc, rendering
    // layer 1 to a texture the UI shows above the clock.
    let image = Image::new_target_texture(
        SKY_TEXTURE_WIDTH,
        SKY_TEXTURE_HEIGHT,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    );
    let image_handle = images.add(image);
    commands.insert_resource(SkyWidgetImage(image_handle.clone()));
    commands.spawn((
        Camera3d::default(),
        Camera {
            // Render before the main pass; the world doesn't depend on it.
            order: -1,
            clear_color: ClearColorConfig::Custom(SKY_DAY_COLOR),
            ..default()
        },
        RenderTarget::Image(image_handle.into()),
        // The widget is flat color art: skip tonemapping so the sky and the
        // sun/moon show the configured colors instead of a compressed copy.
        Tonemapping::None,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: SKY_VIEW_WIDTH,
                height: SKY_VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        // Centered so the view spans the arc plus some ground clearance:
        // y in [center - h/2, center + h/2] must contain the zenith ball.
        Transform::from_xyz(0.0, SKY_VIEW_HEIGHT / 2.0 - 1.5, 30.0)
            .looking_at(Vec3::new(0.0, SKY_VIEW_HEIGHT / 2.0 - 1.5, 0.0), Vec3::Y),
        RenderLayers::layer(1),
        SkyCamera,
    ));
}

/// Advance the clock. Gated on `SimState::Running`, so pause stops time.
pub fn tick_clock(time: Res<Time>, mut clock: ResMut<GameClock>) {
    clock.elapsed += time.delta_secs();
}

/// Ride the sun and moon along the sky arc — sun by day, moon by night —
/// and keep the directional light aimed from the sun at the map center so
/// shadows sweep across the day. The moon's point light fades with darkness
/// (`1 - daylight`), so it never leaks into the day.
#[allow(clippy::type_complexity)]
pub fn move_sky_bodies(
    clock: Res<GameClock>,
    sun: Single<(&mut Transform, &mut Visibility), (With<Sun>, Without<Moon>)>,
    moon: Single<(&mut Transform, &mut Visibility), (With<Moon>, Without<Sun>)>,
    moon_light: Single<&mut PointLight, With<MoonLight>>,
    sun_light: Single<&mut Transform, (With<DirectionalLight>, Without<Sun>, Without<Moon>)>,
) {
    let is_day = clock.phase() == DayPhase::Day;

    let (mut sun_transform, mut sun_visibility) = sun.into_inner();
    sun_transform.translation = sky_position(clock.day_progress());
    *sun_visibility = if is_day {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    *sun_light.into_inner() =
        Transform::from_translation(sun_transform.translation).looking_at(Vec3::ZERO, Vec3::Y);

    let (mut moon_transform, mut moon_visibility) = moon.into_inner();
    moon_transform.translation = sky_position(clock.night_progress());
    *moon_visibility = if is_day {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    moon_light.into_inner().intensity = MOON_LIGHT_INTENSITY * (1.0 - clock.daylight());
}

/// Add/remove layer 0 (the main camera's layer) on every `SkyBodyVisual`
/// while keeping layer 1 (the widget always sees them) — the existing
/// per-phase `Visibility` toggle in `move_sky_bodies` still hides whichever
/// body isn't currently up, so this only ever reveals the one actually on
/// its arc.
pub fn sync_sky_body_visibility(
    show: Res<ShowSkyBodies>,
    mut visuals: Query<&mut RenderLayers, With<SkyBodyVisual>>,
) {
    let wanted = if show.0 {
        RenderLayers::layer(1).with(0)
    } else {
        RenderLayers::layer(1)
    };
    for mut layers in &mut visuals {
        if *layers != wanted {
            *layers = wanted.clone();
        }
    }
}

/// Paint the widget's sky: night <-> day by the daylight factor, with a warm
/// dusk tint blended in mid-fade (`4d(1-d)` peaks at the middle of dawn and
/// dusk, zero in full day or night) — the sunset happens in the widget.
pub fn paint_sky(clock: Res<GameClock>, camera: Single<&mut Camera, With<SkyCamera>>) {
    let daylight = clock.daylight();
    let sky = SKY_NIGHT_COLOR.mix(&SKY_DAY_COLOR, daylight);
    let dusk = 4.0 * daylight * (1.0 - daylight) * SKY_DUSK_STRENGTH;
    camera.into_inner().clear_color = ClearColorConfig::Custom(sky.mix(&SKY_DUSK_COLOR, dusk));
}

/// Tint the widget's unlit sun ball toward horizon orange at dawn/dusk,
/// alongside `paint_sky`'s dusk sky. Same height curve as the real light,
/// but ramping to the ball's own icon yellow (`SUN_COLOR`) at noon — the
/// light's near-white noon color would wash the icon out against the sky.
pub fn paint_sun_ball(
    clock: Res<GameClock>,
    ball: Single<&MeshMaterial3d<StandardMaterial>, With<Sun>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if let Some(material) = materials.get_mut(&ball.0) {
        let height = sun_height(clock.day_progress());
        material.base_color = SUN_COLOR_HORIZON.mix(&SUN_COLOR, height.powf(SUN_COLOR_CURVE));
    }
}

/// Fade the scene lighting with the clock: ambient brightness and color lerp
/// between the day and night presets, and the sun's directional light dims
/// to nothing at night. On top of that, the golden-hour ramps (see the
/// `SUN_COLOR_HORIZON` doc): the sun warms and weakens toward the horizon
/// while the ambient counter-tints toward twilight blue. `day_progress()`
/// clamps to 1 at night, so the height math is harmless while the sun is
/// off anyway.
pub fn apply_lighting(
    clock: Res<GameClock>,
    ambient: Single<&mut AmbientLight, With<Camera3d>>,
    sun: Single<&mut DirectionalLight>,
) {
    let daylight = clock.daylight();
    let progress = clock.day_progress();
    let mut ambient = ambient.into_inner();
    ambient.brightness =
        NIGHT_AMBIENT_BRIGHTNESS + (DAY_AMBIENT_BRIGHTNESS - NIGHT_AMBIENT_BRIGHTNESS) * daylight;
    ambient.color = NIGHT_AMBIENT_COLOR.mix(&DAY_AMBIENT_COLOR, daylight).mix(
        &TWILIGHT_AMBIENT_COLOR,
        (1.0 - sun_height(progress)) * daylight * TWILIGHT_AMBIENT_STRENGTH,
    );
    let mut sun = sun.into_inner();
    sun.illuminance = SUN_ILLUMINANCE * daylight * sun_illuminance_scale(progress);
    sun.color = sun_color(progress);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(elapsed: f32) -> GameClock {
        GameClock { elapsed }
    }

    #[test]
    fn phases_split_at_day_length() {
        assert_eq!(at(0.0).phase(), DayPhase::Day);
        assert_eq!(at(DAY_LENGTH_SECS - 0.1).phase(), DayPhase::Day);
        assert_eq!(at(DAY_LENGTH_SECS).phase(), DayPhase::Night);
        assert_eq!(at(CYCLE_LENGTH_SECS - 0.1).phase(), DayPhase::Night);
    }

    #[test]
    fn cycle_wraps_into_the_next_day() {
        let clock = at(CYCLE_LENGTH_SECS + 1.0);
        assert_eq!(clock.phase(), DayPhase::Day);
        assert_eq!(clock.day_count(), 2);
        assert_eq!(at(0.0).day_count(), 1);
    }

    #[test]
    fn daylight_full_at_midday_zero_at_midnight() {
        assert_eq!(at(DAY_LENGTH_SECS / 2.0).daylight(), 1.0);
        let midnight = DAY_LENGTH_SECS + NIGHT_LENGTH_SECS / 2.0;
        assert_eq!(at(midnight).daylight(), 0.0);
    }

    #[test]
    fn daylight_fades_through_dusk_and_dawn() {
        let mid_dusk = DAY_LENGTH_SECS - DUSK_DAWN_FADE_SECS / 2.0;
        assert!((at(mid_dusk).daylight() - 0.5).abs() < 1e-4);
        let mid_dawn = CYCLE_LENGTH_SECS - DUSK_DAWN_FADE_SECS / 2.0;
        assert!((at(mid_dawn).daylight() - 0.5).abs() < 1e-4);
    }

    #[test]
    fn progress_tracks_each_phase() {
        assert_eq!(at(DAY_LENGTH_SECS / 2.0).day_progress(), 0.5);
        assert_eq!(at(DAY_LENGTH_SECS / 2.0).night_progress(), 0.0);
        let mid_night = DAY_LENGTH_SECS + NIGHT_LENGTH_SECS / 2.0;
        assert_eq!(at(mid_night).night_progress(), 0.5);
        assert_eq!(at(mid_night).day_progress(), 1.0);
    }

    #[test]
    fn sky_bodies_rise_east_and_set_west() {
        // +X is east (screen right, north at the top of the screen).
        assert!(sky_position(0.0).x > 0.0);
        assert!(sky_position(1.0).x < 0.0);
        // The tilted arc peaks below the untilted height, on the
        // camera-facing (+Z) side.
        let noon = sky_position(0.5);
        assert_eq!(noon.y, SKY_ORBIT_HEIGHT * SUN_ARC_TILT.cos());
        assert!(noon.z > 0.0);
    }

    #[test]
    fn sun_never_reaches_zenith() {
        // The light aims from the ball at the origin, so its elevation is
        // the ball's own angle above the horizon plane. It must peak at
        // exactly SUN_MAX_ELEVATION (noon) and never exceed it.
        let mut max_elevation: f32 = 0.0;
        for i in 0..=20 {
            let pos = sky_position(i as f32 / 20.0);
            let elevation = pos.y.atan2(pos.xz().length());
            assert!(elevation <= SUN_MAX_ELEVATION + 1e-4);
            max_elevation = max_elevation.max(elevation);
        }
        assert!((max_elevation - SUN_MAX_ELEVATION).abs() < 1e-3);
    }

    #[test]
    fn sun_yaw_sweeps_east_to_camera_to_west() {
        // Rest-forward is +Z (see `sun_yaw`'s doc comment: the array's tilt
        // is about local X, which tips its +Y normal toward +Z, not +X),
        // rotated by `Quat::from_rotation_y(sun_yaw(p))`: east at sunrise,
        // the camera-facing side at noon, west at sunset.
        let facing = |p: f32| Quat::from_rotation_y(sun_yaw(p)) * Vec3::Z;
        assert!(facing(0.0).x > 0.0);
        assert!(facing(1.0).x < 0.0);
        assert!(facing(0.5).z > 0.0);
        // Continuous and monotonic through the sweep.
        let mut prev = sun_yaw(0.0);
        for i in 1..=10 {
            let p = i as f32 / 10.0;
            let yaw = sun_yaw(p);
            assert!(yaw <= prev);
            prev = yaw;
        }
    }

    #[test]
    fn sun_is_warm_at_horizon_white_at_noon() {
        assert_eq!(sun_color(0.0), SUN_COLOR_HORIZON);
        assert_eq!(sun_color(1.0), SUN_COLOR_HORIZON);
        assert_eq!(sun_color(0.5), SUN_COLOR_NOON);
        // Whitening monotonically as the sun climbs: the blue channel (the
        // one the horizon orange lacks) strictly rises toward noon.
        let mut prev = -1.0;
        for i in 0..=10 {
            let blue = sun_color(i as f32 / 20.0).to_srgba().blue;
            assert!(blue > prev);
            prev = blue;
        }
    }

    #[test]
    fn sun_dims_toward_the_horizon() {
        assert_eq!(sun_illuminance_scale(0.0), SUN_HORIZON_ILLUMINANCE_FACTOR);
        assert_eq!(sun_illuminance_scale(1.0), SUN_HORIZON_ILLUMINANCE_FACTOR);
        assert_eq!(sun_illuminance_scale(0.5), 1.0);
        // Monotonic on each half of the day.
        for i in 0..10 {
            let (a, b) = (i as f32 / 20.0, (i + 1) as f32 / 20.0);
            assert!(sun_illuminance_scale(a) < sun_illuminance_scale(b));
            assert!(sun_illuminance_scale(1.0 - a) < sun_illuminance_scale(1.0 - b));
        }
    }

    #[test]
    fn clock_label_reads_like_a_wall_clock() {
        assert_eq!(at(0.0).clock_label(), "Day 1 \u{2014} 06:00");
        assert_eq!(
            at(DAY_LENGTH_SECS / 2.0).clock_label(),
            "Day 1 \u{2014} 14:00"
        );
        assert_eq!(at(DAY_LENGTH_SECS).clock_label(), "Day 1 \u{2014} 22:00");
        let mid_night = DAY_LENGTH_SECS + NIGHT_LENGTH_SECS / 2.0;
        assert_eq!(at(mid_night).clock_label(), "Day 1 \u{2014} 02:00");
    }

    #[test]
    fn shift_hours_advances_the_wall_clock_by_exactly_an_hour() {
        let mut clock = at(0.0);
        clock.shift_hours(1);
        assert_eq!(clock.clock_label(), "Day 1 \u{2014} 07:00");
    }

    #[test]
    fn shift_hours_clamps_at_day_one_six_am() {
        let mut clock = at(0.0);
        clock.shift_hours(-1);
        assert_eq!(clock.clock_label(), "Day 1 \u{2014} 06:00");
        assert_eq!(clock.elapsed, 0.0);
    }

    #[test]
    fn shift_hours_straddles_dusk_into_the_night_span() {
        // Day 1 21:00 + 1 hour lands exactly on the day/night boundary,
        // reported as 22:00 either way.
        let mut clock = at(0.0);
        clock.shift_hours(15); // 06:00 -> 21:00
        assert_eq!(clock.clock_label(), "Day 1 \u{2014} 21:00");
        clock.shift_hours(1);
        assert_eq!(clock.clock_label(), "Day 1 \u{2014} 22:00");
        assert_eq!(clock.phase(), DayPhase::Night);
    }

    #[test]
    fn shift_hours_rolls_the_day_count_across_midnight() {
        let mut clock = at(0.0);
        clock.shift_hours(24); // one full cycle: Day 1 06:00 -> Day 2 06:00
        assert_eq!(clock.day_count(), 2);
        assert_eq!(clock.clock_label(), "Day 2 \u{2014} 06:00");
    }
}
