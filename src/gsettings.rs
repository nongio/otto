//! The `gsettings` command, for writing to the desktop's settings store.
//!
//! The compositor links no GNOME library: GLib ships the tool, so wherever
//! there is something reading that store there is a `gsettings` to write it
//! with. Every call spawns a process, so callers run on threads of their own.

use std::process::Command;

/// Run `gsettings`, returning what it printed when it succeeded.
pub(crate) fn run(args: &[&str]) -> Option<String> {
    let output = Command::new("gsettings").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `text` as a GVariant string literal, the form `gsettings set` parses and
/// `gsettings get` prints a string or enum value in.
pub(crate) fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::quoted;

    #[test]
    fn strings_are_gvariant_literals() {
        assert_eq!(quoted("slight"), "'slight'");
        assert_eq!(quoted("Inter 9.75"), "'Inter 9.75'");
        assert_eq!(quoted("Ryan's Sans"), r"'Ryan\'s Sans'");
        assert_eq!(quoted(r"a\b"), r"'a\\b'");
    }
}
