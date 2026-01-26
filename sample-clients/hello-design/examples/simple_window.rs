use hello_design::{components::window::SimpleWindow, prelude::*};

struct MyApp {
    window: Option<Window>,
}

impl App for MyApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new::<MyApp>("Simple Window Example", 800, 600)?;

        window.set_background(skia_safe::Color::from_rgb(200, 200, 200));

        window.on_draw(|canvas| {
            let paint = skia_safe::Paint::new(skia_safe::Color4f::new(0.3, 0.5, 0.8, 1.0), None);
            let rect = skia_safe::Rect::from_xywh(50.0, 50.0, 200.0, 150.0);
            canvas.draw_rect(rect, &paint);
            let paint = skia_safe::Paint::new(skia_safe::Color4f::new(0.0, 0.0, 0.0, 1.0), None);
            canvas.draw_str(
                "Hello, Design!",
                (100, 100),
                &skia_safe::Font::default(),
                &paint,
            );
        });

        self.window = Some(window);

        Ok(())
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = MyApp { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
