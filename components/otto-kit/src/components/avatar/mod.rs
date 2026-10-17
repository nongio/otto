#![allow(clippy::module_inception)]
//! Avatar: a person drawn as a circle — their picture when they have one,
//! otherwise their initials on a color picked from their name, otherwise a
//! generic head-and-shoulders silhouette.
//!
//! The component decodes nothing. The caller passes an already-decoded
//! [`skia_safe::Image`] (or `None`), so it decides how pictures are cached.

mod avatar;

#[doc(inline)]
pub use avatar::{draw, ground_color, initials};
