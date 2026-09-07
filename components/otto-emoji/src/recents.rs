//! What was picked before, and the tone that was chosen.
//!
//! State rather than config: the program writes it, and losing it costs a
//! few seconds of scrolling. It lives beside the launcher's history under
//! `$XDG_STATE_HOME/otto/`.

use std::io::Write;
use std::path::PathBuf;

use crate::data::Tone;

/// How many recent picks are kept — three rows of the palette.
pub const KEPT: usize = 30;

fn state_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(dir.join("otto"))
}

fn recent_path() -> Option<PathBuf> {
    Some(state_dir()?.join("emoji-recent"))
}

fn tone_path() -> Option<PathBuf> {
    Some(state_dir()?.join("emoji-tone"))
}

/// The emoji picked before, most recent first, as they were typed — tone and
/// all.
pub fn recent() -> Vec<String> {
    let Some(path) = recent_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .take(KEPT)
        .collect()
}

/// `text` moved to the front, keeping the rest in order and dropping the
/// oldest past [`KEPT`].
pub fn promote(text: &str, existing: Vec<String>) -> Vec<String> {
    std::iter::once(text.to_string())
        .chain(existing.into_iter().filter(|other| other != text))
        .take(KEPT)
        .collect()
}

/// Move `text` to the front of the recent list.
pub fn remember(text: &str) {
    let Some(path) = recent_path() else {
        return;
    };
    let mut body = promote(text, recent()).join("\n");
    body.push('\n');
    write_whole(&path, &body);
}

/// The tone chosen last time, if one was.
pub fn tone() -> Tone {
    tone_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| Tone::from_modifier(&text))
        .unwrap_or_default()
}

pub fn remember_tone(tone: Tone) {
    let Some(path) = tone_path() else {
        return;
    };
    let body = tone.modifier().map(String::from).unwrap_or_default();
    write_whole(&path, &body);
}

/// Written whole and moved into place, so a picker killed mid-write leaves
/// the previous file rather than half of a new one.
fn write_whole(path: &std::path::Path, body: &str) {
    let Some(dir) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(dir) {
        tracing::warn!(%err, "could not create the state directory");
        return;
    }
    let temporary = path.with_extension("tmp");
    let written =
        std::fs::File::create(&temporary).and_then(|mut file| file.write_all(body.as_bytes()));
    match written {
        Ok(()) => {
            if let Err(err) = std::fs::rename(&temporary, path) {
                tracing::warn!(%err, ?path, "could not save");
            }
        }
        Err(err) => tracing::warn!(%err, ?path, "could not write"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promoting_moves_to_the_front_without_duplicates() {
        let existing = vec!["😀".to_string(), "👋".to_string(), "❤️".to_string()];
        assert_eq!(promote("👋", existing.clone()), ["👋", "😀", "❤️"]);
        assert_eq!(promote("🎉", existing), ["🎉", "😀", "👋", "❤️"]);
    }

    #[test]
    fn the_list_is_capped() {
        let existing: Vec<String> = (0..KEPT).map(|i| i.to_string()).collect();
        let promoted = promote("new", existing);
        assert_eq!(promoted.len(), KEPT);
        assert_eq!(promoted[0], "new");
        assert_eq!(promoted.last().unwrap(), &(KEPT - 2).to_string());
    }
}
