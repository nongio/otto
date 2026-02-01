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
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub family: &'static str,
    pub weight: i32,
    pub size: f32,
}

impl TextStyle {
    /// Create a Skia Font from this text style with proper antialiasing
    pub fn font(&self) -> Font {
        use skia::font_style::{Slant, Weight, Width};
        let weight = Weight::from(self.weight);
        let style = FontStyle::new(weight, Width::NORMAL, Slant::Upright);
        let mut font = get_font_with_fallback(self.family, style, self.size);
        font.set_subpixel(true);
        font.set_edging(skia::font::Edging::SubpixelAntiAlias);
        font
    }
}

/// Design system typography scale (based on macOS HIG)
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
}
