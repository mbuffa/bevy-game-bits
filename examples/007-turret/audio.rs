//! Sound-effect plumbing. Gameplay systems emit `PlaySfx` messages; nothing
//! else touches the audio API. The `path()`/`volume()` tables below are the
//! single place to swap in real assets — the current entries are jsfxr
//! placeholder blips shipped with the repo.

use bevy::asset::LoadState;
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

/// Whether an `Sfx`'s asset has been confirmed decodable. `bevy_audio` panics
/// (`rodio::Decoder::new(..).unwrap()`) if it's ever handed bytes it can't
/// decode, so nothing reaches `AudioPlayer` until `verify_sfx` marks it `Ready`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SfxStatus {
    Pending,
    Ready,
    Broken,
}

/// Requests that arrived before their asset finished resolving. Replayed (or
/// dropped) once `verify_sfx` settles the status; capped so a stalled load
/// can't grow this unbounded.
const MAX_DEFERRED: usize = 32;

#[derive(Resource)]
pub struct SfxAssets {
    handles: HashMap<Sfx, Handle<AudioSource>>,
    status: HashMap<Sfx, SfxStatus>,
    deferred: Vec<Sfx>,
}

pub fn load_sfx(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handles: HashMap<_, _> = Sfx::ALL
        .into_iter()
        .map(|sfx| (sfx, asset_server.load(sfx.path())))
        .collect();
    let status = Sfx::ALL.into_iter().map(|sfx| (sfx, SfxStatus::Pending)).collect();
    commands.insert_resource(SfxAssets {
        handles,
        status,
        deferred: Vec::new(),
    });
}

/// bevy_audio unwraps rodio's decode result, so anything we can't recognise must
/// never reach `AudioPlayer`. Covers the two formats this build can decode:
/// wav (feature enabled in Cargo.toml) and ogg/vorbis (on by default).
fn is_decodable(bytes: &[u8]) -> bool {
    (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WAVE"[..])) || bytes.starts_with(b"OggS")
}

/// Resolve pending sfx loads to `Ready`/`Broken` and replay anything that was
/// requested while its asset was still in flight. A load can fail outright
/// (asset missing) or "succeed" with bytes rodio can't decode (an unfetched
/// git-lfs pointer file, or a format whose cargo feature isn't enabled) —
/// both are logged once here so the rest of the game never has to know.
pub fn verify_sfx(
    mut assets: ResMut<SfxAssets>,
    asset_server: Res<AssetServer>,
    audio_sources: Res<Assets<AudioSource>>,
    mut commands: Commands,
) {
    if !assets.status.values().any(|s| *s == SfxStatus::Pending) {
        return;
    }

    let pending: Vec<Sfx> = assets
        .status
        .iter()
        .filter(|(_, status)| **status == SfxStatus::Pending)
        .map(|(sfx, _)| *sfx)
        .collect();

    for sfx in pending {
        let handle = &assets.handles[&sfx];
        let resolved = match asset_server.load_state(handle.id()) {
            LoadState::Failed(err) => {
                error!("sfx {}: load failed ({err}); this sound is muted", sfx.path());
                Some(SfxStatus::Broken)
            }
            LoadState::Loaded => {
                let decodable = audio_sources.get(handle).is_some_and(|source| is_decodable(&source.bytes));
                if !decodable {
                    error!(
                        "sfx {}: not a decodable audio container (unfetched git-lfs pointer, or a format whose cargo feature is off); this sound is muted",
                        sfx.path()
                    );
                }
                Some(if decodable { SfxStatus::Ready } else { SfxStatus::Broken })
            }
            LoadState::NotLoaded | LoadState::Loading => None,
        };
        if let Some(status) = resolved {
            assets.status.insert(sfx, status);
        }
    }

    if assets.deferred.is_empty() {
        return;
    }
    let deferred = std::mem::take(&mut assets.deferred);
    for sfx in deferred {
        match assets.status.get(&sfx) {
            Some(SfxStatus::Ready) => spawn_one_shot(&mut commands, &assets, sfx),
            _ => {} // still pending (re-deferred below) or broken (dropped)
        }
        if assets.status.get(&sfx) == Some(&SfxStatus::Pending) {
            assets.deferred.push(sfx);
        }
    }
}

fn spawn_one_shot(commands: &mut Commands, assets: &SfxAssets, sfx: Sfx) {
    let handle = &assets.handles[&sfx];
    commands.spawn((
        AudioPlayer::new(handle.clone()),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(sfx.volume())),
    ));
}

pub fn play_sfx(mut commands: Commands, mut requests: MessageReader<PlaySfx>, mut assets: ResMut<SfxAssets>) {
    for PlaySfx(sfx) in requests.read() {
        match assets.status.get(sfx) {
            Some(SfxStatus::Ready) => spawn_one_shot(&mut commands, &assets, *sfx),
            Some(SfxStatus::Broken) => {} // already logged in verify_sfx
            Some(SfxStatus::Pending) | None => {
                if assets.deferred.len() < MAX_DEFERRED {
                    assets.deferred.push(*sfx);
                }
            }
        }
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
                if assets.status.get(&Sfx::LaserBeam) != Some(&SfxStatus::Ready) {
                    continue;
                }
                let handle = &assets.handles[&Sfx::LaserBeam];
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
