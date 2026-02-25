use otto_kit::components::button::Button;
use otto_kit::components::titlebar::{Titlebar, TitlebarGroup};
use otto_kit::prelude::*;
use smithay_client_toolkit::seat::pointer::PointerEventKind;

struct WindowWithTitlebarApp {
    window: Option<Window>,
}

impl App for WindowWithTitlebarApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let theme = Theme::light();
        let mut window = Window::new("Window with Titlebar", 800, 600)?;
        window.set_background(Color::WHITE);

        // Set window corner radius to 20
        if let Some(layer) = window.surface_style() {
            layer.set_corner_radius(12.0);
        }

        // Make the titlebar draggable
        let window_clone = window.clone();
        window.on_pointer_event(move |events| {
            for event in events {
                // Check if this is a press event in the titlebar region
                if let PointerEventKind::Press {
                    serial, button: _, ..
                } = event.kind
                {
                    // Click in titlebar area (top 28px), but avoid window controls area on the right
                    if event.position.1 < 28.0 && event.position.0 < 680.0 {
                        if let Some(seat) = AppContext::seat_state().seats().next() {
                            println!("Starting window drag from titlebar...");
                            window_clone.start_move(&seat, serial);
                        }
                    }
                }
            }
        });

        let title = window.title();
        window.on_draw(move |canvas| {
            let window_width = 800.0;
            // Create window controls group
            let controls = TitlebarGroup::new()
                .add(Button::icon("square-x").ghost().with_corner_radius(0.0)) // Close
                .add(Button::icon("minus").ghost()) // Minimize
                // .add(Button::icon("maximize-2").ghost().with_corner_radius(0.0)) // Maximize
                .build();

            // Create titlebar with centered title and controls
            Titlebar::new()
                .at(0.0, 0.0)
                .with_width(window_width)
                .with_height(28.0)
                .with_background(theme.material_titlebar)
                .with_border_bottom(theme.fill_secondary)
                .with_title(
                    Label::new(&title)
                        .with_style(styles::HEADLINE)
                        .with_color(theme.text_primary),
                )
                .with_controls(controls)
                .render(canvas);

            // Draw some content below to show the window area
            canvas.save();
            canvas.translate((24.0, 64.0));
            Label::new("Content area")
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .render(canvas);
            canvas.restore();

            canvas.save();
            canvas.translate((24.0, 100.0));
            Label::new("The titlebar is smaller than a toolbar and has centered text")
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_tertiary)
                .render(canvas);
            canvas.restore();
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing window with titlebar...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WindowWithTitlebarApp { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
