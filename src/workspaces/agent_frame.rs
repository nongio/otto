//! The agent border: a frame in an agent's colour around a workspace it may
//! act on, with a chip naming the agent and offering Stop
//! (`specs/agent-seats.md`, The agent border).
//!
//! Each output has one container, the last child of its `windows_plane`, so
//! it scrolls with the workspaces and draws above their windows. In it, one
//! frame layer per granted workspace sits where that workspace's windows
//! layer does. Workspaces never overlap on the strip, so the container's
//! place among later workspaces does not matter.

use layers::{prelude::*, skia, types::Size};

use crate::config::Config;

/// Logical width of the glow inside the line.
const GLOW: f32 = 12.0;
/// Logical distance of the chip from the top edge.
const CHIP_TOP: f32 = 8.0;
const CHIP_HEIGHT: f32 = 26.0;
const CHIP_PAD: f32 = 12.0;
const CHIP_FONT: f32 = 13.0;
const STOP_LABEL: &str = "Stop";

/// How one frame looks. A change redraws it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentFrameLook {
    pub color: [u8; 3],
    /// The agents acting there, most recent first, for the chip.
    pub names: Vec<String>,
    /// No chip while a window is fullscreen on the output.
    pub chip: bool,
    /// 1 while the agent is active, lower once it is idle.
    pub strength: f32,
}

/// The chip's rectangle and the Stop control's, in logical coordinates
/// relative to the output's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipGeometry {
    pub chip: (f32, f32, f32, f32),
    pub stop: (f32, f32, f32, f32),
}

fn chip_font(scale: f32) -> skia::Font {
    let family = Config::with(|c| c.font_family.clone());
    let style = skia::FontStyle::new(
        skia::font_style::Weight::SEMI_BOLD,
        skia::font_style::Width::NORMAL,
        skia::font_style::Slant::Upright,
    );
    crate::workspaces::utils::FONT_CACHE
        .with(|fonts| fonts.make_font_with_fallback(family, style, CHIP_FONT * scale))
}

fn chip_text(names: &[String]) -> String {
    names.join(" · ")
}

/// Where the chip for `names` sits on an output `width` logical pixels wide.
pub fn chip_geometry(names: &[String], width: f32) -> ChipGeometry {
    let font = chip_font(1.0);
    let (name_width, _) = font.measure_str(chip_text(names), None);
    let (stop_width, _) = font.measure_str(STOP_LABEL, None);
    let stop = stop_width + CHIP_PAD * 2.0;
    let chip_width = CHIP_PAD + name_width + CHIP_PAD + stop;
    let x = (width - chip_width) / 2.0;
    ChipGeometry {
        chip: (x, CHIP_TOP, chip_width, CHIP_HEIGHT),
        stop: (x + chip_width - stop, CHIP_TOP, stop, CHIP_HEIGHT),
    }
}

/// The content function drawing a frame at output `scale`.
pub fn draw_frame(look: AgentFrameLook, scale: f32) -> ContentDrawFunction {
    let draw = move |canvas: &skia::Canvas, w: f32, h: f32| -> skia::Rect {
        let [r, g, b] = look.color.map(|c| c as f32 / 255.0);
        let alpha = look.strength.clamp(0.0, 1.0);
        let width = Config::with(|c| c.agent_cursor.border_width) as f32 * scale;
        let bounds = skia::Rect::from_xywh(0.0, 0.0, w, h);

        // The glow: soft, fading inward from the edge.
        let glow = GLOW * scale;
        let mut glow_paint = skia::Paint::new(skia::Color4f::new(r, g, b, 0.45 * alpha), None);
        glow_paint.set_anti_alias(true);
        glow_paint.set_style(skia::PaintStyle::Stroke);
        glow_paint.set_stroke_width(glow);
        glow_paint.set_mask_filter(skia::MaskFilter::blur(
            skia::BlurStyle::Normal,
            glow / 3.0,
            false,
        ));
        canvas.draw_rect(bounds, &glow_paint);

        // The line, flush with the edge.
        let mut line = skia::Paint::new(skia::Color4f::new(r, g, b, alpha), None);
        line.set_anti_alias(true);
        line.set_style(skia::PaintStyle::Stroke);
        line.set_stroke_width(width * 2.0);
        canvas.draw_rect(bounds, &line);

        if look.chip && !look.names.is_empty() {
            draw_chip(canvas, &look, w / scale, scale, alpha);
        }
        bounds
    };
    draw.into()
}

fn draw_chip(canvas: &skia::Canvas, look: &AgentFrameLook, width: f32, scale: f32, alpha: f32) {
    let geometry = chip_geometry(&look.names, width);
    let rect = |(x, y, w, h): (f32, f32, f32, f32)| {
        skia::Rect::from_xywh(x * scale, y * scale, w * scale, h * scale)
    };
    let [r, g, b] = look.color.map(|c| c as f32 / 255.0);
    let radius = CHIP_HEIGHT * scale / 2.0;

    let mut chip = skia::Paint::new(skia::Color4f::new(r, g, b, alpha), None);
    chip.set_anti_alias(true);
    canvas.draw_rrect(
        skia::RRect::new_rect_xy(rect(geometry.chip), radius, radius),
        &chip,
    );
    // Stop sits on a darker pill at the chip's right end.
    let mut stop = skia::Paint::new(skia::Color4f::new(0.0, 0.0, 0.0, 0.28 * alpha), None);
    stop.set_anti_alias(true);
    let stop_rect = rect(geometry.stop).with_inset((2.0 * scale, 2.0 * scale));
    let stop_radius = stop_rect.height() / 2.0;
    canvas.draw_rrect(
        skia::RRect::new_rect_xy(stop_rect, stop_radius, stop_radius),
        &stop,
    );

    let font = chip_font(scale);
    let mut text = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, alpha), None);
    text.set_anti_alias(true);
    let (_, metrics) = font.metrics();
    let baseline =
        (CHIP_TOP + CHIP_HEIGHT / 2.0) * scale - (metrics.ascent + metrics.descent) / 2.0;
    canvas.draw_str(
        chip_text(&look.names),
        ((geometry.chip.0 + CHIP_PAD) * scale, baseline),
        &font,
        &text,
    );
    canvas.draw_str(
        STOP_LABEL,
        ((geometry.stop.0 + CHIP_PAD) * scale, baseline),
        &font,
        &text,
    );
}

/// A frame layer, sized to its output, faded in.
pub fn new_frame_layer(engine: &layers::engine::Engine, key: &str) -> Layer {
    let layer = engine.new_layer();
    layer.set_key(key.to_string());
    layer.set_pointer_events(false);
    layer.set_picture_cached(true);
    layer.set_opacity(0.0_f32, None);
    layer.set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.15)));
    layer
}

/// Place a frame over the workspace at `logical_index` on the strip.
pub fn place_frame(layer: &Layer, logical_index: usize, width: f32, height: f32, scale: f32) {
    let x = logical_index as f32 * (width + super::WORKSPACE_SPACING * scale);
    layer.set_size(Size::points(width, height), None);
    layer.set_position((x, 0.0), None);
}
