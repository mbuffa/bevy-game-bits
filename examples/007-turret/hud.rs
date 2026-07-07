use bevy::prelude::*;

use crate::config::*;
use crate::enemy::ActiveWave;
use crate::game::{BaseHealth, BuildTimer, CurrentWave, GamePhase, Materials, PlacementRejected};
use crate::waves::WAVES;

#[derive(Component)]
pub struct MaterialsText;

#[derive(Component)]
pub struct BaseHpText;

#[derive(Component)]
pub struct StatusText;

/// Marks the big centered Victory/Defeat banner.
#[derive(Component)]
pub struct Banner;

/// Runs while the materials counter blinks red after a rejected placement.
#[derive(Resource)]
pub struct RejectFlash(pub Timer);

pub const REJECT_FLASH_SECONDS: f32 = 0.5;

pub fn setup(mut commands: Commands) {
    let mut flash = Timer::from_seconds(REJECT_FLASH_SECONDS, TimerMode::Once);
    flash.tick(flash.duration()); // start expired: no flash until a rejection
    commands.insert_resource(RejectFlash(flash));

    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|column| {
            column.spawn((
                MaterialsText,
                Text::new(""),
                TextFont::from_font_size(18.0),
                TextColor(Color::WHITE),
            ));
            column.spawn((
                BaseHpText,
                Text::new(""),
                TextFont::from_font_size(18.0),
                TextColor(Color::WHITE),
            ));
            column.spawn((
                StatusText,
                Text::new(""),
                TextFont::from_font_size(18.0),
                TextColor(Color::WHITE),
            ));
        });
}

pub fn update_materials(
    time: Res<Time>,
    materials: Res<Materials>,
    mut rejected: MessageReader<PlacementRejected>,
    mut flash: ResMut<RejectFlash>,
    text: Single<(&mut Text, &mut TextColor), With<MaterialsText>>,
) {
    if rejected.read().next().is_some() {
        flash.0.reset();
    }
    flash.0.tick(time.delta());

    let (mut text, mut color) = text.into_inner();
    text.0 = format!("Materials: {}", materials.0);
    color.0 = if flash.0.is_finished() {
        Color::WHITE
    } else {
        Color::srgb(1.0, 0.25, 0.25)
    };
}

pub fn update_base_hp(
    base: Res<BaseHealth>,
    mut text: Single<&mut Text, With<BaseHpText>>,
) {
    text.0 = format!("Base HP: {}/{}", base.0, BASE_HP);
}

pub fn update_status(
    state: Res<State<GamePhase>>,
    current: Res<CurrentWave>,
    wave: Res<ActiveWave>,
    timer: Res<BuildTimer>,
    mut text: Single<&mut Text, With<StatusText>>,
) {
    // CurrentWave is 0-based and already advanced past the last cleared wave,
    // so it names the upcoming wave during Building and the live one during
    // WaveActive.
    let wave_label = (current.0 + 1).min(WAVES.len());

    text.0 = match state.get() {
        GamePhase::Building => format!(
            "Wave {}/{} - build phase: {:.0}s (Space to start)",
            wave_label,
            WAVES.len(),
            timer.0.remaining_secs().ceil(),
        ),
        GamePhase::WaveActive => format!(
            "Wave {}/{} - {} to resolve",
            wave_label,
            WAVES.len(),
            wave.spawned - wave.resolved + pending_spawns(&wave, current.0),
        ),
        GamePhase::Victory | GamePhase::Defeat => String::new(),
    };
}

fn pending_spawns(wave: &ActiveWave, current: usize) -> u32 {
    WAVES[current]
        .groups
        .iter()
        .zip(&wave.cursors)
        .map(|(group, cursor)| group.count - cursor)
        .sum()
}

/// Big centered end-screen banner; `DespawnOnExit` removes it on restart.
pub fn spawn_banner(phase: GamePhase) -> impl Fn(Commands) {
    move |mut commands: Commands| {
        let (message, color) = match phase {
            GamePhase::Victory => ("VICTORY", Color::srgb(0.4, 1.0, 0.5)),
            _ => ("DEFEAT", Color::srgb(1.0, 0.3, 0.3)),
        };

        commands
            .spawn((
                DespawnOnExit(phase),
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
            ))
            .with_children(|banner| {
                banner.spawn((
                    Banner,
                    Text::new(message),
                    TextFont::from_font_size(64.0),
                    TextColor(color),
                ));
                banner.spawn((
                    Text::new("Press R to restart"),
                    TextFont::from_font_size(20.0),
                    TextColor(Color::WHITE),
                ));
            });
    }
}
