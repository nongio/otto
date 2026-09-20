//! Rendering a preview to a PNG, with no display attached.
//!
//! This is not a debug detour: it draws through exactly the path the window
//! will use — `otto_kit::preview::draw` onto a Skia canvas — so what it
//! produces is what the window will show. It makes the drawing half reviewable
//! and testable before any Wayland code exists, and it keeps working afterwards
//! as the way to see a preview without a session.

use otto_kit::preview::{self, Preview};
use otto_kit::theme::Theme;
use skia_safe::{Canvas, Color, Paint, Rect};

/// The card's chrome: the header a window draws around the content.
///
/// Kept here rather than in `otto_kit::preview` because it belongs to the
/// *window*, not to the preview — the dock drawing a thumbnail server-side
/// wants the content and none of this.
const HEADER: f32 = 52.0;
const RADIUS: f32 = 12.0;

/// Draw a complete preview card and encode it as a PNG.
pub fn to_png(
    preview: &Preview,
    title: &str,
    width: i32,
    height: i32,
    dark: bool,
) -> Option<Vec<u8>> {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut surface = skia_safe::surfaces::raster_n32_premul((width, height))?;
    let canvas = surface.canvas();

    draw_card(
        canvas,
        &theme,
        preview,
        title,
        width as f32,
        height as f32,
        dark,
    );

    let image = surface.image_snapshot();
    let data = image.encode(None, skia_safe::EncodedImageFormat::PNG, None)?;
    Some(data.as_bytes().to_vec())
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    canvas: &Canvas,
    theme: &Theme,
    preview: &Preview,
    title: &str,
    width: f32,
    height: f32,
    dark: bool,
) {
    // The desktop behind the card. A preview always floats over something, and
    // a flat backdrop would misrepresent how the material reads.
    canvas.clear(if dark {
        Color::from_argb(0xFF, 0x1C, 0x1C, 0x1E)
    } else {
        Color::from_argb(0xFF, 0xE8, 0xE8, 0xED)
    });

    let card = Rect::from_xywh(0.0, 0.0, width, height);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // The card itself. The compositor supplies the frost behind a real window;
    // here the material colour is composited over the backdrop directly, which
    // is the closest a screenshot can get to it.
    paint.set_color(preview::background(theme));
    canvas.draw_round_rect(card, RADIUS, RADIUS, &paint);

    // Header: the file's name, and a hairline under it.
    use otto_kit::common::Renderable;
    otto_kit::components::label::Label::new(title.to_string())
        .with_style(otto_kit::typography::styles::HEADLINE_EMPHASIZED)
        .with_color(theme.text_primary)
        .with_width(width)
        .with_align(otto_kit::components::label::TextAlign::Center)
        // Centred in the header band, not a hardcoded baseline: `HEADER * 0.62`
        // happened to look right at one font size and at no other.
        .centered_on(0.0, HEADER / 2.0)
        .render(canvas);

    paint.set_color(theme.fill_tertiary);
    canvas.draw_rect(Rect::from_xywh(0.0, HEADER - 1.0, width, 1.0), &paint);

    let content = Rect::from_ltrb(0.0, HEADER, width, height);
    canvas.save();
    canvas.clip_rect(content, None, true);
    // Fit, always: this is the offline renderer, and there is nobody here to
    // pinch.
    preview::draw(
        canvas,
        content,
        preview,
        theme,
        0,
        preview::Zoom::FIT,
        &resolve_icon,
    );
    canvas.restore();
}

/// Resolve a row's icon from the desktop icon theme.
///
/// `preview::draw` takes this as a callback so the toolkit's drawing half stays
/// free of the icon cache, and therefore of any runtime — the compositor
/// resolves icons differently from an application, and neither dependency
/// belongs in the draw path.
fn resolve_icon(name: &str, size: i32) -> Option<skia_safe::Image> {
    // `cached_icon_chain`, not `cached_file_icon` — the latter takes a *file
    // path*, and handing it an icon *name* silently resolves nothing. The chain
    // lookup also never reads `AppContext`, so it works from a bare canvas.
    otto_kit::icons::cached_icon_chain(&[name], size)
}

/// A filmstrip of the opening: the same card drawn at several points along the
/// entrance, over a mock desktop with the anchor marked.
///
/// The compositor runs the real transaction; this samples the same geometry
/// from [`crate::opening`], so it shows the motion's shape rather than a
/// screenshot of it.
pub fn opening_filmstrip(
    preview: &Preview,
    title: &str,
    frames: &[f32],
    dark: bool,
) -> Option<Vec<u8>> {
    use crate::opening;

    // A mock desktop with a file list down the left, so the anchor has
    // somewhere to be and the motion has something to be relative to.
    let (cell_w, cell_h) = (420.0f32, 300.0f32);
    let columns = frames.len().min(4) as f32;
    let rows = (frames.len() as f32 / columns).ceil();
    let width = (cell_w * columns) as i32;
    let height = (cell_h * rows) as i32;

    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut surface = skia_safe::surfaces::raster_n32_premul((width, height))?;
    let canvas = surface.canvas();
    canvas.clear(if dark {
        Color::from_argb(0xFF, 0x14, 0x14, 0x16)
    } else {
        Color::from_argb(0xFF, 0xDE, 0xDE, 0xE4)
    });

    // The item being previewed, and where the card comes to rest.
    let anchor = opening::Rect::new(24.0, 96.0, 150.0, 18.0);
    let resting = opening::Rect::new(96.0, 40.0, 300.0, 220.0);

    for (index, t) in frames.iter().enumerate() {
        let col = (index as f32) % columns;
        let row = (index as f32 / columns).floor();
        canvas.save();
        canvas.translate((col * cell_w, row * cell_h));
        canvas.clip_rect(Rect::from_xywh(0.0, 0.0, cell_w, cell_h), None, false);

        draw_mock_desktop(canvas, &theme, anchor, dark);

        let rect = opening::sample(anchor, resting, *t);
        let card = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);

        // The card is drawn once at its resting size and *transformed* — which
        // is what the compositor does with it, and why the entrance never
        // re-lays-out the content or asks for another frame.
        canvas.save();
        canvas.clip_rect(card, None, true);
        canvas.translate((card.left, card.top));
        let scale = rect.width / resting.width;
        canvas.scale((scale, scale));
        draw_card(
            canvas,
            &theme,
            preview,
            title,
            resting.width,
            resting.height,
            dark,
        );
        canvas.restore();

        let mut label = Paint::default();
        label.set_anti_alias(true);
        use otto_kit::common::Renderable;
        otto_kit::components::label::Label::new(format!("t = {t:.2}"))
            .with_style(otto_kit::typography::styles::CAPTION_1)
            .with_color(theme.text_tertiary)
            .at(24.0, cell_h - 18.0)
            .render(canvas);

        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image.encode(None, skia_safe::EncodedImageFormat::PNG, None)?;
    Some(data.as_bytes().to_vec())
}

/// A stand-in file list, with the previewed row highlighted — the thing the
/// card is growing out of.
fn draw_mock_desktop(canvas: &Canvas, theme: &Theme, anchor: crate::opening::Rect, dark: bool) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(if dark {
        Color::from_argb(0xFF, 0x24, 0x24, 0x28)
    } else {
        Color::from_argb(0xFF, 0xF6, 0xF6, 0xF8)
    });
    canvas.draw_round_rect(Rect::from_xywh(12.0, 24.0, 174.0, 250.0), 8.0, 8.0, &paint);

    for index in 0..9 {
        let y = 44.0 + index as f32 * 26.0;
        let selected = (y - anchor.y).abs() < 1.0;
        paint.set_color(if selected {
            theme.accent
        } else {
            theme.fill_tertiary
        });
        canvas.draw_round_rect(
            Rect::from_xywh(24.0, y, if selected { anchor.width } else { 120.0 }, 18.0),
            4.0,
            4.0,
            &paint,
        );
    }
}
