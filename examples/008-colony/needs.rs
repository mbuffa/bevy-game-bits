//! Pawn needs: 0-100 drives shown as gauges in the selection HUD. `Sleep` is
//! the first one; more (Food, Recreation, ...) can join `Needs` and
//! `Needs::gauges` later without touching the panel itself, which just
//! renders whatever that iterator yields.
//!
//! `decay_needs` drains `Sleep` while a pawn is awake and refills it while
//! actually *resting* — `Objective::Sleep` and stationary — at one of two
//! rates: the ground fallback, or faster while `director::InBed` marks it as
//! resting in a built bed. Still walking to a bed (tired but moving) keeps
//! draining at the normal awake rate: only rest recovers, so a distant bed is
//! a real cost. `wants_sleep` is the pure hysteresis
//! `director::choose_objectives` uses to decide when a pawn falls asleep and
//! when it wakes back up.

use bevy::prelude::*;

use crate::config::*;
use crate::director::{InBed, Objective};
use crate::movement::{MoveOrder, Path};
use crate::units::{ManualMode, Pawn};

/// A pawn's 0-100 drives. `sleep` starts high and drains while awake;
/// clamped to `[0, 100]` every tick.
#[derive(Component)]
pub struct Needs {
    pub sleep: f32,
}

impl Default for Needs {
    fn default() -> Self {
        Self { sleep: SLEEP_START }
    }
}

impl Needs {
    /// `(label, value)` for every need, in the order panels should show
    /// them. The only place that needs editing to add a new gauge's data:
    /// the inline Sleep gauge in the info panel is still hand-spawned (see
    /// `ui::sync_needs_panel`), but the Needs tab (`ui::sync_needs_tab`)
    /// walks this iterator, so a new need shows up there for free.
    pub fn gauges(&self) -> impl Iterator<Item = (&'static str, f32)> {
        [("Sleep", self.sleep)].into_iter()
    }
}

/// Hysteresis for the sleep/wake decision, so a pawn sitting right at a
/// threshold doesn't flicker in and out of `Objective::Sleep` every frame:
/// once asleep it keeps sleeping until fully rested; once awake it doesn't
/// fall asleep until properly tired. `was_sleeping` is the pawn's *current*
/// `Objective == Objective::Sleep`, so the caller (`director::choose_objectives`)
/// just feeds its own last decision back in.
pub fn wants_sleep(sleep: f32, was_sleeping: bool) -> bool {
    if was_sleeping {
        sleep < SLEEP_RESTED_THRESHOLD
    } else {
        sleep <= SLEEP_TIRED_THRESHOLD
    }
}

/// Drain `Sleep` at a flat rate while a pawn is awake (or still walking
/// somewhere, tired or not); refill it while actually resting —
/// `Objective::Sleep`, stationary, not drafted — at the bed rate if `InBed`
/// marks it as resting in a built bed, the slower ground-fallback rate
/// otherwise. Drafted pawns never recover this way even if their objective
/// happens to read `Sleep`: the player is controlling them, not the
/// director. Gated on `SimState::Running` like the rest of the growth/weather
/// block, so pausing freezes it along with everything else.
pub fn decay_needs(
    time: Res<Time>,
    mut pawns: Query<
        (
            &mut Needs,
            &Objective,
            Has<ManualMode>,
            Has<InBed>,
            Has<Path>,
            Has<MoveOrder>,
        ),
        With<Pawn>,
    >,
) {
    let dt = time.delta_secs();
    for (mut needs, objective, manual, in_bed, has_path, has_order) in &mut pawns {
        let moving = has_path || has_order;
        let resting = *objective == Objective::Sleep && !manual && !moving;
        let delta = if resting && in_bed {
            SLEEP_RECOVER_BED_PER_SEC
        } else if resting {
            SLEEP_RECOVER_GROUND_PER_SEC
        } else {
            -SLEEP_DECAY_PER_SEC
        };
        needs.sleep = (needs.sleep + delta * dt).clamp(0.0, 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awake_pawn_falls_asleep_only_once_tired() {
        assert!(!wants_sleep(SLEEP_TIRED_THRESHOLD + 1.0, false));
        assert!(wants_sleep(SLEEP_TIRED_THRESHOLD, false));
        assert!(wants_sleep(SLEEP_TIRED_THRESHOLD - 1.0, false));
    }

    #[test]
    fn sleeping_pawn_wakes_only_once_rested() {
        assert!(wants_sleep(SLEEP_RESTED_THRESHOLD - 1.0, true));
        assert!(!wants_sleep(SLEEP_RESTED_THRESHOLD, true));
        assert!(!wants_sleep(SLEEP_RESTED_THRESHOLD + 1.0, true));
    }

    #[test]
    fn hysteresis_holds_the_middle_band_stable() {
        // Between the two thresholds, whichever state the pawn is already in
        // is the state it stays in — this is what stops a value hovering
        // near a single threshold from flip-flopping every frame.
        let mid = (SLEEP_TIRED_THRESHOLD + SLEEP_RESTED_THRESHOLD) / 2.0;
        assert!(!wants_sleep(mid, false));
        assert!(wants_sleep(mid, true));
    }
}
