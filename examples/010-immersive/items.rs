//! The item catalogue: one table keyed by the [`crate::classes::ItemPickup`]
//! `item` string, carrying both the *world* facts a pickup needs now (display
//! name, prompt text, a procedural model) and the *inventory* facts Phase 14
//! will need (footprint in cells, tile colour) — so the grid inventory has
//! nothing left to invent when it lands.
//!
//! Models are built the `ladder::ladder_mesh` way: Bevy primitive `Cuboid`s
//! merged into **one mesh**, so each is a drop-in `SceneRoot` swap later.
//! Cuboids only (not `Cylinder`) so every `Mesh::merge` has identical vertex
//! attributes and can't fail.

use bevy::prelude::*;

/// Which procedural model a catalogue entry draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemShape {
    Lockpick,
    Crowbar,
    /// The placeholder cube — also the fallback for an unknown item key.
    Cube,
}

/// One catalogue entry. `const`-constructible so [`CATALOGUE`] is a `const`.
///
/// `name` / `description` / `cells` / `color` map 1:1 onto
/// `bevy_game_bits::inventory::InventoryItem::new` — the catalogue is the one
/// source of item facts for both the world model and the grid inventory.
pub struct ItemDef {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Inventory footprint (width, height) in cells.
    pub cells: UVec2,
    /// Tile fill colour, and the material tint of the world model.
    pub color: Color,
    pub shape: ItemShape,
}

pub const CATALOGUE: &[ItemDef] = &[
    ItemDef {
        key: "lockpick",
        name: "Lockpick",
        description: "A slim tension pick. Make it your active item and use it on a locked door.",
        cells: UVec2::new(1, 1),
        color: Color::srgb(0.78, 0.74, 0.42),
        shape: ItemShape::Lockpick,
    },
    ItemDef {
        key: "crowbar",
        name: "Crowbar",
        description: "A heavy steel pry bar. Active in hand, it breaks wooden crates.",
        cells: UVec2::new(1, 3),
        color: Color::srgb(0.70, 0.22, 0.16),
        shape: ItemShape::Crowbar,
    },
];

/// The catalogue entry for `key`, if any.
pub fn lookup(key: &str) -> Option<&'static ItemDef> {
    CATALOGUE.iter().find(|d| d.key == key)
}

/// The catalogue entry for a display `name` — the reverse of [`lookup`], for
/// turning a grid `InventoryItem` (which only stores the display name) back
/// into its catalogue key. `pickup::tag_item_kind` uses it.
pub fn lookup_by_name(name: &str) -> Option<&'static ItemDef> {
    CATALOGUE.iter().find(|d| d.name == name)
}

/// Interact prompt for a pickup of `key` — "Pick up Lockpick", or a generic
/// line for an uncatalogued key. Used both by map-authored pickups (whose FGD
/// `prompt` this overrides at spawn) and by `breakable::shatter`'s drop.
pub fn prompt_for(key: &str) -> String {
    match lookup(key) {
        Some(def) => format!("Pick up {}", def.name),
        None => "Pick up item".to_string(),
    }
}

/// A single merged mesh for `shape`, centred on the origin, oriented so local
/// +Y is "up" as it lies on the ground.
pub fn item_mesh(shape: ItemShape) -> Mesh {
    match shape {
        ItemShape::Cube => Mesh::from(Cuboid::new(
            crate::config::PICKUP_SIZE,
            crate::config::PICKUP_SIZE,
            crate::config::PICKUP_SIZE,
        )),
        ItemShape::Lockpick => lockpick_mesh(),
        ItemShape::Crowbar => crowbar_mesh(),
    }
}

/// ~0.11 m long: a flat handle, a thin shaft, and a small up-turned pick tip.
fn lockpick_mesh() -> Mesh {
    // Handle at -Z, shaft running toward +Z, tip turned up at the +Z end.
    let handle = Cuboid::new(0.022, 0.010, 0.045);
    let mut mesh = Mesh::from(handle).translated_by(Vec3::new(0.0, 0.0, -0.030));

    let shaft = Cuboid::new(0.006, 0.006, 0.070);
    mesh.merge(&Mesh::from(shaft).translated_by(Vec3::new(0.0, 0.0, 0.020)))
        .unwrap();

    let tip = Cuboid::new(0.006, 0.006, 0.018);
    mesh.merge(
        &Mesh::from(tip)
            .rotated_by(Quat::from_rotation_x(-0.7))
            .translated_by(Vec3::new(0.0, 0.006, 0.056)),
    )
    .unwrap();
    mesh
}

/// ~0.55 m long: a shaft running along +Z, a flattened chisel at -Z and a
/// two-cuboid hooked claw at +Z.
fn crowbar_mesh() -> Mesh {
    let shaft = Cuboid::new(0.020, 0.020, 0.44);
    let mut mesh = Mesh::from(shaft);

    // Chisel end (-Z): a short flattened, slightly raked blade.
    let chisel = Cuboid::new(0.045, 0.010, 0.07);
    mesh.merge(
        &Mesh::from(chisel)
            .rotated_by(Quat::from_rotation_x(0.35))
            .translated_by(Vec3::new(0.0, 0.006, -0.245)),
    )
    .unwrap();

    // Hooked claw end (+Z): a bend, then the forked tip curling back.
    let bend = Cuboid::new(0.020, 0.020, 0.06);
    mesh.merge(
        &Mesh::from(bend)
            .rotated_by(Quat::from_rotation_x(-0.9))
            .translated_by(Vec3::new(0.0, 0.020, 0.245)),
    )
    .unwrap();
    let claw = Cuboid::new(0.030, 0.014, 0.05);
    mesh.merge(
        &Mesh::from(claw)
            .rotated_by(Quat::from_rotation_x(-1.8))
            .translated_by(Vec3::new(0.0, 0.052, 0.255)),
    )
    .unwrap();
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::primitives::MeshAabb;

    /// Full extents (width, height, depth) of a mesh's bounding box.
    fn mesh_size(mesh: &Mesh) -> Vec3 {
        Vec3::from(
            mesh.compute_aabb()
                .expect("mesh has positions")
                .half_extents,
        ) * 2.0
    }

    #[test]
    fn every_catalogue_key_is_unique() {
        for (i, a) in CATALOGUE.iter().enumerate() {
            for b in &CATALOGUE[i + 1..] {
                assert_ne!(a.key, b.key);
            }
        }
    }

    #[test]
    fn catalogue_names_are_unique_and_round_trip() {
        // `pickup::tag_item_kind` maps a grid item back to its catalogue key
        // by display name, so two entries must not share one.
        for (i, a) in CATALOGUE.iter().enumerate() {
            for b in &CATALOGUE[i + 1..] {
                assert_ne!(a.name, b.name);
            }
            assert_eq!(lookup_by_name(a.name).map(|d| d.key), Some(a.key));
        }
    }

    #[test]
    fn lockpick_is_pocket_sized() {
        let size = mesh_size(&item_mesh(ItemShape::Lockpick));
        assert!(size.z < 0.15, "lockpick {size:?} longer than 0.15 m");
        assert!(
            size.x < 0.05 && size.y < 0.05,
            "lockpick {size:?} too chunky"
        );
    }

    #[test]
    fn crowbar_is_a_forearm_long_bar() {
        let size = mesh_size(&item_mesh(ItemShape::Crowbar));
        assert!(
            (0.45..0.75).contains(&size.z),
            "crowbar length {} out of range",
            size.z
        );
        assert!(size.x < 0.1, "crowbar too wide: {size:?}");
    }
}
