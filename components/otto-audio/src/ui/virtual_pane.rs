//! The Virtual devices section: the outputs, inputs and loopbacks the audio
//! service makes, a card each, and a form to add one.
//!
//! The section only edits the routing file; the service makes and removes
//! the devices when it reads the file again.

use otto_audio_graph::{VirtualDevice, VirtualKind};
use otto_kit::components::text_input::{
    key_for, KeyMods, TextInput, TextInputResponse, TextInputStyle,
};

use super::*;

/// The channel counts the form offers: mono, stereo, quad, 5.1 and 7.1.
const CHANNEL_CHOICES: [u32; 5] = [1, 2, 4, 6, 8];
/// The kinds the form offers, in order.
const KINDS: [VirtualKind; 3] = [
    VirtualKind::Output,
    VirtualKind::Input,
    VirtualKind::Loopback,
];
/// The form card's height.
const FORM_H: f32 = 168.0;
/// One device card.
const DEVICE_CARD_MIN_W: f32 = 260.0;
const DEVICE_CARD_H: f32 = 132.0;
/// A text field's height.
const FIELD_H: f32 = 28.0;
/// The width of a form row's label column.
const FORM_LABEL_W: f32 = 78.0;
/// The notice shown while the service is off.
const NOTICE_H: f32 = 44.0;

/// What a control in this section does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum VirtualAction {
    FocusName,
    Kind(VirtualKind),
    Channels(u32),
    Add,
    /// Start renaming the device at this index.
    Rename(usize),
    Remove(usize),
    FocusRename,
    CommitRename,
    CancelRename,
}

/// The section's own state: the form, and the rename in progress.
pub(super) struct VirtualForm {
    name: TextInput,
    kind: VirtualKind,
    channels: u32,
    /// The id of the device being renamed, and its field.
    renaming: Option<(String, TextInput)>,
}

impl Default for VirtualForm {
    fn default() -> Self {
        let mut name = TextInput::new(String::new(), field_style(&Theme::light()));
        name.state.placeholder = t!("audio-virtual-name-placeholder").to_string();
        Self {
            name,
            kind: VirtualKind::Output,
            channels: 2,
            renaming: None,
        }
    }
}

fn field_style(theme: &Theme) -> TextInputStyle {
    let mut style =
        TextInputStyle::with_theme(theme.clone()).with_background(theme.fill_quaternary);
    style.focus_ring_color = theme.accent;
    style
}

impl Ui {
    pub(super) fn draw_virtual(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        canvas.draw_rect(content, &fill(graph_view::canvas_ground(theme)));
        let left = content.left + 28.0;
        let right = content.right - 28.0;
        let width = right - left;
        let top = content.top + 24.0;

        Label::new(t!("audio-virtual-title"))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, top + 12.0)
            .render(canvas);
        let intro_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&intro_font, t!("audio-virtual-intro"), width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, top + 38.0)
            .render(canvas);

        let mut y = top + 64.0;
        if self.service_down() {
            self.draw_service_notice(
                canvas,
                Rect::from_xywh(left, y, width, NOTICE_H),
                t!("audio-virtual-service-down"),
                theme,
            );
            y += NOTICE_H + 16.0;
        }
        self.draw_virtual_form(canvas, Rect::from_xywh(left, y, width, FORM_H), theme);
        y += FORM_H + 24.0;

        card_heading(
            canvas,
            t!("audio-virtual-made-heading"),
            left,
            y + 6.0,
            theme,
        );
        y += 22.0;
        let devices = self.routing.virtual_devices().to_vec();
        if devices.is_empty() {
            caption(canvas, t!("audio-virtual-none"), left, y + 12.0, theme);
            return;
        }
        let columns = ((width + ENGINE_GAP) / (DEVICE_CARD_MIN_W + ENGINE_GAP))
            .floor()
            .clamp(1.0, 4.0) as usize;
        let card_w = (width - (columns - 1) as f32 * ENGINE_GAP) / columns as f32;
        for (index, device) in devices.iter().enumerate() {
            let (col, row) = (index % columns, index / columns);
            let card = Rect::from_xywh(
                left + col as f32 * (card_w + ENGINE_GAP),
                y + row as f32 * (DEVICE_CARD_H + ENGINE_GAP),
                card_w,
                DEVICE_CARD_H,
            );
            if card.top > content.bottom {
                break;
            }
            self.draw_virtual_card(canvas, card, index, device, theme);
        }
    }

    /// A band saying `text` (what needs the service), with the button that
    /// starts it.
    pub(super) fn draw_service_notice(
        &mut self,
        canvas: &Canvas,
        band: Rect,
        text: &str,
        theme: &Theme,
    ) {
        let tint = theme.accent_yellow;
        let rr = RRect::new_rect_xy(band, 10.0, 10.0);
        canvas.draw_rrect(
            rr,
            &fill(Color::from_argb(0x30, tint.r(), tint.g(), tint.b())),
        );
        let label = t!("audio-service-start");
        let w = push_button::width(label);
        let button = Rect::from_xywh(
            band.right - 12.0 - w,
            band.center_y() - push_button::HEIGHT / 2.0,
            w,
            push_button::HEIGHT,
        );
        let font = styles::SUBHEADLINE.font();
        let text = ellipsize(&font, text, button.left - band.left - 32.0);
        Label::new(text)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(band.left + 16.0, band.center_y())
            .render(canvas);
        self.button(canvas, button, label, Action::StartService, true, theme);
    }

    fn draw_virtual_form(&mut self, canvas: &Canvas, card: Rect, theme: &Theme) {
        let rr = RRect::new_rect_xy(card, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        let left = card.left + 20.0;
        let inner_right = card.right - 20.0;
        card_heading(
            canvas,
            t!("audio-virtual-new-heading"),
            left,
            card.top + 22.0,
            theme,
        );

        let label_font = styles::SUBHEADLINE.font();
        let row_label = |canvas: &Canvas, text: &str, cy: f32| {
            Label::new(ellipsize(&label_font, text, FORM_LABEL_W - 8.0))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(left, cy)
                .render(canvas);
        };
        let control_left = left + FORM_LABEL_W;
        let control_w = (inner_right - control_left).min(420.0);

        // Name.
        let cy = card.top + 54.0;
        row_label(canvas, t!("audio-virtual-name"), cy);
        let field = Rect::from_xywh(control_left, cy - FIELD_H / 2.0, control_w, FIELD_H);
        self.virtual_form.name.style = field_style(theme);
        self.virtual_form
            .name
            .set_size(field.width(), field.height());
        draw_field(canvas, &self.virtual_form.name, field);
        self.targets
            .push((field, Action::Virtual(VirtualAction::FocusName)));

        // Kind.
        let cy = card.top + 90.0;
        row_label(canvas, t!("audio-virtual-kind"), cy);
        let labels: Vec<String> = KINDS.iter().map(|k| kind_label(*k).to_string()).collect();
        let chosen = KINDS
            .iter()
            .position(|k| *k == self.virtual_form.kind)
            .unwrap_or(0);
        let track = Rect::from_xywh(
            control_left,
            cy - segmented::HEIGHT / 2.0,
            control_w,
            segmented::HEIGHT,
        );
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &KINDS.map(|k| Action::Virtual(VirtualAction::Kind(k))),
            theme,
        );

        // Channels.
        let cy = card.top + 126.0;
        row_label(canvas, t!("audio-virtual-channels"), cy);
        let labels: Vec<String> = CHANNEL_CHOICES.iter().map(|c| channels_label(*c)).collect();
        let chosen = CHANNEL_CHOICES
            .iter()
            .position(|c| *c == self.virtual_form.channels)
            .unwrap_or(1);
        let track = Rect::from_xywh(
            control_left,
            cy - segmented::HEIGHT / 2.0,
            control_w,
            segmented::HEIGHT,
        );
        self.segments(
            canvas,
            track,
            &labels,
            chosen,
            &CHANNEL_CHOICES.map(|c| Action::Virtual(VirtualAction::Channels(c))),
            theme,
        );

        // What the chosen kind does, and Add, beside the controls when there
        // is room and under the heading's line otherwise.
        let label = t!("audio-virtual-add");
        let w = push_button::width(label);
        let button = Rect::from_xywh(
            inner_right - w,
            card.bottom - 20.0 - push_button::HEIGHT,
            w,
            push_button::HEIGHT,
        );
        self.role_button(
            canvas,
            button,
            label,
            Action::Virtual(VirtualAction::Add),
            true,
            ButtonRole::Default,
            theme,
        );
        let about_left = control_left + control_w + 24.0;
        let about_w = button.left - 16.0 - about_left;
        if about_w > 120.0 {
            let font = styles::SUBHEADLINE.font();
            let lines = wrap(&font, kind_about(self.virtual_form.kind), about_w, 3);
            for (i, line) in lines.iter().enumerate() {
                Label::new(line)
                    .with_style(styles::SUBHEADLINE)
                    .with_color(theme.text_secondary)
                    .centered_on(about_left, card.top + 54.0 + i as f32 * 18.0)
                    .render(canvas);
            }
        }
    }

    fn draw_virtual_card(
        &mut self,
        canvas: &Canvas,
        card: Rect,
        index: usize,
        device: &VirtualDevice,
        theme: &Theme,
    ) {
        let rr = RRect::new_rect_xy(card, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        let left = card.left + 18.0;
        let width = card.width() - 36.0;

        let renaming = self
            .virtual_form
            .renaming
            .as_ref()
            .is_some_and(|(id, _)| *id == device.id);
        let name_cy = card.top + 28.0;
        if renaming {
            let field = Rect::from_xywh(left - 6.0, name_cy - FIELD_H / 2.0, width + 12.0, FIELD_H);
            if let Some((_, input)) = &mut self.virtual_form.renaming {
                input.style = field_style(theme);
                input.set_size(field.width(), field.height());
                draw_field(canvas, input, field);
            }
            self.targets
                .push((field, Action::Virtual(VirtualAction::FocusRename)));
        } else {
            let font = styles::HEADLINE.font();
            Label::new(ellipsize(&font, &device.name, width))
                .with_style(styles::HEADLINE)
                .with_color(theme.text_primary)
                .centered_on(left, name_cy)
                .render(canvas);
        }

        let what = t_owned!(
            "audio-virtual-card-kind",
            kind = kind_label(device.kind),
            channels = channels_label(device.channels)
        );
        let font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&font, &what, width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, card.top + 54.0)
            .render(canvas);

        // Its state, as the server reports its node.
        let node_state = self.graph.nodes().find(|n| device.owns(n)).map(|n| n.state);
        let (dot, state) = match node_state {
            _ if self.service_down() => {
                (theme.fill_secondary, t!("audio-virtual-state-no-service"))
            }
            None => (theme.fill_secondary, t!("audio-virtual-state-waiting")),
            Some(NodeState::Running) => (theme.accent_green, t!("audio-state-running")),
            Some(NodeState::Idle) => (theme.accent, t!("audio-state-idle")),
            Some(NodeState::Suspended) => (theme.fill_secondary, t!("audio-state-suspended")),
            Some(NodeState::Error) => (theme.accent_red, t!("audio-state-error")),
            Some(NodeState::Creating) => (theme.fill_secondary, t!("audio-state-creating")),
        };
        let cy = card.top + 76.0;
        canvas.draw_circle(Point::new(left + 4.0, cy), 4.0, &fill(dot));
        Label::new(ellipsize(&font, state, width - 14.0))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_primary)
            .centered_on(left + 14.0, cy)
            .render(canvas);

        // Rename and Remove, or Save and Cancel while renaming.
        let y = card.bottom - 16.0 - push_button::HEIGHT;
        let (first, first_action, second, second_action) = if renaming {
            (
                t!("audio-virtual-save-name"),
                VirtualAction::CommitRename,
                t!("audio-virtual-cancel"),
                VirtualAction::CancelRename,
            )
        } else {
            (
                t!("audio-virtual-rename"),
                VirtualAction::Rename(index),
                t!("audio-virtual-remove"),
                VirtualAction::Remove(index),
            )
        };
        let w1 = push_button::width(first);
        let w2 = push_button::width(second);
        let r1 = Rect::from_xywh(left, y, w1, push_button::HEIGHT);
        let r2 = Rect::from_xywh(r1.right + 8.0, y, w2, push_button::HEIGHT);
        let role = if renaming {
            ButtonRole::Default
        } else {
            ButtonRole::Normal
        };
        self.role_button(
            canvas,
            r1,
            first,
            Action::Virtual(first_action),
            true,
            role,
            theme,
        );
        self.button(
            canvas,
            r2,
            second,
            Action::Virtual(second_action),
            true,
            theme,
        );
    }

    pub(super) fn run_virtual(&mut self, action: VirtualAction) {
        let form = &mut self.virtual_form;
        match action {
            VirtualAction::FocusName => {
                form.renaming = None;
                form.name.state.set_focused(true);
            }
            VirtualAction::Kind(kind) => form.kind = kind,
            VirtualAction::Channels(channels) => form.channels = channels,
            VirtualAction::Add => self.add_virtual(),
            VirtualAction::Rename(index) => {
                let Some(device) = self.routing.virtual_devices().get(index) else {
                    return;
                };
                form.name.state.set_focused(false);
                let style = form.name.style.clone();
                form.renaming = Some((device.id.clone(), TextInput::editing(&device.name, style)));
            }
            VirtualAction::FocusRename => {
                if let Some((_, input)) = &mut form.renaming {
                    input.state.set_focused(true);
                }
                form.name.state.set_focused(false);
            }
            VirtualAction::CommitRename => self.commit_rename(),
            VirtualAction::CancelRename => form.renaming = None,
            VirtualAction::Remove(index) => {
                let Some(id) = self
                    .routing
                    .virtual_devices()
                    .get(index)
                    .map(|d| d.id.clone())
                else {
                    return;
                };
                if self.routing.remove_virtual(&id) {
                    self.save_routing_file();
                }
            }
        }
    }

    fn add_virtual(&mut self) {
        let form = &mut self.virtual_form;
        let typed = form.name.value().trim().to_string();
        let name = if typed.is_empty() {
            kind_default_name(form.kind).to_string()
        } else {
            typed
        };
        self.routing.add_virtual(&name, form.kind, form.channels);
        form.name.set_value(String::new());
        form.name.state.set_focused(false);
        self.save_routing_file();
    }

    fn commit_rename(&mut self) {
        let Some((id, input)) = self.virtual_form.renaming.take() else {
            return;
        };
        if self.routing.rename_virtual(&id, input.value()) {
            self.save_routing_file();
        }
    }

    /// Feed a key to the section's focused field. `None` when no field here
    /// takes it, so the key goes on to the window's own keys.
    pub fn text_key(&mut self, keysym: Keysym, text: Option<&str>) -> Option<bool> {
        if self.pane != Pane::VirtualDevices {
            return None;
        }
        let renaming = self
            .virtual_form
            .renaming
            .as_mut()
            .map(|(_, input)| input)
            .filter(|input| input.state.focused());
        let (input, rename) = match renaming {
            Some(input) => (input, true),
            None if self.virtual_form.name.state.focused() => (&mut self.virtual_form.name, false),
            None => return None,
        };
        let mods: KeyMods = self.modifiers.into();
        let (edit, mods) = key_for(keysym, text, mods)?;
        match input.on_key(edit, mods) {
            TextInputResponse::Ignored => return None,
            TextInputResponse::Commit if rename => self.commit_rename(),
            TextInputResponse::Commit => self.add_virtual(),
            TextInputResponse::Cancel if rename => self.virtual_form.renaming = None,
            TextInputResponse::Cancel => self.virtual_form.name.state.set_focused(false),
            // No clipboard here; a copy has nowhere to go.
            TextInputResponse::Clipboard(_)
            | TextInputResponse::Changed
            | TextInputResponse::Moved => {}
        }
        Some(true)
    }

    /// Fill the form and the list for the `--png` preview, without writing
    /// the routing file.
    pub fn preview_virtual(&mut self) {
        if !self.routing.virtual_devices().is_empty() {
            return;
        }
        self.routing
            .add_virtual(t!("audio-virtual-sample-output"), VirtualKind::Output, 2);
        self.routing
            .add_virtual(t!("audio-virtual-sample-input"), VirtualKind::Input, 1);
        self.routing.add_virtual(
            t!("audio-virtual-sample-loopback"),
            VirtualKind::Loopback,
            2,
        );
    }
}

/// Draw `input` in `rect`.
pub(super) fn draw_field(canvas: &Canvas, input: &TextInput, rect: Rect) {
    canvas.save();
    canvas.translate((rect.left, rect.top));
    input.render_at(canvas, rect.width(), rect.height());
    canvas.restore();
}

fn kind_label(kind: VirtualKind) -> &'static str {
    match kind {
        VirtualKind::Output => t!("audio-virtual-kind-output"),
        VirtualKind::Input => t!("audio-virtual-kind-input"),
        VirtualKind::Loopback => t!("audio-virtual-kind-loopback"),
    }
}

/// The name a device gets when none is typed.
fn kind_default_name(kind: VirtualKind) -> &'static str {
    match kind {
        VirtualKind::Output => t!("audio-role-virtual-output"),
        VirtualKind::Input => t!("audio-role-virtual-input"),
        VirtualKind::Loopback => t!("audio-virtual-kind-loopback"),
    }
}

fn kind_about(kind: VirtualKind) -> &'static str {
    match kind {
        VirtualKind::Output => t!("audio-virtual-about-output"),
        VirtualKind::Input => t!("audio-virtual-about-input"),
        VirtualKind::Loopback => t!("audio-virtual-about-loopback"),
    }
}

fn channels_label(channels: u32) -> String {
    match channels {
        1 => t!("audio-virtual-mono").to_string(),
        2 => t!("audio-virtual-stereo").to_string(),
        4 => t!("audio-virtual-quad").to_string(),
        6 => t!("audio-virtual-surround-51").to_string(),
        8 => t!("audio-virtual-surround-71").to_string(),
        n => t_owned!("audio-channels", count = n),
    }
}

/// `text` broken into at most `max` lines of `width`, the last ellipsized.
pub(super) fn wrap(font: &otto_kit::skia::Font, text: &str, width: f32, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if font.measure_str(&candidate, None).0 <= width || line.is_empty() {
            line = candidate;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > max {
        let rest = lines[max - 1..].join(" ");
        lines.truncate(max - 1);
        lines.push(ellipsize(font, &rest, width).to_string());
    }
    lines
}
