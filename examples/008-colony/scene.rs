use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::ScalingMode;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, ShadowFilteringMethod};
use bevy::prelude::*;

use crate::config::*;
use crate::ui::PawnTabWindow;

/// The world-view camera. Every cursor/viewport query must filter on this:
/// the sky widget adds a second offscreen `Camera3d`, and an unfiltered
/// `Single<&Camera>` silently skips its system once two cameras exist.
#[derive(Component)]
pub struct MainCamera;

/// Which of the two camera views (isometric build view / straight-down
/// top-down view) is active, and any in-flight animated swap between them.
/// Owned entirely by `toggle_camera_view` (F2).
#[derive(Resource, Default)]
pub struct CameraViewMode {
    pub top_down: bool,
    anim: Option<CameraAnim>,
    /// The isometric transform (including any orbit yaw from `orbit_camera`)
    /// to restore when leaving top-down, captured the moment we switch away
    /// from it.
    saved_iso: Option<Transform>,
}

struct CameraAnim {
    from: Transform,
    to: Transform,
    from_extents: (f32, f32),
    to_extents: (f32, f32),
    elapsed: f32,
}

/// True while the isometric view is active and settled (no swap animating).
/// Gates `orbit_camera`: orbiting only makes sense isometric, and locking it
/// out during the transition keeps the swap itself predictable.
pub fn iso_view_active(mode: Res<CameraViewMode>) -> bool {
    !mode.top_down && mode.anim.is_none()
}

/// True once the current view (isometric or top-down) has finished
/// transitioning. Gates `zoom_camera`/`pan_camera` so input doesn't fight the
/// ~`CAMERA_VIEW_TRANSITION_SECS` animated swap.
pub fn camera_settled(mode: Res<CameraViewMode>) -> bool {
    mode.anim.is_none()
}

pub fn setup(mut commands: Commands) {
    commands.insert_resource(DirectionalLightShadowMap {
        size: SUN_SHADOW_MAP_SIZE as usize,
    });

    // Orthographic camera starting in a classic isometric view: 45 degrees
    // of yaw (corner-on, walls show two faces) and atan(1/sqrt(2)) of pitch
    // (equal foreshortening on all three axes) — that's what looking at the
    // origin from (d, d, d) gives. MMB-orbit changes the yaw from there.
    // F2 (`toggle_camera_view`) swaps to a straight-down top-down view.
    let (min_width, min_height) = iso_scaling_extents();
    commands.spawn((
        Camera3d::default(),
        MainCamera,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width,
                min_height,
            },
            ..OrthographicProjection::default_3d()
        }),
        default_iso_transform(),
        AmbientLight {
            brightness: 250.0,
            ..default()
        },
        // Temporal shadow filtering dithers the PCSS sampling pattern across
        // frames instead of using a fixed kernel, so the residual texel-swim
        // from the day/night cycle's slowly rotating sun blends into noise
        // TAA smooths out, rather than reading as a discrete shadow "jump".
        // Requires MSAA off; note bevy_hanabi doesn't write motion vectors,
        // so rain/fire/gust particles may show slight temporal ghosting.
        ShadowFilteringMethod::Temporal,
        Msaa::Off,
        TemporalAntiAliasing::default(),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: SUN_ILLUMINANCE,
            shadows_enabled: true,
            soft_shadow_size: Some(SUN_SOFT_SHADOW_SIZE),
            shadow_depth_bias: SUN_SHADOW_DEPTH_BIAS,
            ..default()
        },
        // Bevy's default is 4 cascades split by camera-space depth, tuned for
        // perspective cameras where distant cascades can be coarser. Our
        // camera is orthographic, so every cascade covers the same frustum
        // width/height regardless of depth — extra cascades buy no
        // resolution, they just chop the (small, static) map into
        // independently-snapped shadow grids. Objects sitting near a
        // cascade seam would flip between the two grids' registrations
        // every frame, visible as shadows "jumping" on crisp-edged casters
        // (walls, pawns) while soft tree canopies hid it. One cascade
        // spanning the whole map's camera-space depth (const-derived — an
        // undersized reach shows as a horizontal unshadowed seam, see
        // SUN_SHADOW_MAX_DISTANCE) removes the seams entirely.
        CascadeShadowConfigBuilder {
            num_cascades: 1,
            minimum_distance: 1.0,
            maximum_distance: SUN_SHADOW_MAX_DISTANCE,
            ..default()
        }
        .build(),
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, -0.6, -1.0)),
    ));
}

/// The canonical isometric transform: camera at `(d, d, d)` looking at the
/// origin. The starting view, and the top-down toggle's fallback if there's
/// no saved orbit yaw to restore (see `CameraViewMode::saved_iso`).
fn default_iso_transform() -> Transform {
    Transform::from_xyz(
        CAMERA_START_DISTANCE,
        CAMERA_START_DISTANCE,
        CAMERA_START_DISTANCE,
    )
    .looking_at(Vec3::ZERO, Vec3::Y)
}

/// Isometric `ScalingMode::AutoMin` extents: corner-on, the map spans its
/// diagonal (sqrt(2) x wider).
fn iso_scaling_extents() -> (f32, f32) {
    (
        MAP_WIDTH as f32 * TILE_SIZE * std::f32::consts::SQRT_2 + 4.0,
        MAP_HEIGHT as f32 * TILE_SIZE + 4.0,
    )
}

/// Top-down `ScalingMode::AutoMin` extents: axis-aligned, so (unlike the
/// isometric view) no diagonal factor is needed.
fn topdown_scaling_extents() -> (f32, f32) {
    (CAMERA_TOPDOWN_MIN_WIDTH, CAMERA_TOPDOWN_MIN_HEIGHT)
}

/// Where a camera sitting at `translation` and facing `forward` hits the
/// ground plane (`y = 0`) — "what the camera is currently looking at".
/// `None` for a forward ray parallel to the ground (never happens for either
/// camera view in practice, but a horizontal camera would divide by zero).
fn ground_focus(translation: Vec3, forward: Vec3) -> Option<Vec3> {
    if forward.y == 0.0 {
        return None;
    }
    Some(translation + forward * (-translation.y / forward.y))
}

/// Slide the camera (in the ground plane, keeping its angle) so its view
/// centers on `target`, clamped to the map box like `pan_camera`. The focus
/// point is where the camera's forward ray hits the ground.
pub fn center_camera_on(camera: &mut Transform, target: Vec3) {
    let forward = *camera.forward();
    let Some(focus) = ground_focus(camera.translation, forward) else {
        return;
    };
    let mut shift = target - focus;
    shift.y = 0.0;
    camera.translation += shift;
    let Some(focus) = ground_focus(camera.translation, forward) else {
        return;
    };
    camera.translation.x += focus.x.clamp(-CAMERA_PAN_LIMIT, CAMERA_PAN_LIMIT) - focus.x;
    camera.translation.z += focus.z.clamp(-CAMERA_PAN_LIMIT, CAMERA_PAN_LIMIT) - focus.z;
}

/// F2 swaps between the isometric build view and a straight-down top-down
/// view (axis-aligned, north-up — easier to lay out rectangular rooms),
/// animating the transform and `ScalingMode::AutoMin` extents over
/// `CAMERA_VIEW_TRANSITION_SECS`. Runs on real time (`Time`, not the sim
/// clock) so it plays the same whether the sim is paused or not, matching
/// every other camera control staying ungated (`docs/00-architecture.md`).
/// Re-entering top-down always re-centers on wherever the camera currently
/// looks; leaving it restores the saved isometric orbit yaw re-centered the
/// same way (`center_camera_on`).
pub fn toggle_camera_view(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut mode: ResMut<CameraViewMode>,
    camera: Single<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    let (mut transform, mut projection) = camera.into_inner();
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    // Ignore F2 while a swap is already animating: a press mid-transition
    // would capture the halfway transform into `saved_iso`, permanently
    // corrupting the isometric pose every later "return to iso" restores.
    if keys.just_pressed(CAMERA_TOPDOWN_TOGGLE_KEY) && mode.anim.is_none() {
        let from_extents = match ortho.scaling_mode {
            ScalingMode::AutoMin {
                min_width,
                min_height,
            } => (min_width, min_height),
            _ => iso_scaling_extents(),
        };
        let focus =
            ground_focus(transform.translation, *transform.forward()).unwrap_or(Vec3::ZERO);
        let (to, to_extents) = if mode.top_down {
            // Leaving top-down: restore the saved isometric angle (or the
            // canonical one if there's none yet), re-centered on the current
            // focus so the player doesn't lose their place.
            let mut iso = mode.saved_iso.take().unwrap_or_else(default_iso_transform);
            center_camera_on(&mut iso, focus);
            (iso, iso_scaling_extents())
        } else {
            // Entering top-down: remember the isometric angle (including any
            // orbit yaw) to restore later, then look straight down at the
            // same focus. `Vec3::Y` up is degenerate looking straight down,
            // so use world -Z (matches the grid's +y == world -Z convention,
            // keeping "north" at the top of the screen).
            mode.saved_iso = Some(*transform);
            (
                Transform::from_translation(focus + Vec3::Y * CAMERA_START_DISTANCE)
                    .looking_at(focus, Vec3::NEG_Z),
                topdown_scaling_extents(),
            )
        };
        mode.top_down = !mode.top_down;
        mode.anim = Some(CameraAnim {
            from: *transform,
            to,
            from_extents,
            to_extents,
            elapsed: 0.0,
        });
    }

    let Some(anim) = mode.anim.as_mut() else {
        return;
    };
    anim.elapsed += time.delta_secs();
    let t = (anim.elapsed / CAMERA_VIEW_TRANSITION_SECS).clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t); // smoothstep

    transform.translation = anim.from.translation.lerp(anim.to.translation, eased);
    transform.rotation = anim.from.rotation.slerp(anim.to.rotation, eased);
    ortho.scaling_mode = ScalingMode::AutoMin {
        min_width: anim.from_extents.0 + (anim.to_extents.0 - anim.from_extents.0) * eased,
        min_height: anim.from_extents.1 + (anim.to_extents.1 - anim.from_extents.1) * eased,
    };

    if t >= 1.0 {
        mode.anim = None;
    }
}

/// Hold the middle mouse button and move the mouse horizontally to orbit the
/// camera around the map center. `rotate_around` spins both the position and
/// the orientation about the origin, so the camera keeps facing the map.
/// Gated on `iso_view_active`: top-down is locked north-up (that's the point
/// of the view — axis-aligned rooms), so orbit is disabled there.
pub fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut camera: Single<&mut Transform, With<MainCamera>>,
) {
    if !buttons.pressed(MouseButton::Middle) {
        return;
    }
    let dx = motion.delta.x;
    if dx == 0.0 {
        return;
    }
    let yaw = Quat::from_rotation_y(-dx * CAMERA_ORBIT_SPEED);
    camera.rotate_around(Vec3::ZERO, yaw);
}

/// Scroll the mouse wheel to zoom. The camera is orthographic, so we scale the
/// projection rather than move the camera. Scrolling up shrinks `scale` (zoom
/// in); the result is clamped so the view can't invert or run away. Skips
/// entirely while the mouse is over the pawn tab window (Job History / Skills
/// / Needs), so scrolling its Job History list (`ui::scroll_history`) doesn't
/// also zoom the map underneath. Works in both camera views (gated on
/// `camera_settled`, not `iso_view_active`); `toggle_camera_view` animates
/// `scaling_mode`'s extents directly and leaves this `scale` factor alone, so
/// the player's zoom level survives the swap.
pub fn zoom_camera(
    scroll: Res<AccumulatedMouseScroll>,
    mut projection: Single<&mut Projection, With<MainCamera>>,
    tab_window: Query<&Interaction, With<PawnTabWindow>>,
) {
    let dy = scroll.delta.y;
    if dy == 0.0 {
        return;
    }
    if tab_window.iter().any(|i| *i != Interaction::None) {
        return;
    }
    if let Projection::Orthographic(ortho) = projection.as_mut() {
        ortho.scale =
            (ortho.scale * (1.0 - dy * CAMERA_ZOOM_SPEED)).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX);
    }
}

/// Player-remappable pan keys. Defaults to WASD (from `config.rs`). Stored as a
/// resource so a future settings screen can rebind them without a recompile.
#[derive(Resource)]
pub struct CameraControls {
    pub forward: KeyCode,
    pub back: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
}

impl Default for CameraControls {
    fn default() -> Self {
        Self {
            forward: CAMERA_PAN_FORWARD_KEY,
            back: CAMERA_PAN_BACK_KEY,
            left: CAMERA_PAN_LEFT_KEY,
            right: CAMERA_PAN_RIGHT_KEY,
        }
    }
}

/// WASD pans the viewport across the ground plane, relative to the current
/// camera facing (so it stays screen-correct after orbiting). The look-at point
/// is clamped to the map box so you can't drift off into empty space. Works
/// in both camera views (gated on `camera_settled`, not `iso_view_active`).
pub fn pan_camera(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<CameraControls>,
    time: Res<Time>,
    camera: Single<(&mut Transform, &Projection), With<MainCamera>>,
) {
    let (mut transform, projection) = camera.into_inner();
    let forward = transform.forward();
    let right = transform.right();
    let ground = |v: Vec3| Vec3::new(v.x, 0.0, v.z).normalize_or_zero();

    // Top-down looks straight down, so `forward` is purely vertical and
    // `ground(forward)` collapses to zero — fall back to the camera's up
    // axis (horizontal there, since top-down's up is world -Z) so W/S still
    // pan instead of going dead.
    let forward_dir = {
        let f = ground(*forward);
        if f == Vec3::ZERO {
            ground(*transform.up())
        } else {
            f
        }
    };

    let mut dir = Vec3::ZERO;
    if keys.pressed(controls.forward) {
        dir += forward_dir;
    }
    if keys.pressed(controls.back) {
        dir -= forward_dir;
    }
    if keys.pressed(controls.right) {
        dir += ground(*right);
    }
    if keys.pressed(controls.left) {
        dir -= ground(*right);
    }
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return;
    }

    // Pan speed tracks zoom so a drag covers the same screen distance at any zoom.
    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    transform.translation += dir * CAMERA_PAN_SPEED * scale * time.delta_secs();

    // Clamp: find where the view ray hits the ground, pull it back inside the
    // map box, and shift the camera by the same correction.
    if let Some(focus) = ground_focus(transform.translation, *forward) {
        transform.translation.x += focus.x.clamp(-CAMERA_PAN_LIMIT, CAMERA_PAN_LIMIT) - focus.x;
        transform.translation.z += focus.z.clamp(-CAMERA_PAN_LIMIT, CAMERA_PAN_LIMIT) - focus.z;
    }
}
