//! Preview: one file in a window of its own.
//!
//! The window draws the file exactly as Peek does over the file list, through
//! the same [`otto_files::peek::Session`] and the same toolkit renderer, with a
//! titlebar and a toolbar around it instead of the panel's title strip. One
//! file per window, and one process for all of them: a later start hands its
//! file to the running Preview over the session bus (see [`instance`]) and
//! leaves. See `specs/preview-app.md`.
//!
//! ```sh
//! cargo run -p otto-preview -- ~/Pictures/photo.jpg
//! ```

// Rust guideline compliant 2026-02-21

mod app;
mod chat;
mod chrome;
mod content;
mod cursors;
mod instance;
mod marks;
mod mcp;
mod sidebar;
mod viewer;

use std::io::Read;
use std::path::{Path, PathBuf};

use otto_files::imagesize::{self, Header};

/// The largest window a picture is fitted into when it opens, in points.
const FIT_W: f32 = 1100.0;
const FIT_H: f32 = 800.0;
/// The share of the room the compositor offers a new window (its suggested
/// bounds) that the opening size may take, across and down. A window that
/// fills the screen reads as a takeover rather than a document.
const BOUNDS_SHARE: (f32, f32) = (0.75, 0.8);
/// The window for a file read from the top down: a PDF or a Markdown document.
const DOCUMENT_SIZE: (f32, f32) = (760.0, 860.0);
/// The window for everything else.
const DEFAULT_SIZE: (f32, f32) = (960.0, 720.0);
/// How much of a picture is read to find its size in the header. A JPEG's
/// dimensions can sit behind a large EXIF block.
const HEADER_BYTES: u64 = 256 * 1024;

const USAGE: &str = "usage: otto-preview [--chat] PATH";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // First, before the runtime starts a thread: the sandboxed decode worker
    // is this binary re-executed, and this returns at once on a normal start.
    otto_peek::run_worker_if_requested();
    // The document tools for an agent: no display, no window, no runtime.
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        return mcp::serve();
    }
    otto_kit::i18n::init_from_desktop();

    let Some((path, chat)) = path_from_args() else {
        return Ok(());
    };
    tokio::runtime::Runtime::new()?.block_on(run(path, chat))
}

/// The file named on the command line, as an absolute path, and whether
/// `--chat` asks for the chat beside it. `None` after answering `--help` or
/// `--version`; exits when no file is named.
fn path_from_args() -> Option<(PathBuf, bool)> {
    let mut path = None;
    let mut chat = false;
    for argument in std::env::args_os().skip(1) {
        match argument.to_str() {
            Some("--help" | "-h") => {
                println!("{USAGE}");
                return None;
            }
            Some("--version" | "-V") => {
                println!("otto-preview {}", env!("CARGO_PKG_VERSION"));
                return None;
            }
            Some("--chat") => chat = true,
            Some(other) if other.starts_with('-') && other.len() > 1 => {
                eprintln!("otto-preview: unknown option {other}");
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
            _ if path.is_none() => path = Some(PathBuf::from(argument)),
            _ => {}
        }
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    Some((std::path::absolute(&path).unwrap_or(path), chat))
}

async fn run(path: PathBuf, chat: bool) -> Result<(), Box<dyn std::error::Error>> {
    otto_kit::logging::init("info");

    // A launcher that brought us up with an activation token passes it on,
    // so the Preview that ends up showing the file may come forward with it.
    let request = instance::Request {
        path,
        token: std::env::var("XDG_ACTIVATION_TOKEN").ok(),
        chat,
    };
    let inbox = instance::Inbox::default();
    let documents = instance::Documents::default();
    // Held for the life of the process: the bus name goes with it.
    let _service =
        match instance::claim_or_forward(&request, inbox.clone(), documents.clone()).await {
            Ok(instance::Role::Forwarded) => return Ok(()),
            Ok(instance::Role::Owner(connection)) => Some(connection),
            Err(err) => {
                tracing::warn!(%err, "no session bus; this Preview stands alone");
                None
            }
        };

    // Needs the runtime: without the icon theme every lookup searches hicolor
    // alone, and a card or a listing draws with no icons.
    otto_kit::icon_theme::spawn_icon_theme_watcher();

    otto_kit::AppRunner::new(app::PreviewApp::new(request, inbox, documents)).run()
}

/// What decides the window's size on opening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A picture, with its size in pixels from its header.
    Picture(f32, f32),
    /// A file read from the top down: a PDF or a Markdown document.
    Document,
    /// Anything else.
    Other,
}

impl Shape {
    /// The shape of the file at `path`, read from its header or its name.
    pub fn of(path: &Path) -> Self {
        if let Some((width, height)) = picture_size(path) {
            return Self::Picture(width, height);
        }
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
        match extension.as_deref() {
            Some("pdf" | "md" | "markdown") => Self::Document,
            _ => Self::Other,
        }
    }

    /// The window's size on opening, in points.
    ///
    /// A picture keeps its own shape, fitted with the chrome on top into
    /// [`FIT_W`] × [`FIT_H`] and into [`BOUNDS_SHARE`] of `bounds`, the room
    /// the compositor suggests for a new window, when it has said. A picture
    /// smaller than that opens at its own size. A document opens in a
    /// portrait window and anything else in a general-purpose one, both
    /// shrunk to the same limit. Never smaller than the window's minimum.
    pub fn opening_size(self, bounds: Option<(f32, f32)>) -> (f32, f32) {
        let limit = match bounds {
            Some((width, height)) if width > 0.0 && height > 0.0 => (
                FIT_W.min(width * BOUNDS_SHARE.0),
                FIT_H.min(height * BOUNDS_SHARE.1),
            ),
            _ => (FIT_W, FIT_H),
        };
        let (width, height) = match self {
            Self::Picture(width, height) if width > 0.0 && height > 0.0 => {
                let chrome = chrome::chrome_h(Default::default());
                let pad = 2.0 * otto_kit::preview::IMAGE_PADDING;
                let room = (limit.0 - pad, limit.1 - chrome - pad);
                let scale = (room.0 / width).min(room.1 / height).clamp(0.0, 1.0);
                (width * scale + pad, height * scale + pad + chrome)
            }
            Self::Document => DOCUMENT_SIZE,
            Self::Picture(..) | Self::Other => DEFAULT_SIZE,
        };
        (
            width.min(limit.0).max(app::MIN_W).round(),
            height.min(limit.1).max(app::MIN_H).round(),
        )
    }
}

/// A picture's size from its header, without decoding it here: the bytes
/// are only measured, never handed to a codec in this process.
fn picture_size(path: &Path) -> Option<(f32, f32)> {
    // A pipe or a device would block the open, or the read, forever.
    if !path.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(HEADER_BYTES).read_to_end(&mut bytes).ok()?;
    match imagesize::parse(&bytes) {
        Header::Size(width, height) => Some((width as f32, height as f32)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_sized_picture_keeps_its_shape_inside_the_limit() {
        let (width, height) = Shape::Picture(2880.0, 1920.0).opening_size(Some((1440.0, 900.0)));
        assert!(width <= 1440.0 * BOUNDS_SHARE.0 && height <= 900.0 * BOUNDS_SHARE.1);
        let chrome = chrome::chrome_h(Default::default());
        let pad = 2.0 * otto_kit::preview::IMAGE_PADDING;
        let aspect = (width - pad) / (height - chrome - pad);
        assert!((aspect - 1.5).abs() < 0.01, "aspect {aspect}");
    }

    #[test]
    fn without_bounds_the_fixed_limit_holds() {
        let (width, height) = Shape::Picture(8000.0, 2000.0).opening_size(None);
        assert!(width <= FIT_W && height <= FIT_H);
    }

    #[test]
    fn a_small_picture_opens_at_its_own_size() {
        let (width, _) = Shape::Picture(600.0, 400.0).opening_size(Some((1440.0, 900.0)));
        assert_eq!(width, 600.0 + 2.0 * otto_kit::preview::IMAGE_PADDING);
    }

    #[test]
    fn a_document_is_shrunk_to_a_short_screen() {
        let (_, height) = Shape::Document.opening_size(Some((1440.0, 800.0)));
        assert!(height <= 800.0 * BOUNDS_SHARE.1);
    }
}
