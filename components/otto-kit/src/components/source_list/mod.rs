#![allow(clippy::module_inception)]
//! Source list: the settings window's left sidebar — icon, label, and a
//! selected state that fills the row and tints its contents to contrast.
//!
//! Split the same way as [`list`](crate::components::list): [`SourceListItem`]
//! is the row's content, [`SourceListLayout`] the geometry shared by drawing
//! and hit-testing, and [`draw`] does the actual painting.

mod source_list;

pub use source_list::{draw, SourceListItem, SourceListLayout};
pub use source_list::{
    icon_rect, item_tint, ICON_SIZE, ITEM_HEIGHT, ITEM_INSET, ITEM_STEP, LABEL_INSET,
};
