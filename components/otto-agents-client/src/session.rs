//! Session URIs, and picking one out by hand.

use ahp_types::state::SessionSummary;

/// What every session URI starts with.
pub const SESSION_SCHEME: &str = "ahp-session:/";

/// The id part of a session's URI.
pub fn session_id(session: &SessionSummary) -> &str {
    session
        .resource
        .strip_prefix(SESSION_SCHEME)
        .unwrap_or(&session.resource)
}

/// Why a query named no one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoSession {
    /// There are none at all.
    None,
    /// Nothing matched the query.
    NoMatch(String),
    /// More than one did, so the query has to be longer.
    Ambiguous { query: String, count: usize },
}

impl std::fmt::Display for NoSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "no sessions"),
            Self::NoMatch(query) => write!(f, "no session matches `{query}`"),
            Self::Ambiguous { query, count } => {
                write!(f, "`{query}` matches {count} sessions; give more of the id")
            }
        }
    }
}

impl std::error::Error for NoSession {}

/// The session `query` names: a full URI, an id, or the start of one. With no
/// query, the most recent session — the list is newest first.
pub fn find<'a>(
    sessions: &'a [SessionSummary],
    query: Option<&str>,
) -> Result<&'a SessionSummary, NoSession> {
    let Some(query) = query else {
        return sessions.first().ok_or(NoSession::None);
    };
    let id = query.strip_prefix(SESSION_SCHEME).unwrap_or(query);
    let matches: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|session| session_id(session).starts_with(id))
        .collect();
    match matches.as_slice() {
        [session] => Ok(session),
        [] => Err(NoSession::NoMatch(query.to_owned())),
        many => Err(NoSession::Ambiguous {
            query: query.to_owned(),
            count: many.len(),
        }),
    }
}
