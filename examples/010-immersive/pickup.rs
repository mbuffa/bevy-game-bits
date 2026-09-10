//! `ItemPickup` visuals + collection into the grid inventory.
//!
//! The world model and colour come from [`crate::items::CATALOGUE`] (keyed by
//! `ItemPickup::item`); an uncatalogued key falls back to the placeholder
//! emissive cube. Collecting one routes through
//! `bevy_game_bits::inventory` — [`InventoryCommands::add`] onto the player's
//! [`PlayerPack`] board, which auto-fills a quickbar slot
//! (`inventory::quickbar`). A pickup is only despawned once it's actually in
//! the pack: a full pack leaves the item in the world and flashes
//! [`PackFullFlash`].
//!
//! **RMB.** Collecting is on the [`Grab`] press edge (`Start<Grab>`) — the same
//! button that grabs a `PropCrate` in `carry.rs` — so "grab that object" is one
//! gesture whether you carry it or pocket it. RMB (and E) *also* fire
//! `Interacted` for doors / ladders / switches, but those classes aren't
//! grabbable and a pickup isn't interactable, so a press only ever does one
//! thing. The two `Start<Grab>` observers are likewise disjoint: `carry` acts
//! on a liftable `RigidBody::Dynamic`, this one on an [`ItemPickup`] (which is
//! `RigidBody::Static`).

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::Start;
use bevy_game_bits::inventory::prelude::*;

use crate::carry::Carrying;
use crate::classes::{Interactable, ItemPickup};
use crate::config;
use crate::input::Grab;
use crate::interact::InteractionFocus;
use crate::items::{self, ItemShape};

/// The board entity `main.rs::spawn_pack` built as the player's pack. Every
/// item the player carries lives on it; the quickbar strip is a row of
/// pointers into it.
#[derive(Resource, Clone, Copy, Deref)]
pub struct PlayerPack(pub Entity);

/// Seconds left on the "pack is full" notice (`ui::update_notice` renders it,
/// `pickup::collect_on_grab` sets it). `0.0` = not showing.
#[derive(Resource, Default)]
pub struct PackFullFlash(pub f32);

/// The catalogue key behind a grid [`InventoryItem`], inserted by
/// [`tag_item_kind`] so `use_item::fire_use` can dispatch on data rather than
/// re-parsing a display name. The library's `InventoryItem` is display-only by
/// design; this is the host's join back to [`crate::items`].
#[derive(Component, Clone, Copy)]
pub struct ItemKind(pub &'static str);

/// Tags every grid item that matches a catalogue entry with its [`ItemKind`].
/// Keyed on `On<Add, InventoryItem>` rather than the entity
/// [`InventoryCommands::add`] returns, so every add path — direct, the
/// `AddItem` message, a cross-board transfer — is covered by one rule.
pub fn tag_item_kind(
    add: On<Add, InventoryItem>,
    items: Query<&InventoryItem>,
    mut commands: Commands,
) {
    let Ok(item) = items.get(add.entity) else {
        return;
    };
    if let Some(def) = items::lookup_by_name(&item.name) {
        commands.entity(add.entity).insert(ItemKind(def.key));
    }
}

/// Builds the pickup's mesh + material + a `Sensor` grab volume. `Sensor`
/// reports no solid collision response — a pickup shouldn't block movement —
/// but the collider still lets `interact.rs`'s raycast find it. The grab
/// volume is a fixed `config::PICKUP_GRAB_SIZE` cube regardless of the model's
/// real size, so a tiny lockpick is as easy to aim at as a crate.
pub fn spawn_visuals(
    add: On<Add, ItemPickup>,
    pickups: Query<&ItemPickup>,
    mut interactables: Query<&mut Interactable>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(pickup) = pickups.get(add.entity) else {
        return;
    };
    let (shape, color, emissive) = match items::lookup(&pickup.item) {
        Some(d) => (d.shape, d.color, d.color.to_linear() * config::PICKUP_GLOW),
        // Uncatalogued key: the original bright placeholder cube.
        None => (
            ItemShape::Cube,
            config::PICKUP_COLOR,
            config::PICKUP_EMISSIVE,
        ),
    };

    let mesh = meshes.add(items::item_mesh(shape));
    let material = materials.add(StandardMaterial {
        base_color: color,
        emissive,
        perceptual_roughness: 0.6,
        ..default()
    });

    if let Ok(mut interactable) = interactables.get_mut(add.entity) {
        interactable.prompt = items::prompt_for(&pickup.item);
    }

    let g = config::PICKUP_GRAB_SIZE;
    commands.entity(add.entity).insert((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        RigidBody::Static,
        Collider::cuboid(g, g, g),
        Sensor,
    ));
}

/// RMB on a pickup: add its catalogue item to the pack. Reads the aimed entity
/// from [`InteractionFocus`] (like `carry::start_grab_or_charge`), not an event
/// target. Only despawns the world pickup once the item is actually in the grid
/// — a full pack leaves it where it lies and flashes [`PackFullFlash`], which is
/// the invariant the quickbar's "room in the pack first" rule stands on.
#[allow(clippy::too_many_arguments)]
pub fn collect_on_grab(
    _press: On<Start<Grab>>,
    focus: Res<InteractionFocus>,
    carrying: Query<(), With<Carrying>>,
    pickups: Query<&ItemPickup>,
    pack: Res<PlayerPack>,
    mut inventory: InventoryCommands,
    mut flash: ResMut<PackFullFlash>,
    mut commands: Commands,
) {
    // RMB while a crate is in hand arms a throw (`carry.rs`), never a collect.
    if !carrying.is_empty() {
        return;
    }
    let Some((target, _)) = focus.0 else {
        return;
    };
    let Ok(pickup) = pickups.get(target) else {
        return;
    };
    let item = match items::lookup(&pickup.item) {
        Some(def) => InventoryItem::new(def.name, def.description, def.cells, def.color),
        None => InventoryItem::new(
            pickup.item.clone(),
            "An unlabelled object.",
            UVec2::ONE,
            config::PICKUP_COLOR,
        ),
    };
    match inventory.add(**pack, item) {
        Some(_) => {
            info!("pickup: collected {}", pickup.item);
            commands.entity(target).despawn();
        }
        None => {
            flash.0 = config::PACK_FULL_SECS;
        }
    }
}
