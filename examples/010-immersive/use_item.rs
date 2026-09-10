//! Phase 16: "use the active item" on LMB.
//!
//! The demo's payoff verb — the lockpick opens the locked corridor door (and
//! is **consumed** doing it), the crowbar breaks a wooden crate. The active
//! item is whatever the quickbar's [`ActiveSlot`] points at
//! (`bevy_game_bits::inventory::quickbar`); slot selection and the viewmodel
//! live elsewhere (`inventory::quickbar::select_slot`, `crate::viewmodel`).

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_game_bits::inventory::prelude::*;

use crate::breakable::Breakable;
use crate::classes::Interactable;
use crate::config;
use crate::door::DoorSwing;
use crate::input;
use crate::interact::InteractionFocus;
use crate::pickup::{ItemKind, PlayerPack};

/// LMB: apply the active quickbar item to whatever the crosshair is on. Silent
/// no-op if the hands are free, nothing is focused, or the pairing doesn't
/// mean anything — the same shape as every `On<Interacted>` consumer.
#[allow(clippy::too_many_arguments)]
pub fn fire_use(
    _trigger: On<Fire<input::Use>>,
    focus: Res<InteractionFocus>,
    pack: Res<PlayerPack>,
    boards: Query<(&Quickbar, &ActiveSlot)>,
    kinds: Query<&ItemKind>,
    mut doors: Query<(&mut DoorSwing, &mut Interactable)>,
    mut breakables: Query<&mut Breakable>,
    mut inventory: InventoryCommands,
) {
    let Ok((quickbar, active)) = boards.get(**pack) else {
        return;
    };
    let Some(slot) = active.0 else {
        return;
    };
    let Some(item) = quickbar.slots.get(slot).copied().flatten() else {
        return;
    };
    let Ok(kind) = kinds.get(item) else {
        return;
    };
    let Some((target, _)) = focus.0 else {
        return;
    };

    match kind.0 {
        "lockpick" => {
            if let Ok((mut swing, mut interactable)) = doors.get_mut(target) {
                if swing.locked {
                    swing.locked = false; // `door::sync_lock_plates` turns the plate green
                    interactable.prompt = config::DOOR_PROMPT_OPEN.to_string();
                    inventory.remove(item); // one use — `quickbar::prune_and_assign_slots` frees the slot
                    info!("use_item: picked the lock on door {target}");
                }
            }
        }
        "crowbar" => {
            if let Ok(mut breakable) = breakables.get_mut(target) {
                breakable.health -= config::CROWBAR_DAMAGE;
                info!(
                    "use_item: crowbar hit breakable {target} ({:.0} health left)",
                    breakable.health.max(0.0)
                );
            }
        }
        _ => {}
    }
}
