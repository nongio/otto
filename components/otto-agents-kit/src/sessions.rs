//! The agent sessions otto-agents has, as rows, and a feed that keeps them.
//!
//! Agents mode lists the sessions under its field, and the side canvas lists
//! them in a panel of its own. Both build their rows here, so a session reads
//! the same in either place: its title, then whose it is, what it is doing and
//! where it works.
//!
//! [`SessionFeed`] is the list without a conversation: it connects, lists the
//! agents and the sessions, and lists the sessions again whenever the service
//! announces a change to them, until it is dropped. The launcher's own
//! connection, which also follows a session, lives in its Ask mode.

// Rust guideline compliant 2026-02-21

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ahp::{Client, ClientConfig, SubscriptionEvent};
use ahp_types::commands::ListSessionsResult;
use ahp_types::state::{AgentInfo, SessionStatus, SessionSummary, SnapshotState};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use otto_agents_client::default_url;
use otto_agents_client::uri::to_path as path_from_uri;
use serde_json::json;
use tokio::sync::oneshot;

use crate::item::{Activity, Item, Origin};
use crate::link::{reach, Link, Reporter};

/// How long connecting may take before the service counts as not running.
///
/// Connecting is one round trip to a local service. Past this, the service is
/// not answering, and saying so beats a list that looks like it is loading.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Any error from the connection, sendable between threads.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Connect to the service at `url` and introduce the client as `name`.
///
/// # Errors
///
/// Fails when the socket cannot be reached or the handshake does not finish.
pub async fn connect(url: &str, name: &str) -> Result<Client, BoxError> {
    let transport = otto_agents_client::connect(url).await?;
    let client = Client::connect(transport, ClientConfig::default()).await?;
    client
        .initialize(name.into(), vec![PROTOCOL_VERSION.into()], Vec::new())
        .await?;
    Ok(client)
}

/// The sessions the service has, most recently changed first.
///
/// # Errors
///
/// Fails when the request does not get an answer the protocol can read.
pub async fn list_sessions(client: &Client) -> Result<Vec<SessionSummary>, BoxError> {
    let listed: ListSessionsResult = client
        .request("listSessions", json!({ "channel": ROOT_RESOURCE_URI }))
        .await?;
    Ok(listed.items)
}

/// The sessions whose titles contain `query`, as rows from `source`.
///
/// Each row's [`Origin::index`] is the session's place in `sessions`, so a row
/// picked from a narrowed list still names the right session. `agents` names
/// the agent behind each session; one it does not know is named by its id.
pub fn session_items(
    sessions: &[SessionSummary],
    agents: &[AgentInfo],
    source: usize,
    query: &str,
) -> Vec<Item> {
    let query = query.trim().to_lowercase();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let agent_name = |provider: &str| {
        agents
            .iter()
            .find(|agent| agent.provider == provider)
            .map(|agent| agent.display_name.as_str())
    };
    sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| query.is_empty() || session.title.to_lowercase().contains(&query))
        .map(|(index, session)| Item {
            title: if session.title.is_empty() {
                otto_kit::t_owned!("launcher-agents-untitled")
            } else {
                session.title.clone()
            },
            subtitle: Some(session_subtitle(
                session,
                agent_name(&session.provider),
                home.as_deref(),
            )),
            icon: None,
            activity: Some(session_activity(session)),
            checked: None,
            search_terms: Vec::new(),
            origin: Origin { source, index },
        })
        .collect()
}

/// What a session is doing, as the dot in its row shows it.
pub fn session_activity(session: &SessionSummary) -> Activity {
    let status = SessionStatus::from_bits(session.status);
    if status.contains(SessionStatus::InputNeeded) {
        Activity::Waiting
    } else if status.contains(SessionStatus::InProgress) {
        Activity::Working
    } else {
        Activity::Idle
    }
}

/// A session row's second line: `@agent · status · folder`.
pub fn session_subtitle(
    session: &SessionSummary,
    agent: Option<&str>,
    home: Option<&Path>,
) -> String {
    let status = SessionStatus::from_bits(session.status);
    let status = if status.contains(SessionStatus::InputNeeded) {
        otto_kit::t_owned!("launcher-agents-needs-input")
    } else if status.contains(SessionStatus::InProgress) {
        otto_kit::t_owned!("launcher-agents-working")
    } else if status.contains(SessionStatus::Error) {
        otto_kit::t_owned!("launcher-agents-error")
    } else {
        otto_kit::t_owned!("launcher-agents-idle")
    };
    let folder = session
        .working_directories
        .iter()
        .flatten()
        .next()
        .and_then(|uri| path_from_uri(uri));
    // The agent leads, as a handle: whose session this is comes before what it
    // is doing. Unknown providers still name themselves, since the id is what
    // the configuration calls them.
    let agent = agent.unwrap_or(session.provider.as_str());
    let head = if agent.is_empty() {
        status
    } else {
        format!("@{agent} · {status}")
    };
    match folder {
        Some(folder) => format!("{head} · {}", home_relative(&folder, home)),
        None => head,
    }
}

fn home_relative(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Where a [`SessionFeed`] stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedStatus {
    /// Connecting, or listing for the first time.
    Connecting,
    /// The sessions are listed, and kept up to date.
    Listed,
    /// The service is not running, or stopped answering.
    Unreachable,
}

/// What the feed's thread reports.
enum Update {
    Agents(Vec<AgentInfo>),
    Sessions(Vec<SessionSummary>),
    Unreachable(String),
}

/// The service's sessions, listed and kept up to date in the background.
///
/// The connection lives on a thread of its own — a [`Link`] — with an async
/// runtime the caller's loop does not need to have, and reports back over a
/// channel, waking the loop through [`Self::poll_fd`]. Dropping the feed closes the
/// connection: a feed is for as long as the list is on screen.
///
/// # Examples
///
/// ```no_run
/// use otto_agents_kit::sessions::{FeedStatus, SessionFeed};
///
/// let mut feed = SessionFeed::start("otto-canvas");
/// // In the caller's loop, once `feed.poll_fd()` is readable:
/// if feed.pump() && feed.status() == FeedStatus::Listed {
///     for row in feed.items(0, "") {
///         println!("{}", row.title);
///     }
/// }
/// ```
pub struct SessionFeed {
    link: Link<Update>,
    agents: Vec<AgentInfo>,
    sessions: Vec<SessionSummary>,
    status: FeedStatus,
    /// Dropped with the feed, which is what tells the thread to hang up.
    _stop: oneshot::Sender<()>,
}

impl SessionFeed {
    /// Connect to the service `OTTO_AGENTS_URL` names, or the default one,
    /// introducing the client as `name`.
    pub fn start(name: &str) -> Self {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| default_url());
        Self::connect(url, name)
    }

    /// Connect to the service at `url`, introducing the client as `name`.
    ///
    /// # Panics
    ///
    /// Panics when the process cannot make a socket pair or start a thread,
    /// which only happens when it is out of descriptors or memory.
    pub fn connect(url: String, name: &str) -> Self {
        let (stop, stopped) = oneshot::channel();
        let name = name.to_owned();
        let link = Link::start("session-feed", Update::Unreachable, |reporter| async move {
            feed(&url, &name, stopped, &reporter).await;
        });

        Self {
            link,
            agents: Vec::new(),
            sessions: Vec::new(),
            status: FeedStatus::Connecting,
            _stop: stop,
        }
    }

    /// The socket that becomes readable when there is news.
    pub fn poll_fd(&self) -> RawFd {
        self.link.poll_fd()
    }

    /// Take in whatever the thread has reported. Never blocks. Returns
    /// whether anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        for update in self.link.drain() {
            changed = true;
            match update {
                Update::Agents(agents) => self.agents = agents,
                Update::Sessions(sessions) => {
                    self.sessions = sessions;
                    self.status = FeedStatus::Listed;
                }
                Update::Unreachable(error) => {
                    tracing::info!(%error, "the agent service is unreachable");
                    self.status = FeedStatus::Unreachable;
                }
            }
        }
        changed
    }

    /// Where the feed stands.
    pub fn status(&self) -> FeedStatus {
        self.status
    }

    /// The sessions, most recently changed first.
    pub fn sessions(&self) -> &[SessionSummary] {
        &self.sessions
    }

    /// The sessions whose titles contain `query` as rows from `source`, as
    /// agents mode lists them. Each row's [`Origin::index`] is the session's
    /// place in [`SessionFeed::sessions`].
    pub fn items(&self, source: usize, query: &str) -> Vec<Item> {
        session_items(&self.sessions, &self.agents, source, query)
    }
}

/// The feed's thread: connect, list, and list again on every change the
/// service announces, until the feed is dropped.
async fn feed(
    url: &str,
    name: &str,
    mut stopped: oneshot::Receiver<()>,
    reporter: &Reporter<Update>,
) {
    let client = match reach(url, name).await {
        Ok(client) => client,
        Err(err) => {
            reporter.send(Update::Unreachable(err));
            return;
        }
    };

    // The root channel carries the catalogue's news: sessions added, removed
    // and changed. Without it the list would be as old as the connection.
    let mut root = match client.subscribe(ROOT_RESOURCE_URI.to_string()).await {
        Ok((subscribed, root)) => {
            if let Some(SnapshotState::Root(state)) = subscribed.snapshot.map(|s| s.state) {
                reporter.send(Update::Agents(state.agents));
            }
            Some(root)
        }
        Err(err) => {
            tracing::warn!(%err, "could not subscribe to the agent service's catalogue");
            None
        }
    };
    relist(&client, reporter).await;

    loop {
        let event = async {
            match root.as_mut() {
                Some(root) => root.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = &mut stopped => break,
            event = event => match event {
                Some(
                    SubscriptionEvent::SessionAdded(_)
                    | SubscriptionEvent::SessionRemoved(_)
                    | SubscriptionEvent::SessionSummaryChanged(_),
                ) => relist(&client, reporter).await,
                Some(_) => {}
                None => {
                    reporter.send(Update::Unreachable(
                        "otto-agents closed the connection".into(),
                    ));
                    break;
                }
            },
        }
    }
    client.shutdown().await;
}

async fn relist(client: &Client, reporter: &Reporter<Update>) {
    match list_sessions(client).await {
        Ok(sessions) => reporter.send(Update::Sessions(sessions)),
        Err(err) => tracing::warn!(%err, "could not list the sessions"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn summary(id: &str, title: &str, provider: &str) -> SessionSummary {
        serde_json::from_value(json!({
            "resource": format!("ahp-session:/{id}"),
            "provider": provider,
            "title": title,
            "status": 1,
            "createdAt": "2026-01-01T00:00:00Z",
            "modifiedAt": "2026-01-01T00:00:00Z",
        }))
        .expect("a session summary")
    }

    fn pump_until(feed: &mut SessionFeed, done: impl Fn(&SessionFeed) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            feed.pump();
            if done(feed) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn rows_keep_their_place_in_the_list_when_narrowed() {
        let sessions = [
            summary("a", "Fix the build", "claude"),
            summary("b", "", "claude"),
            summary("c", "Tidy the docs", "codex"),
        ];
        let rows = session_items(&sessions, &[], 3, "docs");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Tidy the docs");
        assert_eq!(
            rows[0].origin,
            Origin {
                source: 3,
                index: 2
            }
        );

        let rows = session_items(&sessions, &[], 0, "");
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[1].title,
            otto_kit::t_owned!("launcher-agents-untitled")
        );
        // An agent nobody named is named by its id.
        assert!(rows[2]
            .subtitle
            .as_deref()
            .is_some_and(|line| line.starts_with("@codex · ")));
    }

    #[test]
    fn a_feed_without_a_service_says_so() {
        // Nothing listens on the discard port.
        let mut feed = SessionFeed::connect("ws://127.0.0.1:9".into(), "test");
        assert_eq!(feed.status(), FeedStatus::Connecting);
        assert!(pump_until(&mut feed, |feed| feed.status() == FeedStatus::Unreachable));
        assert!(feed.sessions().is_empty());
    }

    /// Against a live service: `otto-agents serve --echo`, with
    /// `OTTO_AGENTS_URL` pointing at it when it is not on the default socket.
    #[test]
    #[ignore = "needs a running `otto-agents serve --echo`"]
    fn a_feed_lists_the_sessions() {
        let mut feed = SessionFeed::start("test");
        assert!(pump_until(&mut feed, |feed| feed.status() == FeedStatus::Listed));
    }
}
