//! The parts of otto-emoji that do not need a screen: the emoji table, the
//! search over it, the recently-used list, the typing of a pick into the
//! focused window, and the card's scene. `main.rs` is the surface and input
//! handling around them.

pub mod data;
pub mod pan;
pub mod recents;
pub mod search;
pub mod target;
pub mod typing;
pub mod view;

pub use data::{Emoji, Group, Table, Tone, GROUPS};
pub use pan::Pan;
pub use search::rank;
pub use view::{Cell, Layout, Palette, Pane, CARD_H, CARD_W, COLUMNS};
