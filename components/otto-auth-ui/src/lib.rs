//! The panel Otto shows when it needs a password.
//!
//! Two clients present the same panel for different reasons:
//!
//! * **otto-greeter** — logging in. Talks to greetd over `$GREETD_SOCK`, on a
//!   `wlr-layer-shell` overlay surface. See `specs/login-mode.md`.
//! * **a lock screen** — unlocking a session that already exists. Talks to
//!   PAM, on `ext-session-lock-v1` surfaces.
//! * **otto-authorize** — confirming a change to a sensitive setting with the
//!   user's password. Talks to PAM, on a layer-shell overlay, and shows the
//!   panel as a dialog ([`Panel::new_dialog`]) with Otto's reason and Cancel.
//!
//! The drawing knows nothing of greetd or PAM. A client translates whatever
//! conversation it is having into a [`View`] and hands it to [`Panel::update`],
//! then asks [`Panel::action_at`] where a click landed rather than duplicating
//! the layout. What the clients share is the drawing, and what differs — the
//! protocol, the session picker — stays with them. The one exception is the
//! PAM conversation, which otto-lock and otto-authorize both need word for
//! word: it lives here as [`pam`], behind the `pam` feature, so the greeter
//! does not link libpam.
//!
//! Sizes are logical points, on a canvas the caller has already scaled.

mod appearance;
#[cfg(feature = "pam")]
pub mod pam;
mod panel;
pub mod power;
pub mod reader;
mod secret;
mod user;

pub use appearance::Appearance;
pub use panel::{Action, Field, Finger, Panel, PowerAction, Status, View};
pub use secret::SecretInput;
pub use user::User;
/// Re-exported so a client can hold a taken answer without naming the crate.
pub use zeroize::Zeroizing;
