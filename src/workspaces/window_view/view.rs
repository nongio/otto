use layers::{
    engine::{Engine, TransactionRef},
    prelude::{taffy, Layer, Transition},
    skia,
    types::Point,
    view::RenderLayerTree,
};
use smithay::{reexports::wayland_server::backend::ObjectId, utils::Logical};
use std::sync::{atomic::AtomicBool, Arc};

use crate::shell::WindowElement;

use super::{
    effects::GenieEffect,
    model::{WindowDecorationModel, WindowViewBaseModel},
    render::{decoration_for, view_window_decoration, view_window_shadow},
};

/// How long the titlebar takes to fade between its frosted and its opaque
/// form. Matches otto-kit's `MATERIAL_FADE`: a window the compositor
/// decorates and one that draws its own bar should not fade at different
/// speeds.
const MATERIAL_FADE: f32 = 0.3;

#[derive(Clone)]
pub struct WindowView {
    pub window_id: ObjectId,
    // views
    pub view_base: layers::prelude::View<WindowViewBaseModel>,
    /// Server-side titlebar, empty until the window negotiates SSD
    pub view_decoration: layers::prelude::View<WindowDecorationModel>,

    // layers
    pub window_layer: layers::prelude::Layer,
    pub shadow_layer: layers::prelude::Layer,
    pub content_layer: layers::prelude::Layer,
    pub decoration_layer: layers::prelude::Layer,
    pub mirror_layer: layers::prelude::Layer,

    pub genie_effect: GenieEffect,

    pub unmaximised_rect: smithay::utils::Rectangle<i32, Logical>,
    /// The tiling zone this window is currently snapped into, if any. Set when
    /// a window is tiled (drag-snap or shortcut), cleared on drag-off or restore.
    /// `unmaximised_rect` holds the pre-snap geometry.
    pub tiled_zone: Option<crate::workspaces::TileZone>,
    pub minimizing_animation: Arc<AtomicBool>,
    pub is_unmapped: Arc<AtomicBool>,
}

impl WindowView {
    pub fn new(layers_engine: Arc<Engine>, window: &WindowElement) -> Self {
        let window_id = window.id();
        let layer = window.base_layer().clone();
        layer.set_key("window");
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        let content_layer = layers_engine.new_layer();
        content_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        let shadow_layer = layers_engine.new_layer();
        shadow_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        // The titlebar sits above the client content, so it is appended last.
        let decoration_layer = layers_engine.new_layer();
        decoration_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        decoration_layer.set_hidden(true);
        // The titlebar carries `BackgroundBlur`, and what it has to blur is
        // usually the window *below* it in the same plane, not just the
        // wallpaper. Seeding the plane's pre-blurred background would land
        // behind that same-pass content and leave the window underneath sharp,
        // so opt into the raw background plus a real blur.
        decoration_layer.set_blur_include_content(true);

        let _ = layers_engine.append_layer(&shadow_layer, layer.id());
        let _ = layers_engine.append_layer(&content_layer, layer.id());
        let _ = layers_engine.append_layer(&decoration_layer, layer.id());

        let base_rect = WindowViewBaseModel {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            title: "".to_string(),
            fullscreen: false,
            active: false,
        };
        let view_base =
            layers::prelude::View::new("window_shadow", base_rect, Box::new(view_window_shadow));

        view_base.mount_layer(shadow_layer.clone());

        let view_decoration = layers::prelude::View::new(
            "window_decoration",
            WindowDecorationModel::default(),
            Box::new(view_window_decoration),
        );
        view_decoration.mount_layer(decoration_layer.clone());
        let mirror_layer = window.mirror_layer().clone();
        mirror_layer.set_size(shadow_layer.render_layer().bounds.size(), None);

        shadow_layer.set_image_cached(true);

        let genie_effect = GenieEffect::new();

        Self {
            window_id,
            view_base,
            view_decoration,
            window_layer: layer,
            content_layer,
            decoration_layer,
            shadow_layer,
            genie_effect,
            mirror_layer,
            unmaximised_rect: smithay::utils::Rectangle::default(),
            tiled_zone: None,
            minimizing_animation: Arc::new(AtomicBool::new(false)),
            is_unmapped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Show or hide the server-side titlebar.
    pub fn set_decorated(&self, decorated: bool) {
        self.decoration_layer.set_hidden(!decorated);
    }

    /// Fade the titlebar's material between its frosted and its opaque form.
    ///
    /// The bar is translucent over a blurred backdrop while the window is
    /// focused and filled in when it is not, and both the tint and the blur
    /// belong to the layer rather than to the paint — so this is one colour
    /// animation, not a repaint per frame.
    ///
    /// The blend mode is switched at the *opaque* end in both directions, so
    /// the frost never appears or disappears in view: gaining focus turns the
    /// blur on while the opaque tint still covers it and then thins the tint
    /// to reveal it; losing focus thickens the tint back to opaque and only
    /// drops the blur once nothing can be seen through it. Same sequence
    /// otto-kit's `Window` runs for a client that draws its own bar.
    fn fade_decoration_material(&self, model: &WindowDecorationModel, animate: bool) {
        let frosted = model.active;
        let tint = decoration_for(model).material_tint(frosted);
        let color = skia::Color4f::from(tint);
        let color = layers::types::Color::new_rgba(color.r, color.g, color.b, color.a);

        if frosted {
            // Blur on first, under a tint that is still opaque.
            self.decoration_layer
                .set_blend_mode(layers::types::BlendMode::BackgroundBlur);
        }

        if !animate {
            self.decoration_layer.set_background_color(color, None);
            if !frosted {
                self.decoration_layer
                    .set_blend_mode(layers::types::BlendMode::Normal);
            }
            return;
        }

        let transaction = self
            .decoration_layer
            .set_background_color(color, Transition::ease_out_quad(MATERIAL_FADE));
        if !frosted {
            // A full-window-width gaussian per frame, for a window nobody is
            // looking at, buys nothing — but it has to run until the bar is
            // opaque, or the frost is seen going away.
            transaction.on_finish(
                move |l: &Layer, _| l.set_blend_mode(layers::types::BlendMode::Normal),
                true,
            );
        }
    }

    /// Push new titlebar state. No-op when nothing changed: this runs on every
    /// commit of a decorated window, and re-rendering the view would repaint
    /// the bar at the client's frame rate.
    pub fn update_decoration(&self, model: WindowDecorationModel) {
        let current = self.view_decoration.get_state();
        // Focus is the one change that fades rather than lands: everything
        // else about the bar is redrawn, but the material moves. Done before
        // the equality check below, since a change of focus alone still has to
        // reach the layer.
        // `first` is the very first state a bar is given: the window has just
        // appeared, and there is no previous material to fade from.
        let first = current.width == 0.0;
        if first || current.corner_radius != model.corner_radius || current.scale != model.scale {
            // The tint is the layer's background now, so the layer has to
            // carry the bar's shape: rounded across the top to sit inside the
            // window's frame, square at the bottom where the client's content
            // continues. The layer is sized in physical pixels.
            let radius = model.corner_radius * model.scale.max(1.0);
            self.decoration_layer.set_border_corner_radius(
                layers::types::BorderRadius {
                    top_left: radius,
                    top_right: radius,
                    bottom_right: 0.0,
                    bottom_left: 0.0,
                },
                None,
            );
        }
        if first || current.active != model.active || current.dark != model.dark {
            self.fade_decoration_material(&model, !first);
        }
        if current.width == model.width
            && current.height == model.height
            && current.title == model.title
            && current.active == model.active
            && current.dark == model.dark
            && current.corner_radius == model.corner_radius
            && current.controls_hovered == model.controls_hovered
            && current.pressed == model.pressed
            && current.sharing == model.sharing
            && current.scale == model.scale
        {
            return;
        }
        self.view_decoration.update_state(&model);
    }

    /// Repaint the titlebar without changing its state.
    ///
    /// The accent is read inside the render function rather than carried in
    /// `WindowDecorationModel`, so `update_decoration` would hash to the same
    /// state and skip the repaint — the layer tree is rebuilt directly.
    pub fn rerender_decoration(&self) {
        self.view_decoration.render(&self.decoration_layer);
    }

    /// Current titlebar state, for hit testing.
    pub fn decoration_state(&self) -> WindowDecorationModel {
        self.view_decoration.get_state()
    }

    /// Update the activation state of the window shadow
    pub fn set_active(&self, active: bool) {
        let current_state = self.view_base.get_state();
        let new_state = WindowViewBaseModel {
            active,
            ..current_state.clone()
        };
        self.view_base.update_state(&new_state);
    }

    /// Hide/show the content layer for shadow-only (direct scanout) mode.
    pub fn set_content_hidden(&self, hidden: bool) {
        self.content_layer.set_hidden(hidden);
    }

    pub fn set_is_minimizing(&self, minimizing: bool) {
        self.minimizing_animation
            .store(minimizing, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn is_minimizing(&self) -> bool {
        self.minimizing_animation
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_is_unmapped(&self, unmapped: bool) {
        self.is_unmapped
            .store(unmapped, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_unmapped(&self) -> bool {
        self.is_unmapped.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns true if the window layer's scene node is still alive.
    pub fn is_alive(&self) -> bool {
        self.window_layer
            .engine
            .is_layer_alive(&self.window_layer.id)
    }

    pub fn minimize(&self, to_rect: skia::Rect) -> TransactionRef {
        // Image filters only render through the image-cached (offscreen)
        // path in lay-rs — a filter set on a plain layer is ignored. Cache
        // the layer for the duration of the genie; cleared in on_finish.
        self.window_layer.set_image_cached(true);
        self.window_layer.set_effect(self.genie_effect.clone());
        self.genie_effect.set_destination(to_rect, true);

        let render_layer = self.window_layer.render_bounds_with_children();

        let w = render_layer.width();
        let h = render_layer.height();

        // Read live destination from the genie effect so the draw area
        // tracks the expanding drawer during parallel animation.
        let dst_ref = self.genie_effect.dst.clone();
        self.window_layer
            .set_draw_content(move |_: &skia::Canvas, _w, _h| {
                let dst = *dst_ref.read().unwrap();
                skia::Rect::join2(skia::Rect::from_wh(w, h), dst).with_outset((100.0, 100.0))
            });

        let tr = self
            .window_layer
            .set_image_filter_progress(1.0_f32, Transition::linear(0.7));

        self.set_is_minimizing(true);
        let view_ref = self.clone();
        tr.on_finish(
            move |l: &Layer, _| {
                if view_ref.is_unmapped() {
                    // Clear on the early-return too: the flag drives the
                    // backend's forced-composite mode, and wedging it `true`
                    // when a window closes mid-animation would lock the
                    // output in composite forever.
                    view_ref.set_is_minimizing(false);
                    return;
                }
                // NOTE: is_minimizing stays TRUE here — the caller clears it
                // only after the layer is reparented into the drawer, so the
                // backend holds the forced-composite mode across the whole
                // transition and the planes come back with one clean full
                // redraw.
                // After the animation, drop the shader. The caller reparents
                // the layer into the dock drawer and applies the minimized
                // scale/position — the genie must run while the window is
                // still in the windows-plane subtree (reparenting first put
                // it under the strip-sized dock plane, which clipped the
                // whole animation), so drawer-local placement can only
                // happen after this finishes. Hide the layer for the gap:
                // effect-free and still workspace-parented, it would render
                // one plain full-size frame at its old spot (ghost).
                l.remove_effect();
                l.set_image_cached(false);
                l.set_hidden(true);
            },
            true,
        );
        tr
    }

    pub fn unminimize(&self, from: skia::Rect) -> TransactionRef {
        self.set_is_minimizing(true);
        // Re-enable the shader and reset the scale before running the animation
        // we need set opacity to 0 first to avoid flickering
        self.window_layer.set_hidden(false);
        self.window_layer.set_opacity(0.0_f32, None);
        self.window_layer.set_scale(Point { x: 1.0, y: 1.0 }, None);
        // See minimize(): filters render only via the image-cached path.
        self.window_layer.set_image_cached(true);
        self.window_layer.set_effect(self.genie_effect.clone());

        self.window_layer.engine.update(0.0);
        self.genie_effect.set_destination(from, false);
        let view_ref = self.clone();
        self.window_layer.set_image_filter_progress(1.0_f32, None);
        *self
            .window_layer
            .set_image_filter_progress(0.0_f32, Transition::linear(0.8))
            .on_start(
                |l: &Layer, _| {
                    l.set_opacity(1.0_f32, None);
                },
                true,
            )
            .on_finish(
                move |l: &Layer, _| {
                    l.remove_effect();
                    l.set_image_cached(false);
                    view_ref.set_is_minimizing(false);
                },
                true,
            )
    }

    /// Apply a scale/position to make the window fit inside the minimized drawer rect.
    /// This is used both after the minimize animation and when the dock resizes.
    /// Returns the scale transaction so callers can sequence work strictly
    /// after the scale has been APPLIED by the engine (e.g. unhiding the
    /// layer — unhiding before the scheduled scale lands would show the
    /// window at full size for a frame).
    pub fn apply_minimized_scale(&self, target: skia::Rect) -> Option<TransactionRef> {
        if self.is_unmapped() {
            return None;
        }
        let bounds = self.window_layer.render_layer().bounds_with_children;
        let base_size = (bounds.width(), bounds.height());
        self.apply_minimized_scale_to_layer(&self.window_layer, base_size, target)
    }

    /// Like [`apply_minimized_scale`], but with an explicit unscaled base
    /// size. Needed when the layer is currently HIDDEN (post-genie settle):
    /// hidden means `display: none`, the layout skips the node and
    /// `bounds_with_children` reads zero — the implicit-base variant then
    /// falls back to scale 1.0 and parks the window in the drawer at full
    /// size.
    pub fn apply_minimized_scale_with_base(
        &self,
        base_size: (f32, f32),
        target: skia::Rect,
    ) -> Option<TransactionRef> {
        if self.is_unmapped() {
            return None;
        }
        self.apply_minimized_scale_to_layer(&self.window_layer, base_size, target)
    }

    fn apply_minimized_scale_to_layer(
        &self,
        layer: &Layer,
        base_size: (f32, f32),
        target: skia::Rect,
    ) -> Option<TransactionRef> {
        if self.is_minimizing() {
            return None;
        }
        let (w, h) = base_size;
        let scale_x = if w > 0.0 { target.width() / w } else { 1.0 };
        let scale_y = if h > 0.0 { target.height() / h } else { 1.0 };
        let target_scale = scale_x.min(scale_y);
        // The fit keeps the aspect ratio, so one axis is short of the square
        // drawer: centre the thumbnail in it, or the window sits in the
        // drawer's top-left corner and its tooltip — centred on the drawer —
        // is visibly off-centre over the window.
        layer.set_position(
            Point {
                x: (target.width() - w * target_scale).max(0.0) / 2.0,
                y: (target.height() - h * target_scale).max(0.0) / 2.0,
            },
            None,
        );
        Some(layer.set_scale(
            Point {
                x: target_scale,
                y: target_scale,
            },
            None,
        ))
    }
}

impl std::fmt::Debug for WindowView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowView")
            .field("view_base", &self.view_base)
            .field("window_layer", &self.window_layer)
            .field("content_layer", &self.content_layer)
            .finish()
    }
}
