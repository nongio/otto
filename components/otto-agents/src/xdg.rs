//! The XDG base directories and the `~` spelling of a path, resolved the one
//! way every Otto program resolves them: `otto_foundations::xdg`.
//!
//! A variable is used when it names an absolute path — the spec says relative
//! values are invalid and must be ignored — and the usual place under the home
//! folder is the fallback. The runtime directory has no such fallback: it is
//! the session's, made by the login machinery.

pub use otto_foundations::xdg::{
    cache_home, config_home, data_home, home, runtime_dir, state_home, tilde, tilde_in,
};
