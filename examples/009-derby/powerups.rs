//! Breakable crates and the power-ups they drop. A `LootCrate` that takes
//! a hard enough hit (through its own mass, not the chassis's) shatters
//! into loose debris and leaves a floating pickup at the spot; any car
//! that drives close enough collects it. Three kinds so far — a partial
//! repair, a full repair to whichever part is worst, and temporary damage
//! immunity — picked from a weighted table sized so adding a fourth is a
//! one-line change (`PowerUpKind::TABLE`). Broken crates return to their
//! original hand-placed spot after `CRATE_RESPAWN_DELAY` (`obstacles.rs`
//! owns the respawn timer and the actual crate bundle; this module only
//! announces the break via `CrateBroken`).

use std::f32::consts::{FRAC_PI_4, TAU};

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::config::*;
use bevy_game_bits::vehicle::SoftProp;
use crate::damage::{CarPart, Damage, ImpactBurst, Immunity};
use crate::model::{CarRig, CarRigs};
use bevy_game_bits::vehicle::Vehicle;
use crate::vehicle::CarPaint;

/// Marks a crate as loot-bearing — read by `break_crates`, applied by
/// `obstacles.rs`'s crate spawn (same "owning module defines it, spawner
/// module applies it" split as `vehicle::SoftProp`).
#[derive(Component)]
pub struct LootCrate;

/// A crate's hand-placed spawn slot, floor-level. Tracked on the entity so
/// a respawn lands back on the original spot regardless of how far the
/// crate rolled before it broke.
#[derive(Component, Clone, Copy)]
pub struct CrateOrigin(pub Vec3);

/// Announced by `break_crates` right after a crate is destroyed, so
/// `obstacles::respawn_crates` can start that slot's respawn timer.
#[derive(Message)]
pub struct CrateBroken {
    pub origin: Vec3,
}

/// Tags a transient crate splinter; despawned once its timer runs out —
/// `effects::BurstLifetime`'s exact pattern with a different component.
#[derive(Component)]
pub struct ShardLifetime(Timer);

/// A dropped power-up, floating over the spot its crate died on.
/// No collider and no rigid body at all — see `collect_powerups` for why.
#[derive(Component)]
pub struct PowerUp {
    pub kind: PowerUpKind,
    /// Floor-level anchor. The bob and spin are recomputed from this every
    /// frame rather than accumulated, so a pickup can't walk away from
    /// where it dropped — the same discipline the ram bar's crush pose
    /// uses in `damage::sync_car_parts`.
    pub anchor: Vec3,
    /// Phase offset in [0, τ) so a cluster of drops doesn't bob and spin
    /// in lockstep.
    pub phase: f32,
}

#[derive(Component)]
pub struct PowerUpLifetime(Timer);

/// What a broken crate drops. Adding a kind is two lines: a variant here
/// and a row in `TABLE` — the weight total, the pickup materials and the
/// roll all derive from the table.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerUpKind {
    RepairPatch,
    RepairPart,
    Immunity,
}

impl PowerUpKind {
    /// Drop table: (kind, relative weight, pickup color). Weights don't
    /// have to sum to anything. Colors are `(r, g, b)` tuples rather than
    /// `Color` so the table can stay a `const` (same reason
    /// `vehicle::FELLOW_CAR_COLORS` is).
    ///
    /// The cheap patch is the common drop; the full-part fix is the good
    /// one; immunity is the prize.
    pub const TABLE: [(Self, u32, (f32, f32, f32)); 3] = [
        (Self::RepairPatch, 5, (0.25, 0.85, 0.35)), // green
        (Self::RepairPart, 3, (0.30, 0.60, 1.00)),  // blue
        (Self::Immunity, 2, (1.00, 0.85, 0.20)),    // gold
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RepairPatch => "repair patch",
            Self::RepairPart => "repair part",
            Self::Immunity => "immunity",
        }
    }
}

/// Xorshift64 for the drop table and pickup bob/spin phase — the same
/// hand-rolled pattern as 008-colony's `WanderRng` (director.rs), so the
/// derby keeps its zero-dependency randomness. `rand` is only ever a
/// transitive dep of hanabi/bevy_math here, and isn't re-exported.
#[derive(Resource)]
pub struct DropRng(u64);

impl Default for DropRng {
    // Any nonzero seed; xorshift is a fixed point at 0.
    fn default() -> Self {
        Self(0xD1B5_4A32_D192_ED03)
    }
}

impl DropRng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Weighted pick from `PowerUpKind::TABLE`, returning the row index
    /// (which selects the pickup material) alongside the kind.
    fn roll(&mut self) -> (usize, PowerUpKind) {
        let total: u32 = PowerUpKind::TABLE.iter().map(|&(_, weight, _)| weight).sum();
        let mut ticket = (self.next() % total as u64) as u32;
        for (index, &(kind, weight, _)) in PowerUpKind::TABLE.iter().enumerate() {
            if ticket < weight {
                return (index, kind);
            }
            ticket -= weight;
        }
        (0, PowerUpKind::TABLE[0].0)
    }

    /// Bob/spin phase in [0, τ). The top 53 bits give a uniform float in
    /// [0, 1) — the same trick as `WanderRng::chance`.
    fn phase(&mut self) -> f32 {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) as f32 * TAU
    }
}

/// Shared assets for crate shards and pickups.
#[derive(Resource)]
pub struct PowerUpAssets {
    shard_mesh: Handle<Mesh>,
    shard_material: Handle<StandardMaterial>,
    pickup_mesh: Handle<Mesh>,
    pickup_materials: [Handle<StandardMaterial>; PowerUpKind::TABLE.len()],
}

pub fn setup_powerups(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let shard_mesh = meshes.add(Cuboid::new(CRATE_SHARD_SIZE, CRATE_SHARD_SIZE, CRATE_SHARD_SIZE));
    let shard_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.38, 0.18),
        perceptual_roughness: 0.9,
        ..default()
    });
    let pickup_mesh = meshes.add(Cuboid::new(POWERUP_SIZE, POWERUP_SIZE, POWERUP_SIZE));
    let pickup_materials = PowerUpKind::TABLE.map(|(_, _, (r, g, b))| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            emissive: LinearRgba::new(r, g, b, 1.0) * POWERUP_EMISSIVE,
            ..default()
        })
    });

    commands.insert_resource(PowerUpAssets {
        shard_mesh,
        shard_material,
        pickup_mesh,
        pickup_materials,
    });
}

/// Reads the same contact graph `damage::apply_impact_damage` does, but
/// through the *crate's* own mass. Runs in `FixedUpdate` right after
/// `apply_impact_damage`, on the same one-tick-old solver impulses.
///
/// Walking each crate's own edges in the graph rather than every pair in
/// the arena gives the double-break guard for free: exactly one visit per
/// crate per tick, so a box wedged between a car and the wall can't break
/// — and drop loot — twice.
pub fn break_crates(
    mut commands: Commands,
    mut rng: ResMut<DropRng>,
    assets: Res<PowerUpAssets>,
    collisions: Collisions,
    crates: Query<(Entity, &Transform, &LinearVelocity, &AngularVelocity, &CrateOrigin), With<LootCrate>>,
    mut bursts: MessageWriter<ImpactBurst>,
    mut broken: MessageWriter<CrateBroken>,
) {
    for (entity, transform, linvel, angvel, origin) in &crates {
        let hardest = collisions
            .collisions_with(entity)
            .flat_map(|pair| pair.manifolds.iter())
            .filter_map(|manifold| {
                let impulse = manifold.total_normal_impulse();
                // Through the crate's own 50 kg, not the chassis's 350: a
                // light box ends up at nearly the speed of whatever punts
                // it, so this reads directly as "hit at N m/s".
                let delta_v = impulse / CRATE_MASS;
                (delta_v >= CRATE_BREAK_DELTA_V).then(|| {
                    // Same impulse-weighted average `apply_impact_damage`
                    // uses, but only for the spark shower — the loot
                    // anchors on the crate's own center (`spawn_pickup`),
                    // since a crate crushed against a wall has its
                    // contacts inside the panel.
                    let point = manifold
                        .points
                        .iter()
                        .fold(Vec3::ZERO, |acc, p| acc + p.point * p.normal_impulse)
                        / impulse;
                    (delta_v, point)
                })
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((delta_v, point)) = hardest else {
            continue;
        };

        info!("crate {entity} broke: Δv {delta_v:.1} m/s");
        commands.entity(entity).despawn();
        shatter(&mut commands, &assets, transform, linvel, angvel);
        // Rides the existing spark path — one extra heavy shower on top of
        // whatever `apply_impact_damage` already threw for this same
        // contact, which is exactly the emphasis a break wants.
        bursts.write(ImpactBurst { position: point, heavy: true, delta_v });
        spawn_pickup(&mut commands, &assets, &mut rng, transform.translation);
        broken.write(CrateBroken { origin: origin.0 });
    }
}

/// Bursts a crate into `CRATE_SHARD_COUNT` cubes, one per octant of the
/// box, each carrying the crate's velocity at that corner (v + ω × r) plus
/// an outward fling — `damage::detach`'s recipe for a torn-off part, aimed
/// radially instead of up-and-outward, since a box coming apart should
/// open in every direction rather than cartwheel.
fn shatter(commands: &mut Commands, assets: &PowerUpAssets, transform: &Transform, linvel: &LinearVelocity, angvel: &AngularVelocity) {
    let quarter = CRATE_SIZE / 4.0;
    for index in 0..CRATE_SHARD_COUNT {
        // Octant centers, from the low three bits of the index. At ±quarter
        // with shards half that size the neighbours are `CRATE_SIZE / 2`
        // apart and `CRATE_SHARD_SIZE` wide, so nothing spawns
        // interpenetrating (which the solver would answer with an
        // explosion).
        let local = Vec3::new(
            if index & 1 == 0 { -quarter } else { quarter },
            if index & 2 == 0 { -quarter } else { quarter },
            if index & 4 == 0 { -quarter } else { quarter },
        );
        let offset = transform.rotation * local;
        let velocity = linvel.0 + angvel.0.cross(offset) + offset.normalize_or_zero() * CRATE_SHARD_FLING_SPEED;
        commands.spawn((
            Name::new("CrateShard"),
            RigidBody::Dynamic,
            Collider::cuboid(CRATE_SHARD_SIZE, CRATE_SHARD_SIZE, CRATE_SHARD_SIZE),
            Mass(CRATE_SHARD_MASS),
            AngularDamping(OBSTACLE_ANGULAR_DAMPING),
            Friction::new(OBSTACLE_FRICTION),
            Restitution::new(BOX_RESTITUTION),
            // Splinters are furniture — a car mustn't wreck itself on the
            // debris it just made (same call `damage::detach` makes for
            // torn-off parts). The intact crate deliberately isn't a
            // `SoftProp` (obstacles.rs); its wreckage is.
            SoftProp,
            Mesh3d(assets.shard_mesh.clone()),
            MeshMaterial3d(assets.shard_material.clone()),
            Transform::from_translation(transform.translation + offset).with_rotation(transform.rotation),
            LinearVelocity(velocity),
            AngularVelocity(angvel.0),
            ShardLifetime(Timer::from_seconds(CRATE_SHARD_LIFETIME, TimerMode::Once)),
        ));
    }
}

/// Reaps crate shards. Without this, a session that breaks every crate
/// leaves rigid bodies in the contact graph forever.
pub fn despawn_finished_shards(time: Res<Time>, mut commands: Commands, mut shards: Query<(Entity, &mut ShardLifetime)>) {
    for (entity, mut lifetime) in &mut shards {
        if lifetime.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

fn spawn_pickup(commands: &mut Commands, assets: &PowerUpAssets, rng: &mut DropRng, from: Vec3) {
    let (index, kind) = rng.roll();
    // Anchored on the floor under the crate's center, not the impact
    // point — a crate crushed against a wall has its contacts *in* the
    // panel, and one that breaks on landing has them under the floor.
    // Clamped inside the wall ring for the same reason.
    let anchor = from.with_y(0.0).clamp_length_max(ARENA_RADIUS - POWERUP_WALL_MARGIN);
    commands.spawn((
        Name::new("PowerUp"),
        PowerUp { kind, anchor, phase: rng.phase() },
        PowerUpLifetime(Timer::from_seconds(POWERUP_LIFETIME, TimerMode::Once)),
        Mesh3d(assets.pickup_mesh.clone()),
        // Indexed by the kind's row in `PowerUpKind::TABLE`, so a new kind
        // needs no change here.
        MeshMaterial3d(assets.pickup_materials[index].clone()),
        Transform::from_translation(anchor + Vec3::Y * POWERUP_FLOAT_HEIGHT),
    ));
    info!("crate dropped power-up: {}", kind.label());
}

/// Reaps power-ups nobody collected in time.
pub fn despawn_expired_powerups(time: Res<Time>, mut commands: Commands, mut pickups: Query<(Entity, &mut PowerUpLifetime)>) {
    for (entity, mut lifetime) in &mut pickups {
        if lifetime.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// Bob + spin, recomputed from `PowerUp::anchor` every frame rather than
/// accumulated onto `Transform`.
pub fn animate_powerups(time: Res<Time>, mut pickups: Query<(&PowerUp, &mut Transform)>) {
    let elapsed = time.elapsed_secs();
    for (powerup, mut transform) in &mut pickups {
        transform.translation = powerup.anchor
            + Vec3::Y * (POWERUP_FLOAT_HEIGHT + POWERUP_BOB_AMPLITUDE * (elapsed * POWERUP_BOB_RATE + powerup.phase).sin());
        transform.rotation =
            Quat::from_rotation_y(elapsed * POWERUP_SPIN_RATE + powerup.phase) * Quat::from_rotation_x(FRAC_PI_4);
    }
}

/// Nearest car within `POWERUP_PICKUP_RADIUS` collects a pickup — a plain
/// distance check against chassis centers, not a `Sensor` collider.
///
/// A sensor would ride a car up onto the pickup: avian's
/// `SpatialQueryFilter` has only `mask`/`excluded_entities`, nothing
/// exempts sensors from raycasts, and `vehicle_controller`'s suspension
/// ray accepts any hit facing sufficiently up — the top of a sensor
/// sphere passes that test. It would also be the only sensor and the only
/// collision-event consumer in an example that answers every contact
/// question by polling `Collisions`, adding a permanently-touching,
/// zero-impulse pair to that graph for no benefit. A distance check is
/// also trivially deterministic if two cars arrive on the same frame.
pub fn collect_powerups(
    mut commands: Commands,
    rigs: Res<CarRigs>,
    mut cars: Query<(Entity, &Transform, &CarPaint, &mut Damage, Option<&Children>, &CarClass), With<Vehicle>>,
    parts: Query<&CarPart>,
    pickups: Query<(Entity, &PowerUp, &Transform), Without<Vehicle>>,
) {
    for (pickup_entity, powerup, pickup_transform) in &pickups {
        let mut best: Option<(f32, Entity)> = None;
        for (car, transform, ..) in &cars {
            let distance = transform.translation.distance(pickup_transform.translation);
            if distance <= POWERUP_PICKUP_RADIUS && best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, car));
            }
        }
        let Some((_, car)) = best else {
            continue;
        };
        commands.entity(pickup_entity).despawn();

        let Ok((_, _, paint, mut damage, children, class)) = cars.get_mut(car) else {
            continue;
        };
        let rig = rigs.get(*class);
        match powerup.kind {
            PowerUpKind::RepairPatch => {
                for health in damage.parts_mut() {
                    *health = (*health + REPAIR_PATCH_AMOUNT).min(1.0);
                }
            }
            PowerUpKind::RepairPart => {
                let healths = damage.parts_mut();
                let mut worst = 0;
                for index in 1..healths.len() {
                    if *healths[index] < *healths[worst] {
                        worst = index;
                    }
                }
                *healths[worst] = 1.0;
            }
            PowerUpKind::Immunity => {
                // `insert` replaces any live `Immunity`, so collecting a
                // second one refreshes the clock instead of stacking.
                commands.entity(car).insert(Immunity(Timer::from_seconds(IMMUNITY_DURATION, TimerMode::Once)));
            }
        }

        // What the car still has. `sync_car_parts` despawned the
        // windshield and `detach()`ed the fenders/spoiler/ram bar when
        // they hit zero, so this is exactly the set a repair must not
        // duplicate.
        let present: Vec<CarPart> = children
            .map(|children| children.iter().filter_map(|child| parts.get(child).ok().copied()).collect())
            .unwrap_or_default();
        respawn_missing_parts(&mut commands, &rig, car, &paint.0, &damage, &present);

        info!("power-up: car {car} collected {}", powerup.kind.label());
    }
}

/// Rebuilds the part visuals a car is missing but whose health is now
/// above zero. Health alone isn't enough: `sync_car_parts` despawns the
/// windshield and `detach()`s the fenders/spoiler/ram bar as they die, so
/// any repair that lifts a part off zero would otherwise leave the HUD
/// reading a health while the car is still visibly missing the panel.
///
/// `present` is what the chassis still carries, so a repair to a part
/// that was merely dented can't bolt a second one on.
fn respawn_missing_parts(commands: &mut Commands, rig: &CarRig, car: Entity, paint: &Handle<StandardMaterial>, damage: &Damage, present: &[CarPart]) {
    let missing = |part: CarPart| !present.contains(&part);

    if damage.windshield > 0.0 && missing(CarPart::Windshield) {
        commands.entity(car).with_child((
            Name::new("Windshield"),
            CarPart::Windshield,
            Mesh3d(rig.windshield.mesh.clone()),
            // Pristine glass on purpose: `sync_car_parts`'s Windshield arm
            // isn't change-gated, so if the repair left it under
            // `WINDSHIELD_CRACK_THRESHOLD` it swaps `glass_cracked` back
            // in on its very next run — one system owns that rule.
            MeshMaterial3d(rig.windshield.material.clone()),
            rig.windshield.transform,
        ));
    }
    for side in 0..2 {
        if damage.fenders[side] > 0.0 && missing(CarPart::Fender { side }) {
            commands.entity(car).with_child((
                Name::new("Fender"),
                CarPart::Fender { side },
                Mesh3d(rig.fenders[side].mesh.clone()),
                MeshMaterial3d(paint.clone()),
                rig.fenders[side].transform,
            ));
        }
    }
    if damage.spoiler > 0.0 && missing(CarPart::Spoiler) {
        commands.entity(car).with_child((
            Name::new("Spoiler"),
            CarPart::Spoiler,
            Mesh3d(rig.spoiler.mesh.clone()),
            MeshMaterial3d(paint.clone()),
            rig.spoiler.transform,
        ));
    }
    if damage.shield > 0.0 && missing(CarPart::Shield) {
        commands.entity(car).with_child((
            Name::new("RamBar"),
            CarPart::Shield,
            Mesh3d(rig.ram_bar.mesh.clone()),
            // Pristine trim and the rig's rest pose, with no crush
            // translation and no sag rotation: `sync_car_parts` re-derives
            // both from `1.0 - damage.shield` on its very next run (its
            // `damage.is_changed()` gate passes this frame, since the
            // repair that got us here just wrote `Damage`). Never
            // accumulate the pose here — the bar is driven from health,
            // not from history.
            MeshMaterial3d(rig.ram_bar.material.clone()),
            rig.ram_bar.transform,
        ));
    }
}
