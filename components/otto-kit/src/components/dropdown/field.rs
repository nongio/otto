//! Pure drawing half: the closed field only.
//!
//! No `AppContext`, no `AppRunner`, no wayland-client — this has to draw from
//! a bare Skia canvas, because the compositor paints server-side otto-kit
//! components the same way it paints a titlebar. The open menu itself is the
//! client half, in [`super::menu`].
//!
//! Visual design lifted from the approved prototype in
//! `otto-settings/src/widgets.rs`'s `select` function: rounded box, current
//! value clipped to the field, chevron pair. This just adds interaction
//! states and a hit-test helper that reads the same `rect`.

use skia_safe::{
    Canvas, ClipOp, Color, Image, Paint, PaintStyle, Path, PathBuilder, Point, RRect, Rect,
};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::styles;

/// Height the visual design was tuned at. Callers may draw at other heights
/// — the geometry below is all relative to `rect` — but this is the
/// reference, matching the settings prototype's `CONTROL_H`.
pub const HEIGHT: f32 = 24.0;

const CORNER_RADIUS: f32 = 6.0;
/// Reserved width on the right for the chevron pair; the label clips before it.
const CHEVRON_GUTTER: f32 = 26.0;

/// Where a dropdown's menu opens, and so which way its field's indicator
/// points: the arrow says where the menu will be.
///
/// [`super::menu::DropdownMenu`] places its menu by it and [`draw_placed`]
/// draws the matching indicator, so a field and its menu never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// Under the field: a single arrow pointing down. For a menu of places
    /// or actions, where nothing in it is the field's current value.
    Below,
    /// Over the field: a single arrow pointing up. For a field along the
    /// bottom edge of a window.
    Above,
    /// On the field, the current value's row over the field itself, the
    /// other values above and below it: arrows both ways. For a field that
    /// shows one value of a list.
    #[default]
    OnValue,
}

impl Placement {
    /// The Solar glyph the indicator is drawn with.
    pub fn glyph_name(self) -> &'static str {
        match self {
            Self::Below => "go-down-symbolic",
            Self::Above => "go-up-symbolic",
            Self::OnValue => "otto-chevrons-up-down-symbolic",
        }
    }
}

/// Side of the indicator glyph, in points.
const INDICATOR_SIZE: f32 = 14.0;

/// Interaction state, set by the caller from pointer tracking and from
/// [`super::menu::DropdownMenu::is_open`]. `Open` is distinct from `Pressed`
/// — the field reads as active for as long as the menu is up, not just for
/// the initial click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropdownInteraction {
    Normal,
    Hovered,
    Pressed,
    Open,
    Disabled,
}

/// Is `(x, y)` within the field? Reads the same `rect` [`draw`] paints into,
/// so hit-testing cannot drift from what is on screen.
pub fn hit_test(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

/// Draw the closed field into `rect`: rounded box, `label` clipped before the
/// indicator gutter, and the indicator for a menu that opens on its value.
/// See [`draw_placed`].
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    label: &str,
    interaction: DropdownInteraction,
    theme: &Theme,
) {
    draw_placed(canvas, rect, label, interaction, Placement::OnValue, theme);
}

/// [`draw`], with the indicator for a menu opening at `placement`: pass the
/// placement the field's [`super::menu::DropdownMenu`] was given.
pub fn draw_placed(
    canvas: &Canvas,
    rect: Rect,
    label: &str,
    interaction: DropdownInteraction,
    placement: Placement,
    theme: &Theme,
) {
    draw_with_icon(canvas, rect, None, label, interaction, placement, theme);
}

/// Side of the icon a field may show before its value, in points.
pub const ICON_SIZE: f32 = 16.0;

/// Where the value starts, from the field's left edge.
const TEXT_INSET: f32 = 9.0;
/// The gap between an icon and the value after it.
const ICON_GAP: f32 = 6.0;

/// [`draw_placed`], with `icon` drawn [`ICON_SIZE`] square before the value
/// when there is one: a field whose values are places reads faster by their
/// icons, the way its menu's rows do. The image is drawn as it is, in its
/// own colours.
pub fn draw_with_icon(
    canvas: &Canvas,
    rect: Rect,
    icon: Option<&Image>,
    label: &str,
    interaction: DropdownInteraction,
    placement: Placement,
    theme: &Theme,
) {
    let disabled = interaction == DropdownInteraction::Disabled;
    let open = interaction == DropdownInteraction::Open;

    let mut bg = theme.fill_tertiary;
    bg = match interaction {
        DropdownInteraction::Hovered => lighten(bg),
        DropdownInteraction::Pressed => darken(bg),
        _ => bg,
    };
    if disabled {
        bg = scale_alpha(bg, 0.5);
    }

    let rrect = RRect::new_rect_xy(rect, CORNER_RADIUS, CORNER_RADIUS);
    canvas.draw_rrect(rrect, &fill(bg));

    // An open menu keeps the field reading as active the whole time it's up,
    // the way a native pop-up button's border picks up the accent colour
    // rather than just flashing on the initial click.
    let (border_color, border_width) = if open {
        (theme.accent, 1.5)
    } else {
        (theme.fill_secondary, 1.0)
    };
    canvas.draw_rrect(rrect, &stroke(border_color, border_width));

    canvas.save();
    canvas.clip_rect(
        Rect::from_ltrb(
            rect.left,
            rect.top,
            rect.right - CHEVRON_GUTTER,
            rect.bottom,
        ),
        ClipOp::Intersect,
        true,
    );
    let text_color = if disabled {
        scale_alpha(theme.text_primary, 0.5)
    } else {
        theme.text_primary
    };
    let mut text_x = rect.left + TEXT_INSET;
    if let Some(image) = icon {
        let dst = Rect::from_xywh(
            text_x,
            rect.center_y() - ICON_SIZE / 2.0,
            ICON_SIZE,
            ICON_SIZE,
        );
        let mut paint = Paint::default();
        if disabled {
            paint.set_alpha_f(0.5);
        }
        canvas.draw_image_rect(image, None, dst, &paint);
        text_x += ICON_SIZE + ICON_GAP;
    }
    Label::new(label)
        // A size under the row label beside it: the value is what the
        // button holds, and set as large as the label the two read as one
        // run of text with a box drawn round half of it.
        .with_style(styles::CALLOUT)
        .with_color(text_color)
        .centered_on(text_x, rect.center_y())
        .render(canvas);
    canvas.restore();

    let color = if disabled {
        scale_alpha(theme.text_secondary, 0.5)
    } else {
        theme.text_secondary
    };
    draw_indicator(
        canvas,
        Point::new(rect.right - 13.0, rect.center_y()),
        placement,
        color,
    );
}

/// Draw the indicator for a menu opening at `placement`, centred on `center`
/// in `color`.
///
/// Public for an app that paints its own pop-up button but opens a
/// [`super::menu::DropdownMenu`] from it: its arrow then says the same thing
/// the kit's fields do. Otto's Solar glyph, rasterised at the canvas's device
/// pixels; drawn strokes stand in where the set isn't installed, since the
/// compositor draws fields too.
pub fn draw_indicator(canvas: &Canvas, center: Point, placement: Placement, color: Color) {
    let scale = canvas.local_to_device_as_3x3().scale_x().abs().max(1.0);
    let px = (INDICATOR_SIZE * scale).round() as i32;
    if let Some(image) = crate::icons::solar_icon(
        placement.glyph_name(),
        px,
        crate::icons::SolarStyle::Linear,
        color,
    ) {
        let half = INDICATOR_SIZE / 2.0;
        let dst = Rect::from_ltrb(
            center.x - half,
            center.y - half,
            center.x + half,
            center.y + half,
        );
        canvas.draw_image_rect(&image, None, dst, &Paint::default());
        return;
    }
    draw_chevrons(canvas, center, placement, color);
}

fn draw_chevrons(canvas: &Canvas, center: Point, placement: Placement, color: Color) {
    let (cx, cy) = (center.x, center.y);
    let mut builder = PathBuilder::new();
    let mut up = |dy: f32| {
        builder.move_to(Point::new(cx - 3.5, cy + dy + 1.75));
        builder.line_to(Point::new(cx, cy + dy - 1.75));
        builder.line_to(Point::new(cx + 3.5, cy + dy + 1.75));
    };
    match placement {
        Placement::Above => up(0.0),
        Placement::Below => {}
        Placement::OnValue => up(-3.75),
    }
    let mut down = |dy: f32| {
        builder.move_to(Point::new(cx - 3.5, cy + dy - 1.75));
        builder.line_to(Point::new(cx, cy + dy + 1.75));
        builder.line_to(Point::new(cx + 3.5, cy + dy - 1.75));
    };
    match placement {
        Placement::Below => down(0.0),
        Placement::Above => {}
        Placement::OnValue => down(3.75),
    }

    let mut chevron = stroke(color, 1.4);
    chevron.set_stroke_cap(skia_safe::paint::Cap::Round);
    chevron.set_stroke_join(skia_safe::paint::Join::Round);
    let path: Path = builder.detach();
    canvas.draw_path(&path, &chevron);
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint
}

fn scale_alpha(color: Color, factor: f32) -> Color {
    let a = (color.a() as f32 * factor).round().clamp(0.0, 255.0) as u8;
    Color::from_argb(a, color.r(), color.g(), color.b())
}

fn lighten(color: Color) -> Color {
    mix_toward(color, 255, 0.12)
}

fn darken(color: Color) -> Color {
    mix_toward(color, 0, 0.12)
}

fn mix_toward(color: Color, target: u8, t: f32) -> Color {
    let mix = |c: u8| (c as f32 + (target as f32 - c as f32) * t).round() as u8;
    Color::from_argb(color.a(), mix(color.r()), mix(color.g()), mix(color.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_xywh(10.0, 10.0, 160.0, HEIGHT)
    }

    #[test]
    fn hit_test_matches_the_drawn_rect() {
        let r = rect();
        assert!(hit_test(r, 15.0, 15.0));
        assert!(!hit_test(r, 5.0, 15.0));
        assert!(!hit_test(r, 15.0, 100.0));
    }

    #[test]
    fn hit_test_is_inclusive_of_edges() {
        let r = rect();
        assert!(hit_test(r, r.left, r.top));
        assert!(hit_test(r, r.right, r.bottom));
    }

    #[test]
    fn draw_does_not_panic_across_every_interaction_state_and_theme() {
        // Guards against a canvas-only regression: this has to run with
        // nothing but a raster surface, no AppContext, no wayland-client.
        let mut surface = skia_safe::surfaces::raster_n32_premul((200, 60)).expect("surface");
        let canvas = surface.canvas();
        let r = rect();
        let icon = {
            let mut icon = skia_safe::surfaces::raster_n32_premul((16, 16)).expect("icon");
            icon.canvas().clear(Color::RED);
            icon.image_snapshot()
        };
        for theme in [Theme::light(), Theme::dark()] {
            for state in [
                DropdownInteraction::Normal,
                DropdownInteraction::Hovered,
                DropdownInteraction::Pressed,
                DropdownInteraction::Open,
                DropdownInteraction::Disabled,
            ] {
                for placement in [Placement::Below, Placement::Above, Placement::OnValue] {
                    draw_placed(canvas, r, "Documents", state, placement, &theme);
                    draw_with_icon(
                        canvas,
                        r,
                        Some(&icon),
                        "Documents",
                        state,
                        placement,
                        &theme,
                    );
                }
            }
        }
    }

    /// The arrow says where the menu opens.
    #[test]
    fn each_placement_has_its_own_glyph() {
        assert_eq!(Placement::Below.glyph_name(), "go-down-symbolic");
        assert_eq!(Placement::Above.glyph_name(), "go-up-symbolic");
        assert_eq!(Placement::default(), Placement::OnValue);
        let set = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/icons/solar/linear");
        for placement in [Placement::Below, Placement::Above, Placement::OnValue] {
            let file = set.join(format!("{}.svg", placement.glyph_name()));
            assert!(file.is_file(), "{} is not in the Solar set", file.display());
        }
    }

    /// Without the Solar set the drawn strokes still point the right way:
    /// a point above the centre for Above, below it for Below, both for
    /// OnValue.
    #[test]
    fn the_drawn_indicator_points_where_the_menu_opens() {
        let ink = |placement: Placement| {
            let mut surface = skia_safe::surfaces::raster_n32_premul((20, 20)).expect("surface");
            surface.canvas().clear(Color::TRANSPARENT);
            draw_chevrons(
                surface.canvas(),
                Point::new(10.0, 10.0),
                placement,
                Color::BLACK,
            );
            let image = surface.image_snapshot();
            let info = skia_safe::ImageInfo::new(
                (20, 20),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            let mut px = vec![0u8; 20 * 20 * 4];
            assert!(image.read_pixels(
                &info,
                &mut px,
                80,
                (0, 0),
                skia_safe::image::CachingHint::Allow
            ));
            // The chevrons' points sit on the centre column: where its ink
            // is says which way each one points.
            let column: Vec<u32> = (0..20).map(|y| px[(y * 20 + 10) * 4 + 3] as u32).collect();
            let above: u32 = column[..8].iter().sum();
            let below: u32 = column[13..].iter().sum();
            let centroid = column
                .iter()
                .enumerate()
                .map(|(y, a)| y as f32 * *a as f32)
                .sum::<f32>()
                / column.iter().sum::<u32>().max(1) as f32;
            (above, below, centroid)
        };
        let (above, below, _) = ink(Placement::OnValue);
        assert!(above > 0 && below > 0, "one point up, one down");
        let (_, _, centroid) = ink(Placement::Below);
        assert!(centroid > 10.0, "the point is below the centre");
        let (_, _, centroid) = ink(Placement::Above);
        assert!(centroid < 10.0, "the point is above the centre");
    }
}
