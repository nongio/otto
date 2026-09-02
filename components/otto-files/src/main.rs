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

    // Before the first string is read, and before the Wayland connection: the
    // sidebar and the column headings are built during startup. Asks the
    // compositor rather than reading LANG, so "Preferred languages" moves the
    // file browser too.
    otto_kit::i18n::init_from_desktop();

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

    // `--trash` is the Trash window: the same view layer with none of the
    // browser's chrome. Its own .desktop entry launches us this way.
    //
    // `trash:///` opens it too. The rest of the desktop says "the trash" with
    // that URI — it is how every other file manager is asked for it, and what
    // `xdg-open` hands a scheme handler — and refusing it would make Otto the
    // one file manager that cannot be asked. It is accepted as a way in, not
    // as a location: the Trash is a shell, not somewhere the browser can
    // navigate to (see `specs/file-browser.md`), so both spellings land in the
    // same window and neither becomes a path in the location bar.
    if std::env::args().any(|a| a == "--trash" || is_trash_uri(&a)) {
        return otto_files::app::run_trash();
    }

    // The Trash window with its Empty Trash question already asked — the
    // desktop entry's `empty` action, which the dock offers on the Trash icon.
    if std::env::args().any(|a| a == "--empty-trash") {
        return otto_files::app::run_empty_trash();
    }

    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(otto_files::model::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));

    otto_files::app::run_browser(start)
}

/// Whether `arg` is the freedesktop trash URI, in any of the spellings that
/// reach a scheme handler: `trash:`, `trash://`, `trash:///` and a path under
/// it. The scheme is compared without case, as URI schemes are.
fn is_trash_uri(arg: &str) -> bool {
    let Some(rest) = arg.get(..6) else {
        return false;
    };
    rest.eq_ignore_ascii_case("trash:")
}

#[cfg(test)]
mod tests {
    use super::is_trash_uri;

    #[test]
    fn the_trash_uri_is_recognised_however_it_is_spelled() {
        for uri in ["trash:", "trash://", "trash:///", "TRASH:///", "trash:///sub"] {
            assert!(is_trash_uri(uri), "{uri} should open the Trash");
        }
        for other in ["file:///home", "--trash", "trash", "/home/trash"] {
            assert!(!is_trash_uri(other), "{other} should not open the Trash");
        }
    }
}
