//! Tiling: the tree, the layout that resolves it to rectangles, and the
//! per-workspace state that holds both.
//!
//! `tree` and `layout` are pure — no Smithay, no lay-rs, no pixels beyond a
//! plain rectangle — so they are unit-testable on their own with
//! `cargo test --lib tiling`. The compositor side (animating windows into
//! their cells, xdg states, hooks into map/unmap/focus) lives in
//! `src/shell/tiling.rs`.
//!
//! See `specs/tiling.md` for the behaviour and
//! `docs/developer/tiling-plan.md` for how the phases fit together.

pub mod layout;
pub mod state;
pub mod tree;

pub use layout::{Gaps, Rect};
pub use state::TilingState;
pub use tree::{Axis, Direction};
