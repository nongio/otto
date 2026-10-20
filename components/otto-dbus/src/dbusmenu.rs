//! `com.canonical.dbusmenu`: a menu an app exports for another process to
//! draw.
//!
//! Served by tray apps (Ayatana and libappindicator items name it in their
//! `Menu` property) and read by the bar (`components/otto-bar/src/dbusmenu.rs`).
//! There is no default service or path: both come from the item that exports
//! the menu.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

/// A layout node: `(id, properties, children)`, each child a variant holding
/// another node.
pub type WireLayout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[zbus::proxy(interface = "com.canonical.dbusmenu")]
pub trait DBusMenu {
    /// Tell the app item `id` is about to show, so it can update it. Whether
    /// the layout changed.
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;

    /// Send an event (`clicked`, …) to item `id`.
    fn event(&self, id: i32, event_id: &str, data: &Value<'_>, timestamp: u32) -> zbus::Result<()>;

    /// The layout under `parent_id`, as `(revision, layout)`. A
    /// `recursion_depth` of `-1` is the whole tree; empty `property_names`
    /// asks for every property.
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, WireLayout)>;

    /// Items under `parent` came or went.
    #[zbus(signal)]
    fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;

    /// Items' properties changed: `(id, properties)` updated, `(id, names)`
    /// removed.
    #[zbus(signal)]
    fn items_properties_updated(
        &self,
        updated_props: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
}
