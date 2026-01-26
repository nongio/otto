use hello_design::prelude::*;

struct MyApp {
    window: Option<Window>,
}

impl App for MyApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new::<MyApp>("Simple Window Example", 800, 600)?;

        window.set_background(skia_safe::Color::from_rgb(255, 255, 255));

        window.on_draw(|canvas| {
            let font = styles::H1.font();
            let paint = Paint::new(skia_safe::Color4f::new(0.1, 0.1, 0.1, 1.0), None);
            canvas.draw_str("Hello, Design!", (50.0, 80.0), &font, &paint);
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
