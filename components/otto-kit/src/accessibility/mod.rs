//! Accessibility: what a kit application tells assistive technologies.
//!
//! A screen reader learns about an application over AT-SPI, the freedesktop
//! accessibility bus. Otto-kit speaks it through AccessKit, which owns the
//! D-Bus object model; what the toolkit provides is the part AccessKit cannot
//! know — the tree itself, in [`tree`], and where it is plugged into the run
//! loop, in `adapter`.
//!
//! An application opts in per surface:
//!
//! 1. `AppContext::enable_accessibility(&surface_id)`, once the surface exists.
//! 2. Implement [`crate::App::accessibility`], returning an [`A11yTree`].
//! 3. Implement [`crate::App::on_accessibility_action`] to act on what an
//!    assistive technology asks for — a click, a value, a focus move.
//!
//! Nothing is built unless something is listening: with no assistive
//! technology attached the tree closure is never called.

mod adapter;
pub mod tree;
mod widgets;

pub(crate) use adapter::SurfaceAdapter;
pub use tree::{node_id, A11yTree, ROOT};

// Re-exported so an application describing its interface does not have to
// depend on `accesskit` itself, and cannot end up on a different version of it
// than the toolkit.
pub use accesskit::{
    Action, ActionData, ActionRequest, HasPopup, Live, Node, Orientation, Rect, Role,
    TextSelection, Toggled,
};
