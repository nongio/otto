use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::{ContextMenuRenderer, ContextMenuState, ContextMenuStyle};
use crate::app_runner::AppContext;
use crate::components::menu_item::MenuItem;
use crate::input::keycodes;
use crate::protocols::otto_surface_style_v1::{BlendMode, ClipMode};
use crate::surfaces::PopupSurface;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use wayland_client::{backend::ObjectId, protocol::wl_keyboard, Proxy};
use wayland_protocols::xdg::shell::client::xdg_surface;

/// How much of the menu a wheel notch or a finger's travel moves.
///
/// The axis event arrives in the pointer's own units, which land a notch at
/// well under one row here — the list barely creeps. Multiplied up, a notch
/// clears a couple of rows and a pan moves the distance the fingers did.
const SCROLL_RATE: f32 = 3.0;

/// How far the list at `depth` is scrolled.
///
/// Only the root menu scrolls: a submenu is short by construction, and giving
/// each depth its own offset would buy nothing yet. See
/// [`ContextMenuState::scroll`].
fn scroll_at_depth(state: &ContextMenuState, depth: usize) -> f32 {
    if depth == 0 {
        state.scroll()
    } else {
        0.0
    }
}

type PopupStack = Rc<RefCell<Vec<Rc<RefCell<Option<PopupSurface>>>>>>;
type ItemClickCallback = Rc<RefCell<Option<Rc<dyn Fn(&str)>>>>;
type CloseCallback = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// High-level ContextMenuNext component
///
/// Can be used in two modes:
/// 1. As a rendered component (no surface) - call `render_to(canvas)`
/// 2. As a surface-owning component - call `show()` with parent/positioner
#[derive(Clone)]
pub struct ContextMenu {
    state: Rc<RefCell<ContextMenuState>>,
    style: Rc<RefCell<ContextMenuStyle>>,

    // Popup surfaces - one per depth level (0=root, 1=first submenu, etc.)
    popups: PopupStack,

    // Parent XDG surface for all popups (window surface)
    parent_xdg: Rc<RefCell<Option<xdg_surface::XdgSurface>>>,

    // Registry of surfaces: surface_id -> depth level
    registered_surfaces: Rc<RefCell<HashMap<ObjectId, usize>>>,

    // Callbacks - wrapped in Rc<RefCell<>> so they can be set after construction
    on_item_click: ItemClickCallback,
    on_close: CloseCallback,

    /// A fade-out is in flight. Guards against stacking a second close
    /// transaction (and a second `on_close`) on top of one already running.
    closing: Rc<Cell<bool>>,

    /// A press landed on one of our client's surfaces that is not part of this
    /// menu. Acted on at the *next* pointer batch — see
    /// [`ContextMenu::register_pointer_handler`].
    dismiss_pending: Rc<Cell<bool>>,

    /// What has been typed at the menu, and when the last character arrived.
    /// See [`ContextMenu::handle_text`].
    typeahead: Rc<RefCell<(String, Instant)>>,
}

/// Whether `label` starts with `query`, ignoring case.
///
/// Compared a character at a time rather than by lowercasing both: a font list
/// is long, this runs on every keystroke, and the common answer is "no" at the
/// first character.
fn starts_with_ignoring_case(label: &str, query: &str) -> bool {
    let mut label = label.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| label.next() == Some(wanted))
}

/// How long a typed prefix stands before the next character starts a new one.
///
/// Long enough to type a word at an unhurried pace, short enough that coming
/// back to a menu left open does not have you searching for `nototon` — the
/// same span the platforms that have had type-to-select for decades settled
/// on.
const TYPEAHEAD_TIMEOUT: Duration = Duration::from_millis(1000);

impl ContextMenu {
    // === Construction ===

    /// Create a new context menu without a surface
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self::new_internal(items, true)
    }

    /// Internal constructor with option to skip pointer handler registration
    fn new_internal(items: Vec<MenuItem>, register_handler: bool) -> Self {
        let state = ContextMenuState::new(items);
        let style = ContextMenuStyle::default();

        let mut s = Self {
            state: Rc::new(RefCell::new(state)),
            style: Rc::new(RefCell::new(style)),
            popups: Rc::new(RefCell::new(vec![])), // Start with one popup for root (depth 0)
            parent_xdg: Rc::new(RefCell::new(None)),
            on_item_click: Rc::new(RefCell::new(None)),
            on_close: Rc::new(RefCell::new(None)),
            registered_surfaces: Rc::new(RefCell::new(HashMap::new())),
            closing: Rc::new(Cell::new(false)),
            dismiss_pending: Rc::new(Cell::new(false)),
            typeahead: Rc::new(RefCell::new((String::new(), Instant::now()))),
        };

        // Register pointer handler only for root menu
        if register_handler {
            s.register_pointer_handler();
            s.register_keyboard_leave_handler();
        }
        s
    }

    /// Create with shared state (for submenu coordination)
    pub fn with_state(state: Rc<RefCell<ContextMenuState>>) -> Self {
        let style = ContextMenuStyle::default();

        Self {
            state,
            style: Rc::new(RefCell::new(style)),
            popups: Rc::new(RefCell::new(vec![Rc::new(RefCell::new(None))])),
            parent_xdg: Rc::new(RefCell::new(None)),
            on_item_click: Rc::new(RefCell::new(None)),
            on_close: Rc::new(RefCell::new(None)),
            registered_surfaces: Rc::new(RefCell::new(HashMap::new())),
            closing: Rc::new(Cell::new(false)),
            dismiss_pending: Rc::new(Cell::new(false)),
            typeahead: Rc::new(RefCell::new((String::new(), Instant::now()))),
        }
    }

    // === Builder API ===

    pub fn with_style(self, style: ContextMenuStyle) -> Self {
        *self.style.borrow_mut() = style;
        self
    }

    pub fn on_item_click<F>(self, callback: F) -> Self
    where
        F: Fn(&str) + 'static,
    {
        *self.on_item_click.borrow_mut() = Some(Rc::new(callback));
        self
    }

    /// Fires whenever the menu closes without an item having been chosen —
    /// ESC, clicking outside, or losing keyboard focus. Additive: existing
    /// callers that never set this see no behaviour change, since the
    /// callback defaults to `None`.
    ///
    /// A caller that anchors this menu to its own field (a pop-up button,
    /// say) can use this to know when to stop drawing that field as "open"
    /// without polling — `is_visible()` alone can't tell "closed by the
    /// user" apart from "never opened".
    pub fn on_close<F>(self, callback: F) -> Self
    where
        F: Fn() + 'static,
    {
        *self.on_close.borrow_mut() = Some(Rc::new(callback));
        self
    }

    // === Surface Management ===

    /// Show the menu with an explicit grab serial (recommended for GNOME)
    pub fn show(
        &self,
        parent: &wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
        serial: u32,
    ) {
        // A stale `closing` (e.g. a fade-out whose completion event never
        // arrived) must not wedge the menu permanently open.
        self.closing.set(false);
        self.dismiss_pending.set(false);
        self.typeahead.borrow_mut().0.clear();
        self.show_menu_at_depth(0, parent, positioner, Some(serial));
    }

    /// Show the menu attached to a layer shell surface
    ///
    /// This creates popups with the layer surface as parent using the
    /// wlr-layer-shell `get_popup` request.
    ///
    /// # Arguments
    /// * `layer_surface` - The parent layer shell surface
    /// * `positioner` - XDG positioner defining popup position and size
    pub fn show_for_layer(
        &self,
        layer_surface: &wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
    ) {
        self.closing.set(false);
        self.dismiss_pending.set(false);
        self.show_menu_at_depth_for_layer(0, layer_surface, positioner);
    }

    /// Internal: Show popup at a specific depth level for layer shell parent
    fn show_menu_at_depth_for_layer(
        &self,
        depth: usize,
        layer_surface: &wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
    ) {
        // Check if popup at this depth already exists and is Some
        if self.popups.borrow().len() > depth && self.popups.borrow()[depth].borrow().is_some() {
            return;
        }

        // Get items for this depth and calculate dimensions
        let (width, height) = {
            let state = self.state.borrow();
            let items = state.items_at_depth(depth);
            ContextMenuRenderer::measure_items(items, &self.style.borrow())
        };
        // Create popup surface for layer shell parent
        if let Ok(popup) =
            PopupSurface::new_for_layer(layer_surface, positioner, width as i32, height as i32)
        {
            let surface_id = popup.wl_surface().id();
            popup.wl_surface().commit();
            // Apply visual effects immediately
            ContextMenu::apply_surface_effects(&self.style.borrow(), &popup);
            if let Some(surface_style) = popup.base_surface().surface_style() {
                surface_style.set_opacity(1.0);
            }

            // Register surface with depth
            self.registered_surfaces
                .borrow_mut()
                .insert(surface_id.clone(), depth);

            // Store popup at correct depth
            let popup_ref = Rc::new(RefCell::new(Some(popup)));
            {
                let mut popups_mut = self.popups.borrow_mut();
                // Ensure vector is long enough
                while popups_mut.len() <= depth {
                    popups_mut.push(Rc::new(RefCell::new(None)));
                }
                // Set at specific depth index
                popups_mut[depth] = popup_ref.clone();
            }

            let state = self.state.clone();
            let style = self.style.borrow().clone();

            // Register done callback to close menu when clicked outside
            let menu_self = Rc::new(self.clone());
            AppContext::register_popup_done_callback(surface_id.clone(), move || {
                menu_self.hide_animated();
            });

            AppContext::register_popup_configure_callback(surface_id, move |_serial| {
                // NOTE: SCTK's Popup already calls ack_configure internally
                if let Some(popup) = popup_ref.borrow_mut().as_mut() {
                    popup.mark_configured();
                }
                // Render immediately
                Self::render_menu_at_depth(&state, &style, &popup_ref, depth);
            });
        }
    }

    /// Internal: Show popup at a specific depth level
    fn show_menu_at_depth(
        &self,
        depth: usize,
        parent: &wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
        grab_serial: Option<u32>,
    ) {
        // if already open at this depth, ignore

        // Check if popup at this depth already exists and is Some
        if self.popups.borrow().len() > depth && self.popups.borrow()[depth].borrow().is_some() {
            return;
        }

        // Get items for this depth and calculate dimensions
        let (width, height) = {
            let state = self.state.borrow();
            let items = state.items_at_depth(depth);
            ContextMenuRenderer::measure_items(items, &self.style.borrow())
        };

        // Create popup surface with the provided grab serial
        if let Ok(popup) = PopupSurface::new_with_grab(
            parent,
            positioner,
            width as i32,
            height as i32,
            grab_serial,
        ) {
            let surface_id = popup.wl_surface().id();

            // TODO: Set initial opacity to 0.0 for fade-in animation
            // popup.set_opacity(0.0); // Requires scene surface support
            if let Some(scene_surface) = popup.base_surface().surface_style() {
                scene_surface.set_opacity(0.0); // Start fully transparent
            }
            // Store parent XDG surface for all future submenus
            *self.parent_xdg.borrow_mut() = Some(parent.clone());

            // Register surface with depth
            self.registered_surfaces
                .borrow_mut()
                .insert(surface_id.clone(), depth);

            // Store popup at correct depth
            let popup_ref = Rc::new(RefCell::new(Some(popup)));
            {
                let mut popups_mut = self.popups.borrow_mut();
                // Ensure vector is long enough
                while popups_mut.len() <= depth {
                    popups_mut.push(Rc::new(RefCell::new(None)));
                }
                // Set at specific depth index
                popups_mut[depth] = popup_ref.clone();
            }

            // Set up configure callback

            let state = self.state.clone();
            let style = self.style.borrow().clone();

            // Register done callback to close menu when clicked outside
            let menu_self = Rc::new(self.clone());
            AppContext::register_popup_done_callback(surface_id.clone(), move || {
                menu_self.hide_animated();
            });

            AppContext::register_popup_configure_callback(surface_id, move |_serial| {
                // NOTE: SCTK's Popup already calls ack_configure internally, so we must NOT call it again!

                if let Some(popup) = popup_ref.borrow_mut().as_mut() {
                    ContextMenu::apply_surface_effects(&style, popup);

                    popup.mark_configured();

                    // TODO: Fade-in animation after configure
                    // popup.set_opacity(1.0); // Animate from 0.0 to 1.0
                    if let Some(scene_surface) = popup.base_surface().surface_style() {
                        if let Some(scene) = AppContext::surface_style_manager() {
                            let qh = AppContext::queue_handle();

                            let timing = scene.create_timing_function(qh, ());
                            timing.set_spring(0.1, 0.1);
                            let animation = scene.begin_transaction(qh, ());
                            animation.set_duration(0.1);
                            animation.set_delay(0.2);
                            animation.set_timing_function(&timing);

                            scene_surface.set_opacity(1.0);

                            animation.commit();
                        }
                    }
                    // scene_surface.set_opacity(1.0); // Fade in to fully opaque
                    // Could use scene surface API when available
                }

                // Render immediately - this will attach buffer and commit
                Self::render_menu_at_depth(&state, &style, &popup_ref, depth);
            });
        }
    }

    /// Internal: Show layer shell popup at depth (usually just root)
    #[allow(dead_code)]
    fn show_at_depth_layer(
        &self,
        depth: usize,
        parent: &wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
    ) {
        while self.popups.borrow().len() <= depth {
            self.popups.borrow_mut().push(Rc::new(RefCell::new(None)));
        }
        let style = self.style.borrow().clone();
        *self.popups.borrow()[depth].borrow_mut() = None;

        let (width, height) = {
            let state = self.state.borrow();
            let items = state.items_at_depth(depth);
            ContextMenuRenderer::measure_items(items, &style)
        };

        if let Ok(popup) =
            PopupSurface::new_for_layer(parent, positioner, width as i32, height as i32)
        {
            ContextMenu::apply_surface_effects(&style, &popup);
            let surface_id = popup.wl_surface().id();

            self.registered_surfaces
                .borrow_mut()
                .insert(surface_id.clone(), depth);
            *self.popups.borrow()[depth].borrow_mut() = Some(popup);

            let popup_ref = self.popups.borrow()[depth].clone();
            let state = self.state.clone();

            AppContext::register_popup_configure_callback(surface_id, move |_serial| {
                if let Some(popup) = popup_ref.borrow_mut().as_mut() {
                    ContextMenu::apply_surface_effects(&style, popup);
                    popup.mark_configured();

                    Self::render_menu_at_depth(&state, &style, &popup_ref, depth);
                }
            });
        }
    }

    /// Hide the menu immediately (closes all popups)
    pub fn hide(&self) {
        tracing::debug!("context_menu: hide()");
        self.closing.set(false);
        self.dismiss_pending.set(false);
        let mut reg = self.registered_surfaces.borrow_mut();
        for popup in self.popups.borrow().iter() {
            if let Some(p) = popup.borrow().as_ref() {
                reg.remove(&p.wl_surface().id());
            }
            *popup.borrow_mut() = None;
        }
        drop(reg);
        self.state.borrow_mut().reset();
        if let Some(callback) = self.on_close.borrow().as_ref() {
            callback();
        }
    }

    /// Hide the menu with fade-out animation
    pub fn hide_animated(&self) {
        if self.closing.get() {
            // A fade-out is already running; stacking another transaction would
            // fire `on_close` twice and re-null already-nulled popups.
            return;
        }
        tracing::debug!("context_menu: hide_animated()");
        let close_delay = self.style.borrow().close_delay as f64;

        if let Some(scene) = AppContext::surface_style_manager() {
            self.closing.set(true);
            self.dismiss_pending.set(false);
            let qh = AppContext::queue_handle();

            let animation = scene.begin_transaction(qh, ());
            animation.set_duration(close_delay);
            animation.enable_completion_event();

            // Fade out all popups
            for popup in self.popups.borrow().iter() {
                if let Some(popup_ref) = popup.borrow().as_ref() {
                    if let Some(scene_surface) = popup_ref.base_surface().surface_style() {
                        scene_surface.set_opacity(0.0);
                    }
                }
            }

            // Register callback to destroy popups after animation completes
            let popups = self.popups.clone();
            let state = self.state.clone();
            let on_close = self.on_close.clone();
            let closing = self.closing.clone();
            let registered_surfaces = self.registered_surfaces.clone();
            let transaction_id = animation.id();
            AppContext::register_transaction_completion_callback(
                transaction_id,
                Box::new(move || {
                    for popup in popups.borrow().iter() {
                        if let Some(p) = popup.borrow().as_ref() {
                            registered_surfaces
                                .borrow_mut()
                                .remove(&p.wl_surface().id());
                        }
                        *popup.borrow_mut() = None;
                    }
                    state.borrow_mut().reset();
                    closing.set(false);
                    if let Some(callback) = on_close.borrow().as_ref() {
                        callback();
                    }
                }),
            );

            animation.commit();
        } else {
            // No style protocol available, close immediately
            self.hide();
        }
    }

    // === Submenu Management ===

    /// Show submenu for item at given depth (static helper for callbacks)
    #[allow(clippy::too_many_arguments)]
    fn show_submenu_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &PopupStack,
        style_rc: &Rc<RefCell<ContextMenuStyle>>,
        registered_surfaces: &Rc<RefCell<HashMap<ObjectId, usize>>>,
        parent_xdg: &Rc<RefCell<Option<xdg_surface::XdgSurface>>>,
        depth: usize,
        item_idx: usize,
        delay: f64,
    ) {
        // CRITICAL: Close any existing popup at depth+1 before creating new one
        // This ensures we don't violate XDG protocol (new popup must be on topmost popup)
        {
            let popups_borrowed = popups.borrow();
            if popups_borrowed.len() > depth + 1 && popups_borrowed[depth + 1].borrow().is_some() {
                drop(popups_borrowed);
                Self::hide_submenus_from_static(state, popups, depth + 1);
                state.borrow_mut().close_submenus_from(depth + 1);
            }
        }

        let style = style_rc.borrow().clone(); // Clone style for use in this function and callbacks
                                               // Check if item at this depth has submenu
        let items_at_depth = state.borrow().items_at_depth(depth).to_vec();
        if !items_at_depth
            .get(item_idx)
            .map(|item| item.has_submenu())
            .unwrap_or(false)
        {
            return;
        }

        // Check if already open (state should already be updated by caller for keyboard)
        if !state.borrow().is_submenu_open(depth, item_idx) {
            // For hover/pointer events, update state here
            state.borrow_mut().open_submenu(depth, item_idx);
        }

        // XDG popups MUST be chained to the topmost popup:
        // - depth parameter is the parent's depth
        // - we're creating a child at depth + 1
        // - parent = popups[depth] (the popup we're opening a submenu FROM)
        let parent_surface = {
            let popups_borrowed = popups.borrow();
            if let Some(parent_popup_rc) = popups_borrowed.get(depth) {
                parent_popup_rc
                    .borrow()
                    .as_ref()
                    .and_then(|surf| surf.xdg_surface())
                    .cloned()
            } else {
                // Fallback to window surface (shouldn't happen after root menu is created)
                parent_xdg.borrow().clone()
            }
        };

        if let Some(parent_xdg) = parent_surface {
            // Get submenu items and measure
            let (width, height) = {
                let state_borrow = state.borrow();
                let items_at_depth = state_borrow.items_at_depth(depth);
                if let Some(parent_item) = items_at_depth.get(item_idx) {
                    if let Some(submenu_items) = parent_item.submenu_items() {
                        ContextMenuRenderer::measure_items(submenu_items, &style)
                    } else {
                        return;
                    }
                } else {
                    return;
                }
            };

            // Create positioner
            use smithay_client_toolkit::shell::xdg::XdgPositioner;
            use wayland_protocols::xdg::shell::client::xdg_positioner;

            if let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) {
                // Get parent menu width and calculate Y position
                let (parent_width, anchor_y, _item_height) = {
                    let state_borrow = state.borrow();
                    let items_at_depth = state_borrow.items_at_depth(depth);
                    let style_borrow = style_rc.borrow();

                    // Calculate parent width
                    let (p_width, _) =
                        ContextMenuRenderer::measure_items(items_at_depth, &style_borrow);

                    // Calculate Y position by summing heights before selected item
                    let y_offset: f32 = items_at_depth
                        .iter()
                        .take(item_idx)
                        .map(|item| item.height)
                        .sum();

                    // Get the selected item's height
                    let item_h = items_at_depth
                        .get(item_idx)
                        .map(|item| item.height)
                        .unwrap_or(22.0);

                    // Y position includes top padding
                    (p_width, y_offset + style_borrow.vertical_padding, item_h)
                };

                // Set submenu size
                positioner.set_size(width as i32, height as i32);

                // Define anchor rectangle as a 1px vertical line at parent's right edge
                // positioned at the selected item
                positioner.set_anchor_rect(
                    parent_width as i32 - 5, // x: at right edge of parent
                    anchor_y as i32,         // y: top of selected item
                    1,                       // width: thin vertical line
                    1_i32,                   // height: selected item height
                );

                // Anchor to top-left of this line (which is at parent's right edge)
                positioner.set_anchor(xdg_positioner::Anchor::TopLeft);

                // Place submenu to the right of the anchor point
                positioner.set_gravity(xdg_positioner::Gravity::BottomRight);

                // Ensure popups vec is large enough for submenu
                while popups.borrow().len() <= depth + 1 {
                    popups.borrow_mut().push(Rc::new(RefCell::new(None)));
                }

                // Create submenu surface without grab — the root popup already
                // holds the keyboard grab, so submenus must not steal focus.
                if let Ok(popup) = PopupSurface::new_with_grab(
                    &parent_xdg,
                    &positioner,
                    width as i32,
                    height as i32,
                    None,
                ) {
                    let surface_id = popup.wl_surface().id();
                    ContextMenu::apply_surface_effects(&style, &popup);

                    if let Some(scene_surface) = popup.base_surface().surface_style() {
                        scene_surface.set_opacity(0.0); // Start fully transparent
                    }

                    // Register - borrow_mut for insertion
                    {
                        let mut reg_mut = registered_surfaces.borrow_mut();
                        reg_mut.insert(surface_id.clone(), depth + 1);
                    }

                    // Store
                    *popups.borrow()[depth + 1].borrow_mut() = Some(popup);

                    // Configure callback - need to clone style Rc for closure
                    let popup_ref = popups.borrow()[depth + 1].clone();
                    let state_clone = state.clone();
                    let style_clone = style_rc.clone(); // Clone the Rc
                    let submenu_depth = depth + 1;

                    AppContext::register_popup_configure_callback(surface_id, move |_serial| {
                        if let Some(popup) = popup_ref.borrow_mut().as_mut() {
                            popup.mark_configured();
                        }
                        let style_borrowed = style_clone.borrow();
                        Self::render_menu_at_depth(
                            &state_clone,
                            &style_borrowed,
                            &popup_ref,
                            submenu_depth,
                        );

                        if let Some(scene) = AppContext::surface_style_manager() {
                            if let Some(scene_surface) = popup_ref
                                .borrow()
                                .as_ref()
                                .and_then(|p| p.base_surface().surface_style())
                            {
                                let qh = AppContext::queue_handle();
                                let timing = scene.create_timing_function(qh, ());
                                timing.set_spring(0.1, 0.1);
                                let animation = scene.begin_transaction(qh, ());
                                animation.set_duration(0.1);
                                animation.set_delay(delay);
                                animation.set_timing_function(&timing);

                                scene_surface.set_opacity(1.0);

                                animation.commit();
                            }
                        }
                    });
                }
            }
        }
    }

    /// Hide submenus from depth onwards (static helper)
    fn hide_submenus_from_static(
        _state: &Rc<RefCell<ContextMenuState>>,
        popups: &PopupStack,
        from_depth: usize,
    ) {
        let popups_borrowed = popups.borrow();
        tracing::debug!(
            "hide_submenus_from depth={from_depth}, total_popups={}",
            popups_borrowed.len()
        );
        for i in from_depth..popups_borrowed.len() {
            let had_popup = popups_borrowed[i].borrow().is_some();
            *popups_borrowed[i].borrow_mut() = None;
            tracing::debug!("  popup[{i}]: had={had_popup} -> None");
        }
    }

    // === Event Handling ===

    /// Close the menu when the surface holding keyboard focus loses it.
    ///
    /// With an active popup grab the focused surface is the popup itself, so a
    /// `leave` on one of our own surfaces means focus went somewhere we do not
    /// own — another window, or the compositor releasing the grab (expose,
    /// gestures). A `leave` on the parent toplevel is *not* interesting: that is
    /// exactly what happens when the grab hands focus to the popup as it opens.
    fn register_keyboard_leave_handler(&mut self) {
        let menu = self.clone();
        AppContext::register_keyboard_leave_callback(move |surface_id| {
            if !menu.registered_surfaces.borrow().contains_key(surface_id) {
                return;
            }
            if menu.is_visible() {
                tracing::debug!("context_menu: keyboard leave on own surface → close");
                menu.hide_animated();
            }
        });
    }

    /// Register pointer event handler (called only by root menu)
    fn register_pointer_handler(&mut self) {
        let registered_surfaces = self.registered_surfaces.clone();
        let state = self.state.clone();
        let style = self.style.clone(); // Clone the Rc, not the value
        let popups = self.popups.clone();
        let on_item_click = self.on_item_click.clone();
        let parent_xdg = self.parent_xdg.clone();
        let menu = self.clone();
        let dismiss_pending = self.dismiss_pending.clone();

        AppContext::register_pointer_callback(move |events| {
            for event in events {
                let surface_id = event.surface.id();
                // Look up depth for this surface
                let depth = registered_surfaces.borrow().get(&surface_id).cloned();
                let Some(depth) = depth else {
                    // Not one of ours. A press here while the menu is up is an
                    // outside click; arm the dismissal for the end of this
                    // batch.
                    if matches!(event.kind, PointerEventKind::Press { .. }) && menu.is_visible() {
                        dismiss_pending.set(true);
                    }
                    continue;
                };
                {
                    let (x, y) = event.position;

                    match event.kind {
                        PointerEventKind::Motion { .. } => {
                            Self::handle_motion_static(
                                &state,
                                &popups,
                                &style,
                                &registered_surfaces,
                                &parent_xdg,
                                depth,
                                x,
                                y,
                            );
                        }
                        PointerEventKind::Press { button: 0x110, .. } => {
                            Self::handle_click_static(
                                &state,
                                &popups,
                                &style,
                                &on_item_click,
                                depth,
                                x as f32,
                                y as f32,
                            );
                        }
                        // A wheel over a capped menu scrolls its list. The
                        // popup is a fixed-size surface, so this is the only
                        // way to reach the rows past its bottom edge.
                        PointerEventKind::Axis { vertical, .. } => {
                            Self::handle_scroll_static(
                                &state,
                                &popups,
                                &style,
                                depth,
                                vertical.absolute as f32 * SCROLL_RATE,
                                x as f32,
                                y as f32,
                            );
                        }
                        _ => {}
                    }
                }
            }
        });

        // Act on an armed dismissal once the whole batch is spoken for. The
        // compositor's popup grab is "owner-events": a press on another
        // surface of *our own* client is delivered to us rather than
        // dismissing the popup, so closing on an outside click is our job,
        // not the compositor's.
        //
        // Running at the end of the batch rather than inside the callback
        // above matters twice over. Our callback runs before the app's own
        // handling, and the press may well be on the control that owns this
        // menu (a dropdown field toggling itself shut) — letting the app have
        // the batch first means we never close a menu it is about to reopen.
        // And unlike deferring to the *next* batch, this needs no second
        // event to arrive: a press whose release goes somewhere else still
        // dismisses.
        let menu = self.clone();
        let dismiss_pending = self.dismiss_pending.clone();
        AppContext::register_pointer_batch_end_callback(move || {
            if dismiss_pending.replace(false) && menu.is_visible() && !menu.closing.get() {
                tracing::debug!("context_menu: press outside menu surfaces → close");
                menu.hide_animated();
            }
        });
    }

    /// Handle pointer motion at specific depth
    #[allow(clippy::too_many_arguments)]
    fn handle_motion_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &PopupStack,
        style: &Rc<RefCell<ContextMenuStyle>>,
        registered_surfaces: &Rc<RefCell<HashMap<ObjectId, usize>>>,
        parent_xdg: &Rc<RefCell<Option<xdg_surface::XdgSurface>>>,
        depth: usize,
        x: f64,
        y: f64,
    ) {
        // Get items for this depth
        let (items, scroll) = {
            let state_borrow = state.borrow();
            (
                state_borrow.items_at_depth(depth).to_vec(),
                scroll_at_depth(&state_borrow, depth),
            )
        };

        // Hit test
        let style_borrowed = style.borrow();
        let item_index = ContextMenuRenderer::hit_test_items(
            &items,
            &style_borrowed,
            x as f32,
            y as f32,
            scroll,
        );
        drop(style_borrowed);

        // Update selection at this depth
        let mut state_mut = state.borrow_mut();
        let old_selection = state_mut.selected_at_depth(depth);
        state_mut.select_at_depth(depth, item_index);

        // Only one item should be selected across the entire menu tree.
        // Clear selections at all other depths when the pointer is here.
        let mut depths_to_redraw = Vec::new();
        for d in 0..depth {
            if state_mut.selected_at_depth(d).is_some() {
                state_mut.select_at_depth(d, None);
                depths_to_redraw.push(d);
            }
        }
        // Also clear deeper depths (e.g. pointer moved back from submenu to parent)
        let max_depth = state_mut.depth();
        for d in (depth + 1)..=max_depth {
            if state_mut.selected_at_depth(d).is_some() {
                state_mut.select_at_depth(d, None);
                depths_to_redraw.push(d);
            }
        }

        if old_selection != item_index || !depths_to_redraw.is_empty() {
            drop(state_mut);

            // Redraw menus whose selection was cleared
            for d in depths_to_redraw {
                if d < popups.borrow().len() {
                    let popup_ref = popups.borrow()[d].clone();
                    let style_borrowed = style.borrow();
                    Self::render_menu_at_depth(state, &style_borrowed, &popup_ref, d);
                }
            }

            // Redraw at this depth - clone the popup Rc to avoid holding borrow
            if depth < popups.borrow().len() {
                let popup_ref = popups.borrow()[depth].clone();
                let style_borrowed = style.borrow();
                Self::render_menu_at_depth(state, &style_borrowed, &popup_ref, depth);
            }

            // Handle submenu show/hide
            if let Some(new_idx) = item_index {
                // Check if item at this depth has submenu
                let has_submenu = {
                    let state_borrow = state.borrow();
                    let items_at_depth = state_borrow.items_at_depth(depth);
                    items_at_depth
                        .get(new_idx)
                        .map(|item| item.has_submenu())
                        .unwrap_or(false)
                };
                let already_open = state.borrow().is_submenu_open(depth, new_idx);

                if has_submenu && !already_open {
                    let show_delay = style.borrow().show_delay_mouse;
                    Self::show_submenu_static(
                        state,
                        popups,
                        style,
                        registered_surfaces,
                        parent_xdg,
                        depth,
                        new_idx,
                        show_delay as f64,
                    );
                } else if !has_submenu {
                    // Close any open submenus and update state
                    state.borrow_mut().close_submenus_from(depth);
                    Self::hide_submenus_from_static(state, popups, depth + 1);
                }
            } else {
                // Mouse left the menu area — only close submenus if there isn't
                // an open submenu at this depth (the pointer may have moved into
                // the child submenu surface, which fires its own motion events).
                let has_open_submenu = state.borrow().has_open_submenu_at(depth);
                if !has_open_submenu {
                    state.borrow_mut().close_submenus_from(depth);
                    Self::hide_submenus_from_static(state, popups, depth + 1);
                }
            }
        }
    }

    /// Handle click with animation at specific depth
    fn handle_click_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &PopupStack,
        style: &Rc<RefCell<ContextMenuStyle>>,
        on_item_click: &ItemClickCallback,
        depth: usize,
        x: f32,
        y: f32,
    ) {
        // Get items for this depth
        let (items, scroll) = {
            let state_borrow = state.borrow();
            (
                state_borrow.items_at_depth(depth).to_vec(),
                scroll_at_depth(&state_borrow, depth),
            )
        };

        // Hit test
        let style_borrowed = style.borrow();
        let item_index = ContextMenuRenderer::hit_test_items(&items, &style_borrowed, x, y, scroll);
        drop(style_borrowed);

        if let Some(idx) = item_index {
            let action_id = items
                .get(idx)
                .and_then(|item| item.action_id().map(|s| s.to_string()));
            if action_id.is_some() || items.get(idx).and_then(|item| item.label()).is_some() {
                let callback_id = action_id
                    .or_else(|| {
                        items
                            .get(idx)
                            .and_then(|item| item.label().map(|s| s.to_string()))
                    })
                    .unwrap_or_default();

                Self::flash_and_activate(
                    state,
                    popups,
                    style,
                    on_item_click,
                    depth,
                    idx,
                    &callback_id,
                );
            }
        }
    }

    /// Flash the highlight on the selected item, fire the callback, then close.
    ///
    /// Deselects → redraws → brief pause → reselects → redraws → pause → fires
    /// callback → closes all popups.  Used by both mouse-click and keyboard-enter paths.
    #[allow(clippy::too_many_arguments)]
    fn flash_and_activate(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &PopupStack,
        style: &Rc<RefCell<ContextMenuStyle>>,
        on_item_click: &ItemClickCallback,
        depth: usize,
        idx: usize,
        callback_id: &str,
    ) {
        let popup_ref = if depth < popups.borrow().len() {
            Some(popups.borrow()[depth].clone())
        } else {
            None
        };

        let style_borrowed = style.borrow();

        // Flash: deselect → redraw → pause → reselect → redraw → pause
        state.borrow_mut().select_at_depth(depth, None);
        if let Some(ref popup_ref) = popup_ref {
            Self::render_menu_at_depth(state, &style_borrowed, popup_ref, depth);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));

        state.borrow_mut().select_at_depth(depth, Some(idx));
        if let Some(ref popup_ref) = popup_ref {
            Self::render_menu_at_depth(state, &style_borrowed, popup_ref, depth);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));

        drop(style_borrowed);

        // Fire callback
        if let Some(callback) = on_item_click.borrow().as_ref() {
            callback(callback_id);
        }

        // Close all popups and reset state
        for popup in popups.borrow().iter() {
            *popup.borrow_mut() = None;
        }
        state.borrow_mut().reset();
    }

    /// Scroll the list at `depth` by `delta` and redraw it, then re-run the
    /// hover test: the pointer has not moved, but the row under it has.
    #[allow(clippy::too_many_arguments)]
    fn handle_scroll_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &PopupStack,
        style: &Rc<RefCell<ContextMenuStyle>>,
        depth: usize,
        delta: f32,
        x: f32,
        y: f32,
    ) {
        if depth != 0 || delta == 0.0 {
            return;
        }
        let style_borrowed = style.borrow();
        let (moved, overflow) = {
            let mut state_mut = state.borrow_mut();
            let overflow =
                ContextMenuRenderer::overflow(state_mut.items_at_depth(0), &style_borrowed);
            let before = state_mut.scroll();
            state_mut.set_scroll(before + delta, overflow);
            (state_mut.scroll() != before, overflow)
        };
        drop(style_borrowed);
        if overflow <= 0.0 || !moved {
            return;
        }

        // The row under the pointer changed with the list, so the highlight
        // has to be re-tested before the redraw rather than after it.
        Self::handle_motion_static_hover(state, style, depth, x, y);

        let style_borrowed = style.borrow();
        if let Some(popup) = popups.borrow().get(depth) {
            Self::render_menu_at_depth(state, &style_borrowed, popup, depth);
        }
    }

    /// The selection half of a motion: which row is under `(x, y)` now.
    fn handle_motion_static_hover(
        state: &Rc<RefCell<ContextMenuState>>,
        style: &Rc<RefCell<ContextMenuStyle>>,
        depth: usize,
        x: f32,
        y: f32,
    ) {
        let (items, scroll) = {
            let state_borrow = state.borrow();
            (
                state_borrow.items_at_depth(depth).to_vec(),
                scroll_at_depth(&state_borrow, depth),
            )
        };
        let style_borrowed = style.borrow();
        let index = ContextMenuRenderer::hit_test_items(&items, &style_borrowed, x, y, scroll);
        drop(style_borrowed);
        state.borrow_mut().select_at_depth(depth, index);
    }

    /// Render at a specific depth (static helper for callbacks)
    fn render_menu_at_depth(
        state: &Rc<RefCell<ContextMenuState>>,
        style: &ContextMenuStyle,
        popup: &Rc<RefCell<Option<PopupSurface>>>,
        depth: usize,
    ) {
        // Get items and dimensions before borrowing popup
        let (items_vec, selected, width, height, scroll) = {
            let state_borrow = state.borrow();
            let items = state_borrow.items_at_depth(depth);
            let selected = state_borrow.selected_at_depth(depth);
            let (w, h) = ContextMenuRenderer::measure_items(items, style);
            (
                items.to_vec(),
                selected,
                w,
                h,
                scroll_at_depth(&state_borrow, depth),
            )
        };

        // Now borrow popup and draw (no other borrows held)
        if let Some(popup_surface) = popup.borrow().as_ref() {
            popup_surface.draw(|canvas| {
                ContextMenuRenderer::render_depth(
                    canvas, &items_vec, selected, style, width, height, scroll,
                );
            });
        }
    }

    /// Keep the keyboard's selection inside the box a capped list is drawn
    /// in, scrolling it into view when the arrows have walked it past an edge.
    ///
    /// Only the root list scrolls — see [`scroll_at_depth`] — so a selection
    /// at any other depth is already visible by construction.
    fn reveal_selection(
        state: &Rc<RefCell<ContextMenuState>>,
        style: &ContextMenuStyle,
        depth: usize,
    ) {
        if depth != 0 {
            return;
        }
        let mut state_mut = state.borrow_mut();
        let Some(index) = state_mut.selected_at_depth(0) else {
            return;
        };
        let scroll = state_mut.scroll();
        let (target, overflow) = {
            let items = state_mut.items();
            (
                ContextMenuRenderer::scroll_to_reveal(items, style, index, scroll),
                ContextMenuRenderer::overflow(items, style),
            )
        };
        state_mut.set_scroll(target, overflow);
    }

    /// Put the highlight on one item of the root list, scroll it into view
    /// and repaint — what the arrows do, without walking there.
    ///
    /// For a host that picks the row itself rather than by direction: a
    /// pop-up button's type-ahead searches the values it was opened with,
    /// which the rows no longer carry once they have been elided to fit the
    /// button's column, so it has to say which row it landed on.
    pub fn select_and_reveal(&self, index: usize) {
        let style = self.style.borrow();
        self.state.borrow_mut().select_at_depth(0, Some(index));
        Self::reveal_selection(&self.state, &style, 0);
        let popup = self.popups.borrow().first().cloned();
        if let Some(popup) = popup {
            Self::render_menu_at_depth(&self.state, &style, &popup, 0);
        }
    }

    /// Handle keyboard input
    /// Bring the menu back in step with a highlight that has just moved:
    /// scroll it into view, and repaint the depth it is on along with any
    /// whose highlight it took over. Last input wins — the keyboard owning
    /// the selection means every other depth loses it.
    fn selection_moved(&self, style: &ContextMenuStyle) {
        let mut state_mut = self.state.borrow_mut();
        let current_depth = state_mut.depth();
        let cleared = state_mut.clear_selections_except(current_depth);
        drop(state_mut);
        Self::reveal_selection(&self.state, style, current_depth);
        for d in cleared {
            if d < self.popups.borrow().len() {
                let popup_ref = self.popups.borrow()[d].clone();
                Self::render_menu_at_depth(&self.state, style, &popup_ref, d);
            }
        }
        if current_depth < self.popups.borrow().len() {
            let popup_ref = self.popups.borrow()[current_depth].clone();
            Self::render_menu_at_depth(&self.state, style, &popup_ref, current_depth);
        }
    }

    /// Jump the highlight to the row whose label starts with what has been
    /// typed.
    ///
    /// A menu of every installed font is over a thousand rows: the arrows are
    /// not a way through it, and neither is the wheel. Characters typed
    /// within [`TYPEAHEAD_TIMEOUT`] of each other accumulate into one prefix,
    /// so `n`, `o`, `t` walks to the first `Not…` rather than to the first
    /// `N`, then to the first `O`, then to the first `T`.
    ///
    /// A character that matches nothing after the prefix starts a fresh
    /// search from itself rather than being swallowed — otherwise one typo
    /// makes the menu deaf for a second, which reads as the feature being
    /// broken.
    ///
    /// `text` is the key's own text, as the keyboard produced it; a key with
    /// none (an arrow, a modifier) never reaches here.
    pub fn handle_text(&mut self, text: &str) {
        let typed: String = text.chars().filter(|c| !c.is_control()).collect();
        if typed.is_empty() {
            return;
        }

        let query = {
            let mut typeahead = self.typeahead.borrow_mut();
            if typeahead.1.elapsed() > TYPEAHEAD_TIMEOUT {
                typeahead.0.clear();
            }
            typeahead.0.push_str(&typed);
            typeahead.1 = Instant::now();
            typeahead.0.clone()
        };

        let depth = self.state.borrow().depth();
        let found = self
            .match_prefix(depth, &query)
            .or_else(|| {
                // Nothing starts with the accumulated prefix. Treat what was
                // just typed as the start of a new one.
                (query.len() > typed.len()).then(|| {
                    *self.typeahead.borrow_mut() = (typed.clone(), Instant::now());
                    self.match_prefix(depth, &typed)
                })?
            })
            .or_else(|| {
                // A name remembered from the middle — "Sans" for "Noto Sans"
                // — is a fair thing to type at a list this long, and there is
                // nothing else for the keystroke to mean.
                self.match_substring(depth, &query)
            });

        let Some(index) = found else {
            return;
        };

        self.state.borrow_mut().select_at_depth(depth, Some(index));
        let style = self.style.borrow();
        self.selection_moved(&style);
    }

    /// The first row at `depth` whose label starts with `query`, ignoring case.
    fn match_prefix(&self, depth: usize, query: &str) -> Option<usize> {
        self.find_item(depth, |label| starts_with_ignoring_case(label, query))
    }

    /// The first row at `depth` whose label contains `query`, ignoring case.
    fn match_substring(&self, depth: usize, query: &str) -> Option<usize> {
        let query = query.to_lowercase();
        self.find_item(depth, |label| label.to_lowercase().contains(&query))
    }

    fn find_item(&self, depth: usize, matches: impl Fn(&str) -> bool) -> Option<usize> {
        let state = self.state.borrow();
        state
            .items_at_depth(depth)
            .iter()
            .position(|item| item.label().is_some_and(&matches))
    }

    pub fn handle_key(&mut self, key: u32, key_state: wl_keyboard::KeyState) {
        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }
        let style = self.style.borrow(); // Borrow once

        match key {
            keycodes::DOWN => {
                self.state.borrow_mut().select_next_at_depth(None);
                self.selection_moved(&style);
            }
            keycodes::UP => {
                self.state.borrow_mut().select_previous_at_depth(None);
                self.selection_moved(&style);
            }
            keycodes::HOME | keycodes::END => {
                let mut state_mut = self.state.borrow_mut();
                state_mut.select_edge_at_depth(None, key == keycodes::HOME);
                let current_depth = state_mut.depth();
                let cleared = state_mut.clear_selections_except(current_depth);
                drop(state_mut);
                Self::reveal_selection(&self.state, &style, current_depth);
                for d in cleared {
                    if d < self.popups.borrow().len() {
                        let popup_ref = self.popups.borrow()[d].clone();
                        Self::render_menu_at_depth(&self.state, &style, &popup_ref, d);
                    }
                }
                if current_depth < self.popups.borrow().len() {
                    let popup_ref = self.popups.borrow()[current_depth].clone();
                    Self::render_menu_at_depth(&self.state, &style, &popup_ref, current_depth);
                }
            }
            keycodes::ENTER | keycodes::SPACE => {
                let current_depth = self.state.borrow().depth();
                let state = self.state.borrow();
                let selected_idx = state.selected_at_depth(current_depth);
                // The same identity the mouse path fires with: an item's
                // `action_id` if it has one, its label only as a fallback.
                // A caller that keys on the id — a dropdown, whose ids are
                // the option indices and whose labels are elided to fit the
                // button's column — gets nothing it can match otherwise.
                let callback_id = selected_idx.and_then(|idx| {
                    let item = state.items_at_depth(current_depth).get(idx)?;
                    item.action_id()
                        .or_else(|| item.label())
                        .map(str::to_string)
                });

                if let (Some(idx), Some(label_owned)) = (selected_idx, callback_id) {
                    drop(state);

                    Self::flash_and_activate(
                        &self.state,
                        &self.popups,
                        &self.style,
                        &self.on_item_click,
                        current_depth,
                        idx,
                        &label_owned,
                    );
                }
            }
            keycodes::ESC => {
                tracing::debug!("context_menu: ESC pressed");
                self.state.borrow_mut().request_close();
                drop(style); // Drop before check_close
                self.check_close();
            }
            keycodes::RIGHT => {
                let current_depth = self.state.borrow().depth();
                // Open submenu if current item has one
                let state = self.state.borrow();
                let has_submenu = state.selected_has_submenu(None); // Use state's depth
                let selected_idx = state.selected_index(None); // Use state's depth
                drop(state);

                if has_submenu {
                    if let Some(idx) = selected_idx {
                        // 1. Update state: open submenu and move to first item of submenu
                        //    (open_submenu clears the parent selection)
                        self.state.borrow_mut().open_submenu(current_depth, idx);
                        self.state
                            .borrow_mut()
                            .select_at_depth(current_depth + 1, Some(0));

                        // 2. Re-render parent to clear its highlight
                        if current_depth < self.popups.borrow().len() {
                            let popup_ref = self.popups.borrow()[current_depth].clone();
                            Self::render_menu_at_depth(
                                &self.state,
                                &style,
                                &popup_ref,
                                current_depth,
                            );
                        }

                        // 3. Show the submenu surface
                        let show_delay = self.style.borrow().show_delay_keyboard;
                        Self::show_submenu_static(
                            &self.state,
                            &self.popups,
                            &self.style,
                            &self.registered_surfaces,
                            &self.parent_xdg,
                            current_depth,
                            idx,
                            show_delay as f64,
                        );
                    }
                }
            }
            keycodes::LEFT => {
                let current_depth = self.state.borrow().depth();
                tracing::debug!("context_menu: LEFT pressed, current_depth={current_depth}");
                // Close submenu and move back to parent
                if current_depth > 0 {
                    let target_depth = current_depth - 1;
                    tracing::debug!("context_menu: closing submenu at depth={current_depth}, target={target_depth}");

                    // Remember which parent item had the open submenu so we
                    // can restore selection on it after closing.
                    let parent_item_idx = self.state.borrow().open_submenu_at(target_depth);

                    // Hide submenu surfaces from current depth onwards
                    Self::hide_submenus_from_static(&self.state, &self.popups, current_depth);

                    // Update state: truncate to target_depth and set depth to target_depth
                    self.state.borrow_mut().close_submenus_from(target_depth);

                    // Restore selection on the parent item that had the submenu
                    if let Some(idx) = parent_item_idx {
                        self.state
                            .borrow_mut()
                            .select_at_depth(target_depth, Some(idx));
                    }

                    tracing::debug!("context_menu: after LEFT, is_visible={}", self.is_visible());

                    // Re-render parent menu
                    if target_depth < self.popups.borrow().len() {
                        let popup_ref = self.popups.borrow()[target_depth].clone();
                        Self::render_menu_at_depth(&self.state, &style, &popup_ref, target_depth);
                    }
                }
            }
            _ => {}
        }
    }

    // === Utilities ===

    fn apply_surface_effects(style: &ContextMenuStyle, popup: &PopupSurface) {
        if let Some(scene_surface) = popup.base_surface().surface_style() {
            // The renderer already paints `style.background_color()` into the
            // buffer. Setting it on the scene layer as well composites it
            // twice, so a translucent material lands far more opaque than the
            // same menu drawn compositor-side (the dock's).
            // Shadow geometry is in physical pixels, so it scales with the
            // output; values match the compositor-side menus (the dock's) so a
            // menu looks the same wherever it is drawn. The corner radius does
            // NOT get scaled here — it has to agree with the rounding the
            // renderer paints into the buffer, and scaling it makes the bar's
            // menus visibly rounder than the dock's.
            let scale = crate::app_runner::context::AppContext::fractional_scale();
            let shadow = style.theme.shadow;
            scene_surface.set_corner_radius(style.corner_radius as f64);
            scene_surface.set_masks_to_bounds(ClipMode::Enabled);
            scene_surface.set_shadow(
                shadow.a() as f64 / 255.0,
                16.0 * scale,
                0.0,
                4.0 * scale,
                shadow.r() as f64 / 255.0,
                shadow.g() as f64 / 255.0,
                shadow.b() as f64 / 255.0,
            );
            scene_surface.set_blend_mode(BlendMode::BackgroundBlur);
        }
    }

    // === State Access ===
    pub fn is_visible(&self) -> bool {
        self.popups
            .borrow()
            .iter()
            .any(|popup| popup.borrow().is_some())
    }

    /// Get the measured size (width, height) of the menu at a specific depth
    pub fn get_size_at_depth(&self, depth: usize) -> (f32, f32) {
        let state = self.state.borrow();
        let items = state.items_at_depth(depth);
        ContextMenuRenderer::measure_items(items, &self.style.borrow())
    }

    pub fn state(&self) -> &Rc<RefCell<ContextMenuState>> {
        &self.state
    }

    /// Check whether a wl_surface belongs to this menu (any depth).
    pub fn owns_surface(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        let id = surface.id();
        self.registered_surfaces.borrow().contains_key(&id)
    }

    /// Handle keyboard focus lost - closes the menu.
    ///
    /// Menus opened through [`ContextMenu::show`] / [`ContextMenu::show_for_layer`]
    /// already do this for themselves (see `register_keyboard_leave_handler`);
    /// this stays for hosts that route focus themselves.
    pub fn handle_keyboard_leave(&mut self) {
        self.hide_animated();
    }

    /// Check if menu should close and fire callback
    fn check_close(&mut self) {
        let should = self.state.borrow().should_close();
        tracing::debug!("context_menu: check_close should_close={should}");
        if should {
            // `hide()` itself fires `on_close` — a single point for it,
            // whichever path (ESC, outside click, keyboard leave) closed us.
            self.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_prefix_ignores_case_and_stops_at_the_first_difference() {
        assert!(starts_with_ignoring_case("Noto Sans CJK SC", "noto"));
        assert!(starts_with_ignoring_case("Noto Sans", "NOTO S"));
        assert!(starts_with_ignoring_case("Inter", ""));
        assert!(!starts_with_ignoring_case("Inter", "not"));
        // A query longer than the label is not a prefix of it.
        assert!(!starts_with_ignoring_case("Inter", "Internationale"));
    }

    /// Endonyms are what the language picker lists, so the matching has to
    /// hold up outside ASCII — where a character's lowercase form can be more
    /// than one character.
    #[test]
    fn a_prefix_matches_outside_ascii() {
        assert!(starts_with_ignoring_case("Русский", "рус"));
        assert!(starts_with_ignoring_case("Español", "ESPAÑ"));
        assert!(!starts_with_ignoring_case("Español", "espn"));
    }
}
