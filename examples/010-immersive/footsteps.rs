//! Movement audio: a step per stride while walking, a heavier thump on
//! landing, a quieter/shorter cadence while crouched. `jsfxr` wavs from
//! `config`, pitched and levelled per cue — `surface_of` picks a hard set or a
//! wood set (`Surface::Wood`, a wooden crate underfoot); the thump always uses
//! the hard set.
//!
//! Climbing is **silent** — the rung clank is parked (2026-09-10): it reused
//! the hard footstep wav, which was too loud and read as walking on the
//! ladder. The path accumulator stays live so `advance_footsteps` can restore
//! it by uncommenting once a real climbing set exists.
//!
//! # Why `Transform` delta, not `LinearVelocity`
//!
//! `ladder::climb` sets `LinearVelocity` to zero **every** fixed step and then
//! writes an absolute `Transform` — a velocity-driven system would be silent
//! on a ladder and through the whole crest walk. Measuring the frame's real
//! `Transform` delta covers walking, climbing and cresting with one
//! accumulator. The cost is teleports (ladder mount, bottom step-off, a
//! devtool warp): a single-tick delta over `FOOTSTEP_MAX_STEP_M` is treated as
//! a teleport and adds nothing.
//!
//! # The shared phase
//!
//! [`Footsteps::phase`] is the fractional stride position; a step fires each
//! time it wraps past 1. `viewmodel::bob_viewmodel` reads the same field, so
//! the bob dips exactly on the footfall.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_game_bits::audio::PlaySfx;

use crate::classes::{PropCrate, PropWoodCrate};
use crate::config;
use crate::ladder::Climbing;
use crate::noclip::Noclip;

/// Stride bookkeeping for the player. One component, inserted in
/// `player::spawn_player`.
#[derive(Component, Default)]
pub struct Footsteps {
    /// Fractional stride position in `[0, 1)`. A footfall is a wrap past 1.
    pub phase: f32,
    /// Total footfalls taken — parity picks the wav, and it seeds the jitter
    /// so a run is reproducible in a log.
    pub steps: u32,
    /// Last frame's world position; `None` until the first tick.
    last_pos: Option<Vec3>,
    /// Downward speed (m/s) sampled *before* ahoy moved — it zeroes
    /// `velocity.y` the instant it grounds, so the landing thump has to read
    /// this, not the post-move velocity.
    fall_speed: f32,
    /// Was the player grounded last tick (for the landing edge).
    was_grounded: bool,
    /// Metres of climb path since the last rung clank.
    rung: f32,
}

/// Clip handles, loaded once at `Startup`. The landing thump always uses the
/// hard-surface set (`hard[0]`); so did the rung clank before it was parked.
#[derive(Resource)]
pub struct FootstepClips {
    hard: [Handle<AudioSource>; 2],
    wood: [Handle<AudioSource>; 2],
}

impl FootstepClips {
    /// The step clip for a surface and a step-count parity.
    fn step(&self, surface: Surface, step: u32) -> Handle<AudioSource> {
        let set = match surface {
            Surface::Wood => &self.wood,
            // No dedicated metal set — a metal crate reads as a hard surface.
            Surface::Concrete | Surface::Metal => &self.hard,
        };
        set[(step % 2) as usize].clone()
    }

    fn land(&self) -> Handle<AudioSource> {
        self.hard[0].clone()
    }

    #[allow(dead_code)] // parked with the climbing cue; see `advance_footsteps`
    fn rung(&self) -> Handle<AudioSource> {
        self.hard[0].clone()
    }
}

/// What the player is standing on. `surface_of` resolves it; `FootstepClips`
/// maps it to a sound set — `Metal` currently shares the hard set (no metal
/// assets), `Wood` has its own pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    #[default]
    Concrete,
    Metal,
    Wood,
}

pub fn load_footstep_clips(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(FootstepClips {
        hard: [
            asset_server.load(config::FOOTSTEP_WAV_A),
            asset_server.load(config::FOOTSTEP_WAV_B),
        ],
        wood: [
            asset_server.load(config::FOOTSTEP_WAV_A_WOOD),
            asset_server.load(config::FOOTSTEP_WAV_B_WOOD),
        ],
    });
}

/// `.before(AhoySystems::MoveCharacters)` — capture the fall speed before ahoy
/// grounds the body and zeroes `velocity.y`.
pub fn stash_fall_speed(
    mut players: Query<(&LinearVelocity, &mut Footsteps), With<CharacterController>>,
) {
    for (velocity, mut steps) in &mut players {
        steps.fall_speed = (-velocity.0.y).max(0.0);
    }
}

/// `.after(AhoySystems::MoveCharacters).after(ladder::climb)` — the slot
/// `ladder::climb` occupies, and after it so its absolute position write shows.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn advance_footsteps(
    time: Res<Time>,
    clips: Res<FootstepClips>,
    windows: Query<&bevy_game_bits::inventory::prelude::InventoryWindow>,
    mut players: Query<
        (
            &Transform,
            &CharacterControllerState,
            &mut Footsteps,
            Has<Climbing>,
            &Noclip,
        ),
        With<CharacterController>,
    >,
    crates: Query<(), With<PropCrate>>,
    wood: Query<(), With<PropWoodCrate>>,
    parents: Query<&ChildOf>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    let dt = time.delta_secs().max(1.0e-6);
    let frozen = windows.iter().any(|w| w.open);

    for (transform, state, mut steps, climbing, noclip) in &mut players {
        let pos = transform.translation;
        let Some(last) = steps.last_pos.replace(pos) else {
            continue; // first tick: just seed last_pos
        };
        let delta = pos - last;
        let grounded = state.grounded.is_some();

        // A teleport (or a frozen frame) resets the edge state and contributes
        // no distance.
        if frozen || delta.length() > config::FOOTSTEP_MAX_STEP_M {
            steps.was_grounded = grounded;
            continue;
        }

        // Flying: no walking, no landing, no sound at all. Unlike the
        // teleport guard above, a sane fly speed's per-step delta stays well
        // under `FOOTSTEP_MAX_STEP_M`, and `state.grounded` is computed
        // independently by ahoy's own (discarded) step, so it can still read
        // "grounded" while skimming the real floor — this has to be checked
        // explicitly, the same way `climbing` already is below.
        if noclip.active {
            steps.was_grounded = grounded;
            continue;
        }

        // Landing: a real fall meeting the ground, not a step off a ladder.
        if grounded && !steps.was_grounded && !climbing {
            if let Some(gain) = landing_gain(steps.fall_speed) {
                let volume = config::FOOTSTEP_VOLUME
                    + (config::FOOTSTEP_LAND_VOLUME_MAX - config::FOOTSTEP_VOLUME) * gain;
                sfx.write(
                    PlaySfx::new(clips.land())
                        .with_volume(volume)
                        .with_speed(config::FOOTSTEP_LAND_PITCH),
                );
                steps.phase = 0.0; // the landing *is* the next footfall
            }
        }
        steps.was_grounded = grounded;

        // Climbing: no walking, no landing — and, since 2026-09-10, no sound at
        // all. The clank is PARKED: it reused the hard-surface footstep wav
        // (`clips.rung()` -> `hard[0]`), which was too loud and read as a
        // footstep rather than a rung. The path accumulator below stays live,
        // so restoring the cue is uncommenting this block (and dropping the two
        // `#[allow(dead_code)]`s it feeds — `FootstepClips::rung` and
        // `config::FOOTSTEP_RUNG_VOLUME`) once a real climbing set exists.
        if climbing {
            steps.rung += delta.length();
            while steps.rung >= config::LADDER_RUNG_M {
                steps.rung -= config::LADDER_RUNG_M;
                // steps.steps = steps.steps.wrapping_add(1);
                // let (speed, vol) = jitter(steps.steps);
                // sfx.write(
                //     PlaySfx::new(clips.rung())
                //         .with_volume(config::FOOTSTEP_RUNG_VOLUME * vol)
                //         .with_speed(speed),
                // );
            }
            continue;
        }
        steps.rung = 0.0;

        if !grounded {
            continue;
        }

        // Walking: accumulate horizontal distance, fire on each stride wrap.
        let horizontal = Vec2::new(delta.x, delta.z).length();
        if horizontal / dt < config::FOOTSTEP_MIN_SPEED {
            continue;
        }
        let crouching = state.crouching;
        let stride = config::FOOTSTEP_STRIDE_M
            * if crouching {
                config::FOOTSTEP_CROUCH_STRIDE_SCALE
            } else {
                1.0
            };
        let (phase, fired) = advance(steps.phase, horizontal, stride);
        steps.phase = phase;
        if fired == 0 {
            continue;
        }

        let surface = surface_of(state.grounded.map(|g| g.entity), &crates, &wood, &parents);
        let base = if crouching {
            config::FOOTSTEP_CROUCH_VOLUME
        } else {
            config::FOOTSTEP_VOLUME
        };
        for _ in 0..fired {
            steps.steps = steps.steps.wrapping_add(1);
            let (speed, vol) = jitter(steps.steps);
            sfx.write(
                PlaySfx::new(clips.step(surface, steps.steps))
                    .with_volume(base * vol)
                    .with_speed(speed),
            );
        }
    }
}

/// Advance the fractional stride phase by `distance` metres over `stride`
/// metres/step. Returns the new phase in `[0, 1)` and how many footfalls the
/// move crossed. `distance` is bounded by the teleport guard, so this never
/// runs away.
pub fn advance(phase: f32, distance: f32, stride: f32) -> (f32, u32) {
    let advanced = phase + distance / stride.max(1.0e-3);
    (advanced.fract(), advanced.floor().max(0.0) as u32)
}

/// `None` below `FOOTSTEP_LAND_MIN_SPEED`; ramps 0→1 up to
/// `FOOTSTEP_LAND_MAX_SPEED`; saturates there.
pub fn landing_gain(fall_speed: f32) -> Option<f32> {
    if fall_speed < config::FOOTSTEP_LAND_MIN_SPEED {
        return None;
    }
    let span = config::FOOTSTEP_LAND_MAX_SPEED - config::FOOTSTEP_LAND_MIN_SPEED;
    Some(((fall_speed - config::FOOTSTEP_LAND_MIN_SPEED) / span).clamp(0.0, 1.0))
}

/// Deterministic per-step `(speed, volume_scale)` jitter — an xorshift32 (the
/// `008-colony` wander precedent; the repo has no `rand` dep) seeded from the
/// step count, so a log replays. Bounded by the two `*_JITTER` knobs.
pub fn jitter(seed: u32) -> (f32, f32) {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    let a = (x & 0xffff) as f32 / 65535.0;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    let b = (x & 0xffff) as f32 / 65535.0;
    let speed = 1.0 + (a - 0.5) * 2.0 * config::FOOTSTEP_PITCH_JITTER;
    let volume = 1.0 + (b - 0.5) * 2.0 * config::FOOTSTEP_VOLUME_JITTER;
    (speed, volume)
}

/// Resolve the ground entity into a [`Surface`]. `PropCrate` / `PropWoodCrate`
/// carry their collider on the same entity as the marker; the `ChildOf` walk
/// covers a nested collider (door leaf, ladder visual). Everything else is
/// `Concrete` — the concrete/iron split needs the `BrushesAsset` plane scan
/// (see SPEC.md Phase 17) and waits for a second sound set to justify it.
pub fn surface_of(
    ground: Option<Entity>,
    crates: &Query<(), With<PropCrate>>,
    wood: &Query<(), With<PropWoodCrate>>,
    parents: &Query<&ChildOf>,
) -> Surface {
    let Some(mut e) = ground else {
        return Surface::Concrete;
    };
    loop {
        if wood.contains(e) {
            return Surface::Wood;
        }
        if crates.contains(e) {
            return Surface::Metal;
        }
        match parents.get(e) {
            Ok(p) => e = p.parent(),
            Err(_) => return Surface::Concrete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standstill_takes_no_steps() {
        let (phase, fired) = advance(0.4, 0.0, 1.9);
        assert_eq!(fired, 0);
        assert!((phase - 0.4).abs() < 1.0e-6);
    }

    #[test]
    fn one_stride_is_one_step() {
        let (_, fired) = advance(0.0, 1.9, 1.9);
        assert_eq!(fired, 1);
    }

    #[test]
    fn a_long_move_crosses_several() {
        let (phase, fired) = advance(0.5, 1.9 * 2.0, 1.9);
        assert_eq!(fired, 2);
        assert!((phase - 0.5).abs() < 1.0e-5);
    }

    #[test]
    fn landing_gain_floor_and_saturation() {
        assert_eq!(landing_gain(config::FOOTSTEP_LAND_MIN_SPEED - 0.1), None);
        assert_eq!(landing_gain(config::FOOTSTEP_LAND_MIN_SPEED), Some(0.0));
        assert_eq!(landing_gain(config::FOOTSTEP_LAND_MAX_SPEED), Some(1.0));
        assert_eq!(landing_gain(100.0), Some(1.0));
    }

    #[test]
    fn landing_gain_is_monotone() {
        let mut last = -1.0;
        let mut s = config::FOOTSTEP_LAND_MIN_SPEED;
        while s <= config::FOOTSTEP_LAND_MAX_SPEED {
            let g = landing_gain(s).unwrap();
            assert!(g >= last);
            last = g;
            s += 0.5;
        }
    }

    #[test]
    fn jitter_is_deterministic_and_bounded() {
        for seed in 1..500 {
            let (s1, v1) = jitter(seed);
            let (s2, v2) = jitter(seed);
            assert_eq!((s1, v1), (s2, v2));
            assert!((s1 - 1.0).abs() <= config::FOOTSTEP_PITCH_JITTER + 1.0e-6);
            assert!((v1 - 1.0).abs() <= config::FOOTSTEP_VOLUME_JITTER + 1.0e-6);
        }
    }

    #[test]
    fn jitter_actually_varies() {
        let first = jitter(1);
        assert!((1..50).any(|s| jitter(s) != first));
    }
}
