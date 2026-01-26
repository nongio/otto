use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{
        protocol::{wl_pointer, wl_surface},
        Connection, QueueHandle,
    },
    shell::xdg::{
        popup::{Popup, PopupConfigure, PopupData},
        XdgPositioner, XdgShell, XdgSurface,
    },
};
use std::collections::HashMap;
use wayland_client::{protocol::wl_keyboard, Proxy};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_surface};

use crate::{
    components::{menu::sc_layer_v1::ScLayerV1, window::Window},
    surfaces::{PopupSurface, ScLayerAugment, Surface},
};

use super::{
    data::{Anchor, Gravity, MenuItem, MenuItemId, MenuStyle, Position},
    drawing::draw_menu,
    sc_layer_protocol::{sc_layer_shell_v1, sc_layer_v1},
    MenuError,
};

/// Main menu component
pub struct Menu {
    // Menu data
    items: Vec<MenuItem>,
    style: MenuStyle,

    // Root menu surface (created on show)
    root: Option<MenuSurface>,

    // Interaction state
    hovered_item: Option<usize>,
    pointer_x: f64,
    pointer_y: f64,
    prev_pointer_x: f64,
    prev_pointer_y: f64,

    // Submenu tracking
    hovering_submenu: bool, // Track if pointer is over a submenu surface

    // Click handler
    on_click: Option<Box<dyn Fn(&MenuItemId)>>,

    // sc_layer_shell for enhancing surfaces
    sc_layer_shell: Option<sc_layer_shell_v1::ScLayerShellV1>,
}

impl Menu {
    /// Create a new menu
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            style: MenuStyle::default(),
            root: None,
            hovered_item: None,
            pointer_x: 0.0,
            pointer_y: 0.0,
            prev_pointer_x: 0.0,
            prev_pointer_y: 0.0,
            hovering_submenu: false,
            on_click: None,
            sc_layer_shell: None,
        }
    }

    /// Set the click handler
    pub fn set_on_click<F>(&mut self, handler: F)
    where
        F: Fn(&MenuItemId) + 'static,
    {
        self.on_click = Some(Box::new(handler));
    }

    /// Set custom styling
    pub fn set_style(&mut self, style: MenuStyle) {
        self.style = style;
    }

    /// Show the menu at the specified position
    pub fn open_menu<D>(
        &mut self,
        parent_window: &Window,
        position: Position,
        qh: &QueueHandle<D>,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
        _conn: &Connection,
        display_ptr: *mut std::ffi::c_void,
    ) -> Result<(), MenuError>
    where
        D: wayland_client::Dispatch<wl_surface::WlSurface, SurfaceData>
            + wayland_client::Dispatch<xdg_surface::XdgSurface, PopupData>
            + wayland_client::Dispatch<xdg_popup::XdgPopup, PopupData>
            + wayland_client::Dispatch<sc_layer_v1::ScLayerV1, ()>
            + wayland_client::Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
            + 'static,
    {
        // Update style to reflect sc-layer availability
        self.style.sc_layer = sc_layer_shell.is_some();
        self.sc_layer_shell = sc_layer_shell.cloned();

        // Close existing menu if open
        if self.root.is_some() {
            self.hide();
        }

        // Create root menu surface
        let width = self.style.calculate_menu_width(&self.items) as i32;
        let height = self.style.calculate_menu_height(&self.items) as i32;

        // Create positioner
        let positioner = create_positioner(xdg_shell, qh, &self.items, &position, &self.style)?;

        // Get parent XDG surface from Window component
        let parent_xdg = parent_window.surface().ok_or(MenuError::NoParent)?;
        let parent_xdg = parent_xdg.window().xdg_surface();

        // Create PopupSurface component
        let popup_surface = PopupSurface::new(
            parent_xdg,
            &positioner,
            width,
            height,
            compositor,
            xdg_shell,
            qh,
        )
        .map_err(|_| MenuError::SurfaceCreationFailed)?;

        let menu_surface = MenuSurface {
            popup_surface,
            items: self.items.clone(),
            hovered_item: None,
            needs_redraw: false,
            open_submenus: HashMap::new(),
            frame_callback_pending: false,
        };

        self.root = Some(menu_surface);

        Ok(())
    }

    /// Hide the menu
    pub fn hide(&mut self) {
        // Close all popups without destroying surfaces/EGL context
        if let Some(ref mut root) = self.root {
            root.close_recursive();
        }

        self.root = None;

        // Reset all interaction state
        self.hovered_item = None;
        self.pointer_x = 0.0;
        self.pointer_y = 0.0;
        self.prev_pointer_x = 0.0;
        self.prev_pointer_y = 0.0;
        self.hovering_submenu = false;
    }

    /// Check if menu is visible
    pub fn is_visible(&self) -> bool {
        self.root.is_some()
    }

    /// Handle frame callback
    pub fn on_frame_callback<D>(&mut self, surface: &wl_surface::WlSurface, qh: &QueueHandle<D>)
    where
        D: 'static,
    {
        if let Some(root) = &mut self.root {
            root.handle_frame_callback_recursive(surface, &self.style, qh);
        }
    }

    /// Handle pointer enter event
    pub fn on_pointer_enter<D>(
        &mut self,
        surface: &wl_surface::WlSurface,
        x: f64,
        y: f64,
        qh: &QueueHandle<D>,
    ) where
        D: wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + 'static,
    {
        if let Some(root) = &mut self.root {
            if root.popup_surface.wl_surface() == surface {
                self.pointer_x = x;
                self.pointer_y = y;
                self.update_hover(qh);
            }
        }
    }

    /// Handle pointer motion event
    pub fn on_pointer_motion<D>(
        &mut self,
        surface: &wl_surface::WlSurface,
        x: f64,
        y: f64,
        qh: &QueueHandle<D>,
    ) where
        D: wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + 'static,
    {
        self.prev_pointer_x = self.pointer_x;
        self.prev_pointer_y = self.pointer_y;
        self.pointer_x = x;
        self.pointer_y = y;

        // Check if this is the root surface
        if self
            .root
            .as_ref()
            .map_or(false, |r| r.popup_surface.wl_surface() == surface)
        {
            self.hovering_submenu = false;
            self.update_hover(qh);
        } else {
            // Use recursive helper to find and handle the surface
            self.hovering_submenu = false;
            if let Some(root) = &mut self.root {
                let mut active_path = Vec::new();
                if root.handle_pointer_motion_recursive(
                    surface,
                    y,
                    &self.style,
                    &mut active_path,
                    qh,
                ) {
                    self.hovering_submenu = true;
                    // Only close inactive submenus if we're not hovering a submenu item that's about to open
                    // The submenu opening logic will handle this
                    // Don't call close_inactive_submenus here - it interferes with submenu creation
                }
            }
        }
    }

    /// Handle pointer leave event
    pub fn on_pointer_leave(&mut self, surface: &wl_surface::WlSurface) {
        if let Some(root) = &mut self.root {
            root.handle_pointer_leave_recursive(surface, &self.style);
        }
    }

    /// Handle pointer button event
    pub fn on_pointer_button(&mut self, button: u32, state: wl_pointer::ButtonState) {
        if button == 272 && state == wl_pointer::ButtonState::Pressed {
            // Left click - check root and all submenus recursively
            if let Some(ref handler) = self.on_click {
                if let Some(root) = &self.root {
                    if root.handle_pointer_button_recursive(handler) {
                        // An item was clicked, close the menu
                        self.hide();
                    }
                }
            }
        }
    }

    /// Handle keyboard key event
    pub fn on_keyboard_key(&mut self, key: u32, state: wl_keyboard::KeyState) {
        if state == wl_keyboard::KeyState::Pressed {
            match key {
                1 => {
                    // Escape - close menu
                    self.hide();
                }
                103 => {
                    // Up arrow
                    self.navigate_up();
                }
                108 => {
                    // Down arrow
                    self.navigate_down();
                }
                28 => {
                    // Enter - activate item
                    if let Some(hover_idx) = self.hovered_item {
                        if let Some(item) = self.items.get(hover_idx) {
                            if !item.is_separator() && !item.is_submenu() && item.is_enabled() {
                                if let Some(ref handler) = self.on_click {
                                    if let Some(id) = item.id() {
                                        handler(&MenuItemId::from(id));
                                    }
                                }
                                self.hide();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle configure event for a specific popup
    pub fn on_configure<D>(
        &mut self,
        popup: &Popup,
        configure: PopupConfigure,
        qh: &QueueHandle<D>,
        conn: &Connection,
    ) where
        D: wayland_client::Dispatch<sc_layer_v1::ScLayerV1, ()>
            + wayland_client::Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
            + wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + 'static,
    {
        let popup_surface = popup.wl_surface();

        // Check if it's the root menu

        if let Some(root) = &mut self.root {
            if root.handle_configure_recursive(&popup_surface.id(), configure, &self.style, 0, qh) {
                // Enhance with sc_layer protocol if available (only on first configure)
                conn.roundtrip().ok();
                return;
            }
        }
    }

    /// Update hover state based on pointer position
    fn update_hover<D>(&mut self, qh: &QueueHandle<D>)
    where
        D: wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + 'static,
    {
        let new_hover = self.item_at_position(self.pointer_y as f32);
        if let Some(root) = &mut self.root {
            if new_hover != root.hovered_item {
                root.hovered_item = new_hover;
                self.hovered_item = new_hover; // Sync the top-level field too
                root.set_needs_redraw(qh);

                // Close submenus that don't match the currently hovered item
                let hovered_idx = new_hover;
                for (sub_idx, sub_submenu) in root.open_submenus.iter_mut() {
                    if Some(*sub_idx) != hovered_idx {
                        sub_submenu.close();
                    }
                }
            }
        }
    }

    /// Check if we should open a submenu and return its path and index
    /// Returns (parent_path, item_idx) where parent_path is the indices to navigate to the parent menu
    pub fn should_open_submenu(&self) -> Option<(Vec<usize>, usize)> {
        // Check root menu first
        if let Some(root) = &self.root {
            if let Some(result) = root.check_should_open_submenu_recursive(&[]) {
                return Some(result);
            }
        }
        None
    }

    /// Check if we should close submenus
    pub fn should_close_submenus(&self) -> bool {
        // Count configured submenus
        let configured_count = self.root.as_ref().map_or(0, |r| {
            r.open_submenus
                .values()
                .filter(|s| s.popup_surface.is_configured())
                .count()
        });

        // Never close if we have no configured submenus
        if configured_count == 0 {
            return false;
        }

        // Never close if we're hovering over a submenu surface
        if self.hovering_submenu {
            return false;
        }

        // Check triangle/safe zone - if mouse is moving toward any open submenu, don't close
        if self.is_moving_toward_submenus() {
            return false;
        }

        match self.hovered_item {
            Some(idx) => {
                // Don't close if hovering the parent item of a configured submenu
                if self
                    .root
                    .as_ref()
                    .and_then(|r| r.open_submenus.get(&idx))
                    .map_or(false, |s| s.popup_surface.is_configured())
                {
                    return false;
                }

                // Don't close if hovering another submenu item (we'll switch submenus instead)
                if let Some(item) = self.items.get(idx) {
                    if item.is_submenu() {
                        return false;
                    }
                }

                // Don't close submenus just because we're hovering a regular item
                // Only close if there are open submenus that don't match any hovered items
                false
            }
            None => {
                // If not hovering anything in the root menu, only close if we're
                // also not in the submenu and not moving toward it
                // (triangle logic already checked above)
                true
            }
        }
    }

    /// Check if mouse is moving toward any open submenu (triangle/safe zone logic)
    fn is_moving_toward_submenus(&self) -> bool {
        // Get root menu width to calculate submenu absolute positions
        let root_width = self.style.calculate_menu_width(&self.items);

        // Get the open submenus from root
        let open_submenus = match self.root.as_ref() {
            Some(root) => &root.open_submenus,
            None => return false,
        };

        // Check each configured submenu
        for (item_idx, submenu) in open_submenus.iter() {
            // Skip unconfigured (closed) submenus
            if !submenu.popup_surface.is_configured() {
                continue;
            }

            // Calculate submenu position (positioned to the right of root menu)
            let submenu_left = root_width;
            let submenu_right = root_width + self.style.calculate_menu_width(&submenu.items);

            // Calculate Y position of this submenu (aligned with its parent item)
            let mut submenu_top = self.style.padding_vertical;
            for (idx, item) in self.items.iter().enumerate() {
                if idx == *item_idx {
                    break;
                }
                if item.is_separator() {
                    submenu_top += self.style.separator_height;
                } else {
                    submenu_top += self.style.item_height;
                }
            }
            let submenu_bottom = submenu_top + self.style.calculate_menu_height(&submenu.items);

            // Check if moving toward this submenu using triangle algorithm
            if self.is_moving_toward_rect(
                submenu_left as f64,
                submenu_top as f64,
                submenu_right as f64,
                submenu_bottom as f64,
            ) {
                return true;
            }
        }

        false
    }

    /// Triangle/safe zone algorithm: check if mouse trajectory is toward a rectangle
    /// This creates a triangle from the current mouse position to the two corners
    /// of the rectangle edge closest to the mouse, and checks if the movement
    /// vector points within that triangle
    fn is_moving_toward_rect(
        &self,
        rect_left: f64,
        rect_top: f64,
        rect_right: f64,
        rect_bottom: f64,
    ) -> bool {
        // Movement vector
        let dx = self.pointer_x - self.prev_pointer_x;
        let dy = self.pointer_y - self.prev_pointer_y;

        // If mouse hasn't moved significantly, keep submenu open
        if dx.abs() < 0.1 && dy.abs() < 0.1 {
            return true;
        }

        // Determine which edge of the rectangle is closest to the mouse
        // and use its two corners to form the triangle
        let (corner1_x, corner1_y, corner2_x, corner2_y) = if self.pointer_x < rect_left {
            // Mouse is to the left - use left edge corners
            (rect_left, rect_top, rect_left, rect_bottom)
        } else if self.pointer_x > rect_right {
            // Mouse is to the right - use right edge corners
            (rect_right, rect_top, rect_right, rect_bottom)
        } else {
            // Mouse is horizontally within bounds - use top or bottom edge
            if self.pointer_y < rect_top {
                (rect_left, rect_top, rect_right, rect_top)
            } else {
                (rect_left, rect_bottom, rect_right, rect_bottom)
            }
        };

        // Vectors from current position to the two corners
        let to_corner1_x = corner1_x - self.pointer_x;
        let to_corner1_y = corner1_y - self.pointer_y;
        let to_corner2_x = corner2_x - self.pointer_x;
        let to_corner2_y = corner2_y - self.pointer_y;

        // Use cross product to check if movement vector is between the two corner vectors
        // If the cross products have opposite signs, the movement is within the triangle
        let cross1 = dx * to_corner1_y - dy * to_corner1_x;
        let cross2 = dx * to_corner2_y - dy * to_corner2_x;

        // Movement is toward rectangle if it's between the two corner vectors
        cross1 * cross2 <= 0.0
    }

    /// Find which item is at the given Y position
    fn item_at_position(&self, y: f32) -> Option<usize> {
        find_item_at_position(&self.items, y, &self.style)
    }

    /// Navigate to previous item
    fn navigate_up(&mut self) {
        let current = self.hovered_item.unwrap_or(0);
        if current > 0 {
            self.hovered_item = Some(current - 1);
            if let Some(root) = &mut self.root {
                root.hovered_item = self.hovered_item;
                root.needs_redraw = true;
                root.render(&self.style);
                root.needs_redraw = false;
            }
        }
    }

    /// Navigate to next item
    fn navigate_down(&mut self) {
        let max_index = self.items.iter().filter(|i| !i.is_separator()).count();
        let current = self.hovered_item.unwrap_or(0);
        if current + 1 < max_index {
            self.hovered_item = Some(current + 1);
            if let Some(root) = &mut self.root {
                root.hovered_item = self.hovered_item;
                root.needs_redraw = true;
                root.render(&self.style);
                root.needs_redraw = false;
            }
        }
    }

    /// Get the root surface (if visible)
    pub fn root_surface(&self) -> Option<&wl_surface::WlSurface> {
        self.root.as_ref().map(|r| r.popup_surface.wl_surface())
    }

    /// Check if a surface belongs to this menu
    pub fn owns_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        // Check root surface
        if self
            .root
            .as_ref()
            .map_or(false, |r| r.popup_surface.wl_surface() == surface)
        {
            return true;
        }

        // Use recursive helper to check all submenu levels
        if let Some(root) = &self.root {
            for submenu in root.open_submenus.values() {
                if submenu.owns_surface_recursive(surface) {
                    return true;
                }
            }
        }

        false
    }

    /// Close all open submenus
    pub fn close_all_submenus(&mut self) {
        // Use recursive helper to close all nested submenus
        if let Some(root) = &mut self.root {
            root.close_all_submenus_recursive();
        }

        // Reset submenu-related state
        self.hovering_submenu = false;
    }

    /// Open a submenu at the given item index
    /// parent_path: indices to navigate to the parent menu (empty for root)
    pub fn open_submenu<D>(
        &mut self,
        parent_path: Vec<usize>,
        item_index: usize,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        qh: &QueueHandle<D>,
        pointer_x: f64,
    ) -> Result<(), MenuError>
    where
        D: wayland_client::Dispatch<wl_surface::WlSurface, SurfaceData>
            + wayland_client::Dispatch<xdg_surface::XdgSurface, PopupData>
            + wayland_client::Dispatch<xdg_popup::XdgPopup, PopupData>
            + wayland_client::Dispatch<sc_layer_v1::ScLayerV1, ()>
            + wayland_client::Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
            + 'static,
    {
        // Navigate to the parent menu using the path
        let mut current = self.root.as_ref().ok_or(MenuError::SurfaceCreationFailed)?;

        for &idx in &parent_path {
            current = current
                .open_submenus
                .get(&idx)
                .ok_or(MenuError::SurfaceCreationFailed)?;
        }

        // Get the parent menu items and dimensions
        let parent_items = current.items.clone();
        let (parent_width, _) = current.popup_surface.dimensions();

        // Get the submenu items
        let (submenu_items, item_y_position) =
            if let Some(MenuItem::Submenu { items, .. }) = parent_items.get(item_index) {
                // Calculate Y position of this item
                let mut y = self.style.padding_vertical;
                for (idx, item) in parent_items.iter().enumerate() {
                    if idx == item_index {
                        break;
                    }
                    if item.is_separator() {
                        y += self.style.separator_height;
                    } else {
                        y += self.style.item_height;
                    }
                }
                (items.clone(), y as i32)
            } else {
                return Ok(()); // Not a submenu
            };

        // Navigate to parent menu to check/insert the submenu
        let mut insert_current = self.root.as_mut().ok_or(MenuError::SurfaceCreationFailed)?;
        for &idx in &parent_path {
            insert_current = insert_current
                .open_submenus
                .get_mut(&idx)
                .ok_or(MenuError::SurfaceCreationFailed)?;
        }

        // Create positioner for the submenu
        let submenu_width = self.style.calculate_menu_width(&submenu_items) as i32;
        let submenu_height = self.style.calculate_menu_height(&submenu_items) as i32;

        let positioner =
            XdgPositioner::new(xdg_shell).map_err(|_| MenuError::SurfaceCreationFailed)?;
        positioner.set_size(submenu_width, submenu_height);

        // Calculate anchor X position based on pointer X position
        // Clamp to 70% of parent width to keep some distance from left edge
        let max_x = (parent_width as f64 * 0.4).round() as i32;
        let anchor_x = (pointer_x.round() as i32).max(max_x) + 15;

        positioner.set_anchor_rect(
            anchor_x,
            item_y_position + 10,
            1,
            self.style.item_height as i32,
        );

        use wayland_protocols::xdg::shell::client::xdg_positioner::{Anchor, Gravity};
        positioner.set_anchor(Anchor::TopRight);
        positioner.set_gravity(Gravity::BottomRight);

        // Get parent popup's XDG surface (borrowed from insert_current)
        let parent_xdg = insert_current
            .popup_surface
            .xdg_surface()
            .ok_or(MenuError::SurfaceCreationFailed)?;

        // Check if submenu already exists
        let submenu_surface = insert_current
            .open_submenus
            .entry(item_index)
            .or_insert_with(|| {
                // Create new PopupSurface for submenu
                let popup_surface = PopupSurface::new(
                    parent_xdg,
                    &positioner,
                    submenu_width,
                    submenu_height,
                    compositor,
                    xdg_shell,
                    qh,
                )
                .expect("Failed to create submenu popup surface");

                MenuSurface {
                    popup_surface,
                    items: submenu_items.clone(),
                    hovered_item: None,
                    needs_redraw: true,
                    open_submenus: HashMap::new(),
                    frame_callback_pending: false,
                }
            });

        // If popup is not active, reopen it (recreates popup on existing surface)
        if !submenu_surface.popup_surface.is_active() {
            submenu_surface
                .popup_surface
                .show(parent_xdg, &positioner, xdg_shell, compositor, qh)
                .map_err(|_| MenuError::SurfaceCreationFailed)?;
            submenu_surface.needs_redraw = true;
        }

        Ok(())
    }
}

/// Represents a single menu surface
struct MenuSurface {
    popup_surface: PopupSurface,
    items: Vec<MenuItem>,
    hovered_item: Option<usize>,
    needs_redraw: bool,
    open_submenus: HashMap<usize, MenuSurface>,
    frame_callback_pending: bool,
}

impl MenuSurface {
    /// Close this popup and all submenus recursively without destroying surfaces
    fn close_recursive(&mut self) {
        // Close all submenus first
        for submenu in self.open_submenus.values_mut() {
            submenu.close_recursive();
        }

        // Close this popup (keeps surface and Skia context)
        self.popup_surface.close();
    }

    /// Recursively handle frame callback for this surface or any submenu
    fn handle_frame_callback_recursive<D>(
        &mut self,
        surface: &wl_surface::WlSurface,
        style: &MenuStyle,
        _qh: &QueueHandle<D>,
    ) -> bool
    where
        D: 'static,
    {
        if self.popup_surface.wl_surface() == surface {
            self.on_frame_callback(style);
            return true;
        }

        for submenu in self.open_submenus.values_mut() {
            if submenu.handle_frame_callback_recursive(surface, style, _qh) {
                return true;
            }
        }

        false
    }

    /// Recursively handle configure event for this surface or any of its submenus
    fn handle_configure_recursive<D>(
        &mut self,
        popup_surface_id: &wayland_client::backend::ObjectId,
        configure: PopupConfigure,
        style: &MenuStyle,
        depth: usize,
        qh: &QueueHandle<D>,
    ) -> bool
    where
        D: wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + wayland_client::Dispatch<sc_layer_v1::ScLayerV1, ()>
            + 'static,
    {
        // Check if this is the surface we're looking for
        if &self.popup_surface.wl_surface().id() == popup_surface_id {
            if !self.popup_surface.is_configured() {
                // Mark as configured (configure is already acked by smithay_client_toolkit)
                self.popup_surface.mark_configured();
                // Mark for redraw to ensure we render with current hover state
                self.needs_redraw = true;

                // Apply sc_layer styling on first configure
                let _ = self.popup_surface.augment(
                    Some(|layer: &ScLayerV1| {
                        println!(
                            "Applying sc-layer styling to menu surface at depth {}",
                            depth
                        );
                        layer.set_corner_radius(24.0);
                        layer.set_masks_to_bounds(1);
                        layer.set_blend_mode(sc_layer_v1::BlendMode::BackgroundBlur);
                        layer.set_background_color(0.7, 0.7, 0.7, 0.9);
                        layer.set_border(1.0, 1.0, 1.0, 1.0, 0.8);
                        layer.set_shadow(0.4, 40.0, 0.0, 0.0, 0.0, 0.0, 0.0);
                    }),
                    qh,
                );
            }

            // Render immediately on first configure
            self.render(style);
            return true;
        }

        // Recursively check all submenus
        for (_idx, submenu) in self.open_submenus.iter_mut() {
            if submenu.handle_configure_recursive(
                popup_surface_id,
                configure.clone(),
                style,
                depth + 1,
                qh,
            ) {
                return true;
            }
        }

        false
    }

    /// Recursively check if this surface or any submenu has a hovered submenu item that should be opened
    /// Returns (parent_index_in_THIS_menu, item_index_to_open)
    fn check_should_open_submenu_recursive(
        &self,
        parent_path: &[usize],
    ) -> Option<(Vec<usize>, usize)> {
        // Check if this surface has a hovered submenu item
        if let Some(item_idx) = self.hovered_item {
            if let Some(item) = self.items.get(item_idx) {
                if item.is_submenu() {
                    // Only open if submenu doesn't exist
                    if !self.open_submenus.contains_key(&item_idx) {
                        return Some((parent_path.to_vec(), item_idx));
                    } else {
                        let submenu = self.open_submenus.get(&item_idx).unwrap();
                        if !submenu.popup_surface.is_active() {
                            return Some((parent_path.to_vec(), item_idx));
                        }
                    }
                }
            }
        }

        // Recursively check all configured submenus
        for (idx, submenu) in &self.open_submenus {
            if submenu.popup_surface.is_configured() {
                let mut new_path = parent_path.to_vec();
                new_path.push(*idx);
                if let Some(result) = submenu.check_should_open_submenu_recursive(&new_path) {
                    return Some(result);
                }
            }
        }

        None
    }

    /// Recursively close all submenus of this surface
    fn close_all_submenus_recursive(&mut self) {
        // Close all submenus recursively
        for submenu in self.open_submenus.values_mut() {
            submenu.close();
        }
    }

    /// Recursively handle pointer motion for this surface or any submenu
    /// Returns true if the surface was found and handled
    fn handle_pointer_motion_recursive<D>(
        &mut self,
        surface: &wl_surface::WlSurface,
        y: f64,
        style: &MenuStyle,
        active_path: &mut Vec<usize>,
        qh: &QueueHandle<D>,
    ) -> bool
    where
        D: wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + 'static,
    {
        // Check if this is the surface we're looking for
        if self.popup_surface.wl_surface() == surface {
            // Only handle if configured
            if self.popup_surface.is_configured() {
                // Update this surface's hover state
                let new_hover = self.item_at_position(y as f32, style);
                if self.hovered_item != new_hover {
                    self.hovered_item = new_hover;
                    self.set_needs_redraw(qh);

                    // Close submenus that don't match the currently hovered item
                    let hovered_idx = new_hover;
                    for (sub_idx, sub_submenu) in self.open_submenus.iter_mut() {
                        if Some(*sub_idx) != hovered_idx {
                            sub_submenu.close();
                            sub_submenu.hovered_item = None;
                        }
                    }
                }
            }
            return true;
        }

        // Recursively check all submenus (not just configured ones)
        for (idx, submenu) in self.open_submenus.iter_mut() {
            active_path.push(*idx);
            if submenu.handle_pointer_motion_recursive(surface, y, style, active_path, qh) {
                return true;
            }
            active_path.pop();
        }

        false
    }

    /// Recursively check if this surface or any submenu owns the given surface
    fn owns_surface_recursive(&self, surface: &wl_surface::WlSurface) -> bool {
        // Check this surface
        if self.popup_surface.is_configured() && self.popup_surface.wl_surface() == surface {
            return true;
        }

        // Recursively check all submenus
        for submenu in self.open_submenus.values() {
            if submenu.owns_surface_recursive(surface) {
                return true;
            }
        }

        false
    }

    /// Recursively handle pointer button click - returns true if an item was clicked
    fn handle_pointer_button_recursive<F>(&self, handler: &F) -> bool
    where
        F: Fn(&MenuItemId),
    {
        // Check if this surface has a clicked item
        if let Some(hover_idx) = self.hovered_item {
            if let Some(item) = self.items.get(hover_idx) {
                if !item.is_separator() && !item.is_submenu() && item.is_enabled() {
                    // Clicked an action item
                    if let Some(id) = item.id() {
                        handler(&MenuItemId::from(id));
                        return true;
                    }
                }
            }
        }

        // Recursively check submenus
        for submenu in self.open_submenus.values() {
            if submenu.popup_surface.is_configured()
                && submenu.handle_pointer_button_recursive(handler)
            {
                return true;
            }
        }

        false
    }

    /// Request redraw with frame callback synchronization
    fn set_needs_redraw<D>(&mut self, qh: &QueueHandle<D>)
    where
        D: wayland_client::Dispatch<
                wayland_client::protocol::wl_callback::WlCallback,
                wl_surface::WlSurface,
            > + 'static,
    {
        self.needs_redraw = true;
        if self.popup_surface.is_configured() {
            if !self.frame_callback_pending {
                self.popup_surface
                    .wl_surface()
                    .frame(qh, self.popup_surface.wl_surface().clone());
                self.popup_surface.wl_surface().commit();
                self.frame_callback_pending = true;
            }
        }
    }

    /// Handle frame callback - render if dirty
    fn on_frame_callback(&mut self, style: &MenuStyle) {
        self.frame_callback_pending = false;

        if self.needs_redraw && self.popup_surface.is_configured() {
            self.render(style);
            self.needs_redraw = false;
        }
    }

    fn render(&mut self, style: &MenuStyle) {
        // Don't render if not configured yet
        if !self.popup_surface.is_configured() {
            return;
        }

        let (width, _height) = self.popup_surface.dimensions();
        let hovered_item = self.hovered_item;
        let items = &self.items;

        self.popup_surface.draw(|canvas| {
            draw_menu(canvas, items, width as f32, hovered_item, style);
        });

        // Ensure the surface is committed
        self.popup_surface.wl_surface().commit();
    }

    /// Find which item is at the given Y position
    fn item_at_position(&self, y: f32, style: &MenuStyle) -> Option<usize> {
        find_item_at_position(&self.items, y, style)
    }

    /// Recursively handle pointer leave
    /// If leaving a surface that has an open submenu, keep the highlight on the parent item
    /// Otherwise, clear the highlight
    fn handle_pointer_leave_recursive(
        &mut self,
        surface: &wl_surface::WlSurface,
        style: &MenuStyle,
    ) {
        // Check if this is our surface
        if self.popup_surface.wl_surface() == surface {
            // Check if we have any open/configured submenus
            let has_open_submenu = self
                .open_submenus
                .values()
                .any(|s| s.popup_surface.is_configured());

            if !has_open_submenu {
                // No submenus open, clear the highlight
                if self.hovered_item.is_some() {
                    self.hovered_item = None;
                    self.needs_redraw = true;
                    self.render(style);
                    self.needs_redraw = false;
                }
            }
            // If we have an open submenu, keep the highlight on the parent item
            return;
        }

        // Recursively check submenus
        for submenu in self.open_submenus.values_mut() {
            submenu.handle_pointer_leave_recursive(surface, style);
        }
    }

    fn close(&mut self) {
        self.popup_surface.close();
        for submenu in self.open_submenus.values_mut() {
            submenu.close();
        }
    }
}

/// Helper function to find which item is at a given Y position
fn find_item_at_position(items: &[MenuItem], y: f32, style: &MenuStyle) -> Option<usize> {
    let mut current_y = style.padding_vertical;
    let mut item_index = 0;

    for item in items {
        if item.is_separator() {
            current_y += style.separator_height;
        } else {
            let item_bottom = current_y + style.item_height;
            if y >= current_y && y < item_bottom {
                return Some(item_index);
            }
            current_y = item_bottom;
        }
        item_index += 1;
    }

    None
}

/// Create an XDG positioner for menu placement
fn create_positioner<D>(
    xdg_shell: &XdgShell,
    _qh: &QueueHandle<D>,
    items: &[MenuItem],
    position: &Position,
    style: &MenuStyle,
) -> Result<XdgPositioner, MenuError>
where
    D: 'static,
{
    use wayland_protocols::xdg::shell::client::xdg_positioner::{
        Anchor as WlAnchor, ConstraintAdjustment, Gravity as WlGravity,
    };

    let width = style.calculate_menu_width(items) as i32;
    let height = style.calculate_menu_height(items) as i32;

    // Create the positioner
    let positioner = XdgPositioner::new(xdg_shell).map_err(|_| MenuError::SurfaceCreationFailed)?;

    // Set size of the popup
    positioner.set_size(width, height);

    // Set anchor rectangle (1x1 point at the specified position)
    positioner.set_anchor_rect(position.x, position.y, 1, 1);

    // Convert our Anchor enum to protocol Anchor
    let anchor = match position.anchor {
        Anchor::TopLeft => WlAnchor::TopLeft,
        Anchor::Top => WlAnchor::Top,
        Anchor::TopRight => WlAnchor::TopRight,
        Anchor::Right => WlAnchor::Right,
        Anchor::BottomRight => WlAnchor::BottomRight,
        Anchor::Bottom => WlAnchor::Bottom,
        Anchor::BottomLeft => WlAnchor::BottomLeft,
        Anchor::Left => WlAnchor::Left,
    };

    // Convert our Gravity enum to protocol Gravity
    let gravity = match position.gravity {
        Gravity::TopLeft => WlGravity::TopLeft,
        Gravity::Top => WlGravity::Top,
        Gravity::TopRight => WlGravity::TopRight,
        Gravity::Right => WlGravity::Right,
        Gravity::BottomRight => WlGravity::BottomRight,
        Gravity::Bottom => WlGravity::Bottom,
        Gravity::BottomLeft => WlGravity::BottomLeft,
        Gravity::Left => WlGravity::Left,
    };

    positioner.set_anchor(anchor);
    positioner.set_gravity(gravity);

    // Allow compositor to adjust position if it would be off-screen
    positioner.set_constraint_adjustment(
        ConstraintAdjustment::FlipX
            | ConstraintAdjustment::FlipY
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY,
    );

    Ok(positioner)
}
