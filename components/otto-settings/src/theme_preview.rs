//! Small pictures of what an icon or cursor theme actually looks like.
//!
//! A theme is chosen by eye, and its name says nothing about it — the same
//! reasoning the wallpaper row's thumbnail is built on. Both previews are a
//! handful of representative images laid on a light card, so a dark icon set
//! and a light one are judged against the same ground.

use std::cell::RefCell;
use std::collections::HashMap;

use skia_safe::Image;

/// The icons shown, in order. Each entry is a fallback chain: themes disagree
/// about which name a thing has, and a slot that resolves to nothing simply
/// stays empty rather than borrowing another theme's icon.
const ICON_SLOTS: &[&[&str]] = &[
    &["folder", "inode-directory"],
    &["user-home", "folder-home", "go-home"],
    &["text-x-generic", "text-x-preview", "document"],
    &["utilities-terminal", "terminal", "org.gnome.Terminal"],
    &[
        "applications-internet",
        "web-browser",
        "internet-web-browser",
    ],
];

/// The cursors shown, in order, each as a fallback chain over the names the
/// XDG cursor spec and the older X11 names both use.
const CURSOR_SLOTS: &[&[&str]] = &[
    &["default", "left_ptr", "arrow"],
    &["text", "xterm", "ibeam"],
    &["pointer", "hand2", "hand1", "pointing_hand"],
    &["help", "question_arrow", "whats_this"],
    &["wait", "watch"],
];

/// How many slots either preview draws. Fixed, so the card is the same size
/// whatever the theme happens to carry — a preview that changed width with
/// every selection would make the pane jump under the pointer.
pub const SLOTS: usize = 5;

/// One image per slot, `None` where the theme has nothing for it.
///
/// Decoding five icons — SVGs among them — is far too expensive to do per
/// frame, so each theme is resolved once and kept. The cache holds the misses
/// too: a theme with no `folder` must not be re-searched sixty times a second
/// either. One entry per theme looked at this session, which is bounded by how
/// many times somebody opens the pop-up; nothing here needs eviction.
pub fn icon_theme_images(theme: &str, px: i32) -> Slots {
    cached(("icon", theme, px), || {
        // An empty setting is "no preference", and what an application then
        // gets is the desktop's own theme — so that is what the preview shows,
        // rather than nothing at all.
        let theme = (!theme.is_empty())
            .then(|| theme.to_string())
            .or_else(otto_kit::icon_theme::current_icon_theme);
        ICON_SLOTS
            .iter()
            .map(|names| {
                names
                    .iter()
                    .find_map(|name| {
                        otto_kit::icons::exact_icon_in_theme(name, px, theme.as_deref())
                    })
                    .and_then(|path| otto_kit::icons::image_from_path(&path, (px, px)))
            })
            .collect()
    })
}

/// The same for a cursor theme, drawn from its XCursor files.
pub fn cursor_theme_images(theme: &str, px: i32) -> Slots {
    cached(("cursor", theme, px), || {
        // An empty setting means the desktop's own default: the theme the
        // environment names, and failing that the one the cursor search path
        // resolves "default" to.
        let fallback = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string());
        let name = if theme.is_empty() { &fallback } else { theme };
        let loaded = xcursor::CursorTheme::load(name);
        CURSOR_SLOTS
            .iter()
            .map(|names| {
                names
                    .iter()
                    .find_map(|name| loaded.load_icon(name))
                    .and_then(|path| std::fs::read(path).ok())
                    .and_then(|bytes| cursor_image(&bytes, px))
            })
            .collect()
    })
}

/// The frame of an XCursor file closest to `px`, as a Skia image.
///
/// Only the first frame of an animated cursor is taken: the preview is a still
/// picture of the theme, not a place to watch the busy pointer spin.
fn cursor_image(bytes: &[u8], px: i32) -> Option<Image> {
    let images = xcursor::parser::parse_xcursor(bytes)?;
    let best = images
        .iter()
        .min_by_key(|image| (px - image.size as i32).abs())?;
    let (w, h) = (best.width as i32, best.height as i32);
    // XCursor pixels are premultiplied ARGB words, which little-endian byte
    // order makes BGRA — the same reading the compositor's cursor path takes
    // when it hands them to a buffer as `Argb8888`.
    let info = skia_safe::ImageInfo::new(
        (w, h),
        skia_safe::ColorType::BGRA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    skia_safe::images::raster_from_data(
        &info,
        skia_safe::Data::new_copy(&best.pixels_rgba),
        (w * 4) as usize,
    )
}

/// What a preview resolves to: one entry per slot, empty where the theme has
/// nothing.
type Slots = Vec<Option<Image>>;

/// A resolved preview, keyed by kind, theme name and pixel size.
type CacheKey = (&'static str, String, i32);

fn cached(key: (&'static str, &str, i32), build: impl FnOnce() -> Slots) -> Slots {
    thread_local! {
        static CACHE: RefCell<HashMap<CacheKey, Slots>> = RefCell::new(HashMap::new());
    }
    let key = (key.0, key.1.to_string(), key.2);
    CACHE.with(|cache| {
        if let Some(hit) = cache.borrow().get(&key) {
            return hit.clone();
        }
        let built = build();
        cache.borrow_mut().insert(key, built.clone());
        built
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_is_answered_even_when_the_theme_is_unknown() {
        // A theme nobody has installed must still produce one entry per slot,
        // because the card's layout counts on it.
        assert_eq!(icon_theme_images("no-such-theme-at-all", 32).len(), SLOTS);
        assert_eq!(cursor_theme_images("no-such-theme-at-all", 32).len(), SLOTS);
    }
}
