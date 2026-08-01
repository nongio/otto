//! `restore_data` encoding for the ScreenCast impl portal.
//!
//! The spec's session-persistence handshake runs entirely through the
//! frontend: the backend returns `restore_data` — `(vendor, version, data)` —
//! from `Start`, xdg-desktop-portal hands the app an opaque token for it, and
//! on a later `SelectSources` the same tuple comes back to us. Only this
//! backend has to understand `data`, so it carries the picked source verbatim
//! and needs no state kept in the portal process.
//!
//! Without this, an app that re-creates its session (Chrome does, between the
//! preview it renders in its own picker and the real capture) is prompted
//! again for a source the user already approved.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Str, Value};

use crate::portal::interface::SourceSelection;
use crate::portal::{SOURCE_TYPE_MONITOR, SOURCE_TYPE_WINDOW};

/// Vendor name in the `(suv)` tuple. Data written by another desktop's portal
/// carries its own name and is rejected on sight.
pub const RESTORE_VENDOR: &str = "otto";

/// Version of this backend's private `data` payload.
pub const RESTORE_VERSION: u32 = 1;

/// A source selection as it survives across sessions.
///
/// Only the identity is stored. Sizes, outputs and titles are resolved afresh
/// on restore, because the window may have moved or been resized since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoredSource {
    Monitor(String),
    Window(String),
}

impl RestoredSource {
    /// The `SOURCE_TYPE_*` bit this source answers to.
    pub fn source_type(&self) -> u32 {
        match self {
            RestoredSource::Monitor(_) => SOURCE_TYPE_MONITOR,
            RestoredSource::Window(_) => SOURCE_TYPE_WINDOW,
        }
    }
}

/// Build the `(suv)` value to return from `Start`.
pub fn encode_restore_data(source: &RestoredSource) -> Result<OwnedValue, zbus::zvariant::Error> {
    let (source_type, id) = match source {
        RestoredSource::Monitor(connector) => (SOURCE_TYPE_MONITOR, connector.clone()),
        RestoredSource::Window(id) => (SOURCE_TYPE_WINDOW, id.clone()),
    };

    let mut data: HashMap<&str, Value<'_>> = HashMap::new();
    data.insert("source-type", Value::U32(source_type));
    data.insert("id", Value::Str(Str::from(id)));

    // The third field is a *variant*, not the dict itself — the tuple has to
    // marshal as `(suv)` or the frontend rejects it.
    let tuple = Value::from((
        Str::from_static(RESTORE_VENDOR),
        RESTORE_VERSION,
        Value::Value(Box::new(Value::from(data))),
    ));
    OwnedValue::try_from(tuple)
}

/// Peel any number of variant wrappers off a value.
///
/// How deeply a value ends up nested depends on who marshalled it — the dict
/// values in an `a{sv}` are variants, and the frontend re-wraps the payload
/// when it stores and replays it — so unwrap until something concrete appears.
fn unwrap_variants<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    let mut current = value;
    while let Value::Value(inner) = current {
        current = inner.as_ref();
    }
    current
}

/// Read back a `(suv)` produced by [`encode_restore_data`].
///
/// Returns `None` for anything this backend did not write — another desktop's
/// portal, a future payload version, or a malformed tuple. The spec requires
/// exactly that: an unreadable restore payload falls back to prompting.
pub fn decode_restore_data(value: &OwnedValue) -> Option<RestoredSource> {
    let Value::Structure(structure) = &**value else {
        return None;
    };
    let fields = structure.fields();
    let [vendor, version, data] = fields else {
        return None;
    };

    if <&str>::try_from(vendor).ok()? != RESTORE_VENDOR {
        return None;
    }
    if u32::try_from(version).ok()? != RESTORE_VERSION {
        return None;
    }

    let Value::Dict(dict) = unwrap_variants(data) else {
        return None;
    };
    let source_type = match unwrap_variants(&dict.get::<_, Value<'_>>(&"source-type").ok()??) {
        Value::U32(value) => *value,
        _ => return None,
    };
    let id = match unwrap_variants(&dict.get::<_, Value<'_>>(&"id").ok()??) {
        Value::Str(value) => value.to_string(),
        _ => return None,
    };
    if id.is_empty() {
        return None;
    }

    match source_type {
        SOURCE_TYPE_MONITOR => Some(RestoredSource::Monitor(id)),
        SOURCE_TYPE_WINDOW => Some(RestoredSource::Window(id)),
        _ => None,
    }
}

/// Turn a restored source back into a live selection.
///
/// The source is only accepted when the app is still allowed to ask for that
/// type *and* the source is still there — a monitor that was unplugged or a
/// window that was closed must fall through to the picker rather than fail the
/// request.
pub fn resolve_restored(
    source: RestoredSource,
    requested_types: u32,
    outputs: &[String],
    windows: &[crate::otto_client::screencast::WindowSource],
) -> Option<SourceSelection> {
    if requested_types & source.source_type() == 0 {
        return None;
    }

    match source {
        RestoredSource::Monitor(connector) if outputs.contains(&connector) => {
            Some(SourceSelection::Monitor(connector))
        }
        RestoredSource::Monitor(_) => None,
        RestoredSource::Window(id) => windows
            .iter()
            .find(|window| window.id == id)
            .cloned()
            .map(SourceSelection::Window),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otto_client::screencast::WindowSource;

    fn window(id: &str) -> WindowSource {
        WindowSource {
            id: id.to_string(),
            app_id: "app".to_string(),
            title: "title".to_string(),
        }
    }

    #[test]
    fn round_trips_a_window() {
        let source = RestoredSource::Window("toplevel-1".to_string());
        let encoded = encode_restore_data(&source).unwrap();
        assert_eq!(decode_restore_data(&encoded), Some(source));
    }

    #[test]
    fn marshals_as_the_signature_the_spec_asks_for() {
        let encoded =
            encode_restore_data(&RestoredSource::Window("toplevel-1".to_string())).unwrap();
        assert_eq!(encoded.value_signature().to_string(), "(suv)");
    }

    #[test]
    fn round_trips_a_monitor() {
        let source = RestoredSource::Monitor("eDP-1".to_string());
        let encoded = encode_restore_data(&source).unwrap();
        assert_eq!(decode_restore_data(&encoded), Some(source));
    }

    #[test]
    fn rejects_another_vendor() {
        let mut data: HashMap<&str, Value<'_>> = HashMap::new();
        data.insert("source-type", Value::U32(SOURCE_TYPE_WINDOW));
        data.insert("id", Value::Str(Str::from_static("toplevel-1")));
        let tuple = Value::from((Str::from_static("GNOME"), 1u32, Value::from(data)));
        let encoded = OwnedValue::try_from(tuple).unwrap();
        assert_eq!(decode_restore_data(&encoded), None);
    }

    #[test]
    fn rejects_a_future_payload_version() {
        let mut data: HashMap<&str, Value<'_>> = HashMap::new();
        data.insert("source-type", Value::U32(SOURCE_TYPE_WINDOW));
        data.insert("id", Value::Str(Str::from_static("toplevel-1")));
        let tuple = Value::from((
            Str::from_static(RESTORE_VENDOR),
            RESTORE_VERSION + 1,
            Value::from(data),
        ));
        let encoded = OwnedValue::try_from(tuple).unwrap();
        assert_eq!(decode_restore_data(&encoded), None);
    }

    #[test]
    fn resolves_a_window_that_is_still_open() {
        let selection = resolve_restored(
            RestoredSource::Window("toplevel-1".to_string()),
            SOURCE_TYPE_WINDOW,
            &[],
            &[window("toplevel-1")],
        );
        assert!(matches!(
            selection,
            Some(SourceSelection::Window(w)) if w.id == "toplevel-1"
        ));
    }

    #[test]
    fn drops_a_window_that_is_gone() {
        let selection = resolve_restored(
            RestoredSource::Window("toplevel-1".to_string()),
            SOURCE_TYPE_WINDOW,
            &[],
            &[window("toplevel-2")],
        );
        assert!(selection.is_none());
    }

    #[test]
    fn drops_a_source_of_an_unrequested_type() {
        let selection = resolve_restored(
            RestoredSource::Monitor("eDP-1".to_string()),
            SOURCE_TYPE_WINDOW,
            &["eDP-1".to_string()],
            &[],
        );
        assert!(selection.is_none());
    }
}
