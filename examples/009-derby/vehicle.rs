//! Building the derby's cars.
//!
//! The driving model itself — raycast suspension, the slip tire model,
//! Ackermann steering, aero drag, flip rescue, wheel visuals — lives in
//! `bevy_game_bits::vehicle` now, registered by `VehiclePlugin` in `main.rs`.
//! What's left here is everything that makes these cars *derby* cars: the
//! starting-grid ring, the per-slot paint and driver identity, and the pile of
//! destructible-part, particle-emitter and power-up children bolted onto the
//! chassis the library hands back.
//!
//! The seam is two bundles. `chassis_bundle` gives a rigid body with a
//! `VehicleSpec`, a `DriveInput` and a `DrivePower`; `wheel_bundle` gives one
//! raycast wheel. Everything below decorates them.
//!
//! Controls: W/S accelerate/brake, A/D steer, Space handbrake (all via the
//! library's `KeyboardDriver`), R reset.

use std::f32::consts::{FRAC_PI_4, PI};

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_hanabi::prelude::ParticleEffect;

use bevy_game_bits::vehicle::{
    chassis_bundle, wheel_bundle, KeyboardDriver, VehicleGeometry, VehicleTuning,
};

use crate::ai::AiDriver;
use crate::config::*;
use crate::damage::{CarPart, Damage, ImmunityBubble, PartMaterials};
use crate::effects::{EngineEffects, EngineEmitter};
use crate::game::{CameraTarget, Driver};
use crate::model::{CarRig, CarRigs};

/// Marker for the one chassis the keyboard drives (camera, HUD, and R-reset
/// also bind to it). Every other car runs the same physics with whatever its
/// `DriveInput` says — an `AiDriver` writes theirs.
#[derive(Component)]
pub struct Player;

/// Whatever's still procedural — everything else the car needs comes from
/// `model::CarRigs` (the artist-editable per-class `.glb`s). The immunity
/// bubble stays code-only: it's a gameplay effect, not part of the car's own
/// look, so there's nothing for an artist to author. Shared across classes
/// rather than duplicated: the bubble's shape/color don't depend on which car
/// it wraps.
///
/// A `Resource` and not a startup local for the same reason `CarRigs` is:
/// `powerups::respawn_missing_parts` and `damage::sync_car_parts` both outlive
/// startup and need these handles again later.
#[derive(Resource)]
pub struct CarAssets {
    pub bubble_mesh: Handle<Mesh>,
    pub bubble_material: Handle<StandardMaterial>,
}

/// This chassis's paint handle, the one its fenders and spoiler wear. Held
/// explicitly rather than read back off the chassis's own `MeshMaterial3d` —
/// that only works today because it happens to be the same handle; the day
/// the chassis gets its own wrecked/scorched material, respawned panels would
/// otherwise silently inherit it.
#[derive(Component)]
pub struct CarPaint(pub Handle<StandardMaterial>);

/// The starting grid sits on the `SPAWN_RING_RADIUS` ring near the walls,
/// all four cars evenly spaced and facing the center. A car at ring angle
/// `a` (position `R·(sin a, cos a)`) faces the center with yaw `a + π`.
/// The player takes the 225° slot; fellows fill the other three.
const SPAWN_ANGLES: [f32; 4] = [
    5.0 * FRAC_PI_4, // player
    FRAC_PI_4,
    3.0 * FRAC_PI_4,
    7.0 * FRAC_PI_4,
];

/// Spawn transform for a car on the starting-grid ring. `spawn_height` comes
/// from the spawning car's own `CarSpec` — classes ride at different heights.
fn spawn_transform(angle: f32, spawn_height: f32) -> Transform {
    let (sin, cos) = angle.sin_cos();
    Transform::from_xyz(SPAWN_RING_RADIUS * sin, spawn_height, SPAWN_RING_RADIUS * cos)
        .with_rotation(Quat::from_rotation_y(angle + PI))
}

/// Runs once every class's car model has landed — `model::build_car_rig`
/// gates this via `resource_added::<CarRigs>` (`main.rs`).
pub fn spawn_vehicle(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    tuning: Res<VehicleTuning>,
    effects: Res<EngineEffects>,
    rigs: Res<CarRigs>,
    player_class: Res<crate::game::PlayerClass>,
) {
    let assets = CarAssets {
        bubble_mesh: meshes.add(Sphere::new(IMMUNITY_BUBBLE_RADIUS)),
        // Translucent cyan shell for the temporary-immunity power-up —
        // hidden by default (`spawn_car`), toggled by
        // `damage::update_immunity_bubbles`.
        bubble_material: materials.add(StandardMaterial {
            base_color: Color::srgba(0.35, 0.8, 1.0, 0.18),
            emissive: LinearRgba::new(0.2, 0.6, 1.0, 1.0),
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
    };
    // Broken-state materials the damage system swaps in. (Headlights don't
    // swap — each owns its material instance and dims with its health.)
    commands.insert_resource(PartMaterials {
        glass_cracked: materials.add(StandardMaterial {
            base_color: Color::srgba(0.8, 0.85, 0.9, 0.7),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.6,
            ..default()
        }),
        // Scraped, rust-streaked steel — the ram bar past its dent
        // threshold, before it tears off entirely.
        shield_dented: materials.add(StandardMaterial {
            base_color: Color::srgb(0.35, 0.26, 0.2),
            perceptual_roughness: 0.95,
            metallic: 0.4,
            ..default()
        }),
    });

    // One grid slot at a time — slot 0's class is `PlayerClass` (seeded from
    // `DRIVERS[0]`, overridden by `game::cycle_player_class`'s `C` key);
    // every other slot's class is fixed by `DRIVERS` itself.
    for slot in 0..DRIVERS.len() {
        let class = if slot == 0 { player_class.0 } else { DRIVERS[slot].2 };
        spawn_grid_car(&mut commands, &assets, &rigs, &tuning, &mut materials, &effects, slot, class);
    }
    // Kept alive past startup: repair power-ups rebuild destroyed part
    // visuals out of these same handles (powerups.rs).
    commands.insert_resource(assets);
}

/// Spawns one starting-grid slot's car in the given class. Shared by the
/// initial/rematch grid build (`spawn_vehicle`'s loop above) and
/// `game::cycle_player_class`'s single-car respawn when `C` changes the
/// player's class mid-`Countdown`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_grid_car(
    commands: &mut Commands,
    assets: &CarAssets,
    rigs: &CarRigs,
    tuning: &VehicleTuning,
    materials: &mut Assets<StandardMaterial>,
    effects: &EngineEffects,
    slot: usize,
    class: CarClass,
) {
    let (name, rgb, _) = DRIVERS[slot];
    let rig = rigs.get(class);
    // Clones the rig's own chassis material (the artist's roughness/metallic
    // choices survive) and overrides just the color, so each car gets its
    // own paint without sharing — and fighting over — one handle.
    let mut paint = materials.get(&rig.chassis.material).cloned().unwrap_or_default();
    paint.base_color = Color::srgb(rgb.0, rgb.1, rgb.2);
    let material = materials.add(paint);
    let driver = (slot != 0).then(|| AiDriver::new(slot - 1));
    spawn_car(
        commands,
        assets,
        rig,
        class,
        tuning,
        effects,
        materials,
        material,
        spawn_transform(SPAWN_ANGLES[slot], class.spec().spawn_height),
        driver,
        Driver { name, color: Color::srgb(rgb.0, rgb.1, rgb.2) },
    );
}

#[allow(clippy::too_many_arguments)]
fn spawn_car(
    commands: &mut Commands,
    assets: &CarAssets,
    rig: &CarRig,
    class: CarClass,
    tuning: &VehicleTuning,
    effects: &EngineEffects,
    materials: &mut Assets<StandardMaterial>,
    chassis_material: Handle<StandardMaterial>,
    transform: Transform,
    driver: Option<AiDriver>,
    identity: Driver,
) {
    let spec = class.spec();
    // Wheelbase and track come from the loaded model's own wheel nodes, not
    // from `CarSpec` — deriving them means the steering model can never
    // disagree with where the wheels actually are (see `CarRig::wheelbase`).
    let geometry = VehicleGeometry {
        wheelbase: rig.wheelbase(),
        track_width: rig.track_width(),
    };
    let mut car = commands.spawn((
        Name::new(identity.name),
        // Rigid body, collider, mass properties, `Vehicle`, `DriveInput` and
        // `DrivePower` — everything `vehicle_controller` needs.
        chassis_bundle(spec.physics, tuning, geometry),
        class,
        Damage::default(),
        identity,
        Mesh3d(rig.chassis.mesh.clone()),
        MeshMaterial3d(chassis_material.clone()),
        CarPaint(chassis_material.clone()),
        transform,
    ));
    match driver {
        None => {
            // `KeyboardDriver` is what makes this the player's car; removing
            // it (as `game::check_wrecks` does on a KO) takes control away
            // without touching anything else.
            car.insert((Player, KeyboardDriver::default(), CameraTarget));
        }
        Some(driver) => {
            car.insert(driver);
        }
    }
    car.with_children(|chassis| {
        for (i, wheel_part) in rig.wheels.iter().enumerate() {
            // Front wheels steer; rear don't — read off the rig's own
            // geometry (positive local Z is forward, `CarSpec::
            // wheel_mounts`'s convention) rather than a hardcoded index, so
            // a model with its wheels reordered still steers the right ones.
            // All four are driven: AWD, so the launch is engine-limited
            // instead of rear-traction-limited.
            let steering = wheel_part.transform.translation.z > 0.0;
            chassis
                .spawn((
                    Name::new("Wheel"),
                    wheel_bundle(&spec.physics, wheel_part.transform, steering, true),
                    Mesh3d(wheel_part.mesh.clone()),
                    MeshMaterial3d(wheel_part.material.clone()),
                ))
                .with_children(|wheel| {
                    // The stripe sits in the wheel's own (pre-alignment)
                    // frame, so it spins as one rigid unit with the wheel —
                    // see `model.rs::stripe_part`.
                    let stripe = &rig.stripes[i];
                    wheel.spawn((Mesh3d(stripe.mesh.clone()), MeshMaterial3d(stripe.material.clone()), stripe.transform));
                });
        }

        // Destructible part visuals (no colliders — hits land on the
        // chassis box and are attributed to parts by region; see damage.rs).
        // Per-side arrays index 0 = −X, 1 = +X (`vehicle::side_index`).
        for side in 0..2 {
            // Each headlight owns its material instance: the damage system
            // dims its emissive continuously with the light's health.
            let headlight_template = materials.get(&rig.headlights[side].material).cloned().unwrap_or_default();
            let headlight_material = materials.add(headlight_template);
            chassis.spawn((
                Name::new("Headlight"),
                CarPart::Light { side },
                Mesh3d(rig.headlights[side].mesh.clone()),
                MeshMaterial3d(headlight_material),
                rig.headlights[side].transform,
            ));
            // Fenders and the spoiler wear the car's paint, so debris in
            // the arena tells you whose panel it was.
            chassis.spawn((
                Name::new("Fender"),
                CarPart::Fender { side },
                Mesh3d(rig.fenders[side].mesh.clone()),
                MeshMaterial3d(chassis_material.clone()),
                rig.fenders[side].transform,
            ));
            chassis.spawn((
                Name::new("SpoilerStrut"),
                Mesh3d(rig.spoiler_struts[side].mesh.clone()),
                MeshMaterial3d(rig.spoiler_struts[side].material.clone()),
                rig.spoiler_struts[side].transform,
            ));
            // Ram bar mounts: permanent stubs that stay on the nose once
            // the bar itself tears off (mirrors SpoilerStrut).
            chassis.spawn((
                Name::new("ShieldStrut"),
                Mesh3d(rig.shield_struts[side].mesh.clone()),
                MeshMaterial3d(rig.shield_struts[side].material.clone()),
                rig.shield_struts[side].transform,
            ));
        }
        chassis.spawn((
            Name::new("Windshield"),
            CarPart::Windshield,
            Mesh3d(rig.windshield.mesh.clone()),
            MeshMaterial3d(rig.windshield.material.clone()),
            rig.windshield.transform,
        ));
        chassis.spawn((
            Name::new("Spoiler"),
            CarPart::Spoiler,
            Mesh3d(rig.spoiler.mesh.clone()),
            MeshMaterial3d(chassis_material.clone()),
            rig.spoiler.transform,
        ));
        // The ram bar. Like every other part it carries no collider — hits
        // still land on the chassis box and are attributed by region
        // (damage.rs), so the tuned suspension/collider physics is
        // untouched. Steel, not paint: it reads as bolted-on hardware, and
        // shared across the trim struts too — none of them mutate their
        // material in place, only swap the whole handle on damage.
        chassis.spawn((
            Name::new("RamBar"),
            CarPart::Shield,
            Mesh3d(rig.ram_bar.mesh.clone()),
            MeshMaterial3d(rig.ram_bar.material.clone()),
            rig.ram_bar.transform,
        ));

        // Engine-state emitters, parked over the hood, off until the
        // engine health crosses the steam/smoke thresholds (effects.rs).
        chassis.spawn((
            Name::new("SteamEmitter"),
            EngineEmitter::Steam,
            ParticleEffect::new(effects.steam.clone()),
            Transform::from_translation(ENGINE_EMITTER_OFFSET),
        ));
        chassis.spawn((
            Name::new("SmokeEmitter"),
            EngineEmitter::Smoke,
            ParticleEffect::new(effects.smoke.clone()),
            Transform::from_translation(ENGINE_EMITTER_OFFSET),
        ));

        // Temporary-immunity power-up bubble: permanent, hidden until
        // `damage::update_immunity_bubbles` shows it.
        chassis.spawn((
            Name::new("ImmunityBubble"),
            ImmunityBubble,
            Mesh3d(assets.bubble_mesh.clone()),
            MeshMaterial3d(assets.bubble_material.clone()),
            Visibility::Hidden,
            Transform::from_translation(spec.com_offset),
        ));
    });
}

/// R respawns the chassis at its spawn pose with zero velocity. The library's
/// flip rescue keeps ordinary driving from ending up stuck on the roof, but
/// this stays as a cheap, always-available "get unstuck" escape hatch (a car
/// wedged in a corner, or the debug-tuning phase throwing it somewhere silly).
/// Only registered `run_if(in_state(MatchState::Fighting))` (`main.rs`) — R
/// means something else on the results screen (`game::restart_match`).
pub fn reset_vehicle(
    keys: Res<ButtonInput<KeyCode>>,
    mut chassis: Query<
        (&mut Transform, &mut LinearVelocity, &mut AngularVelocity, &CarClass),
        With<Player>,
    >,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    for (mut transform, mut linear_velocity, mut angular_velocity, class) in &mut chassis {
        *transform = spawn_transform(SPAWN_ANGLES[0], class.spec().spawn_height);
        linear_velocity.0 = Vec3::ZERO;
        angular_velocity.0 = Vec3::ZERO;
    }
}
