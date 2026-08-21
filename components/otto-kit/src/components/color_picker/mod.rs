//! Colour well and picker: closed swatch-plus-hex control, and a popup
//! offering three ways to choose a colour.
//!
//! Closes the "Colour well, and later a picker" line on the specialised
//! widget list in `docs/developer/otto-kit-roadmap.md`.
//!
//! Split the way [`dropdown`](crate::components::dropdown) is, for the same
//! reason:
//!
//! - [`well`] is the pure draw function plus a hit-test helper. No
//!   `AppContext`, `AppRunner`, `wayland-client`, or any other
//!   client-runtime dependency — the compositor draws this exact function
//!   server-side, for settings like `accent_color` that live in the
//!   compositor's own config.
//! - [`panel`] is also a pure draw function — the open picker's content
//!   (mode switcher, swatches grid, HSV square and hue strip, hex/RGB
//!   readout) — kept separate from [`popup`] so the gallery example and any
//!   future server-side host can render the open picker without a live
//!   Wayland connection either.
//! - [`popup`] is the client half: it owns a raw popup surface anchored to
//!   the well's rect and routes pointer events into `panel`'s hit-test
//!   helpers. See its module docs for why it does not reuse
//!   [`ContextMenu`](crate::components::context_menu::ContextMenu) the way
//!   `dropdown::menu` does, and for the `AppContext` reentrancy trap it
//!   inherits from that module.
//! - [`hsv`] is the RGB/HSV conversion math, tested in both directions on
//!   its own — the one part of this component pure arithmetic can get
//!   subtly wrong independent of any drawing bug.
//!
//! State lives with the caller, same as every other form control here:
//! which mode was last shown, the current colour, and one
//! [`popup::ColorPickerPopup`] per well field.
//!
//! `well::draw` / `well::hit_test` and `panel::draw` are deliberately
//! namespaced rather than re-exported flat — `toggle`, `slider` and
//! `dropdown::field` do the same, to keep `draw` unambiguous when several
//! form controls are in scope together.

pub mod hsv;
pub mod panel;
pub mod popup;
pub mod well;

pub use panel::{HexField, Mode, Swatch};
pub use popup::ColorPickerPopup;
pub use well::WellInteraction;
