//! Typed scene geometry.
//!
//! Otto has two coordinate spaces (see `docs/developer/README.md`):
//! **physical** pixels, which is what an output's lay-rs scene is laid out
//! in, and **logical** pixels, which is what Smithay hands out for pointer
//! events and output placement. Carrying them as Smithay's
//! `Point`/`Size`/`Rectangle<_, Physical>` and `<_, Logical>` makes mixing the
//! two a compile error instead of a bug that only shows at fractional scale.
//!
//! lay-rs itself speaks untyped `f32`s. The helpers here are the edge between
//! the two: typed geometry goes in, the plain lay-rs value comes out (or the
//! other way round). Everything in between stays typed.

use layers::prelude::Layer;
use smithay::utils::{Logical, Physical, Point, Size};

/// A point in an output's physical scene space, at the `f32` precision lay-rs
/// works in.
pub type PhysicalPoint = Point<f32, Physical>;

/// A size in an output's physical scene space, at the `f32` precision lay-rs
/// works in.
pub type PhysicalSize = Size<f32, Physical>;

/// A physical-pixel point as a lay-rs point, for `set_position`,
/// `change_position` and hit tests such as `View::hover_layer`.
pub fn layer_point(point: Point<f32, Physical>) -> layers::types::Point {
    layers::types::Point {
        x: point.x,
        y: point.y,
    }
}

/// A physical-pixel size as a fixed lay-rs (taffy) size, both axes
/// `Dimension::Length`.
pub fn layer_size(size: Size<f32, Physical>) -> layers::types::Size {
    layers::types::Size {
        width: layers::taffy::style::Dimension::Length(size.w),
        height: layers::taffy::style::Dimension::Length(size.h),
    }
}

/// The top-left corner of a layer's transformed render bounds, in the
/// physical scene space every output subtree is laid out in.
pub fn layer_origin(layer: &Layer) -> Point<f32, Physical> {
    let r = layer.render_bounds_transformed();
    Point::new(r.x(), r.y())
}

/// The centre of a layer's transformed render bounds, in physical scene
/// space.
pub fn layer_center(layer: &Layer) -> Point<f32, Physical> {
    let r = layer.render_bounds_transformed();
    Point::new(r.center_x(), r.center_y())
}

/// Is the physical scene-space `point` inside `layer`'s transformed render
/// bounds, edges included?
///
/// Unlike `View::hover_layer` (strict on every edge) and Smithay's
/// `Rectangle::contains` (half-open), both edges count.
pub fn layer_contains(layer: &Layer, point: Point<f32, Physical>) -> bool {
    let r = layer.render_bounds_transformed();
    point.x >= r.x()
        && point.x <= r.x() + r.width()
        && point.y >= r.y()
        && point.y <= r.y() + r.height()
}

/// A global logical position (a pointer event) in the physical scene space of
/// the output whose global logical origin is `output_origin`.
///
/// Every output subtree renders at the scene origin, so the output's own
/// placement is subtracted first, then the result is scaled by that output's
/// `scale` (`output.current_scale().fractional_scale()`). The arithmetic is
/// done in `f64` and narrowed once at the end.
pub fn output_local_physical(
    location: Point<f64, Logical>,
    output_origin: Point<f64, Logical>,
    scale: f64,
) -> Point<f32, Physical> {
    let local = (location - output_origin).to_physical(scale);
    Point::new(local.x as f32, local.y as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_local_physical_rebases_then_scales() {
        let p = output_local_physical(Point::new(1930.0, 120.0), Point::new(1920.0, 0.0), 1.5);
        assert_eq!((p.x, p.y), (15.0, 180.0));
    }
}
