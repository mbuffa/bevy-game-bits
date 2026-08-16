//! Derby prototype: raycast-suspension cars in a walled circular arena,
//! physics via Avian, destructible parts, AI rivals, power-ups and a match
//! structure.
//!
//! The driving model itself is `bevy_game_bits::vehicle` — this example is
//! its first consumer and where it was extracted from. `VehiclePlugin` owns
//! the suspension/tire/steering physics, `VehicleImpactPlugin` turns solver
//! impulses into located per-car Δv (which `damage.rs` then spends on *this*
//! game's parts), and `SkidMarkPlugin` paints the floor. Everything else in
//! this directory is derby.
//!
//! Controls: W/S accelerate/brake, A/D steer, Space handbrake, R reset (or,
//! once a match is over, start a new one), C cycles the player's car class on
//! the grid, K debug-wrecks one AI car (see `game.rs`).

mod ai;
mod arena;
mod camera;
mod config;
mod damage;
mod effects;
mod export;
mod game;
mod juice;
mod model;
mod obstacles;
mod powerups;
mod ui;
mod vehicle;

use std::path::Path;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_hanabi::HanabiPlugin;

use bevy_game_bits::vehicle::{
    drive_from_keyboard, SkidMarkPlugin, VehicleImpactPlugin, VehiclePlugin, VehicleSet,
};

/// `cargo run --example 009-derby -- export-model <truck|buggy> [path]
/// [--force]` regenerates `assets/models/derby-<class>.glb` from that
/// class's `CarSpec` (`config.rs`), ahead of the artist taking it over — see
/// `export.rs`. No `App` involved, so this returns before one is ever built.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("export-model") {
        let class = match args.get(2).map(String::as_str) {
            Some("truck") => config::CarClass::Truck,
            Some("buggy") => config::CarClass::Buggy,
            other => {
                eprintln!(
                    "export-model needs a class: `export-model <truck|buggy> [path] [--force]` (got {:?})",
                    other.unwrap_or("nothing")
                );
                std::process::exit(1);
            }
        };
        let path = args
            .get(3)
            .filter(|arg| !arg.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| format!("assets/{}", class.asset_path()));
        let force = args.iter().any(|arg| arg == "--force");
        if let Err(err) = export::export_car_model(class.spec(), Path::new(&path), force) {
            eprintln!("export-model failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Derby".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(PhysicsPlugins::default())
        .add_plugins(HanabiPlugin)
        // The extracted driving model. Every tuning default in these three is
        // exactly the number `config.rs` used to hold, so the feel is
        // unchanged — the plugins are added bare rather than with overrides
        // precisely so that stays true and visible.
        .add_plugins(VehiclePlugin::default())
        .add_plugins(VehicleImpactPlugin::default())
        .add_plugins(SkidMarkPlugin::default())
        .add_message::<damage::ImpactBurst>()
        .add_message::<powerups::CrateBroken>()
        .init_resource::<powerups::DropRng>()
        .init_state::<game::MatchState>()
        .init_resource::<game::MatchClock>()
        .insert_resource(game::Countdown(Timer::from_seconds(config::COUNTDOWN_SECS, TimerMode::Once)))
        .init_resource::<game::PlayerClass>()
        .init_resource::<juice::CameraShake>()
        .init_resource::<juice::NearMissWatch>()
        .init_resource::<juice::SlowMotion>()
        .add_systems(
            Startup,
            (
                arena::setup_arena,
                effects::setup_effects,
                // The car model loads asynchronously; `model::build_car_rig`
                // (Update) polls for it and `vehicle::spawn_vehicle` follows
                // once it lands — Startup finishes well before then, so no
                // explicit ordering against `effects::setup_effects` is
                // needed the way the old same-frame chain required.
                model::start_loading_car_model,
                obstacles::setup_obstacles,
                powerups::setup_powerups,
                camera::setup_camera,
                ui::setup_hud,
            ),
        )
        // Bevy's built-in `FixedUpdate` runs before Avian's physics step
        // (which lives in `FixedPostUpdate`), so forces applied here are the
        // ones integrated this tick. The library's `VehicleSet::Input ->
        // Control -> Impact` chain is configured by its own plugins; what
        // follows is derby slotting into it.
        //
        // Note what is *not* gated on `MatchState`: `vehicle_controller`
        // (registered by `VehiclePlugin` in `VehicleSet::Control`),
        // `detect_vehicle_impacts` and `break_crates`. Cars need their
        // suspension every tick — so they settle onto the grid during
        // `Countdown` instead of floating — even while nobody's allowed to
        // drive. What's gated is the *input* and the consequences: no
        // driving, no AI, no new damage, no new KOs outside `Fighting`.
        .add_systems(
            FixedUpdate,
            (
                drive_from_keyboard,
                ai::ai_drivers,
            )
                .chain()
                .in_set(VehicleSet::Input)
                .run_if(in_state(game::MatchState::Fighting)),
        )
        // A damaged engine limps: this mirrors `Damage::engine_power()` into
        // the `DrivePower` the controller reads, which is the whole of what
        // the driving model knows about damage.
        .add_systems(
            FixedUpdate,
            damage::sync_drive_power
                .after(VehicleSet::Input)
                .before(VehicleSet::Control),
        )
        .add_systems(
            FixedUpdate,
            (
                damage::apply_impact_damage.run_if(in_state(game::MatchState::Fighting)),
                juice::watch_near_misses.run_if(in_state(game::MatchState::Fighting)),
                game::check_wrecks.run_if(in_state(game::MatchState::Fighting)),
                powerups::break_crates,
            )
                .chain()
                .after(VehicleSet::Impact),
        )
        // Forces normal speed the instant a match stops `Fighting` — a KO
        // that fired slow-motion must never carry it into the results
        // screen (or a fresh countdown, on a fast-enough `R`).
        .add_systems(OnExit(game::MatchState::Fighting), juice::reset_slow_motion)
        // `Countdown` builds the grid (and every rematch after it) by
        // chaining straight into `vehicle::spawn_vehicle` — the one path
        // that both the first match and every `R`-triggered rematch share.
        .add_systems(
            OnEnter(game::MatchState::Countdown),
            (game::start_match, vehicle::spawn_vehicle).chain(),
        )
        .add_systems(OnEnter(game::MatchState::Countdown), ui::show_countdown_text)
        .add_systems(OnExit(game::MatchState::Countdown), ui::hide_countdown_text)
        .add_systems(OnEnter(game::MatchState::Over), ui::show_results)
        .add_systems(OnExit(game::MatchState::Over), ui::hide_results)
        // Nested into nested sub-tuples (system-tuple `IntoSystemConfigs`
        // impls stop past a fixed arity — see `008-colony`'s Update block
        // for the same split) rather than one flat list.
        .add_systems(
            Update,
            (
                (
                    // Polls the async car-model load; once it lands, moves
                    // `Loading -> Countdown` the same frame (`resource_added`
                    // fires the tick `build_car_rig`'s `insert_resource` is
                    // flushed by this ordering edge — same trick
                    // `collect_powerups.before(sync_car_parts)` below relies
                    // on). `build_car_rig` itself is gated
                    // `not(resource_exists)` so it stops polling (and
                    // re-reading every mesh to recompute half-extents) once
                    // the rig is built, rather than rebuilding it every
                    // frame forever.
                    (
                        model::build_car_rig.run_if(not(resource_exists::<model::CarRigs>)),
                        game::enter_countdown_when_loaded.run_if(resource_added::<model::CarRigs>),
                    )
                        .chain(),
                    game::tick_countdown.run_if(in_state(game::MatchState::Countdown)),
                    game::tick_match_clock.run_if(in_state(game::MatchState::Fighting)),
                    // Ordered ahead of `camera::follow_camera` (elsewhere in
                    // this Update block — `.before` works across sub-tuples)
                    // so a handoff never leaves the camera one frame behind,
                    // pointed at a hulk that no longer has `CameraTarget`.
                    game::hand_off_camera.before(camera::follow_camera),
                    game::restart_match.run_if(in_state(game::MatchState::Over)),
                    game::debug_wreck_car.run_if(in_state(game::MatchState::Fighting)),
                    vehicle::reset_vehicle.run_if(in_state(game::MatchState::Fighting)),
                    // Only on the grid, before the fight starts — mixes with
                    // `resource_exists::<vehicle::CarAssets>` since the very
                    // first grid isn't built yet the instant `Countdown` is
                    // entered (its `OnEnter` systems run first, same frame).
                    game::cycle_player_class
                        .run_if(in_state(game::MatchState::Countdown))
                        .run_if(resource_exists::<vehicle::CarAssets>),
                    juice::tick_slow_motion,
                ),
                (
                    // Must land before `sync_car_parts`: it rebuilds part
                    // visuals a repair just brought back, and the explicit
                    // edge forces the command flush that makes the new
                    // children visible to it in the same frame.
                    //
                    // Both take a `Res<_>` unconditionally (there are no
                    // cars — and so nothing for either to do — until every
                    // class's car model has landed, but `Res<T>` validates
                    // its resource before a system body ever runs, panicking
                    // otherwise), so both are gated on existence rather
                    // than threading `Option<Res<_>>` through their
                    // bodies. `sync_car_parts` gates on `PartMaterials`
                    // rather than `CarRigs` because — unlike in the old
                    // Startup-only world — the first `CarRigs` tick doesn't
                    // guarantee `vehicle::spawn_vehicle` (which inserts
                    // `PartMaterials`, chained right after
                    // `build_car_rig`) has actually run *this* tick
                    // relative to an unordered sibling like this one;
                    // `PartMaterials` existing means it has.
                    powerups::collect_powerups
                        .before(damage::sync_car_parts)
                        .run_if(resource_exists::<model::CarRigs>),
                    damage::sync_car_parts.run_if(resource_exists::<damage::PartMaterials>),
                    damage::tick_immunity,
                    damage::update_immunity_bubbles,
                    effects::update_engine_emitters,
                    effects::spawn_impact_bursts,
                    effects::spawn_landing_dust,
                    effects::despawn_finished_bursts,
                ),
                (
                    powerups::animate_powerups,
                    powerups::despawn_expired_powerups,
                    powerups::despawn_finished_shards,
                    obstacles::respawn_crates,
                    camera::follow_camera,
                    juice::shake_camera.after(camera::follow_camera),
                ),
                (
                    ui::update_hud,
                    ui::update_damage_hud,
                    ui::update_countdown_text.run_if(in_state(game::MatchState::Countdown)),
                    ui::spawn_nameplates,
                    ui::sync_nameplates,
                ),
            ),
        )
        .run();
}
