mod decoration_view;
mod effects;
mod model;
pub(crate) mod render;
mod resize_view;
mod view;

pub use decoration_view::WindowDecorationView;
pub use model::WindowDecorationModel;
pub use model::WindowViewBaseModel;
pub use model::WindowViewSurface;
pub use resize_view::{resize_edges_at, WindowResizeView};
pub use view::WindowView;

#[cfg(test)]
mod drag_damage_tests {
    use layers::prelude::*;
    use layers::skia::Contains;
    use layers::types::Size;

    use super::{model::WindowViewBaseModel, render::view_window_shadow};

    struct ExposeScene {
        engine: std::sync::Arc<Engine>,
        preview: Layer,
        drag_overlay: Layer,
    }

    /// Builds the exposé preview the way Otto does: a window subtree whose
    /// shadow view paints a band outside the window box (`view_window_shadow`),
    /// and a childless mirror layer that draws that subtree as its content
    /// (`WindowView`/`new_toplevel` in `shell/xdg.rs`, mapped into the selector
    /// by `Workspace::map_window`).
    fn expose_scene(width: f32, height: f32) -> ExposeScene {
        let engine = Engine::create(4000.0, 4000.0);

        let root = engine.new_layer();
        root.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        root.set_size(Size::points(4000.0, 4000.0), None);
        engine.add_layer(&root).unwrap();

        // The real window: base layer + the shadow view mounted on a child.
        let window_layer = engine.new_layer();
        window_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        window_layer.set_size(Size::points(width, height), None);
        engine.append_layer(&window_layer, root.id).unwrap();

        let shadow_layer = engine.new_layer();
        shadow_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        engine.append_layer(&shadow_layer, window_layer.id).unwrap();
        View::new(
            "window_shadow",
            WindowViewBaseModel {
                x: 0.0,
                y: 0.0,
                w: width,
                h: height,
                title: String::new(),
                fullscreen: false,
                active: true,
            },
            Box::new(view_window_shadow),
        )
        .mount_layer(shadow_layer.clone());
        shadow_layer.set_image_cached(true);

        // The exposé preview: a mirror of that subtree, sized to the window.
        let preview = engine.new_layer();
        preview.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        preview.set_size(Size::points(width, height), None);
        preview.set_draw_content(window_layer.as_content());
        preview.set_picture_cached(false);
        window_layer.add_follower_node(&preview);

        let selector_container = engine.new_layer();
        selector_container.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        selector_container.set_size(Size::points(4000.0, 4000.0), None);
        engine.append_layer(&selector_container, root.id).unwrap();
        selector_container.add_sublayer(&preview).unwrap();

        // Where `try_activate_drag` reparents the preview while it is dragged.
        let drag_overlay = engine.new_layer();
        drag_overlay.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        drag_overlay.set_size(Size::points(4000.0, 4000.0), None);
        engine.append_layer(&drag_overlay, root.id).unwrap();

        ExposeScene {
            engine,
            preview,
            drag_overlay,
        }
    }

    /// The preview paints the window's shadow, which reaches outside the
    /// preview box. Nothing in the preview's own layer tree says so — it has no
    /// children — so the engine has to take the extent from the subtree it
    /// mirrors, or every rect the damage tracker keeps for it is too small.
    #[test]
    fn expose_preview_covers_the_mirrored_shadow() {
        let scene = expose_scene(800.0, 600.0);
        scene.engine.update(0.016);
        scene.engine.update(0.016);

        let render_layer = scene.preview.render_layer();
        assert!(
            render_layer.global_transformed_bounds_with_children.width()
                > render_layer.global_transformed_bounds.width(),
            "the preview's painted rect must be wider than its own box: {:?} vs {:?}",
            render_layer.global_transformed_bounds_with_children,
            render_layer.global_transformed_bounds,
        );
    }

    /// Dragging a preview toward the workspace row moves and shrinks it a few
    /// times a second (`WindowSelectorView::update_drag_scale`). Every step has
    /// to damage what the preview covered at the PREVIOUS, larger scale —
    /// damaging only the new, smaller rect leaves the old shadow on screen and
    /// the preview drags a ghost behind it.
    #[test]
    fn dragging_a_preview_up_damages_the_size_it_had_before() {
        let scene = expose_scene(800.0, 600.0);
        scene.engine.update(0.016);
        scene.engine.update(0.016);

        // Drag start: the preview moves to the drag overlay and takes an anchor
        // point under the pointer.
        scene.drag_overlay.add_sublayer(&scene.preview).unwrap();
        scene.preview.set_anchor_point((0.5, 0.5), None);
        scene.preview.set_position((1500.0, 1500.0), None);
        scene.engine.update(0.016);

        // Pointer moves up; the scale ramps down toward the workspace-preview
        // scale as it approaches the selector row.
        let steps = [(1200.0, 0.75), (900.0, 0.5), (600.0, 0.3), (400.0, 0.18)];
        for (y, scale) in steps {
            let painted_before = scene
                .preview
                .render_layer()
                .global_transformed_bounds_with_children;
            assert!(
                painted_before.width()
                    > scene.preview.render_layer().global_transformed_bounds.width(),
                "the preview must know it paints the mirrored shadow outside its box: {painted_before:?}"
            );

            scene.engine.clear_damage();
            scene.preview.set_position((1500.0, y), None);
            scene.preview.set_scale((scale, scale), None);
            scene.engine.update(0.016);

            let painted_after = scene
                .preview
                .render_layer()
                .global_transformed_bounds_with_children;
            assert!(
                painted_after.width() < painted_before.width(),
                "step {scale} should shrink the preview: {painted_before:?} -> {painted_after:?}"
            );

            let damage = scene.engine.damage();
            assert!(
                damage.contains(painted_before),
                "damage {damage:?} misses what the preview covered at the previous scale {painted_before:?}"
            );
            assert!(
                damage.contains(painted_after),
                "damage {damage:?} misses what the preview covers now {painted_after:?}"
            );
        }
    }

    /// Same drag, but with the pointer-event rate the compositor actually sees:
    /// several motion events (each setting a new position and scale) land
    /// between two engine updates. The damage of the frame that follows still
    /// has to cover what the preview painted at the last RENDERED size, not at
    /// some intermediate one.
    #[test]
    fn drag_damage_covers_the_last_rendered_size_with_batched_motion() {
        let scene = expose_scene(800.0, 600.0);
        scene.engine.update(0.016);
        scene.engine.update(0.016);

        scene.drag_overlay.add_sublayer(&scene.preview).unwrap();
        scene.preview.set_anchor_point((0.5, 0.5), None);
        scene.preview.set_position((1500.0, 1500.0), None);
        scene.engine.update(0.016);

        let mut scale = 1.0_f32;
        let mut y = 1500.0_f32;
        for _frame in 0..8 {
            let painted_before = scene
                .preview
                .render_layer()
                .global_transformed_bounds_with_children;

            scene.engine.clear_damage();
            // Three motion events inside one frame.
            for _event in 0..3 {
                scale *= 0.97;
                y -= 12.0;
                scene.preview.set_position((1500.0, y), None);
                scene.preview.set_scale((scale, scale), None);
            }
            scene.engine.update(0.016);

            let damage = scene.engine.damage();
            assert!(
                damage.contains(painted_before),
                "damage {damage:?} misses the last rendered size {painted_before:?}"
            );
        }
    }
}
