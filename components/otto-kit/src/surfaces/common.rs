use layers::prelude::taffy;
use layers::types::Size;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;
use wayland_client::{protocol::wl_surface, Proxy};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;

use crate::app_runner::surface_scale::{
    self, BufferGeometry, BufferScale, ScaleSync, ScaledBuffer, View,
};

// Re-export sc-layer protocol for convenience
pub(super) use crate::protocols::{otto_surface_style_manager_v1, otto_surface_style_v1};

use crate::{rendering::SkiaSurface, AppContext};

/// Frame a styled surface with the palette's hairline.
///
/// The compositor strokes it inside the surface bounds, so it follows the
/// corner radius and costs the client nothing to draw — and it is the same
/// line [`crate::theme::Theme::hairline`] puts around a menu, so a window and the
/// menu it opens are edged alike.
pub fn apply_hairline_border(style: &otto_surface_style_v1::OttoSurfaceStyleV1) {
    let color = AppContext::current_theme().hairline();
    let width = crate::theme::Theme::HAIRLINE_WIDTH;
    // Doubled on the way out: the compositor strokes the border centred on the
    // surface bounds, and every window that asks for this also clips itself to
    // them (`set_masks_to_bounds`), so the outer half is thrown away. Asking
    // for twice the width is what makes the surviving inner half the hairline
    // that was asked for.
    style.set_border(
        (width * 2.0) as f64,
        color.r() as f64 / 255.0,
        color.g() as f64 / 255.0,
        color.b() as f64 / 255.0,
        color.a() as f64 / 255.0,
    );
}

/// Core surface with all rendering functionality built-in
///
/// This struct contains the common fields and methods used by
/// ToplevelSurface, SubsurfaceSurface, and PopupSurface.
/// All surface operations are implemented directly on this type.
#[derive(Clone)]
pub struct BaseWaylandSurface {
    // The underlying Wayland surface
    pub(super) wl_surface: wl_surface::WlSurface,
    // Skia surface for rendering
    pub(super) skia_surface: Option<Rc<SkiaSurface>>,
    // Surface style for animation and blending
    pub(super) surface_style: Option<otto_surface_style_v1::OttoSurfaceStyleV1>,
    pub(super) width: i32,
    pub(super) height: i32,
    // The scale the buffer is drawn at, and the protocol objects that carry
    // it. Shared, like the damage below: every clone is the same surface.
    scale: Rc<ScaleState>,
    // not sure this is needed
    pub(super) dirty: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // Optional layer node for layers engine rendering
    pub(super) layer_node: Option<layers::prelude::Layer>,
    // What the next draw is known to have changed; `None` when nobody said,
    // which reports the whole buffer. Shared, so a clone of the surface (a
    // `Window` holds several) records it for the draw the surface performs.
    pub(super) frame_damage: Rc<std::cell::RefCell<Option<FrameDamage>>>,
    // The style geometry the next draw's buffer is claimed at; see
    // `claim_with_next_buffer`. Shared for the same reason as `frame_damage`.
    pub(super) next_buffer_claim: Rc<std::cell::Cell<Option<BufferClaim>>>,
}

/// How a surface's buffer carries its scale: what the compositor is told, and
/// the objects it is told through.
struct ScaleState {
    /// Its `wp_fractional_scale_v1`, through which the compositor says which
    /// scale it prefers for this surface. The events land in
    /// [`surface_scale`]; the object is only held.
    fractional: RefCell<Option<WpFractionalScaleV1>>,
    /// The buffer scale and the `wp_viewport` (made the first time the
    /// surface needs one: to show a fractional buffer at its logical size, or
    /// to crop), and what they were last set to.
    buffer: RefCell<ScaledBuffer>,
    /// The crop asked for with [`BaseWaylandSurface::set_crop`].
    crop: Cell<Option<View>>,
}

impl ScaleState {
    fn new(wl_surface: &wl_surface::WlSurface) -> Self {
        Self {
            fractional: RefCell::new(track_scale(wl_surface)),
            buffer: RefCell::new(ScaledBuffer::new(wl_surface)),
            crop: Cell::new(None),
        }
    }

    fn geometry(&self) -> Option<BufferGeometry> {
        self.buffer.borrow().geometry()
    }
}

impl Drop for ScaleState {
    fn drop(&mut self) {
        if let Some(fractional) = self.fractional.take().filter(|f| f.is_alive()) {
            fractional.destroy();
        }
    }
}

/// Ask for `wl_surface`'s preferred scale, when the compositor can say.
fn track_scale(wl_surface: &wl_surface::WlSurface) -> Option<WpFractionalScaleV1> {
    AppContext::fractional_scale_manager().map(|manager| {
        manager.get_fractional_scale(wl_surface, AppContext::queue_handle(), wl_surface.clone())
    })
}

/// A surface-style size, and optionally a position, in pixels, for the
/// buffer a surface's next draw produces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferClaim {
    pub width: f64,
    pub height: f64,
    pub position: Option<(f64, f64)>,
}

/// What a surface's next draw is known to have changed.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum FrameDamage {
    /// Anything may have changed: the whole buffer is reported.
    All,
    /// Only these rects, in points, changed.
    Rects(Vec<skia_safe::Rect>),
}

impl BaseWaylandSurface {
    /// Create a new surface core
    ///
    /// The buffer is drawn at the scale the compositor prefers for this
    /// surface (see [`AppContext::surface_scale`]), and follows it when it
    /// changes; nothing about the scale is the caller's to set.
    pub fn new(wl_surface: wl_surface::WlSurface, width: i32, height: i32) -> Self {
        let layer_node = AppContext::layers_engine().map(|engine| {
            let l = engine.new_layer();
            // Every surface's root node is its own origin: it is drawn into
            // that surface's canvas alone, and `draw_scene` concatenates the
            // node's transform before rendering it. Left in the engine root's
            // flow, a second surface would be laid out beside the first and
            // paint its content off to the right of its own buffer — which is
            // what an app with a surface per output (the lock screen, the
            // greeter) gets the moment it has more than one, and what a
            // surface recreated after an output comes back gets even with one
            // on screen, since it is appended after the survivors.
            l.set_layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            });
            l.set_size(Size::points(width as f32, height as f32), None);
            l.set_position((0.0, 0.0), None);
            let _ = engine.add_layer(&l);
            l
        });
        let surface_style = AppContext::surface_style_manager()
            .map(|manager| manager.get_surface_style(&wl_surface, AppContext::queue_handle(), ()));
        let scale = Rc::new(ScaleState::new(&wl_surface));
        Self {
            wl_surface,
            skia_surface: None,
            width,
            height,
            scale,
            surface_style,
            layer_node,
            dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            frame_damage: Rc::new(std::cell::RefCell::new(None)),
            next_buffer_claim: Rc::new(std::cell::Cell::new(None)),
        }
    }

    /// Create the Skia surface using shared context
    pub fn create_skia_surface(&mut self) -> Result<(), SurfaceError> {
        use crate::app_runner::AppContext;
        use crate::rendering::SkiaContext;

        // A new buffer: whatever was sent before described the old one.
        self.skia_surface = None;
        self.scale.buffer.borrow_mut().reset();
        self.sync_scale();
        let (buffer_width, buffer_height) = self.buffer_size();

        let surface = AppContext::skia_context(|ctx| {
            ctx.create_surface(&self.wl_surface, buffer_width, buffer_height)
        });

        if let Some(result) = surface {
            // Shared context exists, use it
            self.skia_surface = Some(Rc::new(
                result.map_err(|e| SurfaceError::SkiaError(e.to_string()))?,
            ));
        } else {
            // No shared context yet - create it with this first surface
            println!(
                "No shared context yet , Creating new Skia context for surface {}",
                self.wl_surface.id()
            );
            let (new_ctx, new_surface) = SkiaContext::new(
                AppContext::display_ptr(),
                &self.wl_surface,
                buffer_width,
                buffer_height,
            )
            .map_err(|e| SurfaceError::SkiaError(e.to_string()))?;

            AppContext::set_skia_context(new_ctx);
            self.skia_surface = Some(Rc::new(new_surface));
        }

        Ok(())
    }

    /// Resize the surface and update the Skia surface dimensions.
    ///
    /// A resize to the size it already has returns without doing anything.
    /// Callers re-push their geometry on every configure and most configures
    /// move only one axis — or neither — so without this a window being
    /// dragged by one edge threw away and rebuilt the Skia render target of
    /// every surface whose size had not changed, which measured in
    /// milliseconds apiece. A change of *scale* at the same size is not this
    /// method's to catch: the next draw does, in [`Self::sync_scale`].
    pub fn resize(&mut self, width: i32, height: i32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;

        // Resizes the existing Skia surface in place (the EGL window on the
        // next draw), and re-sends the viewport for the new size.
        self.sync_scale();

        // Update layer node size if using layers engine
        if let Some(layer) = &self.layer_node {
            layer.set_size(
                layers::types::Size::points(width as f32, height as f32),
                None,
            );
            layer.engine.update(0.0);
        }
    }

    /// Bring the buffer in line with the surface's scale, logical size and
    /// crop: the buffer scale, the viewport and the buffer's size, each sent
    /// only if it moved.
    ///
    /// Called from every draw, which is how a new preferred scale reaches the
    /// buffer — the scale is the compositor's to change at any time, and the
    /// next frame is the first one that can show it. Everything here is
    /// surface state, so it lands with the commit that frame's swap makes,
    /// together with the buffer it describes.
    fn sync_scale(&self) -> ScaleSync {
        let sync = self.scale.buffer.borrow_mut().sync(
            surface_scale::surface_scale_120(&self.wl_surface.id()),
            (self.width, self.height),
            self.scale.crop.get(),
            AppContext::viewporter(),
            AppContext::queue_handle(),
        );
        if sync.buffer {
            if let (Some(skia), Some(geometry)) = (&self.skia_surface, self.scale.geometry()) {
                skia.resize(geometry.buffer.0, geometry.buffer.1);
            }
        }
        sync
    }

    /// Show `crop`'s source, a rect in the surface's own points, at its
    /// destination size in points; `None` shows the whole surface again.
    /// Surface state, landing with the surface's next commit.
    ///
    /// Returns `false` when the compositor has no `wp_viewporter`, in which
    /// case nothing is sent.
    pub(super) fn set_crop(&self, crop: Option<View>) -> bool {
        if AppContext::viewporter().is_none() {
            return crop.is_none();
        }
        self.scale.crop.set(crop);
        self.sync_scale();
        true
    }

    /// Move onto a new `wl_surface` — a popup shown again after its surface
    /// was destroyed. The old surface's scale objects go with it; the new one
    /// asks for its own preferred scale, and starts at the stand-in.
    pub(super) fn replace_wl_surface(&mut self, wl_surface: wl_surface::WlSurface) {
        AppContext::forget_surface(&self.wl_surface.id());
        if let Some(fractional) = self.scale.fractional.take().filter(|f| f.is_alive()) {
            fractional.destroy();
        }
        // Dropping the old one destroys its viewport.
        *self.scale.buffer.borrow_mut() = ScaledBuffer::new(&wl_surface);
        *self.scale.fractional.borrow_mut() = track_scale(&wl_surface);
        self.wl_surface = wl_surface;
    }

    /// The buffer's size in pixels, at the scale it was last given.
    pub fn buffer_size(&self) -> (i32, i32) {
        self.scale
            .geometry()
            .map_or((self.width, self.height), |g| g.buffer)
    }

    /// Buffer pixels per logical point, as the buffer was last given it. The
    /// factor the canvas is scaled by, and the one a point measured on this
    /// surface is worth in pixels on screen.
    pub fn buffer_scale(&self) -> f32 {
        self.scale.geometry().map_or(1.0, |g| g.scale.factor())
    }

    /// The scale the compositor prefers for this surface: see
    /// [`AppContext::surface_scale`].
    pub fn scale(&self) -> f64 {
        AppContext::surface_scale(&self.wl_surface.id())
    }

    /// The subpixel layout the buffer's pixels land on, for text to be
    /// rendered for: the layout of the one output the surface is on, turned
    /// with that output's transform.
    ///
    /// Only when the buffer is shown pixel for pixel — at the surface's own
    /// scale, not the next whole one a compositor without a viewporter
    /// shrinks, and with no crop stretching it — since subpixel text
    /// resampled by the compositor fringes. A surface across two outputs, on
    /// none yet, or on one that has not described itself gets `Unknown`:
    /// greyscale text. Re-evaluated every draw, so it follows the surface to
    /// another output on the next frame it paints there.
    fn pixel_geometry(&self, applied: BufferGeometry) -> skia_safe::PixelGeometry {
        use skia_safe::PixelGeometry;
        use smithay_client_toolkit::compositor::SurfaceData;

        let wanted_120 = surface_scale::surface_scale_120(&self.wl_surface.id());
        let one_to_one = match applied.scale {
            BufferScale::Integer(n) => n.max(0) as u32 * 120 == wanted_120,
            BufferScale::Fractional(s) => s == wanted_120,
        } && self.scale.crop.get().is_none();
        if !one_to_one {
            return PixelGeometry::Unknown;
        }
        let Some(data) = self.wl_surface.data::<SurfaceData>() else {
            return PixelGeometry::Unknown;
        };
        let mut outputs = data.outputs();
        let (Some(output), None) = (outputs.next(), outputs.next()) else {
            return PixelGeometry::Unknown;
        };
        AppContext::output_info(&output).map_or(PixelGeometry::Unknown, |info| {
            crate::rendering::pixel_geometry(info.subpixel, info.transform)
        })
    }

    /// Mark every layer this surface draws as needing a repaint, so the next
    /// draw re-records their content — and rasterises their icons and images
    /// again — at the surface's new scale.
    fn repaint_layer_tree(&self) {
        let (Some(layer), Some(engine)) = (&self.layer_node, AppContext::layers_engine()) else {
            return;
        };
        // Descendants include the root itself.
        for node in engine.node_descendants(&layer.id()) {
            if let Some(layer) = engine.get_layer(&node) {
                layer.redraw();
            }
        }
    }

    /// Get the Wayland surface
    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        &self.wl_surface
    }

    /// Get the surface style if available
    pub fn surface_style(&self) -> Option<&otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.surface_style.as_ref()
    }

    /// Set the layer node for layers engine rendering
    pub fn set_layer_node(&mut self, layer: layers::prelude::Layer) {
        self.layer_node = Some(layer);
    }

    /// Get the layer node
    pub fn layer_node(&self) -> Option<&layers::prelude::Layer> {
        self.layer_node.as_ref()
    }

    /// Get dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// Get reference to the SkiaSurface
    pub fn skia_surface(&self) -> Option<&Rc<SkiaSurface>> {
        self.skia_surface.as_ref()
    }

    /// Check if surface is configured and ready to draw
    ///
    /// Can be overridden by specific surface types if needed
    pub fn is_surface_configured(&self) -> bool {
        true
    }

    /// Take the Skia surface, so the caller can drop it — and with it the EGL
    /// resources — before destroying the `wl_surface` underneath.
    ///
    /// Surface types that own an xdg role do this inside their own `destroy`;
    /// this is for the ones that have no role of their own to tear down, such
    /// as a drag icon.
    pub fn take_skia_surface(&mut self) -> Option<Rc<SkiaSurface>> {
        self.skia_surface.take()
    }

    /// Take the surface style, in the same teardown order.
    pub fn take_surface_style(&mut self) -> Option<otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.surface_style.take()
    }

    /// Check if surface should allow drawing
    pub fn can_draw(&self) -> bool {
        self.is_surface_configured()
    }

    /// Draw on the surface using the shared Skia context
    ///
    /// The draw_fn callback is called for custom rendering.
    /// Current buffer size in logical points, as passed to `new` or `resize`.
    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        use crate::app_runner::AppContext;

        if !self.can_draw() {
            return;
        }

        let Some(surface) = self.skia_surface.as_ref() else {
            return;
        };

        // A new scale (or size) the compositor asked for since the last frame
        // reaches the buffer here, with nothing of the old buffer kept.
        let sync = self.sync_scale();
        if sync.scale {
            self.repaint_layer_tree();
        }
        if sync.buffer || sync.scale {
            self.mark_frame_damage_all();
        }
        let Some(applied) = self.scale.geometry() else {
            return;
        };
        // Components rasterising from `AppContext::scale_factor` in here
        // answer for this surface.
        let _drawing = surface_scale::DrawingScope::enter(surface_scale::surface_scale_120(
            &self.wl_surface.id(),
        ));
        surface.set_pixel_geometry(self.pixel_geometry(applied));

        AppContext::skia_context(|ctx| {
            // Draw custom content
            surface.draw(ctx, applied.scale.factor(), |canvas| {
                draw_fn(canvas);
            });

            // Ask to be told when this frame reaches the screen. `wl_surface
            // .frame` is double-buffered state, so the request has to be made
            // before the commit that carries it — and that commit is the one
            // eglSwapBuffers makes below, not one of ours. A surface that
            // already has a request outstanding — a frame loop's — is not
            // asked twice, so its callback still runs once per frame.
            AppContext::request_throttled_frame(&self.wl_surface);
            AppContext::mark_accessibility_stale();

            // Queued now, the claim goes out in the swap's own flush, just
            // ahead of the buffer it describes.
            if let (Some(style), Some(claim)) = (&self.surface_style, self.next_buffer_claim.take())
            {
                style.set_size(claim.width, claim.height);
                if let Some((x, y)) = claim.position {
                    style.set_position(x, y);
                }
            }

            // Present the frame. eglSwapBuffers attaches the buffer, damages it
            // and commits, so committing again here would only ask the
            // compositor to recomposite the output for a surface that has
            // nothing new on it. Damage the application reported for this
            // frame goes out with the swap instead of the whole buffer.
            match self.frame_damage.borrow_mut().take() {
                Some(FrameDamage::Rects(rects)) => {
                    let scale = applied.scale.factor();
                    let bounds = skia_safe::IRect::from_wh(applied.buffer.0, applied.buffer.1);
                    let rects: Vec<skia_safe::IRect> = rects
                        .iter()
                        .filter_map(|r| {
                            // Rounded outwards, so a fractional edge is still
                            // covered, and kept inside the buffer, which a
                            // rounded-up edge at a fractional scale can leave.
                            skia_safe::IRect::intersect(
                                &skia_safe::IRect::from_ltrb(
                                    (r.left * scale).floor() as i32,
                                    (r.top * scale).floor() as i32,
                                    (r.right * scale).ceil() as i32,
                                    (r.bottom * scale).ceil() as i32,
                                ),
                                &bounds,
                            )
                        })
                        .collect();
                    surface.swap_buffers_with_damage(ctx, &rects);
                }
                None | Some(FrameDamage::All) => surface.swap_buffers(ctx),
            }
        });
    }

    /// Record that the next draw changes `rects` (in points), and nothing else
    /// that has not already been recorded. Combined with earlier rects until
    /// that draw consumes them; if the whole surface is already recorded as
    /// changed, it stays so.
    pub fn add_frame_damage(&self, rects: &[skia_safe::Rect]) {
        let mut damage = self.frame_damage.borrow_mut();
        match damage.as_mut() {
            Some(FrameDamage::All) => {}
            Some(FrameDamage::Rects(pending)) => pending.extend_from_slice(rects),
            None => *damage = Some(FrameDamage::Rects(rects.to_vec())),
        }
    }

    /// Claim a style size (and position) for the buffer the next draw produces.
    ///
    /// The style applies a size the moment it arrives, and the swap sends its
    /// buffer on its own, so neither side of a draw is the place to send it:
    /// before, the old buffer is drawn stretched to the new size; after, the
    /// new buffer is drawn at the old one for a frame. Held here, it goes out
    /// inside the next draw, together with the buffer it fits. A later claim
    /// before that draw replaces this one.
    pub fn claim_with_next_buffer(&self, claim: BufferClaim) {
        self.next_buffer_claim.set(Some(claim));
    }

    /// Record that the next draw may change anything: it reports the whole
    /// buffer, whatever narrower damage was recorded before or after it.
    pub fn mark_frame_damage_all(&self) {
        *self.frame_damage.borrow_mut() = Some(FrameDamage::All);
    }

    /// Whether a frame committed on this surface has yet to reach the screen.
    ///
    /// Content that redraws continuously — an animation reading the clock —
    /// should paint only when this is false, so it runs at the compositor's
    /// pace rather than as fast as it can submit.
    pub fn frame_in_flight(&self) -> bool {
        AppContext::frame_in_flight(&self.wl_surface.id())
    }

    /// Render the layer node if one is assigned
    ///
    /// This can be called to render layers content onto the surface canvas.
    pub fn render_layer_node(&self, canvas: &skia_safe::Canvas) {
        use crate::app_runner::AppContext;

        let Some(engine) = AppContext::layers_engine() else {
            return;
        };

        let Some(layer) = &self.layer_node else {
            return;
        };

        // Whatever the app changed before painting is part of this frame.
        AppContext::flush_layers();

        // Render the assigned layer node from the shared engine
        layers::prelude::draw_scene(canvas, engine.scene(), layer.id());
    }

    /// Register a callback to be called on every compositor frame
    ///
    /// This hooks into the Wayland frame callback mechanism, ensuring
    /// your callback is synchronized with the compositor's refresh rate.
    ///
    /// # Example
    /// ```ignore
    /// surface.on_frame(|| {
    ///     println!("Frame rendered!");
    ///     // Render your content here
    /// });
    /// ```
    pub fn on_frame<F>(&self, callback: F)
    where
        F: FnMut() + 'static,
    {
        use crate::app_runner::AppContext;
        use wayland_client::Proxy;

        let surface_id = self.wl_surface.id();
        AppContext::register_frame_callback(surface_id, callback);
        AppContext::register_frame_loop(&self.wl_surface);
    }

    /// Check if surface style protocol is available for this surface
    pub fn has_surface_style(&self) -> bool {
        self.surface_style.is_some()
    }

    /// Get reference to the surface style manager
    pub fn surface_style_manager(
        &self,
    ) -> Option<&otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1> {
        use crate::app_runner::AppContext;
        AppContext::surface_style_manager()
    }
}

/// Error type for surface operations
#[derive(Debug)]
pub enum SurfaceError {
    /// Failed to create the surface
    CreationFailed,
    /// Surface not yet configured by compositor
    NotConfigured,
    /// Skia rendering error
    SkiaError(String),
    /// Wayland protocol error
    WaylandError(String),
    /// sc_layer protocol not available
    SceneSurface,
    /// Failed to resize surface
    ResizeFailed,
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SurfaceError::CreationFailed => write!(f, "Failed to create surface"),
            SurfaceError::NotConfigured => write!(f, "Surface not yet configured"),
            SurfaceError::SkiaError(e) => write!(f, "Skia error: {}", e),
            SurfaceError::WaylandError(e) => write!(f, "Wayland error: {}", e),
            SurfaceError::SceneSurface => write!(f, "scene_surface protocol not available"),
            SurfaceError::ResizeFailed => write!(f, "Failed to resize surface"),
        }
    }
}

impl std::error::Error for SurfaceError {}
