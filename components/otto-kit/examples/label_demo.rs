use otto_kit::prelude::*;
use skia_safe::Color;

struct LabelDemoApp {
    window: Option<Window>,
}

impl App for LabelDemoApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Label Component Demo", 800, 600)?;
        window.set_background(Color::from_rgb(245, 245, 247));

        window.on_draw(|canvas| {
            // Title
            Label::new("Label Component Demo")
                .at(50.0, 50.0)
                .with_style(styles::LARGE_TITLE_EMPHASIZED)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            // Text style examples
            let styles_demo = [
                ("Large Title", styles::LARGE_TITLE),
                ("Title 1", styles::TITLE_1),
                ("Title 2", styles::TITLE_2),
                ("Title 3", styles::TITLE_3),
                ("Headline", styles::HEADLINE),
                ("Body", styles::BODY),
                ("Callout", styles::CALLOUT),
                ("Subheadline", styles::SUBHEADLINE),
                ("Footnote", styles::FOOTNOTE),
                ("Caption 1", styles::CAPTION_1),
            ];

            for (i, (text, style)) in styles_demo.iter().enumerate() {
                Label::new(*text)
                    .at(50.0, 120.0 + i as f32 * 40.0)
                    .with_style(*style)
                    .with_color(Color::from_rgb(60, 60, 60))
                    .render(canvas);
            }

            // Alignment examples
            Label::new("Text Alignment")
                .at(450.0, 120.0)
                .with_style(styles::TITLE_2_EMPHASIZED)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            Label::new("Left Aligned")
                .at(450.0, 170.0)
                .with_width(250.0)
                .with_align(TextAlign::Left)
                .with_color(Color::from_rgb(60, 60, 60))
                .render(canvas);

            Label::new("Center Aligned")
                .at(450.0, 200.0)
                .with_width(250.0)
                .with_align(TextAlign::Center)
                .with_color(Color::from_rgb(60, 60, 60))
                .render(canvas);

            Label::new("Right Aligned")
                .at(450.0, 230.0)
                .with_width(250.0)
                .with_align(TextAlign::Right)
                .with_color(Color::from_rgb(60, 60, 60))
                .render(canvas);

            // Colored labels
            Label::new("Colored Labels")
                .at(450.0, 300.0)
                .with_style(styles::TITLE_2_EMPHASIZED)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            Label::new("Red Label")
                .at(450.0, 340.0)
                .with_style(styles::HEADLINE)
                .with_color(Color::from_rgb(255, 59, 48))
                .render(canvas);

            Label::new("Blue Label")
                .at(450.0, 370.0)
                .with_style(styles::HEADLINE)
                .with_color(Color::from_rgb(0, 122, 255))
                .render(canvas);

            Label::new("Green Label")
                .at(450.0, 400.0)
                .with_style(styles::HEADLINE)
                .with_color(Color::from_rgb(52, 199, 89))
                .render(canvas);
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing label demo...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = LabelDemoApp { window: None };
    AppRunnerWithType::new(app).run()?;
    Ok(())
}
