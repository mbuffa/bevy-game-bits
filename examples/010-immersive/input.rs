//! `bevy_enhanced_input` context and action bindings for the player.
//! `bevy_ahoy` provides the `Movement`/`Jump`/`Crouch`/`RotateCamera` action
//! types; `Interact` is ours, consumed by `interact.rs` (Phase 2).

use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::*;
// Both `bevy::prelude` (a picking event) and `bevy_enhanced_input::prelude`
// export `Press`; name the one we mean explicitly (an explicit import beats a
// glob), as the crate's own doc comment instructs.
use bevy_enhanced_input::prelude::Press;

use crate::config;

/// Marker component identifying the `bevy_enhanced_input` context the
/// player's actions are bound under. Registered via `add_input_context` in
/// `main.rs`.
#[derive(Component, Default)]
pub struct PlayerInput;

/// "Reach out and touch the thing under the crosshair": mount/dismount a
/// ladder, open a hinged door, flip a wall switch — `interact.rs` fires
/// `Interacted` at `InteractionFocus` on the press edge. Bound to **RMB or E**;
/// RMB is shared with `Grab`, harmlessly, because a grabbable class
/// (`PropCrate`/`ItemPickup`) never has an `Interacted` consumer and an
/// interactable one (`FuncLadder`/`PropDoor`/`FuncLightSwitch`) is never
/// liftable — the two press-edge events land on disjoint targets.
#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Interact;

/// RMB press edge: grab an aimed `PropCrate` to carry (`carry.rs`), or pocket
/// an aimed `ItemPickup` into the grid pack (`pickup.rs`). RMB while already
/// carrying does nothing — putting a crate down is `Throw` (LMB). RMB *also*
/// fires `Interact` (`interact.rs`); the two never hit the same target (see
/// that type's doc).
#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Grab;

/// LMB, hands free: use the active inventory item on whatever the crosshair is
/// on — consumed by `use_item.rs` (Phase 16). Edge-triggered like `Interact`.
/// Shares LMB with `Throw`; `fire_use` no-ops while carrying, so the two never
/// both apply.
#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Use;

/// LMB while carrying a crate: a tap places it straight ahead, a hold+release
/// throws it — the charge builds between `Start<Throw>` and `Complete<Throw>`
/// (`carry.rs`). Unconditioned like `Grab` so those edges bracket the hold.
#[derive(Debug, InputAction)]
#[action_output(bool)]
pub struct Throw;

/// `PlayerInput` plus every action binding, inserted onto the player entity
/// when it spawns. Movement/camera bindings mirror bevy_ahoy's own
/// `minimal.rs` example; `Interact` is appended for our own use-raycast.
pub fn player_input_bundle() -> impl Bundle {
    (
        PlayerInput,
        actions!(PlayerInput[
            (
                Action::<Movement>::new(),
                // Normalize the input vector.
                DeadZone::default(),
                Bindings::spawn((Cardinal::wasd_keys(), Axial::left_stick())),
            ),
            (
                // Deliberately unconditioned, like ahoy's own `minimal.rs`:
                // holding Space is meant to re-jump the frame you land, and
                // ahoy's jump buffer is fed by the every-frame `Fire<Jump>`.
                // Anything that wants a jump *press* (edge) instead — e.g.
                // `ladder::let_go_on_jump` — observes `Start<Jump>` rather than
                // adding a `Press` here, so ahoy's buffering keeps working.
                Action::<Jump>::new(),
                bindings![KeyCode::Space, GamepadButton::South],
            ),
            (
                Action::<Crouch>::new(),
                bindings![KeyCode::ControlLeft, GamepadButton::LeftTrigger2],
            ),
            (
                Action::<RotateCamera>::new(),
                Bindings::spawn((
                    Spawn((Binding::mouse_motion(), Scale::splat(config::MOUSE_SENSITIVITY))),
                    Axial::right_stick().with((Scale::splat(4.0), DeadZone::default())),
                )),
            ),
            (
                Action::<Interact>::new(),
                // Edge-triggered. Without a condition a bevy_enhanced_input
                // action is `Down`-like and its `Fire` event triggers *every
                // frame* the key is held — which turns any toggling
                // `Interacted` consumer (`ladder::attach_on_interact`'s
                // lock/unlock) into a coin flip. `door` only survives that by
                // being idempotent. The condition guards the RMB path just as
                // much as the E one.
                //
                // No `require_reset` here (unlike `Use`/`Throw`): RMB never
                // re-grabs the cursor or closes a board — only LMB does
                // (`player::player_cursor_input`) — so there's no
                // context-reactivation edge to hold this action inert against.
                Press::default(),
                bindings![KeyCode::KeyE, GamepadButton::West, MouseButton::Right],
            ),
            (
                // Unconditioned (`carry.rs` / `pickup.rs` act on the
                // `Start<Grab>` press edge, not `Fire<Grab>`), and kept that way
                // rather than `Press` to stay symmetric with `Throw`, whose
                // `Start`/`Complete` really do need to bracket a hold. Same
                // pair ahoy's unconditioned `Jump` uses.
                Action::<Grab>::new(),
                bindings![MouseButton::Right, GamepadButton::RightTrigger2],
            ),
            (
                // Edge-triggered, same reasoning as `Interact`: `use_item.rs`
                // toggles the door lock / damages a crate, so a per-frame `Fire`
                // would flip it back and forth for the length of the click.
                Action::<Use>::new(),
                Press::default(),
                // `require_reset`: the `PlayerInput` context is switched off
                // while the pack is open (`player::sync_cursor_mode`), and the
                // same left-click that closes a board or re-grabs the cursor
                // must not also fire `Use` the frame the context comes back.
                // This is bevy_enhanced_input's own answer — hold the action
                // inert until its inputs go idle again.
                ActionSettings {
                    require_reset: true,
                    ..default()
                },
                bindings![MouseButton::Left, GamepadButton::East],
            ),
            (
                // Unconditioned, like `Grab`: `carry.rs` arms a `ThrowCharge` on
                // `Start<Throw>` and places/throws on `Complete<Throw>`, with the
                // hold between as the charge. Shares LMB with `Use` — harmless,
                // because `start_throw`/`release_prop` only act while `Carrying`
                // and `fire_use` only acts while not. `require_reset` for the
                // same reason `Use` has it: LMB is also the cursor re-grab click.
                Action::<Throw>::new(),
                ActionSettings {
                    require_reset: true,
                    ..default()
                },
                bindings![MouseButton::Left, GamepadButton::East],
            ),
        ]),
    )
}
