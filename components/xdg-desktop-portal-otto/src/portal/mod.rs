//! Portal module implementing XDG Desktop Portal backends.
//!
//! This module provides D-Bus interface implementations for:
//! - `org.freedesktop.impl.portal.ScreenCast`
//! - `org.freedesktop.impl.portal.Settings`

mod access;
mod interface;
mod request;
mod session;
mod settings;
mod state;
mod stream;

pub use access::AccessPortal;
pub use interface::{
    fallback_mapping_id, validate_cursor_mode, validate_persist_mode, ScreenCastPortal,
};
pub use settings::SettingsPortal;
pub use state::{PortalState, SelectedWindow, SessionState};
pub use stream::{build_streams_value_from_descriptors, StreamDescriptor};

pub(crate) use request::Request;
pub(crate) use session::Session;

/// D-Bus object path for the portal desktop interface.
pub const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";

// Source type bitmask values per XDG Desktop Portal spec.
pub const SOURCE_TYPE_MONITOR: u32 = 1;
pub const SOURCE_TYPE_WINDOW: u32 = 2;
#[allow(dead_code)]
pub const SOURCE_TYPE_VIRTUAL: u32 = 4;

// Cursor mode bitmask values per XDG Desktop Portal spec.
pub const CURSOR_MODE_HIDDEN: u32 = 1;
pub const CURSOR_MODE_EMBEDDED: u32 = 2;
pub const CURSOR_MODE_METADATA: u32 = 4;
// We only support HIDDEN and EMBEDDED modes
pub const SUPPORTED_CURSOR_MODES: u32 = CURSOR_MODE_HIDDEN | CURSOR_MODE_EMBEDDED;

/// Returns the D-Bus object path for the portal desktop interface.
#[inline]
pub fn desktop_path() -> &'static str {
    DESKTOP_PATH
}

/// Creates a mapping ID for an output connector name.
///
/// The mapping ID is used to correlate portal streams with compositor outputs.
/// Non-alphanumeric characters (except `-`) are replaced with `_`.
pub fn make_output_mapping_id(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    format!("screencomposer:output-{sanitized}")
}
