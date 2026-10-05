//! `org.otto.FilePicker1`: the file picker the portal brokers
//! `org.freedesktop.impl.portal.FileChooser` requests to.
//!
//! Served by otto-files (`components/otto-files/src/dbus.rs`); the request
//! tuple is the permanent contract in `specs/file-picker.md`.

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.FilePicker1";
/// The object path.
pub const PATH: &str = "/org/otto/FilePicker";

/// A filter: `(label, [(kind, pattern)])`, `kind` `0` glob and `1` MIME.
pub type WireFilter = (String, Vec<(u32, String)>);

/// A choice group: `(id, label, [(option_id, option_label)], default)`.
pub type WireChoice = (String, String, Vec<(String, String)>, String);

/// The `Present` request, field for field the table in
/// `specs/file-picker.md`.
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

/// What `Present` answers: `(response, uris, current_filter, choices)`.
pub type WireOutcome = (u32, Vec<String>, String, Vec<(String, String)>);

#[zbus::proxy(
    interface = "org.otto.FilePicker1",
    default_service = "org.otto.FilePicker1",
    default_path = "/org/otto/FilePicker"
)]
pub trait FilePicker {
    /// Present a picker and return once the user answers or the request is
    /// withdrawn. `response` is `0` accepted, `1` cancelled, `2` ended; `uris`
    /// are percent-encoded `file://` URIs, empty unless accepted.
    fn present(&self, request: WireRequest) -> zbus::Result<WireOutcome>;

    /// Withdraw a pending request: its `Present` answers with `2`.
    fn close(&self, handle: &str) -> zbus::Result<()>;
}
