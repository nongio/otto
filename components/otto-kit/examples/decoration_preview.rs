//! Live preview of the Otto window decoration.
//!
//! A real toplevel that draws `WindowDecoration` — the same struct the
//! compositor will draw for server-side decorated windows — into its own
//! surface, with `background_blur` set so the desktop behind the titlebar is
//! blurred by the compositor for real.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 cargo run -p otto-kit --example decoration_preview
//! ```
//!
//! - drag the titlebar to move the window
//! - hover the controls to reveal their glyphs
//! - click the red dot to quit
//! - click anywhere in the body to cycle through the style variants

use std::sync::{Arc, Mutex};

use otto_kit::components::titlebar::{WindowControl, WindowDecoration};
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::BlendMode;
use smithay_client_toolkit::seat::pointer::PointerEventKind;

const WIDTH: i32 = 720;
const HEIGHT: i32 = 460;
const CORNER: f32 = 12.0;

/// One point in the style space we're evaluating.
#[derive(Clone, Copy)]
struct Variant {
    name: &'static str,
    active: bool,
    dark: bool,
    height: f32,
    /// Title type style
    title: TextStyle,
    /// Compositor blur behind the surface. When off, the titlebar tint is
    /// composited straight over the desktop.
    blur: bool,
}

const VARIANTS: &[Variant] = &[
    Variant {
        name: "light · active · blurred · 13pt title",
        active: true,
        dark: false,
        height: 34.0,
        title: styles::BODY_EMPHASIZED,
        blur: true,
    },
    Variant {
        name: "light · active · blurred · 15pt title",
        active: true,
        dark: false,
        height: 34.0,
        title: styles::TITLE_3_EMPHASIZED,
        blur: true,
    },
    Variant {
        name: "light · active · no blur",
        active: true,
        dark: false,
        height: 34.0,
        title: styles::BODY_EMPHASIZED,
        blur: false,
    },
    Variant {
        name: "light · inactive · blurred",
        active: false,
        dark: false,
        height: 34.0,
        title: styles::BODY_EMPHASIZED,
        blur: true,
    },
    Variant {
        name: "dark · active · blurred",
        active: true,
        dark: true,
        height: 34.0,
        title: styles::BODY_EMPHASIZED,
        blur: true,
    },
    Variant {
        name: "dark · inactive · blurred",
        active: false,
        dark: true,
        height: 34.0,
        title: styles::BODY_EMPHASIZED,
        blur: true,
    },
    Variant {
        name: "light · compact 28pt bar",
        active: true,
        dark: false,
        height: 28.0,
        title: styles::BODY_EMPHASIZED,
        blur: true,
    },
    Variant {
        name: "light · tall 44pt bar · 15pt title",
        active: true,
        dark: false,
        height: 44.0,
        title: styles::TITLE_3_EMPHASIZED,
        blur: true,
    },
];

#[derive(Default)]
struct PreviewState {
    variant: usize,
    hovered: bool,
    pressed: Option<WindowControl>,
}

impl PreviewState {
    fn variant(&self) -> Variant {
        VARIANTS[self.variant % VARIANTS.len()]
    }

    /// Build the decoration the way the compositor would: the surface already
    /// carries `background_blur`, so no local blur.
    fn decoration(&self, title: &str, width: f32) -> WindowDecoration {
        let v = self.variant();
        WindowDecoration {
            title: title.to_string(),
            width,
            titlebar_height: v.height,
            corner_radius: CORNER,
            active: v.active,
            dark: v.dark,
            controls_hovered: self.hovered,
            pressed: self.pressed,
            title_style: v.title,
            ..Default::default()
        }
    }
}

struct DecorationPreview {
    window: Option<Window>,
    state: Arc<Mutex<PreviewState>>,
}

impl App for DecorationPreview {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Documents — index.md", WIDTH, HEIGHT)?;
        // The framework clears the canvas with this before calling on_draw;
        // transparent lets the blurred backdrop through wherever we don't
        // paint an opaque pixel.
        window.set_background(Color::TRANSPARENT);

        if let Some(style) = window.surface_style() {
            style.set_corner_radius(CORNER as f64);
            style.set_blend_mode(BlendMode::BackgroundBlur);
        }

        let state = self.state.clone();
        let title = window.title();
        window.on_draw(move |canvas| {
            let state = state.lock().unwrap();
            let v = state.variant();
            let deco = state.decoration(&title, WIDTH as f32);
            draw_body(canvas, &deco, &v);
            deco.draw(canvas);
        });

        // Pointer: hover reveals the glyphs, the bar drags, the dots act, and
        // the body cycles variants.
        let state = self.state.clone();
        let window_ref = window.clone();
        let title = window.title();
        window.on_pointer_event(move |events| {
            for event in events {
                let (px, py) = (event.position.0 as f32, event.position.1 as f32);
                let mut state = state.lock().unwrap();
                let deco = state.decoration(&title, WIDTH as f32);
                let mut dirty = false;

                match event.kind {
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        let hovered = deco.control_at(px, py).is_some();
                        if hovered != state.hovered {
                            state.hovered = hovered;
                            dirty = true;
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        if state.hovered || state.pressed.is_some() {
                            state.hovered = false;
                            state.pressed = None;
                            dirty = true;
                        }
                    }
                    PointerEventKind::Press { serial, .. } => {
                        if let Some(control) = deco.control_at(px, py) {
                            state.pressed = Some(control);
                            dirty = true;
                        } else if deco.is_drag_area(px, py) {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                window_ref.start_move(&seat, serial);
                            }
                        } else {
                            state.variant = state.variant.wrapping_add(1);
                            let v = state.variant();
                            println!("variant: {}", v.name);
                            // Blur is a property of the surface, not of the
                            // paint, so it has to follow the variant here.
                            if let Some(style) = window_ref.surface_style() {
                                style.set_blend_mode(if v.blur {
                                    BlendMode::BackgroundBlur
                                } else {
                                    BlendMode::Normal
                                });
                            }
                            dirty = true;
                        }
                    }
                    PointerEventKind::Release { .. } => {
                        if let Some(control) = state.pressed.take() {
                            dirty = true;
                            // Only counts if the release lands on the same dot
                            if deco.control_at(px, py) == Some(control)
                                && control == WindowControl::Close
                            {
                                std::process::exit(0);
                            }
                        }
                    }
                    _ => {}
                }

                if dirty {
                    drop(state);
                    window_ref.request_frame();
                }
            }
        });

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

/// The client area under the titlebar. Translucent too, so the compositor's
/// blur of the desktop carries through the whole surface — the titlebar just
/// sits a step more transparent than the body.
fn draw_body(canvas: &Canvas, deco: &WindowDecoration, variant: &Variant) {
    let theme = if variant.dark {
        Theme::dark()
    } else {
        Theme::light()
    };
    let top = deco.content_offset();

    let mut bg = Paint::default();
    bg.set_anti_alias(true);
    bg.set_color(if variant.dark {
        Color::from_argb(0xE0, 0x24, 0x26, 0x2B)
    } else {
        Color::from_argb(0xE0, 0xFF, 0xFF, 0xFF)
    });
    canvas.draw_rect(Rect::from_ltrb(0.0, top, WIDTH as f32, HEIGHT as f32), &bg);

    canvas.save();
    canvas.translate((28.0, top + 28.0));
    Label::new(variant.name)
        .with_style(styles::HEADLINE)
        .with_color(theme.text_primary)
        .render(canvas);
    canvas.restore();

    canvas.save();
    canvas.translate((28.0, top + 58.0));
    Label::new("click the body to cycle · drag the bar to move · red dot quits")
        .with_style(styles::FOOTNOTE)
        .with_color(theme.text_tertiary)
        .render(canvas);
    canvas.restore();

    // Placeholder text rows, so the bar is judged against real content
    let mut row = Paint::default();
    row.set_anti_alias(true);
    row.set_color(if variant.dark {
        Color::from_argb(0x2A, 0xFF, 0xFF, 0xFF)
    } else {
        Color::from_argb(0x1F, 0x00, 0x00, 0x00)
    });
    let widths = [0.62, 0.84, 0.74, 0.48, 0.8, 0.36, 0.66];
    for (i, w) in widths.iter().enumerate() {
        let y = top + 100.0 + i as f32 * 24.0;
        if y > HEIGHT as f32 - 24.0 {
            break;
        }
        canvas.draw_rrect(
            skia_safe::RRect::new_rect_xy(
                Rect::from_xywh(28.0, y, (WIDTH as f32 - 56.0) * w, 10.0),
                5.0,
                5.0,
            ),
            &row,
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = DecorationPreview {
        window: None,
        state: Arc::new(Mutex::new(PreviewState::default())),
    };
    AppRunner::new(app).run()?;
    Ok(())
}
