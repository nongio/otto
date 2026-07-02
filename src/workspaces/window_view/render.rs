use layers::{prelude::*, types::Size};

use crate::config::Config;

use super::model::WindowViewBaseModel;

#[profiling::function]
pub fn view_window_shadow(
    state: &WindowViewBaseModel,
    _view: &View<WindowViewBaseModel>,
) -> LayerTree {
    let w = state.w;
    let h = state.h;
    let is_active = state.active;
    const SAFE_AREA: f32 = 100.0;
    let draw_scale = Config::with(|config| config.screen_scale) as f32;
    let draw_shadow = move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        // draw shadow with different opacity based on activation state
        let window_corner_radius = 24.0 * draw_scale;
        let rect = layers::skia::Rect::from_xywh(
            SAFE_AREA,
            SAFE_AREA,
            w - SAFE_AREA * 2.0,
            h - SAFE_AREA * 2.0,
        );

        let rrect =
            layers::skia::RRect::new_rect_xy(rect, window_corner_radius, window_corner_radius);
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

        // Outer shadow - stronger for active, very light for inactive
        let rect = layers::skia::Rect::from_xywh(
            SAFE_AREA,
            SAFE_AREA + 20.0 * draw_scale,
            w - SAFE_AREA * 2.0,
            h - SAFE_AREA * 2.0,
        );
        let rrect =
            layers::skia::RRect::new_rect_xy(rect, window_corner_radius, window_corner_radius);
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
