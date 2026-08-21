//! Offscreen gallery of window-decoration variants.
//!
//! Renders mock decorated windows straight to a PNG with Skia's raster
//! backend — no compositor, no Wayland — so the titlebar look can be iterated
//! on quickly.
//!
//! ```sh
//! cargo run -p otto-kit --example titlebar_gallery -- /tmp/titlebars.png
//! ```

use otto_kit::components::titlebar::{Titlebar, TitlebarGroup, WindowControl, WindowControls};
use otto_kit::prelude::*;
use skia_safe::{
    surfaces, BlurStyle, ClipOp, EncodedImageFormat, MaskFilter, PaintStyle, RRect, Rect,
};

/// One cell of the gallery: a decorated window on the desktop backdrop.
struct Variant {
    caption: &'static str,
    title: &'static str,
    theme: Theme,
    dark: bool,
    active: bool,
    hovered: bool,
    pressed: Option<WindowControl>,
    titlebar_height: f32,
    title_style: TextStyle,
    /// Traffic lights on the left (macOS-like) vs plain title only
    traffic_lights: bool,
    /// Title hugs the leading edge instead of being centered
    title_leading: bool,
    /// Hairline separating titlebar from content
    separator: bool,
}

impl Variant {
    fn light(caption: &'static str) -> Self {
        Self {
            caption,
            title: "Documents — index.md",
            theme: Theme::light(),
            dark: false,
            active: true,
            hovered: false,
            pressed: None,
            titlebar_height: 34.0,
            title_style: styles::SUBHEADLINE_EMPHASIZED,
            traffic_lights: true,
            title_leading: false,
            separator: true,
        }
    }

    fn dark(caption: &'static str) -> Self {
        Self {
            theme: Theme::dark(),
            dark: true,
            ..Self::light(caption)
        }
    }
}

const WINDOW_W: f32 = 460.0;
const WINDOW_H: f32 = 230.0;
const CORNER: f32 = 12.0;
const CELL_W: f32 = 560.0;
const CELL_H: f32 = 330.0;
const COLS: usize = 2;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "titlebar_gallery.png".to_string());

    let variants = vec![
        Variant {
            caption: "light · active",
            ..Variant::light("light · active")
        },
        Variant {
            active: false,
            title: "Documents — notes.md",
            ..Variant::light("light · inactive")
        },
        Variant {
            hovered: true,
            ..Variant::light("light · pointer over controls")
        },
        Variant {
            hovered: true,
            pressed: Some(WindowControl::Close),
            ..Variant::light("light · close pressed")
        },
        Variant {
            ..Variant::dark("dark · active")
        },
        Variant {
            active: false,
            ..Variant::dark("dark · inactive")
        },
        Variant {
            titlebar_height: 28.0,
            title_style: styles::FOOTNOTE_EMPHASIZED,
            ..Variant::light("compact 28pt titlebar")
        },
        Variant {
            titlebar_height: 44.0,
            title_style: styles::HEADLINE,
            ..Variant::light("tall 44pt titlebar")
        },
        Variant {
            title_leading: true,
            title: "Terminal",
            ..Variant::light("leading title")
        },
        Variant {
            traffic_lights: false,
            separator: false,
            title: "Preview",
            ..Variant::light("no controls · no separator")
        },
    ];

    let rows = variants.len().div_ceil(COLS);
    let scale = 2.0_f32;
    let width = (CELL_W * COLS as f32 * scale) as i32;
    let height = (CELL_H * rows as f32 * scale) as i32;

    let mut surface = surfaces::raster_n32_premul((width, height)).expect("raster surface");
    let canvas = surface.canvas();
    canvas.scale((scale, scale));

    for (i, variant) in variants.iter().enumerate() {
        let col = i % COLS;
        let row = i / COLS;
        canvas.save();
        canvas.translate((col as f32 * CELL_W, row as f32 * CELL_H));
        draw_cell(canvas, variant);
        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&out, data.as_bytes()).expect("write png");
    println!("wrote {out} ({width}x{height})");
}

/// Backdrop + caption + one decorated window.
fn draw_cell(canvas: &Canvas, v: &Variant) {
    // Desktop backdrop: a flat wallpaper stand-in, light or dark, so the
    // window edge and shadow are judged against something realistic.
    let mut bg = Paint::default();
    bg.set_color(if v.dark {
        Color::from_rgb(0x1E, 0x22, 0x2B)
    } else {
        Color::from_rgb(0x8C, 0x9E, 0xB8)
    });
    canvas.draw_rect(Rect::from_wh(CELL_W, CELL_H), &bg);

    // Caption
    canvas.save();
    canvas.translate((24.0, 18.0));
    Label::new(v.caption)
        .with_style(styles::CAPTION_1)
        .with_color(if v.dark {
            Color::from_argb(0xCC, 0xFF, 0xFF, 0xFF)
        } else {
            Color::from_argb(0xE0, 0xFF, 0xFF, 0xFF)
        })
        .render(canvas);
    canvas.restore();

    canvas.save();
    canvas.translate(((CELL_W - WINDOW_W) / 2.0, 56.0));
    draw_window(canvas, v);
    canvas.restore();
}

fn draw_window(canvas: &Canvas, v: &Variant) {
    let frame = Rect::from_wh(WINDOW_W, WINDOW_H);
    let rrect = RRect::new_rect_xy(frame, CORNER, CORNER);

    // Drop shadow — heavier when the window is focused, matching the
    // compositor's own active/inactive shadow split.
    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_color(if v.active {
        Color::from_argb(0x66, 0, 0, 0)
    } else {
        Color::from_argb(0x2E, 0, 0, 0)
    });
    shadow.set_mask_filter(MaskFilter::blur(
        BlurStyle::Normal,
        if v.active { 14.0 } else { 8.0 },
        false,
    ));
    canvas.save();
    canvas.translate((0.0, if v.active { 8.0 } else { 4.0 }));
    canvas.draw_rrect(rrect, &shadow);
    canvas.restore();

    canvas.save();
    canvas.clip_rrect(rrect, ClipOp::Intersect, true);

    // Client content stand-in
    let mut content = Paint::default();
    content.set_color(if v.dark {
        Color::from_rgb(0x24, 0x26, 0x2B)
    } else {
        Color::WHITE
    });
    canvas.draw_rect(frame, &content);
    draw_content_placeholder(canvas, v);

    // Titlebar
    let mut titlebar = Titlebar::new()
        .at(0.0, 0.0)
        .with_width(WINDOW_W)
        .with_height(v.titlebar_height)
        .with_corner_radius(CORNER)
        .with_padding((v.titlebar_height - 12.0) / 2.0)
        .with_background(titlebar_material(v))
        .with_title(
            Label::new(v.title)
                .with_style(v.title_style)
                .with_color(title_color(v)),
        );

    if v.separator {
        titlebar = titlebar.with_border_bottom(separator_color(v));
    }
    if v.traffic_lights {
        titlebar = titlebar.with_leading(
            TitlebarGroup::new().add(
                WindowControls::new()
                    .with_active(v.active)
                    .with_hovered(v.hovered)
                    .with_pressed(v.pressed)
                    .with_dark(v.dark),
            ),
        );
    }
    titlebar.render(canvas);

    // A leading title can't go through Titlebar's centering, so draw it here
    // on top of the (title-less) bar.
    if v.title_leading {
        let x = if v.traffic_lights { 84.0 } else { 14.0 };
        let th = Label::new(v.title)
            .with_style(v.title_style)
            .intrinsic_size()
            .map(|(_, h)| h)
            .unwrap_or(v.title_style.size);
        canvas.save();
        canvas.translate((x, (v.titlebar_height - th) / 2.0));
        Label::new(v.title)
            .with_style(v.title_style)
            .with_color(if v.active {
                v.theme.text_primary
            } else {
                v.theme.text_tertiary
            })
            .render(canvas);
        canvas.restore();
    }

    canvas.restore();

    // Outer hairline: separates the window from the wallpaper on both themes
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(if v.dark {
        Color::from_argb(0x40, 0xFF, 0xFF, 0xFF)
    } else {
        Color::from_argb(0x33, 0x00, 0x00, 0x00)
    });
    canvas.draw_rrect(
        RRect::new_rect_xy(frame.with_inset((0.5, 0.5)), CORNER, CORNER),
        &edge,
    );
}

/// The centered title is drawn by `Titlebar`; when `title_leading` is set the
/// centered one is suppressed by handing it an empty string.
fn title_color(v: &Variant) -> Color {
    if v.title_leading {
        // drawn separately below, keep the centered copy invisible
        return Color::TRANSPARENT;
    }
    if v.active {
        v.theme.text_primary
    } else {
        v.theme.text_tertiary
    }
}

fn titlebar_material(v: &Variant) -> Color {
    if v.active {
        v.theme.material_titlebar
    } else if v.dark {
        Color::from_rgb(0x2A, 0x2C, 0x31)
    } else {
        Color::from_rgb(0xF2, 0xF2, 0xF4)
    }
}

fn separator_color(v: &Variant) -> Color {
    if v.dark {
        Color::from_argb(0x59, 0x00, 0x00, 0x00)
    } else {
        v.theme.fill_secondary
    }
}

/// Rough page content so the titlebar is judged in context.
fn draw_content_placeholder(canvas: &Canvas, v: &Variant) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let bar = if v.dark {
        Color::from_argb(0x2A, 0xFF, 0xFF, 0xFF)
    } else {
        Color::from_argb(0x1F, 0x00, 0x00, 0x00)
    };
    paint.set_color(bar);

    let top = v.titlebar_height + 22.0;
    let widths = [0.62, 0.84, 0.74, 0.48, 0.8, 0.36];
    for (i, w) in widths.iter().enumerate() {
        let y = top + i as f32 * 22.0;
        if y > WINDOW_H - 20.0 {
            break;
        }
        canvas.draw_rrect(
            RRect::new_rect_xy(
                Rect::from_xywh(24.0, y, (WINDOW_W - 48.0) * w, 10.0),
                5.0,
                5.0,
            ),
            &paint,
        );
    }
}
