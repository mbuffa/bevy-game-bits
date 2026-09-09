//! Crosshair, interaction prompt, and inventory line.

use bevy::prelude::*;

use crate::carry::{Carrying, ThrowCharge};
use crate::config;
use crate::interact::InteractionFocus;
use crate::ladder::Climbing;
use crate::pickup::Inventory;

#[derive(Component)]
pub struct PromptText;

#[derive(Component)]
pub struct InventoryText;

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
        InventoryText,
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
    commands.spawn((
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

pub fn update_prompt(
    focus: Res<InteractionFocus>,
    climbing: Query<(), With<Climbing>>,
    carrying: Query<(), With<Carrying>>,
    mut prompts: Query<&mut Text, With<PromptText>>,
) {
    let label = if !climbing.is_empty() {
        "W/S to climb · E or Space to let go".to_string()
    } else if !carrying.is_empty() {
        "RMB to place, hold RMB to throw".to_string()
    } else {
        focus
            .0
            .as_ref()
            .map(|(_, prompt)| prompt.clone())
            .unwrap_or_default()
    };
    for mut text in &mut prompts {
        text.0 = label.clone();
    }
}

pub fn update_inventory(
    inventory: Res<Inventory>,
    mut texts: Query<&mut Text, With<InventoryText>>,
) {
    if !inventory.is_changed() {
        return;
    }
    for mut text in &mut texts {
        text.0 = if inventory.0.is_empty() {
            String::new()
        } else {
            format!("Inventory: {}", inventory.0.join(", "))
        };
    }
}
