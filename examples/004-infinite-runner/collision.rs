use bevy::math::bounding::{Aabb2d, IntersectsVolume};
use bevy::prelude::*;

use crate::actors::{Obstacle, Player};

#[derive(Event, Default)]
pub struct CollisionEvent;

pub fn detect_collisions(
    player: Single<&Transform, With<Player>>,
    obstacles: Query<&Transform, With<Obstacle>>,
    mut events: EventWriter<CollisionEvent>,
) {
    let player_transform = player.into_inner();
    let player_bounding_box = Aabb2d::new(
        player_transform.translation.truncate(),
        player_transform.scale.truncate() / 2.,
    );

    for transform in obstacles.iter() {
        let obstacle_bounding_box = Aabb2d::new(
            transform.translation.truncate(),
            transform.scale.truncate() / 2.,
        );

        if player_bounding_box.intersects(&obstacle_bounding_box) {
            events.send_default();
        }
    }
}
