//! The `bevy_game_bits::inventory` grid inventory, on its own.
//!
//! A Deus Ex-style board of large cells holding axis-aligned rectangular
//! items. Click one to inspect it, hold and drag to move it with a live
//! green/red placement preview, Tab to show and hide the window. It exists
//! to show what the library actually needs — a camera, an item catalogue,
//! and one `spawn_item` call per item — and to be the thing you copy when
//! starting a new project.
//!
//! Everything specific to *this* inventory lives here: the loadout, the
//! keybinding, and the mapping from `InventoryAction` to sounds.

mod audio;

use bevy::prelude::*;
use bevy_game_bits::inventory::prelude::*;

use crate::audio::{AudioPlugin, PlaySfx, Sfx};

const CLEAR_COLOR: Color = Color::srgb(0.08, 0.08, 0.1);

/// The starting loadout and its initial layout. Host data — the library
/// never sees a catalogue, only the items handed to `spawn_item`.
const STARTING_LOADOUT: &[(InventoryItem, UVec2)] = &[
    (
        InventoryItem {
            name: "Hazard Suit",
            description: "Full-body NBC suit. Keeps out gas, radiation, and worse.",
            size: UVec2::new(2, 2),
            color: Color::srgb(0.75, 0.55, 0.1),
        },
        UVec2::new(0, 0),
    ),
    (
        InventoryItem {
            name: "Medkit",
            description: "Field trauma kit. Patches you up over a few seconds.",
            size: UVec2::new(1, 1),
            color: Color::srgb(0.85, 0.85, 0.85),
        },
        UVec2::new(2, 0),
    ),
    (
        InventoryItem {
            name: "Pistol",
            description: "A worn 10mm sidearm. Low damage, but you can always find ammo for it.",
            size: UVec2::new(1, 1),
            color: Color::srgb(0.55, 0.55, 0.6),
        },
        UVec2::new(3, 0),
    ),
    (
        InventoryItem {
            name: "Multitool",
            description: "Cracks simple electronic locks. One tool, no ammo.",
            size: UVec2::new(1, 1),
            color: Color::srgb(0.4, 0.4, 0.7),
        },
        UVec2::new(4, 0),
    ),
    (
        InventoryItem {
            name: "Bio Cell",
            description: "Bioelectric cell. Powers augmentations.",
            size: UVec2::new(1, 1),
            color: Color::srgb(0.2, 0.7, 0.7),
        },
        UVec2::new(5, 0),
    ),
    (
        InventoryItem {
            name: "Lockpick",
            description: "Fragile, but free. Better than nothing for a mechanical lock.",
            size: UVec2::new(1, 1),
            color: Color::srgb(0.75, 0.7, 0.35),
        },
        UVec2::new(6, 0),
    ),
    (
        InventoryItem {
            name: "Crowbar",
            description: "A trusty length of steel. Good for prying crates — and skulls.",
            size: UVec2::new(2, 1),
            color: Color::srgb(0.45, 0.4, 0.4),
        },
        UVec2::new(2, 1),
    ),
    (
        InventoryItem {
            name: "Sniper Rifle",
            description: "Bolt-action, deadly at range. Takes up a full row of pack space.",
            size: UVec2::new(4, 1),
            color: Color::srgb(0.35, 0.45, 0.3),
        },
        UVec2::new(0, 4),
    ),
    (
        InventoryItem {
            name: "GEP Gun",
            description: "Guided Explosive Projectile launcher. Devastating, and it shows: four\nwide, two deep.",
            size: UVec2::new(4, 2),
            color: Color::srgb(0.6, 0.35, 0.15),
        },
        UVec2::new(0, 5),
    ),
];

fn main() {
    App::new()
        .insert_resource(ClearColor(CLEAR_COLOR))
        .add_plugins(DefaultPlugins)
        // Bare — the defaults already are this example's 7x8 board.
        .add_plugins(InventoryPlugin::default())
        .add_plugins(AudioPlugin)
        .add_systems(Startup, (spawn_camera, spawn_loadout.after(InventorySet::Setup)))
        .add_systems(Update, (toggle_inventory, play_inventory_sfx))
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// The whole of putting items on the board: one `spawn_item` call each.
fn spawn_loadout(
    mut commands: Commands,
    ui: Res<InventoryUi>,
    mut grid: ResMut<InventoryGrid>,
    config: Res<InventoryConfig>,
    theme: Res<InventoryTheme>,
) {
    for (item, origin) in STARTING_LOADOUT {
        spawn_item(&mut commands, ui.board, &mut grid, &config, &theme, *item, *origin);
    }
}

/// Tab flips the window. The library owns the open/closed state; which key
/// opens it is this example's business — same shape as `008-colony`'s
/// `toggle_pause`.
fn toggle_inventory(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<InventoryWindowState>>,
    mut next: ResMut<NextState<InventoryWindowState>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        next.set(match state.get() {
            InventoryWindowState::Open => InventoryWindowState::Closed,
            InventoryWindowState::Closed => InventoryWindowState::Open,
        });
    }
}

/// `InventoryAction` -> this example's four sounds. The library fires the
/// facts; picking sounds for them is a game's decision.
fn play_inventory_sfx(mut actions: MessageReader<InventoryAction>, mut sfx: MessageWriter<PlaySfx>) {
    for action in actions.read() {
        let sound = match action {
            InventoryAction::Selected { .. } => Sfx::Select,
            InventoryAction::PickedUp { .. } => Sfx::PickUp,
            InventoryAction::Dropped { .. } => Sfx::Drop,
            InventoryAction::Rejected { .. } => Sfx::Invalid,
        };
        sfx.write(PlaySfx(sound));
    }
}
