//! `org.otto.Shell1`: i3/sway-style commands, trees and events.
//!
//! Served by the compositor (`src/shell_service.rs`); the contract is
//! `docs/developer/shell-dbus-api.md`. The answers and the signal payloads
//! are JSON strings in i3's (or sway's) shapes.

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Shell1";
/// The object path.
pub const PATH: &str = "/org/otto/Shell1";

#[zbus::proxy(
    interface = "org.otto.Shell1",
    default_service = "org.otto.Shell1",
    default_path = "/org/otto/Shell1"
)]
pub trait Shell {
    /// Run a `;`-separated i3-syntax command string; one `(success, message)`
    /// per command.
    fn run_command(&self, command: &str) -> zbus::Result<Vec<(bool, String)>>;

    /// The whole tree, in i3's `GET_TREE` shape.
    fn get_tree(&self) -> zbus::Result<String>;

    /// Every workspace, in i3's `GET_WORKSPACES` shape.
    fn get_workspaces(&self) -> zbus::Result<String>;

    /// Every output, in i3's `GET_OUTPUTS` shape.
    fn get_outputs(&self) -> zbus::Result<String>;

    /// The keyboard, in sway's `GET_INPUTS` shape.
    fn get_inputs(&self) -> zbus::Result<String>;

    /// i3's `workspace` event.
    #[zbus(signal)]
    fn workspace_changed(&self, event: String) -> zbus::Result<()>;

    /// i3's `window` event.
    #[zbus(signal)]
    fn window_changed(&self, event: String) -> zbus::Result<()>;

    /// sway's `input` event.
    #[zbus(signal)]
    fn input_changed(&self, event: String) -> zbus::Result<()>;
}
