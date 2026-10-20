//! `org.kde.StatusNotifierItem`: a tray icon, as the app that owns it
//! exports it.
//!
//! Served by tray apps; the bar is the watcher and host they register with
//! (`components/otto-bar/src/tray.rs`). There is no default service: the
//! bus name comes from the item's registration, and so may the path.

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// The object path of an item registered by bus name alone.
pub const PATH: &str = "/StatusNotifierItem";

#[zbus::proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem"
)]
pub trait StatusNotifierItem {
    /// A freedesktop icon name; empty when the item sends a pixmap.
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    /// The icon as ARGB32 pixmaps, `(width, height, bytes)`, in sizes of the
    /// item's choosing.
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    /// An extra directory to look `icon_name` up in.
    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;

    /// `(icon_name, icon_pixmap, title, body)`.
    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> zbus::Result<OwnedValue>;

    /// `Passive`, `Active` or `NeedsAttention`.
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    /// The item's name, for people.
    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    /// The item's name, for programs.
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    /// The `com.canonical.dbusmenu` object for the item's menu.
    #[zbus(property, name = "Menu")]
    fn menu(&self) -> zbus::Result<OwnedObjectPath>;

    /// Primary click, at screen position `(x, y)`.
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Ask the item to show its own context menu.
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Middle click.
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// The icon changed.
    #[zbus(signal)]
    fn new_icon(&self) -> zbus::Result<()>;

    /// The status changed.
    #[zbus(signal)]
    fn new_status(&self, status: &str) -> zbus::Result<()>;

    /// The tooltip changed.
    #[zbus(signal)]
    fn new_tool_tip(&self) -> zbus::Result<()>;
}
