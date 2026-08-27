//! Quick View — press space on a file and see it.
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
//! desktop's file view. The `otto-quickview` binary remains for previewing a
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
//!     otto_quickview::run_worker_if_requested();
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
pub mod opening;
pub mod payload;
pub mod sandbox;
pub mod spawn;
pub mod uri;

pub use otto_kit::preview::{Fact, Pixels, Preview, PreviewLayout, Row};
pub use spawn::{decode_path, open, Opened};

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
