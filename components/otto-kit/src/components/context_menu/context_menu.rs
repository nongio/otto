use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{ContextMenuRenderer, ContextMenuState, ContextMenuStyle};
use crate::app_runner::AppContext;
use crate::components::menu_item::MenuItem;
use crate::input::keycodes;
use crate::protocols::otto_surface_style_v1::{BlendMode, ClipMode};
use crate::surfaces::PopupSurface;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use wayland_client::{backend::ObjectId, protocol::wl_keyboard, Proxy};
use wayland_protocols::xdg::shell::client::xdg_surface;

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
    popups: Rc<RefCell<Vec<Rc<RefCell<Option<PopupSurface>>>>>>,

    // Parent XDG surface for all popups (window surface)
    parent_xdg: Rc<RefCell<Option<xdg_surface::XdgSurface>>>,

    // Registry of surfaces: surface_id -> depth level
    registered_surfaces: Rc<RefCell<HashMap<ObjectId, usize>>>,

    // Callbacks - wrapped in Rc<RefCell<>> so they can be set after construction
    on_item_click: Rc<RefCell<Option<Rc<dyn Fn(&str)>>>>,
    on_close: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

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
        };

        // Register pointer handler only for root menu
        if register_handler {
            s.register_pointer_handler();
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

    // === Surface Management ===

    /// Show the menu with an explicit grab serial (recommended for GNOME)
    pub fn show(
        &self,
        parent: &wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
        serial: u32,
    ) {
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
    /// * `serial` - Serial from input event for popup grab
    pub fn show_for_layer(
        &self,
        layer_surface: &wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
        serial: u32,
    ) {
        self.show_menu_at_depth_for_layer(0, layer_surface, positioner, Some(serial));
    }

    /// Internal: Show popup at a specific depth level for layer shell parent
    fn show_menu_at_depth_for_layer(
        &self,
        depth: usize,
        layer_surface: &wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
        _grab_serial: Option<u32>,
    ) {
        println!("Showing layer shell context menu at depth {}...", depth);
        // Check if popup at this depth already exists and is Some
        if self.popups.borrow().len() > depth {
            if self.popups.borrow()[depth].borrow().is_some() {
                return;
            }
        }

        // Get items for this depth and calculate dimensions
        let (width, height) = {
            let state = self.state.borrow();
            let items = state.items_at_depth(depth);
            ContextMenuRenderer::measure_items(items, &self.style.borrow())
        };
        println!("Measured menu size: {}x{}", width, height);
        // Create popup surface for layer shell parent
        if let Ok(popup) =
            PopupSurface::new_for_layer(layer_surface, positioner, width as i32, height as i32)
        {
            let surface_id = popup.wl_surface().id();
            popup.wl_surface().commit();
            // Set initial opacity for fade-in animation
            if let Some(surface_style) = popup.base_surface().surface_style() {
                surface_style.set_opacity(0.0); // Start fully transparent
                surface_style.set_blend_mode(BlendMode::BackgroundBlur);
                surface_style.set_corner_radius(10.0);
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
                menu_self.hide();
            });

            AppContext::register_popup_configure_callback(surface_id, move |_serial| {
                // NOTE: SCTK's Popup already calls ack_configure internally
                if let Some(popup) = popup_ref.borrow_mut().as_mut() {
                    popup.mark_configured();

                    // Apply visual effects and fade-in animation
                    if let Some(scene_surface) = popup.base_surface().surface_style() {
                        if let Some(scene) = AppContext::surface_style_manager() {
                            let qh = AppContext::queue_handle();

                            let timing = scene.create_timing_function(qh, ());
                            timing.set_spring(0.1, 0.1);
                            let animation = scene.begin_transaction(qh, ());
                            animation.set_duration(0.5);
                            animation.set_delay(0.0);
                            animation.set_timing_function(&timing);

                            scene_surface.set_blend_mode(BlendMode::BackgroundBlur);
                            scene_surface.set_opacity(1.0);

                            animation.commit();
                        }
                    }
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
        if self.popups.borrow().len() > depth {
            if self.popups.borrow()[depth].borrow().is_some() {
                return;
            }
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
                menu_self.hide();
            });

            AppContext::register_popup_configure_callback(surface_id, move |_serial| {
                // NOTE: SCTK's Popup already calls ack_configure internally, so we must NOT call it again!

                if let Some(popup) = popup_ref.borrow_mut().as_mut() {
                    ContextMenu::apply_surface_effects(&style, &popup);

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
                    ContextMenu::apply_surface_effects(&style, &popup);
                    popup.mark_configured();

                    Self::render_menu_at_depth(&state, &style, &popup_ref, depth);
                }
            });
        }
    }

    /// Hide the menu (closes all popups)
    pub fn hide(&self) {
        // TODO: Add fade-out animation with close_delay from style
        // For now, immediate close
        for popup in self.popups.borrow().iter() {
            *popup.borrow_mut() = None;
        }
        self.state.borrow_mut().reset();
    }

    /// Hide the menu with animation delay
    pub fn hide_animated(&self) {
        // let close_delay = self.style.borrow().close_delay;

        // TODO: Implement actual fade-out animation
        // For now, just sleep for the delay then close

        self.hide();
    }

    // === Submenu Management ===

    /// Show submenu for item at given depth (static helper for callbacks)
    fn show_submenu_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &Rc<RefCell<Vec<Rc<RefCell<Option<PopupSurface>>>>>>,
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
            if popups_borrowed.len() > depth + 1 {
                if popups_borrowed[depth + 1].borrow().is_some() {
                    drop(popups_borrowed);
                    Self::hide_submenus_from_static(state, popups, depth + 1);
                    state.borrow_mut().close_submenus_from(depth + 1);
                }
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
        } else {
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
                    .map(|x| x.clone())
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
                    1 as i32,                // height: selected item height
                );

                // Anchor to top-left of this line (which is at parent's right edge)
                positioner.set_anchor(xdg_positioner::Anchor::TopLeft);

                // Place submenu to the right of the anchor point
                positioner.set_gravity(xdg_positioner::Gravity::BottomRight);

                // Ensure popups vec is large enough for submenu
                while popups.borrow().len() <= depth + 1 {
                    popups.borrow_mut().push(Rc::new(RefCell::new(None)));
                }

                // Create submenu surface
                if let Ok(popup) =
                    PopupSurface::new(&parent_xdg, &positioner, width as i32, height as i32)
                {
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
        popups: &Rc<RefCell<Vec<Rc<RefCell<Option<PopupSurface>>>>>>,
        from_depth: usize,
    ) {
        let popups_borrowed = popups.borrow();
        for i in from_depth..popups_borrowed.len() {
            *popups_borrowed[i].borrow_mut() = None;
        }
        // Don't call close_submenus_from here - state is managed by caller
    }

    // === Event Handling ===

    /// Register pointer event handler (called only by root menu)
    fn register_pointer_handler(&mut self) {
        let registered_surfaces = self.registered_surfaces.clone();
        let state = self.state.clone();
        let style = self.style.clone(); // Clone the Rc, not the value
        let popups = self.popups.clone();
        let on_item_click = self.on_item_click.clone();
        let parent_xdg = self.parent_xdg.clone();

        AppContext::register_pointer_callback(move |events| {
            for event in events {
                let surface_id = event.surface.id();
                let depth = registered_surfaces.borrow().get(&surface_id).cloned();
                // Look up depth for this surface
                if let Some(depth) = depth {
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
                        PointerEventKind::Press { button, .. } => {
                            if button == 0x110 {
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
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    /// Handle pointer motion at specific depth
    fn handle_motion_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &Rc<RefCell<Vec<Rc<RefCell<Option<PopupSurface>>>>>>,
        style: &Rc<RefCell<ContextMenuStyle>>,
        registered_surfaces: &Rc<RefCell<HashMap<ObjectId, usize>>>,
        parent_xdg: &Rc<RefCell<Option<xdg_surface::XdgSurface>>>,
        depth: usize,
        x: f64,
        y: f64,
    ) {
        // Get items for this depth
        let items = {
            let state_borrow = state.borrow();
            state_borrow.items_at_depth(depth).to_vec()
        };

        // Hit test
        let style_borrowed = style.borrow();
        let item_index =
            ContextMenuRenderer::hit_test_items(&items, &style_borrowed, x as f32, y as f32);
        drop(style_borrowed);

        // Update selection at this depth
        let mut state_mut = state.borrow_mut();
        let old_selection = state_mut.selected_at_depth(depth);
        state_mut.select_at_depth(depth, item_index);

        if old_selection != item_index {
            drop(state_mut);

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
                // Mouse left the menu area - close submenus and update state
                state.borrow_mut().close_submenus_from(depth);
                Self::hide_submenus_from_static(state, popups, depth + 1);
            }
        }
    }

    /// Handle click with animation at specific depth
    fn handle_click_static(
        state: &Rc<RefCell<ContextMenuState>>,
        popups: &Rc<RefCell<Vec<Rc<RefCell<Option<PopupSurface>>>>>>,
        style: &Rc<RefCell<ContextMenuStyle>>,
        on_item_click: &Rc<RefCell<Option<Rc<dyn Fn(&str)>>>>,
        depth: usize,
        x: f32,
        y: f32,
    ) {
        // Get items for this depth
        let items = {
            let state_borrow = state.borrow();
            state_borrow.items_at_depth(depth).to_vec()
        };

        // Hit test
        let style_borrowed = style.borrow();
        let item_index = ContextMenuRenderer::hit_test_items(&items, &style_borrowed, x, y);
        drop(style_borrowed);

        if let Some(idx) = item_index {
            if let Some(label) = items.get(idx).and_then(|item| item.label()) {
                let label = label.to_string();

                // Clone popup ref to avoid holding borrow during sleep/callback
                let popup_ref = if depth < popups.borrow().len() {
                    Some(popups.borrow()[depth].clone())
                } else {
                    None
                };

                // Click animation (slowed down for verification)
                state.borrow_mut().select_at_depth(depth, None);
                if let Some(ref popup_ref) = popup_ref {
                    let style_borrowed = style.borrow();
                    Self::render_menu_at_depth(state, &style_borrowed, popup_ref, depth);
                }
                std::thread::sleep(std::time::Duration::from_millis(50)); // Slowed: was 50ms

                state.borrow_mut().select_at_depth(depth, Some(idx));
                if let Some(ref popup_ref) = popup_ref {
                    let style_borrowed = style.borrow();
                    Self::render_menu_at_depth(state, &style_borrowed, popup_ref, depth);
                }
                std::thread::sleep(std::time::Duration::from_millis(100)); // Slowed: was 100ms

                // Fire callback
                if let Some(callback) = on_item_click.borrow().as_ref() {
                    callback(&label);
                }

                // Close all popups and reset state
                for popup in popups.borrow().iter() {
                    *popup.borrow_mut() = None;
                }
                state.borrow_mut().reset();
            }
        }
    }

    /// Render at a specific depth (static helper for callbacks)
    fn render_menu_at_depth(
        state: &Rc<RefCell<ContextMenuState>>,
        style: &ContextMenuStyle,
        popup: &Rc<RefCell<Option<PopupSurface>>>,
        depth: usize,
    ) {
        // Get items and dimensions before borrowing popup
        let (items_vec, selected, width, height) = {
            let state_borrow = state.borrow();
            let items = state_borrow.items_at_depth(depth);
            let selected = state_borrow.selected_at_depth(depth);
            let (w, h) = ContextMenuRenderer::measure_items(items, &style);
            (items.to_vec(), selected, w, h)
        };

        // Now borrow popup and draw (no other borrows held)
        if let Some(popup_surface) = popup.borrow().as_ref() {
            popup_surface.draw(|canvas| {
                ContextMenuRenderer::render_depth(
                    canvas, &items_vec, selected, &style, width, height,
                );
            });
        }
    }

    /// Handle keyboard input
    pub fn handle_key(&mut self, key: u32, key_state: wl_keyboard::KeyState) {
        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }
        let style = self.style.borrow(); // Borrow once

        match key {
            keycodes::DOWN => {
                println!("Key DOWN at depth {}", self.state.borrow().depth());
                self.state.borrow_mut().select_next_at_depth(None); // Use state's depth
                let current_depth = self.state.borrow().depth();
                // Render at current depth
                if current_depth < self.popups.borrow().len() {
                    let popup_ref = self.popups.borrow()[current_depth].clone();
                    Self::render_menu_at_depth(&self.state, &style, &popup_ref, current_depth);
                }
            }
            keycodes::UP => {
                self.state.borrow_mut().select_previous_at_depth(None); // Use state's depth
                let current_depth = self.state.borrow().depth();
                // Render at current depth
                if current_depth < self.popups.borrow().len() {
                    let popup_ref = self.popups.borrow()[current_depth].clone();
                    Self::render_menu_at_depth(&self.state, &style, &popup_ref, current_depth);
                }
            }
            keycodes::ENTER => {
                let current_depth = self.state.borrow().depth();
                let state = self.state.borrow();
                let selected_idx = state.selected_at_depth(current_depth);
                let label = state.selected_label(None); // Use state's depth

                if let (Some(idx), Some(label)) = (selected_idx, label) {
                    let label_owned = label.to_string();
                    drop(state);

                    // Get popup ref for animation
                    let popup_ref = if current_depth < self.popups.borrow().len() {
                        Some(self.popups.borrow()[current_depth].clone())
                    } else {
                        None
                    };

                    // Click animation (same as mouse click)
                    self.state.borrow_mut().select_at_depth(current_depth, None);
                    if let Some(ref popup_ref) = popup_ref {
                        Self::render_menu_at_depth(&self.state, &style, popup_ref, current_depth);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));

                    self.state
                        .borrow_mut()
                        .select_at_depth(current_depth, Some(idx));
                    if let Some(ref popup_ref) = popup_ref {
                        Self::render_menu_at_depth(&self.state, &style, popup_ref, current_depth);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));

                    // Fire callback
                    if let Some(callback) = self.on_item_click.borrow().as_ref() {
                        callback(&label_owned);
                    }

                    // Close all popups and reset state
                    for popup in self.popups.borrow().iter() {
                        *popup.borrow_mut() = None;
                    }
                    self.state.borrow_mut().reset();
                }
            }
            keycodes::ESC => {
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
                        self.state.borrow_mut().open_submenu(current_depth, idx);
                        self.state
                            .borrow_mut()
                            .select_at_depth(current_depth + 1, Some(0));

                        // 2. Show the submenu surface
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
                // Close submenu and move back to parent
                if current_depth > 0 {
                    let target_depth = current_depth - 1;

                    // Hide submenu surfaces from current depth onwards
                    Self::hide_submenus_from_static(&self.state, &self.popups, current_depth);

                    // Update state: truncate to target_depth and set depth to target_depth
                    self.state.borrow_mut().close_submenus_from(target_depth);

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
            scene_surface.set_corner_radius(style.corner_radius as f64);
            scene_surface.set_masks_to_bounds(ClipMode::Enabled);
            // scene_surface.set_background_color(1.0, 0.2, 0.2, 1.0);
            scene_surface.set_shadow(0.2, 2.0, 0.0, 7.0, 0.3, 0.3, 0.3);
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

    /// Handle keyboard focus lost - closes the menu
    pub fn handle_keyboard_leave(&mut self) {
        self.hide();
    }

    /// Check if menu should close and fire callback
    fn check_close(&mut self) {
        if self.state.borrow().should_close() {
            if let Some(callback) = self.on_close.borrow().as_ref() {
                callback();
            }
            self.hide();
        }
    }
}
