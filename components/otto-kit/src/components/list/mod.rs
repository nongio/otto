#![allow(clippy::module_inception)]
//! Grouped-list card: the rounded, hairline-bordered container a settings
//! form's rows sit inside — section title, inset separators, variable-height
//! rows, and a trailing slot per row for whatever control the caller wants
//! to paint there (toggle, slider, pop-up button, ...).
//!
//! Split the same way as [`text_input`](crate::components::text_input):
//! [`ListRow`] is the row's content, [`ListLayout`] the geometry shared by
//! drawing and hit-testing, and the free functions in [`list`] do the actual
//! painting.

mod list;

pub use list::{
    default_card_background, draw, row_height, trailing_rect, ListLayout, ListRow, CORNER_RADIUS,
    LEADING_INSET, ROW_HEIGHT, ROW_HEIGHT_DETAIL, TITLE_HEIGHT, TRAILING_INSET,
};
