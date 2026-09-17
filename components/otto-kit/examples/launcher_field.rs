//! The launcher's ask field, alone: the pieces that sit between a keystroke
//! and the glyph on screen, each behind a switch so they can be taken out one
//! at a time.
//!
//! Built the way `otto-launcher` builds its card:
//!
//! - a fullscreen overlay layer surface that takes the keyboard and draws
//!   nothing, painted once per configure;
//! - a subsurface for the card, frosted by the compositor through
//!   `otto-surface-style` (background blur, corners, clip, shadow), its buffer
//!   as tall as the launcher's card ever gets;
//! - the field as a `lay-rs` layer whose draw content is a clone of the
//!   `TextInput`, replaced on every keystroke;
//! - the engine's damage reported as the frame's damage, and an empty damage
//!   skipping the paint;
//! - the caret blinking on the update loop's clock.
//!
//! Every switch is an environment variable, `1` on and `0` off:
//!
//! | variable          | default | off means                                        |
//! |-------------------|---------|--------------------------------------------------|
//! | `LF_STYLE`        | 1       | no surface-style at all: an opaque buffer ground |
//! | `LF_BLUR`         | 1       | `BlendMode::Normal`, a flat translucent material |
//! | `LF_SHADOW`       | 1       | no shadow                                        |
//! | `LF_TALL`         | 1       | the card buffer is the field's height only       |
//! | `LF_ENGINE`       | 1       | the field is drawn straight into the canvas      |
//! | `LF_DAMAGE`       | 1       | every frame damages the whole card buffer        |
//! | `LF_SKIP_EMPTY`   | 1       | a paint with no engine damage draws anyway       |
//! | `LF_PARENT_ONCE`  | 1       | the fullscreen parent is repainted every frame   |
//! | `LF_BLINK`        | 1       | a steady caret                                   |
//! | `LF_WAIT_FRAME`   | 0       | (on) paint only once the last frame was answered |
//! | `LF_TRACE`        | 0       | (on) print per-keystroke timings to stderr       |
//!
//! The launcher's own behaviour is the defaults. The damage comes from
//! `AppContext::take_layers_damage`, which applies the changes made since the
//! engine last updated first: left to the renderer thread's 12 ms tick, a
//! paint straight after a keystroke found the previous keystroke's damage, or
//! none, and the change waited for the caret to blink.
//!
//! ```sh
//! cargo run -p otto-kit --example launcher_field
//! LF_BLUR=0 LF_TRACE=1 cargo run -p otto-kit --example launcher_field
//! RUST_LOG=otto_kit::frame=debug LF_TRACE=1 cargo run -p otto-kit --example launcher_field
//! ```
//!
//! The text is selectable as the launcher's is: Shift with the arrows, Home and
//! End, Ctrl+A, Ctrl+C/X/V, and with the pointer — press and drag, double-click
//! for a word, triple-click for everything.
//!
//! Escape quits.

use std::time::{Duration, Instant};

use layers::prelude::*;
use layers::types::{Point as LayerPoint, Size as LayerSize};
use otto_kit::clipboard;
use otto_kit::components::text_input::{
    KeyMods, TextInput, TextInputKey, TextInputResponse, TextInputStyle, CARET_BLINK_PERIOD,
};
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::surfaces::{LayerShellSurface, SubsurfaceSurface};
use otto_kit::theme::Theme;
use otto_kit::typography::styles;
use otto_kit::{App, AppContext, AppRunner};
use skia_safe::{Color, Rect};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, BTN_LEFT};
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer as ShellLayer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

// The launcher's geometry (otto-launcher/src/view.rs).
const CARD_W: f32 = 620.0;
const FIELD_H: f32 = 58.0;
const MAX_CARD_H: f32 = 840.0;
/// The room ask mode leaves above the field for the log.
const MAX_LOG_BLOCK: f32 = 397.0;
const RADIUS: f32 = 10.0;
const TOP_FRACTION: f32 = 0.16;
/// What the launcher's card material is floored at while frosted.
const CARD_MIN_ALPHA: u8 = 0xD8;
const FRAME_TIMEOUT: Duration = Duration::from_millis(500);
/// Presses closer together than this, near the same spot, count as one
/// double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

fn switch(name: &str, default: bool) -> bool {
    match std::env::var(name).as_deref() {
        Ok("0") | Ok("false") | Ok("off") => false,
        Ok(_) => true,
        Err(_) => default,
    }
}

#[derive(Debug, Clone, Copy)]
struct Switches {
    style: bool,
    blur: bool,
    shadow: bool,
    tall: bool,
    engine: bool,
    damage: bool,
    skip_empty: bool,
    parent_once: bool,
    blink: bool,
    wait_frame: bool,
    trace: bool,
}

impl Switches {
    fn from_env() -> Self {
        Self {
            style: switch("LF_STYLE", true),
            blur: switch("LF_BLUR", true),
            shadow: switch("LF_SHADOW", true),
            tall: switch("LF_TALL", true),
            engine: switch("LF_ENGINE", true),
            damage: switch("LF_DAMAGE", true),
            skip_empty: switch("LF_SKIP_EMPTY", true),
            parent_once: switch("LF_PARENT_ONCE", true),
            blink: switch("LF_BLINK", true),
            wait_frame: switch("LF_WAIT_FRAME", false),
            trace: switch("LF_TRACE", false),
        }
    }

    fn card_h(&self) -> f32 {
        if self.tall {
            MAX_CARD_H
        } else {
            FIELD_H
        }
    }
}

/// The launcher's field look (otto-launcher `field_style`), light.
fn field_style() -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(Theme::light());
    style.text_style = styles::TITLE_3;
    style.text_style.size = 19.0;
    style.horizontal_padding = 20.0;
    style.corner_radius = 0.0;
    style.focus_ring_width = 0.0;
    style.background = Color::TRANSPARENT;
    style.text_color = Color::BLACK;
    style.placeholder_color = Color::from_argb(100, 0, 0, 0);
    style
}

struct LauncherField {
    sw: Switches,
    surface: Option<LayerShellSurface>,
    card: Option<SubsurfaceSurface>,
    field_layer: Option<layers::prelude::Layer>,
    engine: Option<std::sync::Arc<Engine>>,
    input: TextInput,

    sized: bool,
    dirty: bool,
    parent_painted: bool,
    painted_at: Option<Instant>,
    last_tick: Instant,

    /// When the last keystroke that changed something arrived, until a paint
    /// takes it.
    key_at: Option<Instant>,
    skipped: u32,

    modifiers: Modifiers,
    /// The left button is down after a press on the field.
    dragging: bool,
    /// When and where the last press was, and how many came in a row.
    last_press: Option<(Instant, f64, f64)>,
    clicks: u32,
}

impl LauncherField {
    fn new(sw: Switches) -> Self {
        let mut input = TextInput::editing("", field_style());
        input.state.placeholder = "Ask anything".into();
        input.set_size(CARD_W, FIELD_H);
        Self {
            sw,
            surface: None,
            card: None,
            field_layer: None,
            engine: None,
            input,
            sized: false,
            dirty: true,
            parent_painted: false,
            painted_at: None,
            last_tick: Instant::now(),
            key_at: None,
            skipped: 0,
            modifiers: Modifiers::default(),
            dragging: false,
            last_press: None,
            clicks: 0,
        }
    }

    fn apply_material(&self, card: &SubsurfaceSurface) {
        if !self.sw.style {
            return;
        }
        let Some(style) = card.base_surface().surface_style() else {
            eprintln!("launcher_field: no otto-surface-style; the card will not be frosted");
            return;
        };
        let theme = AppContext::current_theme().material_popup;
        let colour = skia_safe::Color4f::from(Color::from_argb(
            theme.a().max(CARD_MIN_ALPHA),
            theme.r(),
            theme.g(),
            theme.b(),
        ));
        style.set_background_color(
            colour.r as f64,
            colour.g as f64,
            colour.b as f64,
            colour.a as f64,
        );
        style.set_blend_mode(if self.sw.blur {
            BlendMode::BackgroundBlur
        } else {
            BlendMode::Normal
        });
        let scale = AppContext::fractional_scale();
        style.set_corner_radius(RADIUS as f64 * scale);
        style.set_masks_to_bounds(ClipMode::Enabled);
        if self.sw.shadow {
            style.set_shadow(0.32, 32.0, 0.0, 12.0, 0.0, 0.0, 0.0);
        }
        style.set_contents_gravity(ContentsGravity::TopLeft);
        style.set_anchor_point(0.5, 0.0);
    }

    /// Place the card as the launcher does, and cut the material to the field.
    fn place_card(&self, output: (f32, f32)) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        let x = ((output.0 - CARD_W) / 2.0).round();
        // Ask mode's resting place: low enough for the tallest log above it.
        let y = ((output.1 * TOP_FRACTION).round() + MAX_LOG_BLOCK)
            .min(output.1 - (MAX_CARD_H - MAX_LOG_BLOCK))
            .max(0.0);
        card.set_position(x as i32, y as i32);
        if let (true, Some(style)) = (self.sw.style, card.base_surface().surface_style()) {
            let scale = AppContext::fractional_scale();
            style.set_position((x + CARD_W / 2.0) as f64 * scale, y as f64 * scale);
            style.set_size(CARD_W as f64 * scale, FIELD_H as f64 * scale);
        }
    }

    fn frame_in_flight(&self) -> bool {
        self.painted_at
            .is_some_and(|at| at.elapsed() < FRAME_TIMEOUT)
            && self
                .card
                .as_ref()
                .is_some_and(|card| card.base_surface().frame_in_flight())
    }

    /// Hand the field's current state to the scene, as `Palette::update` does.
    fn push(&self) {
        if let Some(layer) = self.field_layer.as_ref() {
            let field = self.input.clone();
            layer.set_draw_content(move |canvas: &skia_safe::Canvas, w: f32, h: f32| {
                field.render_at(canvas, w, h);
                Rect::from_wh(w, h)
            });
        }
    }

    /// Returns whether a frame went out.
    fn paint(&mut self) -> bool {
        let (Some(surface), true) = (self.surface.as_ref(), self.sized) else {
            return false;
        };
        let first_paint = self.painted_at.is_none();

        if !self.sw.parent_once || !self.parent_painted {
            surface.draw(|canvas| {
                canvas.clear(Color::TRANSPARENT);
            });
            self.parent_painted = true;
        }

        let Some(card) = self.card.as_ref() else {
            return false;
        };
        let base = card.base_surface();

        if !first_paint {
            if self.sw.engine && self.sw.damage {
                {
                    let damage = AppContext::take_layers_damage();
                    if damage.is_empty() && self.sw.skip_empty {
                        self.skipped += 1;
                        if self.sw.trace {
                            eprintln!("  paint skipped: no engine damage (#{})", self.skipped);
                        }
                        return false;
                    }
                    if !damage.is_empty() {
                        base.add_frame_damage(&[damage]);
                    }
                }
            } else if self.sw.damage {
                // No engine to ask: the field is all that ever changes.
                base.add_frame_damage(&[Rect::from_wh(CARD_W, FIELD_H)]);
            }
        }

        let engine = self.sw.engine;
        let style = self.sw.style;
        let input = &self.input;
        let drew_at = Instant::now();
        card.draw(|canvas| {
            // With no material from the compositor, the ground is the buffer's.
            canvas.clear(if style {
                Color::TRANSPARENT
            } else {
                Color::from_argb(0xF6, 0x28, 0x28, 0x2C)
            });
            if engine {
                base.render_layer_node(canvas);
            } else {
                input.render_at(canvas, CARD_W, FIELD_H);
            }
        });
        let now = Instant::now();
        self.painted_at = Some(now);
        if self.sw.trace {
            let drew = ms(now - drew_at);
            match self.key_at.take() {
                Some(key) => eprintln!(
                    "key → frame {:>5.2} ms (draw {:>5.2} ms) value={:?}",
                    ms(now - key),
                    drew,
                    self.input.value()
                ),
                None => eprintln!("  frame        (draw {drew:>5.2} ms)"),
            }
        }
        true
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl App for LauncherField {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("launcher_field: {:?}", self.sw);
        if self.sw.engine {
            AppContext::enable_layer_engine(1920.0, 1080.0);
        }

        let surface = LayerShellSurface::with_anchor(
            ShellLayer::Overlay,
            "otto-launcher-field",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            Some(0),
        )?;
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

        let card = SubsurfaceSurface::new(
            surface.base_surface().wl_surface(),
            0,
            0,
            CARD_W as i32,
            self.sw.card_h() as i32,
        )?;
        self.apply_material(&card);

        if self.sw.engine {
            let engine = AppContext::layers_renderer(|r| r.engine().clone())
                .ok_or("the layers engine is unavailable")?;
            let new_layer = |key: &str| {
                let layer = engine.new_layer();
                layer.set_key(key);
                layer.set_layout_style(layers::taffy::Style {
                    position: layers::taffy::style::Position::Absolute,
                    ..Default::default()
                });
                layer
            };
            let root = new_layer("launcher-card");
            match card.base_surface().layer_node() {
                Some(parent) => {
                    let _ = parent.add_sublayer(&root);
                }
                None => {
                    let _ = engine.add_layer(&root);
                }
            }
            let field = new_layer("launcher-field");
            let _ = root.add_sublayer(&field);
            field.set_position(LayerPoint { x: 0.0, y: 0.0 }, None);
            field.set_size(LayerSize::points(CARD_W, FIELD_H), None);
            self.field_layer = Some(field);
            self.engine = Some(engine);
        }

        self.card = Some(card);
        self.surface = Some(surface);
        self.push();
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        if let Some(engine) = self.engine.as_ref() {
            engine.scene_set_size(width as f32, height as f32);
        }
        self.place_card((width as f32, height as f32));
        self.sized = true;
        self.dirty = true;
        self.parent_painted = false;
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        if state != wl_keyboard::KeyState::Pressed {
            return;
        }
        let received = Instant::now();
        let ctrl = self.modifiers.ctrl;
        let key = match event.keysym {
            Keysym::Escape => {
                AppContext::request_exit();
                return;
            }
            Keysym::a | Keysym::A if ctrl => TextInputKey::SelectAll,
            Keysym::c | Keysym::C if ctrl => TextInputKey::Copy,
            Keysym::x | Keysym::X if ctrl => TextInputKey::Cut,
            Keysym::v | Keysym::V if ctrl => match clipboard::text() {
                Some(text) => TextInputKey::Paste(text),
                None => return,
            },
            Keysym::Left => TextInputKey::Left,
            Keysym::Right => TextInputKey::Right,
            Keysym::Home => TextInputKey::Home,
            Keysym::End => TextInputKey::End,
            Keysym::BackSpace => TextInputKey::Backspace,
            Keysym::Delete => TextInputKey::Delete,
            _ => {
                let text: String = event
                    .utf8
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if text.is_empty() || ctrl {
                    return;
                }
                TextInputKey::Text(text)
            }
        };
        let mods = KeyMods {
            shift: self.modifiers.shift,
            ctrl,
        };
        match self.input.on_key(key, mods) {
            TextInputResponse::Clipboard(text) => {
                clipboard::set_text(&text, serial);
                // Cut changed the value as well.
                self.key_at.get_or_insert(received);
                self.dirty = true;
            }
            TextInputResponse::Changed | TextInputResponse::Moved => {
                self.key_at.get_or_insert(received);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        for event in events {
            // Card-local: the card subsurface takes the pointer over itself.
            let on_card = &event.surface == card.wl_surface();
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if !on_card || y as f32 >= FIELD_H {
                        continue;
                    }
                    let now = Instant::now();
                    self.clicks = match self.last_press {
                        Some((at, px, py))
                            if now - at < MULTI_CLICK
                                && (px - x).abs() < 4.0
                                && (py - y).abs() < 4.0 =>
                        {
                            self.clicks + 1
                        }
                        _ => 1,
                    };
                    self.last_press = Some((now, x, y));
                    self.input
                        .on_pointer_down(x as f32, self.clicks, self.modifiers.shift);
                    self.dragging = true;
                    self.key_at.get_or_insert(now);
                    self.dirty = true;
                }
                PointerEventKind::Motion { .. } if self.dragging && on_card => {
                    self.input.on_pointer_drag(x as f32);
                    self.dirty = true;
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT && self.dragging => {
                    self.dragging = false;
                    self.input.on_pointer_up();
                }
                _ => {}
            }
        }
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        let now = Instant::now();
        if self.sw.blink {
            let was_visible = self.input.caret_visible();
            self.input
                .tick(now.duration_since(self.last_tick).as_secs_f32());
            if self.input.caret_visible() != was_visible {
                self.dirty = true;
            }
        }
        self.last_tick = now;

        if !self.dirty {
            return;
        }
        if self.sw.wait_frame && self.frame_in_flight() {
            return;
        }
        self.dirty = false;
        self.push();
        self.paint();
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(if self.dirty {
            // A paint held back for a frame callback, or a skipped one.
            Duration::from_millis(4)
        } else if self.sw.blink {
            Duration::from_secs_f32(CARET_BLINK_PERIOD / 2.0)
        } else {
            Duration::from_secs(1)
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    AppRunner::new(LauncherField::new(Switches::from_env())).run()?;
    Ok(())
}
