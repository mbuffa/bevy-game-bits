use bevy::prelude::*;

use crate::config::*;
use crate::game::{GameAssets, Materials, PlacementRejected};
use crate::turret::{self, TurretKind};

pub fn place_turret_on_click(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    assets: Res<GameAssets>,
    mut materials: ResMut<Materials>,
    mut rejected: MessageWriter<PlacementRejected>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform)>,
) -> Result {
    let kind = if buttons.just_pressed(MouseButton::Left) {
        TurretKind::Kinetic
    } else if buttons.just_pressed(MouseButton::Right) {
        TurretKind::Laser
    } else {
        return Ok(());
    };

    let cost = match kind {
        TurretKind::Kinetic => KINETIC_COST,
        TurretKind::Laser => LASER_COST,
    };
    if materials.0 < cost {
        rejected.write(PlacementRejected);
        return Ok(());
    }

    let (camera, camera_transform) = *camera;
    let Some(cursor) = window.cursor_position() else {
        return Ok(());
    };

    let ray = camera.viewport_to_world(camera_transform, cursor)?;
    let Some(t) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) else {
        return Ok(());
    };
    let hit = ray.get_point(t);

    let position = Vec3::new(
        hit.x.clamp(-FIELD_WIDTH / 2.0 + 1.0, FIELD_WIDTH / 2.0 - 1.0),
        0.0,
        hit.z.clamp(-FIELD_DEPTH / 2.0 + 1.0, FIELD_DEPTH / 2.0 - 1.0),
    );

    materials.0 -= cost;
    turret::spawn_turret(&mut commands, &assets, kind, position);

    Ok(())
}
