//! A place to drop files while a drag is going on and there is no card yet
//! (`otto-canvas-v1`, version 4).
//!
//! Otto tells every client when a drag starts. When it carries files, or
//! does not say what it carries, and no stash is on the card, a small card
//! inviting the drop goes into the side canvas, where the card would sit.
//! The canvas opens when the drag rests at the right edge of the screen. A
//! drop on the invitation starts a stash with the files, and the card takes
//! its place once drawn; otherwise it leaves when the drag ends.

// Rust guideline compliant 2026-02-21

use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wayland_client::protocol::{wl_compositor::WlCompositor, wl_shm, wl_surface::WlSurface};
use wayland_client::{Proxy, QueueHandle};

use otto_kit::color_scheme::current_color_scheme;
use otto_kit::protocols::{
    otto_canvas_item_v1::{KeyboardInteractivity, OttoCanvasItemV1},
    otto_canvas_manager_v1::OttoCanvasManagerV1,
    otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1,
    otto_surface_style_v1::OttoSurfaceStyleV1,
};
use otto_kit::skia::font_style::{Slant, Weight, Width};
use otto_kit::skia::textlayout::{
    FontCollection, ParagraphBuilder, ParagraphStyle, TextAlign, TextStyle,
};
use otto_kit::skia::{
    surfaces, AlphaType, Color, ColorType, FontMgr, FontStyle, ImageInfo, Paint, PaintStyle, RRect,
    Rect,
};
use otto_kit::theme::Theme;
use otto_kit::typography::styles;

use crate::State;

/// The invitation's height, in logical pixels.
const HEIGHT: f32 = 72.0;
/// Space inside the dashed outline, in logical pixels.
const INSET: f32 = 10.0;
const TEXT_SIZE: f32 = 13.0;
const LINE_H: f32 = 18.0;

/// Whether a drag offering `mime_types` may carry files. An empty list is
/// a source that has not said yet, which may.
pub fn may_carry_files(mime_types: &[String]) -> bool {
    mime_types.is_empty()
        || mime_types
            .iter()
            .any(|mime_type| mime_type == otto_kit::clipboard::URI_LIST)
}

/// The invitation in the side canvas, for the length of one drag.
#[derive(Debug)]
pub struct Invite {
    item: OttoCanvasItemV1,
    surface: WlSurface,
    style: Option<OttoSurfaceStyleV1>,
    buffer: Option<Buffer>,
    /// The column's width in logical pixels; zero until configured.
    width: i32,
    scale: i32,
    /// A drag carrying files is over it.
    hovered: bool,
    /// Files were dropped on it; it stays until the card replaces it.
    dropped: bool,
}

impl Invite {
    /// Put the invitation in the canvas, where the card sits.
    pub fn new(
        manager: &OttoCanvasManagerV1,
        compositor: &WlCompositor,
        styles: Option<&OttoSurfaceStyleManagerV1>,
        qh: &QueueHandle<State>,
        scale: i32,
    ) -> Self {
        let surface = compositor.create_surface(qh, ());
        surface.set_buffer_scale(scale);
        let item = manager.get_canvas_item(&surface, qh, ());
        item.set_keyboard_interactivity(KeyboardInteractivity::Never);
        item.set_order(crate::canvas::ORDER);
        // Always this tall, and short enough that its share is all of it.
        if item.version() >= crate::canvas::SHARE_VERSION {
            item.set_content_height(HEIGHT as u32);
        }
        let style = styles.map(|manager| {
            let style = manager.get_surface_style(&surface, qh, ());
            crate::panel::apply_material(&style);
            style
        });
        Self {
            item,
            surface,
            style,
            buffer: None,
            width: 0,
            scale,
            hovered: false,
            dropped: false,
        }
    }

    /// Whether `item` is the invitation's.
    pub fn is(&self, item: &OttoCanvasItemV1) -> bool {
        self.item == *item
    }

    /// The surface drops arrive on.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// Whether files were dropped on it.
    pub fn dropped(&self) -> bool {
        self.dropped
    }

    /// Files were dropped on it.
    pub fn set_dropped(&mut self) {
        self.dropped = true;
        self.hovered = false;
    }

    /// Acknowledge the column's width and draw at it.
    pub fn configure(&mut self, serial: u32, width: u32, pool: &mut SlotPool) {
        self.item.ack_configure(serial);
        self.width = i32::try_from(width).unwrap_or(i32::MAX);
        self.draw(pool);
    }

    /// A drag carrying files came over it (`true`) or left (`false`).
    pub fn set_hovered(&mut self, hovered: bool, pool: &mut SlotPool) {
        if self.hovered != hovered {
            self.hovered = hovered;
            self.draw(pool);
        }
    }

    /// Draw the invitation, once the compositor has sized it.
    fn draw(&mut self, pool: &mut SlotPool) {
        if self.width <= 0 {
            return;
        }
        let scale = self.scale as f32;
        let w = (self.width as f32 * scale) as i32;
        let h = (HEIGHT * scale) as i32;
        let reusable = self.buffer.as_mut().is_some_and(|buffer| {
            buffer.height() == h && buffer.stride() == w * 4 && buffer.canvas(pool).is_some()
        });
        if !reusable {
            match pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) {
                Ok((buffer, _)) => self.buffer = Some(buffer),
                Err(error) => {
                    tracing::warn!(%error, "no buffer for the drop invitation");
                    return;
                }
            }
        }
        let Some(buffer) = self.buffer.as_mut() else {
            return;
        };
        let Some(pixels) = buffer.canvas(pool) else {
            return;
        };
        paint(
            pixels,
            (w, h),
            scale,
            self.width as f32,
            self.hovered,
            self.style.is_some(),
        );
        if buffer.attach_to(&self.surface).is_ok() {
            self.surface.damage_buffer(0, 0, w, h);
            self.surface.commit();
        }
    }

    /// Take the invitation out of the canvas.
    pub fn destroy(self) {
        if let Some(style) = self.style {
            style.destroy();
        }
        self.item.destroy();
        self.surface.destroy();
    }
}

/// Paint the invitation into `pixels`, an ARGB8888 buffer of `size` pixels
/// at `scale` pixels per logical pixel, `width` logical pixels wide.
fn paint(
    pixels: &mut [u8],
    size: (i32, i32),
    scale: f32,
    width: f32,
    hovered: bool,
    frosted: bool,
) {
    let info = ImageInfo::new(size, ColorType::BGRA8888, AlphaType::Premul, None);
    let Some(mut surface) = surfaces::wrap_pixels(&info, pixels, None, None) else {
        tracing::warn!("cannot draw into the drop invitation's buffer");
        return;
    };
    let theme = Theme::for_scheme(current_color_scheme());
    let canvas = surface.canvas();
    canvas.clear(Color::TRANSPARENT);
    canvas.scale((scale, scale));

    let card = Rect::from_xywh(0.0, 0.0, width, HEIGHT);
    let radius = otto_kit::corners::radius(crate::balloon::RADIUS);
    if !frosted {
        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(theme.material_popup.with_a(0xF6));
        canvas.draw_rrect(RRect::new_rect_xy(card, radius, radius), &fill);
    }

    // A dashed outline marks where to drop; it takes the accent while a
    // drag is over it.
    let outline = card.with_inset((INSET, INSET));
    let inner_radius = (radius - INSET).max(4.0);
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_color(if hovered {
        theme.accent
    } else {
        theme.text_secondary
    });
    stroke.set_path_effect(otto_kit::skia::PathEffect::dash(&[6.0, 4.0], 0.0));
    canvas.draw_rrect(
        RRect::new_rect_xy(outline, inner_radius, inner_radius),
        &stroke,
    );

    let mut fonts = FontCollection::new();
    fonts.set_default_font_manager(FontMgr::new(), None);
    let mut text = TextStyle::new();
    text.set_font_size(TEXT_SIZE);
    text.set_font_families(&[styles::BODY.family, "sans-serif"]);
    text.set_font_style(FontStyle::new(
        Weight::from(500),
        Width::NORMAL,
        Slant::Upright,
    ));
    text.set_color(if hovered {
        theme.accent
    } else {
        theme.text_secondary
    });
    text.set_height(LINE_H / TEXT_SIZE);
    text.set_height_override(true);
    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_max_lines(1);
    paragraph_style.set_ellipsis("…");
    paragraph_style.set_text_align(TextAlign::Center);
    let mut builder = ParagraphBuilder::new(&paragraph_style, &fonts);
    builder.push_style(&text);
    builder.add_text(otto_kit::t!("stash-drop-invite"));
    let mut paragraph = builder.build();
    let inner = (outline.width() - 2.0 * INSET).max(1.0);
    paragraph.layout(inner);
    paragraph.paint(canvas, (outline.left + INSET, (HEIGHT - LINE_H) / 2.0));
}

#[cfg(test)]
mod tests {
    use super::may_carry_files;

    #[test]
    fn files_or_an_unknown_drag_are_invited() {
        assert!(may_carry_files(&[]));
        assert!(may_carry_files(&[
            "text/plain".to_owned(),
            "text/uri-list".to_owned()
        ]));
        assert!(!may_carry_files(&["text/plain".to_owned()]));
    }
}
