//! The Wi-Fi password dialog: a card in the middle of the screen that asks
//! for a network's password, and for a hidden network its name and how it
//! is secured.
//!
//! Three things open it:
//!
//! - picking a secured network nobody has joined before in the network menu;
//! - *Join Other Network…*, for a network that does not broadcast its name;
//! - NetworkManager asking Otto's secret agent ([`crate::secret_agent`]) for
//!   a password, which it does when a saved password stops being accepted.
//!
//! It is a layer-shell surface of its own on the overlay layer, holding the
//! keyboard while it is up, laid out from fixed sizes so the paint and the
//! hit test agree by construction (see [`Layout`]).

use otto_kit::accessibility::{Action, ActionData, ActionRequest};
use otto_kit::common::Renderable;
use otto_kit::components::label::Label;
use otto_kit::components::push_button::{self, PushButton, Role};
use otto_kit::components::segmented::{self, Segment, SegmentedInteraction};
use otto_kit::components::text_input::{
    key_for, KeyMods, TextInput, TextInputResponse, TextInputStyle,
};
use otto_kit::focus::{draw_focus_ring, FocusId};
use otto_kit::prelude::*;
use otto_kit::protocols::otto_surface_style_v1::ClipMode;
use otto_kit::skia::{Paint, PaintStyle, RRect, Rect};
use otto_kit::surfaces::LayerShellSurface;
use otto_kit::typography::styles;
use otto_network::{KeyKind, SecretRequest, WifiNetwork};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use wayland_client::Proxy;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::KeyboardInteractivity,
};

/// The card's width.
const WIDTH: f32 = 400.0;
const PAD: f32 = 22.0;
const ICON: f32 = 40.0;
const GAP: f32 = 12.0;
const LABEL_H: f32 = 18.0;
const FIELD_H: f32 = 28.0;
const CHECK: f32 = 15.0;
const RADIUS: f32 = 14.0;
/// The corner radius of the kit's push buttons and segmented track, which
/// the focus ring follows around them.
const CONTROL_RADIUS: f32 = 7.0;
/// The checkbox's corner radius.
const CHECK_RADIUS: f32 = 4.0;

/// What the dialog is asking for, and what answers it.
pub enum Purpose {
    /// A network picked in the menu: the password is all it needs.
    Join {
        device: String,
        network: WifiNetwork,
    },
    /// A network that does not broadcast its name.
    Hidden { device: String },
    /// NetworkManager asked the secret agent. `retry` is set when the
    /// password it had was refused.
    Agent {
        /// Set when NetworkManager withdraws the question.
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        request: SecretRequest,
        retry: bool,
        reply: tokio::sync::oneshot::Sender<Option<String>>,
    },
}

/// The security choices for a hidden network, in the segmented control's
/// order.
const SECURITY: [Option<KeyKind>; 4] = [
    None,
    Some(KeyKind::WpaPsk),
    Some(KeyKind::Sae),
    Some(KeyKind::Wep),
];

fn security_label(index: usize) -> &'static str {
    match index {
        0 => otto_kit::t!("bar-join-security-none"),
        1 => "WPA2",
        2 => "WPA3",
        _ => "WEP",
    }
}

/// A stop in the keyboard order. The security choices are one stop, moved
/// within by Left and Right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Name,
    Security,
    Password,
    ShowPassword,
    Cancel,
    Join,
}

/// Where a press landed, or which control an assistive technology named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Name,
    Password,
    Security(usize),
    ShowPassword,
    Cancel,
    Join,
}

impl Hit {
    /// The control's id, shared by the accessibility tree and the actions
    /// that come back against it.
    fn focus_id(self) -> FocusId {
        match self {
            Hit::Name => FocusId::new("join-name"),
            Hit::Password => FocusId::new("join-password"),
            Hit::Security(index) => FocusId::new(format!("join-security-{index}")),
            Hit::ShowPassword => FocusId::new("join-show-password"),
            Hit::Cancel => FocusId::new("join-cancel"),
            Hit::Join => FocusId::new("join-join"),
        }
    }

    /// The keyboard stop the control belongs to.
    fn focus(self) -> Focus {
        match self {
            Hit::Name => Focus::Name,
            Hit::Password => Focus::Password,
            Hit::Security(_) => Focus::Security,
            Hit::ShowPassword => Focus::ShowPassword,
            Hit::Cancel => Focus::Cancel,
            Hit::Join => Focus::Join,
        }
    }
}

/// How the dialog ended.
pub enum Outcome {
    /// Join with what was typed.
    Join {
        purpose: Purpose,
        name: Vec<u8>,
        key: Option<KeyKind>,
        password: String,
    },
    Cancel(Purpose),
}

/// Where everything on the card sits, from the dialog's shape alone.
#[derive(Debug, Clone, PartialEq)]
struct Layout {
    height: f32,
    title_y: f32,
    body: Vec<(String, f32)>,
    name_label_y: Option<f32>,
    name: Option<Rect>,
    security: Option<Rect>,
    password_label_y: Option<f32>,
    password: Option<Rect>,
    show: Option<Rect>,
    cancel: Rect,
    join: Rect,
}

impl Layout {
    fn new(body: &str, hidden: bool, wants_password: bool) -> Self {
        let text_x = PAD + ICON + GAP;
        let text_w = WIDTH - text_x - PAD;
        let font = styles::BODY.font();
        let lines = otto_kit::typography::wrap(body, text_w, |s| font.measure_str(s, None).0);
        let mut y = PAD;
        let title_y = y;
        y += 24.0;
        let mut body_lines = Vec::new();
        for line in lines {
            body_lines.push((line, y));
            y += 18.0;
        }
        y += GAP;

        let field = |y: f32| Rect::from_xywh(text_x, y, text_w, FIELD_H);
        let (mut name_label_y, mut name, mut security) = (None, None, None);
        if hidden {
            name_label_y = Some(y);
            y += LABEL_H;
            name = Some(field(y));
            y += FIELD_H + GAP;
            security = Some(Rect::from_xywh(text_x, y, text_w, segmented::HEIGHT));
            y += segmented::HEIGHT + GAP;
        }
        let (mut password_label_y, mut password, mut show) = (None, None, None);
        if wants_password {
            password_label_y = Some(y);
            y += LABEL_H;
            password = Some(field(y));
            y += FIELD_H + 8.0;
            show = Some(Rect::from_xywh(text_x, y, text_w, CHECK + 4.0));
            y += CHECK + 4.0 + GAP;
        }
        y += 4.0;

        let join_w = push_button::width(otto_kit::t!("bar-join-join"));
        let cancel_w = push_button::width(otto_kit::t!("bar-join-cancel"));
        let join = Rect::from_xywh(WIDTH - PAD - join_w, y, join_w, push_button::HEIGHT);
        let cancel = Rect::from_xywh(
            join.left - 10.0 - cancel_w,
            y,
            cancel_w,
            push_button::HEIGHT,
        );
        y += push_button::HEIGHT + PAD;

        Self {
            height: y.ceil(),
            title_y,
            body: body_lines,
            name_label_y,
            name,
            security,
            password_label_y,
            password,
            show,
            cancel,
            join,
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let inside = |r: &Rect| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
        if self.name.as_ref().is_some_and(inside) {
            return Some(Hit::Name);
        }
        if let Some(rect) = self.security {
            if let Some(index) = segmented::segment_at(rect, SECURITY.len(), x, y) {
                return Some(Hit::Security(index));
            }
        }
        if self.password.as_ref().is_some_and(inside) {
            return Some(Hit::Password);
        }
        if self.show.as_ref().is_some_and(inside) {
            return Some(Hit::ShowPassword);
        }
        if inside(&self.cancel) {
            return Some(Hit::Cancel);
        }
        if inside(&self.join) {
            return Some(Hit::Join);
        }
        None
    }

    /// Every control on the card in keyboard order, each security choice
    /// on its own.
    fn controls(&self) -> Vec<Hit> {
        let mut controls = Vec::new();
        if self.name.is_some() {
            controls.push(Hit::Name);
        }
        if self.security.is_some() {
            controls.extend((0..SECURITY.len()).map(Hit::Security));
        }
        if self.password.is_some() {
            controls.push(Hit::Password);
        }
        if self.show.is_some() {
            controls.push(Hit::ShowPassword);
        }
        controls.push(Hit::Cancel);
        controls.push(Hit::Join);
        controls
    }
}

pub struct JoinDialog {
    surface: Option<LayerShellSurface>,
    purpose: Option<Purpose>,
    name: TextInput,
    password: TextInput,
    focus: Focus,
    security: usize,
    show_password: bool,
    pressed: Option<Hit>,
}

fn field_style() -> TextInputStyle {
    let theme = AppContext::current_theme();
    let mut style = TextInputStyle::with_theme(theme.clone()).with_background(theme.fill_tertiary);
    style.focus_ring_color = theme.accent;
    style
}

impl JoinDialog {
    pub fn new(purpose: Purpose) -> Self {
        let hidden = matches!(purpose, Purpose::Hidden { .. });
        let mut name = TextInput::new(String::new(), field_style());
        name.set_size(WIDTH - 2.0 * PAD - ICON - GAP, FIELD_H);
        let mut password = TextInput::new(String::new(), field_style());
        password.set_size(WIDTH - 2.0 * PAD - ICON - GAP, FIELD_H);
        password.state.password = true;
        let focus = if hidden { Focus::Name } else { Focus::Password };
        name.state.set_focused(focus == Focus::Name);
        password.state.set_focused(focus == Focus::Password);
        Self {
            surface: None,
            purpose: Some(purpose),
            name,
            password,
            focus,
            // WPA2 is what a hidden network almost always is.
            security: 1,
            show_password: false,
            pressed: None,
        }
    }

    /// Whether NetworkManager withdrew the question this dialog asks.
    pub fn withdrawn(&self) -> bool {
        match &self.purpose {
            Some(Purpose::Agent { cancelled, .. }) => {
                cancelled.load(std::sync::atomic::Ordering::Relaxed)
            }
            _ => false,
        }
    }

    fn hidden(&self) -> bool {
        matches!(self.purpose, Some(Purpose::Hidden { .. }))
    }

    /// The kind of key the dialog asks for, `None` for an open network.
    fn key(&self) -> Option<KeyKind> {
        match &self.purpose {
            Some(Purpose::Join { network, .. }) => network.key_kind(),
            Some(Purpose::Hidden { .. }) => SECURITY[self.security],
            Some(Purpose::Agent { request, .. }) => Some(request.key),
            None => None,
        }
    }

    fn network_name(&self) -> String {
        match &self.purpose {
            Some(Purpose::Join { network, .. }) => network.name(),
            Some(Purpose::Agent { request, .. }) => otto_network::ssid_name(&request.ssid),
            _ => String::new(),
        }
    }

    fn title(&self) -> String {
        if self.hidden() {
            otto_kit::t!("bar-join-hidden-title").to_string()
        } else {
            otto_kit::t_owned!("bar-join-title", name = self.network_name())
        }
    }

    fn body(&self) -> String {
        match &self.purpose {
            Some(Purpose::Hidden { .. }) => otto_kit::t!("bar-join-hidden-body").to_string(),
            Some(Purpose::Agent { retry: true, .. }) => {
                otto_kit::t!("bar-join-body-retry").to_string()
            }
            _ => match self.key() {
                Some(KeyKind::Wep) => otto_kit::t!("bar-join-body-wep").to_string(),
                _ => otto_kit::t!("bar-join-body").to_string(),
            },
        }
    }

    fn layout(&self) -> Layout {
        Layout::new(&self.body(), self.hidden(), self.key().is_some())
    }

    /// Whether Join would do anything: a name for a hidden network, and a
    /// password the network's key can be.
    fn can_join(&self) -> bool {
        if self.hidden() && self.name.value().trim().is_empty() {
            return false;
        }
        match self.key() {
            Some(key) => key.accepts(self.password.value()),
            None => true,
        }
    }

    pub fn is_surface(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.surface
            .as_ref()
            .is_some_and(|s| s.wl_surface() == *surface)
    }

    /// Put the card on screen. False when the surface could not be made:
    /// the caller then ends the dialog, so nothing waits on a card nobody
    /// can see.
    pub fn show(&mut self) -> bool {
        let layout = self.layout();
        let surface = match LayerShellSurface::with_anchor(
            Layer::Overlay,
            "otto-network-join",
            WIDTH as u32,
            layout.height as u32,
            None,
            None,
        ) {
            Ok(surface) => surface,
            Err(e) => {
                tracing::warn!("network join dialog: {e}");
                return false;
            }
        };
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        if let Some(style) = surface.base_surface().surface_style() {
            let theme = AppContext::current_theme();
            let c = otto_kit::skia::Color4f::from(theme.material_popup);
            style.set_background_color(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
            style.set_masks_to_bounds(ClipMode::Enabled);
            style.set_corner_radius(otto_kit::corners::radius(RADIUS) as f64);
            style.set_shadow(0.35, 30.0, 0.0, 10.0, 0.0, 0.0, 0.0);
        }
        AppContext::enable_accessibility(&surface.wl_surface().id());
        self.surface = Some(surface);
        // Only the initial commit: the surface is not configured yet, so
        // nothing is painted here. The first paint is in `on_configure_layer`.
        self.redraw();
        true
    }

    /// The card, described: a dialog with its title and question, the
    /// fields (the password reported as one, never read out), the security
    /// choices and the buttons.
    pub fn accessibility(&self) -> otto_kit::accessibility::A11yTree {
        use otto_kit::accessibility::{A11yTree, Role};
        let layout = self.layout();
        let title = self.title();
        let mut tree = A11yTree::new(title.clone());
        let card = Rect::from_xywh(0.0, 0.0, WIDTH, layout.height);
        tree.region(
            FocusId::new("join-dialog"),
            card,
            Role::Dialog,
            title,
            |tree| {
                tree.label(FocusId::new("join-body"), card, self.body());
                if let Some(rect) = layout.name {
                    tree.text_field(
                        Hit::Name.focus_id(),
                        rect,
                        otto_kit::t!("bar-join-name"),
                        &self.name.state,
                        true,
                    );
                }
                if let Some(rect) = layout.security {
                    for index in 0..SECURITY.len() {
                        tree.radio(
                            Hit::Security(index).focus_id(),
                            segmented::segment_rect(rect, SECURITY.len(), index),
                            security_label(index),
                            index == self.security,
                            true,
                        );
                    }
                }
                if let Some(rect) = layout.password {
                    tree.text_field(
                        Hit::Password.focus_id(),
                        rect,
                        otto_kit::t!("bar-join-password"),
                        &self.password.state,
                        true,
                    );
                }
                if let Some(rect) = layout.show {
                    tree.checkbox(
                        Hit::ShowPassword.focus_id(),
                        rect,
                        otto_kit::t!("bar-join-show-password"),
                        self.show_password,
                        true,
                    );
                }
                tree.button(
                    Hit::Cancel.focus_id(),
                    layout.cancel,
                    otto_kit::t!("bar-join-cancel"),
                    true,
                );
                tree.button(
                    Hit::Join.focus_id(),
                    layout.join,
                    otto_kit::t!("bar-join-join"),
                    self.can_join(),
                );
            },
        );
        tree.set_focus(self.focused_control().focus_id());
        tree
    }

    /// The card's surface, for the accessibility tree's owner.
    pub fn surface_id(&self) -> Option<otto_kit::ObjectId> {
        self.surface.as_ref().map(|s| s.wl_surface().id())
    }

    fn resize(&mut self) {
        let height = self.layout().height;
        if let Some(surface) = &self.surface {
            surface.set_size(WIDTH as u32, height as u32);
        }
    }

    pub fn redraw(&mut self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        surface.draw(|canvas| self.paint(canvas));
        surface.base_surface().wl_surface().commit();
    }

    /// Paint the card at the canvas origin.
    fn paint(&self, canvas: &otto_kit::skia::Canvas) {
        let theme = AppContext::current_theme();
        let layout = self.layout();
        let title = self.title();
        let can_join = self.can_join();
        let key = self.key();
        let (name, password) = (&self.name, &self.password);
        let (security, show_password, pressed) = (self.security, self.show_password, self.pressed);
        let focus = self.focus;
        {
            canvas.clear(otto_kit::skia::Color::TRANSPARENT);
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(theme.material_popup);
            let card = Rect::from_xywh(0.0, 0.0, WIDTH, layout.height);
            canvas.draw_rrect(RRect::new_rect_xy(card, RADIUS, RADIUS), &paint);

            crate::network::draw_badge(canvas, PAD, PAD, ICON, theme.accent, key.is_some());

            let text_x = PAD + ICON + GAP;
            let text_w = WIDTH - text_x - PAD;
            let title_font = styles::HEADLINE_EMPHASIZED.font();
            Label::new(otto_kit::typography::ellipsize(&title_font, &title, text_w))
                .with_style(styles::HEADLINE_EMPHASIZED)
                .with_color(theme.text_primary)
                .centered_on(text_x, layout.title_y + 10.0)
                .render(canvas);
            for (line, y) in &layout.body {
                Label::new(line.clone())
                    .with_style(styles::BODY)
                    .with_color(theme.text_secondary)
                    .centered_on(text_x, y + 8.0)
                    .render(canvas);
            }
            let label = |canvas: &otto_kit::skia::Canvas, text: &str, y: f32| {
                Label::new(text)
                    .with_style(styles::FOOTNOTE_EMPHASIZED)
                    .with_color(theme.text_secondary)
                    .centered_on(text_x, y + LABEL_H / 2.0 - 2.0)
                    .render(canvas);
            };
            let field = |canvas: &otto_kit::skia::Canvas, input: &TextInput, rect: Rect| {
                canvas.save();
                canvas.translate((rect.left, rect.top));
                input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
            };

            if let (Some(y), Some(rect)) = (layout.name_label_y, layout.name) {
                label(canvas, otto_kit::t!("bar-join-name"), y);
                field(canvas, name, rect);
            }
            if let Some(rect) = layout.security {
                let labels: Vec<Segment<'_>> = (0..SECURITY.len())
                    .map(|i| Segment::Text(security_label(i)))
                    .collect();
                segmented::draw(
                    canvas,
                    rect,
                    &labels,
                    Some(security),
                    SegmentedInteraction::Normal,
                    &theme,
                );
                if focus == Focus::Security {
                    draw_focus_ring(canvas, rect, CONTROL_RADIUS);
                }
            }
            if let (Some(y), Some(rect)) = (layout.password_label_y, layout.password) {
                label(canvas, otto_kit::t!("bar-join-password"), y);
                field(canvas, password, rect);
            }
            if let Some(rect) = layout.show {
                let check = Rect::from_xywh(rect.left, rect.top + 2.0, CHECK, CHECK);
                let mut box_paint = Paint::default();
                box_paint.set_anti_alias(true);
                if show_password {
                    box_paint.set_color(theme.accent);
                    canvas.draw_rrect(RRect::new_rect_xy(check, 4.0, 4.0), &box_paint);
                    let mut tick = Paint::default();
                    tick.set_anti_alias(true);
                    tick.set_style(PaintStyle::Stroke);
                    tick.set_stroke_width(1.8);
                    tick.set_color(otto_kit::skia::Color::WHITE);
                    let (l, t) = (check.left, check.top);
                    canvas.draw_line((l + 3.5, t + 7.8), (l + 6.3, t + 10.6), &tick);
                    canvas.draw_line((l + 6.3, t + 10.6), (l + 11.5, t + 4.6), &tick);
                } else {
                    box_paint.set_color(theme.fill_tertiary);
                    canvas.draw_rrect(RRect::new_rect_xy(check, 4.0, 4.0), &box_paint);
                    box_paint.set_style(PaintStyle::Stroke);
                    box_paint.set_stroke_width(1.0);
                    box_paint.set_color(theme.fill_secondary);
                    canvas.draw_rrect(
                        RRect::new_rect_xy(check, CHECK_RADIUS, CHECK_RADIUS),
                        &box_paint,
                    );
                }
                if focus == Focus::ShowPassword {
                    draw_focus_ring(canvas, check, CHECK_RADIUS);
                }
                Label::new(otto_kit::t!("bar-join-show-password"))
                    .with_style(styles::BODY)
                    .with_color(theme.text_primary)
                    .centered_on(check.right + 8.0, check.center_y())
                    .render(canvas);
            }

            push_button::draw(
                canvas,
                layout.cancel,
                otto_kit::t!("bar-join-cancel"),
                PushButton::new(Role::Normal).pressed(pressed == Some(Hit::Cancel)),
                &theme,
            );
            push_button::draw(
                canvas,
                layout.join,
                otto_kit::t!("bar-join-join"),
                PushButton::new(Role::Default)
                    .enabled(can_join)
                    .pressed(pressed == Some(Hit::Join)),
                &theme,
            );
            match focus {
                Focus::Cancel => draw_focus_ring(canvas, layout.cancel, CONTROL_RADIUS),
                Focus::Join => draw_focus_ring(canvas, layout.join, CONTROL_RADIUS),
                // The text fields draw their own ring, and the rest is drawn
                // with its control above.
                Focus::Name | Focus::Security | Focus::Password | Focus::ShowPassword => {}
            }
        }
    }

    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.name.state.set_focused(focus == Focus::Name);
        self.password.state.set_focused(focus == Focus::Password);
    }

    /// The keyboard stops on the card, in order. Join is skipped while it
    /// would do nothing.
    fn focus_order(&self) -> Vec<Focus> {
        let can_join = self.can_join();
        let mut order: Vec<Focus> = Vec::new();
        for control in self.layout().controls() {
            let focus = control.focus();
            if order.last() == Some(&focus) || (focus == Focus::Join && !can_join) {
                continue;
            }
            order.push(focus);
        }
        order
    }

    /// Move the focus to the next stop, or the previous one, wrapping.
    fn move_focus(&mut self, forward: bool) {
        let order = self.focus_order();
        let len = order.len();
        let next = match order.iter().position(|f| *f == self.focus) {
            Some(index) if forward => order[(index + 1) % len],
            Some(index) => order[(index + len - 1) % len],
            None => order[0],
        };
        self.set_focus(next);
    }

    /// The control the keyboard is on, as the accessibility tree names it.
    fn focused_control(&self) -> Hit {
        match self.focus {
            Focus::Name => Hit::Name,
            Focus::Security => Hit::Security(self.security),
            Focus::Password => Hit::Password,
            Focus::ShowPassword => Hit::ShowPassword,
            Focus::Cancel => Hit::Cancel,
            Focus::Join => Hit::Join,
        }
    }

    /// Pick a security choice for a hidden network. Choosing none takes the
    /// password field away, and the focus with it when it was there.
    fn select_security(&mut self, index: usize) {
        self.security = index;
        if !self.focus_order().contains(&self.focus) {
            self.set_focus(Focus::Name);
        }
        self.resize();
    }

    fn toggle_show_password(&mut self) {
        self.show_password = !self.show_password;
        self.password.state.password = !self.show_password;
    }

    /// Act on the focused stop as Space does: a button pressed, the checkbox
    /// toggled. Returns the outcome when it ended the dialog.
    fn activate(&mut self) -> Option<Outcome> {
        match self.focus {
            Focus::Cancel => Some(self.finish(false)),
            Focus::Join => self.can_join().then(|| self.finish(true)),
            Focus::ShowPassword => {
                self.toggle_show_password();
                self.redraw();
                None
            }
            Focus::Name | Focus::Security | Focus::Password => None,
        }
    }

    /// A key was pressed. Returns the outcome when it ended the dialog.
    pub fn on_key(&mut self, event: &KeyEvent) -> Option<Outcome> {
        match event.keysym {
            Keysym::Escape => return Some(self.finish(false)),
            Keysym::Return | Keysym::KP_Enter => {
                return match self.focus {
                    Focus::Cancel | Focus::ShowPassword => self.activate(),
                    _ => self.can_join().then(|| self.finish(true)),
                };
            }
            Keysym::Tab | Keysym::ISO_Left_Tab => {
                let shift = AppContext::current_modifiers().shift;
                self.move_focus(event.keysym == Keysym::Tab && !shift);
                self.redraw();
                return None;
            }
            _ => {}
        }
        let input = match self.focus {
            Focus::Name => &mut self.name,
            Focus::Password => &mut self.password,
            Focus::Security => {
                let index = match event.keysym {
                    Keysym::Left | Keysym::KP_Left => self.security.saturating_sub(1),
                    Keysym::Right | Keysym::KP_Right => (self.security + 1).min(SECURITY.len() - 1),
                    _ => return None,
                };
                if index != self.security {
                    self.select_security(index);
                    self.redraw();
                }
                return None;
            }
            Focus::ShowPassword | Focus::Cancel | Focus::Join => {
                return match event.keysym {
                    Keysym::space | Keysym::KP_Space => self.activate(),
                    _ => None,
                };
            }
        };
        let mods: KeyMods = AppContext::current_modifiers().into();
        let (edit, mods) = key_for(event.keysym, event.utf8.as_deref(), mods)?;
        // A copy or cut comes back as `Clipboard`, and the bar has no
        // clipboard to put it on: nothing typed here, a password above all,
        // ever leaves the dialog that way, shown or not.
        let response = input.on_key(edit, mods);
        if !matches!(response, TextInputResponse::Ignored) {
            self.redraw();
        }
        None
    }

    /// An assistive technology acted on one of the card's controls. Returns
    /// the outcome when it ended the dialog.
    pub fn on_accessibility_action(&mut self, request: &ActionRequest) -> Option<Outcome> {
        let target =
            self.control_for(|id| otto_kit::accessibility::node_id(id) == request.target_node)?;
        let value = match &request.data {
            Some(ActionData::Value(value)) => Some(&**value),
            _ => None,
        };
        let outcome = self.act(request.action, target, value);
        if outcome.is_none() {
            self.redraw();
        }
        outcome
    }

    /// The control on the card whose id `matches`.
    fn control_for(&self, matches: impl Fn(FocusId) -> bool) -> Option<Hit> {
        self.layout()
            .controls()
            .into_iter()
            .find(|control| matches(control.focus_id()))
    }

    /// Do `action` on `target` as the user would by hand: focus it, click
    /// it, or for a text field set what it holds. Returns the outcome when
    /// it ended the dialog.
    fn act(&mut self, action: Action, target: Hit, value: Option<&str>) -> Option<Outcome> {
        match (action, target) {
            (Action::Focus, _) => {
                if target != Hit::Join || self.can_join() {
                    self.set_focus(target.focus());
                }
                None
            }
            (Action::SetValue, Hit::Name | Hit::Password) => {
                let value = value?;
                self.set_focus(target.focus());
                let input = if target == Hit::Name {
                    &mut self.name
                } else {
                    &mut self.password
                };
                input.set_value(value);
                None
            }
            (Action::Click, Hit::Name | Hit::Password) => {
                self.set_focus(target.focus());
                None
            }
            (Action::Click, Hit::Security(index)) => {
                self.set_focus(Focus::Security);
                self.select_security(index);
                None
            }
            (Action::Click, Hit::ShowPassword) => {
                self.toggle_show_password();
                None
            }
            (Action::Click, Hit::Cancel) => Some(self.finish(false)),
            (Action::Click, Hit::Join) => self.can_join().then(|| self.finish(true)),
            _ => None,
        }
    }

    /// A pointer press at `(x, y)` on the card.
    pub fn press(&mut self, x: f32, y: f32) {
        let hit = self.layout().hit(x, y);
        match hit {
            Some(Hit::Name) => {
                self.set_focus(Focus::Name);
                if let Some(rect) = self.layout().name {
                    self.name.on_pointer_down(x - rect.left, 1, false);
                }
            }
            Some(Hit::Password) => {
                self.set_focus(Focus::Password);
                if let Some(rect) = self.layout().password {
                    self.password.on_pointer_down(x - rect.left, 1, false);
                }
            }
            Some(Hit::Security(index)) => self.select_security(index),
            Some(Hit::ShowPassword) => self.toggle_show_password(),
            Some(Hit::Cancel) | Some(Hit::Join) => self.pressed = hit,
            None => {}
        }
        self.redraw();
    }

    /// The pointer let go at `(x, y)`: a button held and still under it acts.
    pub fn release(&mut self, x: f32, y: f32) -> Option<Outcome> {
        let held = self.pressed.take();
        let under = self.layout().hit(x, y);
        self.redraw();
        match (held, under) {
            (Some(Hit::Cancel), Some(Hit::Cancel)) => Some(self.finish(false)),
            (Some(Hit::Join), Some(Hit::Join)) if self.can_join() => Some(self.finish(true)),
            _ => None,
        }
    }

    /// End the dialog, joining or not, and take the card down.
    pub fn finish(&mut self, join: bool) -> Outcome {
        if let Some(surface) = self.surface.take() {
            surface.destroy();
        }
        let purpose = self.purpose.take().expect("a dialog ends once");
        let password = self.password.value().to_string();
        self.password.set_value(String::new());
        if join {
            Outcome::Join {
                // As typed: a network name may begin or end with a space.
                name: self.name.value().as_bytes().to_vec(),
                key: self.key_for(&purpose),
                password,
                purpose,
            }
        } else {
            Outcome::Cancel(purpose)
        }
    }

    fn key_for(&self, purpose: &Purpose) -> Option<KeyKind> {
        match purpose {
            Purpose::Join { network, .. } => network.key_kind(),
            Purpose::Hidden { .. } => SECURITY[self.security],
            Purpose::Agent { request, .. } => Some(request.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hidden_dialog_has_more_to_fill_in() {
        let plain = Layout::new("Enter the password.", false, true);
        let hidden = Layout::new("Enter the name.", true, true);
        assert!(plain.name.is_none() && plain.security.is_none());
        assert!(hidden.name.is_some() && hidden.security.is_some());
        assert!(hidden.height > plain.height);
        // No password field for an open network.
        let open = Layout::new("x", true, false);
        assert!(open.password.is_none() && open.show.is_none());
    }

    #[test]
    fn presses_land_where_things_are_drawn() {
        let layout = Layout::new("Enter the password.", true, true);
        let at = |r: Rect| layout.hit(r.center_x(), r.center_y());
        assert_eq!(at(layout.name.unwrap()), Some(Hit::Name));
        assert_eq!(at(layout.password.unwrap()), Some(Hit::Password));
        assert_eq!(at(layout.cancel), Some(Hit::Cancel));
        assert_eq!(at(layout.join), Some(Hit::Join));
        let security = layout.security.unwrap();
        assert_eq!(
            layout.hit(security.left + 1.0, security.center_y()),
            Some(Hit::Security(0))
        );
        assert_eq!(layout.hit(1.0, 1.0), None);
    }

    fn hidden() -> JoinDialog {
        JoinDialog::new(Purpose::Hidden {
            device: "/dev/0".into(),
        })
    }

    fn key(keysym: Keysym) -> KeyEvent {
        KeyEvent {
            time: 0,
            raw_code: 0,
            keysym,
            utf8: None,
        }
    }

    fn by_id(dialog: &JoinDialog, hit: Hit) -> Option<Hit> {
        dialog.control_for(|id| id == hit.focus_id())
    }

    #[test]
    fn tab_reaches_every_control_and_skips_a_join_that_would_do_nothing() {
        let mut dialog = hidden();
        let mut seen = vec![dialog.focus];
        for _ in 0..5 {
            assert!(dialog.on_key(&key(Keysym::Tab)).is_none());
            seen.push(dialog.focus);
        }
        assert_eq!(
            seen,
            [
                Focus::Name,
                Focus::Security,
                Focus::Password,
                Focus::ShowPassword,
                Focus::Cancel,
                Focus::Name,
            ]
        );

        dialog.name.set_value("cafe");
        dialog.password.set_value("12345678");
        dialog.set_focus(Focus::Cancel);
        dialog.on_key(&key(Keysym::Tab));
        assert_eq!(dialog.focus, Focus::Join);
        dialog.on_key(&key(Keysym::ISO_Left_Tab));
        assert_eq!(dialog.focus, Focus::Cancel);
    }

    #[test]
    fn arrows_pick_the_security_and_none_drops_the_password_stops() {
        let mut dialog = hidden();
        dialog.set_focus(Focus::Security);
        dialog.on_key(&key(Keysym::Right));
        assert_eq!(SECURITY[dialog.security], Some(KeyKind::Sae));
        dialog.on_key(&key(Keysym::Left));
        dialog.on_key(&key(Keysym::Left));
        dialog.on_key(&key(Keysym::Left));
        assert_eq!(dialog.security, 0);
        assert!(!dialog.focus_order().contains(&Focus::Password));
        assert!(!dialog.focus_order().contains(&Focus::ShowPassword));
    }

    #[test]
    fn space_and_return_activate_the_focused_control() {
        let mut dialog = hidden();
        dialog.set_focus(Focus::ShowPassword);
        assert!(dialog.on_key(&key(Keysym::space)).is_none());
        assert!(dialog.show_password && !dialog.password.state.password);
        assert!(dialog.on_key(&key(Keysym::Return)).is_none());
        assert!(!dialog.show_password);

        dialog.set_focus(Focus::Cancel);
        assert!(matches!(
            dialog.on_key(&key(Keysym::Return)),
            Some(Outcome::Cancel(_))
        ));
    }

    #[test]
    fn return_in_a_field_joins_and_escape_cancels() {
        let mut dialog = hidden();
        dialog.name.set_value("cafe");
        assert!(dialog.on_key(&key(Keysym::Return)).is_none());
        dialog.password.set_value("12345678");
        assert!(matches!(
            dialog.on_key(&key(Keysym::Return)),
            Some(Outcome::Join {
                key: Some(KeyKind::WpaPsk),
                ..
            })
        ));

        let mut dialog = hidden();
        dialog.set_focus(Focus::Security);
        assert!(matches!(
            dialog.on_key(&key(Keysym::Escape)),
            Some(Outcome::Cancel(_))
        ));
    }

    #[test]
    fn accessibility_actions_reach_the_dialog_controls() {
        let mut dialog = hidden();
        let wep = by_id(&dialog, Hit::Security(3)).unwrap();
        assert!(dialog.act(Action::Click, wep, None).is_none());
        assert_eq!(SECURITY[dialog.security], Some(KeyKind::Wep));
        assert_eq!(dialog.focus, Focus::Security);

        let name = by_id(&dialog, Hit::Name).unwrap();
        dialog.act(Action::SetValue, name, Some("cafe"));
        assert_eq!(dialog.name.value(), "cafe");
        assert_eq!(dialog.focus, Focus::Name);

        let password = by_id(&dialog, Hit::Password).unwrap();
        dialog.act(Action::Focus, password, None);
        assert_eq!(dialog.focus, Focus::Password);
        dialog.act(Action::SetValue, password, Some("12345"));

        let show = by_id(&dialog, Hit::ShowPassword).unwrap();
        dialog.act(Action::Click, show, None);
        assert!(dialog.show_password);

        let join = by_id(&dialog, Hit::Join).unwrap();
        assert!(matches!(
            dialog.act(Action::Click, join, None),
            Some(Outcome::Join {
                key: Some(KeyKind::Wep),
                ..
            })
        ));
    }

    #[test]
    fn accessibility_names_only_the_controls_on_the_card() {
        let dialog = JoinDialog::new(Purpose::Agent {
            cancelled: Default::default(),
            request: SecretRequest {
                ssid: b"cafe".to_vec(),
                key: KeyKind::WpaPsk,
            },
            retry: false,
            reply: tokio::sync::oneshot::channel().0,
        });
        assert_eq!(by_id(&dialog, Hit::Name), None);
        assert_eq!(by_id(&dialog, Hit::Security(1)), None);
        assert_eq!(by_id(&dialog, Hit::Cancel), Some(Hit::Cancel));
        assert_eq!(dialog.focused_control(), Hit::Password);
    }

    #[test]
    fn the_buttons_sit_inside_the_card() {
        let layout = Layout::new("Enter the password.", false, true);
        assert!(layout.join.right <= WIDTH - PAD + 0.5);
        assert!(layout.cancel.right < layout.join.left);
        assert!(layout.join.bottom < layout.height);
    }
}
