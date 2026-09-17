//! Sessions on disk, so they outlive the service.
//!
//! One JSON file per session, in `$XDG_STATE_HOME/otto-agentsd/sessions/`. What
//! is stored is otto-agentsd's own part: the session's and its chat's AHP state,
//! as clients see them, and the agent's id for the session. The agent keeps
//! its own history; carrying a session on after a restart hands the agent that
//! id, so it can pick its history up again.

use std::io;
use std::path::{Path, PathBuf};

use ahp_types::state::{ChatState, SessionState};
use serde::{Deserialize, Serialize};

/// Bumped when a record changes in a way older readers would get wrong.
const VERSION: u32 = 1;

/// Everything stored about one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub version: u32,
    /// The session's URI.
    pub resource: String,
    pub created_at: String,
    /// The agent's id for the session, once the agent has given one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<String>,
    pub session: SessionState,
    /// The session's one chat.
    pub chat: ChatState,
}

impl SessionRecord {
    pub fn new(
        resource: String,
        created_at: String,
        agent_session: Option<String>,
        session: SessionState,
        chat: ChatState,
    ) -> Self {
        Self {
            version: VERSION,
            resource,
            created_at,
            agent_session,
            session,
            chat,
        }
    }
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `$XDG_STATE_HOME/otto-agentsd/sessions`, or `~/.local/state/otto-agentsd/sessions`.
    pub fn default_dir() -> Option<PathBuf> {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })?;
        let dir = state_home.join("otto-agentsd").join("sessions");
        // The service was called otto-ahp; bring its sessions along once.
        let old = state_home.join("otto-ahp").join("sessions");
        if !dir.exists() && old.is_dir() {
            if let Some(parent) = dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::rename(&old, &dir) {
                tracing::warn!(%error, "could not move sessions from {}", old.display());
            }
        }
        Some(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every session stored. A file that cannot be read is skipped and
    /// logged, rather than keeping the service from starting.
    pub fn load(&self) -> Vec<SessionRecord> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(err) => {
                tracing::warn!(dir = %self.dir.display(), %err, "cannot read the stored sessions");
                return Vec::new();
            }
        };
        let mut records = Vec::new();
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let record = std::fs::read(&path)
                .map_err(|err| err.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<SessionRecord>(&bytes).map_err(|err| err.to_string())
                });
            match record {
                Ok(record) if record.version == VERSION => records.push(record),
                Ok(record) => tracing::warn!(
                    path = %path.display(),
                    version = record.version,
                    "skipping a session stored in a format this version does not know"
                ),
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "skipping an unreadable session")
                }
            }
        }
        records
    }

    /// Writes `record`, replacing what was stored for its session. The file
    /// is written beside its final name and renamed into place, so a crash
    /// mid-write leaves the previous version whole.
    pub fn save(&self, record: &SessionRecord) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(&record.resource);
        let partial = path.with_extension("json.partial");
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        std::fs::write(&partial, bytes)?;
        std::fs::rename(&partial, &path)
    }

    /// The file for the session at `resource`, named after its id with
    /// anything unsafe in a file name replaced.
    fn path(&self, resource: &str) -> PathBuf {
        let id = resource.strip_prefix("ahp-session:/").unwrap_or(resource);
        let name: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{name}.json"))
    }
}
