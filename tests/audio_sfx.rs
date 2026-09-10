//! Headless behaviour tests for `bevy_game_bits::audio`.
//!
//! `MinimalPlugins` + `AssetPlugin` only — **no** `bevy::audio::AudioPlugin`,
//! so no audio device is opened. We assert that `SfxPlugin` puts an
//! `AudioPlayer` + `PlaybackSettings` on a spawned entity (or doesn't), and
//! that a request made before its clip resolves is replayed exactly once.
//! `Assets<AudioSource>` is normally registered by `AudioPlugin`, so the test
//! calls `init_asset` by hand; clip bytes are written directly.

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use bevy_game_bits::audio::prelude::*;
use bevy_game_bits::audio::MAX_DEFERRED;

fn source(bytes: &[u8]) -> AudioSource {
    AudioSource {
        bytes: bytes.to_vec().into(),
    }
}

const WAV: &[u8] = b"RIFF\x24\x08\x00\x00WAVEfmt \x10\x00\x00\x00";
const JUNK: &[u8] = b"not an audio file at all";

fn new_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .init_asset::<AudioSource>()
        .add_plugins(SfxPlugin);
    app.update();
    app
}

fn player_count(app: &mut App) -> usize {
    app.world_mut()
        .query::<&AudioPlayer>()
        .iter(app.world())
        .count()
}

#[test]
fn a_decodable_clip_plays() {
    let mut app = new_app();
    let handle = app
        .world_mut()
        .resource_mut::<Assets<AudioSource>>()
        .add(source(WAV));

    app.world_mut()
        .write_message(PlaySfx::new(handle).with_volume(0.4).with_speed(1.1));
    app.update();

    assert_eq!(player_count(&mut app), 1);
    let settings = app
        .world_mut()
        .query::<&PlaybackSettings>()
        .iter(app.world())
        .next()
        .copied()
        .expect("a PlaybackSettings on the player");
    assert_eq!(settings.speed, 1.1);
}

#[test]
fn an_undecodable_clip_is_muted() {
    let mut app = new_app();
    let handle = app
        .world_mut()
        .resource_mut::<Assets<AudioSource>>()
        .add(source(JUNK));

    app.world_mut().write_message(PlaySfx::new(handle));
    app.update();
    app.update();

    assert_eq!(player_count(&mut app), 0);
}

#[test]
fn a_request_before_the_clip_loads_is_replayed_once() {
    let mut app = new_app();
    // A reserved handle: no asset behind it yet, and the server has never
    // heard of it -> `verdict` returns `None` -> the request is deferred.
    let handle = app
        .world()
        .resource::<Assets<AudioSource>>()
        .reserve_handle();

    app.world_mut().write_message(PlaySfx::new(handle.clone()));
    app.update();
    assert_eq!(
        player_count(&mut app),
        0,
        "nothing plays while the clip is absent"
    );

    let _ = app
        .world_mut()
        .resource_mut::<Assets<AudioSource>>()
        .insert(&handle, source(WAV));
    app.update();
    assert_eq!(
        player_count(&mut app),
        1,
        "the deferred request fires once the clip arrives"
    );

    app.update();
    assert_eq!(player_count(&mut app), 1, "and not again");
}

#[test]
fn the_deferred_queue_is_capped() {
    let mut app = new_app();
    let handle = app
        .world()
        .resource::<Assets<AudioSource>>()
        .reserve_handle();

    for _ in 0..(MAX_DEFERRED + 20) {
        app.world_mut().write_message(PlaySfx::new(handle.clone()));
    }
    app.update();

    // No public len accessor by design; prove the cap held by resolving the
    // clip and counting how many actually play.
    let _ = app
        .world_mut()
        .resource_mut::<Assets<AudioSource>>()
        .insert(&handle, source(WAV));
    app.update();

    assert_eq!(player_count(&mut app), MAX_DEFERRED);
}
