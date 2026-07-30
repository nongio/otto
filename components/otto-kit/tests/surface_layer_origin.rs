//! Every surface's root layer node is its own origin.
//!
//! A surface is drawn by rendering its root node into that surface's canvas,
//! and the renderer concatenates the node's transform first. So the node has
//! to sit at (0, 0) at its full size no matter what else the shared engine
//! holds — an app with a surface per output has one node per screen in the
//! same engine, and a surface recreated after an output comes back is
//! appended after the survivors.
//!
//! Left in the engine root's flow layout, those siblings lay out side by side
//! and shrink to fit: the panel then paints offset to the right of its own
//! buffer. This mirrors what `BaseWaylandSurface::new` builds.

use layers::prelude::*;

fn surface_node(engine: &std::sync::Arc<Engine>, width: f32, height: f32) -> Layer {
    let layer = engine.new_layer();
    layer.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    layer.set_size(layers::types::Size::points(width, height), None);
    layer.set_position((0.0, 0.0), None);
    let _ = engine.add_layer(&layer);
    layer
}

#[test]
fn surface_nodes_keep_their_own_origin() {
    let engine = Engine::create(1920.0, 1080.0);

    // A lock screen with a surface on the laptop panel and one on a virtual
    // output, then the laptop panel goes away with the lid and comes back:
    // its surface is recreated, and its node is now the last of three.
    let panel = surface_node(&engine, 1440.0, 960.0);
    let virt = surface_node(&engine, 960.0, 540.0);
    for _ in 0..3 {
        engine.update(0.016);
    }
    assert_eq!(panel.render_bounds_transformed().left, 0.0);
    assert_eq!(virt.render_bounds_transformed().right, 960.0);

    let panel = surface_node(&engine, 1440.0, 960.0);
    for _ in 0..3 {
        engine.update(0.016);
    }
    let bounds = panel.render_bounds_transformed();
    assert_eq!(
        bounds.left, 0.0,
        "recreated surface must draw at its origin"
    );
    assert_eq!(bounds.right, 1440.0, "recreated surface must keep its size");
}
