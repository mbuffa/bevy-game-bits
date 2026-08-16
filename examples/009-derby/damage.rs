//! Destructible parts. Every chassis carries a `Damage` component (part
//! healths 1.0 → 0.0) and a set of `CarPart` visual children (headlights,
//! windshield, fenders, spoiler, ram bar — the engine is internal, its state
//! shows as steam/smoke from the hood, see `effects.rs`). Dead lights go dark,
//! a dead windshield shatters, dead fenders/spoiler/ram bar tear off as loose
//! rigid bodies. Wheels are unbreakable (their lug nuts are load-bearing, and
//! they have no colliders to hit anyway).
//!
//! *How hard* a hit was is not this module's problem any more. The library's
//! `detect_vehicle_impacts` reads the solver's contact impulses off Avian's
//! contact graph and hands over a `VehicleImpact` per manifold: where it
//! landed on each car in its own local frame, which zone that is, and the Δv
//! and raw damage it earned through that car's own mass. What's left here —
//! and it's the derby-specific half — is deciding *what breaks*.
//!
//! The front ram bar is the reward for playing aggressively: while it has
//! health it absorbs most of a frontal hit (`SHIELD_ABSORB`) and boosts the
//! damage dealt to whoever it hits (`RAM_DAMAGE_BONUS`) — ramming nose-first
//! is a good trade until the bar wears through and tears off. That rule needs
//! the *other* car's bar health from before either car was mutated, which is
//! why `VehicleImpact` carries both sides at once.

use avian3d::prelude::*;
use bevy::prelude::*;

use bevy_game_bits::vehicle::{
    detach_as_debris, ChassisMotion, DebrisConfig, DrivePower, ImpactZone, SoftProp, Vehicle,
    VehicleImpact,
};

use crate::config::*;
use crate::model::CarRigs;

/// Part healths for one chassis, each 1.0 (pristine) → 0.0 (destroyed).
/// Two-element arrays are indexed by side: 0 = −X (driver's right), 1 = +X
/// (driver's left — positive steer turns toward +X).
#[derive(Component)]
pub struct Damage {
    pub engine: f32,
    pub lights: [f32; 2],
    pub windshield: f32,
    pub fenders: [f32; 2],
    pub spoiler: f32,
    pub shield: f32,
}

impl Default for Damage {
    fn default() -> Self {
        Self {
            engine: 1.0,
            lights: [1.0, 1.0],
            windshield: 1.0,
            fenders: [1.0, 1.0],
            spoiler: 1.0,
            shield: 1.0,
        }
    }
}

impl Damage {
    /// Engine power factor for limp mode: 1.0 at full health, down to the
    /// `ENGINE_MIN_POWER` floor at 0 — never a dead stop.
    pub fn engine_power(&self) -> f32 {
        ENGINE_MIN_POWER + (1.0 - ENGINE_MIN_POWER) * self.engine
    }

    /// Overall health, 1.0 (pristine) down to 0.0 (wrecked), used by the AI
    /// to decide when to break off and recover. Weighted rather than a
    /// flat mean: the engine is the only part that changes what the car
    /// can *do* (`engine_power()`'s limp-mode floor), and the ram bar is
    /// the only part that changes what a hit *costs* (`SHIELD_ABSORB`) and
    /// *deals* (`RAM_DAMAGE_BONUS`) — everything else is scored but
    /// cosmetic, and shares what's left of the weight.
    pub fn condition(&self) -> f32 {
        let cosmetic_weight = 1.0 - CONDITION_ENGINE_WEIGHT - CONDITION_SHIELD_WEIGHT;
        let cosmetic_mean = (self.lights[0]
            + self.lights[1]
            + self.windshield
            + self.fenders[0]
            + self.fenders[1]
            + self.spoiler)
            / 6.0;
        CONDITION_ENGINE_WEIGHT * self.engine
            + CONDITION_SHIELD_WEIGHT * self.shield
            + cosmetic_weight * cosmetic_mean
    }

    /// Every part health as a mutable slot, in HUD order. The single
    /// place a new destructible part has to be added for both repair
    /// power-ups to pick it up — the patch job walks all of them, the
    /// full repair takes the minimum.
    pub fn parts_mut(&mut self) -> [&mut f32; 8] {
        // Destructured rather than indexed: `&mut self.lights[0]` and
        // `&mut self.lights[1]` are two `IndexMut` calls, which the
        // borrow checker can't prove disjoint. Array patterns can.
        let [light_left, light_right] = &mut self.lights;
        let [fender_left, fender_right] = &mut self.fenders;
        [
            &mut self.engine,
            light_left,
            light_right,
            &mut self.windshield,
            fender_left,
            fender_right,
            &mut self.spoiler,
            &mut self.shield,
        ]
    }
}

/// Temporary damage immunity from a power-up. While this is on a chassis,
/// `apply_impact_damage` skips every hit aimed *at* it — it still deals
/// full damage (including the ram bonus) to whoever it hits.
///
/// Emphatically not `Damage::shield` / `CarPart::Shield`, which is the
/// physical front ram bar — hence `Immunity` and not any word starting
/// with "shield".
#[derive(Component)]
pub struct Immunity(pub Timer);

/// The translucent bubble drawn around an immune car. A permanent child,
/// one per chassis, hidden until it matters — the same shape as the
/// steam/smoke `EngineEmitter` children, which are also always present
/// and merely toggled.
#[derive(Component)]
pub struct ImmunityBubble;

/// Expires immunity. Removing the component is the whole state change —
/// `apply_impact_damage` reads it with `Has`, and the bubble follows.
pub fn tick_immunity(
    time: Res<Time>,
    mut commands: Commands,
    mut cars: Query<(Entity, &mut Immunity)>,
) {
    for (entity, mut immunity) in &mut cars {
        if immunity.0.tick(time.delta()).is_finished() {
            commands.entity(entity).remove::<Immunity>();
        }
    }
}

/// Drives each bubble's visibility from its car's `Immunity`, exactly the
/// way `update_engine_emitters` drives the steam/smoke spawners from
/// engine health.
pub fn update_immunity_bubbles(
    cars: Query<Has<Immunity>, With<Vehicle>>,
    mut bubbles: Query<(&ChildOf, &mut Visibility), With<ImmunityBubble>>,
) {
    for (child_of, mut visibility) in &mut bubbles {
        let Ok(immune) = cars.get(child_of.parent()) else {
            continue;
        };
        let wanted = if immune {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Marks a destructible visual child of a chassis and names which health it
/// reflects.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum CarPart {
    Light { side: usize },
    Windshield,
    Fender { side: usize },
    Spoiler,
    Shield,
}

/// Shared material handles the part visuals swap to as they break —
/// inserted by `spawn_vehicle` alongside the other car assets. Headlights
/// aren't here: each owns a material instance that dims with its health.
#[derive(Resource)]
pub struct PartMaterials {
    pub glass_cracked: Handle<StandardMaterial>,
    pub shield_dented: Handle<StandardMaterial>,
}

/// Marks a `vehicle::SoftProp` that only exists because a match tore it off a
/// car (`sync_car_parts`, below) — as opposed to the permanent arena balls,
/// which are also `SoftProp` but must survive a restart. `game::start_match`
/// despawns everything tagged with this (plus crate shards) so a new match
/// begins with a clean floor.
#[derive(Component)]
pub struct MatchDebris;

/// One registered impact, announced for the spark-burst effect (`effects.rs`
/// spawns a one-shot emitter at the point) and screenshake (`juice.rs`).
/// `heavy` when the hit removed at least `BURST_HEAVY_DAMAGE` of part
/// health. `delta_v` is the same softness-scaled value the damage amount was
/// computed from, so shake intensity and part damage agree on how hard a
/// hit was.
#[derive(Message)]
pub struct ImpactBurst {
    pub position: Vec3,
    pub heavy: bool,
    pub delta_v: f32,
}

/// Turns a `VehicleImpact` into part damage.
///
/// The library already worked out *how hard* each car was hit and *where* on
/// its own body; everything below is the derby's own rule set — which parts a
/// zone costs, what the ram bar absorbs and confers, and who's immune.
///
/// Runs in `FixedUpdate` after `VehicleSet::Impact`. The impulses behind these
/// messages are one physics tick old, which at 64 Hz nobody can tell.
pub fn apply_impact_damage(
    mut impacts: MessageReader<VehicleImpact>,
    mut cars: Query<(&mut Damage, Has<Immunity>)>,
    mut bursts: MessageWriter<ImpactBurst>,
) {
    for impact in impacts.read() {
        // One burst per manifold, not per car — a car-vs-car hit is one
        // shower of sparks at the shared contact, not two. Reports the
        // harder-hit side's Δv, tiered by the raw per-hit damage before any
        // shield absorption: a bar-blocked ram still throws its heavy shower,
        // because that's the bar taking the hit.
        let heavy = impact.sides().any(|side| side.amount >= BURST_HEAVY_DAMAGE);
        bursts.write(ImpactBurst {
            position: impact.point,
            heavy,
            delta_v: impact.peak_delta_v(),
        });

        // Read pass. Bar health and immunity are snapshotted for *both* cars
        // before either is mutated, because the ram bonus a car receives
        // depends on the other's bar — which the apply pass below is about to
        // wear down. Without this, whichever car happened to be processed
        // second would read a bar the first pass had already damaged.
        let state = [0, 1].map(|index| {
            let side = impact.side(index)?;
            let (damage, immune) = cars.get(side.vehicle).ok()?;
            Some((damage.shield, immune))
        });

        // Apply pass: both entities of a car-vs-car pair take damage, each
        // attributing the same contact through its own zone and its own
        // Δv-derived `amount`.
        for index in 0..2 {
            let Some(hit) = impact.side(index) else {
                continue;
            };
            if !hit.registered() {
                continue;
            }
            let Some((own_shield, immune)) = state[index] else {
                continue;
            };
            // Immunity absorbs everything aimed at *this* car and nothing
            // else — the other index's iteration is untouched and reads its
            // ram-bonus input from the snapshot, so an immune car still deals
            // full damage (and confers its full bonus) to whoever it hits.
            // Skipping before the shield-wear line below also means an immune
            // car's own bar takes no wear from incoming hits: immunity is
            // free defense, not offense.
            if immune {
                info!(
                    "impact: car {} {} Δv {:.1} m/s — IMMUNE",
                    hit.vehicle,
                    hit.zone.label(),
                    hit.delta_v,
                );
                continue;
            }
            // Rammed nose-first by a car whose bar is still standing? Take
            // extra — this is the reward for playing aggressive.
            let ram_bonus = match (impact.opposite(index), state[1 - index]) {
                (Some(other), Some((other_shield, _))) if other.zone == ImpactZone::Front => {
                    1.0 + RAM_DAMAGE_BONUS * other_shield
                }
                _ => 1.0,
            };
            // Your own bar eats most of what comes back through the front,
            // until it's worn through.
            let absorbed = if hit.zone == ImpactZone::Front {
                SHIELD_ABSORB * own_shield
            } else {
                0.0
            };
            let to_body = hit.amount * ram_bonus * (1.0 - absorbed);

            let Ok((mut damage, _)) = cars.get_mut(hit.vehicle) else {
                continue;
            };
            let apply = |health: &mut f32, share: f32| {
                *health = (*health - to_body * share).max(0.0);
            };

            match hit.zone {
                ImpactZone::Top => {
                    // Roof contact — a rollover grinds the windshield.
                    apply(&mut damage.windshield, 1.0);
                }
                ImpactZone::Front => {
                    // The bar wears from the *raw* impact, not the
                    // post-absorption remainder — absorbing it is what
                    // destroys it. The rest: the engine eats most of what
                    // gets through, the nearer headlight pops, the glass
                    // cracks from chassis flex.
                    damage.shield = (damage.shield - hit.amount * SHIELD_WEAR).max(0.0);
                    apply(&mut damage.engine, 0.6);
                    apply(&mut damage.lights[hit.side], 0.25);
                    apply(&mut damage.windshield, 0.15);
                }
                ImpactZone::Rear => {
                    // Rear: mostly the spoiler — backing into people to
                    // protect your engine is the classic tactic.
                    apply(&mut damage.spoiler, 0.8);
                    apply(&mut damage.engine, 0.1);
                }
                ImpactZone::Side => {
                    apply(&mut damage.fenders[hit.side], 1.0);
                }
            }

            info!(
                "impact: car {} {} Δv {:.1} m/s (land {:.2}, ram ×{ram_bonus:.2}) \
                 — bar {:.2} engine {:.2} lights {:.2}/{:.2} windshield {:.2} \
                 fenders {:.2}/{:.2} spoiler {:.2}",
                hit.vehicle,
                hit.zone.label(),
                hit.delta_v,
                hit.landing_softness,
                damage.shield,
                damage.engine,
                damage.lights[0],
                damage.lights[1],
                damage.windshield,
                damage.fenders[0],
                damage.fenders[1],
                damage.spoiler,
            );
        }
    }
}

/// Mirrors engine health into the `DrivePower` the library's controller
/// reads. The whole of what the driving model knows about damage: a wrecked
/// engine limps at `ENGINE_MIN_POWER` rather than stopping dead.
///
/// Runs between `VehicleSet::Input` and `VehicleSet::Control`, so the power
/// the controller uses this tick reflects the damage applied last tick —
/// exactly when it used to read `Damage` directly.
pub fn sync_drive_power(mut cars: Query<(&Damage, &mut DrivePower), With<Vehicle>>) {
    for (damage, mut power) in &mut cars {
        let wanted = damage.engine_power();
        if power.0 != wanted {
            power.0 = wanted;
        }
    }
}

/// Makes part state visible: headlights dim with their health (emissive
/// fades by health², the lens color grays out — at 0 the light is simply
/// dark), a half-gone windshield swaps to the cracked material (shattering
/// entirely at 0), and dead fenders / a dead spoiler tear off as loose
/// rigid bodies that keep the chassis velocity at their mount plus an
/// up-and-outward fling. Strut stubs stay on the chassis as broken mounts.
pub fn sync_car_parts(
    mut commands: Commands,
    materials: Res<PartMaterials>,
    debris: Res<DebrisConfig>,
    rigs: Res<CarRigs>,
    mut material_assets: ResMut<Assets<StandardMaterial>>,
    // `Without<CarPart>` makes this provably disjoint from `parts` below
    // (every part carries `CarPart`; no chassis ever does), which is what
    // lets `parts` hold `&mut Transform` for the ram bar's dent/sag pose
    // alongside this query's `&Transform` on the chassis.
    cars: Query<(Ref<Damage>, &LinearVelocity, &AngularVelocity, &Transform, &CarClass), Without<CarPart>>,
    mut parts: Query<(
        Entity,
        &CarPart,
        &ChildOf,
        &GlobalTransform,
        &Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Transform,
    )>,
) {
    for (entity, part, child_of, global, mesh, mut material, mut transform) in &mut parts {
        let Ok((damage, linvel, angvel, chassis, class)) = cars.get(child_of.parent()) else {
            continue;
        };
        let rig = rigs.get(*class);
        match *part {
            CarPart::Light { side } => {
                // Each headlight owns its material instance (vehicle.rs), so
                // dimming it can't affect any other light. Only touch the
                // asset when the damage actually changed — a mutated
                // material re-uploads to the GPU.
                if !damage.is_changed() {
                    continue;
                }
                let Some(lens) = material_assets.get_mut(&material.0) else {
                    continue;
                };
                let health = damage.lights[side];
                // Quadratic so the first hits already visibly dim the beam.
                lens.emissive = HEADLIGHT_EMISSIVE * health * health;
                let tint = Vec3::new(0.25, 0.24, 0.2).lerp(Vec3::new(1.0, 0.95, 0.8), health);
                lens.base_color = Color::srgb(tint.x, tint.y, tint.z);
            }
            CarPart::Windshield => {
                if damage.windshield <= 0.0 {
                    commands.entity(entity).despawn();
                } else if damage.windshield <= WINDSHIELD_CRACK_THRESHOLD
                    && material.0 != materials.glass_cracked
                {
                    material.0 = materials.glass_cracked.clone();
                }
            }
            CarPart::Fender { side } => {
                if damage.fenders[side] <= 0.0 {
                    detach(
                        &mut commands,
                        entity,
                        rig.fenders[side].half_extents * 2.0,
                        global,
                        mesh,
                        &material,
                        linvel,
                        angvel,
                        chassis,
                        &debris,
                    );
                }
            }
            CarPart::Spoiler => {
                if damage.spoiler <= 0.0 {
                    detach(
                        &mut commands,
                        entity,
                        rig.spoiler.half_extents * 2.0,
                        global,
                        mesh,
                        &material,
                        linvel,
                        angvel,
                        chassis,
                        &debris,
                    );
                }
            }
            CarPart::Shield => {
                // Writes `Transform` every time it runs, so gate on the
                // change tick or every car's ram bar would dirty the
                // transform hierarchy each frame. `Ref::is_changed` is also
                // true on the spawn tick, so the pristine pose still gets
                // set once.
                if !damage.is_changed() {
                    continue;
                }
                if damage.shield <= 0.0 {
                    detach(
                        &mut commands,
                        entity,
                        rig.ram_bar.half_extents * 2.0,
                        global,
                        mesh,
                        &material,
                        linvel,
                        angvel,
                        chassis,
                        &debris,
                    );
                    continue;
                }
                if damage.shield <= SHIELD_DENT_THRESHOLD && material.0 != materials.shield_dented
                {
                    material.0 = materials.shield_dented.clone();
                }
                // Recomputed from the rig's rest pose every time, not
                // accumulated, so repeated hits can't walk the bar off the
                // car: it's driven back into the nose and rolled off one
                // mount as it crumples.
                let crush = 1.0 - damage.shield;
                transform.translation = rig.ram_bar.transform.translation - Vec3::Z * class.spec().shield_crush_depth * crush;
                transform.rotation = rig.ram_bar.transform.rotation * Quat::from_rotation_z(SHIELD_SAG_ANGLE * crush);
            }
        }
    }
}

/// Tears a dead part off its chassis: the library spawns the debris body
/// (chassis velocity at the mount plus an up-and-outward fling), and derby
/// tags it so a restart sweeps it up and so driving over your own bodywork
/// doesn't hurt.
#[allow(clippy::too_many_arguments)]
fn detach(
    commands: &mut Commands,
    part: Entity,
    size: Vec3,
    global: &GlobalTransform,
    mesh: &Mesh3d,
    material: &MeshMaterial3d<StandardMaterial>,
    linvel: &LinearVelocity,
    angvel: &AngularVelocity,
    chassis: &Transform,
    config: &DebrisConfig,
) {
    let debris = detach_as_debris(
        commands,
        part,
        global,
        size,
        mesh.0.clone(),
        material.0.clone(),
        ChassisMotion {
            center: chassis.translation,
            linear_velocity: linvel.0,
            angular_velocity: angvel.0,
        },
        config,
    );
    // Torn-off wreckage is a `SoftProp` too — driving over your own dead
    // fender or ram bar shouldn't hurt.
    commands.entity(debris).insert((SoftProp, MatchDebris));
}
