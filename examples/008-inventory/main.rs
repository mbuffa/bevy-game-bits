//! The `bevy_game_bits::inventory` grid inventory, on its own.
//!
//! A Deus Ex-style board of large cells holding axis-aligned rectangular
//! items. Click one to inspect it, hold and drag to move it with a live
//! green/red placement preview, Tab to show and hide the bag. It exists to
//! show what the library actually needs — a camera, an item catalogue, and a
//! `spawn_item` call per item — and to be the thing you copy when starting a
//! new project.
//!
//! It also exercises the runtime API: `G` loots a random item into the first
//! free spot (spilling over into a second "Stash" board when the bag is
//! full), `+`/`-` grow and shrink the bag a row at a time, and everything
//! routes `InventoryAction` to a sound.
//!
//! Items drag **between** the two boards — pick one up on the stash and drop
//! it on the bag, or the reverse. Double-clicking an item is the fast version
//! of the same move: `link_boards` points each board's `InventoryTransferTarget`
//! at the other one, so a double-click sends the item straight across. `R`
//! toggles whether the stash will exchange items at all (`InventoryAccess::transfers`
//! — the stand-in for a "is the player next to the crate?" check a 3D game
//! would run every frame, and the same switch both dragging and double-click
//! transfer obey); `T` toggles `interactive`, which also stops rearranging
//! (and double-clicking) the stash's own contents.
//!
//! Everything specific to *this* inventory lives here: the loadout, the loot
//! table, the keybindings, and the mapping from `InventoryAction` to sounds.

mod audio;

use bevy::prelude::*;
use bevy_game_bits::inventory::prelude::*;

use crate::audio::{AudioPlugin, PlaySfx, Sfx};

const CLEAR_COLOR: Color = Color::srgb(0.08, 0.08, 0.1);

/// The starting loadout and its initial layout. Host data — the library
/// never sees a catalogue, only the items handed to `spawn_item`. A known
/// layout like this is exactly what `spawn_item` is for; runtime pickups go
/// through [`InventoryCommands`] instead (see `loot`).
const STARTING_LOADOUT: &[(InventoryItem, UVec2)] = &[
    (
        InventoryItem::borrowed(
            "Hazard Suit",
            "Full-body NBC suit. Keeps out gas, radiation, and worse.",
            UVec2::new(2, 2),
            Color::srgb(0.75, 0.55, 0.1),
        ),
        UVec2::new(0, 0),
    ),
    (
        InventoryItem::borrowed(
            "Medkit",
            "Field trauma kit. Patches you up over a few seconds.",
            UVec2::new(1, 1),
            Color::srgb(0.85, 0.85, 0.85),
        ),
        UVec2::new(2, 0),
    ),
    (
        InventoryItem::borrowed(
            "Pistol",
            "A worn 10mm sidearm. Low damage, but you can always find ammo for it.",
            UVec2::new(1, 1),
            Color::srgb(0.55, 0.55, 0.6),
        ),
        UVec2::new(3, 0),
    ),
    (
        InventoryItem::borrowed(
            "Multitool",
            "Cracks simple electronic locks. One tool, no ammo.",
            UVec2::new(1, 1),
            Color::srgb(0.4, 0.4, 0.7),
        ),
        UVec2::new(4, 0),
    ),
    (
        InventoryItem::borrowed(
            "Crowbar",
            "A trusty length of steel. Good for prying crates — and skulls.",
            UVec2::new(2, 1),
            Color::srgb(0.45, 0.4, 0.4),
        ),
        UVec2::new(2, 1),
    ),
    (
        InventoryItem::borrowed(
            "Sniper Rifle",
            "Bolt-action, deadly at range. Takes up a full row of pack space.",
            UVec2::new(4, 1),
            Color::srgb(0.35, 0.45, 0.3),
        ),
        UVec2::new(0, 4),
    ),
    (
        InventoryItem::borrowed(
            "GEP Gun",
            "Guided Explosive Projectile launcher. Devastating, and it shows: four\nwide, two deep.",
            UVec2::new(4, 2),
            Color::srgb(0.6, 0.35, 0.15),
        ),
        UVec2::new(0, 5),
    ),
];

/// What `G` pulls from. Runtime data — no fixed origin, the inventory picks
/// the spot.
const LOOT_TABLE: &[InventoryItem] = &[
    InventoryItem::borrowed(
        "Bio Cell",
        "Bioelectric cell. Powers augmentations.",
        UVec2::new(1, 1),
        Color::srgb(0.2, 0.7, 0.7),
    ),
    InventoryItem::borrowed(
        "Lockpick",
        "Fragile, but free. Better than nothing.",
        UVec2::new(1, 1),
        Color::srgb(0.75, 0.7, 0.35),
    ),
    InventoryItem::borrowed(
        "Frag Grenade",
        "Pull pin, count, throw. Two cells.",
        UVec2::new(1, 2),
        Color::srgb(0.3, 0.5, 0.3),
    ),
    InventoryItem::borrowed(
        "Ammo Box",
        "Rifle rounds, boxed.",
        UVec2::new(2, 1),
        Color::srgb(0.6, 0.5, 0.2),
    ),
    InventoryItem::borrowed(
        "Rebreather",
        "A few minutes of clean air underwater.",
        UVec2::new(2, 2),
        Color::srgb(0.35, 0.55, 0.7),
    ),
];

/// The stash starts with a couple of things in it, so dragging an item
/// *out* of the stash and onto the bag works from the first frame.
const STASH_LOADOUT: &[(InventoryItem, UVec2)] = &[
    (
        InventoryItem::borrowed(
            "Soy Food",
            "A ration bar. Calories, no flavor. Better than starving.",
            UVec2::new(1, 1),
            Color::srgb(0.6, 0.55, 0.3),
        ),
        UVec2::new(0, 0),
    ),
    (
        InventoryItem::borrowed(
            "Scrambler",
            "Turns one hostile bot friendly for a while. Bulky kit.",
            UVec2::new(2, 1),
            Color::srgb(0.3, 0.5, 0.55),
        ),
        UVec2::new(0, 1),
    ),
];

/// The bag's size bounds for `+`/`-`.
const BAG_MIN_ROWS: u32 = 4;
const BAG_MAX_ROWS: u32 = 12;

/// A tiny spillover board, spawned by the host — not the plugin — to show
/// that boards are per-entity and a game can have as many as it likes.
#[derive(Resource, Deref)]
struct StashBoard(Entity);

/// Rotates through [`LOOT_TABLE`] on each `G`.
#[derive(Resource, Default)]
struct LootCursor(usize);

fn main() {
    App::new()
        .insert_resource(ClearColor(CLEAR_COLOR))
        .insert_resource(LootCursor::default())
        .add_plugins(DefaultPlugins)
        // Bare — the defaults are this example's 7x8 top-left bag.
        .add_plugins(InventoryPlugin::default())
        .add_plugins(AudioPlugin)
        .add_systems(
            Startup,
            (
                spawn_camera,
                spawn_stash.in_set(InventorySet::Setup),
                fill_boards.after(InventorySet::Setup),
                link_boards.after(InventorySet::Setup),
            ),
        )
        .add_systems(
            Update,
            (
                toggle_bag,
                toggle_stash_access,
                loot,
                resize_bag,
                spill_to_stash,
                play_inventory_sfx,
            ),
        )
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// A second board, host-spawned (not by the plugin), anchored bottom-right
/// with its panel on the left. Catches `G` overflow, and items drag freely
/// between it and the bag unless `R` cuts it off.
fn spawn_stash(mut commands: Commands) {
    let stash = spawn_inventory(
        &mut commands,
        InventoryBoardSpec {
            config: InventoryConfig {
                cols: 5,
                rows: 5,
                title: Some("STASH".into()),
                ..default()
            },
            layout: InventoryLayout {
                horizontal: JustifyContent::FlexEnd,
                vertical: AlignItems::FlexEnd,
                panel_side: PanelSide::Left,
                ..default()
            },
            ..default()
        },
    );
    commands.insert_resource(StashBoard(stash));
}

/// The whole of putting the starting items on both boards: one `spawn_item`
/// call each, straight onto the right grid. A known layout is what
/// `spawn_item` is for; runtime pickups go through [`InventoryCommands`].
fn fill_boards(
    mut commands: Commands,
    bag: Res<DefaultInventoryBoard>,
    stash: Res<StashBoard>,
    mut boards: Query<
        (&mut InventoryGrid, &InventoryConfig, &InventoryTheme),
        With<InventoryBoard>,
    >,
) {
    for (board, loadout) in [(**bag, STARTING_LOADOUT), (**stash, STASH_LOADOUT)] {
        let Ok((mut grid, config, theme)) = boards.get_mut(board) else {
            continue;
        };
        for (item, origin) in loadout {
            spawn_item(
                &mut commands,
                board,
                &mut grid,
                config,
                theme,
                item.clone(),
                *origin,
            );
        }
    }
}

/// The one line of host wiring [`InventoryTransferTarget`] asks for: point
/// each board at the other, so a double-click sends an item across. A 3D game
/// would rewrite this every frame from "which container is the player
/// standing at"; here `R`/`T` already stand in for that reach check, and the
/// library refuses a transfer to a *closed* board on its own, so a fixed
/// pairing set once at startup is enough.
fn link_boards(
    bag: Res<DefaultInventoryBoard>,
    stash: Res<StashBoard>,
    mut targets: Query<&mut InventoryTransferTarget>,
) {
    if let Ok(mut target) = targets.get_mut(**bag) {
        target.0 = Some(**stash);
    }
    if let Ok(mut target) = targets.get_mut(**stash) {
        target.0 = Some(**bag);
    }
}

/// Tab flips the bag's window. The library owns the open/closed flag as a
/// component; which key toggles it is this example's business.
fn toggle_bag(
    keys: Res<ButtonInput<KeyCode>>,
    bag: Res<DefaultInventoryBoard>,
    mut windows: Query<&mut InventoryWindow>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        if let Ok(mut window) = windows.get_mut(**bag) {
            window.open = !window.open;
        }
    }
}

/// `G` — the runtime add path. The inventory finds the spot or refuses;
/// `spill_to_stash` handles the refusal.
fn loot(
    keys: Res<ButtonInput<KeyCode>>,
    bag: Res<DefaultInventoryBoard>,
    mut cursor: ResMut<LootCursor>,
    mut inv: InventoryCommands,
) {
    if !keys.just_pressed(KeyCode::KeyG) {
        return;
    }
    let item = LOOT_TABLE[cursor.0 % LOOT_TABLE.len()].clone();
    cursor.0 += 1;
    inv.add(**bag, item);
}

/// When the bag rejects an add, try the stash before giving up.
fn spill_to_stash(
    mut actions: MessageReader<InventoryAction>,
    bag: Res<DefaultInventoryBoard>,
    stash: Res<StashBoard>,
    mut add: MessageWriter<AddItem>,
) {
    for action in actions.read() {
        if let InventoryAction::AddRejected { board, item } = action {
            if *board == **bag {
                add.write(AddItem {
                    board: **stash,
                    item: item.clone(),
                    origin: None,
                });
            }
        }
    }
}

/// `R` / `T` stand in for a 3D game's per-frame reach check on the stash:
/// `R` flips whether it will exchange items with the bag at all, `T` whether
/// it responds to the pointer at all (its own contents included).
fn toggle_stash_access(
    keys: Res<ButtonInput<KeyCode>>,
    stash: Res<StashBoard>,
    mut access: Query<&mut InventoryAccess>,
) {
    let Ok(mut access) = access.get_mut(**stash) else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyR) {
        access.transfers = !access.transfers;
        info!("stash transfers: {}", access.transfers);
    }
    if keys.just_pressed(KeyCode::KeyT) {
        access.interactive = !access.interactive;
        info!("stash interactive: {}", access.interactive);
    }
}

/// `+` / `-` grow and shrink the bag by a row, within bounds. A shrink that
/// strands an item relocates it, or evicts it (see `play_inventory_sfx`).
fn resize_bag(
    keys: Res<ButtonInput<KeyCode>>,
    bag: Res<DefaultInventoryBoard>,
    configs: Query<&InventoryConfig>,
    mut inv: InventoryCommands,
) {
    let Ok(config) = configs.get(**bag) else {
        return;
    };
    let grow = keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd);
    let shrink = keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract);
    let rows = match (grow, shrink) {
        (true, false) => (config.rows + 1).min(BAG_MAX_ROWS),
        (false, true) => config.rows.saturating_sub(1).max(BAG_MIN_ROWS),
        _ => return,
    };
    if rows != config.rows {
        inv.resize(**bag, config.cols, rows);
    }
}

/// `InventoryAction` -> this example's four sounds. The library fires the
/// facts; picking sounds for them is a game's decision.
fn play_inventory_sfx(
    mut actions: MessageReader<InventoryAction>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    for action in actions.read() {
        let sound = match action {
            InventoryAction::Selected { .. } | InventoryAction::Activated { .. } => Sfx::Select,
            InventoryAction::PickedUp { .. } => Sfx::PickUp,
            InventoryAction::Dropped { .. }
            | InventoryAction::Transferred { .. }
            | InventoryAction::Added { .. }
            | InventoryAction::Removed { .. } => Sfx::Drop,
            InventoryAction::Rejected { .. }
            | InventoryAction::AddRejected { .. }
            | InventoryAction::Evicted { .. } => Sfx::Invalid,
            InventoryAction::Resized { .. } => Sfx::Select,
        };
        sfx.write(PlaySfx(sound));
    }
}
