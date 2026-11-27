//! The Mixer section: where sound plays and how loud, by device and by app.
//!
//! Outputs are cards with their volume and the tuning on them; pressing an
//! output's name makes it the default. Under them every app stream playing
//! now has its own row, with its volume and an output picker that moves the
//! stream at once and can keep the choice as a rule (`[[rule]]` in the
//! routing file, applied by the service when the app plays again). Inputs
//! show which apps record from them, and the During calls card sets how
//! much other apps are lowered while one uses the microphone.
//!
//! Volumes are the session manager's, set through `otto-audio-graph` as the
//! top bar's sound menu sets them, so a change shows everywhere.

// Rust guideline compliant 2026-02-21

use otto_audio_graph::{DeviceKind, Endpoint, Node, Stream, Volume, EFFECT_PREFIX};
use otto_kit::components::dropdown::{field as dropdown, DropdownInteraction};
use otto_kit::components::slider::{self, SliderInteraction};
use otto_kit::components::toggle::{self, ToggleInteraction};

use super::*;

/// The section's margin and the room between its cards.
const PAD: f32 = 28.0;
const GAP: f32 = 16.0;
/// The title and intro above the cards, which stay put while they scroll.
const HEADER_H: f32 = 72.0;
/// A group's heading over its cards.
const HEADING_H: f32 = 34.0;
/// The narrowest a device card goes before the grid drops a column.
const DEVICE_CARD_MIN_W: f32 = 300.0;
/// A device card: its name and kind, its volume, and its last line.
const DEVICE_CARD_H: f32 = 132.0;
/// A card's inner margin.
const CARD_PAD: f32 = 18.0;
/// One app's row in the Apps playing card.
const APP_ROW_H: f32 = 60.0;
/// An app's icon.
const APP_ICON: f32 = 30.0;
/// The output picker of an app's row, and its volume slider.
const PICKER_W: f32 = 210.0;
const APP_SLIDER_W: f32 = 150.0;
const MIN_APP_SLIDER_W: f32 = 60.0;
/// The least room an app's name keeps before its slider shrinks.
const MIN_NAME_W: f32 = 120.0;
/// The mute button beside a slider.
const MUTE_W: f32 = 26.0;
/// The room a slider's percentage takes after it.
const READOUT_W: f32 = 48.0;
/// The During calls card.
const CALLS_H: f32 = 108.0;
/// The service notice, shown while rules and lowering cannot work.
const NOTICE_H: f32 = 44.0;
/// How close a volume set by hand must come to what the window last asked
/// for to count as it; the server rounds through its cubic volume.
const LIVE_TOLERANCE: f32 = 0.02;

/// What a control of this section does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MixerAction {
    /// Make the output or input node `id` the default.
    Default(u32),
    /// Mute or unmute node `id`, a device or a stream.
    Mute(u32),
    /// The volume slider of node `id`, a device or a stream: dragged.
    Volume(u32),
    /// Open the output picker of stream `id`.
    OutputMenu(u32),
    /// Move stream `stream` onto output node `to`, or to follow the default
    /// with `None`.
    Move { stream: u32, to: Option<u32> },
    /// Keep where stream `id`'s app plays now as its rule, or forget it.
    AlwaysHere(u32),
    /// Turn lowering other apps during calls on or off.
    CallsToggle,
    /// The During calls level slider: dragged.
    CallsLevel,
}

/// A slider dragged in this section.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MixerDrag {
    Volume { node: u32, track: Rect },
    CallsLevel { track: Rect },
}

/// The section's own state.
pub(super) struct MixerState {
    scroll: ScrollView,
    drag: Option<MixerDrag>,
    /// A volume asked for and maybe not reported yet: node and value, so
    /// the slider follows the pointer rather than the server's echo.
    live: Option<(u32, f32)>,
    /// The During calls level was changed and is not written yet.
    unsaved: bool,
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            scroll: ScrollView::new(Rect::from_xywh(0.0, 0.0, 0.0, 0.0)),
            drag: None,
            live: None,
            unsaved: false,
        }
    }
}

impl MixerState {
    pub(super) fn is_animating(&self) -> bool {
        self.scroll.is_animating()
    }
}

/// One output or input, as its card shows it.
struct DeviceRow {
    id: u32,
    title: String,
    kind: &'static str,
    default: bool,
    volume: Option<Volume>,
    /// The tuning line of an output, or the apps recording from an input.
    line: String,
}

impl Ui {
    fn mixer_viewport(&self) -> Rect {
        let content = self.content();
        Rect::from_ltrb(
            content.left,
            content.top + HEADER_H,
            content.right,
            content.bottom,
        )
    }

    /// Outputs or inputs people choose between: devices and virtual
    /// devices, not Otto's effects, which show as an output's tuning.
    fn mixer_devices(&self, endpoint: Endpoint) -> Vec<DeviceRow> {
        let default = self.graph.default_name(endpoint);
        self.graph
            .endpoints(endpoint)
            .into_iter()
            .filter(|n| !n.name().starts_with(EFFECT_PREFIX))
            .map(|node| DeviceRow {
                id: node.id,
                title: node.title().to_string(),
                kind: kind_label(node.device_kind()),
                default: Some(node.name()) == default,
                volume: self.graph.volume(node.id).cloned(),
                line: match endpoint {
                    Endpoint::Output => self.tuning_line(node.name()),
                    Endpoint::Input => self.recorders_line(node.name()),
                },
            })
            .collect()
    }

    /// The tuning on an output: its effects as the sound meets them, or
    /// that it has none.
    pub(super) fn tuning_line(&self, device: &str) -> String {
        let chain = self.routing.output_chain(device);
        if chain.is_empty() {
            return t!("audio-mixer-no-tuning").to_string();
        }
        chain
            .iter()
            .map(|e| {
                if e.enabled {
                    e.name.clone()
                } else {
                    t_owned!("audio-mixer-tuning-off", name = e.name.clone())
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The apps recording from input `device`, directly or through the
    /// effects on it.
    fn recorders_line(&self, device: &str) -> String {
        let mut apps: Vec<String> = Vec::new();
        for stream in self.graph.streams(Endpoint::Input) {
            let records_here = stream.linked.iter().any(|id| {
                self.graph
                    .node(*id)
                    .is_some_and(|n| self.device_under(n.name()) == device)
            });
            if records_here && !apps.contains(&stream.app) {
                apps.push(stream.app.clone());
            }
        }
        if apps.is_empty() {
            t!("audio-mixer-not-used").to_string()
        } else {
            t_owned!("audio-mixer-used-by", apps = apps.join(", "))
        }
    }

    /// The device under node `name`: the device an effect chain ends on,
    /// or `name` itself when it is no effect.
    pub(super) fn device_under(&self, name: &str) -> String {
        let mut at = name.to_string();
        // Bounded, so a hand-edited file that loops cannot hang the window.
        for _ in 0..8 {
            match self.routing.effects().iter().find(|e| e.node_name() == at) {
                Some(effect) => at.clone_from(&effect.device),
                None => break,
            }
        }
        at
    }

    /// The output stream `stream` plays on now, past any effects: by its
    /// links, else where it was asked to go.
    fn output_of(&self, stream: &Stream) -> Option<&Node> {
        let name = stream
            .linked
            .iter()
            .find_map(|id| self.graph.node(*id))
            .map(|n| n.name().to_string())
            .or_else(|| {
                let target = stream.target.as_deref()?;
                Some(self.graph.node_targeted(target)?.name().to_string())
            })?;
        let device = self.device_under(&name);
        self.graph
            .nodes()
            .find(|n| n.name() == device && Endpoint::Output.matches(n))
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    pub(super) fn draw_mixer(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        canvas.draw_rect(content, &fill(graph_view::canvas_ground(theme)));
        let left = content.left + PAD;
        let right = content.right - PAD;
        let width = (right - left).max(1.0);
        let top = content.top + 24.0;
        Label::new(t!("audio-mixer-title"))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, top + 12.0)
            .render(canvas);
        let intro_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&intro_font, t!("audio-mixer-intro"), width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, top + 38.0)
            .render(canvas);

        let viewport = self.mixer_viewport();
        let outputs = self.mixer_devices(Endpoint::Output);
        let inputs = self.mixer_devices(Endpoint::Input);
        let streams = self.graph.streams(Endpoint::Output);
        let columns = ((width + GAP) / (DEVICE_CARD_MIN_W + GAP)).floor().max(1.0) as usize;
        let grid_h = |count: usize| -> f32 {
            let rows = count.div_ceil(columns).max(1) as f32;
            rows * (DEVICE_CARD_H + GAP)
        };
        let notice_h = if self.service_down() {
            NOTICE_H + GAP
        } else {
            0.0
        };
        let apps_h = streams.len().max(1) as f32 * APP_ROW_H + 8.0;
        let content_h = 4.0
            + notice_h
            + HEADING_H
            + grid_h(outputs.len())
            + HEADING_H
            + apps_h
            + GAP
            + HEADING_H
            + grid_h(inputs.len())
            + HEADING_H
            + CALLS_H
            + PAD;
        self.mixer.scroll.set_viewport(viewport);
        self.mixer.scroll.set_content_length(content_h);
        self.mixer.scroll.tick();
        let mut y = viewport.top - self.mixer.scroll.offset() + 4.0;

        let first_target = self.targets.len();
        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));
        if self.service_down() {
            self.draw_service_notice(
                canvas,
                Rect::from_xywh(left, y, width, NOTICE_H),
                t!("audio-mixer-service-down"),
                theme,
            );
            y += NOTICE_H + GAP;
        }

        group_heading(canvas, t!("audio-mixer-outputs"), left, y, theme);
        y += HEADING_H;
        y = self.draw_device_grid(canvas, &outputs, left, width, columns, y, theme);
        if outputs.is_empty() {
            caption(
                canvas,
                t!("audio-mixer-no-outputs"),
                left,
                y - DEVICE_CARD_H / 2.0,
                theme,
            );
        }

        group_heading(canvas, t!("audio-mixer-apps"), left, y, theme);
        y += HEADING_H;
        let card = Rect::from_xywh(left, y, width, apps_h);
        self.draw_apps(canvas, card, &streams, theme);
        y += apps_h + GAP;

        group_heading(canvas, t!("audio-mixer-inputs"), left, y, theme);
        y += HEADING_H;
        y = self.draw_device_grid(canvas, &inputs, left, width, columns, y, theme);
        if inputs.is_empty() {
            caption(
                canvas,
                t!("audio-mixer-no-inputs"),
                left,
                y - DEVICE_CARD_H / 2.0,
                theme,
            );
        }

        group_heading(canvas, t!("audio-mixer-calls-heading"), left, y, theme);
        y += HEADING_H;
        self.draw_calls(canvas, Rect::from_xywh(left, y, width, CALLS_H), theme);
        canvas.restore();

        // The cards scroll: only what of their controls is in view presses.
        let drawn: Vec<(Rect, Action)> = self.targets.drain(first_target..).collect();
        for (rect, action) in drawn {
            let mut visible = rect;
            if visible.intersect(viewport) {
                self.targets.push((visible, action));
            }
        }
        self.mixer.scroll.render(canvas, theme, |_, _| {});
    }

    /// A grid of device cards from `y`; returns where the next group starts.
    #[expect(clippy::too_many_arguments, reason = "the grid's place and size")]
    fn draw_device_grid(
        &mut self,
        canvas: &Canvas,
        devices: &[DeviceRow],
        left: f32,
        width: f32,
        columns: usize,
        y: f32,
        theme: &Theme,
    ) -> f32 {
        let card_w = (width - (columns - 1) as f32 * GAP) / columns as f32;
        for (index, device) in devices.iter().enumerate() {
            let (row, col) = (index / columns, index % columns);
            let rect = Rect::from_xywh(
                left + col as f32 * (card_w + GAP),
                y + row as f32 * (DEVICE_CARD_H + GAP),
                card_w,
                DEVICE_CARD_H,
            );
            self.draw_device_row(canvas, rect, device, theme);
        }
        let rows = devices.len().div_ceil(columns).max(1) as f32;
        y + rows * (DEVICE_CARD_H + GAP)
    }

    fn draw_device_row(&mut self, canvas: &Canvas, rect: Rect, device: &DeviceRow, theme: &Theme) {
        draw_card(canvas, rect, theme);
        let left = rect.left + CARD_PAD;
        let right = rect.right - CARD_PAD;

        // The name, which makes it the default when pressed.
        let badge = device.default.then(|| t!("audio-mixer-default"));
        let badge_w = badge.map_or(0.0, |b| {
            styles::CAPTION_1_EMPHASIZED.font().measure_str(b, None).0 + 14.0
        });
        let title_font = styles::HEADLINE_EMPHASIZED.font();
        let title = ellipsize(
            &title_font,
            &device.title,
            (right - left - badge_w - 8.0).max(0.0),
        );
        let title_w = title_font.measure_str(&title, None).0;
        let cy = rect.top + 26.0;
        Label::new(title)
            .with_style(styles::HEADLINE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, cy)
            .render(canvas);
        if let Some(badge) = badge {
            let pill = Rect::from_xywh(left + title_w + 8.0, cy - 9.0, badge_w, 18.0);
            canvas.draw_rrect(RRect::new_rect_xy(pill, 9.0, 9.0), &fill(theme.accent));
            Label::new(badge)
                .with_style(styles::CAPTION_1_EMPHASIZED)
                .with_color(Color::WHITE)
                .centered_at(pill.center_x(), pill.center_y())
                .render(canvas);
        } else {
            self.targets.push((
                Rect::from_xywh(left - 4.0, cy - 12.0, title_w + 8.0, 24.0),
                Action::Mixer(MixerAction::Default(device.id)),
            ));
        }
        let kind_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&kind_font, device.kind, right - left))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, rect.top + 46.0)
            .render(canvas);

        let row = Rect::from_ltrb(left, rect.top + 64.0, right, rect.top + 92.0);
        self.volume_controls(canvas, row, device.id, device.volume.as_ref(), theme);

        Label::new(ellipsize(&kind_font, &device.line, right - left))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, rect.top + 110.0)
            .render(canvas);
    }

    /// A mute button, a volume slider and its percentage across `row`.
    fn volume_controls(
        &mut self,
        canvas: &Canvas,
        row: Rect,
        node: u32,
        volume: Option<&Volume>,
        theme: &Theme,
    ) {
        let cy = row.center_y();
        let muted = volume.is_some_and(|v| v.mute);
        let mute = Rect::from_xywh(row.left, cy - MUTE_W / 2.0, MUTE_W, MUTE_W);
        draw_speaker(
            canvas,
            mute,
            muted,
            self.pressed == Some(Action::Mixer(MixerAction::Mute(node))),
            theme,
        );
        if volume.is_some() {
            self.targets
                .push((mute, Action::Mixer(MixerAction::Mute(node))));
        }
        let track = Rect::from_ltrb(
            mute.right + 12.0 + slider::KNOB_RADIUS,
            cy - 10.0,
            row.right - READOUT_W - slider::KNOB_RADIUS,
            cy + 10.0,
        );
        let value = match self.mixer.live {
            Some((live, value)) if live == node => value,
            _ => volume.map_or(0.0, Volume::linear),
        };
        let interaction = if volume.is_none() {
            SliderInteraction::Disabled
        } else if matches!(self.mixer.drag, Some(MixerDrag::Volume { node: n, .. }) if n == node) {
            SliderInteraction::Pressed
        } else {
            SliderInteraction::Normal
        };
        if muted {
            canvas.save_layer_alpha(None, 0x80);
        }
        slider::draw(canvas, track, value, 0.0, 1.0, None, interaction, theme);
        if muted {
            canvas.restore();
        }
        let readout = if volume.is_some() {
            t_owned!(
                "audio-effects-percent",
                value = (value * 100.0).round() as u32
            )
        } else {
            "–".to_string()
        };
        let font = styles::SUBHEADLINE.font();
        let w = font.measure_str(&readout, None).0;
        Label::new(readout)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(row.right - w, cy)
            .render(canvas);
        if volume.is_some() {
            self.targets.push((
                track.with_outset((slider::KNOB_RADIUS, 6.0)),
                Action::Mixer(MixerAction::Volume(node)),
            ));
        }
    }

    fn draw_apps(&mut self, canvas: &Canvas, card: Rect, streams: &[Stream], theme: &Theme) {
        draw_card(canvas, card, theme);
        let left = card.left + CARD_PAD;
        let right = card.right - CARD_PAD;
        if streams.is_empty() {
            caption(
                canvas,
                t!("audio-mixer-nothing-playing"),
                left,
                card.top + 4.0 + APP_ROW_H / 2.0,
                theme,
            );
            return;
        }
        let scale = canvas.local_to_device_as_3x3().scale_x().abs().max(1.0);
        for (index, stream) in streams.iter().enumerate() {
            let top = card.top + 4.0 + index as f32 * APP_ROW_H;
            let cy = top + APP_ROW_H / 2.0;
            if index > 0 {
                canvas.draw_line(
                    Point::new(left, top),
                    Point::new(right, top),
                    &stroke(theme.hairline, 1.0),
                );
            }
            let icon = Rect::from_xywh(left, cy - APP_ICON / 2.0, APP_ICON, APP_ICON);
            draw_app_icon(canvas, icon, stream, scale, theme);

            // From the right: the picker, the volume, then the names.
            let picker = Rect::from_xywh(
                right - PICKER_W,
                cy - dropdown::HEIGHT / 2.0,
                PICKER_W,
                dropdown::HEIGHT,
            );
            let menu = Action::Mixer(MixerAction::OutputMenu(stream.id));
            let interaction = if self.menu_open == Some(menu) {
                DropdownInteraction::Open
            } else if self.pressed == Some(menu) {
                DropdownInteraction::Pressed
            } else {
                DropdownInteraction::Normal
            };
            dropdown::draw(
                canvas,
                picker,
                &self.picker_label(stream),
                interaction,
                theme,
            );
            self.targets.push((picker, menu));

            // The slider gives way first in a narrow window, so the app's
            // name keeps room to be read.
            let fixed = MUTE_W + 12.0 + READOUT_W + 2.0 * slider::KNOB_RADIUS;
            let spare = picker.left - 20.0 - fixed - (icon.right + 12.0 + MIN_NAME_W);
            let slider_w = spare.clamp(MIN_APP_SLIDER_W, APP_SLIDER_W);
            let volume_row = Rect::from_xywh(
                picker.left - 20.0 - fixed - slider_w,
                cy - 14.0,
                fixed + slider_w,
                28.0,
            );
            self.volume_controls(canvas, volume_row, stream.id, stream.volume.as_ref(), theme);

            let text_left = icon.right + 12.0;
            let text_w = (volume_row.left - 16.0 - text_left).max(0.0);
            let name_font = styles::BODY_EMPHASIZED.font();
            let sub_font = styles::SUBHEADLINE.font();
            let two_lines = stream.media.is_some();
            Label::new(ellipsize(&name_font, &stream.app, text_w))
                .with_style(styles::BODY_EMPHASIZED)
                .with_color(theme.text_primary)
                .centered_on(text_left, if two_lines { cy - 9.0 } else { cy })
                .render(canvas);
            if let Some(media) = &stream.media {
                Label::new(ellipsize(&sub_font, media, text_w))
                    .with_style(styles::SUBHEADLINE)
                    .with_color(theme.text_secondary)
                    .centered_on(text_left, cy + 10.0)
                    .render(canvas);
            }
        }
    }

    /// What a stream's output picker shows: its output, or the default's
    /// while it follows the default.
    fn picker_label(&self, stream: &Stream) -> String {
        let output = self.output_of(stream).map(|n| n.title().to_string());
        match (&stream.target, output) {
            (Some(_), Some(title)) => title,
            (None, Some(title)) => t_owned!("audio-mixer-default-output", name = title),
            (_, None) => t!("audio-mixer-follow-default").to_string(),
        }
    }

    fn draw_calls(&mut self, canvas: &Canvas, card: Rect, theme: &Theme) {
        draw_card(canvas, card, theme);
        let left = card.left + CARD_PAD;
        let right = card.right - CARD_PAD;
        let calls = &self.routing.calls;
        let on = calls.lower_others;
        let switch = Rect::from_xywh(
            right - toggle::WIDTH,
            card.top + 28.0 - toggle::HEIGHT / 2.0,
            toggle::WIDTH,
            toggle::HEIGHT,
        );
        let action = Action::Mixer(MixerAction::CallsToggle);
        let interaction = if self.pressed == Some(action) {
            ToggleInteraction::Pressed
        } else {
            ToggleInteraction::Normal
        };
        toggle::draw(
            canvas,
            switch,
            toggle::knob_fraction_for(on),
            interaction,
            theme,
        );
        self.targets.push((switch, action));
        let font = styles::BODY.font();
        Label::new(ellipsize(
            &font,
            t!("audio-mixer-calls-lower"),
            switch.left - left - 12.0,
        ))
        .with_style(styles::BODY)
        .with_color(theme.text_primary)
        .centered_on(left, card.top + 28.0)
        .render(canvas);

        let cy = card.top + 70.0;
        let track = Rect::from_ltrb(
            left + slider::KNOB_RADIUS,
            cy - 10.0,
            (right - READOUT_W - slider::KNOB_RADIUS).min(left + 360.0),
            cy + 10.0,
        );
        let level = calls.fraction();
        let interaction = if !on {
            SliderInteraction::Disabled
        } else if matches!(self.mixer.drag, Some(MixerDrag::CallsLevel { .. })) {
            SliderInteraction::Pressed
        } else {
            SliderInteraction::Normal
        };
        slider::draw(canvas, track, level, 0.0, 1.0, None, interaction, theme);
        Label::new(t_owned!(
            "audio-effects-percent",
            value = calls.level.min(100)
        ))
        .with_style(styles::SUBHEADLINE)
        .with_color(if on {
            theme.text_secondary
        } else {
            theme.text_tertiary
        })
        .centered_on(track.right + slider::KNOB_RADIUS + 12.0, cy)
        .render(canvas);
        if on {
            self.targets.push((
                track.with_outset((slider::KNOB_RADIUS, 6.0)),
                Action::Mixer(MixerAction::CallsLevel),
            ));
        }
    }

    // -----------------------------------------------------------------------
    // Pointer
    // -----------------------------------------------------------------------

    /// Open the output picker `action` asks for; `false` when it asks for
    /// none.
    pub(super) fn open_mixer_menu(&mut self, action: Action, rect: Rect, serial: u32) -> bool {
        let Action::Mixer(MixerAction::OutputMenu(id)) = action else {
            return false;
        };
        let Some(stream) = self
            .graph
            .streams(Endpoint::Output)
            .into_iter()
            .find(|s| s.id == id)
        else {
            return true;
        };
        let outputs = self.mixer_devices(Endpoint::Output);
        let current = self.output_of(&stream).map(|n| n.id);
        let mut options = vec![t!("audio-mixer-follow-default").to_string()];
        let mut choices = vec![Action::Mixer(MixerAction::Move {
            stream: id,
            to: None,
        })];
        let mut selected = stream.target.is_none().then_some(0);
        for output in &outputs {
            if stream.target.is_some() && current == Some(output.id) {
                selected = Some(options.len());
            }
            options.push(output.title.clone());
            choices.push(Action::Mixer(MixerAction::Move {
                stream: id,
                to: Some(output.id),
            }));
        }
        let always = t_owned!("audio-mixer-always-here", app = stream.app.clone());
        let ruled = self.routing.rule(&stream.app).is_some();
        options.push(if ruled {
            format!("✓ {always}")
        } else {
            always
        });
        choices.push(Action::Mixer(MixerAction::AlwaysHere(id)));
        self.menu_choices = choices;
        self.menu_open = Some(action);
        self.menu_request = Some(MenuRequest {
            rect,
            serial,
            options,
            selected,
        });
        true
    }

    /// Start a slider drag for `action` pressed at `p`; `false` for a
    /// control that acts on release instead.
    pub(super) fn press_mixer(&mut self, action: Action, rect: Rect, p: Point) -> bool {
        let track = rect.with_inset((slider::KNOB_RADIUS, 6.0));
        let drag = match action {
            Action::Mixer(MixerAction::Volume(node)) => MixerDrag::Volume { node, track },
            Action::Mixer(MixerAction::CallsLevel) => MixerDrag::CallsLevel { track },
            _ => return false,
        };
        self.mixer.drag = Some(drag);
        self.drag_mixer(p);
        true
    }

    /// Follow a slider drag of this section; `false` when there is none.
    pub(super) fn drag_mixer(&mut self, p: Point) -> bool {
        match self.mixer.drag {
            Some(MixerDrag::Volume { node, track }) => {
                let value = slider::value_at(track, 0.0, 1.0, None, p.x);
                self.set_node_volume(node, value);
                true
            }
            Some(MixerDrag::CallsLevel { track }) => {
                let value = slider::value_at(track, 0.0, 1.0, Some(0.05), p.x);
                self.routing.calls.level = (value * 100.0).round() as u32;
                self.mixer.unsaved = true;
                true
            }
            None => false,
        }
    }

    /// End a slider drag of this section; `true` when one ended.
    pub(super) fn release_mixer(&mut self) -> bool {
        if self.mixer.drag.take().is_none() {
            return false;
        }
        if std::mem::take(&mut self.mixer.unsaved) {
            self.save_routing_file();
        }
        true
    }

    /// Scroll over the section: a volume slider steps, anything else
    /// scrolls the cards.
    pub(super) fn scroll_mixer(&mut self, p: Point, dy: f32, discrete: bool, stop: bool) -> bool {
        if let Some(Action::Mixer(MixerAction::Volume(node))) = self.target_at(p) {
            if stop {
                return false;
            }
            let current = self.graph.volume(node).map_or(0.0, Volume::linear);
            // A wheel notch steps 5 %, as the top bar's indicator does.
            let step = if discrete {
                0.05 * dy.signum()
            } else {
                dy / 400.0
            };
            self.set_node_volume(node, (current - step).clamp(0.0, 1.0));
            return true;
        }
        if !self.mixer_viewport().contains(p) {
            return false;
        }
        if stop {
            self.mixer.scroll.on_wheel_end();
            return true;
        }
        if discrete {
            self.mixer.scroll.on_wheel_discrete(dy)
        } else {
            self.mixer.scroll.on_wheel(dy)
        }
    }

    fn set_node_volume(&mut self, node: u32, value: f32) {
        let current = self.graph.volume(node).map(Volume::linear);
        if current.is_some_and(|c| (c - value).abs() < LIVE_TOLERANCE / 4.0) {
            return;
        }
        self.mixer.live = Some((node, value));
        self.connection.set_volume(node, value);
    }

    /// Forget a value asked for once the server reports one.
    pub(super) fn settle_mixer(&mut self, node: u32) {
        if self.mixer.drag.is_none() && self.mixer.live.is_some_and(|(n, _)| n == node) {
            self.mixer.live = None;
        }
    }

    pub(super) fn run_mixer(&mut self, action: MixerAction) {
        match action {
            MixerAction::Default(id) => {
                let Some(node) = self.graph.node(id) else {
                    return;
                };
                let endpoint = if Endpoint::Output.matches(node) {
                    Endpoint::Output
                } else {
                    Endpoint::Input
                };
                self.connection.set_default(endpoint, node.name());
            }
            MixerAction::Mute(id) => {
                let muted = self.graph.volume(id).is_some_and(|v| v.mute);
                self.connection.set_mute(id, !muted);
            }
            MixerAction::Move { stream, to } => self.move_stream(stream, to),
            MixerAction::AlwaysHere(id) => {
                let Some(stream) = self
                    .graph
                    .streams(Endpoint::Output)
                    .into_iter()
                    .find(|s| s.id == id)
                else {
                    return;
                };
                let changed = if self.routing.rule(&stream.app).is_some() {
                    self.routing.remove_rule(&stream.app)
                } else {
                    let output = stream
                        .target
                        .as_ref()
                        .and_then(|_| self.output_of(&stream))
                        .map(|n| n.name().to_string());
                    self.routing.set_rule(&stream.app, output)
                };
                if changed {
                    self.save_routing_file();
                }
            }
            MixerAction::CallsToggle => {
                self.routing.calls.lower_others = !self.routing.calls.lower_others;
                self.save_routing_file();
            }
            // Dragged, or opened on the press.
            MixerAction::Volume(_) | MixerAction::CallsLevel | MixerAction::OutputMenu(_) => {}
        }
    }

    /// Move `stream` onto output node `to` now, through the tuning on it,
    /// or back to following the default. An app with a rule keeps its rule
    /// in step, so the tick in the picker stays true.
    fn move_stream(&mut self, stream: u32, to: Option<u32>) {
        let output = to
            .and_then(|id| self.graph.node(id))
            .map(|n| n.name().to_string());
        let target = output
            .as_deref()
            .map(|name| self.routing.output_entry(name));
        self.connection.set_target(stream, target.as_deref());
        let app = self
            .graph
            .streams(Endpoint::Output)
            .into_iter()
            .find(|s| s.id == stream)
            .map(|s| s.app);
        if let Some(app) = app {
            if self.routing.rule(&app).is_some() && self.routing.set_rule(&app, output) {
                self.save_routing_file();
            }
        }
    }
}

/// A group's heading over its cards, at the top of its band from `top`.
fn group_heading(canvas: &Canvas, text: &str, x: f32, top: f32, theme: &Theme) {
    Label::new(text)
        .with_style(styles::HEADLINE_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(x, top + HEADING_H / 2.0 - 2.0)
        .render(canvas);
}

/// A card's ground and edge.
pub(super) fn draw_card(canvas: &Canvas, rect: Rect, theme: &Theme) {
    let rr = RRect::new_rect_xy(rect, 12.0, 12.0);
    canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
    canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
}

/// What kind of device it is, in a word or two.
pub(super) fn kind_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Bluetooth => t!("audio-devices-kind-bluetooth"),
        DeviceKind::Usb => t!("audio-devices-kind-usb"),
        DeviceKind::Hdmi => t!("audio-kind-hdmi"),
        DeviceKind::AirPlay => t!("audio-airplay-tag"),
        DeviceKind::Virtual => t!("audio-kind-virtual"),
        DeviceKind::BuiltIn => t!("audio-devices-kind-built-in"),
    }
}

/// A mute button: a speaker, struck through while muted.
fn draw_speaker(canvas: &Canvas, rect: Rect, muted: bool, pressed: bool, theme: &Theme) {
    let rr = RRect::new_rect_xy(rect, 6.0, 6.0);
    canvas.draw_rrect(
        rr,
        &fill(if pressed {
            theme.fill_tertiary
        } else {
            theme.fill_quaternary
        }),
    );
    let tint = if muted {
        theme.text_tertiary
    } else {
        theme.text_primary
    };
    let glyph = rect.with_inset((5.0, 5.0));
    draw_pane_glyph_fallback(canvas, Pane::Mixer, glyph, tint);
    if muted {
        let mut paint = stroke(theme.accent_red, 1.6);
        paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
        canvas.draw_line(
            Point::new(glyph.left, glyph.top),
            Point::new(glyph.right, glyph.bottom),
            &paint,
        );
    }
}

/// An app's icon from the person's theme, or its initial in a circle.
fn draw_app_icon(canvas: &Canvas, rect: Rect, stream: &Stream, scale: f32, theme: &Theme) {
    let lower = stream.app.to_lowercase();
    let mut names: Vec<&str> = Vec::new();
    if let Some(icon) = &stream.icon {
        names.push(icon);
    }
    if let Some(binary) = &stream.binary {
        names.push(binary);
    }
    names.push(&lower);
    let px = (rect.width() * scale).round() as i32;
    if let Some(image) =
        otto_kit::icons::cached_icon_chain_at(&names, px, otto_kit::icons::FULL_COLOUR_SIZE)
    {
        otto_kit::icons::draw_icon(canvas, &image, rect, &Paint::default());
        return;
    }
    canvas.draw_circle(
        Point::new(rect.center_x(), rect.center_y()),
        rect.width() / 2.0,
        &fill(theme.fill_tertiary),
    );
    let initial: String = stream
        .app
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    Label::new(initial)
        .with_style(styles::HEADLINE_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_at(rect.center_x(), rect.center_y())
        .render(canvas);
}
