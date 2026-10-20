//! Typed client proxies for every D-Bus interface Otto calls.
//!
//! One proxy per interface, declared once, so every caller (the bar, the
//! portal, the agents daemon, `otto-msg`) speaks the same method signatures
//! instead of keeping a copy of its own or calling methods by name. Most are
//! Otto's own `org.otto.*` interfaces; the rest are the freedesktop and tray
//! interfaces Otto's apps call (UPower, logind, the portals, StatusNotifierItem,
//! dbusmenu). Each module names the interface it covers and where its server
//! lives; a new interface gets its proxy here when its server is written or
//! its first caller is.
//!
//! The crate carries no UI and no runtime: the portal, the agents daemon and
//! the command-line tools take it without the toolkit, and otto-kit re-exports
//! it as `otto_kit::dbus`. Every proxy comes in an async flavour
//! (`FooProxy`) and a blocking one (`FooProxyBlocking`).
//!
//! See `docs/developer/otto-dbus.md`.

// A `pub` item nothing outside the crate can reach is an accident: say
// `pub(crate)` or `pub(super)`, so the public API is what lib.rs exports.
#![warn(unreachable_pub)]

// Otto's own interfaces.
pub mod compositor;
pub mod desk;
pub mod dialog;
pub mod file_picker;
pub mod files;
pub mod island;
pub mod screencast;
pub mod settings;
pub mod shell;
pub mod stash;

// Interfaces other software serves.
pub mod dbusmenu;
pub mod login1;
pub mod portal_access;
pub mod portal_settings;
pub mod status_notifier_item;
pub mod upower;
