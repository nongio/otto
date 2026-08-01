//! Portal state management for tracking active sessions.

use std::collections::HashMap;
use std::time::Instant;

use zbus::zvariant::OwnedObjectPath;

use crate::otto_client::screencast::WindowSource;

/// Global portal state tracking all active sessions.
#[derive(Default)]
pub struct PortalState {
    /// Map from portal session handle to session state.
    pub sessions: HashMap<String, SessionState>,
    /// Last source each app was granted, keyed by app id. See [`RecentGrant`].
    pub recent_grants: HashMap<String, RecentGrant>,
}

/// What the user picked in the source dialog.
#[derive(Clone, Debug)]
pub enum SourceSelection {
    Monitor(String),
    Window(WindowSource),
}

/// A source the user approved, remembered for a short while so the same app
/// isn't asked twice in a row.
///
/// Chrome opens one screencast session to render the preview in its own picker
/// and a second one to actually share, which without this would pop the source
/// dialog twice for a single user action. The grant is only reused for the same
/// app, for the same source, while it is still on offer, and its timestamp is
/// never refreshed — an app cannot keep the window open by re-asking.
pub struct RecentGrant {
    /// The source the user approved.
    pub source: SourceSelection,
    /// When the user approved it.
    pub granted_at: Instant,
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
