//! The compositor's last word: the `otto::summary` line in its log.
//!
//! Otto logs `session summary: <DebugInfo JSON>` at `info` once its renderer
//! and outputs are up, and again when the outputs change. When the
//! compositor cannot answer over D-Bus (it crashed, or it is too old), the
//! last such line in the newest log is the next best thing.

// Rust guideline compliant 2026-02-21

use std::path::Path;

use crate::doctor::compositor::DebugInfo;
use crate::doctor::logs::{self, Origin};
use crate::doctor::sections::{strip_ansi, tail_lines};

/// The tracing target the compositor logs the summary under.
const TARGET: &str = "otto::summary";

/// What comes before the JSON on a summary line.
const MARKER: &str = "session summary: ";

/// The last summary line in `text`, parsed.
///
/// Colours are stripped first, and any other line is skipped, so the log can
/// be read as it is.
pub fn last_in(text: &str) -> Option<DebugInfo> {
    text.lines().rev().find_map(|line| {
        let line = strip_ansi(line);
        let after_target = &line[line.find(TARGET)? + TARGET.len()..];
        let json = &after_target[after_target.find(MARKER)? + MARKER.len()..];
        DebugInfo::from_json(json.trim()).ok()
    })
}

/// The newest summary in the logs [`logs::find`] lists, with the name of the
/// log it came from.
///
/// Only files are read: Otto's logs, a running Otto's stderr, the
/// `otto-session` logs. The journal and the panic log are not searched.
pub fn from_logs(places: logs::Places<'_>) -> Option<(DebugInfo, String)> {
    logs::find(places).into_iter().find_map(|log| {
        let (path, shown): (&Path, &Path) = match &log.origin {
            Origin::File(path) if !path.ends_with("panic.log") => (path, path),
            Origin::Stderr { fd, target, .. } => (fd, target),
            _ => return None,
        };
        let (text, _, _) = tail_lines(path, usize::MAX)?;
        let info = last_in(&text)?;
        Some((info, shown.display().to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const LINE: &str = r#"{"version":"1.8.0","git_commit":"abc1234","backend":"udev","renderer":{"api":"gl","vendor":"Intel","renderer":"Mesa Intel(R) Xe Graphics","version":"OpenGL ES 3.2 Mesa 25.1.4"},"vulkan_fallback":null,"config_files":[],"log_path":null,"outputs":[{"name":"eDP-1","make":"BOE","model":"X","width":2880,"height":1920,"refresh_mhz":120000,"scale":2.0,"x":0,"y":0,"virtual":false}]}"#;

    #[test]
    fn reads_the_last_summary_among_other_lines_and_colours() {
        let older = LINE.replace("\"udev\"", "\"winit\"");
        let text = format!(
            "2026-10-18T10:00:00Z  INFO otto::summary: session summary: {older}\n\
             2026-10-18T10:00:01Z  INFO otto::dock: dock update_state\n\
             \u{1b}[2m2026-10-18T10:05:00Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \
             \u{1b}[2motto::summary\u{1b}[0m\u{1b}[2m:\u{1b}[0m session summary: {LINE}\n\
             2026-10-18T10:05:02Z  WARN otto::udev: page flip failed\n\
             2026-10-18T10:05:03Z  INFO otto::shell: session summary: not this one\n"
        );

        let info = last_in(&text).expect("a summary is found");
        assert_eq!(info.backend, "udev");
        assert_eq!(info.git_commit.as_deref(), Some("abc1234"));
        assert_eq!(info.outputs[0].name, "eDP-1");
        assert_eq!(info.outputs[0].refresh_mhz, 120_000);
        assert_eq!(
            info.renderer.map(|r| r.renderer),
            Some("Mesa Intel(R) Xe Graphics".to_owned())
        );
    }

    #[test]
    fn no_summary_or_a_broken_one_is_none() {
        assert!(last_in("INFO otto: Starting otto\n").is_none());
        assert!(last_in("INFO otto::summary: session summary: {\"version\":\n").is_none());
    }

    #[test]
    fn takes_the_newest_log_that_has_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("otto");
        fs::create_dir_all(&state).unwrap();
        // This session crashed before its renderer came up: no summary yet.
        fs::write(state.join("otto.log"), "INFO otto: Starting otto\n").unwrap();
        let line = format!("INFO otto::summary: session summary: {LINE}\n");
        fs::write(state.join("otto.log.1"), &line).unwrap();
        fs::write(state.join("session.log"), line.replace("udev", "x11")).unwrap();

        let (info, from) = from_logs(logs::Places {
            proc_root: &dir.path().join("proc"),
            log_dir: Some(&state),
            reported: None,
            // SAFETY: getuid has no preconditions.
            uid: unsafe { libc::getuid() },
        })
        .expect("found in otto.log.1");
        assert_eq!(info.backend, "udev");
        assert!(from.ends_with("otto.log.1"), "{from}");
    }
}
