//! Peek — press space on a file and see it.
//!
//! A **library the file views embed**, not a service they call. The preview is
//! a subsurface of whichever window is showing files, which is what makes it
//! feel attached to the file rather than summoned on top of it: a subsurface's
//! parent must be a `wl_surface` owned by the same client, so being parented
//! and being a separate process are mutually exclusive. Choosing parented also
//! dissolves the anchor problem — the host already knows where the row is in
//! its own surface — and hands stacking, focus and dismissal to the parent
//! window instead of leaving them to be managed by hand.
//!
//! Three hosts embed it: the file browser, the save/open file dialog, and the
//! desktop's file view. The `otto-peek` binary remains for previewing a
//! path from a terminal.
//!
//! # What lives where
//!
//! * [`otto_kit::preview`] — the drawing half. Canvas-pure, no `AppContext`,
//!   no wayland-client, so a host draws it into its own surface and the
//!   compositor can draw the same thing server-side.
//! * [`decode`] — the decoders, which run **only** inside the sandboxed worker.
//! * [`spawn`] — the host-side entry point: open a file, run a contained
//!   worker, enforce the deadline.
//! * [`opening`] — the entrance geometry, now usable for real: the host has the
//!   item's rect, so the card can grow out of the file.
//!
//! # Embedding
//!
//! Untrusted files are parsed in a separate process, and that process is *this
//! binary re-executed*. Since the host is the binary once it embeds the
//! library, the host must give the worker a way in — one line, first thing in
//! `main`, before any thread or Wayland connection exists:
//!
//! ```no_run
//! fn main() {
//!     otto_peek::run_worker_if_requested();
//!     // ... the host's own startup
//! }
//! ```
//!
//! Without it the previewer would re-exec the host as a *host*, which would
//! start a second file browser instead of decoding anything.

// The doctest above spells out `fn main` deliberately: where in `main` the
// call goes is the thing being documented.
#![allow(clippy::needless_doctest_main)]

pub mod decode;
pub mod ocr;
pub mod opening;
pub mod payload;
pub mod sandbox;
pub mod spawn;
pub mod thumbcache;
pub mod thumbnailer;
pub mod uri;

pub use otto_kit::preview::{Fact, Pixels, Preview, PreviewLayout, Row, Word};
pub use spawn::{decode_path, open, Opened};

/// A thumbnail of `path`, as Files shows one in place of its icon: the shared
/// thumbnail cache's, or failing that one decoded in the sandbox. `None` for a
/// file whose preview is not a picture (text, an archive), or one that can't
/// be read.
///
/// `modified` is the file's modification time, which a cached thumbnail must
/// match to count.
///
/// **Blocks.** It reads files and may spawn the sandboxed decode worker, so it
/// belongs on a background thread. The host must call
/// [`run_worker_if_requested`] at the top of `main`, since the worker is the
/// host's own executable.
pub fn thumbnail(
    path: &std::path::Path,
    modified: Option<std::time::SystemTime>,
    size: thumbcache::Size,
) -> Option<skia_safe::Image> {
    if let Some(image) = thumbcache::lookup(path, modified, size) {
        return Some(image);
    }
    // The same sandboxed decoder Peek uses, asked for a thumbnail-sized
    // picture rather than a panel-sized one. Untrusted bytes are parsed in the
    // worker, never here.
    let request = decode::Request {
        width: size.pixels(),
        height: size.pixels(),
        // A thumbnail shows one frame, so asking for an animation would buy a
        // strip of hundreds and keep the first of them.
        animate: false,
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Default::default()
    };
    match decode_path(path, &request) {
        Preview::Pixels { pixels, .. } => pixels.to_image(),
        // Everything else a previewer can return — a text listing, an
        // archive's contents, an unavailable file — is not a picture, and
        // standing it in for one would put a grey card where the type icon
        // says something useful.
        _ => None,
    }
}

/// Run the sandboxed decode worker if this process was started as one.
///
/// Returns `false` for a normal start, so the caller carries on. Never returns
/// at all when it *is* a worker: it decodes one file, writes one payload to
/// stdout and exits.
///
/// Must be called before the host starts threads, connects to Wayland, or
/// touches the environment — the worker inherits whatever exists at that
/// moment, and the point of the sandbox is that it inherits almost nothing.
pub fn run_worker_if_requested() -> bool {
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() != Some("--decode-worker") {
        return false;
    }
    let rest: Vec<String> = arguments.collect();
    std::process::exit(decode::run_worker(decode::parse_request(&rest)));
}
