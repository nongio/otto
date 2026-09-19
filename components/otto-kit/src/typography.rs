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
    /// The colour emoji face, looked up once: `None` until asked, then
    /// `Some(None)` on a system without one.
    emoji: RefCell<Option<Option<skia::Typeface>>>,
    /// Shaper for emoji runs, created on first use.
    shaper: RefCell<Option<skia::Shaper>>,
    /// Shaped emoji runs by text and size, with their advance.
    shaped: RefCell<std::collections::HashMap<(String, u32), std::rc::Rc<Shaped>>>,
}

impl FontCache {
    fn new() -> Self {
        Self {
            font_mgr: FontMgr::new(),
            cache: RefCell::new(std::collections::HashMap::new()),
            emoji: RefCell::new(None),
            shaper: RefCell::new(None),
            shaped: RefCell::new(std::collections::HashMap::new()),
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

    /// The colour emoji face at `size`, if the system has one.
    ///
    /// Asked for by name rather than by character: fontconfig answers a
    /// character query with the first face in the fallback list that has a
    /// glyph for it, and icon fonts claim emoji code points — U+2705 came back
    /// as Font Awesome, drawn as a monochrome pictogram or not at all.
    fn emoji_font(&self, size: f32) -> Option<Font> {
        let typeface = self
            .emoji
            .borrow_mut()
            .get_or_insert_with(|| {
                [
                    "Noto Color Emoji",
                    "Twemoji",
                    "JoyPixels",
                    "Apple Color Emoji",
                    "emoji",
                ]
                .iter()
                .filter_map(|family| {
                    self.font_mgr
                        .match_family_style(family, FontStyle::normal())
                })
                .find(|face| face.unichar_to_glyph(0x1F600) != 0)
                .or_else(|| {
                    self.font_mgr.match_family_style_character(
                        "",
                        FontStyle::normal(),
                        &["und-Zsye"],
                        0x1F600,
                    )
                })
            })
            .clone()?;
        let mut font = Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::AntiAlias);
        Some(font)
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
                emoji: false,
            }];
        }

        let base = font.typeface();
        // Each piece is a byte range, the face that draws it (`None` being the
        // face asked for) and whether it is an emoji sequence.
        let mut pieces: Vec<(usize, usize, Option<Font>, bool)> = Vec::new();
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            let (at, c) = chars[i];
            let next = chars.get(i + 1).map(|&(_, n)| n);

            if is_emoji_start(c, next) {
                if let Some(emoji) = self
                    .emoji_font(font.size())
                    .filter(|e| e.typeface().unichar_to_glyph(c as skia::Unichar) != 0)
                {
                    let end = emoji_sequence_end(&chars, i);
                    let end_byte = chars.get(end).map_or(text.len(), |&(b, _)| b);
                    pieces.push((at, end_byte, Some(emoji), true));
                    i = end;
                    continue;
                }
            }

            let end_byte = next.map_or(text.len(), |_| chars[i + 1].0);
            let covered = base.unichar_to_glyph(c as skia::Unichar) != 0;
            // Whitespace is blank in every face, so it stays in the run it
            // follows rather than cutting one in two — as long as that face
            // has a glyph for it, or it draws as a box of its own.
            let previous = pieces.last().filter(|p| !p.3).map(|p| p.2.clone());
            let wanted = match previous {
                Some(current)
                    if c.is_whitespace()
                        && current.as_ref().is_none_or(|f| {
                            f.typeface().unichar_to_glyph(c as skia::Unichar) != 0
                        }) =>
                {
                    current
                }
                _ if covered => None,
                _ => self.fallback(font, c),
            };
            pieces.push((at, end_byte, wanted, false));
            i += 1;
        }

        let mut runs: Vec<TextRun<'a>> = Vec::new();
        let mut open: Option<(usize, usize, Option<Font>, bool)> = None;
        for piece in pieces {
            match &mut open {
                // Emoji sequences are kept whole but not merged with their
                // neighbours in plain faces; adjacent emoji share one run.
                Some(run) if run.3 == piece.3 && same_face(&run.2, &piece.2) => run.1 = piece.1,
                _ => {
                    if let Some((start, end, face, emoji)) = open.take() {
                        runs.push(TextRun {
                            text: &text[start..end],
                            font: face.unwrap_or_else(|| font.clone()),
                            emoji,
                        });
                    }
                    open = Some(piece);
                }
            }
        }
        if let Some((start, end, face, emoji)) = open {
            runs.push(TextRun {
                text: &text[start..end],
                font: face.unwrap_or_else(|| font.clone()),
                emoji,
            });
        }
        runs
    }

    /// An emoji run shaped into a blob, with its advance.
    ///
    /// Shaped rather than drawn glyph by glyph: joiner sequences, skin tones,
    /// keycaps and flags are ligatures in the emoji face, and without shaping
    /// a family comes out as its members one by one and a flag as two
    /// letters. All in one font run, for the reason the emoji palette gives —
    /// the default iterators split a joiner from the emoji beside it.
    fn shaped(&self, run: &TextRun) -> std::rc::Rc<Shaped> {
        let key = (run.text.to_string(), run.font.size().to_bits());
        if let Some(hit) = self.shaped.borrow().get(&key) {
            return hit.clone();
        }
        let mut shaper = self.shaper.borrow_mut();
        let shaper = shaper.get_or_insert_with(|| skia::Shaper::new(self.font_mgr.clone()));
        let bytes = run.text.len();
        let mut handler = Shaped::default();
        let mut fonts = skia::Shaper::new_trivial_font_run_iterator(&run.font, bytes);
        let mut bidi = skia::shapers::primitive::trivial_bidi_run_iterator(0, bytes);
        let mut script = skia::shapers::primitive::trivial_script_run_iterator(0, bytes);
        let mut language = skia::Shaper::new_trivial_language_run_iterator("und", bytes);
        shaper.shape_with_iterators(
            run.text,
            &mut fonts,
            &mut bidi,
            &mut script,
            &mut language,
            f32::INFINITY,
            &mut handler,
        );
        let shaped = std::rc::Rc::new(handler);
        let mut cache = self.shaped.borrow_mut();
        // Answers are streamed and re-laid out as they grow; keep the cache
        // from growing with them.
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, shaped.clone());
        shaped
    }
}

/// A run shaped into glyphs, positioned along a baseline at y = 0.
#[derive(Default)]
struct Shaped {
    glyphs: Vec<skia::GlyphId>,
    positions: Vec<skia::Point>,
    advance: f32,
    /// Where the run being received starts in `glyphs`.
    start: usize,
}

impl skia::shaper::run_handler::RunHandler for Shaped {
    fn begin_line(&mut self) {}
    fn run_info(&mut self, _: &skia::shaper::run_handler::RunInfo) {}
    fn commit_run_info(&mut self) {}
    fn run_buffer(
        &mut self,
        info: &skia::shaper::run_handler::RunInfo,
    ) -> skia::shaper::run_handler::Buffer<'_> {
        self.start = self.glyphs.len();
        let count = self.start + info.glyph_count;
        self.glyphs.resize(count, 0);
        self.positions.resize(count, skia::Point::default());
        skia::shaper::run_handler::Buffer::new(
            &mut self.glyphs[self.start..],
            &mut self.positions[self.start..],
            skia::Point::new(self.advance, 0.0),
        )
    }
    fn commit_run_buffer(&mut self, info: &skia::shaper::run_handler::RunInfo) {
        self.advance += info.advance.x;
    }
    fn commit_line(&mut self) {}
}

/// Whether `c` begins an emoji sequence: a character drawn as an emoji by
/// default, or any character asking for emoji presentation with U+FE0F.
fn is_emoji_start(c: char, next: Option<char>) -> bool {
    if next == Some('\u{FE0F}') {
        return true;
    }
    if next == Some('\u{FE0E}') {
        return false;
    }
    let u = c as u32;
    matches!(
        u,
        0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x1F004
            | 0x1F0CF
            | 0x1F18E
            | 0x1F191..=0x1F19A
            | 0x1F1E6..=0x1F1FF
            | 0x1F201
            | 0x1F21A
            | 0x1F22F
            | 0x1F232..=0x1F236
            | 0x1F238..=0x1F23A
            | 0x1F250..=0x1F251
            // Pictographs, emoticons, transport, supplemental symbols: the
            // few text-default characters in these blocks are drawn as
            // emoji too, which is what a reader of an answer expects.
            | 0x1F300..=0x1F64F
            | 0x1F680..=0x1F6FF
            | 0x1F7E0..=0x1F7FF
            | 0x1F900..=0x1F9FF
            | 0x1FA70..=0x1FAFF
    )
}

/// The index one past the emoji sequence that starts at `start`: its
/// variation selector, skin tone, keycap mark, tag characters, a second
/// regional indicator for a flag, and anything joined on with U+200D.
fn emoji_sequence_end(chars: &[(usize, char)], start: usize) -> usize {
    let regional = |c: char| (0x1F1E6..=0x1F1FF).contains(&(c as u32));
    let mut i = start + 1;
    if regional(chars[start].1) {
        if chars.get(i).is_some_and(|&(_, c)| regional(c)) {
            i += 1;
        }
        return i;
    }
    while let Some(&(_, c)) = chars.get(i) {
        match c as u32 {
            0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F => i += 1,
            0x200D if i + 1 < chars.len() => i += 2,
            _ => break,
        }
    }
    i
}

impl FontCache {
    /// How far `run` advances once drawn.
    fn advance(&self, run: &TextRun) -> f32 {
        if run.emoji {
            self.shaped(run).advance
        } else {
            run.font.measure_str(run.text, None).0
        }
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
    /// An emoji sequence, drawn shaped in the colour emoji face — see
    /// [`draw_runs`].
    pub emoji: bool,
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
    FONT_CACHE.with(|cache| {
        cache
            .text_runs(font, text)
            .iter()
            .map(|run| cache.advance(run))
            .sum()
    })
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
    FONT_CACHE.with(|cache| {
        let mut x = origin.x;
        for run in cache.text_runs(font, text) {
            if run.emoji {
                let shaped = cache.shaped(&run);
                canvas.draw_glyphs_at(
                    &shaped.glyphs,
                    shaped.positions.as_slice(),
                    (x, origin.y),
                    &run.font,
                    paint,
                );
                x += shaped.advance;
            } else {
                canvas.draw_str(run.text, skia::Point::new(x, origin.y), &run.font, paint);
                x += run.font.measure_str(run.text, None).0;
            }
        }
        x - origin.x
    })
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

/// The character boundary of `text` nearest `x` points from its left edge,
/// drawn in `font`.
///
/// What a click in a line of text means: the caret goes to the boundary the
/// pointer is closest to, so pressing on the left half of a character puts it
/// before that character and on the right half after it. `x` past either end
/// answers that end.
pub fn offset_at(font: &Font, text: &str, x: f32) -> usize {
    if x <= 0.0 {
        return 0;
    }
    let mut best = (0usize, x.abs());
    let mut boundary = |offset: usize, width: f32| {
        let distance = (x - width).abs();
        if distance < best.1 {
            best = (offset, distance);
        }
    };
    for (offset, _) in text.char_indices().skip(1) {
        boundary(offset, measure_runs(font, &text[..offset]));
    }
    boundary(text.len(), measure_runs(font, text));
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a press in a line of text puts the caret: the boundary nearest
    /// it, and the ends when it is past either of them.
    #[test]
    fn a_caret_goes_to_the_boundary_nearest_the_point() {
        let font = get_font_with_fallback("sans-serif", FontStyle::normal(), 16.0);
        let text = "selectable";
        assert_eq!(offset_at(&font, text, -10.0), 0);
        assert_eq!(offset_at(&font, text, 0.0), 0);
        assert_eq!(offset_at(&font, text, 10_000.0), text.len());
        // Either side of the boundary after "select" answers that boundary.
        let boundary = measure_runs(&font, "select");
        assert_eq!(offset_at(&font, text, boundary), 6);
        assert_eq!(offset_at(&font, text, boundary - 1.0), 6);
        assert_eq!(offset_at(&font, text, boundary + 1.0), 6);
        // And a caret inside a character goes to the half it is nearer.
        let next = measure_runs(&font, "selecta");
        assert_eq!(offset_at(&font, text, (boundary + next) / 2.0 + 1.0), 7);
    }

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

    /// Pixels in `text` drawn black on white that carry colour — the ink of
    /// a colour emoji, which black text never has.
    fn coloured_ink(text: &str) -> usize {
        let base = styles::BODY.font();
        let (w, h) = (240, 40);
        let mut surface = skia::surfaces::raster_n32_premul((w, h)).unwrap();
        surface.canvas().clear(skia::Color::WHITE);
        let mut paint = skia::Paint::default();
        paint.set_color(skia::Color::BLACK);
        draw_runs(surface.canvas(), text, (2.0, 28.0), &base, &paint);
        let image = surface.image_snapshot();
        let info = skia::ImageInfo::new(
            (w, h),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        assert!(image.read_pixels(
            &info,
            &mut pixels,
            (w * 4) as usize,
            (0, 0),
            skia::image::CachingHint::Allow
        ));
        pixels
            .chunks(4)
            .filter(|p| {
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                (r - g).abs() > 40 || (g - b).abs() > 40 || (r - b).abs() > 40
            })
            .count()
    }

    fn has_emoji_face() -> bool {
        FONT_CACHE.with(|cache| cache.emoji_font(16.0).is_some())
    }

    /// An agent's answer ending in `done ✅ 🎉` drew the check mark in an icon
    /// font fontconfig offered first for U+2705, and the space after it as a
    /// missing glyph: every emoji must come from the colour emoji face.
    #[test]
    fn emoji_are_drawn_in_colour() {
        if !has_emoji_face() {
            return;
        }
        let base = styles::BODY.font();
        let text = "done \u{2705} \u{1F389}";
        for run in text_runs(&base, text) {
            let glyphs = run.font.str_to_glyphs_vec(run.text);
            assert!(
                !glyphs.contains(&0),
                "{:?} has a missing glyph in {}",
                run.text,
                run.font.typeface().family_name()
            );
        }
        for emoji in ["\u{2705}", "\u{1F389}", "\u{2764}\u{FE0F}"] {
            assert!(
                coloured_ink(emoji) > 20,
                "{emoji:?} must be drawn in the colour emoji face"
            );
        }
        assert_eq!(
            text_runs(&base, text)
                .iter()
                .map(|run| run.text)
                .collect::<String>(),
            text
        );
    }

    /// Joiner sequences and flags are ligatures in the emoji face: drawn
    /// unshaped, a family is three people wide.
    #[test]
    fn emoji_sequences_are_shaped_as_one() {
        if !has_emoji_face() {
            return;
        }
        let base = styles::BODY.font();
        let one = measure_runs(&base, "\u{1F468}");
        let family = measure_runs(&base, "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
        assert!(one > 0.0);
        assert!(
            family < one * 1.5,
            "a family must shape into one glyph: {family} vs {one}"
        );
        let flag = measure_runs(&base, "\u{1F1EE}\u{1F1F9}");
        assert!(flag < one * 1.5, "a flag must shape into one glyph");
        let runs = text_runs(&base, "ok \u{1F44D}\u{1F3FD} ok");
        assert_eq!(runs.iter().filter(|run| run.emoji).count(), 1);
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
