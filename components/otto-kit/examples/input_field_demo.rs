use otto_kit::prelude::*;
use otto_kit::KeyEvent;
use skia_safe::Color;
use std::sync::{Arc, Mutex};
use wayland_client::protocol::wl_keyboard;

struct InputFieldDemoApp {
    window: Option<Window>,
    /// All interactive input fields (indices 0..N-1 are focusable)
    fields: Vec<Arc<Mutex<InputField>>>,
    /// Display-only disabled field (not in the tab cycle)
    disabled_field: Arc<Mutex<InputField>>,
}

impl InputFieldDemoApp {
    fn new() -> Self {
        let mut text_input = InputField::new()
            .at(50.0, 120.0)
            .with_width(300.0)
            .with_placeholder("Type something here...")
            .build();
        text_input.focus();

        let styled_input = InputField::new()
            .at(50.0, 220.0)
            .with_width(300.0)
            .with_placeholder("Another field...")
            .with_corner_radius(12.0)
            .with_focused_border_color(Color::from_rgb(52, 199, 89))
            .build();

        let password_input = InputField::new()
            .at(50.0, 320.0)
            .with_width(300.0)
            .with_placeholder("Enter password...")
            .with_mask()
            .build();

        let disabled_input = InputField::new()
            .at(50.0, 420.0)
            .with_width(300.0)
            .with_text("Pre-filled text")
            .with_state(InputFieldState::Disabled)
            .build();

        Self {
            window: None,
            fields: vec![
                Arc::new(Mutex::new(text_input)),
                Arc::new(Mutex::new(styled_input)),
                Arc::new(Mutex::new(password_input)),
            ],
            disabled_field: Arc::new(Mutex::new(disabled_input)),
        }
    }

    fn focused_index(&self) -> Option<usize> {
        for (i, f) in self.fields.iter().enumerate() {
            if f.lock().unwrap().is_focused() {
                return Some(i);
            }
        }
        None
    }

    fn blur_all(&self) {
        for f in &self.fields {
            f.lock().unwrap().blur();
        }
    }

    fn redraw(&mut self) {
        let fields: Vec<_> = self.fields.iter().map(Arc::clone).collect();
        let disabled = Arc::clone(&self.disabled_field);

        if let Some(window) = &mut self.window {
            window.on_draw(move |canvas| {
                Label::new("Input Field Demo")
                    .at(50.0, 40.0)
                    .with_style(styles::LARGE_TITLE_EMPHASIZED)
                    .with_color(Color::from_rgb(30, 30, 30))
                    .render(canvas);

                let sections: &[(&str, usize, bool)] = &[
                    ("Default input (Tab to switch focus):", 0, true),
                    ("Custom styled (green focus, rounded):", 1, true),
                    ("Password (masked):", 2, true),
                ];

                for &(label_text, idx, show_value) in sections {
                    let field = fields[idx].lock().unwrap();
                    let y = field.y;

                    Label::new(label_text)
                        .at(50.0, y - 20.0)
                        .with_style(styles::HEADLINE)
                        .with_color(Color::from_rgb(60, 60, 60))
                        .render(canvas);

                    field.render(canvas);

                    if show_value {
                        Label::new(&format!("Value: \"{}\"", field.text()))
                            .at(50.0, y + 50.0)
                            .with_style(styles::CAPTION_1)
                            .with_color(Color::from_rgb(120, 120, 120))
                            .render(canvas);
                    }
                }

                // Disabled field
                let dis = disabled.lock().unwrap();
                Label::new("Disabled:")
                    .at(50.0, dis.y - 20.0)
                    .with_style(styles::HEADLINE)
                    .with_color(Color::from_rgb(60, 60, 60))
                    .render(canvas);
                dis.render(canvas);
            });
            window.request_frame();
        }
    }
}

impl App for InputFieldDemoApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Input Field Demo", 500, 550)?;
        window.set_background(Color::from_rgb(245, 245, 247));
        self.window = Some(window);
        self.redraw();
        Ok(())
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        if state != wl_keyboard::KeyState::Pressed {
            return;
        }

        // Tab to cycle focus
        if event.raw_code == otto_kit::input::keycodes::TAB {
            let current = self.focused_index().unwrap_or(0);
            self.blur_all();
            let next = (current + 1) % self.fields.len();
            self.fields[next].lock().unwrap().focus();
            self.redraw();
            return;
        }

        // Forward key to the focused input
        let mods = AppContext::modifiers();
        if let Some(idx) = self.focused_index() {
            self.fields[idx].lock().unwrap().handle_key_mod(
                event.raw_code,
                event.utf8.as_deref(),
                mods.shift,
                mods.ctrl,
            );
        }
        self.redraw();
    }

    fn on_pointer_event(
        &mut self,
        _ctx: &AppContext,
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        use smithay_client_toolkit::seat::pointer::PointerEventKind;
        for ev in events {
            if let PointerEventKind::Press { .. } = ev.kind {
                let (x, y) = (ev.position.0 as f32, ev.position.1 as f32);

                self.blur_all();
                for f in &self.fields {
                    let mut field = f.lock().unwrap();
                    if field.hit_test(x, y) {
                        field.focus();
                        field.place_cursor_at_x(x);
                        break;
                    }
                }
                self.redraw();
            }
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(16))
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let mut needs_redraw = false;
        for f in &self.fields {
            needs_redraw |= f.lock().unwrap().tick_cursor_blink();
        }
        if needs_redraw {
            self.redraw();
        }
    }

    fn on_close(&mut self) -> bool {
        true
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = InputFieldDemoApp::new();
    AppRunner::new(app).run()?;
    Ok(())
}
