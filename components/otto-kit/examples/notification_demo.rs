use otto_kit::components::icon::Icon;
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::BlendMode;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use std::sync::{Arc, RwLock};

struct NotificationDemoApp {
    window: Option<Window>,
    notification_type: Arc<RwLock<usize>>,
}

impl App for NotificationDemoApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let theme = Theme::light();
        let mut window = Window::new("Notification", 380, 100)?;

        // Semi-transparent white background
        window.set_background(Color::from_argb(150, 255, 255, 255));

        // Apply background blur
        if let Some(surface) = window.surface() {
            if let Some(layer) = surface.surface_style() {
                layer.set_background_color(0.0, 0.0, 0.0, 0.0);
                layer.set_blend_mode(BlendMode::BackgroundBlur);
            }
        }

        // Click handler to cycle through notification types
        let notification_type = self.notification_type.clone();
        let window_clone = window.clone();
        window.on_pointer_event(move |events| {
            for event in events {
                if let PointerEventKind::Press { .. } = event.kind {
                    if let Ok(mut notif_type) = notification_type.write() {
                        *notif_type = (*notif_type + 1) % 4;
                        if let Some(surface) = window_clone.surface() {
                            surface.request_frame();
                        }
                    }
                }
            }
        });

        let notification_type = self.notification_type.clone();
        window.on_draw(move |canvas| {
            let notif_type = notification_type.read().ok().map(|n| *n).unwrap_or(0);

            // Render only one notification based on current type
            match notif_type {
                0 => {
                    // Message notification with gray title and time
                    Self::draw_notification(
                        canvas,
                        "message-circle",
                        theme.accent_blue,
                        Some("Messages"),
                        "New Message from Alice",
                        "Hey, are you available for a quick call?",
                        Some("2 min ago"),
                        &theme,
                    );
                }
                1 => {
                    // Success notification with time
                    Self::draw_notification(
                        canvas,
                        "circle-check",
                        Color::from_rgb(52, 199, 89), // Green
                        None,
                        "Upload Complete",
                        "Your file has been uploaded successfully",
                        Some("Just now"),
                        &theme,
                    );
                }
                2 => {
                    // Warning notification with gray title
                    Self::draw_notification(
                        canvas,
                        "alert-triangle",
                        Color::from_rgb(255, 149, 0), // Orange
                        Some("System"),
                        "Storage Almost Full",
                        "You have less than 10% storage remaining",
                        None,
                        &theme,
                    );
                }
                _ => {
                    // Error notification with gray title and time
                    Self::draw_notification(
                        canvas,
                        "alert-circle",
                        Color::from_rgb(255, 59, 48), // Red
                        Some("Network"),
                        "Connection Failed",
                        "Unable to connect to the server. Try again.",
                        Some("5 sec ago"),
                        &theme,
                    );
                }
            }
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing notification demo...");
        true
    }
}

impl NotificationDemoApp {
    #[allow(clippy::too_many_arguments)]
    fn draw_notification(
        canvas: &skia_safe::Canvas,
        icon_name: &str,
        icon_color: Color,
        gray_title: Option<&str>,
        title: &str,
        body_text: &str,
        time: Option<&str>,
        theme: &Theme,
    ) {
        let width = 380.0;
        // let height = 100.0;
        let padding = 20.0;
        let icon_size = 32.0;

        // Draw icon on the left
        Icon::filled(icon_name)
            .at(padding, padding)
            .with_size(icon_size)
            .with_color(icon_color)
            .render(canvas);

        // Calculate content area
        let content_x = padding + icon_size + 16.0;
        let content_width = width - content_x - padding - if time.is_some() { 70.0 } else { 20.0 };

        let mut y_offset = padding;

        // Draw time in top right if provided
        if let Some(time_text) = time {
            Label::new(time_text)
                .at(width - 80.0, padding)
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_tertiary)
                .render(canvas);
        }

        // Draw gray title if provided (category/app name)
        if let Some(gray_text) = gray_title {
            Label::new(gray_text)
                .at(content_x, y_offset)
                .with_style(styles::CAPTION_1)
                .with_color(theme.text_secondary)
                .render(canvas);
            y_offset += 18.0;
        }

        // Draw main title (always black text, emphasized)
        Label::new(title)
            .at(content_x, y_offset)
            .with_style(styles::HEADLINE_EMPHASIZED)
            .with_color(theme.text_primary)
            .with_width(content_width)
            .render(canvas);
        y_offset += 22.0;

        // Draw body text (always black text)
        Label::new(body_text)
            .at(content_x, y_offset)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .with_width(content_width)
            .render(canvas);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = NotificationDemoApp {
        window: None,
        notification_type: Arc::new(RwLock::new(0)),
    };
    AppRunner::new(app).run()?;
    Ok(())
}
