use otto_kit::{
    protocols::otto_surface_style_v1::{BlendMode, ClipMode},
    surfaces::LayerShellSurface,
    App, AppContext,
};
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

use crate::bar::Bar;
use crate::config::*;

pub struct TopBarApp {
    surface: Option<LayerShellSurface>,
    bar: Bar,
}

impl TopBarApp {
    pub fn new() -> Self {
        Self {
            surface: None,
            bar: Bar::new(),
        }
    }

    fn redraw(&self) {
        let Some(ref surface) = self.surface else {
            return;
        };
        let bar = &self.bar;

        surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            bar.draw(canvas);
        });

        surface.base_surface().wl_surface().commit();
    }

    /// Apply compositor-side visual properties via the surface style protocol.
    fn apply_surface_style(surface: &LayerShellSurface) {
        let Some(style) = surface.base_surface().surface_style() else {
            tracing::debug!("surface style protocol not available");
            return;
        };

        style.set_blend_mode(BlendMode::BackgroundBlur);
        style.set_masks_to_bounds(ClipMode::Enabled);
        style.set_corner_radius(BAR_CORNER_RADIUS as f64);
        // set_shadow(opacity, radius, offset_x, offset_y, red, green, blue)
        style.set_shadow(0.15, 6.0, 0.0, 2.0, 0.0, 0.0, 0.0);
    }
}

impl App for TopBarApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!("creating topbar layer surface");

        // Fixed width, anchored top-center with margin
        let surface = LayerShellSurface::with_anchor(
            Layer::Top,
            "otto-topbar",
            BAR_WIDTH,
            BAR_HEIGHT,
            Some(Anchor::Top | Anchor::Right),
            Some(BAR_HEIGHT as i32 + BAR_MARGIN_TOP),
        )?;

        surface.set_margin(BAR_MARGIN_TOP, BAR_MARGIN_TOP, 0, 0);
        surface.set_keyboard_interactivity(KeyboardInteractivity::None);

        Self::apply_surface_style(&surface);

        self.surface = Some(surface);
        tracing::info!("topbar ready, waiting for configure");
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        tracing::debug!("configure: {width}x{height}");

        if width > 0 {
            self.bar.width = width as f32;
        }
        if height > 0 {
            self.bar.height = height as f32;
        }

        self.bar.clock.tick();
        self.redraw();

        // Request frame callback to keep the clock ticking.
        if let Some(ref surface) = self.surface {
            let surface_clone = surface.clone();
            let bar_width = self.bar.width;
            let bar_height = self.bar.height;

            surface.on_frame(move || {
                surface_clone.draw(|canvas| {
                    canvas.clear(skia_safe::Color::TRANSPARENT);

                    let mut bar = Bar::new();
                    bar.width = bar_width;
                    bar.height = bar_height;
                    bar.clock.tick();
                    bar.draw(canvas);
                });

                surface_clone.base_surface().wl_surface().commit();
                AppContext::request_frame(&surface_clone.wl_surface());
            });

            AppContext::request_frame(&surface.wl_surface());
        }
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
}
