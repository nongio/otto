use otto_kit::components::button::Button;
use otto_kit::components::toolbar::{Toolbar, ToolbarGroup};
use otto_kit::prelude::*;
use skia_safe::Color;

struct AppLayoutApp {
    window: Option<Window>,
}

impl App for AppLayoutApp {
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Application Layout", 1200, 800)?;
        window.set_background(Color::from_rgb(245, 245, 247));

        window.on_draw(|canvas| {
            let window_width = 1200.0;
            let window_height = 800.0;
            let toolbar_height = 48.0;
            let sidebar_width = 240.0;

            // ===== TOOLBAR =====
            let leading = ToolbarGroup::new()
                .add_item(
                    Button::icon("layout-dashboard")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(59, 130, 246)),
                )
                .add_space(16.0)
                .add_item(
                    Button::new("Dashboard")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(30, 30, 30)),
                )
                .add_item(
                    Button::new("Projects")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(100, 100, 100)),
                )
                .add_item(
                    Button::new("Team")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(100, 100, 100)),
                )
                .add_item(
                    Button::new("Analytics")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(100, 100, 100)),
                )
                .build();

            let trailing = ToolbarGroup::new()
                .add_item(
                    Button::icon("search")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(100, 100, 100)),
                )
                .add_item(
                    Button::icon("bell")
                        .with_background(Color::TRANSPARENT)
                        .with_text_color(Color::from_rgb(100, 100, 100)),
                )
                .add_separator()
                .add_item(
                    Button::icon("user")
                        .with_background(Color::from_rgb(229, 231, 235))
                        .with_text_color(Color::from_rgb(60, 60, 60)),
                )
                .build();

            Toolbar::new()
                .at(0.0, 0.0)
                .with_width(window_width)
                .with_height(toolbar_height)
                .with_background(Color::WHITE)
                .with_border_bottom(Color::from_rgb(229, 231, 235))
                .with_leading(leading)
                .with_trailing(trailing)
                .render(canvas);

            // ===== SIDEBAR =====
            let sidebar_y = toolbar_height;
            let sidebar_height = window_height - toolbar_height;

            // Sidebar background
            let sidebar_rect =
                skia_safe::Rect::from_xywh(0.0, sidebar_y, sidebar_width, sidebar_height);
            let mut sidebar_paint = skia_safe::Paint::default();
            sidebar_paint.set_color(Color::WHITE);
            canvas.draw_rect(sidebar_rect, &sidebar_paint);

            // Sidebar border
            let mut border_paint = skia_safe::Paint::default();
            border_paint.set_color(Color::from_rgb(229, 231, 235));
            border_paint.set_style(skia_safe::PaintStyle::Stroke);
            border_paint.set_stroke_width(1.0);
            canvas.draw_line(
                (sidebar_width - 0.5, sidebar_y),
                (sidebar_width - 0.5, sidebar_y + sidebar_height),
                &border_paint,
            );

            // Sidebar content
            let sidebar_x = 20.0;
            let mut sidebar_item_y = sidebar_y + 30.0;

            Label::new("Navigation")
                .at(sidebar_x, sidebar_item_y)
                .with_style(styles::CAPTION_1)
                .with_color(Color::from_rgb(156, 163, 175))
                .render(canvas);

            sidebar_item_y += 35.0;

            let nav_items = [
                ("home", "Home", true),
                ("folder", "Projects", false),
                ("users", "Team Members", false),
                ("calendar", "Calendar", false),
                ("file-text", "Documents", false),
                ("settings", "Settings", false),
            ];

            for (icon, label, active) in &nav_items {
                let bg_color = if *active {
                    Color::from_rgb(239, 246, 255)
                } else {
                    Color::TRANSPARENT
                };

                let text_color = if *active {
                    Color::from_rgb(59, 130, 246)
                } else {
                    Color::from_rgb(100, 100, 100)
                };

                // Nav item background
                if *active {
                    let item_rect = skia_safe::Rect::from_xywh(
                        12.0,
                        sidebar_item_y - 8.0,
                        sidebar_width - 24.0,
                        36.0,
                    );
                    let rrect = skia_safe::RRect::new_rect_xy(item_rect, 6.0, 6.0);
                    let mut paint = skia_safe::Paint::default();
                    paint.set_color(bg_color);
                    canvas.draw_rrect(rrect, &paint);
                }

                Button::new(*label)
                    .at(sidebar_x, sidebar_item_y)
                    .with_icon(*icon)
                    .with_background(Color::TRANSPARENT)
                    .with_text_color(text_color)
                    .render(canvas);

                sidebar_item_y += 44.0;
            }

            // ===== MAIN CONTENT =====
            let content_x = sidebar_width + 40.0;
            let content_y = toolbar_height + 40.0;
            let content_width = window_width - sidebar_width - 80.0;

            // Page header
            Label::new("Dashboard Overview")
                .at(content_x, content_y)
                .with_style(styles::LARGE_TITLE_EMPHASIZED)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            Label::new("Welcome back! Here's what's happening with your projects.")
                .at(content_x, content_y + 40.0)
                .with_style(styles::BODY)
                .with_color(Color::from_rgb(100, 100, 100))
                .render(canvas);

            // Action buttons
            let button_y = content_y + 80.0;
            Button::new("New Project")
                .at(content_x, button_y)
                .with_icon("plus")
                .primary()
                .render(canvas);

            Button::new("Import")
                .at(content_x + 180.0, button_y)
                .with_icon("upload")
                .outline()
                .render(canvas);

            Button::new("Export")
                .at(content_x + 330.0, button_y)
                .with_icon("download")
                .outline()
                .render(canvas);

            // Stats cards
            let stats_y = button_y + 80.0;
            let card_width = (content_width - 32.0) / 3.0;

            let stats = [
                ("Total Projects", "24", Color::from_rgb(59, 130, 246)),
                ("Active Tasks", "127", Color::from_rgb(16, 185, 129)),
                ("Team Members", "18", Color::from_rgb(139, 92, 246)),
            ];

            for (i, (title, value, color)) in stats.iter().enumerate() {
                let card_x = content_x + (i as f32 * (card_width + 16.0));

                // Card background
                let card_rect = skia_safe::Rect::from_xywh(card_x, stats_y, card_width, 120.0);
                let rrect = skia_safe::RRect::new_rect_xy(card_rect, 12.0, 12.0);
                let mut paint = skia_safe::Paint::default();
                paint.set_color(Color::WHITE);
                canvas.draw_rrect(rrect, &paint);

                // Card border
                let mut border = skia_safe::Paint::default();
                border.set_color(Color::from_rgb(229, 231, 235));
                border.set_style(skia_safe::PaintStyle::Stroke);
                border.set_stroke_width(1.0);
                canvas.draw_rrect(rrect, &border);

                // Card content
                Label::new(*title)
                    .at(card_x + 20.0, stats_y + 30.0)
                    .with_style(styles::SUBHEADLINE)
                    .with_color(Color::from_rgb(100, 100, 100))
                    .render(canvas);

                Label::new(*value)
                    .at(card_x + 20.0, stats_y + 70.0)
                    .with_style(styles::LARGE_TITLE_EMPHASIZED)
                    .with_color(*color)
                    .render(canvas);
            }

            // Recent activity section
            let activity_y = stats_y + 160.0;

            Label::new("Recent Activity")
                .at(content_x, activity_y)
                .with_style(styles::TITLE_2)
                .with_color(Color::from_rgb(30, 30, 30))
                .render(canvas);

            let activity_list_y = activity_y + 50.0;

            // Activity list background
            let list_rect =
                skia_safe::Rect::from_xywh(content_x, activity_list_y, content_width, 200.0);
            let list_rrect = skia_safe::RRect::new_rect_xy(list_rect, 12.0, 12.0);
            let mut paint = skia_safe::Paint::default();
            paint.set_color(Color::WHITE);
            canvas.draw_rrect(list_rrect, &paint);

            // List border
            let mut border = skia_safe::Paint::default();
            border.set_color(Color::from_rgb(229, 231, 235));
            border.set_style(skia_safe::PaintStyle::Stroke);
            border.set_stroke_width(1.0);
            canvas.draw_rrect(list_rrect, &border);

            // Activity items
            let activities = [
                ("New project created", "2 hours ago"),
                ("Team member invited", "5 hours ago"),
                ("Document uploaded", "1 day ago"),
            ];

            let mut item_y = activity_list_y + 30.0;
            for (activity, time) in &activities {
                Label::new(*activity)
                    .at(content_x + 20.0, item_y)
                    .with_style(styles::BODY)
                    .with_color(Color::from_rgb(60, 60, 60))
                    .render(canvas);

                Label::new(*time)
                    .at(content_x + content_width - 150.0, item_y)
                    .with_style(styles::CAPTION_1)
                    .with_color(Color::from_rgb(156, 163, 175))
                    .render(canvas);

                item_y += 50.0;
            }
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        println!("Closing application layout...");
        true
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = AppLayoutApp { window: None };
    AppRunnerWithType::new(app).run()?;
    Ok(())
}
