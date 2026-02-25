use otto_kit::components::button::{Button, ButtonState, IconPosition};
use otto_kit::prelude::*;
use skia_safe::Color;

struct ButtonDemoApp {
    window: Option<Window>,
}

impl App for ButtonDemoApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Button Component Demo", 1000, 800)?;
        window.set_background(Color::from_rgb(245, 245, 245));

        window.on_draw(|canvas| {
            let mut y = 50.0;
            let spacing = 70.0;

            // Title
            Label::new("Button Component Demo")
                .at(50.0, y)
                .with_style(styles::LARGE_TITLE_EMPHASIZED)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            y += 60.0;

            // Button variants
            Label::new("Button Variants:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;

            let mut x = 50.0;

            Button::new("Primary").at(x, y).primary().render(canvas);
            x += 150.0;

            Button::new("Secondary").at(x, y).secondary().render(canvas);
            x += 150.0;

            Button::new("Outline").at(x, y).outline().render(canvas);
            x += 150.0;

            Button::new("Ghost").at(x, y).ghost().render(canvas);
            x += 150.0;

            Button::new("Danger").at(x, y).danger().render(canvas);

            y += spacing;

            // Button states
            Label::new("Button States (Primary):")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            Button::new("Normal")
                .at(x, y)
                .with_state(ButtonState::Normal)
                .render(canvas);
            x += 150.0;

            Button::new("Hovered")
                .at(x, y)
                .with_state(ButtonState::Hovered)
                .render(canvas);
            x += 150.0;

            Button::new("Pressed")
                .at(x, y)
                .with_state(ButtonState::Pressed)
                .render(canvas);
            x += 150.0;

            Button::new("Disabled").at(x, y).disabled().render(canvas);

            y += spacing;

            // Buttons with icons
            Label::new("Buttons with Icons:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            Button::new("Download")
                .at(x, y)
                .with_icon("download")
                .primary()
                .render(canvas);
            x += 180.0;

            Button::new("Upload")
                .at(x, y)
                .with_icon("upload")
                .with_icon_position(IconPosition::Right)
                .secondary()
                .render(canvas);
            x += 180.0;

            Button::icon("heart").at(x, y).danger().render(canvas);
            x += 70.0;

            Button::icon("star").at(x, y).secondary().render(canvas);
            x += 70.0;

            Button::icon("settings").at(x, y).outline().render(canvas);

            y += spacing;

            // Different icon positions
            Label::new("Icon Positions:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            Button::new("Left")
                .at(x, y)
                .with_icon("check")
                .with_icon_position(IconPosition::Left)
                .primary()
                .render(canvas);
            x += 150.0;

            Button::new("Right")
                .at(x, y)
                .with_icon("chevron-right")
                .with_icon_position(IconPosition::Right)
                .primary()
                .render(canvas);
            x += 200.0;

            Button::new("Top")
                .at(x, y)
                .with_size(100.0, 80.0)
                .with_icon("star")
                .with_icon_position(IconPosition::Top)
                .secondary()
                .render(canvas);
            x += 120.0;

            Button::new("Bottom")
                .at(x, y)
                .with_size(100.0, 80.0)
                .with_icon("heart")
                .with_icon_position(IconPosition::Bottom)
                .secondary()
                .render(canvas);

            y += 100.0;

            // Custom styling
            Label::new("Custom Styling:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            Button::new("Custom Colors")
                .at(x, y)
                .with_background(Color::from_rgb(147, 51, 234)) // Purple
                .with_text_color(Color::WHITE)
                .render(canvas);
            x += 200.0;

            Button::new("Large Padding")
                .at(x, y)
                .with_padding(32.0, 16.0)
                .primary()
                .render(canvas);
            x += 220.0;

            Button::new("Sharp Corners")
                .at(x, y)
                .with_corner_radius(0.0)
                .secondary()
                .render(canvas);
            x += 200.0;

            Button::new("Very Round")
                .at(x, y)
                .with_corner_radius(20.0)
                .outline()
                .render(canvas);

            y += spacing;

            // Practical examples
            Label::new("Practical Examples:")
                .at(50.0, y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            y += 40.0;
            x = 50.0;

            Button::new("Save Changes")
                .at(x, y)
                .with_icon("save")
                .primary()
                .render(canvas);
            x += 180.0;

            Button::new("Cancel").at(x, y).ghost().render(canvas);
            x += 130.0;

            Button::new("Delete Account")
                .at(x, y)
                .with_icon("trash")
                .danger()
                .render(canvas);
            x += 200.0;

            Button::icon("search").at(x, y).outline().render(canvas);
            x += 70.0;

            Button::icon("menu").at(x, y).ghost().render(canvas);
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing button demo...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = ButtonDemoApp { window: None };
    AppRunnerWithType::new(app).run()?;
    Ok(())
}
