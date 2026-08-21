//! Live proof that the dropdown's menu half actually opens, anchored to the
//! field that owns it.
//!
//! A real toplevel with two independent dropdown fields. Clicking a field
//! opens a `ContextMenu` anchored to that field's rect; picking an option
//! updates the field and closes the menu; clicking outside or ESC dismisses
//! it without a choice. Having two on screen at once is the point — it
//! proves selection routes back to the field that was actually clicked, not
//! whichever dropdown happened to open last.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 timeout 8 cargo run -p otto-kit --example dropdown_demo
//! ```
//!
//! While it runs: `WAYLAND_DISPLAY=wayland-1 grim /tmp/dropdown.png` to
//! capture the open menu.

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use otto_kit::components::dropdown::field::{self, DropdownInteraction};
use otto_kit::components::dropdown::DropdownMenu;
use otto_kit::prelude::*;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::XdgSurface;

const WIDTH: i32 = 420;
const HEIGHT: i32 = 220;

/// Static layout for one field — never mutated after construction, so it's
/// freely cloned into both the draw and pointer closures.
#[derive(Clone)]
struct FieldSpec {
    rect: Rect,
    options: Vec<String>,
}

/// The bit of a field's state that changes. Plain data (no `Rc`), so it can
/// live in an `Arc<Mutex<_>>` shared with `Window::on_draw`'s `Send` bound —
/// the `DropdownMenu` itself (which owns a `ContextMenu`, and so is `!Send`)
/// stays out of this and is only ever touched from the pointer closure.
#[derive(Clone, Copy, Default)]
struct FieldUi {
    selected: usize,
    hovered: bool,
    pressed: bool,
    open: bool,
}

struct UiState {
    fields: Vec<FieldUi>,
}

struct DropdownDemo {
    window: Option<Window>,
}

impl App for DropdownDemo {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Dropdown Demo", WIDTH, HEIGHT)?;
        window.set_background(Color::from_rgb(0xF3, 0xF4, 0xF6));

        let specs = vec![
            FieldSpec {
                rect: Rect::from_xywh(24.0, 56.0, 190.0, field::HEIGHT),
                options: vec!["Automatic".into(), "Manual".into(), "Off".into()],
            },
            FieldSpec {
                rect: Rect::from_xywh(24.0, 100.0, 190.0, field::HEIGHT),
                options: vec!["Light".into(), "Dark".into(), "System".into()],
            },
        ];

        let ui = Arc::new(Mutex::new(UiState {
            fields: vec![FieldUi::default(); specs.len()],
        }));
        // One `DropdownMenu` per field, reused across every open — see
        // `dropdown::menu`'s module docs for why that matters. Lives only in
        // the pointer closure's captured environment; the draw closure never
        // touches it.
        let menus: Rc<Vec<DropdownMenu>> =
            Rc::new(specs.iter().map(|_| DropdownMenu::new()).collect());

        {
            let ui = ui.clone();
            let specs = specs.clone();
            window.on_draw(move |canvas| {
                let theme = Theme::light();
                Label::new("Click a field to open its menu; try both at once")
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_tertiary)
                    .at(24.0, 24.0)
                    .render(canvas);

                let snapshot = ui.lock().unwrap().fields.clone();
                for (spec, fui) in specs.iter().zip(snapshot.iter()) {
                    let interaction = if fui.open {
                        DropdownInteraction::Open
                    } else if fui.pressed {
                        DropdownInteraction::Pressed
                    } else if fui.hovered {
                        DropdownInteraction::Hovered
                    } else {
                        DropdownInteraction::Normal
                    };
                    field::draw(
                        canvas,
                        spec.rect,
                        &spec.options[fui.selected],
                        interaction,
                        &theme,
                    );
                }
            });
        }

        {
            let ui = ui.clone();
            let specs = specs.clone();
            let menus = menus.clone();
            let window_ref = window.clone();
            window.on_pointer_event(move |events| {
                let mut dirty = false;

                for event in events {
                    let (px, py) = (event.position.0 as f32, event.position.1 as f32);

                    match event.kind {
                        PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                            let mut state = ui.lock().unwrap();
                            for (i, spec) in specs.iter().enumerate() {
                                let hovered = field::hit_test(spec.rect, px, py);
                                if state.fields[i].hovered != hovered {
                                    state.fields[i].hovered = hovered;
                                    dirty = true;
                                }
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            let mut state = ui.lock().unwrap();
                            for f in state.fields.iter_mut() {
                                if f.hovered || f.pressed {
                                    f.hovered = false;
                                    f.pressed = false;
                                    dirty = true;
                                }
                            }
                        }
                        PointerEventKind::Press { serial, .. } => {
                            for (i, spec) in specs.iter().enumerate() {
                                if !field::hit_test(spec.rect, px, py) {
                                    continue;
                                }
                                ui.lock().unwrap().fields[i].pressed = true;
                                dirty = true;

                                if menus[i].is_open() {
                                    menus[i].close();
                                    continue;
                                }
                                // Opening one dropdown closes any sibling
                                // that happens to be open — only one field
                                // should read as "open" at a time.
                                for (j, m) in menus.iter().enumerate() {
                                    if j != i {
                                        m.close();
                                        ui.lock().unwrap().fields[j].open = false;
                                    }
                                }

                                let Some(surface) = window_ref.surface() else {
                                    continue;
                                };
                                let xdg_surface = surface.xdg_window().xdg_surface().clone();
                                let selected = ui.lock().unwrap().fields[i].selected;

                                let ui_select = ui.clone();
                                let window_select = window_ref.clone();
                                let ui_dismiss = ui.clone();
                                let window_dismiss = window_ref.clone();

                                menus[i].open(
                                    &xdg_surface,
                                    spec.rect,
                                    serial,
                                    &spec.options,
                                    Some(selected),
                                    move |index| {
                                        let mut state = ui_select.lock().unwrap();
                                        state.fields[i].selected = index;
                                        state.fields[i].open = false;
                                        drop(state);
                                        window_select.request_frame();
                                    },
                                    move || {
                                        let mut state = ui_dismiss.lock().unwrap();
                                        state.fields[i].open = false;
                                        drop(state);
                                        window_dismiss.request_frame();
                                    },
                                );
                                ui.lock().unwrap().fields[i].open = menus[i].is_open();
                            }
                        }
                        PointerEventKind::Release { .. } => {
                            let mut state = ui.lock().unwrap();
                            for f in state.fields.iter_mut() {
                                if f.pressed {
                                    f.pressed = false;
                                    dirty = true;
                                }
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
    let app = DropdownDemo { window: None };
    AppRunner::new(app).run()?;
    Ok(())
}
