//! `org.freedesktop.impl.portal.FileChooser` backend.
//!
//! The portal is the broker, otto-files is the renderer — the shape
//! [`AccessPortal`](crate::portal::AccessPortal) already established, and for
//! the same reasons: this binary stays a headless zbus service, and the
//! component that opens a window is a separate, bus-activated process.
//!
//! The job here is mechanical and must not lose information: unpack the
//! freedesktop `a{sv}` options into the typed `org.otto.FilePicker1` tuple,
//! and pack the answer back. It does not interpret filters, resolve paths, or
//! check that the returned files exist — that is the picker's contract with
//! the user. See `specs/file-picker.md`.

use std::collections::HashMap;

use tracing::{info, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{interface, ObjectServer};

use crate::otto_client::file_picker::{WireChoice, WireFilter, WireRequest};
use crate::otto_client::OttoClient;

/// Mode values of the picker's wire contract.
const MODE_OPEN: u32 = 0;
const MODE_SAVE: u32 = 1;
const MODE_SAVE_FILES: u32 = 2;

/// The request object the frontend calls `Close` on when the requesting
/// application withdraws — because it exited, or because the user closed the
/// window that asked.
///
/// Unlike the portal's shared [`Request`](crate::portal::Request), which only
/// logs, this one forwards: without it a dialog whose application has gone
/// stays on screen until somebody dismisses it by hand.
struct FileChooserRequest {
    client: OttoClient,
    handle: String,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl FileChooserRequest {
    async fn close(&self) {
        info!(handle = %self.handle, "FileChooser request withdrawn");
        match self.client.file_picker_proxy().await {
            // Activating the picker just to tell it to close a request it
            // never received would be absurd, but the proxy is cheap and the
            // call is a no-op against an unknown handle.
            Ok(proxy) => {
                if let Err(err) = proxy.close(&self.handle).await {
                    warn!(?err, handle = %self.handle, "withdrawing the request failed");
                }
            }
            Err(err) => warn!(?err, "file picker unreachable while withdrawing"),
        }
    }
}

pub struct FileChooserPortal {
    client: OttoClient,
}

impl FileChooserPortal {
    pub fn new(client: OttoClient) -> Self {
        Self { client }
    }

    /// Hand one request to the picker and translate the answer back.
    ///
    /// Every failure path returns `response = 2` with no URIs. It never
    /// returns `0` with nothing: an application told "here is your file" and
    /// handed an empty list will do something worse than fail.
    async fn present(
        &self,
        object_server: &ObjectServer,
        path: &OwnedObjectPath,
        request: WireRequest,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let handle = request.1.clone();

        // Exported before the call, removed after it: the window is only
        // withdrawable while it is up.
        let withdrawable = object_server
            .at(
                path.clone(),
                FileChooserRequest {
                    client: self.client.clone(),
                    handle: handle.clone(),
                },
            )
            .await
            .unwrap_or_else(|err| {
                warn!(?err, %handle, "could not export the request object");
                false
            });

        let result = self.present_inner(request).await;

        if withdrawable {
            if let Err(err) = object_server
                .remove::<FileChooserRequest, _>(path.clone())
                .await
            {
                warn!(?err, %handle, "could not remove the request object");
            }
        }
        result
    }

    async fn present_inner(&self, request: WireRequest) -> (u32, HashMap<String, OwnedValue>) {
        let handle = request.1.clone();
        let proxy = match self.client.file_picker_proxy().await {
            Ok(proxy) => proxy,
            Err(err) => {
                warn!(?err, "file picker unreachable");
                return (2, HashMap::new());
            }
        };

        match proxy.present(request).await {
            Ok((response, uris, current_filter, choices)) => {
                info!(%handle, response, count = uris.len(), "file picker answered");
                let mut results = HashMap::new();
                if response == 0 {
                    insert(&mut results, "uris", Value::from(uris));
                    if !current_filter.is_empty() {
                        // The frontend expects the filter in its own shape.
                        // Only its label survives the round trip, which is
                        // all an application uses it for — deciding an output
                        // format from the filter the user chose.
                        let filter: WireFilter = (current_filter, Vec::new());
                        insert(&mut results, "current_filter", Value::from(filter));
                    }
                    if !choices.is_empty() {
                        insert(&mut results, "choices", Value::from(choices));
                    }
                }
                (response, results)
            }
            Err(err) => {
                // Includes the picker dying mid-request: the peer disappears,
                // the call errors, and the requesting application is told the
                // request ended rather than being left waiting forever.
                warn!(?err, %handle, "file picker call failed");
                (2, HashMap::new())
            }
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserPortal {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }

    async fn open_file(
        &self,
        #[zbus(object_server)] object_server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        info!(?app_id, %title, "OpenFile called");
        let request = build_request(
            MODE_OPEN,
            handle.clone(),
            app_id,
            parent_window,
            title,
            options,
        );
        self.present(object_server, &handle, request).await
    }

    async fn save_file(
        &self,
        #[zbus(object_server)] object_server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        info!(?app_id, %title, "SaveFile called");
        let request = build_request(
            MODE_SAVE,
            handle.clone(),
            app_id,
            parent_window,
            title,
            options,
        );
        self.present(object_server, &handle, request).await
    }

    async fn save_files(
        &self,
        #[zbus(object_server)] object_server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        info!(?app_id, %title, "SaveFiles called");
        let request = build_request(
            MODE_SAVE_FILES,
            handle.clone(),
            app_id,
            parent_window,
            title,
            options,
        );
        self.present(object_server, &handle, request).await
    }
}

/// Pack a freedesktop request into the picker's tuple.
fn build_request(
    mode: u32,
    handle: OwnedObjectPath,
    app_id: String,
    parent_window: String,
    title: String,
    options: HashMap<String, OwnedValue>,
) -> WireRequest {
    (
        mode,
        handle.as_str().to_string(),
        app_id,
        parent_window,
        title,
        string_opt(&options, "accept_label").unwrap_or_default(),
        bool_opt(&options, "multiple").unwrap_or(false),
        bool_opt(&options, "directory").unwrap_or(false),
        bool_opt(&options, "modal").unwrap_or(true),
        string_opt(&options, "current_name").unwrap_or_default(),
        path_opt(&options, "current_folder").unwrap_or_default(),
        path_opt(&options, "current_file").unwrap_or_default(),
        strings_opt(&options, "files"),
        filters_opt(&options, "filters"),
        // `current_filter` arrives as a whole filter; only its label
        // identifies it.
        options
            .get("current_filter")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| WireFilter::try_from(v).ok())
            .map(|(label, _)| label)
            .unwrap_or_default(),
        choices_opt(&options, "choices"),
    )
}

fn insert(results: &mut HashMap<String, OwnedValue>, key: &str, value: Value<'_>) {
    match OwnedValue::try_from(value) {
        Ok(owned) => {
            results.insert(key.to_string(), owned);
        }
        Err(err) => warn!(key, ?err, "dropping result value"),
    }
}

/// `OwnedValue` is not `Clone`, so every read goes through `try_clone`.
fn string_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| String::try_from(v).ok())
}

fn bool_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    options.get(key).and_then(|v| bool::try_from(v).ok())
}

fn strings_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    options
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default()
}

fn filters_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Vec<WireFilter> {
    options
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<WireFilter>::try_from(v).ok())
        .unwrap_or_default()
}

fn choices_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Vec<WireChoice> {
    options
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<WireChoice>::try_from(v).ok())
        .unwrap_or_default()
}

/// A path option, which the portal sends as `ay` — a NUL-terminated byte
/// string, not a D-Bus string.
///
/// The trailing NUL is stripped. A non-absolute or non-UTF-8 path is dropped
/// rather than rejected: the picker then falls back as if the hint were
/// absent, which is friendlier than failing a whole request over it.
fn path_opt(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let bytes = options
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<u8>::try_from(v).ok())?;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
    let path = String::from_utf8(bytes.to_vec()).ok()?;
    path.starts_with('/').then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(pairs: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        pairs
            .into_iter()
            .filter_map(|(k, v)| Some((k.to_string(), OwnedValue::try_from(v).ok()?)))
            .collect()
    }

    #[test]
    fn a_path_option_loses_its_trailing_nul() {
        let opts = options(vec![(
            "current_folder",
            Value::from(b"/tmp/pics\0".to_vec()),
        )]);
        assert_eq!(
            path_opt(&opts, "current_folder").as_deref(),
            Some("/tmp/pics")
        );
    }

    #[test]
    fn a_path_option_without_a_nul_is_still_read() {
        let opts = options(vec![("current_folder", Value::from(b"/tmp/pics".to_vec()))]);
        assert_eq!(
            path_opt(&opts, "current_folder").as_deref(),
            Some("/tmp/pics")
        );
    }

    #[test]
    fn a_relative_path_option_is_dropped() {
        let opts = options(vec![("current_folder", Value::from(b"pics\0".to_vec()))]);
        assert!(path_opt(&opts, "current_folder").is_none());
    }

    #[test]
    fn a_non_utf8_path_option_is_dropped_rather_than_mangled() {
        let opts = options(vec![(
            "current_folder",
            Value::from(b"/tmp/\xff\0".to_vec()),
        )]);
        assert!(path_opt(&opts, "current_folder").is_none());
    }

    #[test]
    fn an_absent_option_takes_its_default() {
        let request = build_request(
            MODE_OPEN,
            OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/x/1").unwrap(),
            "org.example.App".into(),
            String::new(),
            String::new(),
            HashMap::new(),
        );
        assert_eq!(request.0, MODE_OPEN);
        assert!(!request.6, "multiple defaults to false");
        assert!(!request.7, "directory defaults to false");
        assert!(request.8, "modal defaults to true");
        assert!(request.13.is_empty());
    }

    #[test]
    fn filters_and_the_preselected_one_survive_the_translation() {
        let filters: Vec<WireFilter> = vec![
            ("Text".into(), vec![(0u32, "*.txt".into())]),
            ("Images".into(), vec![(1u32, "image/png".into())]),
        ];
        let current: WireFilter = ("Images".into(), vec![(1u32, "image/png".into())]);
        let opts = options(vec![
            ("filters", Value::from(filters.clone())),
            ("current_filter", Value::from(current)),
            ("multiple", Value::from(true)),
        ]);
        let request = build_request(
            MODE_OPEN,
            OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/x/1").unwrap(),
            "org.example.App".into(),
            String::new(),
            String::new(),
            opts,
        );
        assert_eq!(request.13, filters);
        assert_eq!(request.14, "Images");
        assert!(request.6);
    }

    #[test]
    fn the_request_handle_is_carried_through_so_close_can_find_it() {
        let path = "/org/freedesktop/portal/desktop/request/sender/token";
        let request = build_request(
            MODE_OPEN,
            OwnedObjectPath::try_from(path).unwrap(),
            String::new(),
            String::new(),
            String::new(),
            HashMap::new(),
        );
        assert_eq!(request.1, path);
    }
}
