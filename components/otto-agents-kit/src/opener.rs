//! Where a session opens when someone picks it from a list.
//!
//! Most sessions are conversations, and the launcher's Ask shows them. A
//! session an app started about one of its documents belongs to that app:
//! Preview's chat creates its sessions with `otto.app = "otto-preview"` and
//! `otto.subject = [file URI]` in their `_meta` (plan 0016), and picking one
//! opens the file in Preview with the conversation beside it. A session open
//! in a terminal goes to its terminal first; see [`crate::chat::Terminal`].

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Map, Value};

/// The program that opens Preview's sessions.
const PREVIEW: &str = "otto-preview";

/// Where a session opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opener {
    /// In Ask, as a conversation: the launcher's, or the host's own.
    Ask,
    /// In Preview, beside `file`.
    Preview { file: PathBuf },
}

impl Opener {
    /// Where the session whose `_meta` is `meta` opens.
    pub fn from_meta(meta: Option<&Map<String, Value>>) -> Self {
        let Some(otto) = meta.and_then(|meta| meta.get("otto")) else {
            return Self::Ask;
        };
        if otto.get("app").and_then(Value::as_str) != Some(PREVIEW) {
            return Self::Ask;
        }
        let file = otto
            .get("subject")
            .and_then(Value::as_array)
            .and_then(|subject| subject.first())
            .and_then(Value::as_str)
            .and_then(otto_agents_client::uri::to_path);
        match file {
            Some(file) => Self::Preview { file },
            None => Self::Ask,
        }
    }

    /// The command that opens session `session` where it belongs, `None`
    /// for Ask, which the caller shows itself.
    pub fn command(&self, session: &str) -> Option<Vec<String>> {
        match self {
            Self::Ask => None,
            Self::Preview { file } => Some(vec![
                PREVIEW.to_owned(),
                "--session".to_owned(),
                session.to_owned(),
                file.to_string_lossy().into_owned(),
            ]),
        }
    }

    /// Open session `session` where it belongs, detached, so it outlives
    /// whoever asked. Returns `false` for Ask, which the caller shows.
    pub fn open(&self, session: &str) -> bool {
        let Some(command) = self.command(session) else {
            return false;
        };
        let spawned = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                // Reaped once it exits; a later start forwards to the first.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                tracing::info!(session, "opening the session in Preview");
                true
            }
            Err(err) => {
                tracing::warn!(%err, session, "could not start Preview; opening in Ask");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_preview_session_opens_beside_its_file() {
        let meta = meta(serde_json::json!({ "otto": {
            "app": "otto-preview",
            "subject": ["file:///home/me/Photos/IMG%201.jpg"]
        }}));
        let opener = Opener::from_meta(Some(&meta));
        assert_eq!(
            opener,
            Opener::Preview {
                file: PathBuf::from("/home/me/Photos/IMG 1.jpg")
            }
        );
        assert_eq!(
            opener.command("ahp-session:/abc").unwrap(),
            [
                "otto-preview",
                "--session",
                "ahp-session:/abc",
                "/home/me/Photos/IMG 1.jpg"
            ]
        );
    }

    #[test]
    fn everything_else_opens_in_ask() {
        assert_eq!(Opener::from_meta(None), Opener::Ask);
        let terminal = meta(serde_json::json!({ "otto": { "terminal": {} } }));
        assert_eq!(Opener::from_meta(Some(&terminal)), Opener::Ask);
        let no_file = meta(serde_json::json!({ "otto": { "app": "otto-preview" } }));
        assert_eq!(Opener::from_meta(Some(&no_file)), Opener::Ask);
        assert_eq!(Opener::Ask.command("s"), None);
    }
}
