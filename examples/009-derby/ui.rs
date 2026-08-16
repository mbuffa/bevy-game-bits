//! A minimal top-left HUD (speed, throttle, steer) plus a top-right
//! vehicle-state panel: one line per destructible part, colored from green
//! through yellow to red as its health drains. Also owns the match-wide UI:
//! a floating name-plus-health nameplate over every car, the "3, 2, 1, GO!"
//! countdown, and the results panel at the end of a match.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::camera::FollowCamera;
use crate::config::*;
use crate::damage::{Damage, Immunity};
use crate::game::{Countdown, Driver, PlayerClass, Wrecked};
use bevy_game_bits::vehicle::Vehicle;
use crate::vehicle::Player;

#[derive(Component)]
pub struct SpeedText;

/// One line of the vehicle-state panel, by index: 0 engine, 1 lights,
/// 2 windshield, 3 fenders, 4 spoiler, 5 ram bar, 6 immunity countdown.
#[derive(Component)]
pub struct DamageLine(pub usize);

/// The full-screen parent both countdown texts sit in — `show_countdown_text`
/// /`hide_countdown_text` toggle *this* node's `Visibility`, which both
/// children inherit.
#[derive(Component)]
pub struct CountdownRoot;

/// The big centered "3, 2, 1, GO!" text — see `update_countdown_text`.
#[derive(Component)]
pub struct CountdownText;

/// The small "C — Truck"/"C — Buggy" hint under the countdown — the
/// stand-in for a car-selection screen (SPEC.md Phase 19) until one exists.
/// Updated alongside `CountdownText` by `update_countdown_text`.
#[derive(Component)]
pub struct ClassHintText;

pub fn setup_hud(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                top: Val::Px(12.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("speed: 0.0 m/s"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                SpeedText,
            ));
        });

    // Vehicle-state panel, top-right: one colored line per part.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                padding: UiRect::all(Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        ))
        .with_children(|panel| {
            for index in 0..7 {
                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    DamageLine(index),
                ));
            }
        });

    // Countdown, dead-centered in a full-screen node, with the small class
    // hint stacked under the big number — hidden outside
    // `MatchState::Countdown` (`show_countdown_text`/`hide_countdown_text`
    // toggle `CountdownRoot`; both children inherit its `Visibility`).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
            Visibility::Hidden,
            CountdownRoot,
        ))
        .with_children(|root| {
            root.spawn((
                CountdownText,
                Text::new(""),
                TextFont {
                    font_size: 64.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            root.spawn((
                ClassHintText,
                Text::new(""),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.75, 0.75)),
            ));
        });
}

/// Green at full health through yellow to red at zero.
fn health_color(health: f32) -> Color {
    let health = health.clamp(0.0, 1.0);
    if health > 0.5 {
        Color::srgb(0.9 - 0.6 * (health - 0.5) * 2.0, 0.85, 0.25)
    } else {
        Color::srgb(0.9, 0.25 + 0.6 * health * 2.0, 0.2)
    }
}

/// Fills the vehicle-state panel from the player's `Damage`. Arms yield a
/// `(String, Color)` directly rather than a health fed through
/// `health_color` at the end — the immunity row is a countdown, not a
/// health, so it needs its own color.
pub fn update_damage_hud(
    // `Option<Single<..>>`: the car model loads asynchronously (`model.rs`),
    // so there's no `Player` yet for the first several frames — the panel
    // just keeps its spawn-time blank lines (`setup_hud`) until then.
    car: Option<Single<(&Damage, Option<&Immunity>), With<Player>>>,
    mut lines: Query<(&DamageLine, &mut Text, &mut TextColor)>,
) {
    let Some(car) = car else { return };
    let (car, immunity) = car.into_inner();
    let pct = |h: f32| (h * 100.0).round();
    for (line, mut text, mut color) in &mut lines {
        // Per-side arrays index 0 = −X = driver's right, 1 = +X = left.
        let (label, line_color) = match line.0 {
            0 => (
                format!("engine     {:.0}%", pct(car.engine)),
                health_color(car.engine),
            ),
            1 => (
                format!(
                    "lights     L {:.0}% / R {:.0}%",
                    pct(car.lights[1]),
                    pct(car.lights[0])
                ),
                health_color(car.lights[0].min(car.lights[1])),
            ),
            2 => (
                if car.windshield <= 0.0 {
                    "windshield SHATTERED".to_string()
                } else {
                    format!("windshield {:.0}%", pct(car.windshield))
                },
                health_color(car.windshield),
            ),
            3 => (
                format!(
                    "fenders    L {} / R {}",
                    fender_label(car.fenders[1]),
                    fender_label(car.fenders[0])
                ),
                health_color(car.fenders[0].min(car.fenders[1])),
            ),
            4 => (
                if car.spoiler <= 0.0 {
                    "spoiler    DETACHED".to_string()
                } else {
                    format!("spoiler    {:.0}%", pct(car.spoiler))
                },
                health_color(car.spoiler),
            ),
            5 => (
                if car.shield <= 0.0 {
                    "ram bar    TORN OFF".to_string()
                } else {
                    format!("ram bar    {:.0}%", pct(car.shield))
                },
                health_color(car.shield),
            ),
            _ => match immunity {
                Some(immunity) => (
                    format!("IMMUNE     {:.1}s", immunity.0.remaining_secs()),
                    Color::srgb(0.4, 0.85, 1.0),
                ),
                // A blank, fully transparent row rather than a "none"
                // label — the panel keeps its height, the line just isn't
                // there when it doesn't apply.
                None => (String::new(), Color::NONE),
            },
        };
        text.0 = label;
        color.0 = line_color;
    }
}

fn fender_label(health: f32) -> String {
    if health <= 0.0 {
        "GONE".to_string()
    } else {
        format!("{:.0}%", (health * 100.0).round())
    }
}

pub fn update_hud(
    time: Res<Time>,
    // `Option<Single<..>>`: the car model loads asynchronously (`model.rs`),
    // so there's no `Player` yet for the first several frames.
    car: Option<Single<(&Vehicle, &LinearVelocity, &Transform), With<Player>>>,
    mut text: Single<&mut Text, With<SpeedText>>,
    // Acceleration is derived from the velocity delta between frames and
    // exponentially smoothed — the raw per-frame value is unreadable noise.
    mut previous_velocity: Local<Vec3>,
    mut smoothed_accel: Local<f32>,
) {
    let Some(car) = car else { return };
    let (vehicle, linvel, transform) = car.into_inner();
    let dt = time.delta_secs();
    if dt > 0.0 {
        let accel = (linvel.0 - *previous_velocity) / dt;
        let forward = transform.rotation * Vec3::Z;
        let ease = (HUD_ACCEL_SMOOTH_RATE * dt).clamp(0.0, 1.0);
        *smoothed_accel += (accel.dot(forward) - *smoothed_accel) * ease;
        *previous_velocity = linvel.0;
    }
    let speed = linvel.0.length();
    text.0 = format!(
        "speed: {speed:.1} m/s ({:.0} km/h)\naccel: {:+.1} m/s²\nthrottle: {:+.2}  steer: {:+.2}",
        speed * 3.6,
        *smoothed_accel,
        vehicle.throttle,
        vehicle.steer
    );
    if vehicle.handbrake {
        text.0.push_str("\nHANDBRAKE");
    }
}

// --- Countdown -----------------------------------------------------------

pub fn show_countdown_text(mut root: Single<&mut Visibility, With<CountdownRoot>>) {
    **root = Visibility::Visible;
}

pub fn hide_countdown_text(mut root: Single<&mut Visibility, With<CountdownRoot>>) {
    **root = Visibility::Hidden;
}

/// "3, 2, 1, GO!" plus the "C — Truck"/"C — Buggy" hint underneath — reads
/// the remaining time directly off the shared `Countdown` timer rather than
/// keeping its own, so it can never drift from the moment
/// `game::tick_countdown` actually flips the state, and reads `PlayerClass`
/// fresh every tick so pressing `C` updates the hint immediately.
pub fn update_countdown_text(
    countdown: Res<Countdown>,
    player_class: Res<PlayerClass>,
    mut number: Single<&mut Text, (With<CountdownText>, Without<ClassHintText>)>,
    mut hint: Single<&mut Text, (With<ClassHintText>, Without<CountdownText>)>,
) {
    let remaining = countdown.0.remaining_secs();
    number.0 = if remaining <= 0.0 {
        "GO!".to_string()
    } else {
        format!("{}", remaining.ceil() as u32)
    };
    hint.0 = format!("C — {}", player_class.0.label());
}

// --- Nameplates ------------------------------------------------------------

/// A floating name + health bar above one car. Holds the child entities it
/// drives directly rather than walking `Children` every frame — `target` is
/// the car it tracks, `name_text`/`bar_fill` are its own two children.
#[derive(Component)]
pub struct Nameplate {
    target: Entity,
    name_text: Entity,
    bar_fill: Entity,
}

/// One nameplate per car, spawned reactively as `game::Driver` lands
/// (`vehicle::spawn_car` inserts it at spawn) — covers every car spawned by
/// every match, initial or rematch, without a dedicated startup path.
pub fn spawn_nameplates(mut commands: Commands, cars: Query<(Entity, &Driver, &CarClass), Added<Driver>>) {
    for (car, driver, class) in &cars {
        let mut name_text = Entity::PLACEHOLDER;
        let mut bar_fill = Entity::PLACEHOLDER;
        let plate = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(NAMEPLATE_WIDTH),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                Visibility::Hidden,
            ))
            .with_children(|plate| {
                name_text = plate
                    .spawn((
                        Text::new(format!("{} ({})", driver.name, class.label())),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(driver.color),
                        TextLayout::new_with_justify(Justify::Center),
                    ))
                    .id();
                plate
                    .spawn((
                        Node {
                            width: Val::Px(NAMEPLATE_BAR_WIDTH),
                            height: Val::Px(NAMEPLATE_BAR_HEIGHT),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                    ))
                    .with_children(|track| {
                        bar_fill = track
                            .spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::WHITE),
                            ))
                            .id();
                    });
            })
            .id();
        commands.entity(plate).insert(Nameplate { target: car, name_text, bar_fill });
    }
}

/// Projects each nameplate to screen space above its car and refreshes its
/// text/bar from that car's current `Damage`. A wrecked car's name greys
/// out and gains its final placement (`NAME · P3`); its bar drains to
/// empty. Follows the same `world_to_viewport` pattern as 008-colony's
/// `sync_pawn_labels`.
pub fn sync_nameplates(
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform), With<FollowCamera>>,
    cars: Query<(&GlobalTransform, &Damage, &Driver, &CarClass, Option<&Wrecked>)>,
    mut plates: Query<(Entity, &Nameplate, &mut Node, &mut Visibility)>,
    mut texts: Query<(&mut Text, &mut TextColor), Without<Nameplate>>,
    mut fills: Query<(&mut Node, &mut BackgroundColor), Without<Nameplate>>,
) {
    let (camera, camera_transform) = *camera;
    for (plate_entity, plate, mut node, mut visibility) in &mut plates {
        let Ok((transform, damage, driver, class, wrecked)) = cars.get(plate.target) else {
            commands.entity(plate_entity).despawn();
            continue;
        };
        let anchor = transform.translation() + Vec3::Y * NAMEPLATE_HEIGHT_OFFSET;
        let Ok(screen) = camera.world_to_viewport(camera_transform, anchor) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(screen.x - NAMEPLATE_WIDTH / 2.0);
        node.top = Val::Px(screen.y);
        *visibility = Visibility::Visible;

        if let Ok((mut text, mut color)) = texts.get_mut(plate.name_text) {
            text.0 = match wrecked {
                Some(w) => format!("{} ({}) · P{}", driver.name, class.label(), w.place),
                None => format!("{} ({})", driver.name, class.label()),
            };
            color.0 = if wrecked.is_some() { Color::srgb(0.55, 0.55, 0.55) } else { driver.color };
        }
        if let Ok((mut fill_node, mut fill_color)) = fills.get_mut(plate.bar_fill) {
            let condition = damage.condition();
            fill_node.width = Val::Percent((condition * 100.0).clamp(0.0, 100.0));
            fill_color.0 = health_color(condition);
        }
    }
}

// --- Results -----------------------------------------------------------------

/// The results panel spawned OnEnter(Over) — despawned OnExit(Over)
/// (`hide_results`), so it's rebuilt from scratch every match rather than
/// kept around and re-synced.
#[derive(Component)]
pub struct ResultsPanel;

fn place_label(place: usize) -> String {
    match place {
        1 => "1st".to_string(),
        2 => "2nd".to_string(),
        3 => "3rd".to_string(),
        other => format!("{other}th"),
    }
}

fn format_match_time(secs: f32) -> String {
    let secs = secs.max(0.0) as u32;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Builds the standings straight from the world: the one car without a
/// `Wrecked` is the survivor (place 1); everyone else sorts by the place
/// `game::check_wrecks` already resolved for them at the moment of their KO.
pub fn show_results(mut commands: Commands, cars: Query<(&Driver, &CarClass, Option<&Wrecked>), With<Vehicle>>) {
    let mut standings: Vec<(&Driver, &CarClass, Option<&Wrecked>)> = cars.iter().collect();
    standings.sort_by_key(|(_, _, wrecked)| wrecked.map_or(1, |w| w.place));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            ResultsPanel,
        ))
        .with_children(|screen| {
            screen
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(24.0)),
                        row_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("MATCH OVER"),
                        TextFont {
                            font_size: 28.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                    for (driver, class, wrecked) in &standings {
                        let status = match wrecked {
                            None => "survived".to_string(),
                            Some(w) => format!("wrecked {}", format_match_time(w.at)),
                        };
                        let place = wrecked.map_or(1, |w| w.place);
                        panel.spawn((
                            Text::new(format!("{}  {} ({})   {status}", place_label(place), driver.name, class.label())),
                            TextFont {
                                font_size: 18.0,
                                ..default()
                            },
                            TextColor(driver.color),
                        ));
                    }
                    panel.spawn((
                        Text::new("R — new match"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                    ));
                });
        });
}

pub fn hide_results(mut commands: Commands, panels: Query<Entity, With<ResultsPanel>>) {
    for entity in &panels {
        commands.entity(entity).despawn();
    }
}
