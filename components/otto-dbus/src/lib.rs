//! Typed client proxies for Otto's own D-Bus interfaces.
//!
//! One proxy per `org.otto.*` interface, declared once, so every caller —
//! the bar, the portal, the agents daemon, `otto-msg` — speaks the same
//! method signatures instead of keeping a copy of its own or calling methods
//! by name. Each module names the interface it covers and where its server
//! lives; a new interface gets its proxy here when its server is written.
//!
//! The crate carries no UI and no runtime: the portal, the agents daemon and
//! the command-line tools take it without the toolkit, and otto-kit re-exports
//! it as `otto_kit::dbus`. Every proxy comes in an async flavour
//! (`FooProxy`) and a blocking one (`FooProxyBlocking`).
//!
//! See `docs/developer/otto-dbus.md`.

pub mod compositor;
pub mod desk;
pub mod dialog;
pub mod file_picker;
pub mod files;
pub mod island;
pub mod screencast;
pub mod settings;
pub mod shell;
