//! Drawing a file preview.
//!
//! The vocabulary a previewer produces, and the draw functions that paint it.
//! Deliberately split from whoever *decodes* the file: this half takes a
//! canvas, a rect, an already-decoded [`Preview`] and a theme, and paints. It
//! has no `AppContext`, no `wayland-client`, and no runtime — so the compositor
//! can draw a preview server-side (a file thumbnail in the dock) with none of
//! the client machinery, exactly as it already draws titlebars and menus.
//!
//! The set of [`Preview`] shapes is **closed on purpose**. A new content type
//! adds a decoder that produces one of these; it does not add a variant or a
//! drawing path. That is what keeps a PDF card and an audio card looking like
//! the same object rather than like two applications.
//!
//! State stays with the caller, per the toolkit's draw/hit-test convention:
//! [`layout`] computes the geometry once, [`draw`] paints it, and
//! [`PreviewLayout::row_at`] answers what is under a point. Both halves read
//! the same geometry, so they cannot drift.

use skia_safe::{Canvas, Color, Contains, Image, Paint, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

/// Decoded pixels: premultiplied RGBA, tightly packed.
#[derive(Debug, Clone)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    /// The source's true size, which is what zoom is measured against. Larger
    /// than `width`/`height` for a scaled decode.
    pub intrinsic_width: u32,
    pub intrinsic_height: u32,
    pub data: Vec<u8>,
}

impl Pixels {
    /// How far this decode can be zoomed before it starts inventing detail.
    pub fn native_scale(&self) -> f32 {
        if self.width == 0 {
            return 1.0;
        }
        self.intrinsic_width as f32 / self.width as f32
    }

    /// Wrap the buffer as a Skia image. Copies once, because the image outlives
    /// the borrow in every caller here.
    pub fn to_image(&self) -> Option<Image> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let info = skia_safe::ImageInfo::new(
            (self.width as i32, self.height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let row_bytes = self.width as usize * 4;
        // A buffer that contradicts its dimensions must not be wrapped: Skia
        // would read past the end of it.
        if self.data.len() != row_bytes * self.height as usize {
            return None;
        }
        skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&self.data), row_bytes)
    }
}

/// How an image preview is zoomed and panned.
///
/// `scale` multiplies the *fitted* size rather than the source's own, so 1.0
/// is always "the whole picture, as large as the box allows" whatever the
/// image and the box happen to be. `offset` then drags that blown-up rect
/// away from the centre it is otherwise pinned to, as far as the picture has
/// slack over its box and no further. `band` is the exception to that "no
/// further": a host pulling the picture past its stop puts the overshoot
/// there, where nothing clamps it.
///
/// Only [`Preview::Pixels`] honours it. Everything else is laid out to fit by
/// construction and [`clamp_zoom`] pins it back to [`Zoom::FIT`], so a host
/// can carry one zoom for whatever the previewer happened to return without
/// asking what that was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    /// Multiplier over the fitted size; 1.0 is fit.
    pub scale: f32,
    /// How far the zoomed content is dragged from centred, in the box's own
    /// pixels.
    pub offset: (f32, f32),
    /// How far the picture is pulled *past* its stop, in the same pixels —
    /// the rubber band, which a host holds only while a gesture is stretching
    /// it or a spring is bringing it home.
    ///
    /// Kept apart from `offset` rather than folded into it because the two
    /// answer different questions: `offset` is where the picture is allowed
    /// to be, and is clamped on every call, while this is a temporary
    /// displacement that is deliberately not. Everything that reads a `Zoom`
    /// to decide what may happen next — how far there is left to pan,
    /// whether the picture is at fit — reads `offset` and is unaffected;
    /// only the drawing adds this in.
    pub band: (f32, f32),
}

impl Default for Zoom {
    fn default() -> Self {
        Self::FIT
    }
}

impl Zoom {
    /// The whole picture, centred: where every preview starts.
    pub const FIT: Zoom = Zoom {
        scale: 1.0,
        offset: (0.0, 0.0),
        band: (0.0, 0.0),
    };

    /// The furthest in a preview goes. Past roughly this a decode meant for a
    /// panel is only showing its own resampling, so more zoom buys nothing.
    pub const MAX: f32 = 8.0;

    /// Below this, a zoom is snapped back to fit. A pinch that ends a hair
    /// above 1.0 leaves an image very slightly larger than its box, which
    /// pans by a pixel or two and never quite looks centred; there is no
    /// useful state between "fit" and "visibly zoomed in".
    pub const SNAP: f32 = 1.02;

    /// Whether this is the resting state — nothing to pan, nothing to reset.
    pub fn is_fit(&self) -> bool {
        *self == Self::FIT
    }
}

/// One row of a listing — an archive entry or a directory child.
#[derive(Debug, Clone, Default)]
pub struct Row {
    pub name: String,
    pub size: u64,
    /// Seconds since the epoch; 0 when the source carries no date.
    pub mtime: i64,
    /// Icon-theme name, resolved by whoever draws.
    pub icon: String,
    pub is_dir: bool,
}

/// A key/value fact on a metadata card.
#[derive(Debug, Clone)]
pub struct Fact {
    pub key: String,
    pub value: String,
}

/// What a previewer produced.
#[derive(Debug, Clone)]
pub enum Preview {
    /// An image, a rendered page, a poster frame.
    Pixels {
        pixels: Pixels,
        /// Total pages for paginated content; 1 for a plain image.
        pages: u32,
        /// Which page `pixels` holds, 1-based.
        page: u32,
    },
    /// Text, already validated as UTF-8 and bounded by the decoder.
    Text {
        lines: Vec<String>,
        truncated: bool,
        /// Carried for a future highlighter, so adding one is not a wire change.
        language: String,
    },
    /// A listing: archive entries, directory children.
    Rows {
        rows: Vec<Row>,
        truncated: bool,
        summary: String,
    },
    /// Everything described rather than rendered.
    Card {
        title: String,
        subtitle: String,
        facts: Vec<Fact>,
        hero: Option<Pixels>,
    },
    /// Nothing could be shown, and why.
    Unavailable { reason: String },
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

pub const PADDING: f32 = 20.0;

/// The mount around a picture, which is narrower than the gutter text gets.
///
/// Text and listings need room to breathe: a line that runs to the edge of
/// its card is hard to read and looks like it has been cut off. A photograph
/// is not read, it is looked at — the space around it is a frame, and a wide
/// frame on a preview that is already only part of the window wastes the part
/// the user came to see.
pub const IMAGE_PADDING: f32 = 8.0;

/// The padding this preview is laid out with.
fn padding_for(preview: &Preview) -> f32 {
    match preview {
        Preview::Pixels { .. } => IMAGE_PADDING,
        _ => PADDING,
    }
}
pub const ROW_HEIGHT: f32 = 26.0;
pub const LINE_HEIGHT: f32 = 17.0;
/// Width reserved for a row's size column.
pub const SIZE_COLUMN: f32 = 84.0;
/// Gutter width for line numbers.
pub const GUTTER: f32 = 46.0;
pub const HERO: f32 = 128.0;
/// Band heights on a metadata card. Declared rather than inlined so drawing
/// centres text in the same box the layout advances by.
pub const TITLE_BAND: f32 = 34.0;
pub const SUBTITLE_BAND: f32 = 28.0;
pub const FACT_BAND: f32 = 22.0;
pub const CORNER_RADIUS: f32 = 8.0;

/// Geometry shared by drawing and hit-testing.
#[derive(Debug, Clone)]
pub struct PreviewLayout {
    /// The whole content area.
    pub bounds: Rect,
    /// Where the content actually landed. For an image this is the fitted
    /// rectangle after the zoom has been applied to it; for a listing it is
    /// the table.
    pub content: Rect,
    /// The box the content is laid out inside: `bounds` less the padding.
    /// A zoomed image is clamped and clipped to this, so panning stops with
    /// the picture still covering it rather than sliding off the panel.
    pub inner: Rect,
    /// Where an image lands at [`Zoom::FIT`], whatever the zoom in force.
    /// Zoom is measured against this, so a gesture means the same thing on a
    /// wide picture and a tall one.
    pub fit: Rect,
    /// One rect per visible row, for listings. Empty otherwise.
    pub row_rects: Vec<Rect>,
    /// How many rows fit, whether or not that many exist.
    pub visible_rows: usize,
}

impl PreviewLayout {
    /// Which row is under a point, if any. The caller decides what that means.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.row_rects
            .iter()
            .position(|rect| rect.contains(skia_safe::Point::new(x, y)))
    }
}

/// The box the content is laid out inside: the bounds, less the padding that
/// keeps content off the edge of whatever card it is drawn on.
fn inner_of(bounds: Rect, preview: &Preview) -> Rect {
    let padding = padding_for(preview);
    Rect::from_ltrb(
        bounds.left + padding,
        bounds.top + padding,
        bounds.right - padding,
        bounds.bottom - padding,
    )
}

/// Bring a requested zoom into range for this preview in this box.
///
/// Two things are clamped, and both are geometry rather than policy, which is
/// why they live here beside the layout rather than in whatever host is
/// holding the gesture. The scale is held between fit and [`Zoom::MAX`], with
/// the low end snapped back to exactly fit. The offset is then held to the
/// slack the zoomed picture actually has over its box, so the image can never
/// be dragged so far that its own edge comes inside the frame — and a picture
/// with no slack, which is every picture at fit, cannot be panned at all.
///
/// Idempotent, and safe to call on a zoom that is already in range: the host
/// stores what this returns and drawing re-clamps it anyway, because the box
/// changes size underneath a stored zoom whenever the window is resized.
pub fn clamp_zoom(bounds: Rect, preview: &Preview, zoom: Zoom) -> Zoom {
    let Preview::Pixels { pixels, .. } = preview else {
        // Text, listings and cards are laid out to fit by construction. A
        // zoom on one is not clamped, it is refused.
        return Zoom::FIT;
    };
    let inner = inner_of(bounds, preview);
    let fitted = fit(inner, pixels.width as f32, pixels.height as f32);
    let scale = if zoom.scale <= Zoom::SNAP {
        1.0
    } else {
        zoom.scale.min(Zoom::MAX)
    };
    let slack_x = ((fitted.width() * scale - inner.width()) / 2.0).max(0.0);
    let slack_y = ((fitted.height() * scale - inner.height()) / 2.0).max(0.0);
    Zoom {
        scale,
        offset: (
            zoom.offset.0.clamp(-slack_x, slack_x),
            zoom.offset.1.clamp(-slack_y, slack_y),
        ),
        // Carried through untouched: the band is the one part of a zoom that
        // is meant to be out of range, and a host that is not banding has it
        // at zero anyway. A picture snapped back to fit has nothing to be
        // stretched past, though, so that case drops it.
        band: if scale == 1.0 { (0.0, 0.0) } else { zoom.band },
    }
}

/// The zoom that leaves the pixel under `focus` where it is while the scale
/// changes to `scale`.
///
/// This is what makes a pinch feel like it is grabbing the picture rather
/// than the panel: the point between the fingers is the one fixed point of
/// the transform. `focus` is in the same coordinates as `bounds`. The result
/// is already clamped, so a pinch that runs past either end of the range
/// simply stops moving instead of sliding the image away under the fingers.
pub fn zoom_about(
    bounds: Rect,
    preview: &Preview,
    current: Zoom,
    scale: f32,
    focus: (f32, f32),
) -> Zoom {
    let current = clamp_zoom(bounds, preview, current);
    // Clamp the target first: the ratio below has to be the scale change that
    // will actually be drawn, or the offset compensates for a zoom that never
    // happens and the picture drifts at the ends of the range.
    let target = clamp_zoom(bounds, preview, Zoom { scale, ..current });
    let inner = inner_of(bounds, preview);
    let ratio = target.scale / current.scale;
    // Where the content's centre is now, and where scaling about the focal
    // point sends it.
    let centre = (
        inner.center_x() + current.offset.0,
        inner.center_y() + current.offset.1,
    );
    let moved = (
        focus.0 + (centre.0 - focus.0) * ratio,
        focus.1 + (centre.1 - focus.1) * ratio,
    );
    clamp_zoom(
        bounds,
        preview,
        Zoom {
            scale: target.scale,
            offset: (moved.0 - inner.center_x(), moved.1 - inner.center_y()),
            // A pinch places the picture outright, so whatever it was being
            // stretched by is over.
            band: (0.0, 0.0),
        },
    )
}

/// The fitted rect blown up by `zoom` about the box's centre and dragged by
/// the zoom's offset.
fn zoomed(inner: Rect, fitted: Rect, zoom: Zoom) -> Rect {
    let (w, h) = (fitted.width() * zoom.scale, fitted.height() * zoom.scale);
    let cx = inner.center_x() + zoom.offset.0 + zoom.band.0;
    let cy = inner.center_y() + zoom.offset.1 + zoom.band.1;
    Rect::from_ltrb(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
}

/// Where the content will be drawn, computed without drawing it.
///
/// `zoom` applies to images only; pass [`Zoom::FIT`] for a host that does not
/// offer zooming. It is clamped here rather than trusted, because the box a
/// stored zoom was clamped against is not the box it is drawn in after a
/// resize.
pub fn layout(bounds: Rect, preview: &Preview, first_row: usize, zoom: Zoom) -> PreviewLayout {
    let inner = inner_of(bounds, preview);

    match preview {
        Preview::Pixels { pixels, .. } => {
            let fitted = fit(inner, pixels.width as f32, pixels.height as f32);
            let zoom = clamp_zoom(bounds, preview, zoom);
            PreviewLayout {
                bounds,
                content: zoomed(inner, fitted, zoom),
                inner,
                fit: fitted,
                row_rects: Vec::new(),
                visible_rows: 0,
            }
        }
        Preview::Rows { rows, .. } => {
            let visible = ((inner.height() / ROW_HEIGHT).floor().max(0.0)) as usize;
            let shown = visible.min(rows.len().saturating_sub(first_row));
            let row_rects = (0..shown)
                .map(|index| {
                    let top = inner.top + index as f32 * ROW_HEIGHT;
                    Rect::from_ltrb(inner.left, top, inner.right, top + ROW_HEIGHT)
                })
                .collect();
            PreviewLayout {
                bounds,
                content: inner,
                inner,
                fit: inner,
                row_rects,
                visible_rows: visible,
            }
        }
        Preview::Text { .. } => PreviewLayout {
            bounds,
            content: inner,
            inner,
            fit: inner,
            row_rects: Vec::new(),
            visible_rows: ((inner.height() / LINE_HEIGHT).floor().max(0.0)) as usize,
        },
        Preview::Card { .. } | Preview::Unavailable { .. } => PreviewLayout {
            bounds,
            content: inner,
            inner,
            fit: inner,
            row_rects: Vec::new(),
            visible_rows: 0,
        },
    }
}

/// The largest rect of the content's aspect ratio that fits inside `bounds`,
/// centred. Content smaller than the box is centred rather than upscaled —
/// blowing up a 16×16 icon to fill a window makes it look broken.
fn fit(bounds: Rect, width: f32, height: f32) -> Rect {
    if width <= 0.0 || height <= 0.0 {
        return bounds;
    }
    let scale = (bounds.width() / width)
        .min(bounds.height() / height)
        .min(1.0);
    let (w, h) = (width * scale, height * scale);
    let cx = bounds.center_x();
    let cy = bounds.center_y();
    Rect::from_ltrb(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Paint a preview into `bounds`.
///
/// `first_row` scrolls listings and text; the caller owns that offset, as it
/// owns every other piece of interaction state.
///
/// `zoom` magnifies an image and drags it about; it is ignored by everything
/// else. A host with no zoom gesture passes [`Zoom::FIT`] and gets exactly
/// what it got before there was one.
///
/// `resolve_icon` turns a row's icon name into an image. It is a callback
/// rather than a direct call so this module stays free of the icon cache and
/// therefore of any runtime — the compositor and an application resolve icons
/// differently, and neither dependency belongs here.
pub fn draw(
    canvas: &Canvas,
    bounds: Rect,
    preview: &Preview,
    theme: &Theme,
    first_row: usize,
    zoom: Zoom,
    resolve_icon: &dyn Fn(&str, i32) -> Option<Image>,
) {
    let geometry = layout(bounds, preview, first_row, zoom);
    match preview {
        Preview::Pixels { pixels, .. } => draw_pixels(canvas, &geometry, pixels, theme),
        Preview::Text { lines, .. } => draw_text(canvas, &geometry, lines, first_row, theme),
        Preview::Rows { rows, .. } => {
            draw_rows(canvas, &geometry, rows, first_row, theme, resolve_icon)
        }
        Preview::Card {
            title,
            subtitle,
            facts,
            hero,
        } => draw_card(
            canvas,
            &geometry,
            title,
            subtitle,
            facts,
            hero.as_ref(),
            theme,
        ),
        Preview::Unavailable { reason } => draw_unavailable(canvas, &geometry, reason, theme),
    }
}

fn draw_pixels(canvas: &Canvas, geometry: &PreviewLayout, pixels: &Pixels, theme: &Theme) {
    let Some(image) = pixels.to_image() else {
        return draw_unavailable(canvas, geometry, "this image could not be shown", theme);
    };

    // Zoomed in, the content runs past the box it is laid out in; the clip is
    // what turns that box into a window onto the picture rather than letting
    // it spill over the padding and, on a panel, over the card's own corners.
    canvas.save();
    canvas.clip_rect(geometry.inner, None, true);

    // A checkerboard under anything with alpha, so a transparent PNG reads as
    // transparent rather than as the theme's background colour.
    draw_checkerboard(canvas, geometry.content, theme);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    canvas.draw_image_rect_with_sampling_options(
        &image,
        None,
        geometry.content,
        // Mitchell: previews are almost always downscaled, and a box filter
        // makes fine detail shimmer.
        skia_safe::sampling_options::SamplingOptions::from(
            skia_safe::sampling_options::CubicResampler::mitchell(),
        ),
        &paint,
    );
    canvas.restore();
}

/// The transparency checkerboard, drawn only where the content will land.
fn draw_checkerboard(canvas: &Canvas, rect: Rect, theme: &Theme) {
    const SQUARE: f32 = 8.0;
    canvas.save();
    canvas.clip_rect(rect, None, true);
    let mut paint = Paint::default();
    paint.set_color(theme.fill_quaternary);
    let mut y = rect.top;
    let mut row = 0;
    while y < rect.bottom {
        let mut x = rect.left + if row % 2 == 0 { 0.0 } else { SQUARE };
        while x < rect.right {
            canvas.draw_rect(Rect::from_xywh(x, y, SQUARE, SQUARE), &paint);
            x += SQUARE * 2.0;
        }
        y += SQUARE;
        row += 1;
    }
    canvas.restore();
}

fn draw_text(
    canvas: &Canvas,
    geometry: &PreviewLayout,
    lines: &[String],
    first_row: usize,
    theme: &Theme,
) {
    canvas.save();
    canvas.clip_rect(geometry.content, None, false);

    let mut y = geometry.content.top;
    for (offset, line) in lines
        .iter()
        .skip(first_row)
        .take(geometry.visible_rows + 1)
        .enumerate()
    {
        let number = first_row + offset + 1;
        // Both halves are centred on the *same* line centre. `.at()` with a
        // shared y would not align them: it places the baseline at
        // `y + size * 0.8`, and the gutter and the code are different styles,
        // so the same y would put them on two different baselines. Sharing a
        // centre is the one thing that keeps a small number level with the code
        // beside it whatever either style becomes.
        let centre = y + LINE_HEIGHT / 2.0;

        // The gutter is right-aligned and dimmer than the code: it is
        // reference, not content.
        Label::new(number.to_string())
            .with_style(styles::CAPTION_1)
            .with_color(theme.text_tertiary)
            .with_width(GUTTER - 12.0)
            .with_align(crate::components::label::TextAlign::Right)
            .centered_on(geometry.content.left, centre)
            .render(canvas);

        Label::new(line.clone())
            .with_style(mono())
            .with_color(theme.text_primary)
            .centered_on(geometry.content.left + GUTTER, centre)
            .render(canvas);
        y += LINE_HEIGHT;
    }
    canvas.restore();
}

fn draw_rows(
    canvas: &Canvas,
    geometry: &PreviewLayout,
    rows: &[Row],
    first_row: usize,
    theme: &Theme,
    resolve_icon: &dyn Fn(&str, i32) -> Option<Image>,
) {
    // Two paints, deliberately. A paint's alpha applies to images as well as to
    // fills, so painting the alternating bands with the same paint used for the
    // icons drags every icon after the first band down to the band's alpha and
    // they vanish. Keeping them separate is the fix; sharing one and resetting
    // the colour each time would work until someone added another fill.
    let mut band = Paint::default();
    band.set_anti_alias(true);
    let mut image_paint = Paint::default();
    image_paint.set_anti_alias(true);

    for (index, rect) in geometry.row_rects.iter().enumerate() {
        let Some(row) = rows.get(first_row + index) else {
            break;
        };
        // Alternating bands: a long listing is much easier to read across.
        if index % 2 == 1 {
            band.set_color(theme.fill_quaternary);
            canvas.draw_rect(*rect, &band);
        }

        let icon_size = 16;
        let mut text_left = rect.left;
        if let Some(icon) = resolve_icon(&row.icon, icon_size) {
            let top = rect.top + (ROW_HEIGHT - icon_size as f32) / 2.0;
            canvas.draw_image_rect(
                &icon,
                None,
                Rect::from_xywh(rect.left, top, icon_size as f32, icon_size as f32),
                &image_paint,
            );
            text_left += icon_size as f32 + 8.0;
        }

        let cy = rect.center_y();
        Label::new(row.name.clone())
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(text_left, cy)
            .render(canvas);

        if !row.is_dir && row.size > 0 {
            Label::new(human_size(row.size))
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_secondary)
                .with_width(SIZE_COLUMN)
                .with_align(crate::components::label::TextAlign::Right)
                .centered_on(rect.right - SIZE_COLUMN, cy)
                .render(canvas);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    canvas: &Canvas,
    geometry: &PreviewLayout,
    title: &str,
    subtitle: &str,
    facts: &[Fact],
    hero: Option<&Pixels>,
    theme: &Theme,
) {
    let inner = geometry.content;
    let mut y = inner.top;

    // Artwork, when there is any: cover art, an embedded thumbnail, a poster.
    if let Some(image) = hero.and_then(|hero| hero.to_image()) {
        let rect = fit(
            Rect::from_xywh(inner.center_x() - HERO / 2.0, y, HERO, HERO),
            image.width() as f32,
            image.height() as f32,
        );
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        canvas.draw_image_rect_with_sampling_options(
            &image,
            None,
            rect,
            skia_safe::sampling_options::SamplingOptions::from(
                skia_safe::sampling_options::CubicResampler::mitchell(),
            ),
            &paint,
        );
        y = rect.bottom + PADDING;
    }

    Label::new(title.to_string())
        .with_style(styles::TITLE_3_EMPHASIZED)
        .with_color(theme.text_primary)
        .with_width(inner.width())
        .with_align(crate::components::label::TextAlign::Center)
        // `centered_on`, not `.at(y + constant)`: a constant nudge only lines
        // up at one font size, and ascent and descent are not symmetric about
        // the baseline, so a proportional one is wrong too. Each block below
        // declares its band and is centred in it.
        .centered_on(inner.left, y + TITLE_BAND / 2.0)
        .render(canvas);
    y += TITLE_BAND;

    if !subtitle.is_empty() {
        Label::new(subtitle.to_string())
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .with_width(inner.width())
            .with_align(crate::components::label::TextAlign::Center)
            .centered_on(inner.left, y + SUBTITLE_BAND / 2.0)
            .render(canvas);
        y += SUBTITLE_BAND;
    }

    y += PADDING;
    // Facts as a two-column table, keys right-aligned against the centre so
    // the values line up as one column.
    let split = inner.center_x();
    for fact in facts {
        Label::new(fact.key.clone())
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_secondary)
            .with_width(120.0)
            .with_align(crate::components::label::TextAlign::Right)
            .centered_on(split - 132.0, y + FACT_BAND / 2.0)
            .render(canvas);
        Label::new(fact.value.clone())
            .with_style(styles::FOOTNOTE_EMPHASIZED)
            .with_color(theme.text_primary)
            // The key and the value are different styles in the same row, which
            // is exactly the case a shared constant cannot get right for both.
            .centered_on(split + 12.0, y + FACT_BAND / 2.0)
            .render(canvas);
        y += FACT_BAND;
    }
}

fn draw_unavailable(canvas: &Canvas, geometry: &PreviewLayout, reason: &str, theme: &Theme) {
    let inner = geometry.content;
    Label::new(reason.to_string())
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .with_width(inner.width())
        .with_align(crate::components::label::TextAlign::Center)
        .centered_on(inner.left, inner.center_y())
        .render(canvas);
}

/// A monospaced style for code. Derived from the body style so it tracks the
/// theme's sizing rather than hardcoding a second scale.
fn mono() -> TextStyle {
    let mut style = styles::FOOTNOTE;
    style.family = "monospace";
    style
}

/// Human-readable byte count.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The material a preview sits on. Exposed so the compositor and an
/// application can paint the same ground.
pub fn background(theme: &Theme) -> Color {
    theme.material_popup
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32) -> Pixels {
        Pixels {
            width,
            height,
            intrinsic_width: width,
            intrinsic_height: height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    #[test]
    fn small_content_is_centred_rather_than_upscaled() {
        let box_rect = Rect::from_xywh(0.0, 0.0, 400.0, 400.0);
        let fitted = fit(box_rect, 16.0, 16.0);
        assert_eq!(fitted.width(), 16.0);
        assert_eq!(fitted.center_x(), 200.0);
    }

    #[test]
    fn large_content_fits_inside_and_keeps_its_aspect() {
        let fitted = fit(Rect::from_xywh(0.0, 0.0, 100.0, 100.0), 400.0, 200.0);
        assert_eq!(fitted.width(), 100.0);
        assert_eq!(fitted.height(), 50.0);
    }

    #[test]
    fn a_buffer_that_contradicts_its_size_is_refused() {
        let mut bad = pixels(4, 4);
        bad.data.truncate(8);
        assert!(bad.to_image().is_none(), "must not wrap a short buffer");
    }

    #[test]
    fn row_hit_testing_matches_the_drawn_rows() {
        let preview = Preview::Rows {
            rows: vec![Row::default(); 10],
            truncated: false,
            summary: String::new(),
        };
        let bounds = Rect::from_xywh(0.0, 0.0, 300.0, PADDING * 2.0 + ROW_HEIGHT * 4.0);
        let geometry = layout(bounds, &preview, 0, Zoom::FIT);
        assert_eq!(geometry.row_rects.len(), 4);
        // A point inside the second row must report the second row.
        let second = geometry.row_rects[1];
        assert_eq!(
            geometry.row_at(second.left + 1.0, second.center_y()),
            Some(1)
        );
        assert_eq!(
            geometry.row_at(second.left + 1.0, bounds.bottom + 10.0),
            None
        );
    }

    #[test]
    fn scrolling_a_listing_shows_fewer_rows_at_the_end() {
        let preview = Preview::Rows {
            rows: vec![Row::default(); 6],
            truncated: false,
            summary: String::new(),
        };
        let bounds = Rect::from_xywh(0.0, 0.0, 300.0, PADDING * 2.0 + ROW_HEIGHT * 4.0);
        // Four fit, but only two remain past row four.
        assert_eq!(layout(bounds, &preview, 4, Zoom::FIT).row_rects.len(), 2);
    }

    /// A box whose inner area is exactly the 400x200 picture below, so the
    /// fit rect is the picture's own size and the arithmetic in the zoom
    /// tests is easy to follow. Sized off the *image* padding, since that is
    /// the one a picture is laid out with.
    fn image_box() -> Rect {
        Rect::from_xywh(
            0.0,
            0.0,
            400.0 + IMAGE_PADDING * 2.0,
            200.0 + IMAGE_PADDING * 2.0,
        )
    }

    fn image() -> Preview {
        Preview::Pixels {
            pixels: pixels(400, 200),
            pages: 1,
            page: 1,
        }
    }

    #[test]
    fn zoom_is_held_between_fit_and_the_maximum() {
        let bounds = image_box();
        let too_far = clamp_zoom(
            bounds,
            &image(),
            Zoom {
                scale: 40.0,
                ..Zoom::FIT
            },
        );
        assert_eq!(too_far.scale, Zoom::MAX);

        // Anything at or under the snap threshold is fit, exactly.
        let nearly = clamp_zoom(
            bounds,
            &image(),
            Zoom {
                scale: 1.01,
                offset: (30.0, 0.0),
                ..Zoom::FIT
            },
        );
        assert_eq!(nearly, Zoom::FIT);
    }

    #[test]
    fn a_fitted_image_cannot_be_panned() {
        let clamped = clamp_zoom(
            image_box(),
            &image(),
            Zoom {
                scale: 1.0,
                offset: (120.0, -80.0),
                ..Zoom::FIT
            },
        );
        assert_eq!(clamped.offset, (0.0, 0.0));
    }

    /// The picture may be dragged until its edge reaches the edge of the box
    /// and no further, so it always covers the frame it is being looked at
    /// through.
    #[test]
    fn a_zoomed_image_cannot_be_dragged_off_its_box() {
        let bounds = image_box();
        let clamped = clamp_zoom(
            bounds,
            &image(),
            Zoom {
                scale: 2.0,
                offset: (10_000.0, 10_000.0),
                ..Zoom::FIT
            },
        );
        // Twice 400 wide in a 400-wide box leaves 200 of slack sideways, and
        // twice 200 tall in a 200-tall box leaves 100 up and down.
        assert_eq!(clamped.offset, (200.0, 100.0));

        let drawn = layout(bounds, &image(), 0, clamped).content;
        let inner = inner_of(bounds, &image());
        assert!(drawn.left <= inner.left, "{drawn:?}");
        assert!(drawn.bottom >= inner.bottom, "{drawn:?}");
    }

    /// The band is the one displacement the clamp leaves alone: it is what a
    /// host stretches the picture by while a gesture pulls past its stop, and
    /// it draws exactly that far past it.
    #[test]
    fn a_banded_zoom_draws_past_the_stop() {
        let bounds = image_box();
        let stopped = clamp_zoom(
            bounds,
            &image(),
            Zoom {
                scale: 2.0,
                offset: (10_000.0, 0.0),
                ..Zoom::FIT
            },
        );
        let banded = clamp_zoom(
            bounds,
            &image(),
            Zoom {
                band: (30.0, 0.0),
                ..stopped
            },
        );
        // The pan itself is still at its stop — only the stretch is new.
        assert_eq!(banded.offset, stopped.offset);

        let held = layout(bounds, &image(), 0, stopped).content;
        let stretched = layout(bounds, &image(), 0, banded).content;
        assert!(
            (stretched.left - held.left - 30.0).abs() < 0.01,
            "{stretched:?} {held:?}"
        );

        // A picture snapped back to fit has nothing left to be stretched past.
        let refitted = clamp_zoom(
            bounds,
            &image(),
            Zoom {
                scale: 1.0,
                band: (30.0, 0.0),
                ..Zoom::FIT
            },
        );
        assert!(refitted.is_fit(), "{refitted:?}");
    }

    #[test]
    fn only_images_zoom() {
        let text = Preview::Text {
            lines: vec!["one".into()],
            truncated: false,
            language: String::new(),
        };
        let asked = Zoom {
            scale: 4.0,
            offset: (12.0, 12.0),
            ..Zoom::FIT
        };
        assert!(clamp_zoom(image_box(), &text, asked).is_fit());
        assert_eq!(
            layout(image_box(), &text, 0, asked).content,
            inner_of(image_box(), &text)
        );
    }

    /// The pixel under the fingers is the one that does not move.
    #[test]
    fn pinching_keeps_the_focal_point_still() {
        let bounds = image_box();
        let preview = image();
        let inner = inner_of(bounds, &preview);
        // Off-centre, and near enough the middle that the clamp does not
        // truncate the offset the focal point asks for.
        let focus = (inner.center_x() + 40.0, inner.center_y() + 10.0);

        let before = layout(bounds, &preview, 0, Zoom::FIT).content;
        let zoom = zoom_about(bounds, &preview, Zoom::FIT, 2.0, focus);
        let after = layout(bounds, &preview, 0, zoom).content;

        // Where the focal point sat in the picture, as a fraction of it.
        let fx = |rect: Rect| (focus.0 - rect.left) / rect.width();
        let fy = |rect: Rect| (focus.1 - rect.top) / rect.height();
        assert!(
            (fx(before) - fx(after)).abs() < 1e-3,
            "{before:?} {after:?}"
        );
        assert!(
            (fy(before) - fy(after)).abs() < 1e-3,
            "{before:?} {after:?}"
        );
    }

    #[test]
    fn native_scale_reports_the_headroom_a_scaled_decode_left() {
        let scaled = Pixels {
            width: 500,
            height: 500,
            intrinsic_width: 2000,
            intrinsic_height: 2000,
            data: vec![0; 500 * 500 * 4],
        };
        assert_eq!(scaled.native_scale(), 4.0);
    }
}
