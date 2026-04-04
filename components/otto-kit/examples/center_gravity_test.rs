//! Minimal test for ContentsGravity::Center
//!
//! Creates a layer shell surface at top-center. Uses surface style to:
//! - Set background color (dark)
//! - Set corner radius (pill shape)
//! - Set contents gravity to Center
//! - Animate set_size to test centering behavior
//!
//! The content is a simple colored rectangle with text.
//! Press 1/2/3/4 to switch sizes: small circle, compact pill, zoom pill, expanded card.

use otto_kit::{
    app_runner::AppRunner,
    protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity},
    surfaces::LayerShellSurface,
    typography::TextStyle,
    App, AppContext,
};
use skia_safe::{Color, Paint, Rect};
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
};

const LAYER_W: u32 = 480;
const LAYER_H: u32 = 120;

// Content sizes (logical)
const SIZES: [(f32, f32, &str); 4] = [
    (30.0, 30.0, "Circle"),
    (280.0, 30.0, "Compact"),
    (300.0, 36.0, "Zoom"),
    (460.0, 100.0, "Expanded"),
];

struct TestApp {
    layer_surface: Option<LayerShellSurface>,
    current_size: usize,
}

impl TestApp {
    fn draw_content(&self) {
        let Some(ref surface) = self.layer_surface else {
            return;
        };
        let (w, h, label) = SIZES[self.current_size];

        surface.draw(|canvas| {
            // Canvas is already 2x-scaled by SkiaSurface::draw()
            // Logical buffer size = LAYER_W x LAYER_H = 480x120
            // We draw centered in this logical space

            let buf_w = LAYER_W as f32;
            let buf_h = LAYER_H as f32;
            let tx = (buf_w - w) / 2.0;
            let ty = (buf_h - h) / 2.0;

            // Background: green rectangle at center position
            let mut bg = Paint::default();
            bg.set_anti_alias(true);
            bg.set_color(Color::from_rgb(0, 180, 80));
            canvas.draw_rect(Rect::from_xywh(tx, ty, w, h), &bg);

            // Red border
            let mut border = Paint::default();
            border.set_anti_alias(true);
            border.set_color(Color::RED);
            border.set_style(skia_safe::paint::Style::Stroke);
            border.set_stroke_width(2.0);
            canvas.draw_rect(
                Rect::from_xywh(tx + 1.0, ty + 1.0, w - 2.0, h - 2.0),
                &border,
            );

            // White crosshair at buffer center
            let mut cross = Paint::default();
            cross.set_color(Color::WHITE);
            cross.set_stroke_width(1.0);
            canvas.draw_line(
                (buf_w / 2.0 - 10.0, buf_h / 2.0),
                (buf_w / 2.0 + 10.0, buf_h / 2.0),
                &cross,
            );
            canvas.draw_line(
                (buf_w / 2.0, buf_h / 2.0 - 10.0),
                (buf_w / 2.0, buf_h / 2.0 + 10.0),
                &cross,
            );

            // Label text
            let font = TextStyle {
                family: "Inter",
                weight: 600,
                size: 14.0,
            }
            .font();
            let mut text = Paint::default();
            text.set_anti_alias(true);
            text.set_color(Color::WHITE);
            let msg = format!("{} ({}x{})", label, w, h);
            canvas.draw_str(&msg, (tx + 8.0, ty + h / 2.0 + 5.0), &font, &text);
        });
    }

    fn animate_to_current_size(&self) {
        let Some(ref surface) = self.layer_surface else {
            return;
        };
        let Some(style) = surface.base_surface().surface_style() else {
            return;
        };
        let Some(scene) = AppContext::surface_style_manager() else {
            return;
        };
        let qh = AppContext::queue_handle();

        let (w, h, _) = SIZES[self.current_size];
        let scale = 2.0_f64;

        let timing = scene.create_timing_function(qh, ());
        timing.set_spring(0.6, 0.0);
        let anim = scene.begin_transaction(qh, ());
        anim.set_duration(0.5);
        anim.set_timing_function(&timing);

        style.set_size(w as f64 * scale, h as f64 * scale);

        let corner_r = if self.current_size == 0 {
            w as f64 / 2.0 * scale // circle
        } else if self.current_size == 3 {
            16.0 * scale // card
        } else {
            h as f64 / 2.0 * scale // pill
        };
        style.set_corner_radius(corner_r);

        anim.commit();
    }
}

impl App for TestApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let surface = LayerShellSurface::new(Layer::Overlay, "center-test", LAYER_W, LAYER_H)?;
        surface.set_anchor(Anchor::Top);
        surface.set_margin(4, 0, 0, 0);
        surface.set_exclusive_zone(0);
        surface.set_keyboard_interactivity(
            wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        );

        if let Some(style) = surface.base_surface().surface_style() {
            style.set_background_color(0.03, 0.03, 0.03, 1.0);
            style.set_corner_radius(30.0); // initial circle
            style.set_masks_to_bounds(ClipMode::Enabled);
            style.set_shadow(0.2, 2.0, 0.0, 8.0, 0.0, 0.0, 0.0);
            style.set_blend_mode(BlendMode::BackgroundBlur);
            style.set_contents_gravity(ContentsGravity::Center);

            // Initial size: circle
            let scale = 2.0_f64;
            style.set_size(30.0 * scale, 30.0 * scale);
        }

        self.layer_surface = Some(surface);
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, _w: i32, _h: i32, _serial: u32) {
        self.draw_content();
    }

    fn on_keyboard_event(
        &mut self,
        _ctx: &AppContext,
        key: u32,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        if state != wl_keyboard::KeyState::Pressed {
            return;
        }

        // Keys: 1=circle, 2=compact, 3=zoom, 4=expanded, Q=quit
        let new_size = match key {
            2 => Some(0), // 1 key
            3 => Some(1), // 2 key
            4 => Some(2), // 3 key
            5 => Some(3), // 4 key
            16 => {
                std::process::exit(0);
            } // Q key
            _ => None,
        };

        if let Some(idx) = new_size {
            self.current_size = idx;
            self.draw_content();
            self.animate_to_current_size();
            println!("Switched to size {}: {:?}", idx, SIZES[idx]);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Center Gravity Test");
    println!("Press 1=Circle, 2=Compact, 3=Zoom, 4=Expanded, Q=Quit");

    let app = TestApp {
        layer_surface: None,
        current_size: 0,
    };

    AppRunner::new(app).run()?;
    Ok(())
}
