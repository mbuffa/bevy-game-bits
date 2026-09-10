//! The active quickbar item, drawn in the player's hands.
//!
//! One [`ViewModel`] entity is a child of the camera, offset to the
//! bottom-right (`config::VIEWMODEL_OFFSET`). Its mesh child is rebuilt from
//! [`crate::items::item_mesh`] whenever the active item changes, and the whole
//! thing is hidden when the hands are free, while carrying a crate, while
//! climbing, or while the pack is open.
//!
//! Unlike `carry::hold_prop`, this doesn't need the "write the camera late"
//! trick — a viewmodel is a plain child that inherits the camera transform,
//! not a physics body handed back to the world.

use avian3d::prelude::LinearVelocity;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_game_bits::inventory::prelude::*;

use crate::carry::Carrying;
use crate::config;
use crate::items;
use crate::ladder::Climbing;
use crate::pickup::{ItemKind, PlayerPack};

/// The hand/tool holder — a child of the camera. Its own `Transform` is the
/// view-bob offset; the mesh hangs one level deeper so it can be swapped
/// without disturbing the bob.
#[derive(Component)]
pub struct ViewModel;

/// Marks the swappable mesh child of a [`ViewModel`].
#[derive(Component)]
pub struct ViewModelMesh;

/// What the viewmodel currently shows (catalogue key), so [`sync_viewmodel`]
/// only rebuilds the mesh on a real change.
#[derive(Component, Default)]
pub struct ViewModelState(Option<&'static str>);

/// Spawns the (empty, hidden) viewmodel holder as a child of the camera the
/// moment it appears.
pub fn spawn_viewmodel(add: On<Add, CharacterControllerCameraOf>, mut commands: Commands) {
    commands.spawn((
        ViewModel,
        ViewModelState::default(),
        Transform::from_translation(config::VIEWMODEL_OFFSET),
        Visibility::Hidden,
        ChildOf(add.entity),
    ));
}

/// Rebuilds the held mesh when the active item changes; hides the viewmodel in
/// the contexts where a floating tool would be wrong. An idempotent mirror.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync_viewmodel(
    pack: Option<Res<PlayerPack>>,
    boards: Query<(&Quickbar, &ActiveSlot)>,
    kinds: Query<&ItemKind>,
    carrying: Query<(), With<Carrying>>,
    climbing: Query<(), With<Climbing>>,
    windows: Query<&InventoryWindow>,
    mut holders: Query<(Entity, &mut ViewModelState, &mut Visibility, Option<&Children>), With<ViewModel>>,
    meshes_children: Query<(), With<ViewModelMesh>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let suppressed =
        !carrying.is_empty() || !climbing.is_empty() || windows.iter().any(|w| w.open);

    let active_def = pack
        .as_ref()
        .and_then(|pack| boards.get(***pack).ok())
        .and_then(|(quickbar, active)| quickbar.slots.get(active.0?).copied().flatten())
        .and_then(|item| kinds.get(item).ok())
        .and_then(|kind| items::lookup(kind.0));

    let wanted = if suppressed { None } else { active_def.map(|d| d.key) };

    for (holder, mut state, mut visibility, children) in &mut holders {
        if state.0 != wanted {
            state.0 = wanted;
            if let Some(children) = children {
                for &child in children {
                    if meshes_children.get(child).is_ok() {
                        commands.entity(child).despawn();
                    }
                }
            }
            if let Some(def) = wanted.and_then(items::lookup) {
                let mesh = meshes.add(items::item_mesh(def.shape));
                let material = materials.add(StandardMaterial {
                    base_color: def.color,
                    emissive: def.color.to_linear() * config::VIEWMODEL_GLOW,
                    perceptual_roughness: 0.5,
                    ..default()
                });
                commands.spawn((
                    ViewModelMesh,
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::from_scale(Vec3::splat(config::VIEWMODEL_SCALE)).with_rotation(
                        Quat::from_euler(
                            EulerRot::YXZ,
                            config::VIEWMODEL_TILT.x,
                            config::VIEWMODEL_TILT.y,
                            config::VIEWMODEL_TILT.z,
                        ),
                    ),
                    NotShadowCaster,
                    ChildOf(holder),
                ));
            }
        }
        let wanted_visibility = if wanted.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted_visibility {
            *visibility = wanted_visibility;
        }
    }
}

/// A subtle view bob, scaled by the player's ground speed. Pure so it can be
/// unit-tested — the vertical bob dips once per footfall (period `1/HZ`), the
/// horizontal sway runs at half that (one full cycle per stride pair).
pub fn bob_offset(elapsed: f32, speed: f32) -> Vec3 {
    let scale = (speed / config::MOVE_SPEED).clamp(0.0, 1.0);
    let amplitude = config::VIEWMODEL_BOB_AMPLITUDE * scale;
    let w = config::VIEWMODEL_BOB_HZ * std::f32::consts::TAU;
    Vec3::new(
        (elapsed * w * 0.5).sin() * amplitude,
        -(elapsed * w).sin().abs() * amplitude,
        0.0,
    )
}

/// Applies [`bob_offset`] to each [`ViewModel`]'s local translation.
pub fn bob_viewmodel(
    time: Res<Time>,
    player: Query<&LinearVelocity, With<CharacterController>>,
    mut holders: Query<&mut Transform, With<ViewModel>>,
) {
    let speed = player
        .single()
        .map(|v| Vec2::new(v.0.x, v.0.z).length())
        .unwrap_or(0.0);
    let offset = bob_offset(time.elapsed_secs(), speed);
    for mut transform in &mut holders {
        transform.translation = config::VIEWMODEL_OFFSET + offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bob_is_still_at_a_standstill() {
        for t in [0.0, 0.3, 1.7, 9.9] {
            assert_eq!(bob_offset(t, 0.0), Vec3::ZERO);
        }
    }

    #[test]
    fn bob_stays_within_the_amplitude() {
        let max = config::VIEWMODEL_BOB_AMPLITUDE;
        for i in 0..400 {
            let t = i as f32 * 0.05;
            let o = bob_offset(t, config::MOVE_SPEED * 2.0); // clamped to 1.0
            assert!(o.x.abs() <= max + 1e-6, "x {} at {t}", o.x);
            assert!(o.y.abs() <= max + 1e-6, "y {} at {t}", o.y);
            assert!(o.y <= 1e-6, "vertical bob should only ever dip, got {}", o.y);
        }
    }

    #[test]
    fn bob_is_periodic() {
        // Full period is the slower (horizontal) component: 2 / HZ.
        let period = 2.0 / config::VIEWMODEL_BOB_HZ;
        let a = bob_offset(0.37, config::MOVE_SPEED);
        let b = bob_offset(0.37 + period, config::MOVE_SPEED);
        assert!((a - b).length() < 1e-4, "{a:?} vs {b:?}");
    }
}
