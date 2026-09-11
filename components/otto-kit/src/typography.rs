use skia_safe::{self as skia, Font, FontMgr, FontStyle};
use std::cell::RefCell;

/// Cache key that doesn't rely on FontStyle being Hash/Eq
#[derive(Hash, PartialEq, Eq)]
struct CacheKey {
    family: String,
    weight: i32,
    width: i32,
    slant: u8,
    size_hundredths: i32,
}

impl CacheKey {
    fn from_style(family: &str, style: FontStyle, size: f32) -> Self {
        Self {
            family: family.to_string(),
            weight: *style.weight(),
            width: *style.width(),
            slant: style.slant() as u8,
            size_hundredths: (size * 100.0) as i32,
        }
    }
}

/// Font cache with font manager (not thread-local since FontMgr is not Send)
pub struct FontCache {
    font_mgr: FontMgr,
    cache: RefCell<std::collections::HashMap<CacheKey, Font>>,
}

impl FontCache {
    fn new() -> Self {
        Self {
            font_mgr: FontMgr::new(),
            cache: RefCell::new(std::collections::HashMap::new()),
        }
    }

    /// Get or create a font with caching
    pub fn get_font(&self, family: &str, style: FontStyle, size: f32) -> Option<Font> {
        let key = CacheKey::from_style(family, style, size);

        // Check cache first
        if let Some(font) = self.cache.borrow().get(&key) {
            return Some(font.clone());
        }

        // Create new font
        let typeface = self.font_mgr.match_family_style(family, style)?;
        let typeface = self.covering(typeface, family, style);
        let mut font = Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::SubpixelAntiAlias);

        // Cache it
        self.cache.borrow_mut().insert(key, font.clone());
        Some(font)
    }

    /// This cache's own [`covering_typeface`].
    fn covering(&self, typeface: skia::Typeface, family: &str, style: FontStyle) -> skia::Typeface {
        covering_typeface(&self.font_mgr, typeface, family, style)
    }

    /// The face that draws `c` when [`Self`]'s own does not.
    ///
    /// Keyed by the character rather than by the string, so every label in
    /// one script shares a lookup: asking the font manager is a fontconfig
    /// query, far too slow to repeat per label per frame.
    fn fallback(&self, font: &Font, c: char) -> Option<Font> {
        let typeface = font.typeface();
        let style = typeface.font_style();
        let size = font.size();
        // Namespaced away from real family names, which never start with a
        // replacement character, so a fallback entry cannot collide with the
        // cached font for a family of that name.
        let key = CacheKey::from_style(&format!("\u{FFFD}{c}"), style, size);
        if let Some(hit) = self.cache.borrow().get(&key) {
            return Some(hit.clone());
        }

        let replacement = self.font_mgr.match_family_style_character(
            typeface.family_name(),
            style,
            &[],
            c as skia::Unichar,
        )?;
        let mut found = Font::from_typeface(replacement, size);
        found.set_subpixel(true);
        found.set_edging(skia::font::Edging::SubpixelAntiAlias);
        self.cache.borrow_mut().insert(key, found.clone());
        Some(found)
    }

    /// This cache's own [`text_runs`].
    fn text_runs<'a>(&self, font: &Font, text: &'a str) -> Vec<TextRun<'a>> {
        // Almost every string the interface draws is ASCII, and every face it
        // draws with covers ASCII, so this runs on the way to drawing anything
        // at all: settle the common case with a byte scan rather than a cmap
        // lookup per character.
        if text.is_ascii() || text.is_empty() {
            return vec![TextRun {
                text,
                font: font.clone(),
            }];
        }

        let base = font.typeface();
        let mut runs = Vec::new();
        let mut start = 0;
        let mut current: Option<Font> = None;
        let mut open = false;

        for (i, c) in text.char_indices() {
            let covered = base.unichar_to_glyph(c as skia::Unichar) != 0;
            // Whitespace is blank in every face, so it stays in the run it
            // follows rather than cutting one in two.
            let wanted = if c.is_whitespace() && open {
                current.clone()
            } else if covered {
                None
            } else {
                self.fallback(font, c)
            };

            if open && !same_face(&current, &wanted) {
                runs.push(TextRun {
                    text: &text[start..i],
                    font: current.clone().unwrap_or_else(|| font.clone()),
                });
                start = i;
            }
            current = wanted;
            open = true;
        }

        runs.push(TextRun {
            text: &text[start..],
            font: current.unwrap_or_else(|| font.clone()),
        });
        runs
    }
}

/// Whether two runs would be drawn by the same face — `None` being the face
/// the caller asked for.
fn same_face(a: &Option<Font>, b: &Option<Font>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.typeface().unique_id() == b.typeface().unique_id(),
        _ => false,
    }
}

/// The typeface to actually draw with: the one asked for, or one that can
/// draw the language the interface is in.
///
/// Skia draws a string with exactly one typeface and no per-glyph fallback of
/// its own — a run it has no glyph for comes out as empty boxes, not as the
/// same text in another face the way fontconfig would arrange it. Otto's
/// interface is set in Inter, which carries no CJK, so a desktop in Chinese
/// drew every one of its own strings as boxes.
///
/// The substitution is whole-interface rather than per-run: the language is
/// fixed for the life of the process, and a desktop drawn in one language
/// wants one face for its chrome rather than two disagreeing about weight and
/// metrics halfway along a label. The families this picks (Source Han Sans and
/// its Noto siblings) carry Latin as well, so the Latin that remains — a file
/// name, a version number — stays in a face that matches the rest.
///
/// Shared with the compositor, which keeps its own font cache over the same
/// `skia_safe` and would otherwise draw its dock and its window titles in
/// boxes while every application above it was legible.
pub fn covering_typeface(
    font_mgr: &FontMgr,
    typeface: skia::Typeface,
    family: &str,
    style: FontStyle,
) -> skia::Typeface {
    let Some((bcp47, sample)) = script_sample() else {
        return typeface;
    };
    if typeface.unichar_to_glyph(sample) != 0 {
        return typeface;
    }
    for candidate in script_families(bcp47) {
        if let Some(face) = font_mgr.match_family_style(candidate, style) {
            if face.unichar_to_glyph(sample) != 0 {
                return face;
            }
        }
    }
    font_mgr
        .match_family_style_character(family, style, &[bcp47], sample)
        .unwrap_or(typeface)
}

/// The faces that draw a language's own shapes, most preferred first.
///
/// Han unification gives one code point different correct shapes per
/// language: 直, 骨 and 今 are drawn one way in Japanese and another in
/// Simplified Chinese, and a reader of either notices the wrong one at once.
/// The language belongs in the request, and
/// [`FontMgr::match_family_style_character`] takes one — but on a fontconfig
/// system Skia ignores it, answering `Source Han Sans CN` for `ja` as readily
/// as for `zh`. Naming the regional family is what actually picks the shapes,
/// so it is tried first and the language-tagged search is left as the fallback
/// for a machine that has none of these installed.
fn script_families(bcp47: &str) -> &'static [&'static str] {
    let language = bcp47.split(['-', '_']).next().unwrap_or(bcp47);
    match language {
        "ja" => &["Noto Sans CJK JP", "Source Han Sans JP", "Noto Sans JP"],
        "ko" => &["Noto Sans CJK KR", "Source Han Sans KR", "Noto Sans KR"],
        "zh" => match bcp47 {
            t if t.contains("TW") => &["Noto Sans CJK TC", "Source Han Sans TW"],
            t if t.contains("HK") => &["Noto Sans CJK HK", "Source Han Sans HK"],
            _ => &["Noto Sans CJK SC", "Source Han Sans SC", "Noto Sans SC"],
        },
        _ => &[],
    }
}

impl FontCache {
    /// Get font with fallback to system default
    pub fn get_font_with_fallback(&self, family: &str, style: FontStyle, size: f32) -> Font {
        if let Some(font) = self.get_font(family, style, size) {
            return font;
        }

        // Try common fallback fonts
        for fallback in ["sans-serif", "DejaVu Sans", "Liberation Sans", "Arial"] {
            if let Some(font) = self.get_font(fallback, style, size) {
                eprintln!(
                    "Font '{}' not found, using fallback: '{}'",
                    family, fallback
                );
                return font;
            }
        }

        // Last resort: system default
        eprintln!("Font '{}' and all fallbacks failed, using default", family);
        let typeface = self
            .font_mgr
            .legacy_make_typeface(None, style)
            .expect("Failed to create default typeface");
        let typeface = self.covering(typeface, "sans-serif", style);
        let mut font = Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::SubpixelAntiAlias);
        font
    }
}

/// A character out of the script the interface is currently drawn in, with the
/// language tag to disambiguate it, or `None` when the Latin families the
/// design is built on already cover the language.
///
/// One character is enough: a family that has the Han ideograph has the script.
/// Han is shared between Chinese, Japanese and Korean and drawn differently in
/// each, which is what the tag is for — it decides which regional face the
/// font manager hands back for the same code point.
fn script_sample() -> Option<(&'static str, skia::Unichar)> {
    let locale = crate::i18n::current_locale();
    let language = locale.split('-').next().unwrap_or(&locale).to_string();
    match language.as_str() {
        // U+6F22 漢, the character both Chinese and Japanese name their own
        // script with.
        "zh" | "ja" => Some((leak_locale(locale), 0x6F22)),
        // U+D55C 한, the first syllable of the Korean script's own name.
        "ko" => Some((leak_locale(locale), 0xD55C)),
        _ => None,
    }
}

/// The current locale as a `&'static str`.
///
/// Resolved once and never changed for the life of the process — the language
/// is fixed before the first string is looked up — so one leak per process is
/// the whole cost, and it saves threading an owned tag through the font cache.
fn leak_locale(locale: String) -> &'static str {
    static CURRENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CURRENT.get_or_init(|| locale).as_str()
}

thread_local! {
    static FONT_CACHE: FontCache = FontCache::new();
}

/// Get a font from the thread-local cache
pub fn get_font(family: &str, style: FontStyle, size: f32) -> Option<Font> {
    FONT_CACHE.with(|cache| cache.get_font(family, style, size))
}

/// Get a font with fallback from the thread-local cache
pub fn get_font_with_fallback(family: &str, style: FontStyle, size: f32) -> Font {
    FONT_CACHE.with(|cache| cache.get_font_with_fallback(family, style, size))
}

/// One stretch of a string and the face that draws it.
///
/// Skia draws a string with exactly one typeface and does no per-glyph
/// fallback of its own, so a label the interface font only partly covers has
/// to be split before it is drawn: the parts Inter has stay in Inter, and only
/// the characters it lacks — a dingbat in a window title, a language named in
/// its own script — are handed to a face that has them.
pub struct TextRun<'a> {
    pub text: &'a str,
    pub font: Font,
}

/// Split `text` into runs, each in the face that can draw it.
///
/// The face asked for is kept wherever it has glyphs, so a single uncovered
/// character can no longer move a whole label into whatever font happens to
/// carry that character — a window titled `\u{2749} Notes` drew every letter in
/// the monospace face that owns U+2749, because the substitution used to be
/// per string.
///
/// Runs break kerning across the boundary, which is the price of drawing the
/// text at all; an all-ASCII string, which is nearly every string, is returned
/// as one run without a cmap lookup.
pub fn text_runs<'a>(font: &Font, text: &'a str) -> Vec<TextRun<'a>> {
    FONT_CACHE.with(|cache| cache.text_runs(font, text))
}

/// How wide `text` is once drawn as runs.
///
/// Measuring in the face asked for alone reports the width of missing-glyph
/// boxes for anything it lacks, which is not the width that reaches the
/// screen.
pub fn measure_runs(font: &Font, text: &str) -> f32 {
    text_runs(font, text)
        .iter()
        .map(|run| run.font.measure_str(run.text, None).0)
        .sum()
}

/// Draw `text` with its baseline starting at `origin`, run by run, and return
/// the total advance.
pub fn draw_runs(
    canvas: &skia::Canvas,
    text: &str,
    origin: impl Into<skia::Point>,
    font: &Font,
    paint: &skia::Paint,
) -> f32 {
    let origin = origin.into();
    let mut x = origin.x;
    for run in text_runs(font, text) {
        canvas.draw_str(run.text, skia::Point::new(x, origin.y), &run.font, paint);
        x += run.font.measure_str(run.text, None).0;
    }
    x - origin.x
}

/// Predefined text styles for a consistent design system
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub family: &'static str,
    pub weight: i32,
    pub size: f32,
}

impl TextStyle {
    /// Create a Skia Font from this text style with proper antialiasing.
    pub fn font(&self) -> Font {
        self.font_scaled(1.0)
    }

    /// Create a Skia Font scaled by the given factor (e.g. output scale).
    pub fn font_scaled(&self, scale: f32) -> Font {
        use skia::font_style::{Slant, Weight, Width};
        let weight = Weight::from(self.weight);
        let style = FontStyle::new(weight, Width::NORMAL, Slant::Upright);
        let mut font = get_font_with_fallback(self.family, style, self.size * scale);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::SubpixelAntiAlias);
        font
    }
}

/// Design system typography scale (based on macOS HIG)
/// Truncate `text` with a trailing ellipsis so it measures no wider than
/// `max_width` under `font`.
///
/// Text that runs past its column does not merely look wrong — it draws over
/// whatever is beside it, so anything laid out in a fixed width should come
/// through here rather than trusting the content to be short.
///
/// Binary search on the character count: a long name is measured a handful of
/// times instead of once per character, and measuring is the expensive part.
pub fn ellipsize(font: &Font, text: &str, max_width: f32) -> String {
    if font.measure_str(text, None).0 <= max_width {
        return text.to_string();
    }
    const ELLIPSIS: &str = "\u{2026}";
    let budget = (max_width - font.measure_str(ELLIPSIS, None).0).max(0.0);

    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect();
        if font.measure_str(&candidate, None).0 <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{}{ELLIPSIS}", chars[..lo].iter().collect::<String>())
}

pub mod styles {
    use super::*;

    /// Large Title - Window titles, primary headings (26pt)
    pub const LARGE_TITLE: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 26.0,
    };

    /// Large Title Emphasized - Bold variant (26pt)
    pub const LARGE_TITLE_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 26.0,
    };

    /// Title 1 - Section headers (22pt)
    pub const TITLE_1: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 22.0,
    };

    /// Title 1 Emphasized - Bold variant (22pt)
    pub const TITLE_1_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 22.0,
    };

    /// Title 2 - Subsection headers (17pt)
    pub const TITLE_2: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 17.0,
    };

    /// Title 2 Emphasized - Bold variant (17pt)
    pub const TITLE_2_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 17.0,
    };

    /// Title 3 - Tertiary headers (15pt)
    pub const TITLE_3: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 15.0,
    };

    /// Title 3 Emphasized - Semibold variant (15pt)
    pub const TITLE_3_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 15.0,
    };

    /// Headline - List headers, group labels (13pt)
    pub const HEADLINE: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 13.0,
    };

    /// Headline Emphasized - Heavy variant (13pt)
    pub const HEADLINE_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 800, // Heavy
        size: 13.0,
    };

    /// Body - Default text, paragraphs (13pt)
    pub const BODY: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 13.0,
    };

    /// Body Medium - Medium weight variant (13pt)
    pub const BODY_MEDIUM: TextStyle = TextStyle {
        family: "Inter",
        weight: 500, // Medium
        size: 13.0,
    };

    /// Titlebar title - Semibold, one step between body and title 3 (14pt).
    /// The floating window bar's type: 13pt read small on a 34pt bar and
    /// 15pt loud, so the bar sits between them.
    pub const TITLEBAR: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 14.0,
    };

    /// Body Emphasized - Semibold variant (13pt)
    pub const BODY_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 13.0,
    };

    /// Callout - Highlighted text, tooltips (12pt)
    pub const CALLOUT: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 12.0,
    };

    /// Callout Emphasized - Semibold variant (12pt)
    pub const CALLOUT_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 12.0,
    };

    /// Subheadline - Secondary labels (11pt)
    pub const SUBHEADLINE: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 11.0,
    };

    /// Subheadline Emphasized - Semibold variant (11pt)
    pub const SUBHEADLINE_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 11.0,
    };

    /// Footnote - Helper text, status text (10pt)
    pub const FOOTNOTE: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 10.0,
    };

    /// Footnote Emphasized - Semibold variant (10pt)
    pub const FOOTNOTE_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 10.0,
    };

    /// Caption 1 - Metadata, timestamps (10pt)
    pub const CAPTION_1: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Regular
        size: 10.0,
    };

    /// Caption 1 Emphasized - Medium variant (10pt)
    pub const CAPTION_1_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 500, // Medium
        size: 10.0,
    };

    /// Caption 2 - Fine print (10pt)
    pub const CAPTION_2: TextStyle = TextStyle {
        family: "Inter",
        weight: 500, // Medium
        size: 10.0,
    };

    /// Caption 2 Emphasized - Semibold variant (10pt)
    pub const CAPTION_2_EMPHASIZED: TextStyle = TextStyle {
        family: "Inter",
        weight: 600, // Semibold
        size: 10.0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_cache() {
        let font1 = get_font_with_fallback("sans-serif", FontStyle::normal(), 16.0);
        let font2 = get_font_with_fallback("sans-serif", FontStyle::normal(), 16.0);

        // Should be same instance from cache
        assert_eq!(font1.typeface().unique_id(), font2.typeface().unique_id());
    }

    #[test]
    fn test_text_styles() {
        let _title = styles::TITLE_1.font();
        let _body = styles::BODY.font();
        let _caption = styles::CAPTION_1.font();
        // If we get here without panic, fonts loaded successfully
    }

    #[test]
    fn ascii_keeps_the_face_it_was_given() {
        let base = styles::BODY.font();
        let runs = text_runs(&base, "English (United Kingdom)");
        assert_eq!(runs.len(), 1, "an ASCII label must be drawn in one face");
        assert_eq!(
            base.typeface().unique_id(),
            runs[0].font.typeface().unique_id(),
            "an ASCII label must not be moved off the interface font"
        );
    }

    /// A window titled `\u{2749} Notes` used to be drawn entirely in whatever
    /// face owns U+2749 — a monospace one, here — because the substitution was
    /// per string. Only the character without a glyph may move.
    #[test]
    fn one_uncovered_character_does_not_take_the_whole_label_with_it() {
        let base = styles::BODY.font();
        let text = "\u{2749} Window decorations font";
        if base.typeface().unichar_to_glyph(0x2749) != 0 {
            // An interface font that has the dingbat has nothing to
            // substitute, and the split is not what is under test.
            return;
        }
        let runs = text_runs(&base, text);
        let latin: String = runs
            .iter()
            .filter(|run| run.font.typeface().unique_id() == base.typeface().unique_id())
            .map(|run| run.text)
            .collect();
        assert!(
            latin.contains("Window decorations font"),
            "the words must stay in the interface font, drawn instead in {:?}",
            runs.iter()
                .map(|r| r.font.typeface().family_name())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            runs.iter().map(|run| run.text).collect::<String>(),
            text,
            "the runs must reassemble into the string that was asked for"
        );
    }

    #[test]
    fn a_script_the_interface_font_lacks_finds_a_face_that_has_it() {
        let base = styles::BODY.font();
        // The language picker names every language in its own script, so this
        // is drawn by an interface that is itself in English.
        let text = "\u{4E2D}\u{6587}";
        let missing = text
            .chars()
            .any(|c| base.typeface().unichar_to_glyph(c as skia::Unichar) == 0);
        if !missing {
            // A system whose interface font already covers CJK has nothing to
            // substitute, and the fallback is not what is under test.
            return;
        }
        let first = text.chars().next().unwrap() as skia::Unichar;
        if FontMgr::new()
            .match_family_style_character("", FontStyle::default(), &[], first)
            .is_none()
        {
            // A machine with no CJK face installed at all — a bare CI runner,
            // say — has nothing to substitute either. What is under test is
            // the substitution, not the host's font set.
            return;
        }
        for run in text_runs(&base, text) {
            assert!(
                run.text
                    .chars()
                    .all(|c| run.font.typeface().unichar_to_glyph(c as skia::Unichar) != 0),
                "every character must have a glyph, or the label still draws as boxes"
            );
        }
        assert!(
            measure_runs(&base, text) > 0.0,
            "the substituted face must give the text a width"
        );
    }

    /// A language names the regional face that draws its own Han shapes: the
    /// language tag alone does not survive Skia's fontconfig manager.
    #[test]
    fn a_language_asks_for_its_own_han_shapes() {
        assert_eq!(script_families("ja").first(), Some(&"Noto Sans CJK JP"));
        assert_eq!(script_families("ko-KR").first(), Some(&"Noto Sans CJK KR"));
        assert_eq!(script_families("zh-CN").first(), Some(&"Noto Sans CJK SC"));
        assert_eq!(script_families("zh-TW").first(), Some(&"Noto Sans CJK TC"));
        assert_eq!(script_families("zh-HK").first(), Some(&"Noto Sans CJK HK"));
        // A language whose script the interface font already covers asks for
        // nothing, and never reaches this list.
        assert!(script_families("de").is_empty());
    }
}
