//! How glyphs are rasterised: the user's fontconfig rendering preferences.
//!
//! Skia's fontconfig font manager resolves *which* face a family names, but
//! not *how* it is drawn: `antialias`, `hinting`, `hintstyle`, `autohint` and
//! `rgba` from `fonts.conf` are the client's to apply, the way Pango and
//! Chromium apply them. This module asks fontconfig for them, per family, and
//! applies them to a [`Font`] or a paragraph [`TextStyle`], so every piece of
//! text Otto draws, chrome and apps alike, follows the same settings as the
//! GTK and Qt applications next to it.
//!
//! The answer is cached per family for the life of the process: a font is
//! cached with its settings, so a change to `fonts.conf` applies from the
//! next login.
//!
//! Subpixel antialiasing is requested whenever fontconfig does not rule it
//! out, and the surface decides: Skia draws per-channel coverage only into a
//! surface that declares a pixel geometry, and Otto declares one only where
//! the layout is known and the result is shown 1:1 (see `wl_output`'s
//! subpixel and the compositor's `surface_props`). Elsewhere the same font
//! draws grayscale. `lcdfilter` is not applied: Skia filters LCD glyphs with
//! its own default and offers no per-font control.

use std::{
    collections::HashMap,
    ffi::{c_char, c_int, c_void, CString},
    sync::{Mutex, OnceLock},
};

use skia_safe::{font::Edging, textlayout::TextStyle, Font, FontHinting};

/// fontconfig's `rgba`: the subpixel order its user configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontconfigRgba {
    /// Red, green, blue, left to right.
    Rgb,
    /// Blue, green, red, left to right.
    Bgr,
    /// Red, green, blue, top to bottom.
    Vrgb,
    /// Blue, green, red, top to bottom.
    Vbgr,
    /// Subpixel rendering turned off.
    None,
}

/// How text in one family is rasterised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRendering {
    pub edging: Edging,
    pub hinting: FontHinting,
    /// Glyphs placed at fractional positions. Only with no or slight
    /// hinting: medium and full hinting fit horizontal stems to the pixel
    /// grid, which a fractional origin would undo.
    pub subpixel_positioning: bool,
    /// FreeType's autohinter in place of the font's own instructions.
    pub force_auto_hinting: bool,
    /// The subpixel order fontconfig names, if it names one.
    pub rgba: Option<FontconfigRgba>,
}

impl Default for TextRendering {
    /// What fontconfig answers with no configuration of its own beyond the
    /// usual distribution default (`10-hinting-slight.conf`).
    fn default() -> Self {
        Self::from_fontconfig(&Values::default())
    }
}

impl TextRendering {
    /// The settings for `family`, from fontconfig.
    pub fn for_family(family: &str) -> Self {
        static CACHE: OnceLock<Mutex<HashMap<String, TextRendering>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        *cache.entry(family.to_string()).or_insert_with(|| {
            query(family).map_or_else(Self::default, |v| Self::from_fontconfig(&v))
        })
    }

    /// The settings fontconfig applies to text with no family named, which
    /// is where a desktop-wide `rgba` is read from.
    pub fn system() -> Self {
        Self::for_family("")
    }

    /// Applies these settings to `font`.
    pub fn apply_to_font(&self, font: &mut Font) {
        font.set_edging(self.edging);
        font.set_hinting(self.hinting);
        font.set_subpixel(self.subpixel_positioning);
        font.set_force_auto_hinting(self.force_auto_hinting);
    }

    /// Applies these settings to a paragraph's `style`.
    ///
    /// SkParagraph builds its own fonts from the style, defaulting to
    /// grayscale with slight hinting whatever the user configured.
    pub fn apply_to_text_style(&self, style: &mut TextStyle) {
        style.set_font_edging(self.edging);
        style.set_font_hinting(self.hinting);
        style.set_subpixel(self.subpixel_positioning);
    }

    fn from_fontconfig(values: &Values) -> Self {
        let rgba = match values.rgba {
            Some(FC_RGBA_RGB) => Some(FontconfigRgba::Rgb),
            Some(FC_RGBA_BGR) => Some(FontconfigRgba::Bgr),
            Some(FC_RGBA_VRGB) => Some(FontconfigRgba::Vrgb),
            Some(FC_RGBA_VBGR) => Some(FontconfigRgba::Vbgr),
            Some(FC_RGBA_NONE) => Some(FontconfigRgba::None),
            _ => None,
        };
        let hinting = if !values.hinting.unwrap_or(true) {
            FontHinting::None
        } else {
            match values.hint_style.unwrap_or(FC_HINT_SLIGHT) {
                FC_HINT_NONE => FontHinting::None,
                FC_HINT_SLIGHT => FontHinting::Slight,
                FC_HINT_MEDIUM => FontHinting::Normal,
                _ => FontHinting::Full,
            }
        };
        let edging = if !values.antialias.unwrap_or(true) {
            Edging::Alias
        } else if rgba == Some(FontconfigRgba::None) {
            Edging::AntiAlias
        } else {
            Edging::SubpixelAntiAlias
        };
        Self {
            edging,
            hinting,
            subpixel_positioning: matches!(hinting, FontHinting::None | FontHinting::Slight),
            force_auto_hinting: values.autohint.unwrap_or(false),
            rgba,
        }
    }
}

/// The rendering properties fontconfig reported, `None` where it set none.
#[derive(Debug, Default)]
struct Values {
    antialias: Option<bool>,
    hinting: Option<bool>,
    hint_style: Option<c_int>,
    autohint: Option<bool>,
    rgba: Option<c_int>,
}

const FC_HINT_NONE: c_int = 0;
const FC_HINT_SLIGHT: c_int = 1;
const FC_HINT_MEDIUM: c_int = 2;
const FC_RGBA_RGB: c_int = 1;
const FC_RGBA_BGR: c_int = 2;
const FC_RGBA_VRGB: c_int = 3;
const FC_RGBA_VBGR: c_int = 4;
const FC_RGBA_NONE: c_int = 5;
const FC_RESULT_MATCH: c_int = 0;
const FC_MATCH_PATTERN: c_int = 0;

// libfontconfig is already linked: Skia's font manager on Linux is the
// fontconfig one. These are the few calls a rendering query needs.
#[link(name = "fontconfig")]
extern "C" {
    fn FcPatternCreate() -> *mut c_void;
    fn FcPatternDestroy(pattern: *mut c_void);
    fn FcPatternAddString(pattern: *mut c_void, object: *const c_char, value: *const u8) -> c_int;
    fn FcConfigSubstitute(config: *mut c_void, pattern: *mut c_void, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(pattern: *mut c_void);
    fn FcFontMatch(config: *mut c_void, pattern: *mut c_void, result: *mut c_int) -> *mut c_void;
    fn FcPatternGetBool(
        pattern: *const c_void,
        object: *const c_char,
        n: c_int,
        value: *mut c_int,
    ) -> c_int;
    fn FcPatternGetInteger(
        pattern: *const c_void,
        object: *const c_char,
        n: c_int,
        value: *mut c_int,
    ) -> c_int;
}

/// Asks fontconfig how `family` is rendered, as it would for a client
/// matching that family: the pattern is substituted, then matched, so rules
/// targeting the pattern and rules targeting the matched font both apply.
fn query(family: &str) -> Option<Values> {
    let family = CString::new(family).ok()?;
    // SAFETY: every pointer passed is either one fontconfig returned and has
    // not destroyed yet, or a NUL-terminated string that outlives the call; a
    // null config means the current one. Both patterns are destroyed once.
    unsafe {
        let pattern = FcPatternCreate();
        if pattern.is_null() {
            return None;
        }
        if !family.as_bytes().is_empty() {
            FcPatternAddString(pattern, c"family".as_ptr(), family.as_ptr().cast());
        }
        FcConfigSubstitute(std::ptr::null_mut(), pattern, FC_MATCH_PATTERN);
        FcDefaultSubstitute(pattern);
        let mut result = 0;
        let matched = FcFontMatch(std::ptr::null_mut(), pattern, &mut result);
        let source = if matched.is_null() { pattern } else { matched };

        let get_bool = |object: &std::ffi::CStr| {
            let mut value = 0;
            (FcPatternGetBool(source, object.as_ptr(), 0, &mut value) == FC_RESULT_MATCH)
                .then_some(value != 0)
        };
        let get_int = |object: &std::ffi::CStr| {
            let mut value = 0;
            (FcPatternGetInteger(source, object.as_ptr(), 0, &mut value) == FC_RESULT_MATCH)
                .then_some(value)
        };
        let values = Values {
            antialias: get_bool(c"antialias"),
            hinting: get_bool(c"hinting"),
            hint_style: get_int(c"hintstyle"),
            autohint: get_bool(c"autohint"),
            rgba: get_int(c"rgba"),
        };

        if !matched.is_null() {
            FcPatternDestroy(matched);
        }
        FcPatternDestroy(pattern);
        Some(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's fontconfig answer, not a hardcoded rendering, is what text
    /// is drawn with.
    #[test]
    fn follows_fontconfig_preferences() {
        let cases = [
            // Nothing configured: slight hinting, subpixel positioning,
            // subpixel AA left to the surface.
            (
                Values::default(),
                Edging::SubpixelAntiAlias,
                FontHinting::Slight,
                true,
            ),
            (
                Values {
                    hint_style: Some(FC_HINT_NONE),
                    ..Values::default()
                },
                Edging::SubpixelAntiAlias,
                FontHinting::None,
                true,
            ),
            (
                Values {
                    hint_style: Some(FC_HINT_MEDIUM),
                    ..Values::default()
                },
                Edging::SubpixelAntiAlias,
                FontHinting::Normal,
                false,
            ),
            (
                Values {
                    hint_style: Some(3),
                    ..Values::default()
                },
                Edging::SubpixelAntiAlias,
                FontHinting::Full,
                false,
            ),
            // `hinting = false` overrides any hintstyle.
            (
                Values {
                    hinting: Some(false),
                    hint_style: Some(3),
                    ..Values::default()
                },
                Edging::SubpixelAntiAlias,
                FontHinting::None,
                true,
            ),
            // `rgba = none` asks for grayscale on every surface.
            (
                Values {
                    rgba: Some(FC_RGBA_NONE),
                    ..Values::default()
                },
                Edging::AntiAlias,
                FontHinting::Slight,
                true,
            ),
            (
                Values {
                    antialias: Some(false),
                    ..Values::default()
                },
                Edging::Alias,
                FontHinting::Slight,
                true,
            ),
        ];
        for (values, edging, hinting, subpixel_positioning) in cases {
            let rendering = TextRendering::from_fontconfig(&values);
            assert_eq!(
                (
                    rendering.edging,
                    rendering.hinting,
                    rendering.subpixel_positioning
                ),
                (edging, hinting, subpixel_positioning),
                "{values:?}"
            );
        }
    }
}
