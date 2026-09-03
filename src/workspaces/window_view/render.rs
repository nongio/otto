use layers::{prelude::*, types::Size};
use otto_kit::components::titlebar::{WindowControl, WindowDecoration};

use crate::config::Config;

use super::model::{WindowDecorationModel, WindowViewBaseModel};

/// Map the model's control index back to otto-kit's enum. See
/// [`WindowDecorationModel::pressed`] for why it is stored as an index.
pub fn control_from_index(index: u8) -> Option<WindowControl> {
    match index {
        0 => Some(WindowControl::Close),
        1 => Some(WindowControl::Minimize),
        2 => Some(WindowControl::Zoom),
        _ => None,
    }
}

/// Build the otto-kit decoration for a model. Used both to draw the titlebar
/// and to hit-test it, so the click targets can't drift from the pixels.
pub fn decoration_for(state: &WindowDecorationModel) -> WindowDecoration {
    WindowDecoration {
        title: state.title.clone(),
        width: state.width,
        titlebar_height: state.height,
        corner_radius: state.corner_radius,
        active: state.active,
        dark: state.dark,
        controls_hovered: state.controls_hovered,
        pressed: state.pressed.and_then(control_from_index),
        sharing: state.sharing,
        // A window that cannot be resized cannot be zoomed: the dot stays
        // gray and shows no glyph, the way macOS marks a fixed-size panel.
        disabled: if state.fixed_size {
            vec![otto_kit::components::titlebar::WindowControl::Zoom]
        } else {
            Vec::new()
        },
        // The layer carries `BackgroundBlur` while the window is focused, so
        // the compositor already blurs what is behind it — blurring again in
        // the paint would double up. An unfocused window has no blur, so its
        // bar is filled in rather than left translucent over the desktop.
        backdrop_blur: 0.0,
        blurred: state.active,
        // The tint rides on the decoration layer instead of this paint, so
        // focus can fade it between the frosted and the opaque form without
        // repainting the bar — see `WindowView::fade_decoration_material`.
        tint_on_layer: true,
        ..Default::default()
    }
}

/// The server-side titlebar. Drawn by otto-kit's `WindowDecoration`, the same
/// component otto-kit clients draw their own titlebars with.
#[profiling::function]
pub fn view_window_decoration(
    state: &WindowDecorationModel,
    _view: &View<WindowDecorationModel>,
) -> LayerTree {
    let scale = state.scale.max(1.0);
    // The bar sits at the window layer's origin, so its own origin is already
    // on the grid and only the far edge needs snapping. Unsnapped, a 34pt bar
    // at scale 1.75 is 59.5px and its bottom hairline smears over three rows.
    // `update_window_view` rounds the content layer's y offset the same way,
    // so the client still starts exactly where the bar ends.
    let width_px = crate::workspaces::utils::snap_extent_px(0.0, state.width * scale);
    let height_px = crate::workspaces::utils::snap_extent_px(0.0, state.height * scale);
    let deco = decoration_for(state);

    let draw = move |canvas: &layers::skia::Canvas, _w: f32, _h: f32| {
        // The decoration is described in logical points; the layer is sized in
        // physical pixels, so the whole paint scales up by the output scale.
        canvas.save();
        canvas.scale((scale, scale));
        deco.draw(canvas);
        canvas.restore();
        layers::skia::Rect::from_wh(width_px, height_px)
    };

    LayerTreeBuilder::default()
        .key("window_decoration")
        .layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        })
        .position((Point { x: 0.0, y: 0.0 }, None))
        .size((
            Size {
                width: taffy::Dimension::Length(width_px),
                height: taffy::Dimension::Length(height_px),
            },
            None,
        ))
        // The blend mode and the tint are not set here: they are animated on
        // the layer itself when focus changes, and a rebuild of this tree
        // would snap them back to whatever this said. See
        // `WindowView::fade_decoration_material`, which owns both.
        .content(Some(draw))
        .pointer_events(false)
        .build()
        .unwrap()
}

/// How far the shadow band reaches outside the window box on every side.
pub(crate) const SAFE_AREA: f32 = 100.0;

/// Paint the shadow band a window sits in, into a canvas `SAFE_AREA` larger
/// than the window on every side.
///
/// Split out of the view so a test can look at the pixels: the passes below
/// are the only thing that decides whether a corner is reached.
pub(crate) fn paint_window_shadow(
    canvas: &layers::skia::Canvas,
    w: f32,
    h: f32,
    is_active: bool,
    draw_scale: f32,
) {
    // draw shadow with different opacity based on activation state
    let window_corner_radius = 24.0 * draw_scale;
    let rect = layers::skia::Rect::from_xywh(
        SAFE_AREA,
        SAFE_AREA,
        w - SAFE_AREA * 2.0,
        h - SAFE_AREA * 2.0,
    );

    let rrect = layers::skia::RRect::new_rect_xy(rect, window_corner_radius, window_corner_radius);
    canvas.clip_rrect(rrect, layers::skia::ClipOp::Difference, false);

    // Inner shadow - lighter for active, very light for inactive
    let inner_opacity = if is_active { 0.25 } else { 0.08 };
    let mut shadow_paint = layers::skia::Paint::new(
        layers::skia::Color4f::new(0.0, 0.0, 0.0, inner_opacity),
        None,
    );
    shadow_paint.set_mask_filter(layers::skia::MaskFilter::blur(
        layers::skia::BlurStyle::Normal,
        3.0,
        false,
    ));
    canvas.draw_rrect(rrect, &shadow_paint);

    // Ambient shadow - no offset, so the top edge and the two top corners
    // get a falloff of their own. The outer pass below is pushed down to
    // read as a light from above, which leaves everything above the window
    // with nothing but the 3px inner line: the corners came out bare
    // against a bright wallpaper.
    let ambient_opacity = if is_active { 0.18 } else { 0.07 };
    shadow_paint.set_mask_filter(layers::skia::MaskFilter::blur(
        layers::skia::BlurStyle::Normal,
        14.0 * draw_scale,
        false,
    ));
    shadow_paint.set_color4f(
        layers::skia::Color4f::new(0.0, 0.0, 0.0, ambient_opacity),
        None,
    );
    canvas.draw_rrect(rrect, &shadow_paint);

    // Outer shadow - stronger for active, very light for inactive
    let rect = layers::skia::Rect::from_xywh(
        SAFE_AREA,
        SAFE_AREA + 20.0 * draw_scale,
        w - SAFE_AREA * 2.0,
        h - SAFE_AREA * 2.0,
    );
    let rrect = layers::skia::RRect::new_rect_xy(rect, window_corner_radius, window_corner_radius);
    shadow_paint.set_mask_filter(layers::skia::MaskFilter::blur(
        layers::skia::BlurStyle::Normal,
        30.0,
        false,
    ));

    // Active: darker shadow (0.35), Inactive: very light shadow (0.12)
    let outer_opacity = if is_active { 0.35 } else { 0.12 };
    shadow_paint.set_color4f(
        layers::skia::Color4f::new(0.1, 0.1, 0.1, outer_opacity),
        None,
    );

    canvas.draw_rrect(rrect, &shadow_paint);
}

#[profiling::function]
pub fn view_window_shadow(
    state: &WindowViewBaseModel,
    _view: &View<WindowViewBaseModel>,
) -> LayerTree {
    let w = state.w;
    let h = state.h;
    let is_active = state.active;
    let draw_scale = Config::with(|config| config.screen_scale) as f32;
    let draw_shadow = move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        paint_window_shadow(canvas, w, h, is_active, draw_scale);
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    LayerTreeBuilder::default()
        .key("window_shadow")
        .size((
            Size {
                width: taffy::Dimension::Length(w),
                height: taffy::Dimension::Length(h),
            },
            None,
        ))
        .pointer_events(false)
        .image_cache(true)
        .children(vec![LayerTreeBuilder::default()
            .key("window_shadow_inner")
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            })
            .position((
                Point {
                    x: -SAFE_AREA,
                    y: -SAFE_AREA,
                },
                None,
            ))
            .size((
                Size {
                    width: taffy::Dimension::Length(w + SAFE_AREA * 2.0),
                    height: taffy::Dimension::Length(h + SAFE_AREA * 2.0),
                },
                None,
            ))
            .content(Some(draw_shadow))
            .pointer_events(false)
            .build()
            .unwrap()])
        .build()
        .unwrap()
}
