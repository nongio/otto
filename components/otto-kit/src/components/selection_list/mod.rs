#![allow(clippy::module_inception)]
//! Selection list: a card of selectable two-line items with an add/remove
//! footer — the list half of a list-and-detail layout, such as the users in
//! a settings window.
//!
//! Split the same way as [`list`](crate::components::list):
//! [`SelectionListItem`] is an item's content, [`SelectionListLayout`] the
//! geometry shared by drawing and hit-testing, and [`draw`] does the painting.
//! Each item has a leading square the caller paints, typically with an
//! [`avatar`](crate::components::avatar).

mod selection_list;

#[doc(inline)]
pub use selection_list::{
    draw, leading_rect, SelectionListHit, SelectionListItem, SelectionListLayout,
    SelectionListState, FOOTER_HEIGHT, ITEM_HEIGHT, ITEM_STEP, LEADING_SIZE,
};
