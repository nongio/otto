/// Example demonstrating layer styling on a window
///
/// Shows how to use the direct layer API to customize window appearance
use hello_design::prelude::*;

struct MyApp {
    window: Option<Window>,
}

impl App for MyApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new::<MyApp>("Simple Menu Example", 800, 600)?;

        window.set_background(skia_safe::Color::from_argb(50, 180, 180, 180));

        window.on_draw(|canvas| {
            use skia_safe::{Color4f, Paint, Rect};
            let paint = Paint::new(Color4f::new(0.3, 0.5, 0.8, 1.0), None);
            let rect = Rect::from_xywh(50.0, 50.0, 200.0, 150.0);
            canvas.draw_rect(rect, &paint);
        });

        // Direct layer access
        if let Some(layer) = window.layer() {
            layer.set_opacity(1.0);
            layer.set_background_color(0.9, 0.9, 0.95, 0.9);
            layer.set_corner_radius(48.0);
            layer.set_masks_to_bounds(1);
        }

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(MyApp { window: None }).run()
}
