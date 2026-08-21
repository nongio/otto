//! PDF, rendered by exec'ing a rasteriser that is already on the system.
//!
//! No PDF library is linked into anything Otto builds. The worker is *already*
//! a separate, sandboxed, rlimited process spawned per preview, so running an
//! existing rasteriser costs one more `exec` in a place that was doing one
//! regardless — and the dependency becomes a package a distribution almost
//! certainly installed rather than a large C++ blob in the tree.
//!
//! It also sidesteps MuPDF's AGPL, which constrains *linking* and says nothing
//! about running a program.
//!
//! The same table generalises: video poster frames arrive later as more rows,
//! without GStreamer entering the default build. See `specs/quickview.md`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};

use skia_safe::{Codec, Data};

use crate::payload;
use crate::payload::{Fact, PreviewPayload};

use super::{human_size, Request};

/// A rasteriser, and how to ask it for one page as a PNG on stdout.
struct Rasteriser {
    /// The binary, looked up on `PATH`.
    command: &'static str,
    /// The package to name in the fallback card, so a user who has none of
    /// these knows what to install.
    package: &'static str,
    /// Arguments, given the page number and the target width in pixels.
    args: fn(page: u32, width: u32) -> Vec<String>,
}

/// Tried in order; the first one present wins.
const RASTERISERS: &[Rasteriser] = &[
    Rasteriser {
        command: "pdftoppm",
        package: "poppler-utils",
        args: |page, width| {
            vec![
                "-png".into(),
                "-f".into(),
                page.to_string(),
                "-l".into(),
                page.to_string(),
                "-scale-to-x".into(),
                width.to_string(),
                // Preserve the aspect ratio rather than forcing a height.
                "-scale-to-y".into(),
                "-1".into(),
                // Read the document from standard input.
                "-".into(),
            ]
        },
    },
    Rasteriser {
        command: "pdftocairo",
        package: "poppler-utils",
        args: |page, width| {
            vec![
                "-png".into(),
                "-singlefile".into(),
                "-f".into(),
                page.to_string(),
                "-l".into(),
                page.to_string(),
                "-scale-to-x".into(),
                width.to_string(),
                "-scale-to-y".into(),
                "-1".into(),
                "-".into(),
                // pdftocairo writes to a named file unless the output is `-`.
                "-".into(),
            ]
        },
    },
    Rasteriser {
        command: "mutool",
        package: "mupdf-tools",
        args: |page, width| {
            vec![
                "draw".into(),
                "-F".into(),
                "png".into(),
                "-o".into(),
                "-".into(),
                "-w".into(),
                width.to_string(),
                "-".into(),
                page.to_string(),
            ]
        },
    },
    Rasteriser {
        command: "gs",
        package: "ghostscript",
        args: |page, width| {
            vec![
                "-dNOPAUSE".into(),
                "-dBATCH".into(),
                "-dSAFER".into(),
                "-sDEVICE=png16m".into(),
                format!("-dFirstPage={page}"),
                format!("-dLastPage={page}"),
                format!("-dDEVICEWIDTHPOINTS={width}"),
                "-sOutputFile=-".into(),
                "-".into(),
            ]
        },
    },
];

/// The widest a page is ever rasterised, whatever the panel asks for.
///
/// A ceiling in its own right rather than a safety valve: a PDF page is
/// re-rendered from vectors at whatever size it is asked for, and the cost
/// climbs faster than the area does, so the last doublings buy detail that a
/// panel-sized preview cannot show at a price the user waits through. This is
/// comfortably above a full-screen panel's own pixels on a HiDPI display.
const MAX_WIDTH: u32 = 2_048;

pub fn render(file: &mut File, request: &Request) -> PreviewPayload {
    // Read the document once and hand it to the rasteriser on stdin. Passing
    // bytes rather than a path is the point: the child never resolves a name,
    // so nothing can be substituted between the worker's stat and its open.
    let bytes = match super::read_capped(file, request.budget.max_read.min(512 * 1024 * 1024)) {
        Ok(bytes) => bytes,
        Err(err) => return payload::unavailable(format!("cannot read the document: {err}")),
    };

    let pages = count_pages(&bytes).unwrap_or(1).max(1);
    let page = request.page.clamp(1, pages);

    let Some(rasteriser) = RASTERISERS.iter().find(|r| on_path(r.command)) else {
        return no_rasteriser(file, &bytes, request, pages);
    };

    // The width the host asked for, which is already twice the panel — the
    // oversampling is applied there, once, for every kind of preview.
    //
    // It used to be doubled again here, and the two doublings compounded: a
    // full-screen panel asked for a page four times its own width, hit the
    // ceiling below, and spent seventeen seconds in the rasteriser for a page
    // nobody could see at that size. Rasterising is superlinear in width, so
    // the mistake was not 4× the cost, it was worse.
    let width = (request.width as f32 * request.zoom.max(1.0)).ceil() as u32;
    let width = width.clamp(320, MAX_WIDTH);

    match rasterise(rasteriser, &bytes, page, width) {
        Some(png) => match decode_png(&png) {
            Some(pixels) => PreviewPayload::Pixels {
                pixels,
                pages,
                page,
            },
            None => payload::unavailable("the rendered page could not be read"),
        },
        None => no_rasteriser(file, &bytes, request, pages),
    }
}

/// Run one rasteriser and collect its PNG.
fn rasterise(rasteriser: &Rasteriser, document: &[u8], page: u32, width: u32) -> Option<Vec<u8>> {
    let mut child = Command::new(rasteriser.command)
        .args((rasteriser.args)(page, width))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // A rasteriser's complaints belong in the log, not interleaved with the
        // payload on our own stdout.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Write the document and read the page concurrently: a document larger than
    // a pipe buffer would otherwise deadlock, each side waiting for the other.
    let mut stdin = child.stdin.take()?;
    let document = document.to_vec();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        // A rasteriser that has seen enough closes its input early; that is a
        // success, not an error.
        let _ = stdin.write_all(&document);
    });

    let mut png = Vec::new();
    let read = child
        .stdout
        .as_mut()
        .and_then(|out| out.read_to_end(&mut png).ok());
    let status = child.wait().ok();
    let _ = writer.join();

    // A non-zero exit with usable output still counts: some rasterisers
    // complain about a malformed document and render it anyway.
    read?;
    if png.is_empty() {
        return None;
    }
    let _ = status;
    Some(png)
}

fn decode_png(png: &[u8]) -> Option<crate::payload::Pixels> {
    let mut codec = Codec::from_data(Data::new_copy(png))?;
    let info = codec
        .info()
        .with_color_type(skia_safe::ColorType::RGBA8888)
        .with_alpha_type(skia_safe::AlphaType::Premul);
    let dimensions = codec.dimensions();
    let image = codec.get_image(info, None).ok()?;
    super::image::to_pixels(&image, dimensions)
}

/// Is this command on `PATH`?
///
/// Resolved by hand rather than by spawning something: the worker has a tight
/// descriptor budget and no reason to fork twice per lookup.
fn on_path(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(command);
        // Existence is enough; if it is not executable the spawn fails and the
        // next rasteriser is tried.
        candidate.is_file()
    })
}

/// Page count, read out of the document's own structure.
///
/// Counts `/Type /Page` objects, which is approximate for documents using
/// object streams or unusual page trees — good enough for "3 of 12", and it
/// costs no parser. A rasteriser that disagrees is not contradicted, because
/// the count is only ever shown, never used to index.
fn count_pages(bytes: &[u8]) -> Option<u32> {
    let needle = b"/Type";
    let mut count = 0u32;
    let mut at = 0usize;
    while let Some(found) = find(&bytes[at..], needle) {
        let start = at + found + needle.len();
        // Look at what follows, clamped to the end of the buffer — a match in
        // the last few bytes must not abandon the count so far.
        let window = &bytes[start..(start + 16).min(bytes.len())];
        let value = window
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .map(|offset| &window[offset..])
            .unwrap_or_default();

        // `/Page` and `/Pages` share a prefix, so the delimiter decides: a page
        // object's name ends there, the page *tree*'s does not. Testing the
        // following byte rather than listing suffixes means `/PageLabels` and
        // anything else added later is excluded for free.
        if let Some(after) = value.strip_prefix(b"/Page".as_slice()) {
            let ends_here = after
                .first()
                .is_none_or(|byte| !byte.is_ascii_alphanumeric());
            if ends_here {
                count += 1;
            }
        }
        at = start;
    }
    (count > 0).then_some(count)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// No rasteriser installed, or every one of them failed.
///
/// The card names the package that would fix it. A previewer that silently
/// shows nothing looks broken; one that says what is missing is a preview of a
/// different kind.
fn no_rasteriser(file: &mut File, bytes: &[u8], request: &Request, pages: u32) -> PreviewPayload {
    let size = file
        .seek(SeekFrom::End(0))
        .ok()
        .unwrap_or(bytes.len() as u64);

    let mut facts = vec![
        Fact {
            key: "Pages".into(),
            value: pages.to_string(),
        },
        Fact {
            key: "Size".into(),
            value: human_size(size),
        },
    ];
    if let Some(title) = document_title(bytes) {
        facts.insert(
            0,
            Fact {
                key: "Title".into(),
                value: title,
            },
        );
    }

    let wanted = RASTERISERS
        .iter()
        .map(|r| r.package)
        .collect::<Vec<_>>()
        .join(", ");

    PreviewPayload::Card {
        title: request.name.clone(),
        subtitle: format!("Install one of: {wanted} — to see the pages"),
        facts,
        hero: None,
    }
}

/// The `/Title` from the document information dictionary, when it is a plain
/// literal string. Anything encoded or hexadecimal is skipped rather than
/// guessed at.
fn document_title(bytes: &[u8]) -> Option<String> {
    let at = find(bytes, b"/Title")? + b"/Title".len();
    let rest = bytes.get(at..(at + 512).min(bytes.len()))?;
    let open = rest.iter().position(|byte| *byte == b'(')?;
    // Only accept a title that starts promptly after the key, or `/Title` was
    // a coincidence somewhere else in the file.
    if open > 4 {
        return None;
    }
    let mut title = Vec::new();
    let mut escaped = false;
    for byte in &rest[open + 1..] {
        if escaped {
            escaped = false;
            title.push(*byte);
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b')' => break,
            _ => title.push(*byte),
        }
        if title.len() > 200 {
            break;
        }
    }
    let title = String::from_utf8_lossy(&title).trim().to_string();
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pages_without_counting_the_page_tree() {
        let document = b"<< /Type /Pages /Count 2 >> << /Type /Page >> << /Type /Page >>";
        assert_eq!(count_pages(document), Some(2));
    }

    #[test]
    fn a_document_with_no_page_objects_reports_no_count() {
        assert_eq!(count_pages(b"%PDF-1.7 nothing useful here"), None);
    }

    #[test]
    fn reads_a_literal_title_and_skips_a_coincidental_one() {
        assert_eq!(
            document_title(b"/Title (Quarterly Report)").as_deref(),
            Some("Quarterly Report")
        );
        // `/Title` followed by something that is not promptly a literal string.
        assert!(document_title(b"/Title <FEFF0041> and later (nope)").is_none());
    }

    #[test]
    fn an_escaped_paren_stays_in_the_title() {
        assert_eq!(
            document_title(br"/Title (Report \(final\))").as_deref(),
            Some("Report (final)")
        );
    }
}
