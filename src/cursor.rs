use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;

use anyhow::{anyhow, Context};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Physical, Point, Transform};
use smithay::wayland::compositor::with_states;
use tracing::warn;
use xcursor::parser::{parse_xcursor, Image};
use xcursor::CursorTheme;

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

/// Side of the built-in cursor bitmap, in pixels. It is square.
const FALLBACK_SRC_SIDE: i32 = 64;

/// The arrow's tip within that bitmap — the point that tracks the pointer.
const FALLBACK_HOTSPOT: (i32, i32) = (1, 1);

/// Simple xcursor loader for XWayland default cursor.
#[cfg(feature = "xwayland")]
pub struct Cursor {
    icons: Vec<Image>,
    size: u32,
}

#[cfg(feature = "xwayland")]
impl Cursor {
    pub fn load() -> Cursor {
        let name = std::env::var("XCURSOR_THEME")
            .ok()
            .unwrap_or_else(|| "default".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        let theme = CursorTheme::load(&name);
        let icons = theme
            .load_icon("default")
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|mut f| {
                let mut data = Vec::new();
                f.read_to_end(&mut data).ok()?;
                parse_xcursor(&data)
            })
            .unwrap_or_else(|| {
                vec![Image {
                    size: 32,
                    width: 64,
                    height: 64,
                    xhot: 1,
                    yhot: 1,
                    delay: 1,
                    pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
                    pixels_argb: vec![],
                }]
            });
        Cursor { icons, size }
    }

    pub fn get_image(&self, scale: u32, time: std::time::Duration) -> Image {
        let target_size = self.size * scale;
        let nearest = self
            .icons
            .iter()
            .min_by_key(|i| (target_size as i32 - i.size as i32).abs())
            .unwrap();
        let total: u32 = self
            .icons
            .iter()
            .filter(|i| i.width == nearest.width)
            .map(|i| i.delay)
            .sum();
        if total == 0 {
            return nearest.clone();
        }
        let mut millis = time.as_millis() as u32 % total;
        for img in self.icons.iter().filter(|i| i.width == nearest.width) {
            if millis < img.delay {
                return img.clone();
            }
            millis -= img.delay;
        }
        nearest.clone()
    }
}

type XCursorCache = HashMap<(CursorIcon, i32), Option<Rc<XCursor>>>;

pub struct CursorManager {
    theme: CursorTheme,
    size: u8,
    current_cursor: CursorImageStatus,
    named_cursor_cache: RefCell<XCursorCache>,
}

impl CursorManager {
    pub fn new(theme: &str, size: u8) -> Self {
        Self::ensure_env(theme, size);

        let theme = CursorTheme::load(theme);

        Self {
            theme,
            size,
            current_cursor: CursorImageStatus::default_named(),
            named_cursor_cache: Default::default(),
        }
    }

    pub fn reload(&mut self, theme: &str, size: u8) {
        Self::ensure_env(theme, size);
        self.theme = CursorTheme::load(theme);
        self.size = size;
        self.named_cursor_cache.get_mut().clear();
    }

    pub fn check_cursor_image_surface_alive(&mut self) {
        if let CursorImageStatus::Surface(surface) = &self.current_cursor {
            if !surface.alive() {
                self.current_cursor = CursorImageStatus::default_named();
            }
        }
    }

    pub fn get_render_cursor(&self, scale: i32) -> RenderCursor {
        match self.current_cursor.clone() {
            CursorImageStatus::Hidden => RenderCursor::Hidden,
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .hotspot
                });

                RenderCursor::Surface { hotspot, surface }
            }
            CursorImageStatus::Named(icon) => self.get_render_cursor_named(icon, scale),
        }
    }

    fn get_render_cursor_named(&self, icon: CursorIcon, scale: i32) -> RenderCursor {
        self.get_cursor_with_name(icon, scale)
            .map(|cursor| RenderCursor::Named {
                icon,
                scale,
                cursor,
            })
            .unwrap_or_else(|| RenderCursor::Named {
                icon: Default::default(),
                scale,
                cursor: self.get_default_cursor(scale),
            })
    }

    pub fn is_current_cursor_animated(&self, scale: i32) -> bool {
        match &self.current_cursor {
            CursorImageStatus::Hidden => false,
            CursorImageStatus::Surface(_) => false,
            CursorImageStatus::Named(icon) => self
                .get_cursor_with_name(*icon, scale)
                .unwrap_or_else(|| self.get_default_cursor(scale))
                .is_animated_cursor(),
        }
    }

    pub fn get_cursor_with_name(&self, icon: CursorIcon, scale: i32) -> Option<Rc<XCursor>> {
        self.named_cursor_cache
            .borrow_mut()
            .entry((icon, scale))
            .or_insert_with_key(|(icon, scale)| {
                let size = self.size as i32 * scale;
                let mut cursor = Self::load_named(&self.theme, *icon, size);

                if let Err(err) = &cursor {
                    warn!("error loading xcursor {}@{size}: {err:?}", icon.name());
                }

                if *icon == CursorIcon::Default && cursor.is_err() {
                    cursor = Ok(Self::fallback_cursor(size));
                }

                cursor.ok().map(Rc::new)
            })
            .clone()
    }

    pub fn get_default_cursor(&self, scale: i32) -> Rc<XCursor> {
        self.get_cursor_with_name(CursorIcon::Default, scale)
            .unwrap()
    }

    pub fn cursor_image(&self) -> &CursorImageStatus {
        &self.current_cursor
    }

    pub fn set_cursor_image(&mut self, cursor: CursorImageStatus) {
        self.current_cursor = cursor;
    }

    /// Load `icon` from `theme`, preferring the shallowest theme that has it.
    ///
    /// An icon has a modern name (`default`, `pointer`) and a set of legacy X11
    /// names (`left_ptr`, `hand2`). Trying the modern name to exhaustion first
    /// is wrong, because a theme with no `Inherits=` still implicitly inherits
    /// `default` (usually Adwaita): a legacy-named theme — one shipping
    /// `left_ptr` but no `default` — loses the modern name to its fallback
    /// theme even though it has the icon under the old name. The user then gets
    /// the fallback theme's art for most of the desktop and their own theme's
    /// only for the handful of icons whose modern and legacy names coincide
    /// (`move`, `copy`, `crosshair`), so the pointer changes appearance and size
    /// mid-gesture — most visibly when a drag swaps `default` for `move`.
    ///
    /// So rank every candidate name by how far up the inheritance chain it was
    /// found and take the nearest, breaking ties in name order. The configured
    /// theme always wins over the one it falls back to, whatever the name.
    fn load_named(theme: &CursorTheme, icon: CursorIcon, size: i32) -> anyhow::Result<XCursor> {
        let (path, _) = std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .filter_map(|name| theme.load_icon_with_depth(name))
            .min_by_key(|(_, depth)| *depth)
            .ok_or_else(|| anyhow!("no icon named {} in the cursor theme", icon.name()))?;

        Self::parse_xcursor_file(&path, size)
    }

    /// Read one xcursor file and keep only the frames at the size nearest to
    /// `size`, which is in physical pixels.
    fn parse_xcursor_file(path: &std::path::Path, size: i32) -> anyhow::Result<XCursor> {
        let mut file = File::open(path).context("error opening cursor icon file")?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)
            .context("error reading cursor icon file")?;

        let mut images = parse_xcursor(&buf).context("error parsing cursor icon file")?;

        let (width, height) = images
            .iter()
            .min_by_key(|image| (size - image.size as i32).abs())
            .map(|image| (image.width, image.height))
            .unwrap();

        images.retain(move |image| image.width == width && image.height == height);

        // A theme answers a size request with the nearest art it happens to
        // ship, and several legacy X11 themes ship a single 32px bitmap and
        // nothing else. On a HiDPI screen that draws a pointer a fraction of
        // `cursor_size`, so resample the frames to the size that was asked for.
        // A theme with art at the requested size lands here as a no-op.
        let nominal = images.first().map(|image| image.size).unwrap_or(0);
        if nominal != 0 && nominal != size.max(1) as u32 {
            let factor = size.max(1) as f32 / nominal as f32;
            for image in &mut images {
                let dst_w = ((image.width as f32 * factor).round() as i32).max(1);
                let dst_h = ((image.height as f32 * factor).round() as i32).max(1);
                image.pixels_rgba = Self::resample(
                    &image.pixels_rgba,
                    image.width as i32,
                    0,
                    0,
                    image.width as i32,
                    image.height as i32,
                    dst_w,
                    dst_h,
                );
                image.pixels_argb = vec![];
                image.xhot = ((image.xhot as f32 * factor).round() as u32).min(dst_w as u32);
                image.yhot = ((image.yhot as f32 * factor).round() as u32).min(dst_h as u32);
                image.width = dst_w as u32;
                image.height = dst_h as u32;
                image.size = size.max(1) as u32;
            }
        }

        let animation_duration = images.iter().fold(0, |acc, image| acc + image.delay);

        Ok(XCursor {
            images,
            animation_duration,
        })
    }

    fn ensure_env(theme: &str, size: u8) {
        env::set_var("XCURSOR_THEME", theme);
        env::set_var("XCURSOR_SIZE", size.to_string());
    }

    /// The built-in cursor, drawn at `size` physical pixels.
    ///
    /// A theme answers a size request with art that fills the box it reports.
    /// The built-in bitmap is instead one fixed image, an arrow occupying the
    /// top-left corner of a 64×64 canvas, so emitting it unscaled draws a
    /// cursor a fraction of the size that was asked for. That is worst exactly
    /// where the fallback is most likely to be reached: a HiDPI screen with a
    /// theme the process cannot see, such as a greeter running as a user with
    /// no `~/.icons`. Resample the arrow to the requested size instead.
    fn fallback_cursor(size: i32) -> XCursor {
        let (art_x, art_y, art_w, art_h) = Self::fallback_art_bounds();
        // Match the height, as a themed arrow does; the width follows from the
        // aspect ratio so the shape is not distorted.
        let scale = (size.max(1) as f32 / art_h as f32).max(f32::MIN_POSITIVE);
        let width = ((art_w as f32 * scale).round() as i32).max(1);
        let height = ((art_h as f32 * scale).round() as i32).max(1);

        let images = vec![Image {
            size: size.max(1) as u32,
            width: width as u32,
            height: height as u32,
            // The hotspot is the arrow's tip, so it moves with the art.
            xhot: (((FALLBACK_HOTSPOT.0 - art_x) as f32) * scale).round() as u32,
            yhot: (((FALLBACK_HOTSPOT.1 - art_y) as f32) * scale).round() as u32,
            delay: 0,
            pixels_rgba: Self::resample_fallback(art_x, art_y, art_w, art_h, width, height),
            pixels_argb: vec![],
        }];

        XCursor {
            images,
            animation_duration: 0,
        }
    }

    /// Extent of the drawn arrow within the built-in bitmap, as
    /// `(x, y, width, height)`.
    ///
    /// Measured rather than hardcoded so redrawing `resources/cursor.rgba`
    /// cannot silently leave the scaling wrong. Falls back to the whole canvas
    /// for a bitmap that is entirely transparent.
    fn fallback_art_bounds() -> (i32, i32, i32, i32) {
        let side = FALLBACK_SRC_SIDE;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (side, side, -1, -1);

        for y in 0..side {
            for x in 0..side {
                // Argb8888 little-endian: alpha is the last byte of each texel.
                if FALLBACK_CURSOR_DATA[((y * side + x) * 4 + 3) as usize] > 0 {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }

        if max_x < min_x || max_y < min_y {
            return (0, 0, side, side);
        }
        (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
    }

    /// Bilinearly scale the `src_*` region of the built-in bitmap into a new
    /// `dst_w × dst_h` buffer.
    ///
    /// The channels are premultiplied, which is what makes interpolating them
    /// independently correct — straight alpha would bleed the colour of fully
    /// transparent texels into the arrow's edge.
    fn resample_fallback(
        src_x: i32,
        src_y: i32,
        src_w: i32,
        src_h: i32,
        dst_w: i32,
        dst_h: i32,
    ) -> Vec<u8> {
        Self::resample(
            FALLBACK_CURSOR_DATA,
            FALLBACK_SRC_SIDE,
            src_x,
            src_y,
            src_w,
            src_h,
            dst_w,
            dst_h,
        )
    }

    /// Bilinearly resample the `src_w`×`src_h` region at (`src_x`, `src_y`) of
    /// an Argb8888 buffer `stride` texels wide into a `dst_w`×`dst_h` image.
    #[allow(clippy::too_many_arguments)]
    fn resample(
        src: &[u8],
        stride: i32,
        src_x: i32,
        src_y: i32,
        src_w: i32,
        src_h: i32,
        dst_w: i32,
        dst_h: i32,
    ) -> Vec<u8> {
        let side = stride;
        let texel = |x: i32, y: i32, c: usize| -> f32 {
            let x = x.clamp(src_x, src_x + src_w - 1);
            let y = y.clamp(src_y, src_y + src_h - 1);
            src[((y * side + x) * 4) as usize + c] as f32
        };

        let mut pixels = vec![0u8; (dst_w * dst_h * 4) as usize];
        for y in 0..dst_h {
            // Sample at texel centres, so the edges of the source region map to
            // the edges of the destination rather than half a texel outside it.
            let sy = (y as f32 + 0.5) * src_h as f32 / dst_h as f32 - 0.5 + src_y as f32;
            let y0 = sy.floor();
            let fy = sy - y0;

            for x in 0..dst_w {
                let sx = (x as f32 + 0.5) * src_w as f32 / dst_w as f32 - 0.5 + src_x as f32;
                let x0 = sx.floor();
                let fx = sx - x0;
                let (x0, y0) = (x0 as i32, y0 as i32);

                for c in 0..4 {
                    let top = texel(x0, y0, c) * (1.0 - fx) + texel(x0 + 1, y0, c) * fx;
                    let bottom = texel(x0, y0 + 1, c) * (1.0 - fx) + texel(x0 + 1, y0 + 1, c) * fx;
                    let value = top * (1.0 - fy) + bottom * fy;
                    pixels[((y * dst_w + x) * 4) as usize + c] =
                        value.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        pixels
    }
}

pub enum RenderCursor {
    Hidden,
    Surface {
        hotspot: Point<i32, Logical>,
        surface: WlSurface,
    },
    Named {
        icon: CursorIcon,
        scale: i32,
        cursor: Rc<XCursor>,
    },
}

type TextureCache = HashMap<(CursorIcon, i32), Vec<MemoryRenderBuffer>>;

#[derive(Default)]
pub struct CursorTextureCache {
    cache: RefCell<TextureCache>,
}

impl CursorTextureCache {
    pub fn clear(&mut self) {
        self.cache.get_mut().clear();
    }

    pub fn get(
        &self,
        icon: CursorIcon,
        scale: i32,
        cursor: &XCursor,
        idx: usize,
    ) -> MemoryRenderBuffer {
        self.cache
            .borrow_mut()
            .entry((icon, scale))
            .or_insert_with(|| {
                cursor
                    .frames()
                    .iter()
                    .map(|frame| {
                        MemoryRenderBuffer::from_slice(
                            &frame.pixels_rgba,
                            Fourcc::Argb8888,
                            (frame.width as i32, frame.height as i32),
                            scale,
                            Transform::Normal,
                            None,
                        )
                    })
                    .collect()
            })[idx]
            .clone()
    }
}

pub struct XCursor {
    images: Vec<Image>,
    animation_duration: u32,
}

impl XCursor {
    pub fn frame(&self, mut millis: u32) -> (usize, &Image) {
        if self.animation_duration == 0 {
            return (0, &self.images[0]);
        }

        millis %= self.animation_duration;

        let mut res = 0;
        for (i, img) in self.images.iter().enumerate() {
            if millis < img.delay {
                res = i;
                break;
            }
            millis -= img.delay;
        }

        (res, &self.images[res])
    }

    pub fn frames(&self) -> &[Image] {
        &self.images
    }

    pub fn is_animated_cursor(&self) -> bool {
        self.images.len() > 1
    }

    pub fn hotspot(image: &Image) -> Point<i32, Physical> {
        (image.xhot as i32, image.yhot as i32).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this guards: the built-in arrow used to be emitted at a fixed
    /// 64×64 whatever size was asked for, so on a HiDPI screen without a
    /// readable cursor theme it drew at a fraction of the configured size.
    #[test]
    fn fallback_cursor_fills_the_requested_size() {
        for size in [16, 24, 32, 48, 64, 96, 128] {
            let cursor = CursorManager::fallback_cursor(size);
            let image = &cursor.images[0];

            assert_eq!(
                image.height as i32, size,
                "the arrow should be as tall as the size requested"
            );
            assert!(
                image.width > 0 && image.width <= image.height,
                "aspect ratio should be preserved, got {}x{}",
                image.width,
                image.height
            );
            assert_eq!(
                image.pixels_rgba.len(),
                (image.width * image.height * 4) as usize,
                "buffer must match the declared dimensions"
            );
            assert!(
                image.pixels_rgba.chunks(4).any(|texel| texel[3] > 0),
                "the arrow must still be visible after resampling"
            );
            assert!(
                (image.xhot as i32) < image.width as i32
                    && (image.yhot as i32) < image.height as i32,
                "the hotspot must stay inside the image"
            );
        }
    }

    /// The bitmap's opaque region is measured, not assumed; if it is ever
    /// redrawn the bounds must still describe something inside the canvas.
    #[test]
    fn fallback_art_bounds_lie_within_the_bitmap() {
        let (x, y, w, h) = CursorManager::fallback_art_bounds();
        assert!(x >= 0 && y >= 0, "bounds start inside the canvas");
        assert!(w > 0 && h > 0, "bounds are non-empty");
        assert!(
            x + w <= FALLBACK_SRC_SIDE && y + h <= FALLBACK_SRC_SIDE,
            "bounds end inside the canvas"
        );
        assert!(
            FALLBACK_HOTSPOT.0 >= x
                && FALLBACK_HOTSPOT.1 >= y
                && FALLBACK_HOTSPOT.0 < x + w
                && FALLBACK_HOTSPOT.1 < y + h,
            "the hotspot must sit within the art, or scaling it moves the tip"
        );
    }
}

#[cfg(test)]
mod theme_lookup_tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// `XCURSOR_PATH` is process-global, so the tests that plant a theme on
    /// disk take turns.
    static XCURSOR_PATH_LOCK: Mutex<()> = Mutex::new(());

    /// Encode a single-frame xcursor file: one `size`×`size` image filled with
    /// `fill` (Argb8888, little-endian byte order as the parser reads it).
    fn write_cursor(path: &Path, size: u32, fill: [u8; 4]) {
        let mut out = Vec::new();
        out.extend_from_slice(b"Xcur");
        out.extend_from_slice(&16u32.to_le_bytes()); // header length
        out.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // file version
        out.extend_from_slice(&1u32.to_le_bytes()); // one table entry

        let chunk_pos = 16 + 12;
        out.extend_from_slice(&0xfffd_0002u32.to_le_bytes()); // image chunk
        out.extend_from_slice(&size.to_le_bytes()); // nominal size
        out.extend_from_slice(&(chunk_pos as u32).to_le_bytes());

        for word in [0x24u32, 0xfffd_0002, size, 1, size, size, 0, 0, 0] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        for _ in 0..(size * size) {
            out.extend_from_slice(&fill);
        }

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, out).unwrap();
    }

    /// The bug this guards: a theme with no `Inherits=` still implicitly
    /// inherits `default`, so asking for the modern name first found the
    /// icon in the *fallback* theme and never tried the legacy name in the
    /// configured one. The user got the fallback theme's art for most of the
    /// desktop and their own only where the two names coincide (`move`), so
    /// starting a drag visibly swapped cursor themes.
    #[test]
    fn the_configured_theme_wins_over_the_one_it_falls_back_to() {
        let _guard = XCURSOR_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join("otto-cursor-inherit-test");
        let _ = std::fs::remove_dir_all(&root);

        // The configured theme has the icon only under its legacy X11 name.
        write_cursor(&root.join("legacy/cursors/left_ptr"), 32, [1, 2, 3, 255]);
        // The implicit fallback theme has it under the modern name.
        write_cursor(&root.join("default/cursors/default"), 32, [9, 9, 9, 255]);

        std::env::set_var("XCURSOR_PATH", &root);
        let theme = CursorTheme::load("legacy");
        let cursor = CursorManager::load_named(&theme, CursorIcon::Default, 32).unwrap();
        std::env::remove_var("XCURSOR_PATH");

        assert_eq!(
            cursor.images[0].pixels_rgba[0..4],
            [1, 2, 3, 255],
            "Default should resolve to the configured theme's left_ptr, \
             not to the fallback theme's default"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The bug this guards: a theme that ships a single 32px bitmap — several
    /// legacy X11 themes do — drew a pointer a fraction of `cursor_size` on a
    /// HiDPI screen, because the nearest available art was emitted unscaled.
    #[test]
    fn art_smaller_than_the_request_is_scaled_up_to_it() {
        let _guard = XCURSOR_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join("otto-cursor-resample-test");
        let _ = std::fs::remove_dir_all(&root);

        write_cursor(&root.join("small/cursors/left_ptr"), 32, [1, 2, 3, 255]);

        std::env::set_var("XCURSOR_PATH", &root);
        let theme = CursorTheme::load("small");
        let cursor = CursorManager::load_named(&theme, CursorIcon::Default, 80).unwrap();
        std::env::remove_var("XCURSOR_PATH");

        let image = &cursor.images[0];
        assert_eq!(
            (image.width, image.height),
            (80, 80),
            "a 32px bitmap asked for at 80 should be resampled, not emitted small"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
