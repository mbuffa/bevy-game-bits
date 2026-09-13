//! An optional Day/HH:MM calendar layered on [`WorldMapTime`].
//!
//! [`WorldMapTime`] (core) is the raw per-frame world-time tick — it already
//! only advances while the player travels. This is the "bring your own
//! `GameTime`" seam: add [`WorldMapTimePlugin`] and the module keeps
//! [`WorldMapClock`] and draws a clock chip; leave it out and read
//! [`WorldMapTime`] or [`TravelProgress`](super::TravelProgress) from your own
//! system ordered `.after(WorldMapSet::Travel)`.
//!
//! Slow terrain needs no special handling here — it just means more real
//! seconds spent moving, which this already charges for.

use bevy::prelude::*;

use crate::world_map::travel::WorldMapTime;
use crate::world_map::WorldMapSet;

/// Wall-clock-ish in-game time. `elapsed_minutes` counts in-game minutes since
/// "Day 1, 00:00"; it only advances while a traveller is in motion.
#[derive(Resource, Clone, Debug)]
pub struct WorldMapClock {
    pub elapsed_minutes: f64,
    /// In-game minutes that pass per real second of travel (scaled by
    /// `Time<Virtual>` speed, so a host speed control affects it for free).
    pub minutes_per_second: f32,
}

impl Default for WorldMapClock {
    fn default() -> Self {
        // Start at Day 1, 08:00.
        Self {
            elapsed_minutes: 8.0 * 60.0,
            minutes_per_second: 30.0,
        }
    }
}

impl WorldMapClock {
    const DAY_MINUTES: f64 = 24.0 * 60.0;

    /// 1-based day number.
    pub fn day(&self) -> u32 {
        (self.elapsed_minutes / Self::DAY_MINUTES).floor() as u32 + 1
    }

    /// `(hour, minute)` within the current day, both 0-based.
    pub fn hour_minute(&self) -> (u32, u32) {
        let m = self.elapsed_minutes.rem_euclid(Self::DAY_MINUTES);
        ((m / 60.0) as u32, (m % 60.0) as u32)
    }

    /// `"Day 3 — 14:30"`, for the clock chip.
    pub fn label(&self) -> String {
        let (h, m) = self.hour_minute();
        format!("Day {} \u{2014} {:02}:{:02}", self.day(), h, m)
    }
}

/// Advances [`WorldMapClock`] by this frame's [`WorldMapTime`] — so it moves
/// exactly when the world does (the player travelling, or a
/// [`WorldClockHold`](super::WorldClockHold)), and not while a caravan crosses
/// the map on its own. With several maps the fastest-advancing one drives the
/// one shared calendar.
pub fn advance_clock(mut clock: ResMut<WorldMapClock>, times: Query<&WorldMapTime>) {
    let dt = times.iter().map(|t| t.delta_secs).fold(0.0_f32, f32::max);
    if dt <= 0.0 {
        return;
    }
    clock.elapsed_minutes += (dt * clock.minutes_per_second) as f64;
}

/// Keeps [`WorldMapClock`] and draws the clock chip. Independent of
/// [`WorldMapPlugin`](super::WorldMapPlugin) — omit it entirely for a host
/// that owns its own time.
#[derive(Default)]
pub struct WorldMapTimePlugin;

impl Plugin for WorldMapTimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldMapClock>()
            .add_systems(Update, advance_clock.after(WorldMapSet::Travel));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_formats_day_and_time() {
        let clock = WorldMapClock {
            elapsed_minutes: 8.0 * 60.0 + 5.0,
            minutes_per_second: 1.0,
        };
        assert_eq!(clock.label(), "Day 1 \u{2014} 08:05");
    }

    #[test]
    fn day_rolls_over_at_midnight() {
        let clock = WorldMapClock {
            elapsed_minutes: 24.0 * 60.0 + 30.0,
            minutes_per_second: 1.0,
        };
        assert_eq!(clock.day(), 2);
        assert_eq!(clock.hour_minute(), (0, 30));
        assert_eq!(clock.label(), "Day 2 \u{2014} 00:30");
    }
}
