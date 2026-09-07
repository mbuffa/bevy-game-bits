//! Loading a map from a `*.worldmap.json` file — the repo's first custom Bevy
//! [`AssetLoader`] — and [`WorldMapSource`], the component that says where one
//! map's data comes from.

use std::borrow::Cow;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;

use crate::world_map::data::WorldMapData;

/// A loaded map file. Wraps the raw [`WorldMapData`]; resolve it into a
/// [`WorldMapGrid`](super::WorldMapGrid) with
/// [`WorldMapGrid::from_data`](super::WorldMapGrid::from_data).
#[derive(Asset, TypePath, Debug, Clone)]
pub struct WorldMapAsset(pub WorldMapData);

/// Where one map's data comes from. A `Component` on the map entity, read once
/// by [`build_world_map`](super::build_world_map).
///
/// [`WorldMapPlugin`](super::WorldMapPlugin) can only hold a
/// [`Path`](Self::Path) at `build` time — there's no `AssetServer` yet — so
/// [`spawn_world_map`](super::spawn_world_map) keeps taking a plain spec and
/// resolves the path to a [`Handle`](Self::Handle) on its first frame.
/// [`Inline`](Self::Inline) skips the asset system entirely, for a
/// procedurally built map and for every headless test.
#[derive(Component, Clone, Debug)]
pub enum WorldMapSource {
    /// An asset path, e.g. `"maps/wastes.worldmap.json"`.
    Path(Cow<'static, str>),
    /// An already-requested handle.
    Handle(Handle<WorldMapAsset>),
    /// The data itself, no file involved.
    Inline(Box<WorldMapData>),
}

impl Default for WorldMapSource {
    fn default() -> Self {
        Self::Path(Cow::Borrowed("maps/wastes.worldmap.json"))
    }
}

/// Why a `*.worldmap.json` file failed to load.
#[derive(Debug)]
pub enum WorldMapLoadError {
    /// The bytes couldn't be read.
    Io(std::io::Error),
    /// The bytes weren't valid map JSON.
    Json(serde_json::Error),
}

impl std::fmt::Display for WorldMapLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "reading the map file: {e}"),
            Self::Json(e) => write!(f, "parsing the map file: {e}"),
        }
    }
}

impl std::error::Error for WorldMapLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
        }
    }
}

/// The [`AssetLoader`] for `*.worldmap.json`. Registered by
/// [`WorldMapPlugin`](super::WorldMapPlugin).
#[derive(Default, TypePath)]
pub struct WorldMapLoader;

impl AssetLoader for WorldMapLoader {
    type Asset = WorldMapAsset;
    type Settings = ();
    type Error = WorldMapLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<WorldMapAsset, WorldMapLoadError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(WorldMapLoadError::Io)?;
        let data: WorldMapData = serde_json::from_slice(&bytes).map_err(WorldMapLoadError::Json)?;
        Ok(WorldMapAsset(data))
    }

    fn extensions(&self) -> &[&str] {
        &["worldmap.json"]
    }
}
