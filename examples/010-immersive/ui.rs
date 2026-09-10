//! Crosshair, interaction prompt, the throw-charge slider, and a transient
//! notice line. The inventory readout is the quickbar strip now
//! (`bevy_game_bits::inventory::quickbar`), so there's no top-left item list.

use bevy::prelude::*;
use bevy_game_bits::inventory::prelude::InventoryWindow;

use crate::breakable::Breakable;
use crate::carry::{Carrying, ThrowCharge};
use crate::config;
use crate::door::DoorSwing;
use crate::interact::InteractionFocus;
use crate::ladder::Climbing;
use bevy_game_bits::inventory::prelude::{ActiveSlot, Quickbar};
use crate::pickup::{ItemKind, PackFullFlash, PlayerPack};

#[derive(Component)]
pub struct PromptText;

/// Transient top-centre line — currently just "pack is full".
#[derive(Component)]
pub struct NoticeText;

/// The centre-screen crosshair dot, hidden while a pack board is open.
#[derive(Component)]
pub struct Crosshair;

/// The throw-charge slider track under the crosshair; hidden unless a
/// `ThrowCharge` is live.
#[derive(Component)]
pub struct ChargeBar;

/// The filled portion of `ChargeBar`; its `Node::width` percent is the charge
/// ratio.
#[derive(Component)]
pub struct ChargeFill;

pub fn setup_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        PromptText,
        TextFont::from_font_size(20.0),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Percent(15.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::new_with_justify(Justify::Center),
    ));
    commands.spawn((
        Text::new(""),
        NoticeText,
        TextFont::from_font_size(18.0),
        TextColor(Color::srgb(1.0, 0.55, 0.3)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(12.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::new_with_justify(Justify::Center),
    ));
    commands.spawn((
        Crosshair,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(50.0),
            left: Val::Percent(50.0),
            width: Val::Px(4.0),
            height: Val::Px(4.0),
            margin: UiRect::all(Val::Px(-2.0)),
            ..default()
        },
        BackgroundColor(Color::WHITE),
    ));
    commands
        .spawn((
            ChargeBar,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(50.0),
                left: Val::Percent(50.0),
                width: Val::Px(120.0),
                height: Val::Px(6.0),
                margin: UiRect::new(Val::Px(-60.0), Val::ZERO, Val::Px(18.0), Val::ZERO),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.7)),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.4)),
            Visibility::Hidden,
        ))
        .with_child((
            ChargeFill,
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgb(1.0, 0.75, 0.2)),
        ));
}

/// Show the slider only while charging, and drive its fill from `ThrowCharge`.
pub fn update_charge_bar(
    charge: Query<&ThrowCharge>,
    mut bar: Query<&mut Visibility, With<ChargeBar>>,
    mut fill: Query<&mut Node, With<ChargeFill>>,
) {
    let ratio = charge
        .iter()
        .next()
        .map(|c| (c.0 / config::CARRY_CHARGE_SECS).clamp(0.0, 1.0));
    for mut visibility in &mut bar {
        *visibility = if ratio.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut node in &mut fill {
        node.width = Val::Percent(ratio.unwrap_or(0.0) * 100.0);
    }
}

/// The active quickbar item's catalogue key, if the hands aren't free.
fn active_kind<'a>(
    pack: &Res<PlayerPack>,
    boards: &Query<(&Quickbar, &ActiveSlot)>,
    kinds: &'a Query<&ItemKind>,
) -> Option<&'a str> {
    let (quickbar, active) = boards.get(***pack).ok()?;
    let item = quickbar.slots.get(active.0?).copied().flatten()?;
    kinds.get(item).ok().map(|k| k.0)
}

#[allow(clippy::too_many_arguments)]
pub fn update_prompt(
    focus: Res<InteractionFocus>,
    pack: Res<PlayerPack>,
    boards: Query<(&Quickbar, &ActiveSlot)>,
    kinds: Query<&ItemKind>,
    windows: Query<&InventoryWindow>,
    climbing: Query<(), With<Climbing>>,
    carrying: Query<(), With<Carrying>>,
    doors: Query<&DoorSwing>,
    breakables: Query<(), With<Breakable>>,
    mut prompts: Query<&mut Text, With<PromptText>>,
) {
    let held = active_kind(&pack, &boards, &kinds);

    let label = if windows.iter().any(|w| w.open) {
        // The strip and board are the whole UI while the pack is open.
        String::new()
    } else if !climbing.is_empty() {
        "W/S to climb · E or Space to let go".to_string()
    } else if !carrying.is_empty() {
        "RMB to place, hold RMB to throw".to_string()
    } else if let Some((target, prompt)) = focus.0.as_ref() {
        match held {
            Some("lockpick") if doors.get(*target).is_ok_and(|d| d.locked) => {
                format!("{prompt}  ·  LMB: pick the lock")
            }
            Some("crowbar") if breakables.contains(*target) => {
                format!("{prompt}  ·  LMB: pry it open")
            }
            _ => prompt.clone(),
        }
    } else {
        String::new()
    };
    for mut text in &mut prompts {
        if text.0 != label {
            text.0 = label.clone();
        }
    }
}

/// Hide the crosshair while a pack board is open (the prompt blanks itself in
/// `update_prompt`). Idempotent mirror.
pub fn sync_hud_visibility(
    windows: Query<&InventoryWindow>,
    mut crosshair: Query<&mut Visibility, With<Crosshair>>,
) {
    let wanted = if windows.iter().any(|w| w.open) {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut visibility in &mut crosshair {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Counts down [`PackFullFlash`] and shows [`config::PACK_FULL_MSG`] while it
/// runs.
pub fn update_notice(
    time: Res<Time>,
    mut flash: ResMut<PackFullFlash>,
    mut texts: Query<&mut Text, With<NoticeText>>,
) {
    if flash.0 > 0.0 {
        flash.0 = (flash.0 - time.delta_secs()).max(0.0);
    }
    let wanted = if flash.0 > 0.0 {
        config::PACK_FULL_MSG
    } else {
        ""
    };
    for mut text in &mut texts {
        if text.0 != wanted {
            text.0 = wanted.to_string();
        }
    }
}
