//! Live proof that the colour picker's popup actually opens, anchored to
//! the well that owns it, and that a colour can be chosen from each of the
//! three modes.
//!
//! A real toplevel with a single well. Clicking it opens the picker
//! anchored to the well's rect; picking a swatch, or dragging in the HSV
//! square/hue strip, updates the well live; clicking outside or ESC
//! dismisses it without discarding the last chosen colour. Follows
//! `dropdown_demo.rs`.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 timeout 10 cargo run -p otto-kit --example color_picker_demo
//! ```
//!
//! While it runs: `WAYLAND_DISPLAY=wayland-1 grim /tmp/picker.png` to
//! capture the open popup.

use std::sync::{Arc, Mutex};

use otto_kit::components::color_picker::panel::Swatch;
use otto_kit::components::color_picker::well::{self, WellInteraction};
use otto_kit::components::color_picker::ColorPickerPopup;
use otto_kit::prelude::*;
use skia_safe::Color;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::XdgSurface;

const WIDTH: i32 = 320;
const HEIGHT: i32 = 180;

/// The bit of the well's state that changes. Plain data, shared with
/// `Window::on_draw`'s `Send` bound the same way `dropdown_demo`'s `FieldUi`
/// is — the popup itself (`!Send`, owns Wayland objects) stays out of this
/// and is only ever touched from the pointer closure.
#[derive(Clone, Copy)]
struct WellUi {
    color: Color,
    hovered: bool,
    pressed: bool,
    open: bool,
}

struct ColorPickerDemo {
    window: Option<Window>,
}

impl App for ColorPickerDemo {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Colour Picker Demo", WIDTH, HEIGHT)?;
        window.set_background(Color::from_rgb(0xF3, 0xF4, 0xF6));

        let well_rect = Rect::from_xywh(24.0, 24.0, 0.0, well::HEIGHT); // width filled in per-draw from measure()

        let ui = Arc::new(Mutex::new(WellUi {
            color: Color::from_rgb(0x0A, 0x84, 0xFF),
            hovered: false,
            pressed: false,
            open: false,
        }));

        // Built once, up front — see `color_picker::popup`'s module docs
        // for why building this lazily inside a pointer handler panics.
        let popup = ColorPickerPopup::new(vec![
            Swatch::new("Blue", Color::from_rgb(0x0A, 0x84, 0xFF)),
            Swatch::new("Purple", Color::from_rgb(0xBF, 0x5A, 0xF2)),
            Swatch::new("Pink", Color::from_rgb(0xFF, 0x2D, 0x55)),
            Swatch::new("Red", Color::from_rgb(0xFF, 0x3B, 0x30)),
            Swatch::new("Orange", Color::from_rgb(0xFF, 0x95, 0x00)),
            Swatch::new("Yellow", Color::from_rgb(0xFF, 0xCC, 0x00)),
            Swatch::new("Green", Color::from_rgb(0x34, 0xC7, 0x59)),
        ]);

        {
            let ui = ui.clone();
            window.on_draw(move |canvas| {
                let theme = Theme::light();
                Label::new("Click the swatch to open the picker")
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_tertiary)
                    .at(24.0, 24.0)
                    .render(canvas);

                let snapshot = *ui.lock().unwrap();
                let interaction = if snapshot.open {
                    WellInteraction::Open
                } else if snapshot.pressed {
                    WellInteraction::Pressed
                } else if snapshot.hovered {
                    WellInteraction::Hovered
                } else {
                    WellInteraction::Normal
                };
                let rect = Rect::from_xywh(
                    well_rect.left,
                    56.0,
                    well::measure(snapshot.color),
                    well::HEIGHT,
                );
                well::draw(canvas, rect, snapshot.color, interaction, &theme);
            });
        }

        {
            let ui = ui.clone();
            let window_ref = window.clone();
            window.on_pointer_event(move |events| {
                let mut dirty = false;

                for event in events {
                    let (px, py) = (event.position.0 as f32, event.position.1 as f32);
                    let color = ui.lock().unwrap().color;
                    let rect = Rect::from_xywh(24.0, 56.0, well::measure(color), well::HEIGHT);

                    match event.kind {
                        PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                            let hovered = well::hit_test(rect, px, py);
                            let mut state = ui.lock().unwrap();
                            if state.hovered != hovered {
                                state.hovered = hovered;
                                dirty = true;
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            let mut state = ui.lock().unwrap();
                            if state.hovered || state.pressed {
                                state.hovered = false;
                                state.pressed = false;
                                dirty = true;
                            }
                        }
                        PointerEventKind::Press { serial, .. } => {
                            if !well::hit_test(rect, px, py) {
                                continue;
                            }
                            ui.lock().unwrap().pressed = true;
                            dirty = true;

                            if popup.is_open() {
                                popup.close();
                                ui.lock().unwrap().open = false;
                                continue;
                            }

                            let Some(surface) = window_ref.surface() else {
                                continue;
                            };
                            let xdg_surface = surface.xdg_window().xdg_surface().clone();

                            let ui_change = ui.clone();
                            let window_change = window_ref.clone();
                            let ui_close = ui.clone();
                            let window_close = window_ref.clone();

                            popup.open(
                                &xdg_surface,
                                rect,
                                serial,
                                color,
                                Theme::light(),
                                move |new_color| {
                                    let mut state = ui_change.lock().unwrap();
                                    state.color = new_color;
                                    drop(state);
                                    window_change.request_frame();
                                },
                                move || {
                                    let mut state = ui_close.lock().unwrap();
                                    state.open = false;
                                    drop(state);
                                    window_close.request_frame();
                                },
                            );
                            ui.lock().unwrap().open = popup.is_open();
                        }
                        PointerEventKind::Release { .. } => {
                            let mut state = ui.lock().unwrap();
                            if state.pressed {
                                state.pressed = false;
                                dirty = true;
                            }
                        }
                        _ => {}
                    }
                }

                if dirty {
                    window_ref.request_frame();
                }
            });
        }

        self.window = Some(window);
        Ok(())
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = ColorPickerDemo { window: None };
    AppRunner::new(app).run()
}
