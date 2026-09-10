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

/// A subtle view bob driven by the **stride phase**, not wall-clock time, so
/// it stays locked to [`footsteps`](crate::footsteps): `phase` is the running
/// stride count plus fraction (`Footsteps::steps as f32 + Footsteps::phase`).
/// Pure so it can be unit-tested. The vertical component dips to its minimum
/// exactly on each footfall (integer `phase`); the horizontal sway swings one
/// way per stride, crossing zero at every footfall — so its period is two
/// strides. `speed_scale` (0 at a standstill, 1 at `MOVE_SPEED`) only scales
/// the amplitude.
pub fn bob_offset(phase: f32, speed_scale: f32) -> Vec3 {
    let amplitude = config::VIEWMODEL_BOB_AMPLITUDE * speed_scale.clamp(0.0, 1.0);
    Vec3::new(
        (phase.rem_euclid(2.0) * std::f32::consts::PI).sin() * amplitude,
        -(0.5 + 0.5 * (phase * std::f32::consts::TAU).cos()) * amplitude,
        0.0,
    )
}

/// Applies [`bob_offset`] to each [`ViewModel`]'s local translation, phased off
/// the player's [`Footsteps`](crate::footsteps::Footsteps).
pub fn bob_viewmodel(
    player: Query<(&LinearVelocity, &crate::footsteps::Footsteps), With<CharacterController>>,
    mut holders: Query<&mut Transform, With<ViewModel>>,
) {
    let (phase, speed_scale) = player
        .single()
        .map(|(v, steps)| {
            (
                steps.steps as f32 + steps.phase,
                Vec2::new(v.0.x, v.0.z).length() / config::MOVE_SPEED,
            )
        })
        .unwrap_or((0.0, 0.0));
    let offset = bob_offset(phase, speed_scale);
    for mut transform in &mut holders {
        transform.translation = config::VIEWMODEL_OFFSET + offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bob_is_still_at_a_standstill() {
        for phase in [0.0, 0.3, 1.7, 9.9] {
            assert_eq!(bob_offset(phase, 0.0), Vec3::ZERO);
        }
    }

    #[test]
    fn bob_stays_within_the_amplitude() {
        let max = config::VIEWMODEL_BOB_AMPLITUDE;
        for i in 0..400 {
            let phase = i as f32 * 0.05;
            let o = bob_offset(phase, 2.0); // speed_scale clamped to 1.0
            assert!(o.x.abs() <= max + 1e-6, "x {} at {phase}", o.x);
            assert!(o.y.abs() <= max + 1e-6, "y {} at {phase}", o.y);
            assert!(o.y <= 1e-6, "vertical bob should only ever dip, got {}", o.y);
        }
    }

    #[test]
    fn vertical_dips_on_the_footfall() {
        // Minimum (most negative) at integer phase, back to zero mid-stride.
        assert!((bob_offset(0.0, 1.0).y + config::VIEWMODEL_BOB_AMPLITUDE).abs() < 1e-6);
        assert!(bob_offset(0.5, 1.0).y.abs() < 1e-6);
        assert!((bob_offset(1.0, 1.0).y + config::VIEWMODEL_BOB_AMPLITUDE).abs() < 1e-6);
    }

    #[test]
    fn bob_is_periodic_over_two_strides() {
        let a = bob_offset(0.37, 1.0);
        let b = bob_offset(0.37 + 2.0, 1.0);
        assert!((a - b).length() < 1e-4, "{a:?} vs {b:?}");
        // …but not over one — the horizontal sway has flipped.
        let c = bob_offset(0.37 + 1.0, 1.0);
        assert!((a.x - c.x).abs() > 1e-4);
    }
}
