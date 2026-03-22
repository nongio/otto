use otto_kit::{
    protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity},
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
    _spacer_surface: Option<LayerShellSurface>,
    left: LeftPanel,
    right: RightPanel,
}

impl TopBarApp {
    pub fn new() -> Self {
        Self {
            left_surface: None,
            right_surface: None,
            _spacer_surface: None,
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

    fn apply_surface_style(surface: &LayerShellSurface, gravity: ContentsGravity) {
        let Some(style) = surface.base_surface().surface_style() else {
            tracing::debug!("surface style protocol not available");
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
        let Some(style) = surface.base_surface().surface_style() else { return };
        let Some(scene) = AppContext::surface_style_manager() else { return };
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

    fn setup_right_frame_callback(&self) {
        let Some(ref surface) = self.right_surface else { return };

        let surface_clone = surface.clone();
        let height = self.right.height;
        let mut last_target_width: f32 = 0.0;

        surface.on_frame(move || {
            let mut right = RightPanel::new();
            right.height = height;
            right.clock.tick();

            let target = right.target_width();
            if (target - last_target_width).abs() >= 1.0 {
                last_target_width = target;
                // Resize the client buffer immediately so the next draw is correct
                surface_clone.set_size(target.ceil() as u32, height as u32);
                // Animate the compositor-side layer with a spring
                Self::animate_right_size(&surface_clone, target, height);
            }
            right.width = target;

            surface_clone.draw(|canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
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
        if !events.is_empty() {
            tracing::info!("on_pointer_event: {} events", events.len());
        }
        let Some(ref right_surface) = self.right_surface else { return };
        let right_wl = right_surface.wl_surface();

        for event in events {
            let on_right = event.surface == right_wl;

            match event.kind {
                PointerEventKind::Press { button, .. } => {
                    tracing::info!(
                        "pointer press: button={button} pos=({:.0},{:.0}) on_right={on_right}",
                        event.position.0, event.position.1
                    );

                    if !on_right {
                        continue;
                    }

                    let x = event.position.0 as f32;
                    let hit = self.right.tray_item_at(x);
                    let items = crate::tray::current_items();
                    let item_name = hit.and_then(|i| items.get(i).map(|t| t.service.clone()));
                    tracing::info!(
                        "tray hit-test: x={x:.0} hit={hit:?} item={item_name:?} (total={})",
                        items.len()
                    );

                    if let Some(index) = hit {
                        match button {
                            // BTN_LEFT = 0x110 = 272
                            272 => {
                                tracing::info!("tray icon left-clicked: index={index} service={item_name:?}");
                                crate::tray::activate_item(
                                    index,
                                    x as i32,
                                    event.position.1 as i32,
                                );
                            }
                            // BTN_RIGHT = 0x111 = 273
                            273 => {
                                tracing::info!("tray icon right-clicked: index={index} service={item_name:?}");
                                crate::tray::context_menu_item(
                                    index,
                                    x as i32,
                                    event.position.1 as i32,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
