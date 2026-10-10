//! File-driven debug toggles (`touch /tmp/otto-<name>`) and command files
//! (`echo … > $XDG_RUNTIME_DIR/otto-action`), for driving and inspecting a
//! live session from a shell without a rebuild.
//!
//! Every check is a `stat()` or a `read()` on a hot path — per frame, per
//! commit, per event-loop turn — so the whole mechanism is compiled in only
//! with the `debug-hooks` feature (part of `dev`). Without it these helpers
//! are constants and the branches they guard fold away.

/// Whether the hooks are compiled in.
pub const ENABLED: bool = cfg!(feature = "debug-hooks");

/// A toggle is on while the file exists.
#[cfg(feature = "debug-hooks")]
#[inline]
pub fn toggle(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[cfg(not(feature = "debug-hooks"))]
#[inline(always)]
pub fn toggle(_path: &str) -> bool {
    false
}

/// Read and delete a command file, so one write runs once.
#[cfg(feature = "debug-hooks")]
pub fn take_file(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(text)
}

#[cfg(not(feature = "debug-hooks"))]
#[inline(always)]
pub fn take_file(_path: &str) -> Option<String> {
    None
}

/// Where a command file lives: the path in the environment variable `env`
/// when it is set, otherwise `name` in the user's runtime directory
/// (`$XDG_RUNTIME_DIR`, else `/run/user/<uid>`).
///
/// A command file *runs* something, so it never defaults to world-writable
/// `/tmp`, where any local user could drive the session. With neither an
/// override nor a runtime directory the hook is off (`None`). Resolved once
/// per process by the callers: it is polled per frame.
pub fn command_file_path(env: &str, name: &str) -> Option<String> {
    resolve_command_file(std::env::var(env).ok(), otto_kit::xdg::runtime_dir(), name)
}

fn resolve_command_file(
    overridden: Option<String>,
    runtime_dir: Option<std::path::PathBuf>,
    name: &str,
) -> Option<String> {
    overridden
        .filter(|path| !path.is_empty())
        .or_else(|| Some(runtime_dir?.join(name).to_str()?.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_environment_overrides_the_runtime_directory() {
        assert_eq!(
            resolve_command_file(
                Some("/srv/harness.action".into()),
                Some(PathBuf::from("/run/user/1000")),
                "otto-action",
            )
            .as_deref(),
            Some("/srv/harness.action")
        );
    }

    #[test]
    fn the_default_is_in_the_runtime_directory_not_tmp() {
        assert_eq!(
            resolve_command_file(None, Some(PathBuf::from("/run/user/1000")), "otto-action")
                .as_deref(),
            Some("/run/user/1000/otto-action")
        );
        assert_eq!(resolve_command_file(None, None, "otto-action"), None);
        assert_eq!(
            resolve_command_file(Some(String::new()), None, "otto-action"),
            None
        );
    }
}
