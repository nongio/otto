//! The desk's overflow panel on screen: a surface of its own above the
//! windows.
//!
//! The desk is a layer surface on the bottom layer, so anything drawn into
//! it is covered by every window and clipped to the desk. The panel is
//! instead a card on an overlay: a transparent layer surface on the top
//! layer, the same size as the desk's and at the same place, so the desk's
//! coordinates are the overlay's too. The overlay is there for three things.
//! It carries the card above the windows. It takes the pointer everywhere
//! the card is not, which is how a click away closes the panel without
//! landing on whatever is underneath. And it can hold the keyboard while the
//! panel is clicked, on demand, so another window can still take it, which
//! closes the panel.
//!
//! The card is a [`PlacedSurface`] on the overlay: the compositor blurs and
//! tints what is behind it, rounds it and casts its shadow, exactly as it
//! does for the palette's card and Peek's. Its entrance and exit run here,
//! one frame at a time: each frame the card is moved and sized to where the
//! panel is on its way out of the tile (or back into it), and painted again
//! at that size. A paint waits for the compositor to show the last one, so
//! the animation is paced by the display, and the blur and the shadow follow
//! the card because they follow its surface.
//!
//! Not an `xdg_popup`. A popup of the desk is hit-tested only through the
//! desk's own input region and only after every window, so over a window it
//! would be seen and not clickable; and a grabbed popup keeps the pointer
//! grab, which refuses the drag that carries an item out of the panel.

// Rust guideline compliant 2026-02-21

use otto_kit::components::scroll::{Paint, PlacedSurface};
use otto_kit::protocols::otto_surface_style_v1::BlendMode;
use otto_kit::surfaces::layer_shell::{Anchor, KeyboardInteractivity, Layer};
use otto_kit::surfaces::LayerShellSurface;
use smithay_client_toolkit::seat::pointer::PointerEvent;
use wayland_client::backend::ObjectId;
use wayland_client::Proxy;

use super::desk_overflow::OverflowShown;
use super::*;

/// The layer surface a menu over the panel hangs off.
pub(super) type LayerSurfaceProxy =
    smithay_client_toolkit::reexports::protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

/// The overlay's layer-shell namespace.
const NAMESPACE: &str = "otto-files-desk-overflow";

/// The card's shadow: opacity, and blur and drop in points. The palette's
/// card's, a little lighter, since this one sits close to the desk's icons.
const SHADOW: (f64, f64, f64) = (0.26, 24.0, 8.0);

/// The overflow panel's surfaces while it is on screen.
pub(super) struct OverflowSurface {
    /// The transparent overlay the card sits on.
    overlay: LayerShellSurface,
    /// The size the overlay's transparent ground was painted at. The
    /// compositor hit-tests a layer surface against its buffer, so the ground
    /// has to cover the surface, not merely be there.
    ground: Option<(i32, i32)>,
    /// The panel itself.
    card: Option<PlacedSurface>,
    /// Bumped whenever what the card shows may have changed, and handed to
    /// the card as the key of its paint.
    generation: u64,
    /// Where the card is, in the desk's coordinates, and how far the panel
    /// at rest is scaled to fill it: what the pointer needs to put a point
    /// on the card back into the desk's coordinates.
    placed: Option<(Rect, Rect)>,
    /// Whether the overlay and the card have stopped taking the pointer: a
    /// panel on its way out is out of the way at once.
    released: bool,
}

impl OverflowSurface {
    /// Open the overlay on the output the desk is on.
    fn open(desk: &wl_surface::WlSurface) -> Result<Self, otto_kit::surfaces::SurfaceError> {
        let output = AppContext::surface_output(&desk.id());
        // Anchored to every edge with no exclusive zone, exactly like the
        // desk: the compositor gives both the same usable area, so the two
        // share their coordinates.
        let overlay =
            LayerShellSurface::with_setup_on(Layer::Top, NAMESPACE, 0, 0, output.as_ref(), |l| {
                l.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
                l.set_exclusive_zone(0);
                l.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
            })?;
        Ok(Self {
            overlay,
            ground: None,
            card: None,
            generation: 0,
            placed: None,
            released: false,
        })
    }

    /// Paint the transparent ground once the overlay has been sized, and
    /// again if it is resized. Returns whether the overlay is ready.
    fn sync_ground(&mut self) -> bool {
        if !self.overlay.is_configured() {
            return false;
        }
        let size = self.overlay.dimensions();
        if self.ground != Some(size) {
            self.overlay.draw(|canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
            });
            self.ground = Some(size);
        }
        true
    }

    /// Stop taking the pointer anywhere: the panel is going, and whatever is
    /// under it — the desk, another window, the target of a drag just
    /// started out of it — gets the pointer straight away.
    pub(super) fn release_input(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        if let Some(card) = self.card.as_mut() {
            card.set_takes_input(false);
        }
        let surface = self.overlay.wl_surface();
        let region = AppContext::compositor_state()
            .wl_compositor()
            .create_region(AppContext::queue_handle(), ());
        surface.set_input_region(Some(&region));
        region.destroy();
        surface.commit();
    }

    /// The overlay's surface, which takes the keyboard when it is clicked.
    pub(super) fn overlay_id(&self) -> ObjectId {
        self.overlay.wl_surface().id()
    }

    /// The overlay's layer surface, which a menu opened over the panel hangs
    /// off so it is above the panel too.
    pub(super) fn overlay_layer(&self) -> LayerSurfaceProxy {
        self.overlay.layer_surface()
    }

    /// Where an event on one of the panel's surfaces is, in the desk's
    /// coordinates: on the card, `Some(Some(point))`; on the overlay around
    /// it, `Some(None)`; on neither, `None`.
    fn locate(&self, event: &PointerEvent) -> Option<Option<(f32, f32)>> {
        let id = event.surface.id();
        if id == self.overlay_id() {
            return Some(None);
        }
        let card = self.card.as_ref()?;
        if id != card.wl_surface().id() {
            return None;
        }
        let (placed, resting) = self.placed?;
        // The card shows the panel at rest scaled evenly into `placed`, from
        // its top-left corner.
        let scale = (placed.width() / resting.width().max(1.0)).max(f32::EPSILON);
        let x = resting.left + event.position.0 as f32 / scale;
        let y = resting.top + event.position.1 as f32 / scale;
        Some(Some((x, y)))
    }

    /// Take the card and the overlay down. Destroying the overlay hands the
    /// keyboard back to the desk if it had it.
    fn close(mut self) {
        // The card first: it is the overlay's child.
        self.card.take();
        self.overlay.destroy();
    }
}

/// The card's material, handed to the compositor: the blur behind it, its
/// rounding and its shadow.
fn style_card(card: &PlacedSurface) {
    let Some(style) = card.style() else {
        return;
    };
    let scale = AppContext::fractional_scale();
    style.set_shadow(
        SHADOW.0,
        SHADOW.1 * scale,
        0.0,
        SHADOW.2 * scale,
        0.0,
        0.0,
        0.0,
    );
    style.set_blend_mode(if otto_kit::frosting::enabled() {
        BlendMode::BackgroundBlur
    } else {
        BlendMode::Normal
    });
}

impl FilesApp {
    /// Bring the overflow panel's surfaces into line with the browser: open
    /// them when the panel opens, move and paint the card on every frame of
    /// its entrance and exit and whenever what it shows changes, and take
    /// them down once it has gone back into its tile.
    ///
    /// `changed` says the desk repainted this pass, which may mean the panel
    /// shows something new: a selection, a thumbnail, a rename's caret.
    pub(super) fn sync_overflow_surface(&mut self, changed: bool) {
        let Some(desk) = self.window.as_ref().and_then(Window::wl_surface) else {
            return;
        };
        let mut browser = self.state.lock().unwrap();
        browser.settle_overflow_panel();
        let shown = browser.overflow_shown();
        let mut slot = self.overflow_surface.borrow_mut();

        let Some(shown) = shown else {
            if let Some(surface) = slot.take() {
                surface.close();
            }
            self.overflow_focus_seen = false;
            return;
        };
        if slot.is_none() {
            if shown.closing {
                // Nothing on screen to take back into the tile.
                return;
            }
            match OverflowSurface::open(&desk) {
                Ok(surface) => *slot = Some(surface),
                Err(err) => {
                    tracing::warn!(?err, "no overlay for the desk's overflow panel");
                    return;
                }
            }
        }
        let Some(surface) = slot.as_mut() else {
            return;
        };
        if shown.closing {
            surface.release_input();
        }
        if !surface.sync_ground() {
            return;
        }

        let resting = shown.resting();
        let rect = shown.rect();
        if surface.card.is_none() {
            let Ok(mut card) = PlacedSurface::new(&surface.overlay.wl_surface(), rect) else {
                return;
            };
            style_card(&card);
            card.set_takes_input(!surface.released);
            surface.card = Some(card);
        }
        let scale = rect.width() / resting.width().max(1.0);
        let scrolling = browser
            .overflow_panel
            .as_ref()
            .is_some_and(|session| session.scroll.is_animating());
        if changed || shown.animating() || scrolling {
            surface.generation = surface.generation.wrapping_add(1);
        }
        let key = surface.generation;
        let Some(card) = surface.card.as_mut() else {
            return;
        };
        if card.set_rect(rect) {
            // The pointer is hit-tested at the subsurface's position, which
            // is the overlay's state and lands with the overlay's commit.
            surface.overlay.wl_surface().commit();
        }
        if let Some(style) = card.style() {
            style.set_corner_radius((view::OVERFLOW_PANEL_RADIUS * scale) as f64);
            style.set_opacity(shown.opacity() as f64);
        }
        let paint = card.paint(key, |canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            canvas.save();
            canvas.scale((scale, scale));
            canvas.translate((-resting.left, -resting.top));
            paint_panel(&mut browser, canvas, shown);
            canvas.restore();
        });
        // The pointer is mapped through what the card shows, so the rect is
        // taken once a paint has put it there.
        if paint == Paint::Painted || surface.placed.is_none() {
            surface.placed = Some((rect, resting));
        }
    }

    /// Close the overflow panel when the keyboard has left this process
    /// altogether: another window took it. Run every update while the panel
    /// is open. A panel opened without the keyboard — by a screen reader,
    /// say — waits until the keyboard has been here once.
    pub(super) fn follow_overflow_focus(&mut self) {
        let open = self.state.lock().unwrap().overflow_panel.is_some();
        if !open {
            self.overflow_focus_seen = false;
            return;
        }
        if AppContext::keyboard_focus().is_some() {
            self.overflow_focus_seen = true;
            return;
        }
        if self.overflow_focus_seen {
            self.overflow_focus_seen = false;
            self.state.lock().unwrap().close_overflow_panel();
        }
    }

    /// Whether the keyboard is on the overflow panel's overlay.
    pub(super) fn overflow_has_keyboard(&self) -> bool {
        let focus = AppContext::keyboard_focus();
        self.overflow_surface
            .borrow()
            .as_ref()
            .is_some_and(|surface| focus == Some(surface.overlay_id()))
    }

    /// The overflow panel's pointer: the card's events, put into the desk's
    /// coordinates and handed to the desk's own pointer handling, and a press
    /// on the overlay around the card, which closes the panel. Registered
    /// once, on the desk, for whichever panel is up.
    pub(super) fn install_overflow_pointer(&self, window: &Window, context_menu: ContextMenu) {
        let state = Arc::clone(&self.state);
        let surface = Rc::clone(&self.overflow_surface);
        let modifiers = Arc::clone(&self.modifiers);
        let window = window.clone();

        AppContext::register_pointer_callback(move |events| {
            for event in events {
                let located = surface
                    .borrow()
                    .as_ref()
                    .and_then(|surface| surface.locate(event));
                let Some(located) = located else {
                    continue;
                };
                let Some((x, y)) = located else {
                    // The overlay around the card: a press there is a press
                    // away from the panel. Over it the pointer is over
                    // nothing of ours, and wears the ordinary arrow.
                    match event.kind {
                        PointerEventKind::Press { .. } => {
                            state.lock().unwrap().close_overflow_panel();
                            if let Some(surface) = surface.borrow_mut().as_mut() {
                                surface.release_input();
                            }
                            AppContext::request_wakeup();
                        }
                        PointerEventKind::Enter { .. } => {
                            AppContext::set_cursor_shape(CursorShape::Default);
                        }
                        _ => {}
                    }
                    continue;
                };
                let mut moved = event.clone();
                moved.position = (x as f64, y as f64);
                let mods = *modifiers.lock().unwrap();
                let after = state.lock().unwrap().on_pointer(&moved, mods, &window);
                match after {
                    After::Next
                    | After::Stop
                    | After::GroupMenu { .. }
                    | After::LocationMenu { .. }
                    | After::FilterMenu { .. } => {}
                    After::Drag(drag) => {
                        // An item carried out of the panel goes wherever it
                        // is dropped; the panel goes back into its tile and
                        // out of the way first, so the drop lands on what is
                        // under it.
                        state.lock().unwrap().close_overflow_panel();
                        if let Some(surface) = surface.borrow_mut().as_mut() {
                            surface.release_input();
                        }
                        super::pointer::start_drag(&window, drag);
                    }
                    After::Menu(menu) => {
                        let layer = surface
                            .borrow()
                            .as_ref()
                            .map(OverflowSurface::overlay_layer);
                        super::pointer::show_context_menu_over(
                            &window,
                            layer,
                            &context_menu,
                            &state,
                            menu,
                        );
                    }
                }
                AppContext::request_wakeup();
            }
        });
    }
}

/// Paint the panel at rest in the desk's coordinates: its ground, its
/// items, its bar, and a rename's field when one of its items is being
/// renamed.
fn paint_panel(browser: &mut Browser, canvas: &skia_safe::Canvas, shown: OverflowShown) {
    let theme = browser.theme();
    let title = browser.title();
    if let Some(scroll) = browser.overflow_scroll() {
        let mut frame = browser.frame(&theme, &title);
        // What is on screen, which while closing is what it was showing
        // when it started to close rather than the desk as it is now.
        frame.desk_overflow = Some(shown.overflow);
        view::draw_overflow_panel(canvas, &frame, scroll);
    }

    let Some((panel, scroll)) = shown.overflow.panel else {
        return;
    };
    let tile = shown.overflow.tile;
    let Some(index) = browser
        .rename
        .as_ref()
        .map(|session| session.index)
        .filter(|index| tile.contains(*index) && !shown.closing)
    else {
        return;
    };
    let cell = panel.cell_rect(index - tile.first, scroll);
    let rect = view::grid_rename_rect_over(cell);
    let Some(session) = browser.rename.as_mut() else {
        return;
    };
    canvas.save();
    canvas.clip_rect(panel.rect, None, Some(true));
    session.input.set_size(rect.width(), rect.height());
    canvas.translate((rect.left, rect.top));
    session.input.render_at(canvas, rect.width(), rect.height());
    canvas.restore();
}
