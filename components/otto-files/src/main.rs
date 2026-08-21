//! The `otto-files` binary. Everything of substance lives in the library —
//! see `lib.rs` — so the picker's D-Bus service and the browser window are
//! two entry points into one body of code rather than two programs.

use std::path::PathBuf;

/// The runtime is not decoration: otto-kit only starts the colour-scheme and
/// icon-theme watchers when one is current, and without them
/// `current_icon_theme()` stays empty. An empty theme name makes every icon
/// lookup search `hicolor` alone, which ships no `folder` or mimetype icons —
/// so the whole listing draws without icons. The theme comes from Otto's own
/// settings, surfaced through the settings portal.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // First, before the tokio runtime starts a thread and before anything
    // connects to Wayland. The sandboxed decode worker is *this binary*
    // re-executed, so without this line a preview would start a second file
    // browser instead of decoding a file. Returns immediately on a normal
    // start; never returns at all when this process is a worker.
    otto_quickview::run_worker_if_requested();

    tokio::runtime::Runtime::new()?.block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // `--picker` is how the bus activates us: no window until a request
    // arrives, and the process outlives each one so a run of picks shares a
    // warm icon and thumbnail cache.
    if std::env::args().any(|a| a == "--picker") {
        return otto_files::app::run_picker().await;
    }

    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(otto_files::model::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));

    otto_files::app::run_browser(start)
}
