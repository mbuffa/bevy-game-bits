//! Sound-effect plumbing. Gameplay systems emit `PlaySfx` messages; nothing
//! else touches the audio API. The `path()`/`volume()` tables below are the
//! single place to swap in real assets — the current entries are jsfxr
//! placeholder blips shipped with the repo.

use bevy::audio::Volume;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::game::GamePhase;
use crate::weapons::LaserWeapon;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sfx {
    TurretPlaced,
    WaveStart,
    BuildPhase,
    MaterialsGained,
    KineticShot,
    LaserBeam,
    EnemyDestroyed,
    Victory,
    Defeat,
}

impl Sfx {
    pub const ALL: [Sfx; 9] = [
        Sfx::TurretPlaced,
        Sfx::WaveStart,
        Sfx::BuildPhase,
        Sfx::MaterialsGained,
        Sfx::KineticShot,
        Sfx::LaserBeam,
        Sfx::EnemyDestroyed,
        Sfx::Victory,
        Sfx::Defeat,
    ];

    pub const fn path(self) -> &'static str {
        match self {
            Sfx::TurretPlaced => "sfx/jsfxr/click.wav",
            Sfx::WaveStart => "sfx/jsfxr/weepwoop.wav",
            Sfx::BuildPhase => "sfx/jsfxr/cute.wav",
            Sfx::MaterialsGained => "sfx/jsfxr/pickup.wav",
            Sfx::KineticShot => "sfx/jsfxr/click2.wav",
            Sfx::LaserBeam => "sfx/jsfxr/leaves.wav",
            Sfx::EnemyDestroyed => "sfx/jsfxr/impact.wav",
            // Placeholder reuse until dedicated end-screen sounds exist.
            Sfx::Victory => "sfx/jsfxr/pickup.wav",
            Sfx::Defeat => "sfx/jsfxr/impact.wav",
        }
    }

    pub const fn volume(self) -> f32 {
        match self {
            // 10 rounds/s per turret; keep each click quiet.
            Sfx::KineticShot => 0.3,
            Sfx::LaserBeam => 0.4,
            Sfx::EnemyDestroyed => 0.8,
            _ => 1.0,
        }
    }
}

/// Request to play a one-shot sound, decoupling gameplay from audio playback
/// (same style as `DamageMessage`).
#[derive(Message)]
pub struct PlaySfx(pub Sfx);

#[derive(Resource)]
pub struct SfxAssets(HashMap<Sfx, Handle<AudioSource>>);

pub fn load_sfx(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(SfxAssets(
        Sfx::ALL
            .into_iter()
            .map(|sfx| (sfx, asset_server.load(sfx.path())))
            .collect(),
    ));
}

pub fn play_sfx(
    mut commands: Commands,
    mut requests: MessageReader<PlaySfx>,
    assets: Res<SfxAssets>,
) {
    for PlaySfx(sfx) in requests.read() {
        let Some(handle) = assets.0.get(sfx) else {
            continue;
        };
        commands.spawn((
            AudioPlayer::new(handle.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(sfx.volume())),
        ));
    }
}

/// Marks the looping beam sound parented to a firing laser turret.
#[derive(Component)]
pub struct LaserLoopSound;

/// Keep one looping beam sound alive per firing laser turret. The sound is a
/// child of the turret root, so it despawns with the turret (restart included).
pub fn update_laser_loops(
    mut commands: Commands,
    state: Res<State<GamePhase>>,
    assets: Res<SfxAssets>,
    turrets: Query<(Entity, &LaserWeapon, Option<&Children>)>,
    loops: Query<(), With<LaserLoopSound>>,
) {
    // `fire_lasers` stops running on Victory/Defeat, leaving `firing_at`
    // stale; treat frozen gameplay as not firing so beams fall silent.
    let live = matches!(state.get(), GamePhase::Building | GamePhase::WaveActive);

    for (turret, weapon, children) in &turrets {
        let firing = live && weapon.firing_at.is_some();
        let sound = children
            .into_iter()
            .flatten()
            .find(|child| loops.contains(**child))
            .copied();

        match (firing, sound) {
            (true, None) => {
                let Some(handle) = assets.0.get(&Sfx::LaserBeam) else {
                    continue;
                };
                commands.entity(turret).with_children(|parent| {
                    parent.spawn((
                        LaserLoopSound,
                        AudioPlayer::new(handle.clone()),
                        PlaybackSettings::LOOP.with_volume(Volume::Linear(Sfx::LaserBeam.volume())),
                    ));
                });
            }
            (false, Some(sound)) => commands.entity(sound).despawn(),
            _ => {}
        }
    }
}
