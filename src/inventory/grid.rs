//! The pure occupancy model: which cell holds which item, and whether a
//! footprint fits. No Bevy scheduling here — `drag.rs`, `items.rs` and
//! `ui.rs` are the only things that touch ECS state; this module is plain
//! data so it's cleanly unit-testable (`cargo test --lib`).

use bevy::prelude::*;

/// The result of an [`InventoryGrid::resize`]: which items had to move to a
/// new origin, and which no longer fit anywhere and were dropped from the
/// grid entirely. Empty vecs on a grow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResizeOutcome {
    /// `(item, new_origin)` for every item whose old footprint no longer fit
    /// but which found a fresh row-major spot.
    pub relocated: Vec<(Entity, UVec2)>,
    /// Items that fit nowhere on the new board and were removed from the grid.
    /// The caller still has to despawn their nodes.
    pub evicted: Vec<Entity>,
}

/// Row-major occupancy grid: `cells[y * cols + x]` is the item (if any)
/// covering that cell. The single source of truth for hit-testing and
/// placement — there's no separate picking layer, and deliberately no
/// `Default`: a board with no size isn't a thing you ever want by accident.
/// A `Component` on the board entity;
/// [`spawn_inventory`](super::spawn_inventory) builds one from
/// [`InventoryConfig`](super::InventoryConfig). Construct one yourself only in
/// tests.
#[derive(Component)]
pub struct InventoryGrid {
    cells: Vec<Option<Entity>>,
    cols: u32,
    rows: u32,
}

impl InventoryGrid {
    pub fn new(cols: u32, rows: u32) -> Self {
        Self {
            cells: vec![None; (cols * rows) as usize],
            cols,
            rows,
        }
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    fn index(&self, cell: UVec2) -> Option<usize> {
        (cell.x < self.cols && cell.y < self.rows).then(|| (cell.y * self.cols + cell.x) as usize)
    }

    /// The entity occupying `cell`, if any. `None` for both "empty" and
    /// "out of bounds" — callers that care about the difference should
    /// bounds-check separately (`fits` does).
    pub fn at(&self, cell: UVec2) -> Option<Entity> {
        self.index(cell).and_then(|i| self.cells[i])
    }

    /// Does a `size`-shaped item fit with its top-left corner at `origin`?
    /// A negative or overflowing `origin` is always a miss. Cells occupied
    /// by `ignoring` don't count as taken — that's how a dragged item is
    /// kept in the grid (so nothing else can slide under it mid-drag)
    /// without blocking its own drop.
    pub fn fits(&self, origin: IVec2, size: UVec2, ignoring: Option<Entity>) -> bool {
        if origin.x < 0 || origin.y < 0 {
            return false;
        }
        let origin = origin.as_uvec2();
        if origin.x + size.x > self.cols || origin.y + size.y > self.rows {
            return false;
        }
        for dy in 0..size.y {
            for dx in 0..size.x {
                match self.at(origin + UVec2::new(dx, dy)) {
                    Some(occupant) if Some(occupant) != ignoring => return false,
                    _ => {}
                }
            }
        }
        true
    }

    /// Stamp `item` into every cell of `size` starting at `origin`. Caller
    /// must have already checked `fits` — this doesn't re-check.
    /// [`spawn_item`](super::spawn_item) is the checked way in.
    pub fn place(&mut self, item: Entity, origin: UVec2, size: UVec2) {
        for dy in 0..size.y {
            for dx in 0..size.x {
                if let Some(i) = self.index(origin + UVec2::new(dx, dy)) {
                    self.cells[i] = Some(item);
                }
            }
        }
    }

    /// Free every cell currently held by `item`, wherever it is. Grid-wide
    /// scan rather than tracking size separately — the board is small, and
    /// this way a caller can never clear the wrong footprint.
    pub fn clear(&mut self, item: Entity) {
        for cell in &mut self.cells {
            if *cell == Some(item) {
                *cell = None;
            }
        }
    }

    /// The first origin, scanning left-to-right then top-to-bottom, where a
    /// `size`-shaped item would fit — what "put this loot somewhere" wants.
    /// `None` if the board is too full.
    pub fn first_fit(&self, size: UVec2) -> Option<UVec2> {
        for y in 0..self.rows {
            for x in 0..self.cols {
                let origin = UVec2::new(x, y);
                if self.fits(origin.as_ivec2(), size, None) {
                    return Some(origin);
                }
            }
        }
        None
    }

    /// Would a `size`-shaped item fit *anywhere* right now? The question a
    /// host asks before spawning loot it might have to drop on the floor
    /// instead.
    pub fn has_room_for(&self, size: UVec2) -> bool {
        self.first_fit(size).is_some()
    }

    /// How many cells are currently empty. Not the same as "can I fit a 2x2"
    /// — the free cells may be scattered — but a cheap upper bound and handy
    /// for a HUD.
    pub fn free_cells(&self) -> u32 {
        self.cells.iter().filter(|c| c.is_none()).count() as u32
    }

    /// Re-shape the board to `cols`x`rows` and re-place `occupants` —
    /// `(item, current_origin, size)` for every item currently on the board,
    /// which the caller must supply because the grid stores only entities per
    /// cell, not sizes.
    ///
    /// Processed row-major over the *old* board so the outcome is
    /// deterministic. An item whose current footprint still fits keeps its
    /// origin; one that doesn't is re-placed with [`first_fit`](Self::first_fit);
    /// one that still fits nowhere is evicted (cleared from the grid — the
    /// caller despawns the node).
    pub fn resize(
        &mut self,
        cols: u32,
        rows: u32,
        occupants: &[(Entity, UVec2, UVec2)],
    ) -> ResizeOutcome {
        self.cols = cols;
        self.rows = rows;
        self.cells = vec![None; (cols * rows) as usize];

        let mut ordered: Vec<&(Entity, UVec2, UVec2)> = occupants.iter().collect();
        ordered.sort_by_key(|(_, origin, _)| (origin.y, origin.x));

        let mut outcome = ResizeOutcome::default();
        for &&(item, origin, size) in &ordered {
            if self.fits(origin.as_ivec2(), size, None) {
                self.place(item, origin, size);
            } else if let Some(new_origin) = self.first_fit(size) {
                self.place(item, new_origin, size);
                outcome.relocated.push((item, new_origin));
            } else {
                outcome.evicted.push(item);
            }
        }
        outcome
    }
}

/// The cell a board-relative pixel position falls in. Signed, and not
/// clamped to the board: dragging past an edge legitimately hovers a
/// negative or overflowing cell, and [`InventoryGrid::fits`] is what rejects
/// that, not this. Flooring means a position exactly on a boundary belongs
/// to the cell after it.
pub fn hovered_cell(px: Vec2, cell_px: f32) -> IVec2 {
    (px / cell_px).floor().as_ivec2()
}

/// Which of an item's own cells the cursor grabbed, from a pixel offset
/// measured from the item's top-left corner. Floors to the containing cell,
/// so a press exactly on a cell boundary belongs to the cell after it.
pub fn grab_offset(grab_px_in_item: Vec2, cell_px: f32) -> UVec2 {
    (grab_px_in_item / cell_px).floor().as_uvec2()
}

/// The landing spot for a drag: the hovered cell, shifted back by whichever
/// of the item's own cells was grabbed. `hovered` is a signed cell coordinate
/// (not clamped to the board) because dragging past an edge legitimately
/// hovers a negative or overflowing cell; `fits` is what rejects those, not
/// this.
pub fn target_origin(hovered: IVec2, grab_offset: UVec2) -> IVec2 {
    hovered - grab_offset.as_ivec2()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::new();
        let mut list: Vec<Entity> = (0..count).map(|_| world.spawn_empty().id()).collect();
        list.sort();
        list
    }

    #[test]
    fn fresh_grid_is_empty() {
        for (cols, rows) in [(7, 8), (3, 3)] {
            let grid = InventoryGrid::new(cols, rows);
            for y in 0..rows {
                for x in 0..cols {
                    assert_eq!(grid.at(UVec2::new(x, y)), None);
                }
            }
        }
    }

    #[test]
    fn place_covers_every_footprint_cell() {
        let item = entities(1)[0];
        let mut grid = InventoryGrid::new(7, 8);
        grid.place(item, UVec2::new(1, 2), UVec2::new(4, 2));
        for dy in 0..2 {
            for dx in 0..4 {
                assert_eq!(grid.at(UVec2::new(1 + dx, 2 + dy)), Some(item));
            }
        }
        // A cell just outside the footprint stays empty.
        assert_eq!(grid.at(UVec2::new(5, 2)), None);
        assert_eq!(grid.at(UVec2::new(1, 4)), None);
    }

    #[test]
    fn fits_rejects_negative_and_overflowing_origin() {
        for (cols, rows) in [(7u32, 8u32), (3, 3)] {
            let grid = InventoryGrid::new(cols, rows);
            assert!(!grid.fits(IVec2::new(-1, 0), UVec2::new(1, 1), None));
            assert!(!grid.fits(IVec2::new(0, -1), UVec2::new(1, 1), None));
            assert!(!grid.fits(IVec2::new(cols as i32, 0), UVec2::new(1, 1), None));
            assert!(!grid.fits(IVec2::new(0, rows as i32), UVec2::new(1, 1), None));
            // Exactly flush against the far edge does fit.
            assert!(grid.fits(IVec2::new((cols - 1) as i32, 0), UVec2::new(1, 1), None));
            assert!(grid.fits(IVec2::new(0, (rows - 1) as i32), UVec2::new(1, 1), None));
        }
        let grid = InventoryGrid::new(7, 8);
        assert!(!grid.fits(IVec2::new(4, 0), UVec2::new(4, 1), None));
        assert!(grid.fits(IVec2::new(3, 0), UVec2::new(4, 1), None));
    }

    #[test]
    fn fits_rejects_overlap_but_ignores_self() {
        let items = entities(2);
        let (rifle, other) = (items[0], items[1]);
        let mut grid = InventoryGrid::new(7, 8);
        grid.place(rifle, UVec2::new(0, 0), UVec2::new(4, 1));

        // A different item can't land on the rifle...
        assert!(!grid.fits(IVec2::new(2, 0), UVec2::new(2, 2), Some(other)));
        // ...but the rifle itself doesn't block its own re-placement, which
        // is how a dragged item stays in the grid throughout the drag.
        assert!(grid.fits(IVec2::new(0, 0), UVec2::new(4, 1), Some(rifle)));
        assert!(grid.fits(IVec2::new(1, 0), UVec2::new(4, 1), Some(rifle)));
    }

    #[test]
    fn clear_frees_every_cell() {
        let item = entities(1)[0];
        let mut grid = InventoryGrid::new(7, 8);
        grid.place(item, UVec2::new(0, 0), UVec2::new(4, 2));
        grid.clear(item);
        for dy in 0..2 {
            for dx in 0..4 {
                assert_eq!(grid.at(UVec2::new(dx, dy)), None);
            }
        }
    }

    #[test]
    fn first_fit_finds_the_first_free_origin_row_major() {
        let item = entities(1)[0];
        let mut grid = InventoryGrid::new(3, 2);
        // Fill (0,0) and (1,0); row-major scan should skip to (2,0).
        grid.place(item, UVec2::new(0, 0), UVec2::new(2, 1));
        assert_eq!(grid.first_fit(UVec2::new(1, 1)), Some(UVec2::new(2, 0)));
        // A 2x1 item no longer fits anywhere in row 0; the next row-major
        // free origin is (0,1).
        assert_eq!(grid.first_fit(UVec2::new(2, 1)), Some(UVec2::new(0, 1)));
    }

    #[test]
    fn first_fit_returns_none_on_a_full_board() {
        let item = entities(1)[0];
        let mut grid = InventoryGrid::new(2, 2);
        grid.place(item, UVec2::new(0, 0), UVec2::new(2, 2));
        assert_eq!(grid.first_fit(UVec2::new(1, 1)), None);
    }

    #[test]
    fn has_room_for_and_free_cells_track_occupancy() {
        let item = entities(1)[0];
        let mut grid = InventoryGrid::new(3, 2);
        assert_eq!(grid.free_cells(), 6);
        assert!(grid.has_room_for(UVec2::new(3, 2)));

        grid.place(item, UVec2::new(0, 0), UVec2::new(2, 2));
        assert_eq!(grid.free_cells(), 2);
        // Two cells free, but they're a 1x2 column — no 2x1 fits.
        assert!(grid.has_room_for(UVec2::new(1, 2)));
        assert!(!grid.has_room_for(UVec2::new(2, 1)));
    }

    #[test]
    fn resize_growing_keeps_every_origin() {
        let items = entities(2);
        let mut grid = InventoryGrid::new(3, 3);
        grid.place(items[0], UVec2::new(0, 0), UVec2::new(2, 1));
        grid.place(items[1], UVec2::new(2, 2), UVec2::new(1, 1));

        let outcome = grid.resize(
            5,
            5,
            &[
                (items[0], UVec2::new(0, 0), UVec2::new(2, 1)),
                (items[1], UVec2::new(2, 2), UVec2::new(1, 1)),
            ],
        );
        assert_eq!(outcome, ResizeOutcome::default());
        assert_eq!(grid.at(UVec2::new(0, 0)), Some(items[0]));
        assert_eq!(grid.at(UVec2::new(2, 2)), Some(items[1]));
    }

    #[test]
    fn resize_shrinking_relocates_what_it_can_and_evicts_the_rest() {
        let items = entities(3);
        // Lay three 2x1 items down the left of a 2-wide board.
        let mut grid = InventoryGrid::new(2, 3);
        grid.place(items[0], UVec2::new(0, 0), UVec2::new(2, 1));
        grid.place(items[1], UVec2::new(0, 1), UVec2::new(2, 1));
        grid.place(items[2], UVec2::new(0, 2), UVec2::new(2, 1));

        // Shrink to 2x2: row 0 still fits in place, row 1 still fits in place,
        // row 2's item has nowhere to go.
        let occupants = [
            (items[0], UVec2::new(0, 0), UVec2::new(2, 1)),
            (items[1], UVec2::new(0, 1), UVec2::new(2, 1)),
            (items[2], UVec2::new(0, 2), UVec2::new(2, 1)),
        ];
        let outcome = grid.resize(2, 2, &occupants);
        assert_eq!(outcome.relocated, vec![]);
        assert_eq!(outcome.evicted, vec![items[2]]);
        assert_eq!(grid.at(UVec2::new(0, 0)), Some(items[0]));
        assert_eq!(grid.at(UVec2::new(0, 1)), Some(items[1]));

        // A shrink that forces a relocation: put a 1x1 at (2,2) on a 3x3,
        // shrink to 3x1 — it can't stay, but (2,0) is free.
        let mut grid = InventoryGrid::new(3, 3);
        grid.place(items[0], UVec2::new(0, 0), UVec2::new(2, 1));
        grid.place(items[1], UVec2::new(2, 2), UVec2::new(1, 1));
        let outcome = grid.resize(
            3,
            1,
            &[
                (items[0], UVec2::new(0, 0), UVec2::new(2, 1)),
                (items[1], UVec2::new(2, 2), UVec2::new(1, 1)),
            ],
        );
        assert_eq!(outcome.relocated, vec![(items[1], UVec2::new(2, 0))]);
        assert_eq!(outcome.evicted, vec![]);
        assert_eq!(grid.at(UVec2::new(2, 0)), Some(items[1]));
    }

    #[test]
    fn target_origin_shifts_back_by_grab_offset() {
        // A 4x1 rifle grabbed on its 4th cell (offset 3,0), hovered with
        // that cell over column 5, should land with its origin at column 2.
        assert_eq!(
            target_origin(IVec2::new(5, 0), UVec2::new(3, 0)),
            IVec2::new(2, 0)
        );
        // Grabbed on its origin cell, the origin follows the hover exactly.
        assert_eq!(
            target_origin(IVec2::new(5, 0), UVec2::new(0, 0)),
            IVec2::new(5, 0)
        );
        // Can go negative — `fits` is what rejects that, not this helper.
        assert_eq!(
            target_origin(IVec2::new(1, 0), UVec2::new(3, 0)),
            IVec2::new(-2, 0)
        );
        // Hovering past an edge (dragging the item off the board) is a
        // legitimate, signed hover coordinate, not a clamped one.
        assert_eq!(
            target_origin(IVec2::new(-1, 0), UVec2::new(0, 0)),
            IVec2::new(-1, 0)
        );
    }

    #[test]
    fn grab_offset_floors_to_containing_cell() {
        let cell = 64.0;
        assert_eq!(grab_offset(Vec2::new(0.0, 0.0), cell), UVec2::new(0, 0));
        assert_eq!(
            grab_offset(Vec2::new(cell - 0.01, 0.0), cell),
            UVec2::new(0, 0)
        );
        // Exactly on the boundary belongs to the next cell.
        assert_eq!(grab_offset(Vec2::new(cell, 0.0), cell), UVec2::new(1, 0));
        assert_eq!(
            grab_offset(Vec2::new(cell * 3.5, cell * 1.9), cell),
            UVec2::new(3, 1)
        );
    }

    #[test]
    fn hovered_cell_floors_and_goes_negative_past_the_left_edge() {
        let cell = 64.0;
        assert_eq!(hovered_cell(Vec2::new(0.0, 0.0), cell), IVec2::new(0, 0));
        assert_eq!(
            hovered_cell(Vec2::new(cell - 0.01, cell - 0.01), cell),
            IVec2::new(0, 0)
        );
        assert_eq!(hovered_cell(Vec2::new(cell, cell), cell), IVec2::new(1, 1));
        // Past the left/top edge: negative, not clamped to 0.
        assert_eq!(
            hovered_cell(Vec2::new(-1.0, -1.0), cell),
            IVec2::new(-1, -1)
        );
        assert_eq!(
            hovered_cell(Vec2::new(-cell - 1.0, 0.0), cell),
            IVec2::new(-2, 0)
        );
    }
}
