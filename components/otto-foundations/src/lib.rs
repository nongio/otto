//! The base every Otto crate can link: no UI, no async runtime, no D-Bus.
//!
//! What lives here is what several programs have to agree on and used to work
//! out each on its own, slightly differently each time:
//!
//! - [`xdg`] resolves the XDG base directories, the `user-dirs.dirs` folders
//!   and the `~` spelling of a path under the home folder.
//! - [`uri`] turns paths into `file://` URIs and back.
//! - [`matching`] scores typed text against a name.
//! - [`desktop_file`] reads and edits the `[Desktop Entry]` group of a
//!   desktop entry file.
//! - [`user`] names the user the session belongs to.
//!
//! otto-kit re-exports them all (`otto_kit::xdg`, `otto_kit::uri`,
//! `otto_kit::matching`, `otto_kit::desktop_file`, `otto_kit::user`), so an
//! app imports them from the toolkit; a program that does not link the
//! toolkit (the portal, the agents daemon, otto-search) depends on this crate
//! directly.

// A `pub` item nothing outside the crate can reach is an accident: say
// `pub(crate)` or `pub(super)`, so the public API is what lib.rs exports.
#![warn(unreachable_pub)]

pub mod desktop_file;
pub mod matching;
pub mod uri;
pub mod user;
pub mod xdg;
