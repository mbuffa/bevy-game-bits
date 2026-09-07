//! What one map is *made of* ([`WorldMapConfig`]), what it *looks like*
//! ([`WorldMapTheme`]), where it *sits* ([`WorldMapLayout`]), and the
//! [`WorldMapSpec`] that bundles all three plus a [`WorldMapSource`] into the
//! one argument [`spawn_world_map`](super::spawn_world_map) takes.
//!
//! The `Config` / `Theme` split is the same one `inventory::config` makes: the
//! **model** — travel speed, arrival tolerance, reveal radius — is what
//! `travel.rs` and the camera clamp read, and the **view** — every colour,
//! size, and z-layer — is what `ui.rs` reads. Re-skinning through
//! [`WorldMapTheme`] provably can't change where the traveller goes.

use std::borrow::Cow;

use bevy::prelude::*;

use crate::world_map::asset::WorldMapSource;

/// The numbers the travel **model** runs on. A `Component` on the map entity.
#[derive(Component, Clone, Debug)]
pub struct WorldMapConfig {
    /// Base travel speed over `speed: 1.0` terrain, in tiles per second. The
    /// terrain multiplier scales this down.
    pub travel_tiles_per_sec: f32,
    /// The traveller has arrived once it's within this many tiles of the
    /// target.
    pub arrive_epsilon_tiles: f32,
    /// Arrow-key pan speed, in world units per second.
    pub pan_px_per_sec: f32,
    /// Whether the camera eases to keep the traveller in view while it's
    /// moving. A manual pan suspends it until the next target is set.
    pub follow_traveler: bool,
    /// Per-second fraction of the remaining distance the following camera
    /// closes each frame (`1.0 - follow_lerp` is the frame-rate-independent
    /// decay base). `12.0` is a firm-but-smooth follow.
    pub follow_lerp: f32,
    /// How close (in tiles) the traveller's path must pass an undiscovered
    /// **ordinary** location for it to be revealed. `None` means proximity
    /// never reveals an ordinary location — one stays hidden until its
    /// `discovered` flag is set some other way.
    pub reveal_radius_tiles: Option<f32>,
    /// How close (in tiles) the traveller's path must pass an undiscovered
    /// [`SecretLocation`](super::SecretLocation) — deliberately tiny, since a
    /// secret is a spot you have to walk almost exactly. Independent of
    /// [`reveal_radius_tiles`](Self::reveal_radius_tiles); `None` disables
    /// proximity reveal for secrets only.
    pub secret_reveal_radius_tiles: Option<f32>,
    /// How close (in tiles) a [`Party`](super::Party) must come to the player
    /// for [`track_intercepts`](super::track_intercepts) to register contact.
    /// `None` disables interception entirely.
    pub intercept_radius_tiles: Option<f32>,
    /// How far above the token, in world units, the "interact" square sits.
    pub interact_widget_offset_px: f32,
    /// Half the edge length, in world units, of the "interact" square — its
    /// click hit box and its drawn size both.
    pub interact_widget_half_px: f32,
    /// Verb the interact menu's location row reads — `"{enter_verb} {name}"`.
    pub enter_verb: Cow<'static, str>,
    /// Verb the interact menu's party rows read — `"{hail_verb} {name}"`.
    pub hail_verb: Cow<'static, str>,
    /// Heading shown above the aside list. `None` draws no heading row.
    pub title: Option<Cow<'static, str>>,
    /// Show the live `you … · cursor …` tile-coordinate line in the aside — the
    /// aiming aid a sub-tile secret hunt needs. `false` hides it.
    pub show_coords: bool,
    /// Pause [`WorldMapTime`](super::WorldMapTime) — and so all party movement
    /// and the clock — whenever the player isn't travelling (the Fallout 1/2
    /// overworld: it only runs while you walk). `false` runs the world
    /// continuously in real time. A [`WorldClockHold`](super::WorldClockHold) on
    /// the map keeps time flowing regardless.
    pub pause_time_when_idle: bool,
}

impl Default for WorldMapConfig {
    fn default() -> Self {
        Self {
            travel_tiles_per_sec: 1.2,
            arrive_epsilon_tiles: 0.04,
            pan_px_per_sec: 320.0,
            follow_traveler: true,
            follow_lerp: 12.0,
            reveal_radius_tiles: Some(1.6),
            secret_reveal_radius_tiles: Some(0.15),
            intercept_radius_tiles: Some(0.35),
            interact_widget_offset_px: 26.0,
            interact_widget_half_px: 11.0,
            enter_verb: Cow::Borrowed("Enter"),
            hail_verb: Cow::Borrowed("Hail"),
            title: Some(Cow::Borrowed("WORLD MAP")),
            show_coords: true,
            pause_time_when_idle: true,
        }
    }
}

/// Every colour, size, and z-layer one map is drawn with. A `Component` on the
/// map entity. Nothing here is read by the travel model.
#[derive(Component, Clone, Debug)]
pub struct WorldMapTheme {
    /// Fills the gap left by [`tile_inset_px`](Self::tile_inset_px), reading as
    /// grid lines between tiles, and the void beyond the map edge.
    pub grid_color: Color,
    /// Each tile quad is drawn this many world units smaller than its cell on
    /// every side, letting [`grid_color`](Self::grid_color) show through as a
    /// seam.
    pub tile_inset_px: f32,
    pub location_radius_px: f32,
    pub location_color: Color,
    /// Dot for a discovered [`SecretLocation`](super::SecretLocation) — smaller
    /// and cooler, so a found secret reads apart from a town.
    pub secret_location_radius_px: f32,
    pub secret_location_color: Color,
    pub location_label_color: Color,
    pub location_label_font_size: f32,
    /// Gap between the top of a location's circle and its name label. Reused for
    /// a [`Party`](super::Party)'s name label.
    pub location_label_gap_px: f32,
    pub traveler_radius_px: f32,
    pub traveler_color: Color,
    pub target_marker_radius_px: f32,
    pub target_marker_color: Color,
    /// Half-diagonal of a [`Party`](super::Party)'s diamond token.
    pub party_radius_px: f32,
    pub party_label_color: Color,
    /// Fill of the single "interact" square over the player token.
    pub interact_widget_color: Color,
    /// Popup panel behind the interact square when it lists more than one thing.
    pub menu_background: Color,
    pub menu_row_background: Color,
    pub menu_row_hover_background: Color,
    pub menu_text_color: Color,
    pub menu_font_size: f32,
    pub menu_padding_px: f32,
    pub menu_row_gap_px: f32,
    /// Gap, in screen pixels, between the interact square and the popup's edge.
    pub menu_offset_px: f32,
    pub z_tiles: f32,
    pub z_target: f32,
    pub z_locations: f32,
    pub z_labels: f32,
    /// Party tokens sit just under the player token.
    pub z_party: f32,
    pub z_traveler: f32,
    pub z_interact_widget: f32,
    pub aside_background: Color,
    pub aside_heading_color: Color,
    pub aside_heading_font_size: f32,
    pub aside_row_background: Color,
    /// Row background for the location the traveller is currently standing on.
    pub aside_row_current_background: Color,
    pub aside_row_text_color: Color,
    pub aside_row_font_size: f32,
    pub aside_padding_px: f32,
    pub aside_row_gap_px: f32,
    pub status_text_color: Color,
    pub status_font_size: f32,
    pub clock_text_color: Color,
    pub clock_font_size: f32,
    pub coords_text_color: Color,
    pub coords_font_size: f32,
}

impl Default for WorldMapTheme {
    fn default() -> Self {
        Self {
            grid_color: Color::srgb(0.06, 0.07, 0.09),
            tile_inset_px: 1.0,
            location_radius_px: 9.0,
            location_color: Color::srgb(0.9, 0.82, 0.4),
            secret_location_radius_px: 6.0,
            secret_location_color: Color::srgb(0.55, 0.8, 0.85),
            location_label_color: Color::srgb(0.95, 0.93, 0.86),
            location_label_font_size: 13.0,
            location_label_gap_px: 4.0,
            traveler_radius_px: 6.0,
            traveler_color: Color::srgb(0.95, 0.35, 0.3),
            target_marker_radius_px: 4.0,
            target_marker_color: Color::srgba(0.95, 0.35, 0.3, 0.7),
            party_radius_px: 7.0,
            party_label_color: Color::srgb(0.9, 0.9, 0.82),
            interact_widget_color: Color::srgb(0.4, 0.9, 0.5),
            menu_background: Color::srgba(0.10, 0.11, 0.14, 0.98),
            menu_row_background: Color::srgba(1.0, 1.0, 1.0, 0.05),
            menu_row_hover_background: Color::srgba(0.4, 0.9, 0.5, 0.22),
            menu_text_color: Color::srgb(0.92, 0.9, 0.84),
            menu_font_size: 13.0,
            menu_padding_px: 8.0,
            menu_row_gap_px: 4.0,
            menu_offset_px: 6.0,
            z_tiles: 0.0,
            z_target: 1.0,
            z_locations: 2.0,
            z_labels: 3.0,
            z_party: 3.5,
            z_traveler: 4.0,
            z_interact_widget: 5.0,
            aside_background: Color::srgba(0.10, 0.11, 0.14, 0.96),
            aside_heading_color: Color::srgb(0.92, 0.89, 0.84),
            aside_heading_font_size: 18.0,
            aside_row_background: Color::srgba(1.0, 1.0, 1.0, 0.06),
            aside_row_current_background: Color::srgba(0.9, 0.82, 0.4, 0.25),
            aside_row_text_color: Color::srgb(0.9, 0.88, 0.82),
            aside_row_font_size: 13.0,
            aside_padding_px: 14.0,
            aside_row_gap_px: 6.0,
            status_text_color: Color::srgb(0.75, 0.78, 0.82),
            status_font_size: 13.0,
            clock_text_color: Color::srgb(0.9, 0.88, 0.82),
            clock_font_size: 15.0,
            coords_text_color: Color::srgb(0.55, 0.6, 0.66),
            coords_font_size: 12.0,
        }
    }
}

/// Which side of the screen the aside panel takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AsideSide {
    Left,
    Right,
}

/// Where the aside sits and how wide it is. A `Component` on the map entity,
/// read once at [`spawn_world_map`](super::spawn_world_map).
#[derive(Component, Clone, Copy, Debug)]
pub struct WorldMapLayout {
    pub aside_side: AsideSide,
    pub aside_width_px: f32,
    /// Margin between the clock chip / status line and the screen edge.
    pub screen_margin_px: f32,
}

impl Default for WorldMapLayout {
    fn default() -> Self {
        Self {
            aside_side: AsideSide::Right,
            aside_width_px: 240.0,
            screen_margin_px: 12.0,
        }
    }
}

/// Everything [`spawn_world_map`](super::spawn_world_map) needs to build one
/// map. `Default` points [`source`](Self::source) at
/// `maps/wastes.worldmap.json` — the example's map.
#[derive(Clone, Default)]
pub struct WorldMapSpec {
    pub source: WorldMapSource,
    pub config: WorldMapConfig,
    pub theme: WorldMapTheme,
    pub layout: WorldMapLayout,
}

/// The slice of the viewport the map is actually visible through, as an offset
/// rect around the camera position (world units, `+Y` up): a camera at `C`
/// sees the map through `C + visible_rect(..)`. The aside is opaque and
/// full-height, so it trims one side of `X` and leaves `Y` alone.
///
/// This is what the pan/follow clamp has to reason about — clamping against
/// the raw window would let the map's far edge slide permanently under the
/// panel.
pub fn visible_rect(viewport: Vec2, layout: &WorldMapLayout) -> Rect {
    let half = viewport / 2.0;
    match layout.aside_side {
        AsideSide::Right => Rect {
            min: Vec2::new(-half.x, -half.y),
            max: Vec2::new(half.x - layout.aside_width_px, half.y),
        },
        AsideSide::Left => Rect {
            min: Vec2::new(-half.x + layout.aside_width_px, -half.y),
            max: Vec2::new(half.x, half.y),
        },
    }
}

/// Clamp a wanted camera centre (world units) so the map covers the visible
/// region on each axis. On an axis where the map is smaller than the visible
/// region, the map is centred in that region instead of clamped.
pub fn clamp_camera_center(map_size: Vec2, visible: Rect, wanted: Vec2) -> Vec2 {
    let mut c = wanted;
    for axis in 0..2 {
        let vis_min = visible.min[axis];
        let vis_max = visible.max[axis];
        let vis_extent = vis_max - vis_min;
        let map_half = map_size[axis] / 2.0;
        if map_size[axis] <= vis_extent {
            // Map narrower than the hole: sit the map's centre (world 0) in
            // the middle of the visible region.
            c[axis] = -(vis_min + vis_max) / 2.0;
        } else {
            // c + vis_min >= -map_half  and  c + vis_max <= map_half
            let lo = -map_half - vis_min;
            let hi = map_half - vis_max;
            c[axis] = c[axis].clamp(lo, hi);
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(side: AsideSide) -> WorldMapLayout {
        WorldMapLayout {
            aside_side: side,
            aside_width_px: 200.0,
            ..default()
        }
    }

    #[test]
    fn visible_rect_trims_the_aside_side() {
        let win = Vec2::new(1000.0, 600.0);

        let r = visible_rect(win, &layout(AsideSide::Right));
        assert_eq!(r.min, Vec2::new(-500.0, -300.0));
        assert_eq!(r.max, Vec2::new(300.0, 300.0)); // 500 - 200

        let l = visible_rect(win, &layout(AsideSide::Left));
        assert_eq!(l.min, Vec2::new(-300.0, -300.0)); // -500 + 200
        assert_eq!(l.max, Vec2::new(500.0, 300.0));
    }

    #[test]
    fn clamp_keeps_a_large_map_covering_the_visible_region() {
        let win = Vec2::new(1000.0, 600.0);
        let visible = visible_rect(win, &layout(AsideSide::Right));
        let map = Vec2::new(4000.0, 4000.0);

        // Pushed hard left: the map's left edge (-2000) can't come past the
        // visible region's left edge (camera + -500).
        let clamped = clamp_camera_center(map, visible, Vec2::new(-9999.0, 0.0));
        assert_eq!(clamped.x, -2000.0 - (-500.0)); // = -1500
        assert_eq!(clamped.x + visible.min.x, -2000.0);

        // Pushed hard right: the map's right edge (2000) can't come past the
        // visible region's right edge (camera + 300).
        let clamped = clamp_camera_center(map, visible, Vec2::new(9999.0, 0.0));
        assert_eq!(clamped.x + visible.max.x, 2000.0);
    }

    #[test]
    fn clamp_centres_a_small_map_in_the_visible_region() {
        let win = Vec2::new(1000.0, 600.0);
        let visible = visible_rect(win, &layout(AsideSide::Right));
        let map = Vec2::new(200.0, 200.0);

        let clamped = clamp_camera_center(map, visible, Vec2::new(1234.0, 56.0));
        // Visible region X spans [-500, 300], centre -100 -> camera at +100 so
        // the map centre lands there. Y is untrimmed, so camera stays at 0.
        assert_eq!(clamped, Vec2::new(100.0, 0.0));
    }
}
