use otto_kit::{
    protocols::otto_surface_style_v1::{BlendMode, ClipMode},
    surfaces::LayerShellSurface,
    App, AppContext,
};
use smithay_client_toolkit::{
    seat::pointer::{PointerEvent, PointerEventKind},
    shell::xdg::window::WindowConfigure,
};
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

use crate::bar::{LeftPanel, RightPanel};
use crate::config::*;

pub struct TopBarApp {
    left_surface: Option<LayerShellSurface>,
    right_surface: Option<LayerShellSurface>,
    left: LeftPanel,
    right: RightPanel,
}

impl TopBarApp {
    pub fn new() -> Self {
        Self {
            left_surface: None,
            right_surface: None,
            left: LeftPanel::new(),
            right: RightPanel::new(),
        }
    }

    fn redraw_left(&self) {
        let Some(ref surface) = self.left_surface else { return };
        let left = &self.left;
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            left.draw(canvas);
        });
        surface.base_surface().wl_surface().commit();
    }

    fn redraw_right(&self) {
        let Some(ref surface) = self.right_surface else { return };
        let right = &self.right;
        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            right.draw(canvas);
        });
        surface.base_surface().wl_surface().commit();
    }

    fn apply_surface_style(surface: &LayerShellSurface) {
        let Some(style) = surface.base_surface().surface_style() else {
            tracing::debug!("surface style protocol not available");
            return;
        };

        style.set_blend_mode(BlendMode::BackgroundBlur);
        style.set_masks_to_bounds(ClipMode::Enabled);
        style.set_corner_radius(BAR_CORNER_RADIUS as f64);
        style.set_shadow(0.15, 6.0, 0.0, 2.0, 0.0, 0.0, 0.0);
    }

    fn setup_right_frame_callback(&self) {
        let Some(ref surface) = self.right_surface else { return };

        let surface_clone = surface.clone();
        let width = self.right.width;
        let height = self.right.height;

        surface.on_frame(move || {
            surface_clone.draw(|canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
                let mut right = RightPanel::new();
                right.width = width;
                right.height = height;
                right.clock.tick();
                right.draw(canvas);
            });

            surface_clone.base_surface().wl_surface().commit();
            AppContext::request_frame(&surface_clone.wl_surface());
        });

        AppContext::request_frame(&surface.wl_surface());
    }
}

impl App for TopBarApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("creating topbar surfaces");

        // Left panel: app name + menus, anchored top-left
        let left = LayerShellSurface::with_anchor(
            Layer::Top,
            "otto-topbar-left",
            LEFT_WIDTH,
            BAR_HEIGHT,
            Some(Anchor::Top | Anchor::Left),
            Some(BAR_HEIGHT as i32 + BAR_MARGIN_TOP),
        )?;
        left.set_margin(BAR_MARGIN_TOP, 0, 0, BAR_MARGIN_SIDE);
        left.set_keyboard_interactivity(KeyboardInteractivity::None);
        Self::apply_surface_style(&left);

        // Right panel: tray + clock, anchored top-right
        let right = LayerShellSurface::with_anchor(
            Layer::Top,
            "otto-topbar-right",
            RIGHT_WIDTH,
            BAR_HEIGHT,
            Some(Anchor::Top | Anchor::Right),
            Some(BAR_HEIGHT as i32 + BAR_MARGIN_TOP),
        )?;
        right.set_margin(BAR_MARGIN_TOP, BAR_MARGIN_SIDE, 0, 0);
        right.set_keyboard_interactivity(KeyboardInteractivity::None);
        Self::apply_surface_style(&right);

        self.left_surface = Some(left);
        self.right_surface = Some(right);

        crate::tray::spawn_tray_watcher();

        tracing::info!("topbar ready, waiting for configure");
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        tracing::debug!("configure: {width}x{height}");

        // Both panels share the same height
        if height > 0 {
            self.left.height = height as f32;
            self.right.height = height as f32;
        }

        // Update widths from configure if provided
        if width > 0 {
            // We get called for each surface; just use the fixed widths
        }

        self.right.clock.tick();
        self.redraw_left();
        self.redraw_right();
        self.setup_right_frame_callback();
    }

    fn on_configure(&mut self, _ctx: &AppContext, _configure: WindowConfigure, _serial: u32) {}

    fn on_keyboard_event(
        &mut self,
        _ctx: &AppContext,
        _key: u32,
        _state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
    }

    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {}

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        let Some(ref right_surface) = self.right_surface else { return };
        let right_wl = right_surface.wl_surface();

        for event in events {
            // Only handle clicks on the right panel
            if event.surface != right_wl {
                continue;
            }

            // BTN_LEFT = 0x110 = 272
            if let PointerEventKind::Press { button: 272, .. } = event.kind {
                let x = event.position.0 as f32;
                if let Some(index) = self.right.tray_item_at(x) {
                    tracing::info!("tray icon left-clicked: index={index}");
                    crate::tray::activate_item(index, x as i32, event.position.1 as i32);
                }
            }

            // BTN_RIGHT = 0x111 = 273
            if let PointerEventKind::Press { button: 273, .. } = event.kind {
                let x = event.position.0 as f32;
                if let Some(index) = self.right.tray_item_at(x) {
                    tracing::info!("tray icon right-clicked: index={index}");
                    crate::tray::context_menu_item(index, x as i32, event.position.1 as i32);
                }
            }
        }
    }
}
