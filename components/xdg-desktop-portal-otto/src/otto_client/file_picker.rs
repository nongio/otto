//! Client for `org.otto.FilePicker1`, the file picker Otto brokers
//! `org.freedesktop.impl.portal.FileChooser` requests to.
//!
//! The same broker/renderer split as [`super::dialog`], for the same reason:
//! the portal stays a headless zbus service and the component that draws a
//! window is a separate, bus-activated process. See `specs/file-picker.md`.

use zbus::Result;

use crate::otto_client::OttoClient;

/// A filter as it goes over the wire: `(label, [(kind, pattern)])`, `kind`
/// `0` glob and `1` MIME.
pub type WireFilter = (String, Vec<(u32, String)>);

/// A choice group: `(id, label, [(option_id, option_label)], default)`.
pub type WireChoice = (String, String, Vec<(String, String)>, String);

/// The `Present` request tuple. Field for field the table in
/// `specs/file-picker.md`, which is a permanent contract — the names, the
/// types and the order do not change.
#[allow(clippy::type_complexity)]
pub type WireRequest = (
    u32,             // mode: 0 open, 1 save, 2 save-multiple
    String,          // handle
    String,          // app_id
    String,          // parent_window
    String,          // title
    String,          // accept_label
    bool,            // multiple
    bool,            // directory
    bool,            // modal
    String,          // current_name
    String,          // current_folder
    String,          // current_file
    Vec<String>,     // files
    Vec<WireFilter>, // filters
    String,          // current_filter
    Vec<WireChoice>, // choices
);

/// What `Present` answers with: `(response, uris, current_filter, choices)`.
pub type WireOutcome = (u32, Vec<String>, String, Vec<(String, String)>);

/// D-Bus proxy for `org.otto.FilePicker1` (served by otto-files).
#[zbus::proxy(
    interface = "org.otto.FilePicker1",
    default_service = "org.otto.FilePicker1",
    default_path = "/org/otto/FilePicker"
)]
trait FilePicker {
    /// Present a picker and block until the user answers or the request is
    /// withdrawn.
    ///
    /// Returns `(response, uris, current_filter, choices)`:
    /// - `response`: `0` accepted, `1` cancelled, `2` ended for another reason.
    /// - `uris`: percent-encoded absolute `file://` URIs; empty unless `0`.
    /// - `current_filter`: the label of the filter in force at acceptance.
    /// - `choices`: `[(group_id, selected_option_id)]`.
    async fn present(&self, request: WireRequest) -> Result<WireOutcome>;

    /// Withdraw a pending request: its `Present` resolves with `response = 2`.
    async fn close(&self, handle: &str) -> Result<()>;
}

impl OttoClient {
    /// Build a proxy to the picker.
    ///
    /// The picker is bus-activated, so this succeeds — and starts it — even
    /// when nothing is running yet. A failure here is a real one.
    pub async fn file_picker_proxy(&self) -> Result<FilePickerProxy<'_>> {
        FilePickerProxy::new(&self.connection).await
    }
}
