//! Debug "noclip" fly mode: free 3D flight with no collision and no gravity,
//! on by default (`config::NOCLIP_DEFAULT`). `N` toggles it at runtime.
//!
//! `bevy_ahoy`'s `run_kcc` is one private monolithic system with no seam for
//! a new movement mode (its own source even marks the slot: *"here we'd
//! handle things like spectator, dead, noclip, etc."*) — the same situation
//! `ladder.rs` documents for climbing. So this module doesn't hook into ahoy
//! either; it brackets it with only public API, the same two-system shape:
//!
//! | system | when | job |
//! |---|---|---|
//! | `toggle_noclip` | `Update` | raw `N` press: flip `Noclip::active`, insert/remove `ColliderDisabled` |
//! | `stash_noclip_input` | `RunFixedMainLoop`, `BeforeFixedMainLoop` | move the frame's intent into `Noclip::wish` (zero if nothing is held), clearing `AccumulatedInput` so ahoy walks/jumps nowhere |
//! | `fly` | `FixedPostUpdate`, after `MoveCharacters`, before `PhysicsSystems::First` | write an absolute `Transform` from the look direction + wish, zero `LinearVelocity`, discarding whatever ahoy did this step |
//!
//! Writing `Transform` in that last slot is exactly how `ladder::climb` moves
//! the body too, and for the same reason: nothing here calls avian or ahoy's
//! sweep, so whatever `run_kcc` computed that step (depenetration, ground
//! snap, gravity integration) is simply overwritten rather than fought. No
//! `CharacterController` field or `SpatialQueryFilter` trick gets this for
//! free — `air_move` only ever integrates *horizontal* wish velocity, so
//! vertical is gravity-plus-one-jump-impulse no matter how collision is
//! filtered, which rules out true flight without this bracket.
//!
//! **The overwrite has to be a real overwrite.** `Noclip::anchor` is `fly`'s
//! own tracked position — advanced only by its own computed offsets, never
//! read back from `Transform` — the same role `Climbing::height` plays for
//! the ladder. Reading `transform.translation` as the base to add an offset
//! to (an earlier, buggy version of this file did exactly that) silently
//! un-does the whole bracket: by the time `fly` runs, `Transform` already
//! *is* `run_kcc`'s collision-respecting result for the step, so adding to it
//! is just "ahoy's normal blocked-by-walls movement, plus a bit of extra
//! drift" — indistinguishable from a flight *speed boost*, not real noclip.
//! `anchor` resets to `None` on every frame `fly` doesn't drive the entity
//! (noclip off, or climbing) so the next active frame reseeds cleanly from
//! wherever the body actually is, rather than snapping back to a stale value.
//!
//! `ColliderDisabled` isn't needed for the player's own pass-through — the
//! anchor overwrite already guarantees that on its own, same as the ladder —
//! but it stops the player's live collider from shoving dynamic crates while
//! flying through them (the `carry.rs` precedent for "this body shouldn't
//! participate in collision resolution right now"). It does *not* stop
//! ahoy's own sweep from finding the *world's* (un-disabled) colliders as
//! obstacles — that's a separate concern from the pass-through fix above.
//!
//! The toggle is a raw key (`player::player_cursor_input`'s Tab/Escape
//! idiom), not routed through `bevy_enhanced_input` — it needs to keep
//! working with the pack open or the cursor released, neither of which this
//! debug tool should care about.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_ahoy::input::AccumulatedInput;
use bevy_ahoy::prelude::*;

use crate::config;
use crate::ladder::Climbing;

/// On the player. `active` is read by `stash_noclip_input`/`fly`; `wish` is
/// refreshed by `stash_noclip_input` every frame — `Vec2::ZERO` on any frame
/// nothing is held, the same "a missing value is a release" semantics
/// `ladder::Climbing::wish` relies on.
#[derive(Component)]
pub struct Noclip {
    pub active: bool,
    wish: Vec2,
    /// `fly`'s own authoritative position while actively flying — see the
    /// module doc's "the overwrite has to be a real overwrite" section.
    /// `None` means "reseed from the live `Transform` next active frame":
    /// true at spawn, right after toggling on, and right after a ladder
    /// climb hands control back.
    anchor: Option<Vec3>,
}

impl Noclip {
    pub fn new(active: bool) -> Self {
        Self {
            active,
            wish: Vec2::ZERO,
            anchor: None,
        }
    }
}

/// `N`, raw and unconditioned: flip noclip and its `ColliderDisabled` in
/// lock-step, whatever else has input focus.
pub fn toggle_noclip(
    keys: Res<ButtonInput<KeyCode>>,
    mut players: Query<(Entity, &mut Noclip)>,
    mut commands: Commands,
) {
    if !keys.just_pressed(KeyCode::KeyN) {
        return;
    }
    for (entity, mut noclip) in &mut players {
        noclip.active = !noclip.active;
        info!(
            "devtools: noclip {}",
            if noclip.active { "ON" } else { "OFF" }
        );
        let mut player = commands.entity(entity);
        if noclip.active {
            player.insert(ColliderDisabled);
        } else {
            player.remove::<ColliderDisabled>();
        }
    }
}

/// Move the frame's intent into `Noclip::wish` and clear `AccumulatedInput`
/// so ahoy's controller walks and jumps nowhere for a flying player. A no-op
/// while inactive — real movement passes straight through to ahoy.
///
/// `Has<Climbing>` as data, not a `Without` filter: a ladder grab (E) doesn't
/// check or care about `Noclip` — it's a fully independent system — so with
/// noclip on by default the two would otherwise both be live on the same
/// entity. Climbing wins, so this no-ops while climbing exactly as it does
/// while inactive, letting `ladder::stash_input`'s own `AccumulatedInput::take`
/// have the frame's real movement regardless of registration order between
/// the two modules' systems. (`fly` needs the query to keep matching a
/// climbing entity too, to clear its own `anchor` — see there; this system
/// mirrors that shape for consistency even though it has no anchor of its
/// own to clear.)
///
/// Same per-*frame* (not per fixed step) timing as `ladder::stash_input`, and
/// for the same reason: `AccumulatedInput` has a one-frame lifetime, so a
/// single `take` here feeds every substep of a multi-substep frame the same
/// intent.
pub fn stash_noclip_input(mut players: Query<(&mut AccumulatedInput, &mut Noclip, Has<Climbing>)>) {
    for (mut input, mut noclip, climbing) in &mut players {
        if !noclip.active || climbing {
            continue;
        }
        noclip.wish = input.last_movement.take().unwrap_or(Vec2::ZERO);
        input.jumped.take();
    }
}

/// The flight itself: advance `Noclip::anchor` and assert it onto `Transform`
/// every fixed step, after ahoy has run — the `ladder::climb` technique, done
/// properly this time (see the module doc's "the overwrite has to be a real
/// overwrite").
///
/// `wish.y` (forward/back) flies along the camera's full look direction,
/// pitch included, so looking up and holding W climbs; `wish.x` (strafe)
/// moves along the camera's horizontal right. Space/Ctrl add pure vertical
/// thrust on top, read as raw held keys — simpler than routing through
/// `Jump`/`Crouch`, which ahoy models as one-shot events, not held state.
///
/// `Has<Climbing>` as data, not a `Without` filter: `climb` already writes an
/// absolute `Transform` in this same slot while climbing, so `fly` must not
/// also touch `Transform` then (the original stacking bug) — but it still
/// needs to see the entity to clear `anchor`, or resuming after a climb would
/// snap back to wherever flight left off *before* the grab instead of
/// continuing from the post-climb position.
#[allow(clippy::type_complexity)]
pub fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&Transform, With<CharacterControllerCameraOf>>,
    mut players: Query<
        (
            &mut Transform,
            &mut LinearVelocity,
            &mut Noclip,
            &CharacterControllerCamera,
            Has<Climbing>,
        ),
        Without<CharacterControllerCameraOf>,
    >,
) {
    let dt = time.delta_secs();
    for (mut transform, mut velocity, mut noclip, camera_of, climbing) in &mut players {
        // Not driving this entity this step — inactive, or climbing has taken
        // over. Leave `Transform`/`LinearVelocity` alone (touching them here
        // would zero every fixed step's velocity and quietly kill gravity,
        // jumping and walking for good) and stale the anchor so the next
        // active frame reseeds from wherever the body actually ends up,
        // rather than resuming from a now-stale position.
        if !noclip.active || climbing {
            noclip.anchor = None;
            continue;
        }
        let Ok(camera) = cameras.get(camera_of.get()) else {
            continue;
        };
        velocity.0 = Vec3::ZERO;

        let mut delta = *camera.forward() * noclip.wish.y + *camera.right() * noclip.wish.x;
        if delta.length_squared() > 1.0 {
            delta = delta.normalize();
        }
        let mut offset = delta * config::NOCLIP_SPEED_MPS * dt;

        if keys.pressed(KeyCode::Space) {
            offset.y += config::NOCLIP_VERTICAL_MPS * dt;
        }
        if keys.pressed(KeyCode::ControlLeft) {
            offset.y -= config::NOCLIP_VERTICAL_MPS * dt;
        }

        // The anchor, never `Transform`, is the base — this is what makes the
        // write immune to whatever `run_kcc` did to `Transform` this step
        // (its own gravity/ground-snap/depenetration against real geometry).
        let base = noclip.anchor.unwrap_or(transform.translation);
        let new_pos = base + offset;
        noclip.anchor = Some(new_pos);
        transform.translation = new_pos;
    }
}
