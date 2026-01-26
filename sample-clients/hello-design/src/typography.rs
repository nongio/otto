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
        let mut font = Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::SubpixelAntiAlias);

        // Cache it
        self.cache.borrow_mut().insert(key, font.clone());
        Some(font)
    }

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
        let mut font = Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::SubpixelAntiAlias);
        font
    }
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

/// Predefined text styles for a consistent design system
pub struct TextStyle {
    pub family: &'static str,
    pub weight: i32,
    pub size: f32,
}

impl TextStyle {
    /// Create a Skia Font from this text style
    pub fn font(&self) -> Font {
        use skia::font_style::{Slant, Weight, Width};
        let weight = Weight::from(self.weight);
        let style = FontStyle::new(weight, Width::NORMAL, Slant::Upright);
        get_font_with_fallback(self.family, style, self.size)
    }
}

/// Design system typography scale
pub mod styles {
    use super::*;

    /// Display - Extra large text for hero sections
    pub const DISPLAY: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 57.0,
    };

    /// Headline 1 - Page titles
    pub const H1: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 32.0,
    };

    /// Headline 2 - Section headers
    pub const H2: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 28.0,
    };

    /// Headline 3 - Subsection headers
    pub const H3: TextStyle = TextStyle {
        family: "Inter",
        weight: 700, // Bold
        size: 24.0,
    };

    /// Title - Card titles, dialog titles
    pub const TITLE: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Normal
        size: 20.0,
    };

    /// Body - Default text
    pub const BODY: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Normal
        size: 16.0,
    };

    /// Body Small - Secondary text
    pub const BODY_SMALL: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Normal
        size: 14.0,
    };

    /// Label - Button text, form labels
    pub const LABEL: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Normal
        size: 14.0,
    };

    /// Caption - Helper text, metadata
    pub const CAPTION: TextStyle = TextStyle {
        family: "Inter",
        weight: 400, // Normal
        size: 12.0,
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
        let _h1 = styles::H1.font();
        let _body = styles::BODY.font();
        let _caption = styles::CAPTION.font();
        // If we get here without panic, fonts loaded successfully
    }
}


