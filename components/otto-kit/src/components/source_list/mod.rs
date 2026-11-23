#![allow(clippy::module_inception)]
//! Source list: a window's left sidebar — icon, label, an optional subtitle
//! and trailing glyphs, and a selected state that fills the row and tints its
//! contents to contrast.
//!
//! Split the same way as [`list`](crate::components::list): [`SourceListItem`]
//! is the row's content, [`SourceListLayout`] the geometry shared by drawing,
//! hit-testing, the keyboard and accessibility, and [`draw`] does the actual
//! painting. [`SourceListSize`] picks the row size; its
//! [`SourceListMetrics`] hold every measurement in one place.

mod source_list;

pub use source_list::{
    draw, icon_rect, icon_tint, item_tint, GlyphSlot, SourceListItem, SourceListLayout, SourceListMetrics,
    SourceListSize,
};
