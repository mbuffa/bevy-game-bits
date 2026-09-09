//! The "use" raycast: what the player is looking at, and the event fired
//! when they press E on it. Mirrors the `SpatialQuery::cast_ray` pattern
//! already used in `examples/009-derby/vehicle.rs:405-530`.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::classes::Interactable;
use crate::{config, input};

/// What the player is currently looking at within range, if anything.
#[derive(Resource, Default)]
pub struct InteractionFocus(pub Option<(Entity, String)>);

/// Fired at the focused entity when the player presses `Interact`.
#[derive(EntityEvent, Debug, Clone)]
pub struct Interacted {
    #[event_target]
    pub entity: Entity,
}

/// The collider the ray hit isn't always the entity that owns the
/// `Interactable`: the ladder's solid part is a child of the `func_ladder`
/// brush (their faces are coplanar, so which one a ray catches is a toss-up),
/// and a glTF `SceneRoot` swap would nest the collider deeper still. Walk up
/// `ChildOf` to the nearest ancestor that is `Interactable`.
fn resolve_interactable(
    hit: Entity,
    interactables: &Query<&Interactable>,
    parents: &Query<&ChildOf>,
) -> Option<(Entity, String)> {
    let mut entity = hit;
    loop {
        if let Ok(i) = interactables.get(entity) {
            return Some((entity, i.prompt.clone()));
        }
        entity = parents.get(entity).ok()?.parent();
    }
}

/// Casts from the camera's eye along its forward axis every frame. Casting
/// from the camera (not the body) matches what the crosshair points at —
/// ahoy keeps the two roughly in sync, but with an eye-height offset.
pub fn update_focus(
    spatial_query: SpatialQuery,
    camera: Single<(&GlobalTransform, &CharacterControllerCameraOf)>,
    interactables: Query<&Interactable>,
    parents: Query<&ChildOf>,
    mut focus: ResMut<InteractionFocus>,
) {
    let (eye, camera_of) = *camera;
    // Exclude the player's own collider so the ray doesn't hit ourselves.
    let filter = SpatialQueryFilter::from_excluded_entities([camera_of.character_controller]);

    focus.0 = spatial_query
        .cast_ray(
            eye.translation(),
            eye.forward(),
            config::INTERACT_RANGE,
            true,
            &filter,
        )
        .and_then(|hit| resolve_interactable(hit.entity, &interactables, &parents));
}

pub fn fire_interact(
    _trigger: On<Fire<input::Interact>>,
    focus: Res<InteractionFocus>,
    mut commands: Commands,
) {
    if let Some((entity, _)) = focus.0 {
        commands.trigger(Interacted { entity });
    }
}
