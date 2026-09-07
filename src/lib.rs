//! Reusable game mechanics extracted from the examples in this repo.
//!
//! Each module is a self-contained plugin an example (or another project) can
//! drop in. Mechanics start life inside an example, and move here once a
//! second consumer proves what's actually general about them.

pub mod inventory;
pub mod jump;
pub mod vehicle;
pub mod world_map;
