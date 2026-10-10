//! The base every Otto crate can link: no UI, no async runtime, no D-Bus.
//!
//! What lives here is what several programs have to agree on and used to work
//! out each on its own, slightly differently each time:
//!
//! - [`xdg`] resolves the XDG base directories, the `user-dirs.dirs` folders
//!   and the `~` spelling of a path under the home folder.
//! - [`uri`] turns paths into `file://` URIs and back.
//! - [`matching`] scores typed text against a name.
//!
//! otto-kit re-exports all three (`otto_kit::xdg`, `otto_kit::uri`,
//! `otto_kit::matching`), so an app imports them from the toolkit; a program
//! that does not link the toolkit (the portal, the agents daemon, otto-search)
//! depends on this crate directly.

pub mod matching;
pub mod uri;
pub mod xdg;
