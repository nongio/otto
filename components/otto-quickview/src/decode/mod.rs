//! The decode worker: the only place in Otto that interprets an untrusted file.
//!
//! Runs as a separate short-lived process (`otto-quickview --decode-worker`),
//! sandboxed by [`crate::sandbox`], holding one read-only descriptor and
//! writing one [`PreviewPayload`] to stdout. It has no Wayland connection, no
//! bus connection, and no path it could open.
//!
//! Dispatch is on the **sniffed** type, never on the file's name. A file called
//! `.png` that is not one must not reach the PNG decoder; the sandbox would
//! contain the consequences either way, but the correct preview is the one
//! matching the bytes.

mod image;
mod listing;
mod media;
mod pdf;
mod text;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::FromRawFd;

use otto_kit::filetype;

use crate::payload;
use crate::payload::PreviewPayload;
use crate::sandbox::{self, Budget, FILE_FD};

/// What the parent asked for. Everything the decoders need to know, and
/// nothing about where the file lives.
#[derive(Debug, Clone)]
pub struct Request {
    /// Target size in physical pixels — roughly twice the window, so a scaled
    /// decode still has detail to show.
    pub width: u32,
    pub height: u32,
    /// 1-based page for paginated content.
    pub page: u32,
    /// Zoom factor being displayed. Past 1.0 the image decoders stop
    /// downsampling, which is what keeps a photograph sharp when you look
    /// closely.
    pub zoom: f32,
    /// The type the parent sniffed. The worker re-sniffs anyway — it has the
    /// bytes and the parent's answer is a hint, not a trust boundary.
    pub mime: String,
    /// The file's display name. Used only in card titles; never for dispatch.
    pub name: String,
    pub budget: Budget,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 1200,
            page: 1,
            zoom: 1.0,
            mime: String::new(),
            name: String::new(),
            budget: Budget::default(),
        }
    }
}

/// How much of the file the sniffer sees. The shared-mime-info magic rules do
/// not look further than this.
const SNIFF_BYTES: usize = 4096;

/// Worker entry point. Returns the process exit code.
///
/// Never propagates an error to the parent as a non-zero exit when it could
/// instead say *why* on the wire: "unavailable, and here is the reason" is a
/// preview, and a dead worker is not.
pub fn run_worker(request: Request) -> i32 {
    // From the environment, not from the portal: the worker holds no bus
    // connection by design. The parent forwards its own locale in `LANGUAGE`
    // (see `spawn::decode`), so the two agree.
    otto_kit::i18n::init_from_env();

    // SAFETY: the worker's own `main` runs before any thread is started, and
    // the parent applied the same limits pre-exec. Applying them again covers
    // what could not survive `execve`.
    if let Err(err) = unsafe { sandbox::apply(request.budget) } {
        // Refuse to decode rather than decode uncontained.
        let fallback = payload::unavailable(otto_kit::t_owned!(
            "quickview-error-sandbox",
            error = err.to_string()
        ));
        let _ = payload::write_to(&fallback, &mut std::io::stdout());
        return 0;
    }

    // SAFETY: the parent guarantees this descriptor is open and read-only.
    let mut file = unsafe { File::from_raw_fd(FILE_FD) };

    let payload = decode(&mut file, &request);
    let mut stdout = std::io::stdout();
    if payload::write_to(&payload, &mut stdout).is_err() || stdout.flush().is_err() {
        return 1;
    }
    0
}

/// Choose a previewer and run it, and give what comes back the file's icon.
///
/// The icon is stamped here rather than in each decoder because this is where
/// the *sniffed* type is known — the worker read the bytes, and the parent only
/// ever had the name. A decoder that drew the file itself is left alone; one
/// that could only describe it, or gave up, gets the picture the browser's own
/// listing was showing for that file.
fn decode(file: &mut File, request: &Request) -> PreviewPayload {
    let (payload, mime) = previewed(file, request);
    payload::with_icon(
        payload,
        match mime {
            Some(mime) => otto_kit::filetype::icon_names(mime),
            // Nothing was sniffed — the file could not be read that far. The
            // parent stamps its name-derived chain over this on the way out.
            None => Vec::new(),
        },
    )
}

/// The previewer's own answer, and the type it was chosen by. `None` for a
/// file that could not be read as far as its signature.
fn previewed(file: &mut File, request: &Request) -> (PreviewPayload, Option<&'static str>) {
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(err) => {
            return (
                payload::unavailable(otto_kit::t_owned!(
                    "quickview-error-stat-file",
                    error = err.to_string()
                )),
                None,
            )
        }
    };

    if metadata.is_dir() {
        return (listing::directory(file, request), Some("inode/directory"));
    }
    if metadata.len() == 0 {
        // Nothing to sniff, so nothing to be truthful about: the card gets the
        // parent's name-derived icon, which is what the row shows too.
        return (
            PreviewPayload::Card {
                title: request.name.clone(),
                subtitle: otto_kit::t_owned!("quickview-empty-file"),
                facts: vec![],
                hero: None,
                icon: Vec::new(),
            },
            None,
        );
    }

    let mut head = vec![0u8; SNIFF_BYTES.min(metadata.len() as usize)];
    if let Err(err) = file.read_exact(&mut head) {
        return (
            payload::unavailable(otto_kit::t_owned!(
                "quickview-error-read-file",
                error = err.to_string()
            )),
            None,
        );
    }
    if file.seek(SeekFrom::Start(0)).is_err() {
        return (
            payload::unavailable(otto_kit::t_owned!("quickview-error-not-seekable")),
            None,
        );
    }

    // Content decides which decoder runs; the name may only pick a *subtype* of
    // what the content already proved. `filetype::refine_opt` is that rule, and
    // it lives in the shared module rather than here so the browser's
    // thumbnailer and this previewer cannot drift apart on it.
    //
    // Why it is needed at all: an SVG behind an XML declaration and a comment
    // sniffs as `application/xml`, and would be read as source code rather than
    // drawn — and whether it does so depends on the icon theme, since some
    // themes' SVGs sniff cleanly and others do not. Refining is what makes that
    // consistent. It stays safe because a `.png` full of XML is not refined:
    // `image/png` is not a subclass of `application/xml`, so the PNG decoder
    // remains unreachable.
    //
    // `None` in, `None` out: `sniff` already falls back to `text/plain` for
    // anything textual, so a `None` means "binary, no signature matched", and
    // unidentifiable content does not become identifiable through its name.
    let Some(mime) = filetype::refine_opt(filetype::sniff(&head), &request.name) else {
        const UNKNOWN: &str = "application/octet-stream";
        return (media::generic(&metadata, request, UNKNOWN), Some(UNKNOWN));
    };

    (dispatch(mime, file, &metadata, request), Some(mime))
}

fn dispatch(
    mime: &str,
    file: &mut File,
    metadata: &std::fs::Metadata,
    request: &Request,
) -> PreviewPayload {
    // Order matters only where types overlap: SVG is `image/svg+xml` and also
    // a subclass of text, and it should be drawn rather than read.
    if mime == "image/svg+xml" {
        return image::svg(file, request);
    }
    if mime == "application/pdf" {
        return pdf::render(file, request);
    }
    if mime.starts_with("image/") {
        return image::raster(file, request);
    }
    if mime == "application/zip" || filetype::is_subclass_of(mime, "application/zip") {
        return listing::zip(file, metadata, request);
    }
    if mime == "application/x-tar" {
        return listing::tar(file, metadata, request);
    }
    if mime.starts_with("audio/") {
        return media::audio(file, metadata, request, mime);
    }
    if mime.starts_with("video/") {
        return media::video(file, metadata, request, mime);
    }
    // Text last, and via the hierarchy rather than a language list: every
    // source file in existence is a subclass of text/plain, and enumerating
    // them would be a losing game.
    if mime == "text/plain" || filetype::is_subclass_of(mime, "text/plain") {
        return text::read(file, request, mime);
    }

    media::generic(metadata, request, mime)
}

/// Parse the worker's own arguments.
///
/// Lives here rather than in the binary because every embedding host re-execs
/// itself as a worker, so all of them must parse these identically.
///
/// Deliberately total: a worker that cannot understand an argument still
/// previews, with defaults, rather than dying and looking like a decoder
/// failure.
pub fn parse_request(arguments: &[String]) -> Request {
    let mut request = Request::default();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let mut value = || rest.next().cloned().unwrap_or_default();
        match argument.as_str() {
            "--width" => request.width = value().parse().unwrap_or(request.width),
            "--height" => request.height = value().parse().unwrap_or(request.height),
            "--page" => request.page = value().parse().unwrap_or(request.page),
            "--zoom" => request.zoom = value().parse().unwrap_or(request.zoom),
            "--name" => request.name = value(),
            "--mime" => request.mime = value(),
            _ => {}
        }
    }
    request
}

/// A bounded read. Every decoder goes through this rather than `read_to_end`:
/// the budget is the difference between previewing a 2 GB video and trying to
/// hold one in memory.
pub(crate) fn read_capped(file: &mut File, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Human-readable byte count, for the facts on a card.
pub(crate) fn human_size(bytes: u64) -> String {
    // Below a kilobyte the count is exact and needs a plural rule; above it
    // the unit is a symbol and only the number varies.
    if bytes < 1024 {
        return otto_kit::t_owned!("quickview-size-bytes", count = bytes as f64);
    }
    const UNITS: [&str; 4] = [
        "quickview-size-kb",
        "quickview-size-mb",
        "quickview-size-gb",
        "quickview-size-tb",
    ];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    otto_kit::t_owned!(UNITS[unit], value = format!("{value:.1}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_reads_naturally() {
        // Against the source catalogue: the test asserts the shape of the
        // string, and the shape is what a translator preserves.
        otto_kit::i18n::init(&["en-GB".to_string()]);
        assert_eq!(human_size(0), "0 bytes");
        assert_eq!(human_size(999), "999 bytes");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MB");
    }
}
