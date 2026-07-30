//! The panel Otto shows when it needs a password.
//!
//! Two clients present the same panel for different reasons:
//!
//! * **otto-greeter** — logging in. Talks to greetd over `$GREETD_SOCK`, on a
//!   `wlr-layer-shell` overlay surface. See `specs/login-mode.md`.
//! * **a lock screen** — unlocking a session that already exists. Talks to
//!   PAM, on `ext-session-lock-v1` surfaces.
//!
//! Neither backend appears in this crate. A client translates whatever
//! conversation it is having into a [`View`] and hands it to [`Panel::update`],
//! then asks [`Panel::action_at`] where a click landed rather than duplicating
//! the layout. That is the whole interface: what the two clients
//! share is the drawing, and what differs — the protocol, the authentication,
//! the session picker — stays with them.
//!
//! Sizes are logical points, on a canvas the caller has already scaled.

mod appearance;
mod panel;
mod user;

pub use appearance::Appearance;
pub use panel::{Action, Field, Finger, Panel, PowerAction, Status, View};
pub use user::User;
