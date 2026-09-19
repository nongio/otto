//! Sessions on disk, so they outlive the service.
//!
//! One JSON file per session, in `$XDG_STATE_HOME/otto-agents/sessions/`. What
//! is stored is otto-agents' own part: the session's and its chat's AHP state,
//! as clients see them, and the agent's id for the session. The agent keeps
//! its own history; carrying a session on after a restart hands the agent that
//! id, so it can pick its history up again.

use std::fs::{DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use ahp_types::state::{ChatState, SessionState};
use serde::{Deserialize, Serialize};

/// Bumped when a record changes in a way older readers would get wrong.
const VERSION: u32 = 1;

/// Sessions are the person's conversations: the directory and its files are
/// readable by them alone.
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

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

    /// `$XDG_STATE_HOME/otto-agents/sessions`, or `~/.local/state/otto-agents/sessions`.
    pub fn default_dir() -> Option<PathBuf> {
        Some(
            crate::xdg::state_home()?
                .join("otto-agents")
                .join("sessions"),
        )
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every session stored. A file that cannot be read is skipped and
    /// logged, rather than keeping the service from starting. Anything stored
    /// with wider permissions than [`Store::save`] gives is tightened.
    pub fn load(&self) -> Vec<SessionRecord> {
        self.tighten();
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
                Ok(record) if record.version == VERSION => {
                    // A record whose file is not the one its resource names
                    // would be read again beside its replacement, showing the
                    // session twice. It is moved rather than left to.
                    if path != self.path(&record.resource) {
                        let _ = std::fs::remove_file(&path);
                        if let Err(err) = self.save(&record) {
                            tracing::warn!(%err, "could not rename a stored session");
                        }
                    }
                    records.push(record);
                }
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
        DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(&self.dir)?;
        let path = self.path(&record.resource);
        let partial = path.with_extension("json.partial");
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&partial)?;
        file.write_all(&bytes)?;
        drop(file);
        std::fs::rename(&partial, &path)
    }

    /// Takes group and world access off the directory and its records, for
    /// a store written before they were private.
    fn tighten(&self) {
        let mut paths = vec![(self.dir.clone(), DIR_MODE)];
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            paths.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_file())
                    .map(|path| (path, FILE_MODE)),
            );
        }
        for (path, mode) in paths {
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.permissions().mode() & 0o077 == 0 {
                continue;
            }
            match std::fs::set_permissions(&path, Permissions::from_mode(mode)) {
                Ok(()) => {
                    tracing::info!(path = %path.display(), mode = format_args!("{mode:o}"), "made private")
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "could not make private")
                }
            }
        }
    }

    /// Removes what is stored for the session at `resource`. A session that
    /// was never stored is fine.
    pub fn delete(&self, resource: &str) -> io::Result<()> {
        match std::fs::remove_file(self.path(resource)) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    /// The file for the session at `resource`: its id with anything unsafe in
    /// a file name replaced, and a digest of the whole resource appended.
    ///
    /// The replacement alone would not name one session: `ahp-session:/a.b`
    /// and `ahp-session:/a_b` flatten to the same file, and one session's
    /// record would land on the other's. The digest is what makes it one file
    /// per session; the readable part is only so the directory can be read.
    fn path(&self, resource: &str) -> PathBuf {
        let id = resource
            .strip_prefix(otto_agents_client::session::SESSION_SCHEME)
            .unwrap_or(resource);
        let name: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect();
        self.dir
            .join(format!("{name}-{:016x}.json", digest(resource)))
    }
}

/// FNV-1a over a session's resource, to tell apart two whose ids flatten to
/// the same file name. Nothing trusts this beyond naming a file.
fn digest(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{new_chat_state, new_session_state};

    fn record(id: &str) -> SessionRecord {
        let now = "2026-09-18T00:00:00Z";
        SessionRecord::new(
            format!("ahp-session:/{id}"),
            now.into(),
            None,
            new_session_state("echo", "file:///tmp"),
            new_chat_state(&format!("ahp-chat:/{id}"), now),
        )
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn records_are_private() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let store = Store::new(temp.path().join("sessions"));
        store.save(&record("one")).expect("save");

        assert_eq!(mode(store.dir()), 0o700);
        assert_eq!(mode(&store.path("ahp-session:/one")), 0o600);
        assert_eq!(store.load().len(), 1);
    }

    #[test]
    fn loading_tightens_an_older_store() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let dir = temp.path().join("sessions");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, Permissions::from_mode(0o755)).expect("chmod");
        let path = dir.join("two.json");
        std::fs::write(&path, serde_json::to_vec(&record("two")).unwrap()).expect("write");
        std::fs::set_permissions(&path, Permissions::from_mode(0o644)).expect("chmod");

        let store = Store::new(&dir);
        assert_eq!(store.load().len(), 1);
        assert_eq!(mode(&dir), 0o700);
        // Stored under a name of someone else's choosing, it is moved to the
        // one its own resource names — and lands there private.
        assert!(!path.exists(), "the old file was left behind");
        assert_eq!(mode(&store.path("ahp-session:/two")), 0o600);
    }

    /// Two ids that flatten to the same file name are still two sessions.
    #[test]
    fn sessions_whose_ids_look_alike_get_their_own_files() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let store = Store::new(temp.path().join("sessions"));
        store.save(&record("a.b")).expect("save");
        store.save(&record("a_b")).expect("save");

        assert_ne!(
            store.path("ahp-session:/a.b"),
            store.path("ahp-session:/a_b")
        );
        let mut stored: Vec<String> = store.load().into_iter().map(|r| r.resource).collect();
        stored.sort();
        assert_eq!(stored, ["ahp-session:/a.b", "ahp-session:/a_b"]);
    }

    /// The point of the store: a session outlives the service.
    #[test]
    fn a_session_survives_a_restart() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let dir = temp.path().join("sessions");
        let store = Store::new(&dir);
        let mut saved = record("one");
        saved.agent_session = Some("agent-side-id".into());
        store.save(&saved).expect("save");

        // A second store over the same directory is what a restart amounts to.
        let records = Store::new(&dir).load();
        let [loaded] = &records[..] else {
            panic!("expected one record: {records:?}");
        };
        assert_eq!(loaded.resource, saved.resource);
        assert_eq!(loaded.agent_session.as_deref(), Some("agent-side-id"));
        assert_eq!(loaded.session.provider, saved.session.provider);
        assert_eq!(loaded.created_at, saved.created_at);

        Store::new(&dir).delete(&saved.resource).expect("delete");
        assert!(Store::new(&dir).load().is_empty());
    }
}
