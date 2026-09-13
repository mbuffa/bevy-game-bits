//! The map itself: the JSON-facing [`WorldMapData`] format, the resolved
//! runtime [`WorldMapGrid`], and the coordinate math that maps a tile to a
//! spot on screen and back.
//!
//! No Bevy scheduling in this file — `travel.rs`, `ui.rs` and `time.rs` are
//! the only things that touch ECS state. This module is plain data plus pure
//! functions so it unit-tests under `cargo test --lib`, the same rule
//! `inventory::grid` follows.
//!
//! # Two coordinate spaces
//!
//! - **Tile space**: `(0, 0)` at the top-left corner, `+X` right, `+Y`
//!   **down** — the order `tiles[row][col]` is written in. A whole number is a
//!   corner; cell `(c, r)`'s centre is `(c as f32 + 0.5, r as f32 + 0.5)`.
//! - **World space**: Bevy 2D, `+Y` **up**, the map centred on the origin.
//!
//! They disagree about which way `Y` runs, so there is exactly one conversion
//! pair — [`tile_to_world`] / [`world_to_tile`] — and everything else routes
//! through it.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Default `tile_px` when a map file omits it.
fn default_tile_px() -> f32 {
    48.0
}

/// The on-disk map format — what a `*.worldmap.json` file deserializes into,
/// and what [`WorldMapSource::Inline`](super::WorldMapSource::Inline) carries
/// for a procedurally built map.
///
/// Resolve it into a [`WorldMapGrid`] with [`WorldMapGrid::from_data`], which
/// is where an unknown terrain id or a ragged tile grid is caught.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldMapData {
    /// Shown in the aside header.
    pub name: String,
    /// Width in tiles.
    pub cols: u32,
    /// Height in tiles.
    pub rows: u32,
    /// One tile's edge length in world units. Defaults to `48.0`.
    #[serde(default = "default_tile_px")]
    pub tile_px: f32,
    /// The terrain palette. Every string in [`tiles`](Self::tiles) must match
    /// one of these ids.
    pub terrains: Vec<TerrainData>,
    /// Row-major: `tiles[row][col]`, each entry a [`TerrainData::id`]. Must be
    /// exactly `rows` rows of `cols` columns.
    pub tiles: Vec<Vec<String>>,
    /// Named places on the map. May be empty.
    #[serde(default)]
    pub locations: Vec<LocationData>,
    /// Where the traveller starts, in **tile** units — fractional is fine
    /// (`[1.5, 8.5]` is the centre of cell `(1, 8)`).
    pub start: [f32; 2],
}

/// One entry in the terrain palette.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TerrainData {
    /// The key [`WorldMapData::tiles`] refers to this terrain by.
    pub id: String,
    /// Display name. Defaults to [`id`](Self::id) when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// Travel-speed multiplier: `1.0` is open ground, `0.35` is mountains,
    /// `0.0` is impassable (the traveller stops dead and the target clears).
    pub speed: f32,
    /// Fill colour, `"#rrggbb"` or `"#rgb"`.
    pub color: String,
}

/// A named place — a city, a settlement, a vault, or a [`secret`](Self::secret)
/// cache. Its position is given **either** as [`cell`](Self::cell) (a tile, snapped
/// to that tile's centre) **or** as [`at`](Self::at) (a tile-space float point) —
/// exactly one, never both.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LocationData {
    /// Stable identifier, handy for a host keying save data off it.
    pub id: String,
    /// Shown on the map and in the aside.
    pub name: String,
    /// Which tile it sits on, `[col, row]` — resolves to that tile's centre.
    /// Mutually exclusive with [`at`](Self::at).
    #[serde(default)]
    pub cell: Option<[u32; 2]>,
    /// A tile-space float point `[x, y]` for a place that isn't tile-aligned —
    /// a [`secret`](Self::secret) whose whole point is that you have to walk
    /// the *exact* spot. Mutually exclusive with [`cell`](Self::cell).
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    /// Flavour text for the aside. Optional.
    #[serde(default)]
    pub description: Option<String>,
    /// A secret location stays hidden until the traveller walks almost exactly
    /// over it — [`WorldMapConfig::secret_reveal_radius_tiles`](super::WorldMapConfig::secret_reveal_radius_tiles),
    /// not the wide [`reveal_radius_tiles`](super::WorldMapConfig::reveal_radius_tiles)
    /// an ordinary place is spotted from. It also flips the
    /// [`discovered`](Self::discovered) default to `false`.
    #[serde(default)]
    pub secret: bool,
    /// Whether it's known from the start. An undiscovered location is hidden
    /// from the map and the aside until the traveller passes close. Defaults to
    /// `false` for a [`secret`](Self::secret), `true` otherwise.
    #[serde(default)]
    pub discovered: Option<bool>,
}

impl LocationData {
    /// The location's position in tile space: [`at`](Self::at) if set, else the
    /// centre of [`cell`](Self::cell). `None` if neither is set — a state
    /// [`WorldMapGrid::from_data`] rejects.
    pub fn tile_pos(&self) -> Option<Vec2> {
        match (self.at, self.cell) {
            (Some([x, y]), _) => Some(Vec2::new(x, y)),
            (None, Some([c, r])) => Some(Vec2::new(c as f32 + 0.5, r as f32 + 0.5)),
            (None, None) => None,
        }
    }

    /// Whether the location is known from the start — the [`discovered`](Self::discovered)
    /// flag if the file set it, otherwise `false` for a secret and `true` for
    /// anything else.
    pub fn discovered(&self) -> bool {
        self.discovered.unwrap_or(!self.secret)
    }
}

impl WorldMapData {
    /// A small hand-built map — a 6×6 grid with a lake of slow water in the
    /// middle, two towns (one hidden), and a secret cache off a float point.
    /// Handy as a test fixture and as an
    /// [`Inline`](super::WorldMapSource::Inline) starting point before you have
    /// a real file.
    pub fn demo() -> Self {
        let p = "plains";
        let w = "water";
        let m = "mountain";
        let row = |cells: [&str; 6]| cells.iter().map(|s| s.to_string()).collect();
        Self {
            name: "Demo Vale".to_string(),
            cols: 6,
            rows: 6,
            tile_px: 48.0,
            terrains: vec![
                TerrainData {
                    id: p.to_string(),
                    name: Some("Plains".to_string()),
                    speed: 1.0,
                    color: "#6b7a4a".to_string(),
                },
                TerrainData {
                    id: w.to_string(),
                    name: Some("Water".to_string()),
                    speed: 0.3,
                    color: "#3a5a7a".to_string(),
                },
                TerrainData {
                    id: m.to_string(),
                    name: Some("Mountain".to_string()),
                    speed: 0.35,
                    color: "#7a6f63".to_string(),
                },
            ],
            tiles: vec![
                row([p, p, p, p, p, p]),
                row([p, p, m, m, p, p]),
                row([p, p, w, w, p, p]),
                row([p, p, w, w, p, p]),
                row([p, p, p, p, p, p]),
                row([p, p, p, p, p, p]),
            ],
            locations: vec![
                LocationData {
                    id: "haven".to_string(),
                    name: "Haven".to_string(),
                    cell: Some([1, 1]),
                    at: None,
                    description: Some("A walled town on the north road.".to_string()),
                    secret: false,
                    discovered: Some(true),
                },
                LocationData {
                    id: "ford".to_string(),
                    name: "The Ford".to_string(),
                    cell: Some([4, 4]),
                    at: None,
                    description: Some(
                        "A river crossing, and the market that grew around it.".to_string(),
                    ),
                    secret: false,
                    discovered: Some(false),
                },
                LocationData {
                    id: "cache".to_string(),
                    name: "Buried Cache".to_string(),
                    cell: None,
                    at: Some([5.25, 0.25]),
                    description: Some("Someone meant to come back for this.".to_string()),
                    secret: true,
                    discovered: None,
                },
            ],
            start: [0.5, 5.5],
        }
    }
}

/// A resolved terrain kind: the palette entry with its colour parsed and its
/// name filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    pub id: String,
    pub name: String,
    /// Travel-speed multiplier (see [`TerrainData::speed`]).
    pub speed: f32,
    pub color: Color,
}

/// The runtime form of one map: the resolved terrain palette plus a flat
/// row-major grid of indices into it. A `Component` on the map entity —
/// everything about one map lives on its entity, so a host can hold two maps
/// at once.
///
/// Build one with [`from_data`](Self::from_data); [`build_world_map`](super::build_world_map)
/// does that from a loaded [`WorldMapAsset`](super::WorldMapAsset).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct WorldMapGrid {
    cols: u32,
    rows: u32,
    tile_px: f32,
    terrains: Vec<Terrain>,
    /// `tiles[row * cols + col]` indexes [`terrains`](Self::terrains).
    tiles: Vec<u16>,
}

/// Why a [`WorldMapData`] couldn't be turned into a [`WorldMapGrid`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldMapError {
    /// The terrain palette was empty.
    NoTerrains,
    /// A tile named a terrain id that isn't in the palette.
    UnknownTerrain { id: String },
    /// A terrain's `color` string didn't parse as a hex colour.
    BadColor { id: String, hex: String },
    /// `tiles` didn't have exactly `rows` rows.
    WrongRowCount { expected: u32, found: usize },
    /// Row `row` didn't have exactly `cols` columns.
    WrongColCount {
        row: usize,
        expected: u32,
        found: usize,
    },
    /// A location's position was outside the `cols` × `rows` grid. `cell` is
    /// the tile the position fell in.
    LocationOutOfBounds { id: String, cell: [u32; 2] },
    /// A location gave neither `cell` nor `at`.
    LocationHasNoPosition { id: String },
    /// A location gave both `cell` and `at`.
    LocationHasTwoPositions { id: String },
}

impl std::fmt::Display for WorldMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTerrains => write!(f, "the terrain palette is empty"),
            Self::UnknownTerrain { id } => write!(f, "a tile refers to unknown terrain {id:?}"),
            Self::BadColor { id, hex } => {
                write!(f, "terrain {id:?} has an unparseable color {hex:?}")
            }
            Self::WrongRowCount { expected, found } => {
                write!(f, "expected {expected} tile rows, found {found}")
            }
            Self::WrongColCount {
                row,
                expected,
                found,
            } => write!(f, "tile row {row} has {found} columns, expected {expected}"),
            Self::LocationOutOfBounds { id, cell } => {
                write!(f, "location {id:?} sits off the map at {cell:?}")
            }
            Self::LocationHasNoPosition { id } => {
                write!(f, "location {id:?} gives neither `cell` nor `at`")
            }
            Self::LocationHasTwoPositions { id } => {
                write!(f, "location {id:?} gives both `cell` and `at`")
            }
        }
    }
}

impl std::error::Error for WorldMapError {}

impl WorldMapGrid {
    /// Resolve a [`WorldMapData`]: parse every terrain colour, check the tile
    /// grid is rectangular and every id is known, and check every location is
    /// on the map.
    pub fn from_data(data: &WorldMapData) -> Result<Self, WorldMapError> {
        if data.terrains.is_empty() {
            return Err(WorldMapError::NoTerrains);
        }

        let terrains: Vec<Terrain> =
            data.terrains
                .iter()
                .map(|t| {
                    let color = Srgba::hex(&t.color).map(Color::from).map_err(|_| {
                        WorldMapError::BadColor {
                            id: t.id.clone(),
                            hex: t.color.clone(),
                        }
                    })?;
                    Ok(Terrain {
                        id: t.id.clone(),
                        name: t.name.clone().unwrap_or_else(|| t.id.clone()),
                        speed: t.speed,
                        color,
                    })
                })
                .collect::<Result<_, _>>()?;

        if data.tiles.len() != data.rows as usize {
            return Err(WorldMapError::WrongRowCount {
                expected: data.rows,
                found: data.tiles.len(),
            });
        }

        let index_of = |id: &str| terrains.iter().position(|t| t.id == id);
        let mut tiles = Vec::with_capacity((data.cols * data.rows) as usize);
        for (row, cells) in data.tiles.iter().enumerate() {
            if cells.len() != data.cols as usize {
                return Err(WorldMapError::WrongColCount {
                    row,
                    expected: data.cols,
                    found: cells.len(),
                });
            }
            for id in cells {
                let idx =
                    index_of(id).ok_or_else(|| WorldMapError::UnknownTerrain { id: id.clone() })?;
                tiles.push(idx as u16);
            }
        }

        for location in &data.locations {
            if location.cell.is_some() && location.at.is_some() {
                return Err(WorldMapError::LocationHasTwoPositions {
                    id: location.id.clone(),
                });
            }
            let Some(pos) = location.tile_pos() else {
                return Err(WorldMapError::LocationHasNoPosition {
                    id: location.id.clone(),
                });
            };
            let cell = cell_of(pos);
            if !(0..data.cols as i32).contains(&cell.x) || !(0..data.rows as i32).contains(&cell.y)
            {
                return Err(WorldMapError::LocationOutOfBounds {
                    id: location.id.clone(),
                    cell: [cell.x.max(0) as u32, cell.y.max(0) as u32],
                });
            }
        }

        Ok(Self {
            cols: data.cols,
            rows: data.rows,
            tile_px: data.tile_px,
            terrains,
            tiles,
        })
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn tile_px(&self) -> f32 {
        self.tile_px
    }

    /// The resolved terrain palette, in file order.
    pub fn terrains(&self) -> &[Terrain] {
        &self.terrains
    }

    /// The map's full size in world units, `(cols · tile_px, rows · tile_px)`.
    pub fn size_px(&self) -> Vec2 {
        Vec2::new(
            self.cols as f32 * self.tile_px,
            self.rows as f32 * self.tile_px,
        )
    }

    /// Is `cell` inside the grid?
    pub fn contains(&self, cell: IVec2) -> bool {
        cell.x >= 0 && cell.y >= 0 && (cell.x as u32) < self.cols && (cell.y as u32) < self.rows
    }

    /// The terrain on `cell`, or `None` if `cell` is off the map.
    pub fn terrain_at(&self, cell: IVec2) -> Option<&Terrain> {
        self.tile_terrain_index(cell).map(|i| &self.terrains[i])
    }

    /// The index into [`terrains`](Self::terrains) of `cell`'s terrain, or
    /// `None` if `cell` is off the map.
    pub fn tile_terrain_index(&self, cell: IVec2) -> Option<usize> {
        if !self.contains(cell) {
            return None;
        }
        let i = cell.y as usize * self.cols as usize + cell.x as usize;
        Some(self.tiles[i] as usize)
    }

    /// The travel-speed multiplier at a **tile-space** position. Off-map
    /// positions clamp to the nearest edge cell rather than returning `0.0`,
    /// so a traveller resting exactly on the far border still samples real
    /// ground.
    pub fn speed_at(&self, tile: Vec2) -> f32 {
        let cell = cell_of(tile).clamp(
            IVec2::ZERO,
            IVec2::new(self.cols as i32 - 1, self.rows as i32 - 1),
        );
        self.terrain_at(cell).map(|t| t.speed).unwrap_or(0.0)
    }

    /// Clamp a tile-space position to stay on the map. A resting spot exactly
    /// on the far edge is pulled a hair inside so [`cell_of`] still names a
    /// real cell.
    pub fn clamp_tile_pos(&self, tile: Vec2) -> Vec2 {
        const MARGIN: f32 = 1.0e-3;
        Vec2::new(
            tile.x.clamp(0.0, self.cols as f32 - MARGIN),
            tile.y.clamp(0.0, self.rows as f32 - MARGIN),
        )
    }
}

/// Tile-space → world-space. `size_px` is [`WorldMapGrid::size_px`]; the map is
/// centred on the origin, and `Y` is flipped (tile `+Y` is down, world `+Y` is
/// up).
pub fn tile_to_world(tile: Vec2, size_px: Vec2, tile_px: f32) -> Vec2 {
    Vec2::new(
        tile.x * tile_px - size_px.x / 2.0,
        size_px.y / 2.0 - tile.y * tile_px,
    )
}

/// World-space → tile-space, the exact inverse of [`tile_to_world`].
pub fn world_to_tile(world: Vec2, size_px: Vec2, tile_px: f32) -> Vec2 {
    Vec2::new(
        (world.x + size_px.x / 2.0) / tile_px,
        (size_px.y / 2.0 - world.y) / tile_px,
    )
}

/// The cell a tile-space position falls in. Signed and unclamped — a position
/// past an edge legitimately names a negative or overflowing cell, and
/// [`WorldMapGrid::contains`] is what rejects that. Flooring means a position
/// exactly on a boundary belongs to the cell after it.
pub fn cell_of(tile: Vec2) -> IVec2 {
    tile.floor().as_ivec2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_world_round_trip() {
        let size = Vec2::new(288.0, 288.0); // 6 x 6 @ 48
        let tile_px = 48.0;
        for tile in [
            Vec2::ZERO,
            Vec2::new(6.0, 6.0),
            Vec2::new(1.5, 8.5),
            Vec2::new(3.25, 0.75),
        ] {
            let world = tile_to_world(tile, size, tile_px);
            let back = world_to_tile(world, size, tile_px);
            assert!(
                (tile - back).length() < 1.0e-4,
                "{tile} -> {world} -> {back}"
            );
        }
    }

    #[test]
    fn tile_origin_is_top_left_in_world() {
        let size = Vec2::new(288.0, 288.0);
        assert_eq!(
            tile_to_world(Vec2::ZERO, size, 48.0),
            Vec2::new(-144.0, 144.0)
        );
        assert_eq!(
            tile_to_world(Vec2::new(6.0, 6.0), size, 48.0),
            Vec2::new(144.0, -144.0)
        );
    }

    #[test]
    fn cell_of_floors_and_goes_negative() {
        assert_eq!(cell_of(Vec2::new(0.0, 0.0)), IVec2::new(0, 0));
        assert_eq!(cell_of(Vec2::new(2.9, 1.1)), IVec2::new(2, 1));
        assert_eq!(cell_of(Vec2::new(3.0, 3.0)), IVec2::new(3, 3));
        assert_eq!(cell_of(Vec2::new(-0.1, -0.1)), IVec2::new(-1, -1));
    }

    #[test]
    fn from_data_resolves_the_demo_map() {
        let grid = WorldMapGrid::from_data(&WorldMapData::demo()).unwrap();
        assert_eq!(grid.cols(), 6);
        assert_eq!(grid.rows(), 6);
        assert_eq!(grid.size_px(), Vec2::new(288.0, 288.0));
        // (2,2) is water in the demo layout.
        assert_eq!(grid.terrain_at(IVec2::new(2, 2)).unwrap().id, "water");
        assert_eq!(grid.terrain_at(IVec2::new(0, 0)).unwrap().id, "plains");
    }

    #[test]
    fn from_data_rejects_an_empty_palette() {
        let mut data = WorldMapData::demo();
        data.terrains.clear();
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::NoTerrains)
        );
    }

    #[test]
    fn from_data_rejects_an_unknown_terrain_id() {
        let mut data = WorldMapData::demo();
        data.tiles[0][0] = "lava".to_string();
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::UnknownTerrain {
                id: "lava".to_string()
            })
        );
    }

    #[test]
    fn from_data_rejects_a_ragged_grid() {
        let mut data = WorldMapData::demo();
        data.tiles[2].pop();
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::WrongColCount {
                row: 2,
                expected: 6,
                found: 5,
            })
        );

        let mut data = WorldMapData::demo();
        data.tiles.pop();
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::WrongRowCount {
                expected: 6,
                found: 5,
            })
        );
    }

    #[test]
    fn from_data_rejects_a_bad_color() {
        let mut data = WorldMapData::demo();
        data.terrains[0].color = "not a color".to_string();
        assert!(matches!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::BadColor { .. })
        ));
    }

    #[test]
    fn from_data_rejects_an_off_map_location() {
        let mut data = WorldMapData::demo();
        data.locations[0].cell = Some([9, 0]);
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::LocationOutOfBounds {
                id: "haven".to_string(),
                cell: [9, 0],
            })
        );
    }

    #[test]
    fn from_data_rejects_an_off_map_float_point() {
        let mut data = WorldMapData::demo();
        data.locations[2].at = Some([8.5, 0.2]); // "cache" is the float one
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::LocationOutOfBounds {
                id: "cache".to_string(),
                cell: [8, 0],
            })
        );
    }

    #[test]
    fn from_data_rejects_a_location_with_no_position() {
        let mut data = WorldMapData::demo();
        data.locations[0].cell = None;
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::LocationHasNoPosition {
                id: "haven".to_string(),
            })
        );
    }

    #[test]
    fn from_data_rejects_a_location_with_two_positions() {
        let mut data = WorldMapData::demo();
        data.locations[0].at = Some([1.5, 1.5]);
        assert_eq!(
            WorldMapGrid::from_data(&data),
            Err(WorldMapError::LocationHasTwoPositions {
                id: "haven".to_string(),
            })
        );
    }

    #[test]
    fn location_data_resolves_position_and_discovered_default() {
        let data = WorldMapData::demo();
        // "cache" is a secret given as a float point.
        let cache = &data.locations[2];
        assert_eq!(cache.tile_pos(), Some(Vec2::new(5.25, 0.25)));
        assert!(!cache.discovered(), "a secret defaults to undiscovered");
        // "haven" is a plain town given as a cell -> resolves to the centre.
        let haven = &data.locations[0];
        assert_eq!(haven.tile_pos(), Some(Vec2::new(1.5, 1.5)));
        assert!(haven.discovered());
    }

    #[test]
    fn speed_at_samples_terrain_and_clamps_off_map() {
        let grid = WorldMapGrid::from_data(&WorldMapData::demo()).unwrap();
        assert_eq!(grid.speed_at(Vec2::new(0.5, 0.5)), 1.0); // plains
        assert_eq!(grid.speed_at(Vec2::new(2.5, 2.5)), 0.3); // water
                                                             // Off the left edge clamps to column 0, still plains.
        assert_eq!(grid.speed_at(Vec2::new(-5.0, 0.5)), 1.0);
    }

    #[test]
    fn clamp_tile_pos_keeps_cell_of_valid() {
        let grid = WorldMapGrid::from_data(&WorldMapData::demo()).unwrap();
        let clamped = grid.clamp_tile_pos(Vec2::new(20.0, -3.0));
        assert!(grid.contains(cell_of(clamped)), "{clamped}");
        assert_eq!(cell_of(clamped), IVec2::new(5, 0));
    }
}
