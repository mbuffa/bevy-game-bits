//! Crosshair, interaction prompt, and inventory line.

use bevy::prelude::*;

use crate::interact::InteractionFocus;
use crate::pickup::Inventory;

#[derive(Component)]
pub struct PromptText;

#[derive(Component)]
pub struct InventoryText;

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
}

pub fn update_prompt(focus: Res<InteractionFocus>, mut prompts: Query<&mut Text, With<PromptText>>) {
    for mut text in &mut prompts {
        text.0 = focus.0.as_ref().map(|(_, prompt)| prompt.clone()).unwrap_or_default();
    }
}

pub fn update_inventory(inventory: Res<Inventory>, mut texts: Query<&mut Text, With<InventoryText>>) {
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
