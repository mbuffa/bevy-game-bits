//! Dev-tooling debug window, toggled with backtick: a tabbed panel of live
//! internals and dev switches. Jobs lists the Director's job pool; Weather
//! holds the rain and heavy-wind toggles. A new tab = a `DebugTab` variant, a content node
//! in `setup`, and a system keeping that content fresh.

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::construction::{Battery, Bed, Blueprint, SolarPanel, ToDemolish, Turbine};
use crate::crops::Crop;
use crate::daynight::{GameClock, ShowSkyBodies};
use crate::director::{AssignedTo, Job, JobKind, JobPriority, Stuck};
use crate::fire::{self, FireMap, ScorchMap};
use crate::flora::BerryBush;
use crate::items::ItemStack;
use crate::map::{self, ShowSeams};
use crate::power::{PowerConsumer, PowerGrid};
use crate::temperature::ShowTemperatureOverlay;
use crate::units::Pawn;
use crate::weather::{ShowAltitudeWindOverlay, ShowFuelOverlay, ShowWindOverlay, Weather, Wind};
use crate::zones::{ActiveTool, Tool};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugTab {
    #[default]
    Jobs,
    Weather,
    Fire,
    Build,
    Sky,
    Power,
    Terrain,
    Time,
}

impl DebugTab {
    /// Tab-bar order; new tabs only need an entry here plus their content.
    pub const ALL: [DebugTab; 8] = [
        DebugTab::Jobs,
        DebugTab::Weather,
        DebugTab::Fire,
        DebugTab::Build,
        DebugTab::Sky,
        DebugTab::Power,
        DebugTab::Terrain,
        DebugTab::Time,
    ];

    fn label(&self) -> &'static str {
        match self {
            DebugTab::Jobs => "Jobs",
            DebugTab::Weather => "Weather",
            DebugTab::Fire => "Fire",
            DebugTab::Build => "Build",
            DebugTab::Sky => "Sky",
            DebugTab::Power => "Power",
            DebugTab::Terrain => "Terrain",
            DebugTab::Time => "Time",
        }
    }
}

/// Which tab's content is showing.
#[derive(Resource, Default)]
pub struct ActiveDebugTab(pub DebugTab);

/// The window root; its `Visibility` is what backtick flips.
#[derive(Component)]
pub struct DebugWindow;

/// A tab-bar button selecting its tab.
#[derive(Component)]
pub struct DebugTabButton(DebugTab);

/// A content column, shown only while its tab is active.
#[derive(Component)]
pub struct DebugTabContent(DebugTab);

/// The Jobs tab's multiline text.
#[derive(Component)]
pub struct JobsTabText;

/// The Weather tab's rain on/off switch (and its label text).
#[derive(Component)]
pub struct RainToggleButton;

#[derive(Component)]
pub struct RainToggleText;

/// The Weather tab's Spring↔Winter season switch (and its label text).
#[derive(Component)]
pub struct SeasonButton;

#[derive(Component)]
pub struct SeasonText;

/// The Weather tab's heavy-wind switch (and its label text).
#[derive(Component)]
pub struct WindToggleButton;

#[derive(Component)]
pub struct WindToggleText;

/// The Weather tab's wind-direction shifter: each press turns the desired
/// heading a quarter turn (the wind then swings there smoothly).
#[derive(Component)]
pub struct WindShiftButton;

/// The Weather tab's wind-shadow overlay switch (and its label text).
#[derive(Component)]
pub struct WindOverlayButton;

#[derive(Component)]
pub struct WindOverlayText;

/// The Weather tab's altitude-wind overlay switch (and its label text) —
/// shows where turbine rotors are sheltered, distinct from the ground
/// `WindOverlayButton` above it.
#[derive(Component)]
pub struct AltitudeWindOverlayButton;

#[derive(Component)]
pub struct AltitudeWindOverlayText;

/// The Weather tab's fuel overlay switch (and its label text).
#[derive(Component)]
pub struct FuelOverlayButton;

#[derive(Component)]
pub struct FuelOverlayText;

/// The Weather tab's temperature overlay switch (and its label text).
#[derive(Component)]
pub struct TempOverlayButton;

#[derive(Component)]
pub struct TempOverlayText;

/// The Terrain tab's water-level nudgers: each press moves every water
/// body's level one `WATER_LEVEL_STEP` up (`raise`) or down.
#[derive(Component)]
pub struct WaterLevelButton {
    raise: bool,
}

/// The live "Water level: …" readout above the nudgers.
#[derive(Component)]
pub struct WaterLevelText;

/// The Fire tab's ignite-tool armer (and its label text).
#[derive(Component)]
pub struct IgniteToolButton;

#[derive(Component)]
pub struct IgniteToolText;

/// The Fire tab's put-everything-out button.
#[derive(Component)]
pub struct ExtinguishAllButton;

/// The Build tab's demolish-tool armer (and its label text): destroys any
/// built Wall/Door/Turbine/SolarPanel on the cells it's dragged over.
#[derive(Component)]
pub struct DemolishToolButton;

#[derive(Component)]
pub struct DemolishToolText;

/// The Sky tab's sun/moon-in-main-view switch (and its label text).
#[derive(Component)]
pub struct SkyBodiesToggleButton;

#[derive(Component)]
pub struct SkyBodiesToggleText;

/// The Power tab's multiline text.
#[derive(Component)]
pub struct PowerTabText;

/// The Terrain tab's show-seams switch (and its label text): reveals the
/// dirt/grass/shore checkerboard, off by default (the shipped look is
/// seamless).
#[derive(Component)]
pub struct ShowSeamsButton;

#[derive(Component)]
pub struct ShowSeamsText;

/// The Time tab's +1h/-1h nudgers: each press jumps the `GameClock` a whole
/// hour forward (`forward`) or back.
#[derive(Component)]
pub struct HourButton {
    forward: bool,
}

/// The live "Day N — HH:MM" readout above the Time tab's nudgers.
#[derive(Component)]
pub struct HourClockText;

pub fn setup(mut commands: Commands) {
    commands
        .spawn((
            // No fixed `width`: the window hugs its widest row (currently
            // the tab bar) instead of a hand-tuned constant that overflows
            // every time a tab is added. The Jobs/Power tabs' free-form text
            // pins its own wrap width below, so it can't blow that back out.
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PANEL_MARGIN),
                top: Val::Percent(25.0),
                padding: UiRect::all(Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..default()
            },
            // Opaque, not the translucent `PANEL_BACKGROUND`: sits over busy
            // map content and needs to stay legible (see the pawn tab
            // window, `ui.rs`).
            BackgroundColor(PANEL_BACKGROUND_OPAQUE),
            // Hover target for the `click_select` click-through guard.
            Interaction::default(),
            Visibility::Hidden,
            // Always render above HUD panels, regardless of spawn order.
            GlobalZIndex(Z_DEVTOOLS),
            DebugWindow,
        ))
        .with_children(|window| {
            window.spawn((
                Text::new("Debug"),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(PANEL_TEXT),
            ));
            window
                .spawn(Node {
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|tabs| {
                    for tab in DebugTab::ALL {
                        tabs.spawn((
                            Button,
                            DebugTabButton(tab),
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new(tab.label()),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                    }
                });
            window
                .spawn((
                    Node {
                        width: Val::Px(DEBUG_WINDOW_WIDTH),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    },
                    DebugTabContent(DebugTab::Jobs),
                ))
                .with_child((
                    Text::new(""),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                    JobsTabText,
                ));
            window
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    DebugTabContent(DebugTab::Weather),
                ))
                .with_children(|content| {
                    content
                        .spawn((
                            Button,
                            RainToggleButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Rain: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            RainToggleText,
                        ));
                    content
                        .spawn((
                            Button,
                            SeasonButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Season: Spring"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            SeasonText,
                        ));
                    content
                        .spawn((
                            Button,
                            WindToggleButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Heavy Wind: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            WindToggleText,
                        ));
                    content
                        .spawn((
                            Button,
                            WindShiftButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Shift Wind +90\u{b0}"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                    content
                        .spawn((
                            Button,
                            WindOverlayButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Wind overlay: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            WindOverlayText,
                        ));
                    content
                        .spawn((
                            Button,
                            AltitudeWindOverlayButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Altitude wind: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            AltitudeWindOverlayText,
                        ));
                    content
                        .spawn((
                            Button,
                            FuelOverlayButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Fuel overlay: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            FuelOverlayText,
                        ));
                    content
                        .spawn((
                            Button,
                            TempOverlayButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Temp overlay: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            TempOverlayText,
                        ));
                });
            window
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    DebugTabContent(DebugTab::Fire),
                ))
                .with_children(|content| {
                    content
                        .spawn((
                            Button,
                            IgniteToolButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Ignite tool"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            IgniteToolText,
                        ));
                    content
                        .spawn((
                            Button,
                            ExtinguishAllButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Extinguish all"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                });
            window
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    DebugTabContent(DebugTab::Build),
                ))
                .with_children(|content| {
                    content
                        .spawn((
                            Button,
                            DemolishToolButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Demolish tool"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            DemolishToolText,
                        ));
                });
            window
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    DebugTabContent(DebugTab::Sky),
                ))
                .with_children(|content| {
                    content
                        .spawn((
                            Button,
                            SkyBodiesToggleButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Show Sun/Moon: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            SkyBodiesToggleText,
                        ));
                });
            window
                .spawn((
                    Node {
                        width: Val::Px(DEBUG_WINDOW_WIDTH),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    },
                    DebugTabContent(DebugTab::Power),
                ))
                .with_child((
                    Text::new(""),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                    PowerTabText,
                ));
            window
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    DebugTabContent(DebugTab::Terrain),
                ))
                .with_children(|content| {
                    content
                        .spawn((
                            Button,
                            ShowSeamsButton,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("Show seams: Off"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            ShowSeamsText,
                        ));
                    content.spawn((
                        Text::new("Water level: \u{2014}"),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(PANEL_TEXT),
                        WaterLevelText,
                    ));
                    for raise in [true, false] {
                        content
                            .spawn((
                                Button,
                                WaterLevelButton { raise },
                                Node {
                                    padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(BUTTON_BACKGROUND),
                            ))
                            .with_child((
                                Text::new(format!(
                                    "{} water {}{}",
                                    if raise { "Raise" } else { "Lower" },
                                    if raise { "+" } else { "-" },
                                    WATER_LEVEL_STEP,
                                )),
                                TextFont {
                                    font_size: 12.0,
                                    ..default()
                                },
                                TextColor(PANEL_TEXT),
                            ));
                    }
                });
            window
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    DebugTabContent(DebugTab::Time),
                ))
                .with_children(|content| {
                    content.spawn((
                        Text::new("Day 1 \u{2014} 06:00"),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(PANEL_TEXT),
                        HourClockText,
                    ));
                    for forward in [true, false] {
                        content
                            .spawn((
                                Button,
                                HourButton { forward },
                                Node {
                                    padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(BUTTON_BACKGROUND),
                            ))
                            .with_child((
                                Text::new(if forward { "+1 hour" } else { "-1 hour" }),
                                TextFont {
                                    font_size: 12.0,
                                    ..default()
                                },
                                TextColor(PANEL_TEXT),
                            ));
                    }
                });
        });
}

/// Backtick shows/hides the window. Not gated on `SimState` — inspecting a
/// paused world is the whole point of a debug window.
pub fn toggle_debug_window(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&mut Visibility, With<DebugWindow>>,
) {
    if keys.just_pressed(DEBUG_WINDOW_KEY) {
        let mut visibility = window.into_inner();
        *visibility = match *visibility {
            Visibility::Hidden => Visibility::Visible,
            _ => Visibility::Hidden,
        };
    }
}

/// Tab-bar clicks switch the active tab.
pub fn run_tab_buttons(
    interactions: Query<(&Interaction, &DebugTabButton), Changed<Interaction>>,
    mut active: ResMut<ActiveDebugTab>,
) {
    for (interaction, button) in &interactions {
        if *interaction == Interaction::Pressed {
            active.0 = button.0;
        }
    }
}

/// Show only the active tab's content; tint its button as pressed.
pub fn sync_tabs(
    active: Res<ActiveDebugTab>,
    mut contents: Query<(&DebugTabContent, &mut Node)>,
    mut buttons: Query<(&DebugTabButton, &mut BackgroundColor)>,
) {
    for (content, mut node) in &mut contents {
        node.display = if content.0 == active.0 {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (button, mut background) in &mut buttons {
        background.0 = if button.0 == active.0 {
            BUTTON_PRESSED
        } else {
            BUTTON_BACKGROUND
        };
    }
}

/// The rain switch flips the `Weather` resource; everything else (the
/// particle effect, humidity) follows it from there.
pub fn run_rain_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<RainToggleButton>)>,
    mut weather: ResMut<Weather>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            weather.rain = !weather.rain;
            info!("weather: rain {}", if weather.rain { "on" } else { "off" });
        }
    }
}

/// Keep the switch's label and tint on the live weather.
pub fn sync_rain_toggle(
    weather: Res<Weather>,
    button: Single<&mut BackgroundColor, With<RainToggleButton>>,
    text: Single<&mut Text, With<RainToggleText>>,
) {
    button.into_inner().0 = if weather.rain {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if weather.rain {
        "Rain: On"
    } else {
        "Rain: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// The season switch flips `ActiveSeason` between Spring and Winter; the
/// ambient, precipitation form, freeze level and melt all follow from
/// there. Outdoor temperatures snap to the new ambient on the next tick.
pub fn run_season_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<SeasonButton>)>,
    mut season: ResMut<crate::temperature::ActiveSeason>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            season.0 = season.0.toggled();
            info!("weather: season {}", season.0.label());
        }
    }
}

/// Keep the switch's label and tint on the live season (tinted in winter —
/// it's the "cold mode" switch).
pub fn sync_season_toggle(
    season: Res<crate::temperature::ActiveSeason>,
    button: Single<&mut BackgroundColor, With<SeasonButton>>,
    text: Single<&mut Text, With<SeasonText>>,
) {
    let winter = season.0 == crate::temperature::Season::Winter;
    button.into_inner().0 = if winter {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = format!("Season: {}", season.0.label());
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// The water-level nudgers move every body's level one step; the surface,
/// nav-cost and algae followers react to the `WaterMap` change on their
/// own. Only touches the `ResMut` on an actual press, so quiet frames
/// don't trip the followers' change detection.
pub fn run_water_level_buttons(
    interactions: Query<(&Interaction, &WaterLevelButton), Changed<Interaction>>,
    mut water: ResMut<map::WaterMap>,
) {
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let step = if button.raise {
            WATER_LEVEL_STEP
        } else {
            -WATER_LEVEL_STEP
        };
        water.shift_all_levels(step);
        info!("water: levels now {:?}", water.levels());
    }
}

/// Keep the water-level readout on the live bodies. They all move in
/// lockstep today (`shift_all_levels`), so the first body's level speaks
/// for every one of them.
pub fn sync_water_level_label(
    water: Res<map::WaterMap>,
    text: Single<&mut Text, With<WaterLevelText>>,
) {
    let wanted = match water.levels().first() {
        Some(level) => format!(
            "Water level: {level:.2} ({} {})",
            water.body_count(),
            if water.body_count() == 1 {
                "body"
            } else {
                "bodies"
            }
        ),
        None => String::from("Water level: \u{2014}"),
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// The Time tab's +1h/-1h buttons jump the `GameClock` a whole hour; the
/// day/night lighting, sky arc, and clock chip all follow it from there.
pub fn run_hour_buttons(
    interactions: Query<(&Interaction, &HourButton), Changed<Interaction>>,
    mut clock: ResMut<GameClock>,
) {
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        clock.shift_hours(if button.forward { 1 } else { -1 });
        info!("clock: {}", clock.clock_label());
    }
}

/// Keep the Time tab's readout on the live clock.
pub fn update_hour_clock_text(
    window: Single<&Visibility, With<DebugWindow>>,
    clock: Res<GameClock>,
    text: Single<&mut Text, With<HourClockText>>,
) {
    if **window == Visibility::Hidden {
        return;
    }
    let wanted = clock.clock_label();
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// The heavy-wind switch flips `Wind.heavy`; the gust system, sway shader,
/// and rain slant all follow the resource from there.
pub fn run_wind_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<WindToggleButton>)>,
    mut wind: ResMut<Wind>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            wind.heavy = !wind.heavy;
            info!(
                "weather: heavy wind {}",
                if wind.heavy { "on" } else { "off" }
            );
        }
    }
}

/// Keep the heavy-wind switch's label and tint on the live wind.
pub fn sync_wind_toggle(
    wind: Res<Wind>,
    button: Single<&mut BackgroundColor, With<WindToggleButton>>,
    text: Single<&mut Text, With<WindToggleText>>,
) {
    button.into_inner().0 = if wind.heavy {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if wind.heavy {
        "Heavy Wind: On"
    } else {
        "Heavy Wind: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// Each press turns the wind's desired heading a quarter turn; the actual
/// direction swings there smoothly at `WIND_TURN_SPEED` (and the turbines
/// chase it slower still). A momentary action, not a toggle — no sync
/// system.
pub fn run_wind_shift(
    interactions: Query<&Interaction, (Changed<Interaction>, With<WindShiftButton>)>,
    mut wind: ResMut<Wind>,
) {
    use std::f32::consts::TAU;
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            wind.shift_offset = (wind.shift_offset + WIND_SHIFT_STEP).rem_euclid(TAU);
            info!(
                "weather: wind shifted, desired offset now {:.0} degrees",
                wind.shift_offset.to_degrees()
            );
        }
    }
}

/// The wind-overlay switch flips `ShowWindOverlay`; the overlay reconciler
/// follows it from there.
pub fn run_wind_overlay_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<WindOverlayButton>)>,
    mut show: ResMut<ShowWindOverlay>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            show.0 = !show.0;
            info!(
                "weather: wind overlay {}",
                if show.0 { "on" } else { "off" }
            );
        }
    }
}

/// Keep the wind-overlay switch's label and tint on the live setting.
pub fn sync_wind_overlay_toggle(
    show: Res<ShowWindOverlay>,
    button: Single<&mut BackgroundColor, With<WindOverlayButton>>,
    text: Single<&mut Text, With<WindOverlayText>>,
) {
    button.into_inner().0 = if show.0 {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if show.0 {
        "Wind overlay: On"
    } else {
        "Wind overlay: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// The altitude-wind-overlay switch flips `ShowAltitudeWindOverlay`; the
/// overlay reconciler follows it from there. The `run_wind_overlay_toggle`
/// model, one band over.
pub fn run_altitude_wind_overlay_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<AltitudeWindOverlayButton>)>,
    mut show: ResMut<ShowAltitudeWindOverlay>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            show.0 = !show.0;
            info!(
                "weather: altitude wind overlay {}",
                if show.0 { "on" } else { "off" }
            );
        }
    }
}

/// Keep the altitude-wind-overlay switch's label and tint on the live
/// setting.
pub fn sync_altitude_wind_overlay_toggle(
    show: Res<ShowAltitudeWindOverlay>,
    button: Single<&mut BackgroundColor, With<AltitudeWindOverlayButton>>,
    text: Single<&mut Text, With<AltitudeWindOverlayText>>,
) {
    button.into_inner().0 = if show.0 {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if show.0 {
        "Altitude wind: On"
    } else {
        "Altitude wind: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// The fuel-overlay switch flips `ShowFuelOverlay`; the overlay reconciler
/// follows it from there.
pub fn run_fuel_overlay_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<FuelOverlayButton>)>,
    mut show: ResMut<ShowFuelOverlay>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            show.0 = !show.0;
            info!(
                "weather: fuel overlay {}",
                if show.0 { "on" } else { "off" }
            );
        }
    }
}

/// Keep the fuel-overlay switch's label and tint on the live setting.
pub fn sync_fuel_overlay_toggle(
    show: Res<ShowFuelOverlay>,
    button: Single<&mut BackgroundColor, With<FuelOverlayButton>>,
    text: Single<&mut Text, With<FuelOverlayText>>,
) {
    button.into_inner().0 = if show.0 {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if show.0 {
        "Fuel overlay: On"
    } else {
        "Fuel overlay: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// The temperature-overlay switch flips `ShowTemperatureOverlay`; the
/// overlay reconciler follows it from there.
pub fn run_temp_overlay_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<TempOverlayButton>)>,
    mut show: ResMut<ShowTemperatureOverlay>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            show.0 = !show.0;
            info!("temperature: overlay {}", if show.0 { "on" } else { "off" });
        }
    }
}

/// Keep the temperature-overlay switch's label and tint on the live setting.
pub fn sync_temp_overlay_toggle(
    show: Res<ShowTemperatureOverlay>,
    button: Single<&mut BackgroundColor, With<TempOverlayButton>>,
    text: Single<&mut Text, With<TempOverlayText>>,
) {
    button.into_inner().0 = if show.0 {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if show.0 {
        "Temp overlay: On"
    } else {
        "Temp overlay: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// Arm the Ignite devtool (a `zones::Tool`, so dragging paints fire like
/// the roof stamps paint policy); right-click/Escape disarms it as usual.
pub fn run_ignite_button(
    interactions: Query<&Interaction, (Changed<Interaction>, With<IgniteToolButton>)>,
    mut tool: ResMut<ActiveTool>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            tool.0 = Some(Tool::Ignite);
            info!("fire: ignite tool armed");
        }
    }
}

/// Keep the ignite button's tint/label on whether the tool is armed.
pub fn sync_ignite_button(
    tool: Res<ActiveTool>,
    button: Single<&mut BackgroundColor, With<IgniteToolButton>>,
    text: Single<&mut Text, With<IgniteToolText>>,
) {
    let armed = tool.0 == Some(Tool::Ignite);
    button.into_inner().0 = if armed {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if armed {
        "Ignite tool: armed"
    } else {
        "Ignite tool"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// Put out every fire on the map (partial scorch stays, like a rain douse)
/// and drop any un-lit ignition requests.
pub fn run_extinguish_all(
    interactions: Query<&Interaction, (Changed<Interaction>, With<ExtinguishAllButton>)>,
    mut fire: ResMut<FireMap>,
    mut scorch: ResMut<ScorchMap>,
    mut burns: MessageWriter<fire::BurnCommand>,
) {
    for interaction in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let burning = fire.burning_cells();
        info!("fire: extinguishing {} burning cells", burning.len());
        for (cell, _) in burning {
            if let Some(completeness) = fire.extinguish(cell) {
                let level = scorch.get(cell);
                scorch.set(cell, level.max(fire::extinguish_scorch(completeness)));
                // The devtool is just a very sudden rain: plants on
                // mostly-burnt cells still die.
                burns.write(fire::BurnCommand { cell, completeness });
            }
        }
        fire.pending.clear();
    }
}

/// Arm the Demolish devtool (a `zones::Tool`, same drag-and-commit shape as
/// Ignite): dragging over cells despawns any built Wall/Door/Turbine/
/// SolarPanel there (a multi-cell building demolishes fully from any one of
/// its footprint cells) and reverts terrain/nav/roof state — see
/// `zones::drag_zone_tool`'s `Tool::Demolish` arm and
/// `construction::demolish_at`.
pub fn run_demolish_button(
    interactions: Query<&Interaction, (Changed<Interaction>, With<DemolishToolButton>)>,
    mut tool: ResMut<ActiveTool>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            tool.0 = Some(Tool::Demolish);
            info!("construction: demolish tool armed");
        }
    }
}

/// Keep the demolish button's tint/label on whether the tool is armed.
pub fn sync_demolish_button(
    tool: Res<ActiveTool>,
    button: Single<&mut BackgroundColor, With<DemolishToolButton>>,
    text: Single<&mut Text, With<DemolishToolText>>,
) {
    let armed = tool.0 == Some(Tool::Demolish);
    button.into_inner().0 = if armed {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if armed {
        "Demolish tool: armed"
    } else {
        "Demolish tool"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// The Sky tab's switch flips `ShowSkyBodies`; `daynight::sync_sky_body_visibility`
/// follows it from there.
pub fn run_sky_bodies_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<SkyBodiesToggleButton>)>,
    mut show: ResMut<ShowSkyBodies>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            show.0 = !show.0;
            info!(
                "daynight: sun/moon in main view {}",
                if show.0 { "on" } else { "off" }
            );
        }
    }
}

/// Keep the switch's label and tint on the live setting.
pub fn sync_sky_bodies_toggle(
    show: Res<ShowSkyBodies>,
    button: Single<&mut BackgroundColor, With<SkyBodiesToggleButton>>,
    text: Single<&mut Text, With<SkyBodiesToggleText>>,
) {
    button.into_inner().0 = if show.0 {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if show.0 {
        "Show Sun/Moon: On"
    } else {
        "Show Sun/Moon: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}

/// Rebuild the Jobs list while the window is open: one line per job, sorted
/// by (priority, entity) so the list doesn't shuffle between frames.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update_jobs_tab(
    window: Single<&Visibility, With<DebugWindow>>,
    jobs: Query<(
        Entity,
        &Job,
        &JobPriority,
        Option<&AssignedTo>,
        Option<&Stuck>,
    )>,
    pawns: Query<&Name, With<Pawn>>,
    stacks: Query<(&ItemStack, &GridCoords)>,
    plants: Query<(&GridCoords, Has<BerryBush>), Or<(With<BerryBush>, With<Crop>)>>,
    trees: Query<&GridCoords, With<crate::trees::Tree>>,
    sites: Query<&GridCoords, With<Blueprint>>,
    demolish_sites: Query<&GridCoords, With<ToDemolish>>,
    beds: Query<&GridCoords, With<Bed>>,
    text: Single<&mut Text, With<JobsTabText>>,
) {
    if **window == Visibility::Hidden {
        return;
    }

    let mut rows: Vec<(u32, Entity, String)> = jobs
        .iter()
        .map(|(entity, job, priority, assigned, stuck)| {
            let what = match job.kind {
                JobKind::Harvest { target } => match plants.get(target) {
                    Ok((grid, true)) => format!("Harvest bush ({}, {})", grid.x, grid.y),
                    Ok((grid, false)) => format!("Harvest crop ({}, {})", grid.x, grid.y),
                    Err(_) => "Harvest ?".to_string(),
                },
                JobKind::Haul { stack } | JobKind::Merge { stack } => {
                    let verb = if matches!(job.kind, JobKind::Haul { .. }) {
                        "Haul"
                    } else {
                        "Merge"
                    };
                    match stacks.get(stack) {
                        Ok((item, grid)) => format!(
                            "{verb} {} {} ({}, {})",
                            item.amount,
                            item.kind.label(),
                            grid.x,
                            grid.y
                        ),
                        Err(_) => format!("{verb} ?"),
                    }
                }
                JobKind::Walk { goal } => format!("Walk to ({}, {})", goal.x, goal.y),
                JobKind::Farming { cell, plant } => {
                    format!("Farm {} ({}, {})", plant.label(), cell.x, cell.y)
                }
                JobKind::Cut { target } => match trees.get(target) {
                    Ok(grid) => format!("Cut tree ({}, {})", grid.x, grid.y),
                    Err(_) => "Cut ?".to_string(),
                },
                JobKind::Supply => "Supply build sites".to_string(),
                JobKind::Build { site } => match sites.get(site) {
                    Ok(grid) => format!("Build ({}, {})", grid.x, grid.y),
                    Err(_) => "Build ?".to_string(),
                },
                JobKind::Demolish { site } => match demolish_sites.get(site) {
                    Ok(grid) => format!("Demolish ({}, {})", grid.x, grid.y),
                    Err(_) => "Demolish ?".to_string(),
                },
                JobKind::BuildRoof { cell } => format!("Roof ({}, {})", cell.x, cell.y),
                JobKind::RemoveRoof { cell } => format!("Unroof ({}, {})", cell.x, cell.y),
                JobKind::Sleep { bed } => match beds.get(bed) {
                    Ok(grid) => format!("Sleep in bed ({}, {})", grid.x, grid.y),
                    Err(_) => "Sleep ?".to_string(),
                },
            };
            let who = if let Some(stuck) = stuck {
                format!("stuck {:.1}s", stuck.0.remaining_secs())
            } else if let Some(AssignedTo(pawn)) = assigned {
                pawns
                    .get(*pawn)
                    .map(|name| name.to_string())
                    .unwrap_or_else(|_| "?".to_string())
            } else {
                "free".to_string()
            };
            (
                priority.0,
                entity,
                format!("[{}] {what} -> {who}", priority.0),
            )
        })
        .collect();
    rows.sort_by_key(|(priority, entity, _)| (*priority, *entity));

    let total = rows.len();
    let mut lines: Vec<String> = Vec::with_capacity(total.min(DEBUG_JOBS_MAX_LINES) + 2);
    lines.push(format!("{total} job(s)"));
    lines.extend(
        rows.iter()
            .take(DEBUG_JOBS_MAX_LINES)
            .map(|(_, _, line)| line.clone()),
    );
    if total > DEBUG_JOBS_MAX_LINES {
        lines.push(format!("... and {} more", total - DEBUG_JOBS_MAX_LINES));
    }
    let content = lines.join("\n");

    let mut text = text.into_inner();
    if text.0 != content {
        text.0 = content;
    }
}

/// Live grid readout while the window is open: totals plus a count of every
/// producer/storage/consumer kind, so a stalled turbine or an empty battery
/// is obvious without selecting each one.
pub fn update_power_tab(
    window: Single<&Visibility, With<DebugWindow>>,
    grid: Res<PowerGrid>,
    turbines: Query<(), With<Turbine>>,
    solar_panels: Query<(), With<SolarPanel>>,
    batteries: Query<(), With<Battery>>,
    consumers: Query<(), With<PowerConsumer>>,
    text: Single<&mut Text, With<PowerTabText>>,
) {
    if **window == Visibility::Hidden {
        return;
    }
    let content = format!(
        "Production: {:.1}/s\nConsumption: {:.1}/s\nCoverage: {:.0}%\nStored: {:.0} / {:.0}\nTurbines: {}  Solar: {}  Batteries: {}  Lights: {}",
        grid.production,
        grid.consumption,
        grid.powered_fraction * 100.0,
        grid.stored,
        grid.capacity,
        turbines.iter().count(),
        solar_panels.iter().count(),
        batteries.iter().count(),
        consumers.iter().count(),
    );
    let mut text = text.into_inner();
    if text.0 != content {
        text.0 = content;
    }
}

/// The Terrain tab's show-seams switch flips `ShowSeams`; the dirt,
/// grass-cover, and shore-wedge reconcilers follow it from there.
pub fn run_seamless_toggle(
    interactions: Query<&Interaction, (Changed<Interaction>, With<ShowSeamsButton>)>,
    mut show: ResMut<ShowSeams>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            show.0 = !show.0;
            info!("terrain: show seams {}", if show.0 { "on" } else { "off" });
        }
    }
}

/// Keep the show-seams switch's label and tint on the live setting.
pub fn sync_seamless_toggle(
    show: Res<ShowSeams>,
    button: Single<&mut BackgroundColor, With<ShowSeamsButton>>,
    text: Single<&mut Text, With<ShowSeamsText>>,
) {
    button.into_inner().0 = if show.0 {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let wanted = if show.0 {
        "Show seams: On"
    } else {
        "Show seams: Off"
    };
    let mut text = text.into_inner();
    if text.0 != wanted {
        text.0 = wanted.to_string();
    }
}
