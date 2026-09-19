//! Recognising text in a picture with an external recogniser.
//!
//! The recogniser is exec'd inside the decode worker, through the same door
//! the PDF rasterisers use: pixels go down its standard input as a PNG, and
//! an **hOCR** document comes back on standard output. hOCR is the format
//! the OCR world already agrees on — tesseract, kraken, ocropus and the
//! wrappers around the neural engines all write it — so the engine is a
//! setting rather than a dependency. Nothing is linked; a machine without a
//! recogniser previews pictures without words.
//!
//! The default is tesseract. The parent decides the languages (it knows the
//! locale and can look at which packs are installed) and passes them as the
//! worker's `--languages` argument; a custom command gets them in the
//! `{languages}` placeholder and the `OCR_LANGUAGES` variable.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::decode::on_path;
use crate::payload::Pixels;
use otto_kit::preview::Word;

/// The recogniser used when no setting names one: tesseract, reading the
/// picture from stdin and writing hOCR to stdout.
pub const DEFAULT_COMMAND: &str = "tesseract stdin stdout -l {languages} --psm 3 hocr";

/// Words the recogniser is less sure of than this are dropped before they
/// leave the worker. Real text scores in the eighties and nineties; texture
/// and noise score far lower, and a selection box over noise is worse than
/// none.
const MIN_CONFIDENCE: f32 = 60.0;

/// The most words a picture may carry, matched by the wire format's bound.
const MAX_WORDS: usize = crate::payload::MAX_WORDS;

/// How long the recogniser is given to answer. Shorter than the parent's own
/// deadline for a recognising decode, so an engine that has wedged is killed
/// here rather than left running when the parent gives up on the worker: the
/// recogniser is the worker's child, and killing the worker does not reach
/// it.
const DEADLINE: Duration = Duration::from_secs(20);

/// The most hOCR a recogniser may write. A document past this is a recogniser
/// that has lost its place; the words in it are beyond `MAX_WORDS` anyway, and
/// reading it to the end would only turn a bad engine into a dead worker.
const MAX_HOCR: u64 = 16 * 1024 * 1024;

/// Pictures larger than this are not recognised at all. The preview decode
/// already caps at a few megapixels; this is a guard against the recogniser
/// being handed something it would chew on past every deadline.
const MAX_PIXELS: u64 = 16_000_000;

/// The recogniser to run: `configured`, or [`DEFAULT_COMMAND`] when it names
/// nothing. The one spelling of the question, so the host and the worker
/// cannot disagree about which engine ran.
pub fn command_or_default(configured: &str) -> &str {
    let configured = configured.trim();
    if configured.is_empty() {
        DEFAULT_COMMAND
    } else {
        configured
    }
}

/// Whether the recogniser `command` names is installed. Cheap: a `PATH`
/// walk, no exec. An absolute path is checked as it is.
pub fn available(command: &str) -> bool {
    match command.split_whitespace().next() {
        Some(program) if program.contains('/') => std::path::Path::new(program).is_file(),
        Some(program) => on_path(program),
        None => false,
    }
}

/// Recognise the text in `pixels` with `command`. Boxes are in the
/// coordinates of `pixels`. Empty when nothing was recognised, the recogniser
/// is missing, or it failed.
pub fn recognise(pixels: &Pixels, command: &str, languages: &str) -> Vec<Word> {
    if pixels.width == 0 || pixels.height == 0 {
        return Vec::new();
    }
    if u64::from(pixels.width) * u64::from(pixels.height) > MAX_PIXELS {
        return Vec::new();
    }
    let Some(png) = encode_png(pixels) else {
        return Vec::new();
    };
    let languages = if languages.is_empty() {
        "eng"
    } else {
        languages
    };
    let Some(hocr) = run(&png, command, languages) else {
        return Vec::new();
    };
    parse_hocr(&hocr)
}

fn encode_png(pixels: &Pixels) -> Option<Vec<u8>> {
    let image = pixels.to_image()?;
    let data = image.encode(None, skia_safe::EncodedImageFormat::PNG, None)?;
    Some(data.as_bytes().to_vec())
}

/// Exec the recogniser on a PNG and collect its hOCR.
fn run(png: &[u8], command: &str, languages: &str) -> Option<String> {
    if !available(command) {
        return None;
    }
    let mut words = command
        .split_whitespace()
        .map(|word| word.replace("{languages}", languages));
    let program = words.next()?;
    let mut child = Command::new(program)
        .args(words)
        .env("OCR_LANGUAGES", languages)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Its progress chatter and language complaints are not the payload.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Write and read concurrently: a PNG larger than a pipe buffer would
    // otherwise deadlock, each side waiting for the other.
    let mut stdin = child.stdin.take()?;
    let png = png.to_vec();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&png);
    });

    // Read on a thread too, so the deadline is enforceable: reading inline
    // would block in `read_to_string` with no way to notice time passing.
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let result = stdout
            .by_ref()
            .take(MAX_HOCR)
            .read_to_string(&mut out)
            .map(|_| out);
        let _ = sender.send(result);
    });

    let hocr = match receiver.recv_timeout(DEADLINE) {
        Ok(Ok(out)) => Some(out),
        Ok(Err(_)) => None,
        // Overran. The picture is treated as having no text, and the engine
        // is reaped here so it does not outlive the preview that wanted it.
        Err(_) => {
            let _ = child.kill();
            None
        }
    };
    let _ = child.wait();
    // Both threads end once the child's pipes are closed, which killing it
    // does.
    let _ = writer.join();
    let _ = reader.join();
    hocr
}

/// Words out of an hOCR document: every `ocrx_word`, in document order,
/// which is reading order. Blocks, paragraphs and lines are counted from the
/// `ocr_carea`, `ocr_par` and `ocr_line` elements the word sits in; a
/// recogniser that emits none of them puts everything on one line.
///
/// A hand-rolled scan rather than an HTML parser: hOCR is a small, regular
/// dialect, and only the class, the `title` and the text of a few elements
/// matter. Anything the scan does not understand is skipped, never fatal.
pub fn parse_hocr(hocr: &str) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    let (mut block, mut paragraph, mut line) = (0u32, 0u32, 0u32);
    let mut at = 0;
    while let Some(open) = hocr[at..].find('<') {
        let start = at + open;
        let Some(close) = hocr[start..].find('>') else {
            break;
        };
        let end = start + close + 1;
        let tag = &hocr[start + 1..end - 1];
        at = end;
        if tag.starts_with('/') || tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let class = attribute(tag, "class").unwrap_or_default();
        if has_class(&class, "ocr_carea") {
            block += 1;
        } else if has_class(&class, "ocr_par") {
            paragraph += 1;
        } else if has_class(&class, "ocr_line")
            || has_class(&class, "ocr_textfloat")
            || has_class(&class, "ocr_header")
            || has_class(&class, "ocr_caption")
        {
            line += 1;
        } else if has_class(&class, "ocrx_word") {
            let title = attribute(tag, "title").unwrap_or_default();
            // The text runs to the element's close; inner markup such as
            // <strong> is dropped and entities are decoded.
            let Some(close) = hocr[at..].find("</span>") else {
                break;
            };
            let inner = &hocr[at..at + close];
            at += close + "</span>".len();
            let text = decode_entities(&strip_tags(inner));
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let Some((left, top, right, bottom)) = bbox(&title) else {
                continue;
            };
            if right <= left || bottom <= top {
                continue;
            }
            let confidence = property(&title, "x_wconf")
                .and_then(|value| value.trim().parse::<f32>().ok())
                // A recogniser that reports no confidence is trusted; the
                // floor is for the ones that tell us they are guessing.
                .unwrap_or(100.0);
            if confidence < MIN_CONFIDENCE {
                continue;
            }
            words.push(Word {
                text: text.to_string(),
                left,
                top,
                width: right - left,
                height: bottom - top,
                confidence: confidence.clamp(0.0, 100.0) as u8,
                block: block.max(1),
                paragraph: paragraph.max(1),
                line: line.max(1),
            });
        }
    }
    if words.len() > MAX_WORDS {
        // Keep the most confident, then put them back in reading order.
        let mut ranked: Vec<(usize, u8)> = words
            .iter()
            .enumerate()
            .map(|(index, word)| (index, word.confidence))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(MAX_WORDS);
        ranked.sort_by_key(|(index, _)| *index);
        words = ranked
            .into_iter()
            .map(|(index, _)| words[index].clone())
            .collect();
    }
    words
}

/// An attribute's value out of a tag's text, quoted either way.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(found) = rest.find(name) {
        let after = &rest[found + name.len()..];
        let boundary = found == 0 || !rest.as_bytes()[found - 1].is_ascii_alphanumeric();
        let after = after.trim_start();
        if boundary && after.starts_with('=') {
            let value = after[1..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let value = &value[1..];
                let close = value.find(quote)?;
                return Some(value[..close].to_string());
            }
            let close = value.find(char::is_whitespace).unwrap_or(value.len());
            return Some(value[..close].to_string());
        }
        rest = &rest[found + name.len()..];
    }
    None
}

fn has_class(classes: &str, class: &str) -> bool {
    classes.split_whitespace().any(|c| c == class)
}

/// A property out of an hOCR `title`: `bbox 1 2 3 4; x_wconf 92`.
fn property<'a>(title: &'a str, name: &str) -> Option<&'a str> {
    title.split(';').map(str::trim).find_map(|item| {
        item.strip_prefix(name)
            .filter(|rest| rest.starts_with(char::is_whitespace))
    })
}

fn bbox(title: &str) -> Option<(u32, u32, u32, u32)> {
    let mut numbers = property(title, "bbox")?
        .split_whitespace()
        .map(|n| n.parse::<u32>().ok());
    Some((
        numbers.next()??,
        numbers.next()??,
        numbers.next()??,
        numbers.next()??,
    ))
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// The entities hOCR writers actually emit, plus numeric references.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|semi| *semi <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The recogniser's `-l` argument for this machine: the locale's language
/// plus English, keeping only those with an installed pack. `eng` when
/// nothing is installed, so the recogniser at least says so itself.
pub fn languages() -> String {
    let locale = otto_kit::i18n::current_locale();
    let mut wanted: Vec<&str> = Vec::new();
    if let Some(code) = tesseract_code(&locale) {
        wanted.push(code);
    }
    if !wanted.contains(&"eng") {
        wanted.push("eng");
    }
    let installed: Vec<&str> = wanted
        .into_iter()
        .filter(|code| pack_installed(code))
        .collect();
    if installed.is_empty() {
        "eng".to_string()
    } else {
        installed.join("+")
    }
}

/// Tesseract's name for a locale's language, from its BCP 47 tag.
fn tesseract_code(locale: &str) -> Option<&'static str> {
    let language = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let region = locale
        .split(['-', '_'])
        .nth(1)
        .unwrap_or("")
        .to_ascii_uppercase();
    Some(match language.as_str() {
        "en" => "eng",
        "de" => "deu",
        "it" => "ita",
        "es" => "spa",
        "fr" => "fra",
        "pt" => "por",
        "ru" => "rus",
        "uk" => "ukr",
        "pl" => "pol",
        "ja" => "jpn",
        "zh" if region == "TW" || region == "HK" => "chi_tra",
        "zh" => "chi_sim",
        "nl" => "nld",
        "sv" => "swe",
        "da" => "dan",
        "fi" => "fin",
        "nb" | "no" | "nn" => "nor",
        "cs" => "ces",
        "tr" => "tur",
        "el" => "ell",
        "hu" => "hun",
        "ro" => "ron",
        "ko" => "kor",
        "ar" => "ara",
        _ => return None,
    })
}

/// Where language packs live: `TESSDATA_PREFIX` when set, else the places
/// distributions put them.
fn tessdata_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(prefix) = std::env::var_os("TESSDATA_PREFIX") {
        dirs.push(PathBuf::from(prefix));
    }
    dirs.push(PathBuf::from("/usr/share/tessdata"));
    dirs.push(PathBuf::from("/usr/share/tesseract-ocr/5/tessdata"));
    dirs.push(PathBuf::from("/usr/share/tesseract-ocr/4.00/tessdata"));
    dirs.push(PathBuf::from("/usr/local/share/tessdata"));
    dirs
}

fn pack_installed(code: &str) -> bool {
    tessdata_dirs()
        .into_iter()
        .any(|dir| dir.join(format!("{code}.traineddata")).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<html><body>
  <div class='ocr_page' id='page_1' title='image "hello.png"; bbox 0 0 400 120; ppageno 0'>
   <div class='ocr_carea' id='block_1_1' title="bbox 21 41 157 61">
    <p class='ocr_par' id='par_1_1' lang='eng' title="bbox 21 41 157 61">
     <span class='ocr_line' id='line_1_1' title="bbox 21 41 157 61; baseline 0 -1">
      <span class='ocrx_word' id='word_1_1' title='bbox 21 41 75 60; x_wconf 89'>Hello</span>
      <span class='ocrx_word' id='word_1_2' title='bbox 82 44 127 60; x_wconf 92'><strong>Otto</strong></span>
      <span class='ocrx_word' id='word_1_3' title='bbox 134 44 157 61; x_wconf 96'>42</span>
     </span>
     <span class='ocr_line' id='line_1_2' title="bbox 21 80 100 98">
      <span class='ocrx_word' id='word_1_4' title='bbox 21 80 41 90; x_wconf 12'>~~</span>
      <span class='ocrx_word' id='word_1_5' title='bbox 50 80 70 90; x_wconf 95'>   </span>
      <span class='ocrx_word' id='word_1_6' title='bbox 72 80 100 98; x_wconf 91'>R&amp;D&#39;s</span>
     </span>
    </p>
   </div>
  </div>
</body></html>"#;

    #[test]
    fn hocr_yields_confident_words_in_reading_order() {
        let words = parse_hocr(SAMPLE);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hello", "Otto", "42", "R&D's"]);
        assert_eq!(
            (words[1].left, words[1].top, words[1].width, words[1].height),
            (82, 44, 45, 16)
        );
        assert_eq!(words[1].confidence, 92);
        assert_eq!(
            (words[2].block, words[2].paragraph, words[2].line),
            (1, 1, 1)
        );
        assert_eq!(words[3].line, 2);
    }

    #[test]
    fn a_bare_word_list_is_still_words() {
        // A minimal writer: words only, no areas, paragraphs or lines.
        let words = parse_hocr(
            "<span class=\"ocrx_word\" title=\"bbox 0 0 10 10\">a</span><span class=\"ocrx_word\" title=\"bbox 12 0 20 10\">b</span>",
        );
        assert_eq!(words.len(), 2);
        assert_eq!(
            (words[0].block, words[0].paragraph, words[0].line),
            (1, 1, 1)
        );
        assert_eq!(words[0].confidence, 100);
    }

    #[test]
    fn a_command_is_available_by_its_program() {
        assert!(available("sh -c true"));
        assert!(available("/bin/sh"));
        assert!(!available("no-such-recogniser-anywhere {languages}"));
        assert!(!available(""));
    }

    #[test]
    fn locales_map_to_pack_names() {
        assert_eq!(tesseract_code("it-IT"), Some("ita"));
        assert_eq!(tesseract_code("en_GB"), Some("eng"));
        assert_eq!(tesseract_code("zh-TW"), Some("chi_tra"));
        assert_eq!(tesseract_code("zh-CN"), Some("chi_sim"));
        assert_eq!(tesseract_code("xx"), None);
    }
}
