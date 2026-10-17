//! `org.otto.Stash1`: the things collected to ask about in Ask.
//!
//! Served by otto-stash (`components/otto-stash/src/dbus.rs`). Items travel
//! as file paths, each with whether it is struck out (kept, but not sent).

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Stash1";
/// The object path.
pub const PATH: &str = "/org/otto/Stash1";

#[zbus::proxy(
    interface = "org.otto.Stash1",
    default_service = "org.otto.Stash1",
    default_path = "/org/otto/Stash1"
)]
pub trait Stash {
    /// Add what is selected in the focused app; returns once it is in the
    /// stash, or it turned out nothing was selected.
    fn add(&self) -> zbus::Result<()>;

    /// Add a file. `path` must be absolute.
    fn add_file(&self, path: &str) -> zbus::Result<()>;

    /// Let the user pick a screen region to add.
    fn add_region(&self) -> zbus::Result<()>;

    /// Open Ask with what is stashed.
    fn send(&self) -> zbus::Result<()>;

    /// Drop the stash without sending it.
    fn cancel(&self) -> zbus::Result<()>;

    /// Everything stashed, oldest first: text as `selection-N.txt`, each with
    /// whether it is struck out.
    fn items(&self) -> zbus::Result<Vec<(String, bool)>>;

    /// The caller shows the stash: its card steps aside until the caller
    /// leaves the bus.
    fn hold(&self) -> zbus::Result<()>;

    /// Strike the item at `index` out, or bring it back.
    fn toggle(&self, index: u32) -> zbus::Result<()>;

    /// Take the item at `index` out of the stash.
    fn remove(&self, index: u32) -> zbus::Result<()>;

    /// What is stashed went to Ask: the stash is over.
    fn sent(&self) -> zbus::Result<()>;

    /// Everything stashed, after every change.
    #[zbus(signal)]
    fn changed(&self, items: Vec<(String, bool)>) -> zbus::Result<()>;
}
