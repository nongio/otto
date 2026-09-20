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
//! without GStreamer entering the default build. See `specs/peek.md`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};

use skia_safe::{Codec, Data};

use crate::payload;
use crate::payload::{Fact, Page, Pixels, PreviewPayload, Word};

use super::{human_size, on_path, Request};

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
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "peek-error-read-document",
                error = err.to_string()
            ))
        }
    };

    if request.text {
        return text_layer(&bytes);
    }

    // `pdfinfo` reads the page tree and is right about documents the scan
    // below cannot count — anything whose objects are in compressed streams,
    // which is most of what a modern producer writes. The scan stays as the
    // answer for a system with no poppler on it.
    let sizes = request.document.then(|| page_sizes(&bytes)).flatten();
    let pages = sizes
        .as_ref()
        .map(|sizes| sizes.len() as u32)
        .or_else(|| count_pages(&bytes))
        .unwrap_or(1)
        .max(1);
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
    //
    // A document is the same lesson once more: it rests with a whole page in
    // the panel, gutters and all, so the page is drawn at a fraction of the
    // panel's width and that fraction is what is asked for. The host sends
    // the box in physical pixels rather than an oversampled one, and asks
    // again for a wider raster if the reader zooms in.
    let width = match &sizes {
        Some(sizes) => {
            let strip: Vec<Page> = sizes
                .iter()
                .map(|(width, height)| Page::blank(*width, *height))
                .collect();
            otto_kit::preview::page_raster_width(
                skia_safe::Rect::from_wh(request.width as f32, request.height as f32),
                &strip,
                (page as usize).saturating_sub(1),
            )
            .ceil() as u32
        }
        None => (request.width as f32 * request.zoom.max(1.0)).ceil() as u32,
    };
    let width = width.clamp(320, MAX_WIDTH);

    match rasterise(rasteriser, &bytes, page, width) {
        Some(png) => match decode_png(&png) {
            Some(mut pixels) => {
                // A rasterised page is a picture like any other, so a
                // document with no text layer — a scan — is recognised like
                // any other picture. One with a text layer never asks for
                // this: its words are exact and cost no recogniser.
                if request.ocr {
                    pixels.words = crate::ocr::recognise(
                        &pixels,
                        request.recogniser_command(),
                        &request.languages,
                    );
                }
                if !request.document {
                    return PreviewPayload::Pixels {
                        pixels,
                        pages,
                        page,
                    };
                }
                strip(sizes, pages, page, pixels)
            }
            None => payload::unavailable(otto_kit::t_owned!("peek-error-page-readback")),
        },
        None => no_rasteriser(file, &bytes, request, pages),
    }
}

/// How far into a document the text layer is read.
///
/// Reading it costs roughly a millisecond a page, which is nothing for a
/// report and a visible wait for a thousand-page manual. Past this the
/// document still scrolls and still shows every page; what it loses is the
/// selection on pages nobody has scrolled a thousand pages to reach.
const MAX_TEXT_PAGES: u32 = 1_000;

/// The document's own text, boxed, with no pixels.
///
/// A second, cheaper pass than rendering: the host has the pages up already
/// and this fills in what they *say*, so the words come from the file rather
/// than from a recogniser's reading of a picture of the file. Exact, free of
/// a recogniser, and right for a page that has not been rasterised at all.
fn text_layer(document: &[u8]) -> PreviewPayload {
    if !on_path("pdftotext") {
        return payload::unavailable(otto_kit::t_owned!(
            "peek-pdf-install-rasteriser",
            packages = "poppler-utils"
        ));
    }
    let out = run(
        "pdftotext",
        &[
            "-bbox-layout".into(),
            "-f".into(),
            "1".into(),
            "-l".into(),
            MAX_TEXT_PAGES.to_string(),
            "-".into(),
            "-".into(),
        ],
        document,
    );
    let Some((pages, words)) = out
        .as_deref()
        .map(String::from_utf8_lossy)
        .and_then(|xml| parse_bbox(&xml))
    else {
        return payload::unavailable(otto_kit::t_owned!("peek-error-page-readback"));
    };
    PreviewPayload::Pages { pages, words }
}

/// `pdftotext -bbox-layout`, which is XHTML with a `<word>` per word inside
/// the `<flow>`, `<block>` and `<line>` it was read in.
///
/// Scanned rather than parsed as XML: the shape is fixed, the producer is one
/// known program, and a parser dependency to read four attributes would be a
/// poor trade. Anything unexpected ends the scan with what it has, because
/// half a document's words are still a selection.
///
/// Boxes come out in **strip coordinates** — points, pages stacked — so they
/// are already in the space a selection is made in.
fn parse_bbox(xml: &str) -> Option<(Vec<Page>, Vec<Word>)> {
    /// One word where `pdftotext` put it: on a page, in that page's points.
    struct Raw {
        page: usize,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        text: String,
        paragraph: u32,
        line: u32,
    }

    let mut sizes: Vec<(f32, f32)> = Vec::new();
    let mut raw: Vec<Raw> = Vec::new();
    let mut paragraph = 0u32;
    let mut line = 0u32;
    let mut rest = xml;

    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        let after = &rest[end + 1..];
        rest = after;
        match tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
        {
            "page" => {
                let (Some(width), Some(height)) =
                    (attribute(tag, "width"), attribute(tag, "height"))
                else {
                    break;
                };
                if sizes.len() >= payload::MAX_PAGES {
                    break;
                }
                sizes.push((width, height));
            }
            "block" => paragraph += 1,
            "line" => line += 1,
            "word" => {
                let (Some(page), Some(left), Some(top), Some(right), Some(bottom)) = (
                    sizes.len().checked_sub(1),
                    attribute(tag, "xMin"),
                    attribute(tag, "yMin"),
                    attribute(tag, "xMax"),
                    attribute(tag, "yMax"),
                ) else {
                    break;
                };
                let Some(close) = after.find("</word>") else {
                    break;
                };
                rest = &after[close..];
                if raw.len() >= payload::MAX_DOC_WORDS {
                    break;
                }
                let text = unescape(&after[..close]);
                if text.is_empty() {
                    continue;
                }
                raw.push(Raw {
                    page,
                    left,
                    top,
                    right,
                    bottom,
                    text,
                    paragraph,
                    line,
                });
            }
            _ => {}
        }
    }

    if sizes.is_empty() {
        return None;
    }
    let pages: Vec<Page> = sizes
        .into_iter()
        .map(|(width, height)| Page::blank(width, height))
        .collect();
    let words = raw
        .into_iter()
        .map(|word| {
            let page = otto_kit::preview::page_in_strip(&pages, word.page);
            Word {
                text: word.text,
                left: (page.left + word.left).max(0.0) as u32,
                top: (page.top + word.top).max(0.0) as u32,
                width: (word.right - word.left).max(0.0) as u32,
                height: (word.bottom - word.top).max(0.0) as u32,
                // The file says so: there is nothing to be unsure about.
                confidence: 100,
                // A page is a block, so a selection dragged across a page
                // break copies with a break in it.
                block: word.page as u32,
                paragraph: word.paragraph,
                line: word.line,
            }
        })
        .collect();
    Some((pages, words))
}

/// One numeric attribute of a tag: `xMin="64.8"`.
fn attribute(tag: &str, name: &str) -> Option<f32> {
    let mut at = 0;
    // A prefix match would read `xMin` out of `xMinFoo`, and `width` out of
    // another tag's `widthHint`; the quote after the name is what makes this
    // an attribute rather than a coincidence.
    loop {
        let found = at + tag[at..].find(name)?;
        let after = &tag[found + name.len()..];
        let starts_a_name = found > 0
            && tag[..found]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        if let (false, Some(value)) = (starts_a_name, after.strip_prefix("=\"")) {
            let end = value.find('"')?;
            return value[..end].parse().ok();
        }
        at = found + name.len();
    }
}

/// The five entities `pdftotext` writes. Anything else is its own text.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.trim().to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// The whole document as a strip, with the one page that has been rasterised
/// in its place.
///
/// Every page's geometry is known here and none of its pixels are, which is
/// the point: the host lays the document out, scrolls it and says which page
/// is showing while it holds two pages' worth of image rather than three
/// hundred. It asks for the rest as they come into view.
fn strip(
    sizes: Option<Vec<(f32, f32)>>,
    pages: u32,
    page: u32,
    mut pixels: Pixels,
) -> PreviewPayload {
    let sizes = sizes.unwrap_or_else(|| uniform_sizes(pages, pixels.width, pixels.height));
    let mut strip: Vec<Page> = sizes
        .into_iter()
        .map(|(width, height)| Page::blank(width, height))
        .collect();
    let index = (page as usize).saturating_sub(1).min(strip.len() - 1);

    // Recognised words are boxed in the page image; a document's are boxed in
    // the strip, because that is where a selection is made.
    let words = otto_kit::preview::words_in_strip(
        &strip,
        index,
        &pixels.words,
        pixels.width,
        pixels.height,
    );
    pixels.words = Vec::new();
    strip[index].pixels = Some(pixels);
    PreviewPayload::Pages {
        pages: strip,
        words,
    }
}

/// Every page's size in points, from `pdfinfo`.
///
/// Sizes rather than one size repeated because documents really do mix them
/// — a report with a landscape table in it — and a strip laid out from the
/// first page's shape would put every later page in the wrong place.
fn page_sizes(document: &[u8]) -> Option<Vec<(f32, f32)>> {
    if !on_path("pdfinfo") {
        return None;
    }
    // Asking for more pages than the document has is how the count comes back
    // in the same exec: poppler prints the ones that exist and stops.
    let out = run(
        "pdfinfo",
        &[
            "-f".into(),
            "1".into(),
            "-l".into(),
            payload::MAX_PAGES.to_string(),
            "-".into(),
        ],
        document,
    )?;
    let text = String::from_utf8_lossy(&out);

    let mut sizes: Vec<(f32, f32)> = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("Page") else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let Some(number) = fields.next().and_then(|n| n.parse::<usize>().ok()) else {
            continue;
        };
        match fields.next() {
            // `Page    1 size:  612 x 792 pts (letter)`
            Some("size:") => {
                let width: f32 = fields.next()?.parse().ok()?;
                // The `x` between them.
                fields.next()?;
                let height: f32 = fields.next()?.parse().ok()?;
                if number != sizes.len() + 1 {
                    return None;
                }
                sizes.push((width, height));
            }
            // `Page    1 rot:   90` — a rotated page is shown turned, so the
            // strip has to lay out the shape it will be drawn as.
            Some("rot:") => {
                let degrees: i32 = fields.next()?.parse().ok()?;
                if degrees.rem_euclid(180) == 90 {
                    let size = sizes.get_mut(number.checked_sub(1)?)?;
                    *size = (size.1, size.0);
                }
            }
            _ => {}
        }
    }
    (!sizes.is_empty()).then_some(sizes)
}

/// Every page the shape of the one that was rasterised: what a document whose
/// page sizes could not be read is laid out as.
fn uniform_sizes(pages: u32, width: u32, height: u32) -> Vec<(f32, f32)> {
    // In points, so the strip is measured in the same unit either way and a
    // text layer arriving later lands on it.
    let height = if width == 0 {
        792.0
    } else {
        612.0 * height as f32 / width as f32
    };
    vec![(612.0, height); pages.max(1) as usize]
}

/// Run one rasteriser and collect its PNG.
fn rasterise(rasteriser: &Rasteriser, document: &[u8], page: u32, width: u32) -> Option<Vec<u8>> {
    let png = run(
        rasteriser.command,
        &(rasteriser.args)(page, width),
        document,
    )?;
    (!png.is_empty()).then_some(png)
}

/// Feed the document to a tool on stdin and collect what it writes.
///
/// Bytes rather than a path throughout: the child never resolves a name, so
/// nothing can be substituted between the worker's stat and its open.
fn run(command: &str, args: &[String], document: &[u8]) -> Option<Vec<u8>> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // A tool's complaints belong in the log, not interleaved with the
        // payload on our own stdout.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Write the document and read the answer concurrently: a document larger
    // than a pipe buffer would otherwise deadlock, each side waiting for the
    // other.
    let mut stdin = child.stdin.take()?;
    let document = document.to_vec();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        // A tool that has seen enough closes its input early; that is a
        // success, not an error.
        let _ = stdin.write_all(&document);
    });

    let mut out = Vec::new();
    let read = child
        .stdout
        .as_mut()
        .and_then(|sink| sink.read_to_end(&mut out).ok());
    // A non-zero exit with usable output still counts: some of these complain
    // about a malformed document and answer anyway.
    let _ = child.wait();
    let _ = writer.join();
    read?;
    Some(out)
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
            key: otto_kit::t_owned!("peek-fact-pages"),
            value: pages.to_string(),
        },
        Fact {
            key: otto_kit::t_owned!("peek-fact-size"),
            value: human_size(size),
        },
    ];
    if let Some(title) = document_title(bytes) {
        facts.insert(
            0,
            Fact {
                key: otto_kit::t_owned!("peek-fact-title"),
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
        subtitle: otto_kit::t_owned!("peek-pdf-install-rasteriser", packages = wanted),
        facts,
        hero: None,
        // Stamped by `decode`, which is where the sniffed type is known.
        icon: Vec::new(),
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

    /// `pdftotext -bbox-layout`, read into words boxed in the strip: the
    /// first page's own points, the second page's shifted down by a page and
    /// a gap.
    #[test]
    fn a_text_layer_lands_in_strip_coordinates() {
        let xml = r#"<html><body><doc>
  <page width="612.000000" height="792.000000">
    <flow><block xMin="72.0" yMin="96.0" xMax="200.0" yMax="110.0">
      <line xMin="72.0" yMin="96.0" xMax="200.0" yMax="110.0">
        <word xMin="72.0" yMin="96.0" xMax="132.0" yMax="110.0">Quarterly</word>
        <word xMin="140.0" yMin="96.0" xMax="200.0" yMax="110.0">report &amp; co</word>
      </line>
    </block></flow>
  </page>
  <page width="612.000000" height="792.000000">
    <flow><block xMin="72.0" yMin="96.0" xMax="200.0" yMax="110.0">
      <line xMin="72.0" yMin="96.0" xMax="200.0" yMax="110.0">
        <word xMin="72.0" yMin="96.0" xMax="132.0" yMax="110.0">Appendix</word>
      </line>
    </block></flow>
  </page>
</doc></body></html>"#;
        let (pages, words) = parse_bbox(xml).expect("a text layer");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].width, 612.0);

        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "Quarterly");
        assert_eq!((words[0].left, words[0].top), (72, 96));
        assert_eq!(words[0].width, 60);
        // Entities are text, not markup.
        assert_eq!(words[1].text, "report & co");
        // The same line of the same block, so a selection over both copies as
        // one line.
        assert_eq!((words[0].line, words[1].line), (1, 1));

        // The second page is a page and a gap further down, and is a block of
        // its own so a selection across the break copies with one.
        let second = otto_kit::preview::page_in_strip(&pages, 1);
        assert_eq!(words[2].top, second.top as u32 + 96);
        assert_eq!(words[2].block, 1);
        assert_ne!(words[0].block, words[2].block);
    }

    /// An attribute is read by its own name, not by a name another one starts
    /// with.
    #[test]
    fn attributes_are_read_whole() {
        assert_eq!(
            attribute(r#"word xMin="12.5" xMax="30.0""#, "xMin"),
            Some(12.5)
        );
        assert_eq!(attribute(r#"word xMax="30.0""#, "xMin"), None);
        assert_eq!(attribute(r#"page width="612.0""#, "width"), Some(612.0));
    }

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
