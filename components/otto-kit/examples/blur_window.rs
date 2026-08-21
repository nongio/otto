//! The smallest window that is actually frosted.
//!
//! A blur needs the pixels *behind* the surface, and the only process that has
//! them is the compositor — so the frost is not painted here. The window asks
//! for it through `otto-surface-style`: a background colour on the compositor's
//! own layer, plus `BlendMode::BackgroundBlur`, and the client buffer stays
//! transparent so the result shows through. Anything painted as a ground in the
//! buffer — even a translucent one — sits on top of the blur instead.
//!
//! ```sh
//! cargo run -p otto-kit --example blur_window
//! ```

use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode};

const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 320.0;
const CORNER: f32 = 18.0;

struct BlurWindow {
    window: Option<Window>,
}

impl App for BlurWindow {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Blur Window", WIDTH as i32, HEIGHT as i32)?;
        // No opaque backing colour, or there is nothing for the blur to show
        // through.
        window.set_background(Color::TRANSPARENT);

        match window.surface_style() {
            Some(style) => {
                // The material's colour goes on the compositor's layer, not into
                // the buffer: `BackgroundBlur` blurs what is behind that layer
                // and tints the result with this colour.
                let colour = skia_safe::Color4f::from(AppContext::current_theme().material_popup);
                style.set_background_color(
                    colour.r as f64,
                    colour.g as f64,
                    colour.b as f64,
                    colour.a as f64,
                );
                style.set_blend_mode(BlendMode::BackgroundBlur);
                style.set_corner_radius(CORNER as f64);
                style.set_masks_to_bounds(ClipMode::Enabled);
            }
            None => {
                eprintln!("blur_window: no otto-surface-style — the window will not be frosted")
            }
        }

        window.on_draw(|canvas| {
            let theme = AppContext::current_theme();
            // Every frame lands in a fresh, uninitialised buffer, so start by
            // clearing it. Clear to transparent, never to a colour — the frost
            // is the compositor's layer underneath, and a ground painted here
            // would cover it.
            canvas.clear(Color::TRANSPARENT);

            Label::new("Frosted")
                .with_style(styles::TITLE_1_EMPHASIZED)
                .with_color(theme.text_primary)
                .centered_on(WIDTH / 2.0, HEIGHT / 2.0 - 14.0)
                .with_align(TextAlign::Center)
                .render(canvas);

            Label::new("background blur, drawn by the compositor")
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(WIDTH / 2.0, HEIGHT / 2.0 + 16.0)
                .with_align(TextAlign::Center)
                .render(canvas);
        });

        AppContext::register_window(window.clone());
        self.window = Some(window);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Runtime::new()?
        .block_on(async { AppRunner::new(BlurWindow { window: None }).run() })
}
