//! Client for `org.otto.FilePicker1`, the file picker Otto brokers
//! `org.freedesktop.impl.portal.FileChooser` requests to.
//!
//! The same broker/renderer split as [`super::dialog`], for the same reason:
//! the portal stays a headless zbus service and the component that draws a
//! window is a separate, bus-activated process. See `specs/file-picker.md`.

use zbus::Result;

use crate::otto_client::OttoClient;
pub use otto_dbus::file_picker::{FilePickerProxy, WireChoice, WireFilter, WireRequest};

impl OttoClient {
    /// Build a proxy to the picker.
    ///
    /// The picker is bus-activated, so this succeeds — and starts it — even
    /// when nothing is running yet. A failure here is a real one.
    pub async fn file_picker_proxy(&self) -> Result<FilePickerProxy<'_>> {
        FilePickerProxy::new(&self.connection).await
    }
}
