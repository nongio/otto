//! Logging setup, the same for every Otto program.

// Rust guideline compliant 2026-02-21

use tracing_subscriber::EnvFilter;

/// Logs to stderr, filtered by `RUST_LOG` or else by `default_filter`.
///
/// `RUST_LOG` is used when it is set and parses; otherwise `default_filter`
/// applies: `"info"` for most programs, or a crate-scoped directive such as
/// `"otto_dictate=info"` where dependencies are chatty. Standard output stays
/// the program's own: `otto-agents acp` speaks its protocol there.
///
/// Call once, first thing in `main`. A second call, or a subscriber some
/// other code already installed, is left as it is rather than panicking.
pub fn init(default_filter: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter(
            std::env::var("RUST_LOG").ok().as_deref(),
            default_filter,
        ))
        .with_writer(std::io::stderr)
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
