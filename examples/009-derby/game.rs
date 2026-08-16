//! The match: turns the sandbox into a game with a start, an end, and a
//! winner. Four states —
//!
//! - `Loading` — waiting on the async car model (`model.rs`); no cars exist
//!   yet.
//! - `Countdown` — cars are freshly spawned and sitting on the grid, but
//!   `DriveInput` is never written (`read_player_input`/`ai_drivers` are
//!   gated to `Fighting`), so nobody can move. `vehicle_controller` still
//!   runs, so suspension settles the cars onto the floor before "GO!".
//! - `Fighting` — the derby itself. `check_wrecks` watches every car's
//!   `Damage::condition()` each physics tick; crossing `WRECK_CONDITION`
//!   knocks it out. Last one rolling ends the match.
//! - `Over` — results on screen; `R` starts a new `Countdown`.
//!
//! `start_match` (`OnEnter(Countdown)`) is the one place that resets the
//! world: it clears the previous match's cars, torn-off debris and
//! power-ups, then calls `vehicle::spawn_vehicle` again — the same path
//! that builds the *first* match, so "new match" and "first match" are one
//! code path, not two.

use bevy::prelude::*;

use crate::ai::AiDriver;
use crate::config::*;
use crate::damage::{Damage, MatchDebris};
use crate::effects::EngineEffects;
use crate::model::CarRigs;
use crate::powerups::{PowerUp, ShardLifetime};
use bevy_game_bits::vehicle::{DriveInput, KeyboardDriver, Vehicle, VehicleTuning};
use crate::vehicle::{self, CarAssets, Player};

#[derive(States, Default, Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum MatchState {
    #[default]
    Loading,
    Countdown,
    Fighting,
    Over,
}

/// Name and paint of whoever drives this chassis — on the player and every
/// AI car alike, so nameplates and the results panel never need to branch
/// on `vehicle::Player`. Set once at spawn from `config::DRIVERS`.
#[derive(Component, Clone, Copy)]
pub struct Driver {
    pub name: &'static str,
    pub color: Color,
}

/// Knocked out: `Damage::condition()` crossed `WRECK_CONDITION`. The car
/// stays in the arena as an inert obstacle — its `DriveInput` is zeroed and
/// stays that way (`read_player_input`/`ai_drivers` skip it), but it keeps
/// its `RigidBody` and suspension, so it can still be shoved around.
///
/// `place` is the final standing (1 = last car rolling, 4 = first to go
/// down) and is assigned once, at the moment of the KO, by counting down
/// from how many cars were still alive (`check_wrecks`) — cheaper than
/// re-deriving standings from `at` every time the results panel draws.
#[derive(Component)]
pub struct Wrecked {
    pub place: usize,
    pub at: f32,
}

/// Which chassis the chase camera follows. Starts on the player; if the
/// player wrecks mid-match, `hand_off_camera` moves it to a survivor so a
/// dead player still gets to watch the rest of the fight.
#[derive(Component)]
pub struct CameraTarget;

/// Seconds of `Fighting` elapsed this match — stamped onto each `Wrecked`
/// as its time of death, and read back for the results panel.
#[derive(Resource, Default)]
pub struct MatchClock(pub f32);

/// Counts down `Countdown`. Reset to `COUNTDOWN_SECS` every time
/// `start_match` runs.
#[derive(Resource)]
pub struct Countdown(pub Timer);

/// `Loading -> Countdown`, the moment every class's car model lands. Gated
/// externally (`main.rs`) on `resource_added::<model::CarRigs>`, which fires
/// exactly once per process — this system itself doesn't need to know
/// about `CarRigs` at all.
pub fn enter_countdown_when_loaded(mut next_state: ResMut<NextState<MatchState>>) {
    next_state.set(MatchState::Countdown);
}

/// Which class the player currently drives — seeded from `DRIVERS[0]`'s
/// class, changed only by `cycle_player_class` (`C` on the starting grid).
/// The seam the eventual pre-match car-selection screen (SPEC.md Phase 19)
/// replaces: for now this *is* the selection UI, one key at a time.
#[derive(Resource)]
pub struct PlayerClass(pub CarClass);

impl Default for PlayerClass {
    fn default() -> Self {
        Self(DRIVERS[0].2)
    }
}

/// `C` during `Countdown`: advances `PlayerClass` and respawns just the
/// player's grid slot in the new class — a targeted respawn rather than a
/// full re-entry into `Countdown`, so the AI cars and the settled arena stay
/// exactly where `start_match` put them. Gated on `CarAssets` existing (it's
/// inserted at the tail of `vehicle::spawn_vehicle`, chained right after
/// `start_match` on `OnEnter(Countdown)`), so this can never fire before the
/// very first grid has actually been built.
#[allow(clippy::too_many_arguments)]
pub fn cycle_player_class(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut player_class: ResMut<PlayerClass>,
    mut countdown: ResMut<Countdown>,
    assets: Res<CarAssets>,
    rigs: Res<CarRigs>,
    tuning: Res<VehicleTuning>,
    effects: Res<EngineEffects>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player: Query<Entity, With<Player>>,
) {
    if !keys.just_pressed(KeyCode::KeyC) {
        return;
    }
    player_class.0 = player_class.0.next();
    if let Ok(entity) = player.single() {
        commands.entity(entity).despawn();
    }
    vehicle::spawn_grid_car(&mut commands, &assets, &rigs, &tuning, &mut materials, &effects, 0, player_class.0);
    // Time to actually look at the new car before the fight starts.
    countdown.0 = Timer::from_seconds(COUNTDOWN_SECS, TimerMode::Once);
    info!("player class -> {}", player_class.0.label());
}

/// Clears out the previous match — every chassis, every piece of match
/// debris (torn-off parts and crate shards, but *not* the permanent arena
/// balls — see `damage::MatchDebris`), every uncollected power-up — and
/// resets the clock and countdown timer. `vehicle::spawn_vehicle` runs
/// right after this (chained in `main.rs`) to rebuild the grid.
pub fn start_match(
    mut commands: Commands,
    mut clock: ResMut<MatchClock>,
    mut countdown: ResMut<Countdown>,
    vehicles: Query<Entity, With<Vehicle>>,
    debris: Query<Entity, Or<(With<MatchDebris>, With<ShardLifetime>)>>,
    pickups: Query<Entity, With<PowerUp>>,
) {
    for entity in &vehicles {
        commands.entity(entity).despawn();
    }
    for entity in &debris {
        commands.entity(entity).despawn();
    }
    for entity in &pickups {
        commands.entity(entity).despawn();
    }
    clock.0 = 0.0;
    countdown.0 = Timer::from_seconds(COUNTDOWN_SECS, TimerMode::Once);
}

/// `Countdown -> Fighting` once the timer runs out.
pub fn tick_countdown(time: Res<Time>, mut countdown: ResMut<Countdown>, mut next_state: ResMut<NextState<MatchState>>) {
    if countdown.0.tick(time.delta()).just_finished() {
        next_state.set(MatchState::Fighting);
    }
}

pub fn tick_match_clock(time: Res<Time>, mut clock: ResMut<MatchClock>) {
    clock.0 += time.delta_secs();
}

/// Watches every car's overall health each physics tick; anyone who drops
/// to `WRECK_CONDITION` or below is knocked out this tick. Runs after
/// `damage::apply_impact_damage` in the `FixedUpdate` chain, so it sees
/// this tick's damage. `place` counts down as the field thins — the first
/// car to go this tick takes the worst remaining place, and so on, so two
/// simultaneous KOs still resolve to distinct standings.
pub fn check_wrecks(
    mut commands: Commands,
    clock: Res<MatchClock>,
    mut next_state: ResMut<NextState<MatchState>>,
    cars: Query<(Entity, &Damage, Has<Wrecked>), With<Vehicle>>,
) {
    let mut alive = cars.iter().filter(|(_, _, wrecked)| !wrecked).count();
    for (entity, damage, wrecked) in &cars {
        if wrecked || damage.condition() > WRECK_CONDITION {
            continue;
        }
        // Zeroing `DriveInput` stops the car this tick; dropping the two
        // driver components stops anything writing it again. `KeyboardDriver`
        // matters even though `drive_from_keyboard` is already gated on
        // `Fighting`: a wreck doesn't end the match (the survivors fight on),
        // so without this the player would keep steering their own hulk.
        commands
            .entity(entity)
            .insert((Wrecked { place: alive, at: clock.0 }, DriveInput::default()))
            .remove::<AiDriver>()
            .remove::<KeyboardDriver>();
        alive -= 1;
        info!("KO: car {entity} wrecked, place {}", alive + 1);
    }
    if alive <= 1 {
        next_state.set(MatchState::Over);
    }
}

/// If the car the camera is watching just got wrecked, hand it off to
/// anyone still standing — otherwise a dead player would spend the rest of
/// the match staring at their own hulk.
pub fn hand_off_camera(
    mut commands: Commands,
    dead_target: Query<Entity, (With<CameraTarget>, With<Wrecked>)>,
    survivors: Query<Entity, (With<Vehicle>, Without<Wrecked>, Without<CameraTarget>)>,
) {
    let Ok(dead_target) = dead_target.single() else {
        return;
    };
    commands.entity(dead_target).remove::<CameraTarget>();
    if let Some(survivor) = survivors.iter().next() {
        commands.entity(survivor).insert(CameraTarget);
    }
}

/// `Over -> Countdown`: `R` starts a new match.
pub fn restart_match(keys: Res<ButtonInput<KeyCode>>, mut next_state: ResMut<NextState<MatchState>>) {
    if keys.just_pressed(KeyCode::KeyR) {
        next_state.set(MatchState::Countdown);
    }
}

/// Dev/verification tool: `K` instantly wrecks one live AI car, so the
/// end-of-match flow (results panel, camera hand-off, restart) can be
/// exercised in seconds instead of grinding out a whole derby.
pub fn debug_wreck_car(keys: Res<ButtonInput<KeyCode>>, mut cars: Query<&mut Damage, (With<AiDriver>, Without<Wrecked>)>) {
    if !keys.just_pressed(KeyCode::KeyK) {
        return;
    }
    if let Some(mut damage) = cars.iter_mut().next() {
        for part in damage.parts_mut() {
            *part = 0.0;
        }
    }
}
