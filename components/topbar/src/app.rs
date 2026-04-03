use otto_kit::{
    components::context_menu::ContextMenu,
    components::menu_item::{MenuItem as KitMenuItem, MenuItemIcon},
    protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity},
    surfaces::LayerShellSurface,
    App, AppContext,
};
use smithay_client_toolkit::{
    seat::pointer::{PointerEvent, PointerEventKind},
    shell::xdg::window::WindowConfigure,
    shell::xdg::XdgPositioner,
};
use std::collections::HashMap;
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_protocols::xdg::shell::client::xdg_positioner;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

use crate::bar::{LeftPanel, RightPanel};
use crate::config::*;

/// Tracks which tray icon's context menu is currently open.
struct OpenMenu {
    /// The ContextMenu object managing the popup.
    menu: ContextMenu,
    /// Tray item index that owns this menu.
    tray_index: usize,
}

/// Tracks an open app menu popup from the left panel.
struct OpenAppMenu {
    menu: ContextMenu,
    /// Left panel menu item index (0 = app name, 1+ = menu items).
    item_index: usize,
}

pub struct TopBarApp {
    left_surface: Option<LayerShellSurface>,
    right_surface: Option<LayerShellSurface>,
    _spacer_surface: Option<LayerShellSurface>,
    left: LeftPanel,
    right: RightPanel,
    last_left_width: f32,
    last_right_width: f32,
    last_tray_gen: u64,
    last_focus_gen: u64,
    last_appmenu_gen: u64,
    /// Currently open tray context menu (only one at a time).
    open_menu: Option<OpenMenu>,
    /// Tray index awaiting an async dbusmenu fetch (keeps active highlight).
    pending_menu_index: Option<usize>,
    /// Currently open app menu popup from the left panel.
    open_app_menu: Option<OpenAppMenu>,
    /// Left panel item index awaiting an async submenu fetch.
    pending_app_menu_index: Option<usize>,
}

impl TopBarApp {
    pub fn new() -> Self {
        Self {
            left_surface: None,
            right_surface: None,
            _spacer_surface: None,
            left: LeftPanel::new(),
            right: RightPanel::new(),
            last_left_width: 0.0,
            last_right_width: 0.0,
            last_tray_gen: 0,
            last_focus_gen: 0,
            last_appmenu_gen: 0,
            open_menu: None,
            pending_menu_index: None,
            open_app_menu: None,
            pending_app_menu_index: None,
        }
    }

    fn redraw_left(&self) {
        let Some(ref surface) = self.left_surface else {
            return;
        };
        let left = &self.left;
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            left.draw(canvas);
        });
        surface.base_surface().wl_surface().commit();
    }

    fn redraw_right(&self) {
        let Some(ref surface) = self.right_surface else {
            return;
        };
        let right = &self.right;
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            right.draw(canvas);
        });
        surface.base_surface().wl_surface().commit();
    }

    fn apply_surface_style(surface: &LayerShellSurface, gravity: ContentsGravity) {
        let Some(style) = surface.base_surface().surface_style() else {
            return;
        };

        let theme = AppContext::current_theme();
        let c = skia_safe::Color4f::from(theme.material_medium);
        style.set_background_color(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
        style.set_blend_mode(BlendMode::BackgroundBlur);
        style.set_masks_to_bounds(ClipMode::Enabled);
        style.set_corner_radius(BAR_CORNER_RADIUS as f64);
        style.set_shadow(0.25, 8.0, 0.0, 3.0, 0.0, 0.0, 0.0);
        style.set_contents_gravity(gravity);
    }

    fn animate_right_size(surface: &LayerShellSurface, width: f32, height: f32) {
        let Some(style) = surface.base_surface().surface_style() else {
            return;
        };
        let Some(scene) = AppContext::surface_style_manager() else {
            return;
        };
        let qh = AppContext::queue_handle();

        let timing = scene.create_timing_function(qh, ());
        timing.set_spring(0.5, 0.7);
        let txn = scene.begin_transaction(qh, ());
        txn.set_duration(0.5);
        txn.set_timing_function(&timing);

        let scale = 2.0_f64;
        style.set_size(width as f64 * scale, height as f64 * scale);

        txn.commit();
    }

    fn update_left_panel(&mut self, animate: bool) {
        let Some(ref surface) = self.left_surface else {
            return;
        };

        let target = self.left.target_width();
        if (target - self.last_left_width).abs() >= 1.0 {
            self.last_left_width = target;
            surface.set_size(target.ceil() as u32, self.left.height as u32);
            if animate {
                Self::animate_right_size(surface, target, self.left.height);
            }
        }
        self.left.width = target;
        self.redraw_left();
    }

    fn update_right_panel(&mut self, animate: bool) {
        let Some(ref surface) = self.right_surface else {
            return;
        };

        let target = self.right.target_width();
        if (target - self.last_right_width).abs() >= 1.0 {
            self.last_right_width = target;
            surface.set_size(target.ceil() as u32, self.right.height as u32);
            if animate {
                Self::animate_right_size(surface, target, self.right.height);
            }
        }
        self.right.width = target;
        self.redraw_right();
    }

    /// Close any open context menu.
    fn close_menu(&mut self) {
        if let Some(open) = self.open_menu.take() {
            open.menu.hide_animated();
        }
        self.pending_menu_index = None;
        self.right.tray_menu_state.set_active(None);
        // Release keyboard focus
        if let Some(ref surface) = self.right_surface {
            surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        }
        self.redraw_right();
    }

    /// Close any open app menu popup from the left panel.
    fn close_app_menu(&mut self) {
        if let Some(open) = self.open_app_menu.take() {
            open.menu.hide_animated();
        }
        self.pending_app_menu_index = None;
        self.left.menu_state.set_active(None);
        // Release keyboard focus
        if let Some(ref surface) = self.left_surface {
            surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        }
        self.redraw_left();
    }

    /// Show a context menu from a pending dbusmenu fetch.
    fn show_pending_menu(&mut self, pending: crate::tray::PendingMenu, tray_index: usize) {
        let Some(ref surface) = self.right_surface else {
            return;
        };

        // Convert dbusmenu items to otto-kit MenuItems
        let kit_items = convert_dbusmenu_items(&pending.layout.items);
        if kit_items.is_empty() {
            return;
        }

        // Create the ContextMenu
        let svc = pending.service.clone();
        let mp = pending.menu_path.clone();

        // Build id→label map for resolving stale IDs at activation time
        let id_labels = build_id_label_map(&pending.layout.items);

        let menu = ContextMenu::new(kit_items).on_item_click(move |action_id| {
            if let Ok(id) = action_id.parse::<i32>() {
                let label = id_labels.get(&id).cloned().unwrap_or_default();
                crate::tray::activate_menu_item(&svc, &mp, id, &label);
            }
        });

        // Create positioner: anchor below the tray icon, right-aligned
        if let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) {
            // Measure menu dimensions
            let style = otto_kit::components::context_menu::ContextMenuStyle::default();
            let state = menu.state();
            let items = state.borrow().items_at_depth(0).to_vec();
            let (menu_w, menu_h) =
                otto_kit::components::context_menu::ContextMenuRenderer::measure_items(
                    &items, &style,
                );

            positioner.set_size(menu_w as i32, menu_h as i32);

            // Anchor rect: the tray icon bounding box in the right panel
            if let Some((ix, iy, iw, ih)) = self.right.tray_item_rect(tray_index) {
                positioner.set_anchor_rect(ix as i32, iy as i32, iw as i32, ih as i32);
            } else {
                // Fallback: thin rect at pointer X, full bar height
                positioner.set_anchor_rect(pending.anchor_x, 0, 1, self.right.height as i32);
            }
            // Popup top-right corner aligns to icon bottom-right corner
            positioner.set_anchor(xdg_positioner::Anchor::BottomRight);
            positioner.set_gravity(xdg_positioner::Gravity::BottomLeft);
            positioner.set_offset(0, 1); // Small gap below the bar
            positioner.set_constraint_adjustment(
                xdg_positioner::ConstraintAdjustment::SlideX
                    | xdg_positioner::ConstraintAdjustment::SlideY
                    | xdg_positioner::ConstraintAdjustment::FlipX
                    | xdg_positioner::ConstraintAdjustment::FlipY,
            );

            menu.show_for_layer(&surface.layer_surface(), &positioner);

            // Grab keyboard focus on the layer surface for arrow-key navigation
            surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

            self.open_menu = Some(OpenMenu { menu, tray_index });
        }
    }

    /// Show a submenu popup from the left panel for an app menu item.
    fn show_app_submenu(&mut self, pending: crate::appmenu::PendingSubmenu) {
        let Some(ref surface) = self.left_surface else {
            return;
        };

        // The item_index in pending is relative to the visible menu items
        // (0-based among non-empty visible items). We need to find the
        // children of that top-level item.
        let visible_items: Vec<_> = pending
            .layout
            .items
            .iter()
            .filter(|i| i.visible && !i.label.is_empty())
            .collect();

        let Some(top_item) = visible_items.get(pending.item_index) else {
            return;
        };

        let kit_items = convert_dbusmenu_items(&top_item.children);
        if kit_items.is_empty() {
            return;
        }

        let id_labels = build_id_label_map(&top_item.children);

        let menu = ContextMenu::new(kit_items).on_item_click(move |action_id| {
            if let Ok(id) = action_id.parse::<i32>() {
                let label = id_labels.get(&id).cloned().unwrap_or_default();
                crate::appmenu::activate_menu_item(id, &label);
            }
        });

        if let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state()) {
            let style = otto_kit::components::context_menu::ContextMenuStyle::default();
            let state = menu.state();
            let items = state.borrow().items_at_depth(0).to_vec();
            let (menu_w, menu_h) =
                otto_kit::components::context_menu::ContextMenuRenderer::measure_items(
                    &items, &style,
                );

            positioner.set_size(menu_w as i32, menu_h as i32);
            positioner.set_anchor_rect(pending.anchor_x, self.left.height as i32, 1, 1);
            positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
            positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
            positioner.set_offset(0, 1); // Small gap below the bar
            positioner.set_constraint_adjustment(
                xdg_positioner::ConstraintAdjustment::SlideX
                    | xdg_positioner::ConstraintAdjustment::SlideY
                    | xdg_positioner::ConstraintAdjustment::FlipX
                    | xdg_positioner::ConstraintAdjustment::FlipY,
            );

            let item_index = pending.item_index;
            menu.show_for_layer(&surface.layer_surface(), &positioner);

            // Grab keyboard focus on the layer surface for arrow-key navigation
            surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

            self.open_app_menu = Some(OpenAppMenu { menu, item_index });
        }
    }

    /// Handle a click on the right panel (tray icons).
    fn handle_right_click(&mut self, event: &PointerEvent) {
        let x = event.position.0 as f32;
        let hit = self.right.tray_item_at(x);

        if let Some(index) = hit {
            let already_open = self
                .open_menu
                .as_ref()
                .map(|m| m.tray_index == index)
                .unwrap_or(false);

            if already_open {
                self.close_menu();
            } else {
                self.close_menu();
                self.right.tray_menu_state.set_active(Some(index));
                self.pending_menu_index = Some(index);
                self.redraw_right();
                crate::tray::context_menu_item(index, x as i32, event.position.1 as i32);
            }
        } else {
            self.close_menu();
        }
    }

    /// Handle a click on the left panel (app menu items).
    fn handle_left_click(&mut self, event: &PointerEvent) {
        let x = event.position.0 as f32;
        let hit = self.left.menu_item_at(x);
        let Some(index) = hit else {
            self.close_app_menu();
            return;
        };

        // Index 0 is the app name — skip it (or could open "about" in future)
        if index == 0 {
            self.close_app_menu();
            return;
        }

        // Menu item indices are 1-based in the left panel MenuBar;
        // the dbusmenu top-level item index is (index - 1).
        let menu_index = index - 1;

        let already_open = self
            .open_app_menu
            .as_ref()
            .map(|m| m.item_index == menu_index)
            .unwrap_or(false);

        if already_open {
            self.close_app_menu();
        } else {
            self.close_app_menu();
            self.left.menu_state.set_active(Some(index));
            self.pending_app_menu_index = Some(menu_index);
            self.redraw_left();

            // Compute anchor_x for the popup position
            let anchor_x = self.left.item_anchor_x(index);
            crate::appmenu::fetch_submenu_for_item(menu_index, anchor_x as i32);
        }
    }
}

impl App for TopBarApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        // Invisible spacer spanning the full top edge — its only job is to
        // reserve exclusive space so maximized windows are pushed down.
        // We cannot use the left or right panel for this because they are
        // corner-anchored, and Smithay applies exclusive zones from both
        // edges of a corner anchor.
        let spacer = LayerShellSurface::with_anchor(
            Layer::Top,
            "otto-topbar-spacer",
            0, // fill width
            1, // minimal height (transparent)
            Some(Anchor::Top | Anchor::Left | Anchor::Right),
            Some(BAR_HEIGHT as i32 + BAR_MARGIN_TOP),
        )?;
        spacer.set_keyboard_interactivity(KeyboardInteractivity::None);

        // Left panel: app name + menus, anchored top-left (no exclusive zone)
        let left = LayerShellSurface::with_anchor(
            Layer::Top,
            "otto-topbar-left",
            LEFT_WIDTH,
            BAR_HEIGHT,
            Some(Anchor::Top | Anchor::Left),
            None,
        )?;
        left.set_margin(BAR_MARGIN_TOP, 0, 0, BAR_MARGIN_SIDE);
        left.set_keyboard_interactivity(KeyboardInteractivity::None);
        Self::apply_surface_style(&left, ContentsGravity::TopLeft); // TopLeft

        // Right panel: tray + clock, anchored top-right (no exclusive zone)
        let right = LayerShellSurface::with_anchor(
            Layer::Top,
            "otto-topbar-right",
            RIGHT_WIDTH,
            BAR_HEIGHT,
            Some(Anchor::Top | Anchor::Right),
            None,
        )?;
        right.set_margin(BAR_MARGIN_TOP, BAR_MARGIN_SIDE, 0, 0);
        right.set_keyboard_interactivity(KeyboardInteractivity::None);
        Self::apply_surface_style(&right, ContentsGravity::TopRight); // TopRight

        self.left_surface = Some(left);
        self.right_surface = Some(right);
        self._spacer_surface = Some(spacer);

        crate::tray::spawn_tray_watcher();
        crate::focus::spawn_focus_watcher();
        crate::appmenu::spawn_appmenu_registrar();

        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, _width: i32, _height: i32, _serial: u32) {
        // Configure fires for each layer surface (spacer, left, right).
        // We use fixed dimensions, so just redraw on any configure.
        self.right.clock.tick();
        self.update_left_panel(false);
        self.update_right_panel(false);
    }

    fn on_configure(&mut self, _ctx: &AppContext, _configure: WindowConfigure, _serial: u32) {}

    fn on_keyboard_event(
        &mut self,
        _ctx: &AppContext,
        key: u32,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        // Forward to open tray context menu
        if let Some(ref mut open) = self.open_menu {
            open.menu.handle_key(key, state);
            if !open.menu.is_visible() {
                self.close_menu();
            }
            return;
        }

        // Forward to open app menu
        if let Some(ref mut open) = self.open_app_menu {
            open.menu.handle_key(key, state);
            if !open.menu.is_visible() {
                self.close_app_menu();
            }
        }
    }

    fn on_keyboard_leave(&mut self, _ctx: &AppContext, surface: &wl_surface::WlSurface) {
        // Only close menus when focus leaves one of our layer surfaces.
        // Submenu popups are created without a keyboard grab, so they never
        // steal focus and this callback only fires when the user truly
        // leaves the topbar (e.g. clicks on another window).
        let is_our_layer = self
            .right_surface
            .as_ref()
            .map(|s| s.wl_surface() == *surface)
            .unwrap_or(false)
            || self
                .left_surface
                .as_ref()
                .map(|s| s.wl_surface() == *surface)
                .unwrap_or(false);

        if !is_our_layer {
            return;
        }

        if self.open_menu.is_some() {
            self.close_menu();
        }
        if self.open_app_menu.is_some() {
            self.close_app_menu();
        }
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let mut dirty = false;

        // Check if clock text changed
        if self.right.clock.tick() {
            dirty = true;
        }

        // Check if tray items changed (also bumped when pending menu arrives)
        let gen = crate::tray::generation();
        if gen != self.last_tray_gen {
            self.last_tray_gen = gen;
            self.right.sync_tray_items();
            dirty = true;

            // Check for a pending context menu to display
            if let Some(pending) = crate::tray::take_pending_menu() {
                // Find which tray index this menu belongs to
                let items = crate::tray::current_items();
                let tray_index = items
                    .iter()
                    .position(|t| t.service == pending.service)
                    .unwrap_or(0);

                self.pending_menu_index = None;
                // Close any existing menu first
                self.close_menu();
                // Keep active highlight for the menu owner
                self.right.tray_menu_state.set_active(Some(tray_index));
                // Use stored input serial for popup keyboard grab
                self.show_pending_menu(pending, tray_index);
            }
        }

        if dirty {
            self.update_right_panel(true);
        }

        // Check if focused app changed
        let focus_gen = crate::focus::generation();
        if focus_gen != self.last_focus_gen {
            self.last_focus_gen = focus_gen;
            let focused = crate::focus::current_focused_app();
            let name = otto_kit::desktop_entry::display_name_for_app(&focused.app_id);
            // Close any open app menu when focus changes
            self.close_app_menu();
            self.left.set_app_name(&name);
            self.update_left_panel(true);

            // Request the app menu for the newly focused app
            crate::appmenu::request_menu_for_app(&focused.app_id, 0);
        }

        // Check if app menu was fetched
        let appmenu_gen = crate::appmenu::generation();
        if appmenu_gen != self.last_appmenu_gen {
            self.last_appmenu_gen = appmenu_gen;

            // Check for a pending submenu popup
            if let Some(pending) = crate::appmenu::take_pending_submenu() {
                self.pending_app_menu_index = None;
                self.close_app_menu();
                // item_index is 0-based among visible menu items;
                // in the left panel, index 0 = app name, so menu item N is at N+1
                self.left
                    .menu_state
                    .set_active(Some(pending.item_index + 1));
                self.show_app_submenu(pending);
            } else {
                // Menu layout itself changed — update left panel items
                self.left
                    .set_app_menu(crate::appmenu::current_menu().as_ref());
                self.update_left_panel(true);
            }
        }

        // Clear active highlight if no menu is open and none is pending
        if self.open_menu.is_none()
            && self.pending_menu_index.is_none()
            && self.right.tray_menu_state.active_index().is_some()
        {
            self.right.tray_menu_state.set_active(None);
            self.redraw_right();
        }

        // Clear left panel active highlight if no app menu popup is open
        if self.open_app_menu.is_none()
            && self.pending_app_menu_index.is_none()
            && self.left.menu_state.active_index().is_some()
        {
            self.left.menu_state.set_active(None);
            self.redraw_left();
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        // Wake once per second to update the clock.
        Some(std::time::Duration::from_secs(1))
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        let right_wl = self.right_surface.as_ref().map(|s| s.wl_surface().clone());
        let left_wl = self.left_surface.as_ref().map(|s| s.wl_surface().clone());

        for event in events {
            let on_right = right_wl
                .as_ref()
                .map(|w| event.surface == *w)
                .unwrap_or(false);
            let on_left = left_wl
                .as_ref()
                .map(|w| event.surface == *w)
                .unwrap_or(false);

            if let PointerEventKind::Press { .. } = event.kind {
                if on_right {
                    self.handle_right_click(event);
                } else if on_left {
                    self.handle_left_click(event);
                }
            }
        }
    }
}

/// Build a map from dbusmenu item id → label (raw, with mnemonics).
/// Used to resolve stale IDs when activating items.
fn build_id_label_map(items: &[crate::dbusmenu::MenuItem]) -> HashMap<i32, String> {
    let mut map = HashMap::new();
    collect_id_labels(items, &mut map);
    map
}

fn collect_id_labels(items: &[crate::dbusmenu::MenuItem], map: &mut HashMap<i32, String>) {
    for item in items {
        if !item.label.is_empty() {
            map.insert(item.id, item.label.clone());
        }
        collect_id_labels(&item.children, map);
    }
}
fn convert_dbusmenu_items(items: &[crate::dbusmenu::MenuItem]) -> Vec<KitMenuItem> {
    items
        .iter()
        .filter(|item| item.visible)
        .map(|item| {
            if item.item_type == crate::dbusmenu::MenuItemType::Separator {
                return KitMenuItem::separator();
            }

            let label = item.label.replace('_', ""); // strip mnemonics

            // Resolve icon: prefer named XDG icon; fall back to raw pixmap
            let icon = item
                .icon_name
                .as_deref()
                .filter(|n| !n.is_empty())
                .map(|n| MenuItemIcon::Named(n.to_string()))
                .or_else(|| {
                    item.icon_data
                        .as_ref()
                        .map(|(w, h, data)| MenuItemIcon::Pixmap {
                            data: data.clone(),
                            width: *w,
                            height: *h,
                        })
                });

            if !item.children.is_empty() {
                let children = convert_dbusmenu_items(&item.children);
                let mut kit = KitMenuItem::submenu(label, children);
                if let Some(icon) = icon {
                    kit = kit.with_icon(icon);
                }
                if !item.enabled {
                    kit = kit.disabled();
                }
                kit
            } else {
                let mut kit = KitMenuItem::action(&label).with_action_id(item.id.to_string());
                if let Some(icon) = icon {
                    kit = kit.with_icon(icon);
                }
                if !item.enabled {
                    kit = kit.disabled();
                }
                kit
            }
        })
        .collect()
}
