mod application_window;

pub mod resize;

use smithay_client_toolkit::seat::pointer::PointerEvent;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use wayland_client::protocol::wl_seat;

use crate::app_runner::AppContext;
pub use crate::protocols::otto_surface_style_v1;
use crate::surfaces::{SurfaceError, ToplevelSurface};

pub use application_window::{ApplicationWindow, WindowLayout};

/// Default layer augmentation - applies rounded corners
fn default_layer_augmentation(layer: &otto_surface_style_v1::OttoSurfaceStyleV1) {
    layer.set_corner_radius(16.0);
    layer.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
}

type CanvasDrawFn = Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>;

/// Window component using ToplevelSurface
///
/// This is a high-level window component that uses ToplevelSurface for
/// surface management while providing a simple API for window content.
///
/// By default, windows have rounded corners (12px radius). Use `on_layer()`
/// to customize or override the default layer augmentation.
///
/// Window uses the shared layers rendering engine from AppContext.
/// Assign a Layer node to this window to render it.
///
/// Window is Clone-able, allowing it to be shared across the application.
#[derive(Clone)]
pub struct Window {
    #[allow(clippy::arc_with_non_send_sync)]
    surface: Arc<RwLock<Option<ToplevelSurface>>>,
    background_color: Arc<RwLock<skia_safe::Color>>,
    title: Arc<RwLock<String>>,
    on_draw_fn: CanvasDrawFn,
    /// Whether the application asked for a blurred backdrop. The blur is only
    /// actually asked of the compositor while the window is focused — see
    /// [`Window::set_background_blur`].
    blur_wanted: Arc<AtomicBool>,
}

impl Window {
    /// Create a new window with ToplevelSurface
    ///
    /// Uses AppContext to access all required Wayland states.
    /// Automatically registers with AppRunner to handle configuration.
    /// Creates sc_layer immediately if available, with default rounded corners.
    pub fn new(title: &str, width: i32, height: i32) -> Result<Self, SurfaceError> {
        // Get all required states from AppContext

        let surface = ToplevelSurface::new(title, width, height)?;

        // Apply default layer styling immediately
        if let Some(surface_style) = surface.surface_style() {
            eprintln!("Applying corner radius to window surface style");
            default_layer_augmentation(surface_style);
        } else {
            eprintln!("Warning: No surface style available - window will not have rounded corners");
        }

        // Named rather than read back off the window below: `apply_background`
        // takes the write lock on `background_color`, so passing it a value
        // read through that same lock deadlocks before the window ever maps.
        let background = skia_safe::Color::from_rgb(245, 245, 245);

        let window = Self {
            #[allow(clippy::arc_with_non_send_sync)]
            surface: Arc::new(RwLock::new(Some(surface))),
            background_color: Arc::new(RwLock::new(background)),
            title: Arc::new(RwLock::new(title.to_string())),
            on_draw_fn: Arc::new(Mutex::new(None)),
            blur_wanted: Arc::new(AtomicBool::new(false)),
        };
        // Hand the default to the compositor too, so the background is carried
        // by the style from the first frame and a window that never calls
        // `set_background` looks the same either way.
        window.apply_background(background);

        // Auto-register configure handler now that Window is Clone.
        //
        // Every registered handler runs for every configure, so each window
        // has to recognise its own: an application with two of them would
        // otherwise resize both to whatever size the last configure carried.
        let window_clone = window.clone();
        AppContext::register_configure_handler(move || {
            if let Some((surface_id, configure, serial)) = AppContext::current_surface_configure() {
                if window_clone.surface_id() != Some(surface_id) {
                    return;
                }
                window_clone.on_configure(configure, serial);
            }
        });

        // Register window for automatic updates
        AppContext::register_window(window.clone());

        Ok(window)
    }

    /// Set the background color
    pub fn with_background(self, color: impl Into<skia_safe::Color>) -> Self {
        self.apply_background(color.into());
        self
    }

    /// Set the background color (mutable version)
    pub fn set_background(&mut self, color: impl Into<skia_safe::Color>) {
        self.apply_background(color.into());
        // Only the painted fallback needs a redraw; a style background is the
        // compositor's to composite and lands without this client drawing.
        if self.surface_style().is_none() {
            if let Ok(surface_guard) = self.surface.read() {
                if let Some(ref surface) = *surface_guard {
                    surface.request_frame();
                }
            }
        }
    }

    /// The background is a *style* on the surface wherever the compositor can
    /// carry one: it is then composited under the window's content without
    /// this client painting a full-window rect every frame, and — for a
    /// translucent colour over `BlendMode::BackgroundBlur` — it tints the
    /// blurred backdrop rather than a flat sample of it.
    ///
    /// The painted fallback is kept for the case where the protocol is absent
    /// (running under another compositor), where clearing the canvas is the
    /// only way to have a background at all.
    fn apply_background(&self, color: skia_safe::Color) {
        if let Ok(mut bg_guard) = self.background_color.write() {
            *bg_guard = color;
        }
        if let Some(style) = self.surface_style() {
            let scale = 1.0 / 255.0;
            style.set_background_color(
                color.r() as f64 * scale,
                color.g() as f64 * scale,
                color.b() as f64 * scale,
                color.a() as f64 * scale,
            );
        }
    }

    /// Set a custom content drawing function
    pub fn with_on_draw<F>(self, draw_fn: F) -> Self
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.on_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
        self
    }

    /// Set a custom content drawing function (mutable version)
    pub fn on_draw<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.on_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
    }

    /// Assign a layer node to render in this window
    ///
    /// The layer and all its children will be rendered when the window draws.
    ///
    /// # Example
    /// ```no_run
    /// let layer = LayerFrame::new();
    /// layer.set_size(200.0, 100.0);
    /// window.set_layer_node(layer.layer().clone());
    /// ```
    pub fn set_layer_node(&mut self, layer: layers::prelude::Layer) {
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                surface.set_layer_node(layer);
            }
        }
    }

    /// Get the layer node assigned to this window
    pub fn layer_node(&self) -> Option<layers::prelude::Layer> {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                return surface.layer_node().cloned();
            }
        }
        None
    }

    /// Get direct access to the surface style for configuration
    ///
    /// Returns None if surface style was not available when the window was created.
    ///
    /// # Example
    /// ```no_run
    /// if let Some(surface_style) = window.surface_style() {
    ///     surface_style.set_corner_radius(24.0);
    ///     surface_style.set_opacity(0.9);
    /// }
    /// ```
    pub fn surface_style(&self) -> Option<otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.surface.read().ok()?.as_ref()?.surface_style().cloned()
    }

    /// Ask the compositor to blur what is behind this window.
    ///
    /// The blur is dropped while the window is unfocused and restored when it
    /// comes back: an unfocused window is chrome the user is not looking at,
    /// and a full-window gaussian per frame is the most expensive thing the
    /// compositor does on its behalf. The request is remembered, so a window
    /// that asked for blur once gets it back on the next activate without the
    /// application doing anything.
    pub fn set_background_blur(&self, enabled: bool) {
        self.blur_wanted.store(enabled, Ordering::Relaxed);
        self.apply_blend_mode();
    }

    /// Whether the application asked for a blurred backdrop, regardless of
    /// whether the window is focused right now.
    pub fn background_blur(&self) -> bool {
        self.blur_wanted.load(Ordering::Relaxed)
    }

    /// Push the blend mode the window's current state calls for.
    fn apply_blend_mode(&self) {
        let Some(style) = self.surface_style() else {
            return;
        };
        let blurred = self.blur_wanted.load(Ordering::Relaxed) && self.is_activated();
        style.set_blend_mode(if blurred {
            otto_surface_style_v1::BlendMode::BackgroundBlur
        } else {
            otto_surface_style_v1::BlendMode::Normal
        });
    }

    /// Internal: Handle window configure event
    fn on_configure(&self, configure: WindowConfigure, serial: u32) {
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                let _ = surface.handle_configure(configure, serial);
            }
        }
        // The configure is where activation arrives, so the blur follows it.
        self.apply_blend_mode();
        self.render();
    }

    /// Render the window content
    fn render_with<F>(&self, render_extra: F)
    where
        F: FnOnce(),
    {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                if !surface.is_configured() {
                    return;
                }

                let on_draw_fn = self.on_draw_fn.clone();
                // Where the compositor carries the background as a style, the
                // buffer starts empty: painting the colour here as well would
                // put an opaque copy of it *over* the blurred backdrop the
                // style is composited against.
                let background_color = if surface.surface_style().is_some() {
                    skia_safe::Color::TRANSPARENT
                } else {
                    self.background_color
                        .read()
                        .ok()
                        .map(|c| *c)
                        .unwrap_or(skia_safe::Color::WHITE)
                };

                surface.draw(|canvas| {
                    canvas.clear(background_color);

                    // Draw custom content on top if provided
                    if let Ok(mut draw_fn_guard) = on_draw_fn.lock() {
                        if let Some(ref mut content_fn) = *draw_fn_guard {
                            content_fn(canvas);
                        }
                    }
                });

                // Render extra content (e.g., subsurfaces)
                render_extra();
            }
        }
    }

    /// Render the window content
    fn render(&self) {
        self.render_with(|| {});
    }

    /// Update the window - render if dirty
    pub(crate) fn update(&self) {
        if let Some(surface) = self.surface() {
            if surface.is_dirty() {
                // Painting again before the last frame has been presented
                // does not get it on screen any sooner — it queues a buffer
                // the compositor has not asked for, and `eglSwapBuffers`
                // then blocks in the driver waiting for one to come free,
                // stalling the whole event loop (input included) for tens of
                // milliseconds. The window stays dirty and paints on the
                // loop iteration after the frame callback instead.
                if surface.frame_in_flight() {
                    return;
                }
                self.render();
                surface.clear_dirty();
            }
        }
    }

    /// Get the underlying ToplevelSurface
    pub fn surface(&self) -> Option<ToplevelSurface> {
        self.surface.read().ok()?.clone()
    }

    /// This window's own `wl_surface` id, which is how it is told apart from
    /// every other window in the process. `None` once it has been closed.
    pub fn surface_id(&self) -> Option<wayland_client::backend::ObjectId> {
        use wayland_client::Proxy;
        Some(self.surface()?.wl_surface().id())
    }

    /// Whether this window still has a surface — that is, whether it has not
    /// been closed.
    pub fn is_alive(&self) -> bool {
        self.surface
            .read()
            .ok()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Take this window off the screen for good.
    ///
    /// Dropping the surface destroys the `xdg_toplevel` and its `wl_surface`,
    /// which is what unmaps the window; every clone of this handle goes with
    /// it, since they all share the one surface. The handle stays valid and
    /// inert afterwards — drawing and updating a closed window do nothing —
    /// so a callback still holding a clone is not a problem.
    ///
    /// This is how a *secondary* window goes away. The window an application
    /// *is* closes by ending the application.
    pub fn close(&self) {
        if let Some(id) = self.surface_id() {
            AppContext::unregister_close_handler(&id);
        }
        if let Ok(mut guard) = self.surface.write() {
            guard.take();
        }
    }

    /// Handle the compositor's close request for this window — the titlebar's
    /// close control, or a "quit" from the dock — instead of letting it end
    /// the application. See [`AppContext::register_close_handler`].
    pub fn on_close_request<F>(&self, handler: F)
    where
        F: FnMut() + 'static,
    {
        if let Some(id) = self.surface_id() {
            AppContext::register_close_handler(id, handler);
        }
    }

    /// Check if the window is configured
    pub fn is_configured(&self) -> bool {
        self.surface
            .read()
            .ok()
            .and_then(|s| s.as_ref().map(|surf| surf.is_configured()))
            .unwrap_or(false)
    }

    /// Get window dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        self.surface
            .read()
            .ok()
            .and_then(|s| s.as_ref().map(|surf| surf.dimensions()))
            .unwrap_or((0, 0))
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> Option<wayland_client::protocol::wl_surface::WlSurface> {
        let guard = self.surface.read().ok()?;
        guard.as_ref().map(|s| s.wl_surface().clone())
    }

    /// Register a pointer event handler for this window
    /// The callback receives all pointer events when they occur
    ///
    /// # Example
    /// ```no_run
    /// window.on_pointer_event(|events| {
    ///     for event in events {
    ///         match &event.kind {
    ///             PointerEventKind::Press { button, serial, .. } => {
    ///                 // Handle button press
    ///             }
    ///             _ => {}
    ///         }
    ///     }
    /// });
    /// ```
    pub fn on_pointer_event<F>(&self, mut callback: F)
    where
        F: FnMut(&[PointerEvent]) + 'static,
    {
        // Clone the window Arc to check surface on each event
        let window_clone = self.clone();

        AppContext::register_pointer_callback(move |events| {
            // Get our surface ID dynamically each time
            if let Some(our_wl_surface) = window_clone.wl_surface() {
                use wayland_client::Proxy;
                // Filter to only events for our surface
                let our_events: Vec<&PointerEvent> = events
                    .iter()
                    .filter(|e| e.surface.id() == our_wl_surface.id())
                    .collect();

                if !our_events.is_empty() {
                    // eprintln!("Window got {} pointer events", our_events.len());
                    let borrowed_events: Vec<PointerEvent> =
                        our_events.iter().map(|&e| e.clone()).collect();
                    callback(&borrowed_events);
                }
            }
        });
    }

    /// Start an interactive window move
    /// Call this in response to a pointer button press to make the window draggable
    ///
    /// # Arguments
    /// * `seat` - The seat that initiated the move
    /// * `serial` - The serial from the pointer button press event
    ///
    /// # Example
    /// ```no_run
    /// window.on_pointer_event(|events| {
    ///     for event in events {
    ///         if let PointerEventKind::Press { serial, .. } = event.kind {
    ///             // Start window move when pressed
    ///             window.start_move(seat, serial);
    ///         }
    ///     }
    /// });
    /// ```
    /// Begin a compositor-driven resize from `edge`.
    ///
    /// Call it from a pointer press that landed on an edge — see
    /// [`resize::edge_at`]. The compositor takes the pointer from there and
    /// drives the resize until the button is released.
    pub fn start_resize(&self, seat: &wl_seat::WlSeat, serial: u32, edge: resize::ResizeEdge) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.xdg_window().resize(seat, serial, edge.to_xdg());
            }
        }
    }

    pub fn start_move(&self, seat: &wl_seat::WlSeat, serial: u32) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.xdg_window().move_(seat, serial);
            }
        }
    }

    /// Whether the compositor's last configure said the window is maximized.
    pub fn is_maximized(&self) -> bool {
        self.surface
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|s| s.is_maximized()))
            .unwrap_or(false)
    }

    /// Whether the compositor's last configure said this is the focused
    /// window. Chrome that dims itself when the focus moves away — the title,
    /// the traffic lights — reads this each time it draws.
    pub fn is_activated(&self) -> bool {
        self.surface
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|s| s.is_activated()))
            .unwrap_or(false)
    }

    /// Ask the compositor to minimize the window — what the yellow traffic
    /// light does.
    pub fn minimize(&self) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.xdg_window().set_minimized();
            }
        }
    }

    /// Maximize, or restore if already maximized — what the green traffic
    /// light does. The compositor answers with a configure, so the new size
    /// arrives the same way any other resize does.
    pub fn toggle_maximized(&self) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                if surface.is_maximized() {
                    surface.xdg_window().unset_maximized();
                } else {
                    surface.xdg_window().set_maximized();
                }
            }
        }
    }
    pub fn request_frame(&self) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.request_frame();
            }
        }
    }
    pub fn title(&self) -> String {
        self.title
            .read()
            .ok()
            .map(|t| t.clone())
            .unwrap_or_default()
    }
    /// The smallest size the compositor should allow, in logical points.
    ///
    /// Unset by default — a window can be resized down to whatever the
    /// compositor permits — so set it if the layout has a genuine floor.
    pub fn set_min_size(&self, width: u32, height: u32) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.xdg_window().set_min_size(Some((width, height)));
            }
        }
    }

    /// The largest size the compositor should allow, in logical points.
    ///
    /// Set it to the same size as the minimum for a window that does not
    /// resize at all — a panel whose layout is fixed, rather than a document
    /// window the user sizes to the work.
    pub fn set_max_size(&self, width: u32, height: u32) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.xdg_window().set_max_size(Some((width, height)));
            }
        }
    }

    pub fn set_title(&mut self, title: &str) {
        if let Ok(mut title_guard) = self.title.write() {
            *title_guard = title.to_string();
        }
    }
}
