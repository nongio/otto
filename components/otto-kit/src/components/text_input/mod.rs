#![allow(clippy::module_inception)]
//! Single-line editable text field with a first-class selection model.
//!
//! Split the same way as [`context_menu`](crate::components::context_menu):
//! [`TextInputState`] is the model, [`TextInputStyle`] the looks,
//! [`TextInputRenderer`] the stateless drawing and geometry, and [`TextInput`]
//! the widget that ties them together for a host to drive.

mod renderer;
mod state;
mod style;
mod text_input;

pub use renderer::TextInputRenderer;
pub use state::{Movement, TextInputState};
pub use style::TextInputStyle;
pub use text_input::{KeyMods, TextInput, TextInputKey, TextInputResponse, CARET_BLINK_PERIOD};
