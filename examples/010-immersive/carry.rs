//! RMB grab / carry / charged-throw for `PropCrate` metal crates — the repo's
//! first dynamic rigid bodies, and the way onto platform B (stack them into a
//! staircase, since it has no ladder). See SPEC.md Phase 8.
//!
//! | system / observer | when | job |
//! |---|---|---|
//! | `setup_crate_assets` | `Startup` | one shared mesh + solid/ghost materials |
//! | `spawn_crates` | `On<Add, PropCrate>` | dynamic body, cuboid collider, `Mass`, friction/damping |
//! | `start_grab_or_charge` | `On<Start<Grab>>` | RMB press: grab the aimed crate, or (already carrying) begin charging a throw |
//! | `advance_charge` | `Update` | tick `ThrowCharge` up to `CARRY_CHARGE_SECS` |
//! | `release_prop` | `On<Complete<Grab>>` | RMB release *while charging*: place (tap) or throw (held) |
//! | `hold_prop` | `PostUpdate`, before `TransformSystems::Propagate` | park the held crate in front of the camera |
//!
//! **Grab gate.** Only a `RigidBody::Dynamic` body at or under
//! `config::CARRY_MAX_MASS` can be lifted. Ladder brushes are `Static` and
//! doors `Kinematic` (see `door.rs`), so both are rejected by body *type*
//! before mass is even read — that's "you can't grab a ladder" for free.
//!
//! **Why the held crate loses its `RigidBody` rather than gaining
//! `RigidBodyDisabled`.** avian's `position_to_transform` filter is
//! `Or<(With<RigidBody>, With<ApplyPosToTransform>)>`, so with the component
//! gone avian provably never writes the crate's `Transform` and can't fight
//! `hold_prop`. The one-way `transform_to_position` still runs and keeps
//! `Position` glued to the carry pose, so on release the body resumes exactly
//! where it hangs. `ChildOf` goes too, so the crate's local `Transform` is
//! world space (the `SceneRoot` is identity — no visible jump).
//! `ColliderDisabled` is required, not cosmetic: otherwise the crate floating
//! at arm's length is what `interact::update_focus`'s raycast hits every frame.

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_ahoy::prelude::*;
// Not in either prelude glob without ambiguity — import by name, as `input.rs`
// and `ladder.rs` do for `Press` / `Start`.
use bevy_enhanced_input::prelude::{Complete, Start};

use crate::classes::PropCrate;
use crate::config;
use crate::input::Grab;
use crate::interact::InteractionFocus;

/// Shared crate materials, built once at `Startup`. `ghost` is the
/// half-opacity look a crate wears while carried. Meshes are *not* shared —
/// `spawn_crates` sizes each one from `PropCrate::size`.
#[derive(Resource)]
pub struct CrateAssets {
    solid: Handle<StandardMaterial>,
    ghost: Handle<StandardMaterial>,
}

/// On the player while a crate is held.
#[derive(Component)]
pub struct Carrying {
    pub prop: Entity,
}

/// On the held crate.
#[derive(Component)]
pub struct Carried;

/// On the player between an RMB press *made while already carrying* and its
/// release: seconds held so far, capped at `config::CARRY_CHARGE_SECS`, and
/// the value the HUD slider shows. Absent means "not charging" —
/// `release_prop` no-ops without it, so the release that merely ends the
/// initial grab press doesn't fling the crate straight back out.
#[derive(Component)]
pub struct ThrowCharge(pub f32);

pub fn setup_crate_assets(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let metal = |base: Color| StandardMaterial {
        base_color: base,
        metallic: config::CRATE_METALLIC,
        perceptual_roughness: config::CRATE_ROUGHNESS,
        ..default()
    };
    commands.insert_resource(CrateAssets {
        solid: materials.add(metal(config::CRATE_COLOR)),
        ghost: materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..metal(config::CRATE_COLOR.with_alpha(config::CARRY_ALPHA))
        }),
    });
}

/// `PropCrate` from the map → a real dynamic crate, sized from `size` and
/// weighted from `mass` (which doubles as the lift gate — see the module
/// docs). The ~800 kg base crate under platform B is just a very heavy one of
/// these; nothing lifts it, and `PLAYER_PUSH_MASS` (0.5) can't nudge it, so it
/// serves as a fixed step. (A *liftable* crate bigger than `CRATE_SIZE` would
/// want `CARRY_DISTANCE`/`CARRY_DROP` scaled for the hold pose — not a case
/// today.)
pub fn spawn_crates(
    add: On<Add, PropCrate>,
    crates: Query<&PropCrate>,
    assets: Res<CrateAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let Ok(prop) = crates.get(add.entity) else {
        return;
    };
    let s = prop.size;
    commands.entity(add.entity).insert((
        Mesh3d(meshes.add(Cuboid::new(s, s, s))),
        MeshMaterial3d(assets.solid.clone()),
        RigidBody::Dynamic,
        Collider::cuboid(s, s, s),
        Mass(prop.mass),
        Friction::new(config::CRATE_FRICTION).with_combine_rule(CoefficientCombine::Max),
        LinearDamping(config::CRATE_LINEAR_DAMPING),
        AngularDamping(config::CRATE_ANGULAR_DAMPING),
        Name::new("PropCrate"),
    ));
}

/// RMB press: grab the aimed crate, or — if a crate is already in hand — start
/// charging a throw.
pub fn start_grab_or_charge(
    press: On<Start<Grab>>,
    mut commands: Commands,
    focus: Res<InteractionFocus>,
    assets: Res<CrateAssets>,
    carriers: Query<Has<Carrying>>,
    bodies: Query<(&RigidBody, &ComputedMass)>,
) {
    let player = press.context;

    if matches!(carriers.get(player), Ok(true)) {
        commands.entity(player).insert(ThrowCharge(0.0));
        return;
    }

    let Some((target, _)) = focus.0 else {
        return;
    };
    let liftable = matches!(
        bodies.get(target),
        Ok((RigidBody::Dynamic, mass)) if mass.value() <= config::CARRY_MAX_MASS
    );
    if !liftable {
        return;
    }

    commands
        .entity(target)
        .remove::<(RigidBody, ChildOf)>()
        .insert((
            Carried,
            ColliderDisabled,
            NotShadowCaster,
            MeshMaterial3d(assets.ghost.clone()),
        ));
    commands.entity(player).insert(Carrying { prop: target });
}

pub fn advance_charge(time: Res<Time>, mut charging: Query<&mut ThrowCharge>) {
    let dt = time.delta_secs();
    for mut charge in &mut charging {
        charge.0 = (charge.0 + dt).min(config::CARRY_CHARGE_SECS);
    }
}

/// RMB release. Only fires the place/throw if a `ThrowCharge` is present (the
/// press that armed it was made while carrying); the release that ends the
/// initial grab is a no-op. A near-zero charge places the crate straight down;
/// a full charge launches it along the view at `CARRY_THROW_SPEED`.
pub fn release_prop(
    release: On<Complete<Grab>>,
    mut commands: Commands,
    assets: Res<CrateAssets>,
    spatial: SpatialQuery,
    players: Query<(&Carrying, &ThrowCharge)>,
    camera: Query<(&Transform, &CharacterControllerCameraOf)>,
    colliders: Query<&Collider>,
) {
    let player = release.context;
    let Ok((carrying, charge)) = players.get(player) else {
        return;
    };
    let prop = carrying.prop;
    let Ok((cam, cam_of)) = camera.single() else {
        return;
    };
    let fwd = cam.forward();

    // Don't drop it into a wall or through the deck: sweep the crate shape
    // forward from the eye and stop at first contact.
    let mut dist = config::CARRY_DISTANCE;
    if let (Ok(shape), Ok(dir)) = (colliders.get(prop), Dir3::new(*fwd)) {
        let filter =
            SpatialQueryFilter::from_excluded_entities([cam_of.character_controller, prop]);
        if let Some(hit) = spatial.cast_shape(
            shape,
            cam.translation,
            Quat::IDENTITY,
            dir,
            &ShapeCastConfig::from_max_distance(config::CARRY_DISTANCE),
            &filter,
        ) {
            dist = hit.distance.max(0.0);
        }
    }
    let drop_at = cam.translation + *fwd * dist + Vec3::NEG_Y * config::CARRY_DROP;

    // A brief press is a *place*: drop it straight down with no launch speed,
    // so setting a crate on a stack is precise. Past the dead-zone the throw
    // scales linearly with how long RMB was held.
    let ratio = (charge.0 / config::CARRY_CHARGE_SECS).clamp(0.0, 1.0);
    let speed = if charge.0 < config::CARRY_PLACE_SECS {
        0.0
    } else {
        config::CARRY_THROW_SPEED * ratio
    };

    commands
        .entity(prop)
        .remove::<(Carried, ColliderDisabled, NotShadowCaster)>()
        .insert((
            RigidBody::Dynamic,
            MeshMaterial3d(assets.solid.clone()),
            Transform::from_translation(drop_at),
            LinearVelocity(*fwd * speed),
            AngularVelocity::default(),
        ));
    commands.entity(player).remove::<(Carrying, ThrowCharge)>();
}

/// Park the held crate a fixed offset in front of the camera, axis-aligned
/// (identity rotation — keeping it level is what lets a carried crate line up
/// on a stack). `PostUpdate` before `Propagate`, same slot and reason as
/// `ladder::turn_to_ladder`: the camera is a root entity, so its `Transform`
/// is already this frame's world pose here, and writing the crate's
/// `Transform` now lets this frame's propagate carry it into `GlobalTransform`
/// (then avian mirrors it into `Position`).
pub fn hold_prop(
    camera: Query<&Transform, (With<CharacterControllerCameraOf>, Without<Carried>)>,
    mut carried: Query<&mut Transform, With<Carried>>,
) {
    let Ok(cam) = camera.single() else {
        return;
    };
    let pos = cam.transform_point(Vec3::new(0.0, -config::CARRY_DROP, -config::CARRY_DISTANCE));
    for mut transform in &mut carried {
        transform.translation = pos;
        transform.rotation = Quat::IDENTITY;
    }
}
