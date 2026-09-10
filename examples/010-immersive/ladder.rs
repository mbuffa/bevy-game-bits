//! Half-Life / Deus Ex-style ladder climbing, bracketed around bevy_ahoy's
//! kinematic controller.
//!
//! Aim at a ladder and press **RMB (or E)** to lock on: your body snaps onto
//! the ladder's front face (`LADDER_STANDOFF` out from its centre line, so the
//! rungs are in front of you and not through the near plane), your view eases
//! round to face the rungs, and then forward climbs and backward descends with
//! mouse-look free. Press RMB/E again
//! (or Space) to let go. There is no automatic "walk into it" grab — the
//! interact press is the only way on, so every mount lands you square whether
//! you're at the foot of the ladder or peering down at its head from the deck.
//!
//! `bevy_ahoy`'s `run_kcc` is one private monolithic system with no seam for
//! a new movement mode (its own source even marks the slot: *"here we'd
//! handle things like spectator, dead, noclip, etc."*). So this module
//! doesn't hook into it — it brackets it, using only public API:
//!
//! | system | when | job |
//! |---|---|---|
//! | `stash_input` | `RunFixedMainLoop`, `BeforeFixedMainLoop` | move the frame's intent into `Climbing` (zero if nothing is held), clearing `AccumulatedInput` so ahoy walks/jumps nowhere |
//! | `attach_on_interact` | `On<Interacted>` | the RMB/E grab: mount, or toggle off if already climbing |
//! | `let_go_on_jump` | `On<Start<Jump>>` | a Space *press* while climbing detaches (the press edge, not ahoy's buffer) |
//! | `turn_to_ladder` | `PostUpdate`, before `TransformSystems::Propagate` | ease the camera yaw onto the ladder facing after a grab |
//! | `climb` | `FixedPostUpdate`, after `MoveCharacters`, before `PhysicsSystems::First` | assert the climb position onto `Transform` and zero `LinearVelocity`, discarding whatever ahoy did this step |
//!
//! The climb height lives in `Climbing`, not in the `Transform` — `climb`
//! writes an absolute position every step, so ahoy's gravity and
//! ground-snap (which still run) are simply overwritten rather than fought.
//! Writing `Transform` in that window is exactly how ahoy moves the body
//! (avian syncs `Transform` → `Position` in `PhysicsSystems::Prepare`).
//!
//! The climb only moves while you hold forward or back. Release the stick and
//! `stash_input` reads a zero wish (ahoy publishes *nothing* on a released
//! dead-zoned axis, and `None` means "released"), so `climb` holds you where
//! you hang — at any height, in either direction.
//!
//! Leaving the ladder off the **top** is a walk, not a teleport: `climb` keeps
//! asserting the body's position while it slides `LADDER_DISMOUNT_STEP` forward
//! onto the deck (`Climbing::crest`), and only then detaches, a short drop
//! above solid ground. Hand a released body to ahoy any earlier — at the edge,
//! or metres up — and its controller drags it back toward the mount line and
//! over the lip before it can ground.
//!
//! The `func_ladder` brush is `skip`-textured — invisible. Its collider is
//! turned into a `Sensor` (the aim/grab target and the AABB source, never a
//! wall); `setup_ladders` builds the visible rails-and-rungs mesh
//! (`ladder_mesh`) from that AABB and gives *it* a solid `Collider` sized to
//! the mesh, so the rungs block movement while the generous grab volume around
//! them does not. Ready to swap for a custom `SceneRoot` model.

use std::f32::consts::{PI, TAU};

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_ahoy::input::AccumulatedInput;
use bevy_ahoy::prelude::*;
// `Start<A>` (the `just_pressed` edge of an action) isn't in either prelude
// glob; import it by name, as `input.rs` does for `Press`.
use bevy_enhanced_input::prelude::Start;
use bevy_trenchbroom::physics::SceneCollidersReady;

use crate::classes::FuncLadder;
use crate::config;
use crate::interact::Interacted;

/// Resolved ladder geometry, computed once from the brush's own collider
/// AABB — same `On<SceneCollidersReady>` timing as `door::setup_doors`, and
/// for the same reason (the `ColliderAabb` doesn't exist yet at `On<Add>`).
#[derive(Component)]
pub struct Ladder {
    /// Flat unit vector the climber faces — into the ladder, toward the deck.
    facing: Vec3,
    /// World `(x, z)` of the ladder's own centre line. The body doesn't sit
    /// here — it hangs `LADDER_STANDOFF` out along `-facing` (see `mount`) —
    /// but the top dismount steps off from here, measured against the ladder
    /// itself.
    line: Vec2,
    /// Transform-space Y range the climber's origin may occupy —
    /// `aabb.{min,max}.y + PLAYER_HEIGHT / 2`, so the feet track the rungs.
    range: (f32, f32),
}

impl Ladder {
    /// World `(x, z)` the climber's origin hangs at: `LADDER_STANDOFF` out
    /// from the centre line along the outward normal (`-facing`), so the rungs
    /// are in front of the camera instead of through it. The one source of
    /// this offset — grab, per-frame stick and bottom dismount all call it.
    fn mount(&self) -> Vec2 {
        self.line - self.facing.xz() * config::LADDER_STANDOFF
    }
}

/// On the player while attached to a ladder. `height` is the authoritative
/// origin Y; `wish` / `jump` are refreshed by `stash_input` every frame —
/// `wish` is `Vec2::ZERO` on any frame nothing is held.
#[derive(Component)]
pub struct Climbing {
    ladder: Entity,
    height: f32,
    wish: Vec2,
    jump: bool,
    /// Metres walked forward off the top so far, once `height` has reached the
    /// ceiling of `range` with an upward wish. `climb` keeps asserting the
    /// body's position through this walk — releasing it mid-crest lets ahoy
    /// pull the body back off the deck edge — and only detaches once it has
    /// covered `LADDER_DISMOUNT_STEP`.
    crest: Option<f32>,
}

/// On the player's camera entity right after an interact-grab: the target yaw
/// (radians) `turn_to_ladder` eases the view onto, then removes itself.
#[derive(Component)]
pub struct LadderTurn(f32);

/// Turn each `FuncLadder` brush into a resolved `Ladder` + `Sensor`, and
/// spawn the low-poly ladder mesh in place of the invisible (`skip`-textured)
/// brush.
///
/// `ColliderAabb` is the brush's true world box here — with no `angle` on the
/// class there's no spurious brush rotation, so the collider entity's
/// `Transform` is identity and its local AABB *is* its world AABB.
pub fn setup_ladders(
    ready: On<SceneCollidersReady>,
    ladders: Query<(&FuncLadder, &ColliderAabb), Without<Ladder>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for &entity in &ready.collider_entities {
        let Ok((ladder, aabb)) = ladders.get(entity) else {
            continue;
        };
        // `angle_to_quat`'s own convention: yaw about +Y applied to −Z.
        // face_yaw 0 → −Z, 90 → −X (bevy_trenchbroom util.rs test). The
        // world axes line up because the brush itself is unrotated.
        let face_yaw = ladder.face_yaw;
        let facing = Quat::from_rotation_y(face_yaw.to_radians()) * Vec3::NEG_Z;

        commands.entity(entity).insert((
            Ladder {
                facing,
                line: Vec2::new(aabb.center().x, aabb.center().z),
                range: (
                    aabb.min.y + config::PLAYER_HEIGHT * 0.5,
                    aabb.max.y + config::PLAYER_HEIGHT * 0.5,
                ),
            },
            // The brush volume never blocks: it's the aim/grab target and the
            // source of `Ladder`'s AABB, nothing more (as in pickup.rs). What
            // stops the player is the solid `Collider` on the visual child
            // below — a 0.10 m slab, not this 1 m grab volume.
            Sensor,
        ));

        // The visible ladder: rails + rungs, sized from the volume's wide
        // horizontal extent and height. A child of the (identity-transform)
        // brush entity, so its local transform is world space.
        let size = aabb.size();
        let width = size.dot(facing.cross(Vec3::Y).abs()).max(0.3);
        let mesh = meshes.add(ladder_mesh(width, size.y));
        let material = materials.add(StandardMaterial {
            base_color: config::LADDER_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        });
        commands.entity(entity).with_child((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            // The solid part of the ladder — exactly `ladder_mesh`'s bounding
            // box (`Collider::cuboid` and `Cuboid::new` both take full
            // extents). Solid through the rung gaps, like any shooter's ladder.
            // Avian gives this child `ColliderOf { body: <brush> }`
            // automatically — nearest `RigidBody` up the hierarchy, and
            // bevy_trenchbroom put `RigidBody::Static` on the brush — which is
            // what makes it block: ahoy moves through avian's `MoveAndSlide`,
            // whose collider set is `(With<ColliderOf>, Without<Sensor>)`. The
            // parent's `Sensor` is per-collider and doesn't reach here; a
            // `Collider` with *no* `RigidBody` ancestor would be silently
            // invisible to the KCC (no `ColliderOf`), which looks exactly like
            // "still walks through" with no error.
            Collider::cuboid(width, size.y, config::LADDER_VISUAL_DEPTH),
            Transform::from_translation(aabb.center())
                .with_rotation(Quat::from_rotation_y(face_yaw.to_radians())),
            Name::new("LadderVisual"),
        ));
    }
}

/// A low-poly ladder — two rails plus evenly spaced rungs — merged into one
/// mesh centred on the origin. Rails run along local +Y; the ladder is
/// `width` wide on local X and `config::LADDER_VISUAL_DEPTH` deep on local Z.
/// One mesh + one material, so a future custom model is a drop-in
/// `SceneRoot` swap.
fn ladder_mesh(width: f32, height: f32) -> Mesh {
    let rail_t = config::LADDER_RAIL_THICKNESS;
    let rung_t = config::LADDER_RUNG_THICKNESS;
    let depth = config::LADDER_VISUAL_DEPTH;

    // Two rails, full height, at the outer edges of `width`.
    let rail = Cuboid::new(rail_t, height, depth);
    let rail_x = (width - rail_t) * 0.5;
    let mut mesh = Mesh::from(rail).translated_by(Vec3::new(-rail_x, 0.0, 0.0));
    mesh.merge(&Mesh::from(rail).translated_by(Vec3::new(rail_x, 0.0, 0.0)))
        .unwrap();

    // Rungs span between the rails, one every `LADDER_RUNG_SPACING`, the run
    // centred so the top and bottom rungs sit just inside the rail ends.
    let rung = Cuboid::new((width - 2.0 * rail_t).max(rung_t), rung_t, depth);
    let usable = (height - rail_t).max(0.0);
    let gaps = (usable / config::LADDER_RUNG_SPACING).floor().max(1.0) as usize;
    let span = gaps as f32 * config::LADDER_RUNG_SPACING;
    for i in 0..=gaps {
        let y = -span * 0.5 + i as f32 * config::LADDER_RUNG_SPACING;
        mesh.merge(&Mesh::from(rung).translated_by(Vec3::new(0.0, y, 0.0)))
            .unwrap();
    }
    mesh
}

/// RMB or E on a ladder: the one and only way on. Snap the body onto the
/// ladder's centre line at whatever height you're at, and kick off the yaw ease
/// that turns your view to face the rungs (so W/S read as up/down). Press it
/// again while climbing and it lets go — a lock/unlock toggle. Idempotent per
/// press: works whether one binding or both fire (they can't — one press, one
/// `Interacted`), and safe against RMB also raising `Start<Grab>` (a ladder is
/// `Static`, so `carry::start_grab` rejects it).
pub fn attach_on_interact(
    trigger: On<Interacted>,
    ladders: Query<&Ladder>,
    mut players: Query<
        (
            Entity,
            &mut Transform,
            &CharacterControllerCamera,
            Has<Climbing>,
        ),
        With<CharacterController>,
    >,
    mut commands: Commands,
) {
    let Ok(ladder) = ladders.get(trigger.entity) else {
        return;
    };
    let Ok((player, mut transform, camera, climbing)) = players.single_mut() else {
        return;
    };

    // Already on a ladder: E lets go where you hang, gravity takes over.
    if climbing {
        commands.entity(player).remove::<Climbing>();
        return;
    }

    let bundle = grab(trigger.entity, ladder, transform.translation.y, Vec2::ZERO);
    let mount = ladder.mount();
    transform.translation = Vec3::new(mount.x, bundle.height, mount.y);
    commands.entity(player).insert(bundle);

    // Turn to face into the ladder. `Quat::from_rotation_y(y) * NEG_Z` is
    // `(-sin y, 0, -cos y)`, so this inverts `facing` back to its yaw.
    let yaw = f32::atan2(-ladder.facing.x, -ladder.facing.z);
    commands.entity(camera.get()).insert(LadderTurn(yaw));
}

/// Ease the camera's yaw onto the ladder facing after an interact-grab, then drop
/// the marker. Pitch is left alone — you keep looking wherever you were.
///
/// In `PostUpdate`, not `Update`: ahoy's `copy_character_look_to_camera`
/// rewrites the camera rotation from `CharacterLook` every `Update` with no
/// public set to order against, and would clobber a yaw written there.
pub fn turn_to_ladder(
    mut commands: Commands,
    time: Res<Time>,
    mut cameras: Query<(Entity, &mut Transform, &LadderTurn)>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, turn) in &mut cameras {
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        let delta = (turn.0 - yaw + PI).rem_euclid(TAU) - PI;
        if delta.abs() < 1.0_f32.to_radians() {
            transform.rotation = Quat::from_euler(EulerRot::YXZ, turn.0, pitch, 0.0);
            commands.entity(entity).remove::<LadderTurn>();
            continue;
        }
        let new_yaw = yaw + delta * (1.0 - (-config::LADDER_TURN_RATE * dt).exp());
        transform.rotation = Quat::from_euler(EulerRot::YXZ, new_yaw, pitch, 0.0);
    }
}

/// Build the `Climbing` for a grab, clamping the entry height into range.
fn grab(ladder_entity: Entity, ladder: &Ladder, entry_y: f32, wish: Vec2) -> Climbing {
    let (lo, hi) = ladder.range;
    Climbing {
        ladder: ladder_entity,
        height: entry_y.clamp(lo, hi),
        wish,
        jump: false,
        crest: None,
    }
}

/// Move the frame's intent into `Climbing` and clear `AccumulatedInput` so
/// ahoy's controller walks and jumps nowhere for a climbing player. Movement
/// is `take`n (not just read) so ahoy sees zero.
///
/// Runs once per **frame** (`RunFixedMainLoop`, `BeforeFixedMainLoop`), not per
/// fixed step: `AccumulatedInput` has a one-frame lifetime — ahoy's own
/// `clear_accumulated_input` ends it in `AfterFixedMainLoop` — so a single
/// `take` here feeds every substep the same intent. A missing `last_movement`
/// is a **release**: ahoy's `apply_movement` only writes it on `Fire<Movement>`,
/// and a dead-zoned axis doesn't fire on a zero vector, so `None` → zero wish is
/// what makes letting go of W stop the climb where you hang. (Defaulting to the
/// *last* value instead — as an earlier version did — is why release used to
/// carry you to the top. Doing this `take`/default in `FixedPostUpdate` would
/// stall every second substep of a multi-substep frame back to zero.)
pub fn stash_input(mut climbers: Query<(&mut AccumulatedInput, &mut Climbing)>) {
    for (mut input, mut climbing) in &mut climbers {
        climbing.wish = input.last_movement.take().unwrap_or(Vec2::ZERO);
        // Discarded, not read: `jumped` is ahoy's jump *buffer*, not a press
        // (see `let_go_on_jump`). Taking it every step is what stops ahoy
        // launching a climbing body off the rungs; the decision to actually
        // let go comes from the `Start<Jump>` press edge, not from here.
        input.jumped.take();
    }
}

/// A Space *press* while climbing lets go. `Start<Jump>` is the `just_pressed`
/// edge — the only thing that answers "did they ask to jump *since* grabbing?".
/// `AccumulatedInput::jumped` can't: it's ahoy's jump buffer, refreshed to
/// elapsed-zero every frame Space is held (`apply_jump` observes `Fire<Jump>`
/// and ahoy's `Jump` action is unconditioned) and left `Some` by `handle_jump`
/// whenever it declines to jump. Reading it — even age-checked against
/// `jump_input_buffer` — made "run at the ladder, jump, press E" a coin flip on
/// whether E landed within that window of releasing Space. Same "state vs
/// event" trap as the `Press` condition on `Interact` (`input.rs`).
pub fn let_go_on_jump(jump: On<Start<Jump>>, mut climbers: Query<&mut Climbing>) {
    if let Ok(mut climbing) = climbers.get_mut(jump.context) {
        climbing.jump = true;
    }
}

/// The climb itself: assert the climb position onto `Transform` and zero
/// `LinearVelocity`, every fixed step, after ahoy has run.
pub fn climb(
    mut commands: Commands,
    time: Res<Time>,
    mut players: Query<(Entity, &mut Transform, &mut LinearVelocity, &mut Climbing)>,
    ladders: Query<&Ladder>,
) {
    let dt = time.delta_secs();
    for (player, mut transform, mut velocity, mut climbing) in &mut players {
        let Ok(ladder) = ladders.get(climbing.ladder) else {
            commands.entity(player).remove::<Climbing>();
            continue;
        };
        velocity.0 = Vec3::ZERO;
        let (lo, hi) = ladder.range;

        // Cresting the top: walk the body forward onto the deck under `climb`'s
        // own control, `LADDER_CLIMB_SPEED` per step, holding it at ladder-top
        // height. A bare teleport-and-detach here doesn't stick — ahoy drags
        // the released body back toward the mount line and off the deck's edge
        // before it grounds. Walking it a full `LADDER_DISMOUNT_STEP` in and
        // letting go with the feet only a short drop above the deck leaves that
        // drift no room to matter.
        if let Some(walked) = climbing.crest {
            let walked =
                (walked + config::LADDER_CLIMB_SPEED * dt).min(config::LADDER_DISMOUNT_STEP);
            // Absolute like the height assertion below — a `+=` here would
            // accumulate against ahoy's drift and barely move.
            let p = ladder.line + ladder.facing.xz() * walked;
            transform.translation = Vec3::new(p.x, hi, p.y);
            if walked >= config::LADDER_DISMOUNT_STEP {
                commands.entity(player).remove::<Climbing>();
            } else {
                climbing.crest = Some(walked);
            }
            continue;
        }

        // Jump off — push away from the rungs and let ahoy take over.
        if climbing.jump {
            velocity.0 =
                -ladder.facing * config::LADDER_PUSH_OFF + Vec3::Y * config::LADDER_JUMP_OFF;
            commands.entity(player).remove::<Climbing>();
            continue;
        }

        climbing.height =
            (climbing.height + climbing.wish.y * config::LADDER_CLIMB_SPEED * dt).clamp(lo, hi);

        // Reached the top with an upward wish: snap onto the ladder line at the
        // top and hand over to the crest walk above from next step.
        if climbing.wish.y > 0.0 && climbing.height >= hi - 1.0e-3 {
            climbing.crest = Some(0.0);
            transform.translation = Vec3::new(ladder.line.x, hi, ladder.line.y);
            continue;
        }
        // Off the bottom, still descending: step onto the floor.
        if climbing.wish.y < 0.0 && climbing.height <= lo + 1.0e-3 {
            let mount = ladder.mount();
            transform.translation = Vec3::new(mount.x, lo, mount.y);
            commands.entity(player).remove::<Climbing>();
            continue;
        }

        // Stick: ease x/z onto the mount line (standoff from the rungs), hold
        // the climbed height.
        let mount = ladder.mount();
        let mut pos = transform.translation;
        pos.x.smooth_nudge(&mount.x, config::LADDER_SNAP_RATE, dt);
        pos.z.smooth_nudge(&mount.y, config::LADDER_SNAP_RATE, dt);
        pos.y = climbing.height;
        transform.translation = pos;
    }
}
