use otto_kit::prelude::*;
use otto_kit::KeyEvent;
use skia_safe::Color;
use std::sync::{Arc, Mutex};
use wayland_client::protocol::wl_keyboard;

struct InputFieldDemoApp {
    window: Option<Window>,
    input1: Arc<Mutex<InputField>>,
    input2: Arc<Mutex<InputField>>,
    input3: Arc<Mutex<InputField>>,
}

impl InputFieldDemoApp {
    fn new() -> Self {
        let mut input1 = InputField::new()
            .at(50.0, 120.0)
            .with_width(300.0)
            .with_placeholder("Type something here...")
            .build();
        input1.focus();

        let input2 = InputField::new()
            .at(50.0, 220.0)
            .with_width(300.0)
            .with_placeholder("Another field...")
            .with_corner_radius(12.0)
            .with_focused_border_color(Color::from_rgb(52, 199, 89))
            .build();

        let input3 = InputField::new()
            .at(50.0, 320.0)
            .with_width(300.0)
            .with_text("Pre-filled text")
            .with_state(InputFieldState::Disabled)
            .build();

        Self {
            window: None,
            input1: Arc::new(Mutex::new(input1)),
            input2: Arc::new(Mutex::new(input2)),
            input3: Arc::new(Mutex::new(input3)),
        }
    }

    fn redraw(&mut self) {
        let i1 = self.input1.clone();
        let i2 = self.input2.clone();
        let i3 = self.input3.clone();

        if let Some(window) = &mut self.window {
            window.on_draw(move |canvas| {
                let input1 = i1.lock().unwrap();
                let input2 = i2.lock().unwrap();
                let input3 = i3.lock().unwrap();

                // Title
                Label::new("Input Field Demo")
                    .at(50.0, 40.0)
                    .with_style(styles::LARGE_TITLE_EMPHASIZED)
                    .with_color(Color::from_rgb(30, 30, 30))
                    .render(canvas);

                // ── Field 1 ──
                Label::new("Default input (Tab to switch focus):")
                    .at(50.0, 100.0)
                    .with_style(styles::HEADLINE)
                    .with_color(Color::from_rgb(60, 60, 60))
                    .render(canvas);

                input1.render(canvas);

                Label::new(&format!("Value: \"{}\"", input1.text()))
                    .at(50.0, 170.0)
                    .with_style(styles::CAPTION_1)
                    .with_color(Color::from_rgb(120, 120, 120))
                    .render(canvas);

                // ── Field 2 ──
                Label::new("Custom styled (green focus, rounded):")
                    .at(50.0, 200.0)
                    .with_style(styles::HEADLINE)
                    .with_color(Color::from_rgb(60, 60, 60))
                    .render(canvas);

                input2.render(canvas);

                Label::new(&format!("Value: \"{}\"", input2.text()))
                    .at(50.0, 270.0)
                    .with_style(styles::CAPTION_1)
                    .with_color(Color::from_rgb(120, 120, 120))
                    .render(canvas);

                // ── Field 3 ──
                Label::new("Disabled:")
                    .at(50.0, 300.0)
                    .with_style(styles::HEADLINE)
                    .with_color(Color::from_rgb(60, 60, 60))
                    .render(canvas);

                input3.render(canvas);
            });
            window.request_frame();
        }
    }
}

impl App for InputFieldDemoApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let mut window = Window::new("Input Field Demo", 500, 500)?;
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

        // Tab to switch focus between input1 and input2
        if event.raw_code == otto_kit::input::keycodes::TAB {
            let mut i1 = self.input1.lock().unwrap();
            let mut i2 = self.input2.lock().unwrap();
            if i1.is_focused() {
                i1.blur();
                i2.focus();
            } else {
                i2.blur();
                i1.focus();
            }
            drop(i1);
            drop(i2);
            self.redraw();
            return;
        }

        // Forward key to the focused input (always redraw for cursor movement)
        let mods = AppContext::modifiers();
        {
            let mut i1 = self.input1.lock().unwrap();
            let mut i2 = self.input2.lock().unwrap();
            if i1.is_focused() {
                i1.handle_key_mod(event.raw_code, event.utf8.as_deref(), mods.shift, mods.ctrl);
            } else if i2.is_focused() {
                i2.handle_key_mod(event.raw_code, event.utf8.as_deref(), mods.shift, mods.ctrl);
            }
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

                let mut i1 = self.input1.lock().unwrap();
                let mut i2 = self.input2.lock().unwrap();

                let hit1 = i1.hit_test(x, y);
                let hit2 = i2.hit_test(x, y);

                i1.blur();
                i2.blur();

                if hit1 {
                    i1.focus();
                    i1.place_cursor_at_x(x);
                } else if hit2 {
                    i2.focus();
                    i2.place_cursor_at_x(x);
                }

                drop(i1);
                drop(i2);
                self.redraw();
            }
        }
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(16))
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let b1 = self.input1.lock().unwrap().tick_cursor_blink();
        let b2 = self.input2.lock().unwrap().tick_cursor_blink();
        if b1 || b2 {
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
