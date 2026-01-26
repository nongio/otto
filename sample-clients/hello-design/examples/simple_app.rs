use hello_design::{components::menu::sc_layer_v1, prelude::*};

/// Your application struct - define your app state here
struct MyApp {
    window: Option<Window>,
}

/// Implement the App trait to make your struct runnable with AppRunner
///
/// Required methods:
///   - fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>>
///   - fn on_close(&mut self) -> bool
impl App for MyApp {
    /// Called when the app is ready and the window has been created
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("App is ready!");

        // Create a window - super simple API!
        // Window gets everything it needs from AppContext automatically
        let mut window = Window::new::<MyApp>("Simple Window Example", 800, 600)?;

        // Customize the window
        window.set_background(skia_safe::Color::from_argb(50, 180, 180, 180));

        // Add some custom content
        window.on_draw(|canvas| {
            // Draw a simple shape
            let paint = skia_safe::Paint::new(skia_safe::Color4f::new(0.3, 0.5, 0.8, 1.0), None);
            let rect = skia_safe::Rect::from_xywh(50.0, 50.0, 200.0, 150.0);
            canvas.draw_rect(rect, &paint);
        });

        // Customize the layer appearance directly (no closure!)
        if let Some(layer) = window.layer() {
            layer.set_opacity(1.0);
            layer.set_background_color(0.9, 0.9, 0.95, 0.9);
            layer.set_corner_radius(48.0); // Larger radius than default
            layer.set_masks_to_bounds(1);
            layer.set_blend_mode(sc_layer_v1::BlendMode::BackgroundBlur);
            layer.set_border(1.0, 0.9, 0.9, 0.9, 0.9);
        }

        self.window = Some(window);

        Ok(())
    }

    /// Called when the user requests to close the window
    fn on_close(&mut self) -> bool {
        println!("App is closing...");
        true // Return false to prevent closing
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create your app instance
    let app = MyApp { window: None };

    // Run the app - this handles all the Wayland/window setup
    AppRunner::new(app).run()?;

    Ok(())
}
