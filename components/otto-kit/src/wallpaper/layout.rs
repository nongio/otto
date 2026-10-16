//! How a wallpaper is laid out on a screen: the mode, the position and the
//! zoom, and where they put the image.

use skia_safe as skia;

/// How the wallpaper fills the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// Scaled to cover the screen, cropping what does not fit. The default.
    #[default]
    Fill,
    /// Scaled to fit inside the screen, whole, with the fill colour around
    /// it.
    Fit,
    /// Stretched to the screen's exact shape, ignoring the image's.
    Stretch,
    /// At its own size, with the fill colour around it or cropped.
    Center,
    /// Repeated across the screen at its own size.
    Tile,
    /// One image across every screen, filling the desktop's bounding box
    /// as [`Mode::Fill`] fills one screen. Each screen shows its own part,
    /// at its own resolution.
    Span,
}

impl Mode {
    /// Every mode, in the order a picker lists them.
    pub const ALL: [Mode; 6] = [
        Mode::Fill,
        Mode::Fit,
        Mode::Stretch,
        Mode::Center,
        Mode::Tile,
        Mode::Span,
    ];

    /// The configuration token: what the setting stores.
    pub fn token(self) -> &'static str {
        match self {
            Mode::Fill => "fill",
            Mode::Fit => "fit",
            Mode::Stretch => "stretch",
            Mode::Center => "center",
            Mode::Tile => "tile",
            Mode::Span => "span",
        }
    }

    /// The mode a configuration token names. Case-insensitive, and accepting
    /// the British spelling of `center`.
    pub fn parse(token: &str) -> Option<Mode> {
        let token = token.trim().to_ascii_lowercase();
        Mode::ALL
            .into_iter()
            .find(|mode| mode.token() == token)
            .or(match token.as_str() {
                "centre" => Some(Mode::Center),
                _ => None,
            })
    }

    /// Whether the position setting moves the image in this mode.
    pub fn uses_position(self) -> bool {
        matches!(self, Mode::Fill | Mode::Fit | Mode::Center | Mode::Span)
    }

    /// Whether the zoom setting applies in this mode.
    pub fn uses_zoom(self) -> bool {
        matches!(self, Mode::Fill | Mode::Span)
    }

    /// Whether the image is drawn at its own size, scaled only by the
    /// output's scale.
    pub fn is_natural_size(self) -> bool {
        matches!(self, Mode::Center | Mode::Tile)
    }
}

/// The most a wallpaper can be zoomed in.
pub const MAX_ZOOM: f32 = 8.0;

/// A wallpaper's layout settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub mode: Mode,
    /// Which part of the image is shown where it does not fit, and where it
    /// sits where it does not fill: CSS's `object-position`, as fractions.
    /// `(0.5, 0.5)` centres; `(0.5, 0.0)` keeps the top edge of a cropped
    /// image, or puts a smaller one against the top of the screen. The image
    /// point at that fraction lands at the same fraction of the screen, so
    /// the position is always valid and never uncovers the screen.
    pub position: (f32, f32),
    /// Extra magnification on top of covering the screen, from 1.0. Panned
    /// by `position`. Only [`Mode::uses_zoom`] modes zoom.
    pub zoom: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            mode: Mode::Fill,
            position: (0.5, 0.5),
            zoom: 1.0,
        }
    }
}

impl Layout {
    /// The same layout with every value in range, and a value that is not a
    /// number put back to its default.
    pub fn sanitised(self) -> Self {
        let fraction = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.5
            }
        };
        let zoom = if self.zoom.is_finite() {
            self.zoom.clamp(1.0, MAX_ZOOM)
        } else {
            1.0
        };
        Self {
            mode: self.mode,
            position: (fraction(self.position.0), fraction(self.position.1)),
            zoom,
        }
    }
}

/// Where a laid-out image lands on the target, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrangement {
    /// The image's bounds on the target. For a tiled image, the first tile.
    pub dest: skia::Rect,
    /// Whether the image repeats from `dest` across the whole target.
    pub tiled: bool,
}

/// Where one screen sits on the desktop, for [`Mode::Span`]: the desktop's
/// bounding box and the screen's own rect, both in logical points in the
/// same global space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpanView {
    pub desktop: skia::Rect,
    pub screen: skia::Rect,
}

/// Lay out an `image`-sized picture on a `target`-sized screen whose scale is
/// `output_scale` (physical pixels per logical point).
///
/// Nothing is rounded here: the frame renderer snaps the visible edges to
/// whole pixels and resamples the exact region inside them.
///
/// [`Mode::Span`] lays the image out on the desktop described by `span`
/// and returns where that puts it on this screen; without a `span` it fills
/// the screen alone.
pub fn arrange(
    layout: Layout,
    image: (f32, f32),
    target: (i32, i32),
    output_scale: f32,
    span: Option<SpanView>,
) -> Arrangement {
    let target_f = (target.0.max(0) as f32, target.1.max(0) as f32);
    match span {
        Some(span) if layout.mode == Mode::Span && !span.screen.is_empty() => {
            let desktop = span.desktop;
            let on_desktop = arrange_in(layout, image, (desktop.width(), desktop.height()), 1.0);
            // Desktop points to this screen's pixels.
            let sx = target_f.0 / span.screen.width();
            let sy = target_f.1 / span.screen.height();
            let dx = desktop.left - span.screen.left;
            let dy = desktop.top - span.screen.top;
            let d = on_desktop.dest;
            Arrangement {
                dest: skia::Rect::from_ltrb(
                    (d.left + dx) * sx,
                    (d.top + dy) * sy,
                    (d.right + dx) * sx,
                    (d.bottom + dy) * sy,
                ),
                tiled: false,
            }
        }
        _ => arrange_in(layout, image, target_f, output_scale),
    }
}

fn arrange_in(
    layout: Layout,
    image: (f32, f32),
    target: (f32, f32),
    output_scale: f32,
) -> Arrangement {
    let layout = layout.sanitised();
    let (iw, ih) = (
        image.0.max(f32::MIN_POSITIVE),
        image.1.max(f32::MIN_POSITIVE),
    );
    let (tw, th) = (target.0.max(0.0), target.1.max(0.0));
    let output_scale = if output_scale.is_finite() && output_scale > 0.0 {
        output_scale
    } else {
        1.0
    };

    let (sx, sy) = match layout.mode {
        Mode::Fill | Mode::Span => {
            let s = (tw / iw).max(th / ih) * layout.zoom;
            (s, s)
        }
        Mode::Fit => {
            let s = (tw / iw).min(th / ih);
            (s, s)
        }
        Mode::Stretch => (tw / iw, th / ih),
        // An image's pixels are taken to be logical points, so it shows the
        // same size on every screen whatever its scale.
        Mode::Center | Mode::Tile => (output_scale, output_scale),
    };
    let (w, h) = (iw * sx, ih * sy);
    let (x, y) = match layout.mode {
        Mode::Stretch | Mode::Tile => (0.0, 0.0),
        _ => ((tw - w) * layout.position.0, (th - h) * layout.position.1),
    };
    Arrangement {
        dest: skia::Rect::from_xywh(x, y, w, h),
        tiled: layout.mode == Mode::Tile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (i32, i32) = (3440, 1440);
    const PHOTO: (f32, f32) = (3840.0, 2160.0);

    fn layout(mode: Mode) -> Layout {
        Layout {
            mode,
            ..Layout::default()
        }
    }

    fn covers(rect: skia::Rect, target: (i32, i32)) -> bool {
        rect.left <= 1e-3
            && rect.top <= 1e-3
            && rect.right >= target.0 as f32 - 1e-3
            && rect.bottom >= target.1 as f32 - 1e-3
    }

    fn inside(rect: skia::Rect, target: (i32, i32)) -> bool {
        rect.left >= -1e-3
            && rect.top >= -1e-3
            && rect.right <= target.0 as f32 + 1e-3
            && rect.bottom <= target.1 as f32 + 1e-3
    }

    #[test]
    fn every_mode_round_trips_through_its_token() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.token()), Some(mode));
        }
        assert_eq!(Mode::parse("Centre"), Some(Mode::Center));
        assert_eq!(Mode::parse(" FIT "), Some(Mode::Fit));
        assert_eq!(Mode::parse("zoom"), None);
    }

    /// Fill covers the screen at every position and zoom, and the position
    /// picks the part that shows: 0 the start, 1 the end.
    #[test]
    fn fill_covers_and_position_picks_the_crop() {
        for zoom in [1.0, 1.5, 3.0] {
            for position in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (0.3, 0.8)] {
                let a = arrange(
                    Layout {
                        mode: Mode::Fill,
                        position,
                        zoom,
                    },
                    PHOTO,
                    SCREEN,
                    1.0,
                    None,
                );
                assert!(covers(a.dest, SCREEN), "{zoom} {position:?}: {:?}", a.dest);
            }
        }
        let top = arrange(
            Layout {
                position: (0.5, 0.0),
                ..layout(Mode::Fill)
            },
            PHOTO,
            SCREEN,
            1.0,
            None,
        );
        assert_eq!(top.dest.top, 0.0, "the top edge is kept");
        let bottom = arrange(
            Layout {
                position: (0.5, 1.0),
                ..layout(Mode::Fill)
            },
            PHOTO,
            SCREEN,
            1.0,
            None,
        );
        assert!(
            (bottom.dest.bottom - SCREEN.1 as f32).abs() < 1e-3,
            "the bottom edge is kept"
        );
    }

    /// Zoom magnifies around the position: at 2x the centre half of the
    /// covering image fills the screen.
    #[test]
    fn zoom_magnifies() {
        let one = arrange(layout(Mode::Fill), PHOTO, SCREEN, 1.0, None);
        let two = arrange(
            Layout {
                zoom: 2.0,
                ..layout(Mode::Fill)
            },
            PHOTO,
            SCREEN,
            1.0,
            None,
        );
        assert!((two.dest.width() - one.dest.width() * 2.0).abs() < 1e-2);
        assert!((two.dest.center_x() - SCREEN.0 as f32 / 2.0).abs() < 1e-2);
        // Modes that do not zoom ignore it.
        let fit = arrange(
            Layout {
                zoom: 3.0,
                ..layout(Mode::Fit)
            },
            PHOTO,
            SCREEN,
            1.0,
            None,
        );
        assert_eq!(fit, arrange(layout(Mode::Fit), PHOTO, SCREEN, 1.0, None));
    }

    /// Fit shows the whole image, inside the screen, touching two edges, and
    /// the position slides it along the free axis.
    #[test]
    fn fit_shows_everything() {
        let a = arrange(layout(Mode::Fit), PHOTO, SCREEN, 1.0, None);
        assert!(inside(a.dest, SCREEN), "{:?}", a.dest);
        assert!((a.dest.height() - SCREEN.1 as f32).abs() < 1e-3);
        assert!((a.dest.center_x() - SCREEN.0 as f32 / 2.0).abs() < 1e-3);
        let left = arrange(
            Layout {
                position: (0.0, 0.5),
                ..layout(Mode::Fit)
            },
            PHOTO,
            SCREEN,
            1.0,
            None,
        );
        assert_eq!(left.dest.left, 0.0);
    }

    #[test]
    fn stretch_is_the_screen() {
        let a = arrange(layout(Mode::Stretch), PHOTO, SCREEN, 1.0, None);
        assert_eq!(a.dest, skia::Rect::from_iwh(SCREEN.0, SCREEN.1));
    }

    /// Centre and tile draw the image at its own size in logical points: at
    /// scale 2 it covers twice the pixels.
    #[test]
    fn natural_size_follows_the_output_scale() {
        let a = arrange(layout(Mode::Center), (800.0, 600.0), SCREEN, 1.0, None);
        assert_eq!((a.dest.width(), a.dest.height()), (800.0, 600.0));
        assert_eq!((a.dest.left, a.dest.top), (1320.0, 420.0));
        let b = arrange(layout(Mode::Center), (800.0, 600.0), SCREEN, 2.0, None);
        assert_eq!((b.dest.width(), b.dest.height()), (1600.0, 1200.0));
        let t = arrange(layout(Mode::Tile), (64.0, 48.0), SCREEN, 1.5, None);
        assert!(t.tiled);
        assert_eq!(t.dest, skia::Rect::from_xywh(0.0, 0.0, 96.0, 72.0));
    }

    /// Span lays the image over the desktop and gives each screen its part:
    /// two side-by-side screens split a 2:1 image down the middle, even when
    /// the right one has twice the pixel density.
    #[test]
    fn span_gives_each_screen_its_part_of_the_desktop() {
        let desktop = skia::Rect::from_xywh(0.0, 0.0, 200.0, 50.0);
        let left = SpanView {
            desktop,
            screen: skia::Rect::from_xywh(0.0, 0.0, 100.0, 50.0),
        };
        let right = SpanView {
            desktop,
            screen: skia::Rect::from_xywh(100.0, 0.0, 100.0, 50.0),
        };
        let a = arrange(
            layout(Mode::Span),
            (400.0, 100.0),
            (100, 50),
            1.0,
            Some(left),
        );
        assert_eq!(a.dest, skia::Rect::from_xywh(0.0, 0.0, 200.0, 50.0));
        let b = arrange(
            layout(Mode::Span),
            (400.0, 100.0),
            (200, 100),
            2.0,
            Some(right),
        );
        assert_eq!(b.dest, skia::Rect::from_xywh(-200.0, 0.0, 400.0, 100.0));
        // Without the desktop, span fills its own screen.
        assert_eq!(
            arrange(layout(Mode::Span), PHOTO, SCREEN, 1.0, None),
            arrange(layout(Mode::Fill), PHOTO, SCREEN, 1.0, None)
        );
    }

    /// Out-of-range settings are pulled back rather than producing a layout
    /// that uncovers the screen or divides by zero.
    #[test]
    fn nonsense_settings_are_sanitised() {
        let wild = Layout {
            mode: Mode::Fill,
            position: (f32::NAN, 7.0),
            zoom: -3.0,
        };
        let sane = wild.sanitised();
        assert_eq!(sane.position, (0.5, 1.0));
        assert_eq!(sane.zoom, 1.0);
        assert!(covers(
            arrange(wild, PHOTO, SCREEN, f32::NAN, None).dest,
            SCREEN
        ));
        assert_eq!(
            Layout {
                zoom: 100.0,
                ..Layout::default()
            }
            .sanitised()
            .zoom,
            MAX_ZOOM
        );
    }
}
