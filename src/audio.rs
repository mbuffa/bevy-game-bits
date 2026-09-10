//! One-shot sound-effect playback, with the one guard that keeps `bevy_audio`
//! from taking the process down.
//!
//! A game writes a [`PlaySfx`] message with a clip handle, a linear volume and
//! a playback speed; [`SfxPlugin`] spawns a self-despawning [`AudioPlayer`] for
//! it. What this module deliberately does **not** know is what any sound
//! *means* — there's no catalogue enum, no "footstep" or "pickup" here. Keying
//! the readiness table by [`AssetId`] instead of a caller-side type is what
//! keeps it that way: the same plumbing serves an inventory click and a
//! footstep without either leaking into the library.
//!
//! # Why the guard exists
//!
//! `bevy_audio` decodes a clip with `rodio::Decoder::new(..).unwrap()` — handed
//! bytes it can't decode (an unfetched git-lfs pointer file, or a container
//! whose cargo feature is off), it panics. So nothing reaches [`AudioPlayer`]
//! until [`is_decodable`] has sniffed its header. A request that arrives before
//! its clip has loaded is deferred (up to [`MAX_DEFERRED`]) and replayed once
//! the verdict settles; a clip that fails to load or fails the sniff is logged
//! once and then silently muted forever.
//!
//! ```ignore
//! use bevy_game_bits::audio::prelude::*;
//!
//! app.add_plugins(SfxPlugin);
//!
//! fn on_step(mut sfx: MessageWriter<PlaySfx>, clips: Res<MyClips>) {
//!     sfx.write(PlaySfx::new(clips.footstep.clone()).with_volume(0.4).with_speed(1.05));
//! }
//! ```

use bevy::audio::Volume;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// A request to play one sound, once. `volume` is linear (1.0 = unchanged),
/// `speed` multiplies the playback rate and therefore also the pitch.
#[derive(Message, Clone, Debug)]
pub struct PlaySfx {
    pub clip: Handle<AudioSource>,
    pub volume: f32,
    pub speed: f32,
}

impl PlaySfx {
    /// Play `clip` at its recorded volume and pitch.
    pub fn new(clip: Handle<AudioSource>) -> Self {
        Self {
            clip,
            volume: 1.0,
            speed: 1.0,
        }
    }

    /// Set the linear volume (1.0 = unchanged).
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Set the playback-rate multiplier (also the pitch; 1.0 = unchanged).
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }
}

/// The most [`PlaySfx`] requests held back waiting on a still-loading clip. A
/// stalled load can't grow the queue past this.
pub const MAX_DEFERRED: usize = 32;

/// Per-clip decode verdicts (`true` = playable, `false` = muted, already
/// logged) plus the queue of requests whose clip wasn't resolved yet.
#[derive(Resource, Default)]
pub struct SfxGuard {
    verdict: HashMap<AssetId<AudioSource>, bool>,
    deferred: Vec<PlaySfx>,
}

/// Ordering handles for [`SfxPlugin`]'s systems. Order against these, not the
/// system functions.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SfxSet {
    /// Replays the requests that were waiting on a clip to finish loading.
    Verify,
    /// Drains [`PlaySfx`] and spawns the players.
    Play,
}

/// Covers the two containers a default Bevy build can decode: WAV (the `wav`
/// cargo feature) and Ogg/Vorbis (on by default). Anything else — including an
/// unfetched git-lfs pointer, which begins `version https://git-lfs...` — is
/// rejected before it can reach `rodio`.
pub fn is_decodable(bytes: &[u8]) -> bool {
    (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WAVE"[..]))
        || bytes.starts_with(b"OggS")
}

/// `Some(true)` = playable, `Some(false)` = muted, `None` = clip still loading.
/// The verdict is cached on first resolution and logged once when negative.
fn verdict(
    id: AssetId<AudioSource>,
    asset_server: &AssetServer,
    audio_sources: &Assets<AudioSource>,
    guard: &mut SfxGuard,
) -> Option<bool> {
    if let Some(&v) = guard.verdict.get(&id) {
        return Some(v);
    }
    if let Some(source) = audio_sources.get(id) {
        let ok = is_decodable(&source.bytes);
        if !ok {
            error!(
                "sfx {id}: not a decodable audio container (unfetched git-lfs pointer, \
                 or a format whose cargo feature is off); this sound is muted"
            );
        }
        guard.verdict.insert(id, ok);
        return Some(ok);
    }
    match asset_server.load_state(id) {
        bevy::asset::LoadState::Failed(err) => {
            error!("sfx {id}: load failed ({err}); this sound is muted");
            guard.verdict.insert(id, false);
            Some(false)
        }
        _ => None,
    }
}

fn spawn_one_shot(commands: &mut Commands, req: &PlaySfx) {
    commands.spawn((
        AudioPlayer::new(req.clip.clone()),
        PlaybackSettings::DESPAWN
            .with_volume(Volume::Linear(req.volume))
            .with_speed(req.speed),
    ));
}

/// Retry the requests parked on a not-yet-loaded clip. Cheap no-op while the
/// queue is empty.
pub fn verify_deferred(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    audio_sources: Res<Assets<AudioSource>>,
    mut guard: ResMut<SfxGuard>,
) {
    if guard.deferred.is_empty() {
        return;
    }
    for req in std::mem::take(&mut guard.deferred) {
        match verdict(req.clip.id(), &asset_server, &audio_sources, &mut guard) {
            Some(true) => spawn_one_shot(&mut commands, &req),
            Some(false) => {}
            None => guard.deferred.push(req),
        }
    }
}

/// Drain [`PlaySfx`]: play what's ready, defer what isn't, drop what's broken.
pub fn play_sfx(
    mut commands: Commands,
    mut requests: MessageReader<PlaySfx>,
    asset_server: Res<AssetServer>,
    audio_sources: Res<Assets<AudioSource>>,
    mut guard: ResMut<SfxGuard>,
) {
    for req in requests.read() {
        match verdict(req.clip.id(), &asset_server, &audio_sources, &mut guard) {
            Some(true) => spawn_one_shot(&mut commands, req),
            Some(false) => {}
            None => {
                if guard.deferred.len() < MAX_DEFERRED {
                    guard.deferred.push(req.clone());
                }
            }
        }
    }
}

/// Registers [`PlaySfx`], the [`SfxGuard`], and the two systems in `Update`.
pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlaySfx>()
            .init_resource::<SfxGuard>()
            .configure_sets(Update, (SfxSet::Verify, SfxSet::Play).chain())
            .add_systems(
                Update,
                (
                    verify_deferred.in_set(SfxSet::Verify),
                    play_sfx.in_set(SfxSet::Play),
                ),
            );
    }
}

/// `use bevy_game_bits::audio::prelude::*;`
pub mod prelude {
    pub use super::{is_decodable, PlaySfx, SfxGuard, SfxPlugin, SfxSet};
}

#[cfg(test)]
mod tests {
    use super::is_decodable;

    #[test]
    fn wav_and_ogg_headers_pass() {
        assert!(is_decodable(b"RIFF\0\0\0\0WAVEfmt "));
        assert!(is_decodable(b"OggS\0\x02\0\0"));
    }

    #[test]
    fn a_git_lfs_pointer_is_rejected() {
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 35860\n";
        assert!(!is_decodable(pointer));
    }

    #[test]
    fn junk_and_empty_are_rejected() {
        assert!(!is_decodable(b""));
        assert!(!is_decodable(b"RIFF\0\0\0\0AVI LIST")); // RIFF, but not WAVE
        assert!(!is_decodable(b"\x89PNG\r\n\x1a\n"));
    }
}
