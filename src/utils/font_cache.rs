//! The thread-local Skia font cache every compositor-drawn text goes through.

use std::cell::RefCell;
use std::collections::HashMap;

use otto_kit::text_rendering::TextRendering;

#[allow(unused)]
pub struct FontCache {
    pub font_collection: layers::skia::textlayout::FontCollection,
    pub font_mgr: layers::skia::FontMgr,
    pub type_face_font_provider: RefCell<layers::skia::textlayout::TypefaceFontProvider>,
    /// Maps requested family name (lowercased) → resolved system family name from fuzzy matching.
    /// Avoids re-scanning all system families on every call to `make_font_with_fallback`.
    family_name_cache: RefCell<HashMap<String, String>>,
}

impl FontCache {
    /// Create a Font rendered as fontconfig configures `family`
    pub fn make_font(
        &self,
        family: impl AsRef<str>,
        style: layers::skia::FontStyle,
        size: f32,
    ) -> Option<layers::skia::Font> {
        let typeface = self.font_mgr.match_family_style(family.as_ref(), style)?;
        // The interface font need not cover the language the desktop is drawn
        // in — Inter has no CJK — and Skia does no per-glyph fallback of its
        // own, so the dock and the window titles came out as empty boxes.
        let typeface = otto_kit::typography::covering_typeface(
            &self.font_mgr,
            typeface,
            family.as_ref(),
            style,
        );
        let mut font = layers::skia::Font::from_typeface(typeface, size);
        TextRendering::for_family(family.as_ref()).apply_to_font(&mut font);
        Some(font)
    }

    /// Try fuzzy matching against available system font families.
    /// Attempts case-insensitive exact match first, then prefix matching
    /// (preferring shorter names as they're closer to the base family).
    fn fuzzy_match_font(
        &self,
        family: &str,
        style: layers::skia::FontStyle,
        size: f32,
    ) -> Option<layers::skia::Font> {
        let family_lower = family.to_lowercase();
        let mut best_prefix_match: Option<String> = None;

        for name in self.font_mgr.family_names() {
            let name_lower = name.to_lowercase();

            if name_lower == family_lower {
                tracing::debug!("Font '{}' matched (case-insensitive) to '{}'", family, name);
                return self.make_font(&name, style, size);
            }

            if name_lower.starts_with(&family_lower)
                && best_prefix_match
                    .as_ref()
                    .is_none_or(|prev| name.len() < prev.len())
            {
                best_prefix_match = Some(name);
            }
        }

        if let Some(ref matched_name) = best_prefix_match {
            tracing::debug!("Font '{}' fuzzy-matched to '{}'", family, matched_name);
            return self.make_font(matched_name, style, size);
        }

        None
    }

    /// Create a Font with fallback to system default if family not found
    pub fn make_font_with_fallback(
        &self,
        family: impl AsRef<str>,
        style: layers::skia::FontStyle,
        size: f32,
    ) -> layers::skia::Font {
        if let Some(font) = self.make_font(&family, style, size) {
            return font;
        }

        // Try fuzzy matching (case-insensitive, prefix), using the cache to avoid
        // re-scanning all system families on every call from the render path.
        let family_lower = family.as_ref().to_lowercase();
        let cached_name = self.family_name_cache.borrow().get(&family_lower).cloned();
        if let Some(ref resolved) = cached_name {
            if let Some(font) = self.make_font(resolved, style, size) {
                return font;
            }
        } else if let Some(font) = self.fuzzy_match_font(family.as_ref(), style, size) {
            // Store the resolved name so future calls skip the O(N) scan.
            self.family_name_cache
                .borrow_mut()
                .insert(family_lower, font.typeface().family_name());
            return font;
        }

        // Try common fallback fonts
        for fallback in ["sans-serif", "DejaVu Sans", "Liberation Sans", "Arial"] {
            if let Some(font) = self.make_font(fallback, style, size) {
                tracing::warn!(
                    "Font '{}' not found, using fallback: '{}'",
                    family.as_ref(),
                    fallback
                );
                return font;
            }
        }

        // Last resort: use default typeface from font manager
        tracing::error!(
            "Font '{}' and all fallbacks failed, using default",
            family.as_ref()
        );
        let typeface = self
            .font_mgr
            .legacy_make_typeface(None, style)
            .expect("Failed to create default typeface");
        let typeface =
            otto_kit::typography::covering_typeface(&self.font_mgr, typeface, "sans-serif", style);
        let mut font = layers::skia::Font::from_typeface(typeface, size);
        TextRendering::for_family("sans-serif").apply_to_font(&mut font);
        font
    }
}

thread_local! {
    pub static FONT_CACHE: FontCache = {
        let font_mgr = layers::skia::FontMgr::new();
        let type_face_font_provider = layers::skia::textlayout::TypefaceFontProvider::new();
        let mut font_collection = layers::skia::textlayout::FontCollection::new();
        font_collection.set_asset_font_manager(Some(type_face_font_provider.clone().into()));
        font_collection.set_dynamic_font_manager(font_mgr.clone());
        FontCache { font_collection, font_mgr, type_face_font_provider: RefCell::new(type_face_font_provider), family_name_cache: RefCell::new(HashMap::new()) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_cache() -> FontCache {
        let font_mgr = layers::skia::FontMgr::new();
        let type_face_font_provider = layers::skia::textlayout::TypefaceFontProvider::new();
        let mut font_collection = layers::skia::textlayout::FontCollection::new();
        font_collection.set_asset_font_manager(Some(type_face_font_provider.clone().into()));
        font_collection.set_dynamic_font_manager(font_mgr.clone());
        FontCache {
            font_collection,
            font_mgr,
            type_face_font_provider: RefCell::new(type_face_font_provider),
            family_name_cache: RefCell::new(HashMap::new()),
        }
    }

    // These tests require fonts-dejavu-core (Ubuntu/Debian) or ttf-dejavu (Arch)
    // to be installed, providing "DejaVu Sans", "DejaVu Sans Condensed", etc.
    // Run manually with: cargo test --lib -p otto -- utils::font_cache::tests --ignored

    #[test]
    #[ignore]
    fn exact_match_works() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        assert!(
            cache.make_font("DejaVu Sans", style, 12.0).is_some(),
            "exact match for 'DejaVu Sans' should succeed"
        );
    }

    #[test]
    #[ignore]
    fn case_insensitive_match() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        // Whether Skia's own match_family_style is case-insensitive is platform-dependent.
        // Our fuzzy_match_font should always handle it regardless.
        let font = cache.fuzzy_match_font("dejavu sans", style, 12.0);
        assert!(
            font.is_some(),
            "case-insensitive match for 'dejavu sans' should succeed"
        );
    }

    #[test]
    #[ignore]
    fn prefix_match_picks_shortest() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();

        // "DejaVu" should prefix-match to "DejaVu Sans" (shortest family starting with "DejaVu")
        // rather than "DejaVu Sans Mono" or "DejaVu Sans Condensed"
        let font = cache
            .fuzzy_match_font("DejaVu", style, 12.0)
            .expect("prefix match for 'DejaVu' should find a font");
        let family_name = font.typeface().family_name();
        assert_eq!(
            family_name, "DejaVu Sans",
            "expected shortest DejaVu family ('DejaVu Sans'), got '{}'",
            family_name
        );
    }

    #[test]
    #[ignore]
    fn fallback_with_fuzzy_returns_font() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        // make_font_with_fallback should use fuzzy matching before falling back to generic fallbacks
        let font = cache.make_font_with_fallback("dejavu sans", style, 12.0);
        let family_name = font.typeface().family_name();
        assert!(
            family_name.starts_with("DejaVu"),
            "expected DejaVu family, got '{}'",
            family_name
        );
    }

    #[test]
    #[ignore]
    fn nonexistent_font_falls_back() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        let font = cache.make_font_with_fallback("ZzzNonExistentFont999", style, 12.0);
        let family_name = font.typeface().family_name();
        assert!(
            !family_name.is_empty() && family_name != "ZzzNonExistentFont999",
            "expected a fallback family, got '{}'",
            family_name
        );
    }
}
