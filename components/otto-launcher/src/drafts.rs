//! What was typed in Ask and not sent, kept per session.
//!
//! The launcher is a fresh process each time, so a request half written when
//! the card closes would be gone. It is kept here instead, under the session
//! it was for, and put back in the field when that session is opened again.
//! A request that has no session yet — the first of a new one — is kept under
//! [`NEW`].
//!
//! State rather than config: written by the program, and losing it costs a
//! draft.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The key of the draft that has no session yet.
pub const NEW: &str = "";

/// How many drafts are kept, most recently written first. Sessions that are
/// gone take their drafts with them only by falling off the end.
const KEPT: usize = 64;

/// Where the drafts live: `$XDG_STATE_HOME/otto/ask-drafts.json`.
fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(dir.join("otto").join("ask-drafts.json"))
}

/// The draft kept for `key`, a session URI or [`NEW`].
pub fn load(key: &str) -> Option<String> {
    load_from(&path()?, key)
}

/// Keep `text` as the draft for `key`; nothing but whitespace forgets it.
pub fn save(key: &str, text: &str) {
    if let Some(path) = path() {
        save_to(&path, key, text);
    }
}

/// The drafts as `[key, text]` pairs, most recently written first.
fn read(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(Value::Array(entries)) = serde_json::from_str(&text) else {
        tracing::warn!(path = %path.display(), "the drafts are not readable; starting afresh");
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let key = entry.get("session")?.as_str()?;
            let text = entry.get("text")?.as_str()?;
            Some((key.to_string(), text.to_string()))
        })
        .collect()
}

fn load_from(path: &Path, key: &str) -> Option<String> {
    read(path)
        .into_iter()
        .find(|(other, _)| other == key)
        .map(|(_, text)| text)
}

/// Written whole and moved into place, so a launcher killed mid-write leaves
/// the previous drafts rather than half of new ones.
fn save_to(path: &Path, key: &str, text: &str) {
    let existing = read(path);
    let unchanged = match existing.iter().find(|(other, _)| other == key) {
        Some((_, kept)) => kept == text,
        None => text.trim().is_empty(),
    };
    if unchanged {
        return;
    }
    let kept = (!text.trim().is_empty()).then(|| (key.to_string(), text.to_string()));
    let entries: Vec<Value> = kept
        .into_iter()
        .chain(existing.into_iter().filter(|(other, _)| other != key))
        .take(KEPT)
        .map(|(key, text)| serde_json::json!({ "session": key, "text": text }))
        .collect();

    let Some(dir) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(dir) {
        tracing::warn!(%err, "could not create the state directory");
        return;
    }
    let temporary = path.with_extension("tmp");
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(Value::Array(entries).to_string().as_bytes())?;
        file.write_all(b"\n")
    });
    match written {
        Ok(()) => {
            if let Err(err) = std::fs::rename(&temporary, path) {
                tracing::warn!(%err, "could not save the draft");
            }
        }
        Err(err) => tracing::warn!(%err, "could not write the draft"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_kept_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otto/ask-drafts.json");
        save_to(&path, "ahp-session:/a", "half a question");
        save_to(&path, NEW, "a new one");
        assert_eq!(
            load_from(&path, "ahp-session:/a").as_deref(),
            Some("half a question")
        );
        assert_eq!(load_from(&path, NEW).as_deref(), Some("a new one"));
        assert_eq!(load_from(&path, "ahp-session:/b"), None);

        save_to(&path, "ahp-session:/a", "rewritten");
        assert_eq!(
            load_from(&path, "ahp-session:/a").as_deref(),
            Some("rewritten")
        );
    }

    #[test]
    fn an_emptied_field_forgets_the_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ask-drafts.json");
        save_to(&path, "ahp-session:/a", "something");
        save_to(&path, "ahp-session:/a", "  ");
        assert_eq!(load_from(&path, "ahp-session:/a"), None);
        // Nothing to keep and nothing kept writes nothing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ask-drafts.json");
        save_to(&path, NEW, "");
        assert!(!path.exists());
    }

    #[test]
    fn only_the_most_recent_drafts_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ask-drafts.json");
        for index in 0..KEPT + 3 {
            save_to(&path, &format!("ahp-session:/{index}"), "draft");
        }
        assert_eq!(read(&path).len(), KEPT);
        assert_eq!(load_from(&path, "ahp-session:/0"), None);
        assert!(load_from(&path, &format!("ahp-session:/{}", KEPT + 2)).is_some());
    }
}
