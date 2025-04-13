use bevy::prelude::*;

pub const SCOREBOARD_TEXT_PADDING: Val = Val::Px(4.0);
pub const SCOREBOARD_FONT_SIZE: f32 = 48.0;

#[derive(Resource)]
pub struct Score(pub u32);

#[derive(Component)]
pub struct InstructionsText;

#[derive(Component)]
pub struct TitleText;

#[derive(Component)]
pub struct ScoreText;

#[derive(Component)]
pub struct GameOverText;

#[derive(Component)]
pub struct HitSpaceText;

#[derive(Resource)]
pub struct WindowSize(pub f32, pub f32);

pub fn update_score_text(score_text: Single<&mut Text, With<ScoreText>>, score: Res<Score>) {
    let mut text = score_text.into_inner();
    text.0 = score.0.to_string();
}

pub fn display_game_over_text(mut commands: Commands) {
    commands.spawn((
        Text2d::new("GAME OVER"),
        TextLayout::new_with_justify(JustifyText::Center),
        TextFont::from_font_size(48.0),
        GameOverText,
        InstructionsText,
        Transform::from_xyz(0.0, 0.0 + 48.0 + 16.0, 0.0),
    ));

    add_instructions_text(&mut commands)
}

pub fn add_instructions_text(commands: &mut Commands) {
    commands.spawn((
        Text2d::new("Hit Space to Play"),
        TextLayout::new_with_justify(JustifyText::Center),
        TextFont::from_font_size(16.0),
        HitSpaceText,
        InstructionsText,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}

pub fn maybe_hide_instructions_text(
    mut commands: Commands,
    ui_elements_query: Query<Entity, With<InstructionsText>>,
) {
    for entity in ui_elements_query.iter() {
        commands.entity(entity).despawn();
    }
}
