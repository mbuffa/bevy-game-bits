//! `ItemPickup` visuals + collection into a simple inventory.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::classes::ItemPickup;
use crate::config;
use crate::interact::Interacted;

/// Collected item keys, in pickup order. `ui.rs` renders this as a line of
/// text; nothing consumes items yet (that's for whatever uses this later).
#[derive(Resource, Default)]
pub struct Inventory(pub Vec<String>);

/// A small emissive cube standing in for a real model, plus a sensor
/// collider so `interact.rs`'s raycast can find it. `Sensor` means it
/// reports no solid collision response — pickups shouldn't block movement.
pub fn spawn_visuals(
    add: On<Add, ItemPickup>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(
        config::PICKUP_SIZE,
        config::PICKUP_SIZE,
        config::PICKUP_SIZE,
    ));
    let material = materials.add(StandardMaterial {
        base_color: config::PICKUP_COLOR,
        emissive: config::PICKUP_EMISSIVE,
        ..default()
    });

    commands.entity(add.entity).insert((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        RigidBody::Static,
        Collider::cuboid(
            config::PICKUP_SIZE,
            config::PICKUP_SIZE,
            config::PICKUP_SIZE,
        ),
        Sensor,
    ));
}

pub fn collect_on_interact(
    trigger: On<Interacted>,
    pickups: Query<&ItemPickup>,
    mut inventory: ResMut<Inventory>,
    mut commands: Commands,
) {
    if let Ok(pickup) = pickups.get(trigger.entity) {
        inventory.0.push(pickup.item.clone());
        commands.entity(trigger.entity).despawn();
    }
}
