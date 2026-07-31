//! Portal state management for tracking active sessions.

use std::collections::HashMap;

use zbus::zvariant::OwnedObjectPath;

/// Global portal state tracking all active sessions.
#[derive(Default)]
pub struct PortalState {
    /// Map from portal session handle to session state.
    pub sessions: HashMap<String, SessionState>,
}

/// State for a single screencast session.
#[derive(Clone)]
pub struct SessionState {
    /// Object path of the corresponding compositor session.
    pub sc_session: OwnedObjectPath,
    /// Output connectors selected for this session.
    ///
    /// Empty when the user picked a window instead — the two are mutually
    /// exclusive, since the picker offers a single choice.
    pub selected_outputs: Vec<String>,
    /// Window selected for this session, if the user picked a window source.
    pub selected_window: Option<SelectedWindow>,
    /// Cursor mode (Hidden=1, Embedded=2, Metadata=4).
    pub cursor_mode: u32,
    /// Persistence mode (None=0, Application=1, Permanent=2).
    pub persist_mode: Option<u32>,
    /// Counter for generating unique stream IDs.
    pub next_stream_id: u32,
}

/// A window the user chose in the source picker.
#[derive(Clone, Debug)]
pub struct SelectedWindow {
    /// `ext-foreign-toplevel-list-v1` identifier.
    pub id: String,
    /// Title at pick time — for logging and the stream's mapping id only.
    pub title: String,
}
