//! Logging setup, the same for every app.

use tracing_subscriber::EnvFilter;

/// Log to stderr, filtered by `RUST_LOG` when it is set and parses, else by
/// `default_filter` (`"info"` for every Otto app).
///
/// Call once, first thing in `main`. A second call, or a subscriber some
/// other code already installed, is left as it is rather than panicking.
pub fn init(default_filter: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter(
            std::env::var("RUST_LOG").ok().as_deref(),
            default_filter,
        ))
        .try_init();
}

/// `rust_log` when it is set and parses, else `default_filter`.
fn filter(rust_log: Option<&str>, default_filter: &str) -> EnvFilter {
    rust_log
        .and_then(|directives| EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| EnvFilter::new(default_filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_log_wins_and_the_default_fills_in() {
        assert_eq!(filter(Some("debug"), "info").to_string(), "debug");
        assert_eq!(filter(None, "info").to_string(), "info");
        // A directive that does not parse is not a reason to log nothing.
        assert_eq!(filter(Some("=[bad"), "info").to_string(), "info");
    }
}
