//! Turning collisions into damage numbers — the half of a destruction model
//! that isn't about your game's parts.
//!
//! Each fixed step this reads the solver's contact impulses back off Avian's
//! contact graph and, for every manifold touching a vehicle, works out *how
//! hard* and *where*: the impulse-weighted contact point, that point in the
//! chassis's own local frame, which region of the body it landed on, and the
//! Δv it cost — computed through **that vehicle's own mass**, so the same
//! collision hands a lighter car more Δv for free. The result is one
//! [`VehicleImpact`] message per manifold, carrying both sides.
//!
//! What breaks, and by how much, is left entirely to you. A game reads
//! [`VehicleImpact`] and applies its own part taxonomy, armour rules and
//! thresholds; nothing here knows a headlight from a spoiler.
//!
//! Two discounts are built in, because both are properties of the *physics*
//! rather than of any part list:
//!
//! - **Landings.** A contact pushing a vehicle along its own up axis is the
//!   suspension's job, not the bodywork's, so it's discounted by
//!   [`ImpactTuning::landing_softness`] before the damage threshold. A car
//!   that lands on its *roof* is pushed the other way and gets none of it.
//! - **Soft props.** [`SoftProp`] marks bodies whose impacts should scale
//!   with their own speed: a parked ball is furniture, a punted one is a
//!   projectile. This can't be done with mass or restitution, because the
//!   contact impulse is Galilean-invariant — a car at speed into a parked
//!   ball and a ball at that speed into a parked car produce identical
//!   impulses. Only reading the prop's own velocity tells them apart.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::control::Vehicle;
use super::spec::VehicleSpec;
use super::VehicleSet;

/// Thresholds and shaping for the impulse-to-damage pipeline.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ImpactTuning {
    /// Contact Δv (m/s) below which an impact registers as nothing at all —
    /// no message side, no damage. Leaning on a wall and nudging a kerb live
    /// under here.
    pub min_delta_v: f32,
    /// Damage per m/s of Δv *past* [`min_delta_v`](Self::min_delta_v), before
    /// [`VehicleSpec::damage_scale`]. In whatever units your health is in;
    /// with 0..=1 part healths, 0.06 means a 10 m/s hit costs about half a
    /// part.
    pub damage_per_delta_v: f32,
    /// How much of a contact's Δv is discounted by how directly it pushes the
    /// vehicle along its own up axis. At 1.0 a perfectly vertical landing
    /// does no damage at all; at 0.0 there's no landing discount.
    pub landing_softness: f32,
    /// Chassis-local |z| beyond which a contact counts as
    /// [`ImpactZone::Front`]/[`Rear`](ImpactZone::Rear).
    pub front_z: f32,
    /// Chassis-local y above which a mid-body contact counts as
    /// [`ImpactZone::Top`].
    pub top_y: f32,
    /// Impact scale for a [`SoftProp`] at rest — the floor of the ramp.
    pub prop_softness_min: f32,
    /// Speed (m/s) at which a [`SoftProp`] hits at full strength.
    pub prop_softness_full_speed: f32,
}

impl Default for ImpactTuning {
    fn default() -> Self {
        Self {
            min_delta_v: 1.5,
            damage_per_delta_v: 0.06,
            landing_softness: 0.75,
            front_z: 0.6,
            top_y: 0.2,
            prop_softness_min: 0.15,
            prop_softness_full_speed: 10.0,
        }
    }
}

/// Marks a body a vehicle can shove without wrecking itself: its impacts
/// scale with its own speed instead of hitting at full strength regardless of
/// whether it was standing still.
///
/// Put it on pushable props — balls, cones, torn-off debris. Leave it off
/// walls, scenery and anything that should always hit hard.
#[derive(Component, Debug)]
pub struct SoftProp;

/// Which region of a chassis a contact landed on, in its own local frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImpactZone {
    Top,
    Front,
    Rear,
    Side,
}

impl ImpactZone {
    /// Classifies a chassis-local contact point.
    ///
    /// Front and rear are tested *before* the roof deliberately: a nose-on
    /// contact that happens to land high on the front face must still route
    /// through whatever protects the front, or front armour would silently
    /// stop working against anything hit at speed. The roof case that matters
    /// — a rollover grinding along — lands its contact near mid-chassis
    /// anyway.
    pub fn of(local: Vec3, front_z: f32, top_y: f32) -> Self {
        if local.z > front_z {
            Self::Front
        } else if local.z < -front_z {
            Self::Rear
        } else if local.y > top_y {
            Self::Top
        } else {
            Self::Side
        }
    }

    /// Lower-case name, for log lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Front => "front",
            Self::Rear => "rear",
            Self::Side => "side",
        }
    }
}

/// One vehicle's side of one contact, resolved before anything mutates.
#[derive(Clone, Copy, Debug)]
pub struct ImpactSide {
    /// The chassis this describes.
    pub vehicle: Entity,
    /// What it hit. Not necessarily a vehicle — a wall, a prop, the ground.
    pub other: Entity,
    pub zone: ImpactZone,
    /// Side index for the contact's chassis-local x: 0 = −X, 1 = +X. Handy
    /// for indexing per-side part arrays.
    pub side: usize,
    /// The contact point in this chassis's local frame.
    pub local_point: Vec3,
    /// Velocity change (m/s) this contact cost this vehicle, after both the
    /// soft-prop and landing discounts.
    pub delta_v: f32,
    /// Damage this contact earned: `(delta_v - min_delta_v) ·
    /// damage_per_delta_v · damage_scale`, or 0 below the threshold. Your
    /// armour and part rules apply on top.
    pub amount: f32,
    /// The landing discount that was applied, 0..=1 — 1.0 for a pure
    /// side-on crash, lower the more the contact was a landing. Exposed so a
    /// game can tell "fell hard" from "got rammed".
    pub landing_softness: f32,
}

impl ImpactSide {
    /// Whether this side earned any damage at all.
    pub fn registered(&self) -> bool {
        self.amount > 0.0
    }
}

/// One contact manifold that involved at least one vehicle, with both sides
/// resolved.
///
/// Both sides ride in one message on purpose. A rule where what one vehicle
/// takes depends on the *other's* state — front armour that boosts the damage
/// it deals, say — needs to read that state from before either side was
/// mutated. Having the snapshot handed to you makes that correct by
/// construction instead of by careful system ordering.
#[derive(Message, Debug)]
pub struct VehicleImpact {
    /// Impulse-weighted contact point, in world space.
    pub point: Vec3,
    /// The two colliders' sides, in manifold order. `None` where that
    /// collider wasn't a vehicle — a wall, a prop, the ground.
    pub sides: [Option<ImpactSide>; 2],
}

impl VehicleImpact {
    /// Every vehicle side of this contact — one entry for car-vs-scenery, two
    /// for car-vs-car.
    pub fn sides(&self) -> impl Iterator<Item = &ImpactSide> {
        self.sides.iter().flatten()
    }

    /// The side at `index`, if that collider was a vehicle.
    pub fn side(&self, index: usize) -> Option<&ImpactSide> {
        self.sides.get(index)?.as_ref()
    }

    /// The *other* collider's side, as seen from `index` — what the vehicle
    /// at `index` was hit by, if that was also a vehicle.
    pub fn opposite(&self, index: usize) -> Option<&ImpactSide> {
        self.side(1 - index)
    }

    /// The largest Δv either side took — how hard this hit was, for effects
    /// that want one number per collision rather than one per car.
    pub fn peak_delta_v(&self) -> f32 {
        self.sides()
            .fold(0.0_f32, |peak, side| peak.max(side.delta_v))
    }
}

/// How much of an impact with this body counts: soft at rest, full strength
/// once it's doing [`ImpactTuning::prop_softness_full_speed`]. Anything that
/// isn't a [`SoftProp`] is always 1.0.
fn prop_softness(
    entity: Entity,
    props: &Query<&LinearVelocity, With<SoftProp>>,
    tuning: &ImpactTuning,
) -> f32 {
    let Ok(velocity) = props.get(entity) else {
        return 1.0;
    };
    let t = (velocity.0.length() / tuning.prop_softness_full_speed).clamp(0.0, 1.0);
    tuning.prop_softness_min + (1.0 - tuning.prop_softness_min) * t
}

/// Reads last step's contact impulses off Avian's contact graph and writes a
/// [`VehicleImpact`] for every manifold where at least one vehicle side
/// cleared [`ImpactTuning::min_delta_v`].
///
/// Runs in `FixedUpdate` under [`VehicleSet::Impact`], after the controller.
/// The impulses it sees are one physics tick old, which at 64 Hz nobody can
/// tell.
pub fn detect_vehicle_impacts(
    tuning: Res<ImpactTuning>,
    collisions: Collisions,
    vehicles: Query<(&Transform, &VehicleSpec), With<Vehicle>>,
    // Disjoint from `vehicles` by component: this only reads
    // `LinearVelocity`, which that query never touches.
    props: Query<&LinearVelocity, With<SoftProp>>,
    mut impacts: MessageWriter<VehicleImpact>,
) {
    for pair in collisions.iter() {
        if !vehicles.contains(pair.collider1) && !vehicles.contains(pair.collider2) {
            continue;
        }
        // A property of the pair, so both the Δv threshold and any per-hit
        // effect can use it directly. Vehicle-vs-vehicle and
        // vehicle-vs-scenery are 1.0 · 1.0 = 1.0.
        let softness = prop_softness(pair.collider1, &props, &tuning)
            * prop_softness(pair.collider2, &props, &tuning);

        for manifold in &pair.manifolds {
            let total_impulse = manifold.total_normal_impulse();
            if total_impulse <= 0.0 {
                continue;
            }
            // Locate the hit: impulse-weighted average of the manifold's
            // contact points, in world space.
            let point = manifold
                .points
                .iter()
                .fold(Vec3::ZERO, |acc, p| acc + p.point * p.normal_impulse)
                / total_impulse;

            // `manifold.normal` points from collider1 to collider2, so
            // `-normal`/`+normal` is the direction the contact pushes each of
            // them respectively.
            let colliders = [
                (pair.collider1, pair.collider2, -manifold.normal),
                (pair.collider2, pair.collider1, manifold.normal),
            ];
            let sides = colliders.map(|(entity, other, push_dir)| {
                let (transform, spec) = vehicles.get(entity).ok()?;
                let local = transform
                    .rotation
                    .inverse()
                    .mul_vec3(point - transform.translation);
                // A contact pushing this vehicle along its *own* up axis is a
                // landing, discounted before the threshold. `max(0.0)`: a car
                // pushed the other way — landed on its roof, or lying under
                // another car — gets none of the discount.
                let landing_softness = 1.0
                    - tuning.landing_softness * push_dir.dot(transform.rotation * Vec3::Y).max(0.0);
                // Softness scales Δv *before* the threshold, not the damage
                // after it — so a soft hit has to be genuinely violent to
                // register at all.
                let delta_v = softness * landing_softness * total_impulse / spec.mass;
                let amount = if delta_v >= tuning.min_delta_v {
                    (delta_v - tuning.min_delta_v) * tuning.damage_per_delta_v * spec.damage_scale
                } else {
                    0.0
                };
                Some(ImpactSide {
                    vehicle: entity,
                    other,
                    zone: ImpactZone::of(local, tuning.front_z, tuning.top_y),
                    side: side_index(local.x),
                    local_point: local,
                    delta_v,
                    amount,
                    landing_softness,
                })
            });

            // Neither side reached the threshold — a lean or a nudge, for
            // both of them.
            if !sides.iter().flatten().any(ImpactSide::registered) {
                continue;
            }
            impacts.write(VehicleImpact { point, sides });
        }
    }
}

/// Side index for a chassis-local x: 0 = −X, 1 = +X. The convention
/// [`ImpactSide::side`] uses, exposed so per-side part arrays elsewhere can
/// agree with it.
pub fn side_index(local_x: f32) -> usize {
    if local_x < 0.0 {
        0
    } else {
        1
    }
}

/// Physical properties for parts torn off a vehicle by
/// [`detach_as_debris`].
#[derive(Resource, Clone, Copy, Debug)]
pub struct DebrisConfig {
    pub mass: f32,
    /// Extra speed (m/s) added up-and-outward on top of the chassis's own
    /// velocity at the mount, so a part visibly leaves rather than dribbling
    /// off.
    pub fling_speed: f32,
    pub friction: f32,
    pub restitution: f32,
    pub angular_damping: f32,
}

impl Default for DebrisConfig {
    fn default() -> Self {
        Self {
            mass: 8.0,
            fling_speed: 3.0,
            friction: 0.6,
            restitution: 0.1,
            angular_damping: 0.3,
        }
    }
}

/// The chassis motion a part is being torn off, in world space.
#[derive(Clone, Copy, Debug)]
pub struct ChassisMotion {
    /// The chassis's own translation — the origin the mount's lever arm is
    /// measured from.
    pub center: Vec3,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

/// Despawns a part child of a vehicle and spawns a free-flying debris body at
/// its world pose, inheriting the chassis velocity at that mount (`v + ω × r`)
/// plus an up-and-outward fling.
///
/// Returns the debris entity, so callers can tag it — most games want to
/// sweep debris up on a restart, and to mark it [`SoftProp`] so driving over
/// your own torn-off bodywork doesn't hurt.
#[allow(clippy::too_many_arguments)]
pub fn detach_as_debris(
    commands: &mut Commands,
    part: Entity,
    part_global: &GlobalTransform,
    size: Vec3,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    chassis: ChassisMotion,
    config: &DebrisConfig,
) -> Entity {
    let transform = part_global.compute_transform();
    let r = transform.translation - chassis.center;
    let outward = r.with_y(0.0).normalize_or_zero();
    let velocity = chassis.linear_velocity
        + chassis.angular_velocity.cross(r)
        + (outward + Vec3::Y).normalize() * config.fling_speed;

    commands.entity(part).despawn();
    commands
        .spawn((
            Name::new("Debris"),
            RigidBody::Dynamic,
            Collider::cuboid(size.x, size.y, size.z),
            Mass(config.mass),
            AngularDamping(config.angular_damping),
            Friction::new(config.friction),
            Restitution::new(config.restitution),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            transform,
            LinearVelocity(velocity),
            AngularVelocity(chassis.angular_velocity),
        ))
        .id()
}

/// Adds the impulse-to-damage pipeline. Optional: without it, vehicles still
/// collide, they just don't report it.
///
/// Register your own consumer after [`VehicleSet::Impact`]:
///
/// ```ignore
/// app.add_plugins(VehicleImpactPlugin::default())
///     .add_systems(FixedUpdate, apply_damage.after(VehicleSet::Impact));
/// ```
#[derive(Default)]
pub struct VehicleImpactPlugin {
    pub tuning: ImpactTuning,
    /// Also inserted as a resource, for convenience — [`detach_as_debris`]
    /// takes it by reference and doesn't care where it came from.
    pub debris: DebrisConfig,
}

impl Plugin for VehicleImpactPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.tuning)
            .insert_resource(self.debris)
            .add_message::<VehicleImpact>()
            .configure_sets(FixedUpdate, VehicleSet::Impact.after(VehicleSet::Control))
            .add_systems(
                FixedUpdate,
                detect_vehicle_impacts.in_set(VehicleSet::Impact),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONT_Z: f32 = 0.6;
    const TOP_Y: f32 = 0.2;

    fn zone(x: f32, y: f32, z: f32) -> ImpactZone {
        ImpactZone::of(Vec3::new(x, y, z), FRONT_Z, TOP_Y)
    }

    #[test]
    fn contacts_past_the_axle_line_are_front_or_rear() {
        assert_eq!(zone(0.0, 0.0, 1.5), ImpactZone::Front);
        assert_eq!(zone(0.0, 0.0, -1.5), ImpactZone::Rear);
    }

    /// The ordering that matters: a nose-on hit landing high on the front
    /// face is still a *front* hit, so front armour keeps protecting against
    /// anything hit at speed.
    #[test]
    fn a_high_contact_on_the_nose_is_front_not_top() {
        assert_eq!(zone(0.0, 0.5, 1.5), ImpactZone::Front);
        // ...while the same height mid-body is the roof.
        assert_eq!(zone(0.0, 0.5, 0.0), ImpactZone::Top);
    }

    #[test]
    fn mid_body_contacts_below_the_roof_line_are_sides() {
        assert_eq!(zone(0.9, 0.0, 0.0), ImpactZone::Side);
        assert_eq!(zone(-0.9, -0.2, 0.3), ImpactZone::Side);
    }

    #[test]
    fn side_index_splits_at_the_center_line() {
        assert_eq!(side_index(-0.9), 0);
        assert_eq!(side_index(0.9), 1);
        // Dead center has to land somewhere; +X is the documented choice.
        assert_eq!(side_index(0.0), 1);
    }

    /// A vehicle that isn't in the manifold at all leaves its slot empty, and
    /// the helpers have to cope with that rather than assuming two cars.
    #[test]
    fn a_one_sided_impact_reports_one_side() {
        let side = ImpactSide {
            vehicle: Entity::from_raw_u32(1).unwrap(),
            other: Entity::from_raw_u32(2).unwrap(),
            zone: ImpactZone::Front,
            side: 1,
            local_point: Vec3::new(0.2, 0.0, 1.5),
            delta_v: 6.0,
            amount: 0.27,
            landing_softness: 1.0,
        };
        let impact = VehicleImpact {
            point: Vec3::ZERO,
            sides: [Some(side), None],
        };
        assert_eq!(impact.sides().count(), 1);
        assert!(impact.side(0).is_some());
        assert!(impact.opposite(0).is_none());
        assert_eq!(impact.peak_delta_v(), 6.0);
    }
}
