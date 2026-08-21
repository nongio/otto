//! Pop-up button ("dropdown"): field chrome plus its anchored menu.
//!
//! Closes roadmap gap 5 ("popup anchoring from inside a window") — see
//! `docs/developer/otto-kit-roadmap.md`.
//!
//! Split into two modules so the constraint is visible in the file layout,
//! not just documented:
//!
//! - [`field`] is the pure draw function plus a hit-test helper. No
//!   `AppContext`, `AppRunner`, `wayland-client`, or any other client-runtime
//!   dependency — the compositor draws this exact function server-side, the
//!   way it already draws `Titlebar`.
//! - [`menu`] is the client half: it owns showing a
//!   [`ContextMenu`](crate::components::context_menu::ContextMenu) anchored
//!   to the field's rect and reports the chosen index back to the caller.
//!   Reused, not written twice — see its module docs for how it avoids
//!   leaking a pointer callback per open and how selection routes back to
//!   the right dropdown when several are on screen.
//!
//! State lives with the caller, same as `toggle` and `slider`: which option
//! is selected, and one [`menu::DropdownMenu`] per dropdown field.
//!
//! `field::draw` / `field::hit_test` are deliberately namespaced rather than
//! re-exported flat — `toggle` and `slider` do the same for their plain
//! `draw`/`hit_test`, to keep the name unambiguous when several form
//! controls are in scope together.

pub mod field;
pub mod menu;

pub use field::DropdownInteraction;
pub use menu::DropdownMenu;
