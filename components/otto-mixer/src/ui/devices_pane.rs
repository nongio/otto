//! The Devices section: a list of the sound devices, and the one picked: how
//! its output sounds (see `device_sound.rs`), a Bluetooth device's choice
//! between its best sound and its microphone, and, folded away, its profile,
//! the port each of its outputs and inputs uses, and a Bluetooth codec.
//!
//! Every choice is a device param the server keeps, so nothing here is
//! written to the routing file, except the AirPlay speakers switch under the
//! cards, which the service follows.

// Rust guideline compliant 2026-02-21

use otto_audio_graph::{
    codec_named, AirPlayMissing, Availability, Device, Direction, Endpoint, Profile,
    AIRPLAY_PACKAGE,
};
use otto_kit::components::dropdown::{field as dropdown, DropdownInteraction};
use otto_kit::components::toggle::{self, ToggleInteraction};

use super::device_sound::{Block, DeviceAction, Shown, DEVICE_LIST_W, DISCLOSURE_H, LIST_ITEM_H};
use super::*;

const DEVICE_GAP: f32 = 16.0;
/// A card's inner margin.
const DEVICE_PAD: f32 = 20.0;
/// A card's name and kind, above its rows.
const DEVICE_HEAD_H: f32 = 62.0;
/// One labelled row: a label and its control.
const DEVICE_ROW_H: f32 = 42.0;
/// A line of explanation under the rows.
const DEVICE_NOTE_H: f32 = 30.0;
/// The label column of a card's rows.
const DEVICE_LABEL_W: f32 = 130.0;
/// The device's name and kind above its cards, which stay put while they
/// scroll.
const DEVICES_HEADER_H: f32 = 72.0;
/// The most choices a segmented control holds before a pop-up takes over.
const MAX_SEGMENTS: usize = 5;
/// A segment's room around its label.
const SEGMENT_PAD: f32 = 20.0;
/// A band in the AirPlay card: what it needs, or the service notice.
const NOTICE_H: f32 = 44.0;
/// One AirPlay speaker in the AirPlay card.
const SPEAKER_ROW_H: f32 = 32.0;

/// A choice a card offers.
#[derive(Debug, Clone, PartialEq)]
struct Choice {
    label: String,
    action: Action,
    /// Offered but not usable now: a jack with nothing plugged in.
    dimmed: bool,
}

/// One labelled row of a card.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    /// A pick among `choices`, `chosen` the one in force; `menu` is the
    /// action that opens them as a pop-up when they do not fit as segments.
    Pick {
        label: String,
        choices: Vec<Choice>,
        chosen: Option<usize>,
        menu: Action,
        /// Whether the choices may show as segments at all.
        segments: bool,
    },
    /// A fact: a label and its value.
    Fact { label: String, value: String },
}

/// A card's content, read from the device and the nodes it has.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DeviceCard {
    title: String,
    kind: String,
    /// On the right of the title: what it offers now, or that it is off.
    badge: Option<String>,
    rows: Vec<Row>,
    note: Option<String>,
}

impl DeviceCard {
    pub(super) fn height(&self) -> f32 {
        let note = if self.note.is_some() {
            DEVICE_NOTE_H
        } else {
            0.0
        };
        DEVICE_HEAD_H + self.rows.len() as f32 * DEVICE_ROW_H + note + DEVICE_PAD - 6.0
    }
}

impl Ui {
    /// The detail's scrolling area, right of the device list and under the
    /// selected device's name.
    pub(super) fn devices_viewport(&self) -> Rect {
        let content = self.content();
        Rect::from_ltrb(
            content.left + DEVICE_LIST_W,
            content.top + DEVICES_HEADER_H,
            content.right,
            content.bottom,
        )
    }

    /// The sound devices, cards first and Bluetooth devices after, each by
    /// name.
    fn sound_devices(&self) -> Vec<&Device> {
        let mut devices: Vec<&Device> = self.graph.devices().filter(|d| d.is_audio()).collect();
        devices.sort_by(|a, b| {
            (a.is_bluetooth(), a.title(), a.id).cmp(&(b.is_bluetooth(), b.title(), b.id))
        });
        devices
    }

    /// What the detail shows: the device picked in the list, else the one
    /// playing the default output, else the first.
    pub(super) fn shown_device(&self) -> Shown {
        let devices = self.sound_devices();
        match self.devices.selected {
            Some(Shown::AirPlay) => return Shown::AirPlay,
            Some(Shown::Device(id)) if devices.iter().any(|d| d.id == id) => {
                return Shown::Device(id)
            }
            _ => {}
        }
        let default = self
            .graph
            .default_node(Endpoint::Output)
            .and_then(|n| n.prop("device.id"))
            .and_then(|id| id.parse::<u32>().ok());
        default
            .filter(|id| devices.iter().any(|d| d.id == *id))
            .or_else(|| devices.first().map(|d| d.id))
            .map_or(Shown::AirPlay, Shown::Device)
    }

    pub(super) fn draw_devices(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        let list = Rect::from_ltrb(
            content.left,
            content.top,
            content.left + DEVICE_LIST_W,
            content.bottom,
        );
        let main = Rect::from_ltrb(list.right, content.top, content.right, content.bottom);
        canvas.draw_rect(main, &fill(graph_view::canvas_ground(theme)));
        let shown = self.shown_device();
        self.draw_device_list(canvas, list, shown, theme);

        let left = main.left + 28.0;
        let right = main.right - 28.0;
        let width = (right - left).max(1.0);
        let top = content.top + 24.0;
        let device = match shown {
            Shown::Device(id) => self.graph.device(id).cloned(),
            Shown::AirPlay => None,
        };
        let (title, subtitle) = match &device {
            Some(device) => {
                let card = self.device_card(device);
                let subtitle = match card.badge {
                    Some(badge) => format!("{} · {badge}", card.kind),
                    None => card.kind,
                };
                (card.title, subtitle)
            }
            None => (
                t!("audio-airplay-title").to_string(),
                t!("audio-airplay-intro").to_string(),
            ),
        };
        let title_font = styles::TITLE_2_EMPHASIZED.font();
        Label::new(ellipsize(&title_font, &title, width))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, top + 12.0)
            .render(canvas);
        let intro_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&intro_font, &subtitle, width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, top + 38.0)
            .render(canvas);

        let viewport = self.devices_viewport();
        let mut y = 4.0;
        let mut placed: Vec<(Rect, Block)> = Vec::new();
        match &device {
            None => {
                let airplay = self.airplay_card();
                placed.push((
                    Rect::from_xywh(left, y, width, airplay.height()),
                    Block::AirPlay(airplay),
                ));
                y += placed[0].0.height() + DEVICE_GAP;
            }
            Some(device) => {
                if let Some(sound) = self.sound_card(device) {
                    let h = sound.height();
                    placed.push((Rect::from_xywh(left, y, width, h), Block::Sound(sound)));
                    y += h + DEVICE_GAP;
                }
                if let Some(bluetooth) = self.bluetooth_card(device) {
                    let h = bluetooth.height(width - 2.0 * DEVICE_PAD);
                    placed.push((
                        Rect::from_xywh(left, y, width, h),
                        Block::Bluetooth(bluetooth),
                    ));
                    y += h + DEVICE_GAP;
                }
                placed.push((
                    Rect::from_xywh(left, y, width, DISCLOSURE_H),
                    Block::Disclosure,
                ));
                y += DISCLOSURE_H + 8.0;
                if self.devices.details {
                    let card = self.device_card(device);
                    let h = card.height();
                    placed.push((Rect::from_xywh(left, y, width, h), Block::Details(card)));
                    y += h + DEVICE_GAP;
                }
            }
        }
        self.devices_scroll.set_viewport(viewport);
        self.devices_scroll.set_content_length(y + 8.0);
        self.devices_scroll.tick();
        let shift = viewport.top - self.devices_scroll.offset();

        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));
        let first_target = self.targets.len();
        for (at, block) in placed {
            let rect = at.with_offset((0.0, shift));
            if rect.bottom < viewport.top || rect.top > viewport.bottom {
                continue;
            }
            match block {
                Block::AirPlay(card) => {
                    self.draw_airplay_card(canvas, rect, &card, viewport, theme)
                }
                Block::Sound(card) => self.draw_sound_card(canvas, rect, &card, theme),
                Block::Bluetooth(card) => self.draw_bluetooth_card(canvas, rect, &card, theme),
                Block::Disclosure => {
                    let cy = rect.center_y();
                    draw_disclosure(
                        canvas,
                        Point::new(rect.left + 6.0, cy),
                        self.devices.details,
                        theme,
                    );
                    let font = styles::BODY_EMPHASIZED.font();
                    let label = t!("audio-devices-details");
                    let w = font.measure_str(label, None).0;
                    Label::new(label)
                        .with_style(styles::BODY_EMPHASIZED)
                        .with_color(theme.text_secondary)
                        .centered_on(rect.left + 18.0, cy)
                        .render(canvas);
                    self.targets.push((
                        Rect::from_xywh(rect.left - 4.0, rect.top, w + 30.0, rect.height()),
                        Action::Device(DeviceAction::Details),
                    ));
                }
                Block::Details(card) => self.draw_device_card(canvas, rect, &card, viewport, theme),
            }
        }
        canvas.restore();
        // The detail scrolls: only what of its controls is in view presses.
        let drawn: Vec<(Rect, Action)> = self.targets.drain(first_target..).collect();
        for (rect, action) in drawn {
            self.push_visible_target(rect, action, viewport);
        }
        self.devices_scroll.render(canvas, theme, |_, _| {});
    }

    /// The list of devices beside the detail, AirPlay speakers last.
    fn draw_device_list(&mut self, canvas: &Canvas, list: Rect, shown: Shown, theme: &Theme) {
        canvas.draw_rect(list, &fill(panel_ground(theme)));
        canvas.draw_line(
            Point::new(list.right, list.top),
            Point::new(list.right, list.bottom),
            &stroke(theme.hairline, 1.0),
        );
        let left = list.left + 12.0;
        let width = list.width() - 24.0;
        let mut y = list.top + 16.0;
        card_heading(
            canvas,
            t!("audio-devices-title"),
            left + 12.0,
            y + 8.0,
            theme,
        );
        y += 22.0;
        let mut items: Vec<(String, String, Shown)> = self
            .sound_devices()
            .into_iter()
            .map(|d| {
                (
                    d.title().to_string(),
                    device_kind(d).to_string(),
                    Shown::Device(d.id),
                )
            })
            .collect();
        if items.is_empty() && self.graph.is_synced() {
            Label::new(t!("audio-devices-empty"))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_tertiary)
                .centered_on(left + 12.0, y + 10.0)
                .render(canvas);
            y += 30.0;
        }
        items.push((
            t!("audio-airplay-title").to_string(),
            t!("audio-airplay-tag").to_string(),
            Shown::AirPlay,
        ));
        for (title, kind, item_of) in items {
            if y + LIST_ITEM_H > list.bottom {
                break;
            }
            let item = Rect::from_xywh(left, y, width, LIST_ITEM_H);
            if item_of == shown {
                let rr = RRect::new_rect_xy(item, 10.0, 10.0);
                let a = theme.accent;
                canvas.draw_rrect(rr, &fill(Color::from_argb(0x22, a.r(), a.g(), a.b())));
                canvas.draw_rrect(
                    rr,
                    &stroke(Color::from_argb(0x66, a.r(), a.g(), a.b()), 1.0),
                );
            }
            let font = styles::SUBHEADLINE_EMPHASIZED.font();
            Label::new(ellipsize(&font, &title, width - 24.0))
                .with_style(styles::SUBHEADLINE_EMPHASIZED)
                .with_color(theme.text_primary)
                .centered_on(item.left + 12.0, item.top + 17.0)
                .render(canvas);
            let font = styles::SUBHEADLINE.font();
            Label::new(ellipsize(&font, &kind, width - 24.0))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(item.left + 12.0, item.top + 35.0)
                .render(canvas);
            let action = match item_of {
                Shown::Device(id) => DeviceAction::Select(id),
                Shown::AirPlay => DeviceAction::SelectAirPlay,
            };
            self.targets.push((item, Action::Device(action)));
            y += LIST_ITEM_H + 4.0;
        }
    }

    fn draw_device_card(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        card: &DeviceCard,
        viewport: Rect,
        theme: &Theme,
    ) {
        let rr = RRect::new_rect_xy(rect, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        let left = rect.left + DEVICE_PAD;
        let right = rect.right - DEVICE_PAD;

        let badge_w = card.badge.as_ref().map_or(0.0, |badge| {
            let font = styles::SUBHEADLINE.font();
            let w = font.measure_str(badge, None).0;
            Label::new(badge)
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(right - w, rect.top + 28.0)
                .render(canvas);
            w + 12.0
        });
        let title_font = styles::HEADLINE_EMPHASIZED.font();
        Label::new(ellipsize(&title_font, &card.title, right - left - badge_w))
            .with_style(styles::HEADLINE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, rect.top + 28.0)
            .render(canvas);
        let kind_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&kind_font, &card.kind, right - left))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, rect.top + 48.0)
            .render(canvas);

        let label_font = styles::BODY.font();
        let control_left = left + DEVICE_LABEL_W;
        let control_w = (right - control_left).max(1.0);
        for (index, row) in card.rows.iter().enumerate() {
            let cy = rect.top + DEVICE_HEAD_H + index as f32 * DEVICE_ROW_H + DEVICE_ROW_H / 2.0;
            let label = match row {
                Row::Pick { label, .. } | Row::Fact { label, .. } => label,
            };
            Label::new(ellipsize(&label_font, label, DEVICE_LABEL_W - 12.0))
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .centered_on(left, cy)
                .render(canvas);
            let control = Rect::from_xywh(
                control_left,
                cy - dropdown::HEIGHT / 2.0,
                control_w,
                dropdown::HEIGHT,
            );
            match row {
                Row::Fact { value, .. } => {
                    Label::new(ellipsize(&label_font, value, control_w))
                        .with_style(styles::BODY)
                        .with_color(theme.text_primary)
                        .centered_on(control_left, cy)
                        .render(canvas);
                }
                Row::Pick {
                    choices,
                    chosen,
                    menu,
                    segments,
                    ..
                } => {
                    if *segments && fits_as_segments(choices, control_w) {
                        self.draw_choice_segments(
                            canvas, control, choices, *chosen, viewport, theme,
                        );
                    } else {
                        self.draw_choice_field(
                            canvas, control, choices, *chosen, *menu, viewport, theme,
                        );
                    }
                }
            }
        }
        if let Some(note) = &card.note {
            let cy = rect.top + DEVICE_HEAD_H + card.rows.len() as f32 * DEVICE_ROW_H + 12.0;
            let font = styles::SUBHEADLINE.font();
            Label::new(ellipsize(&font, note, right - left))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(left, cy)
                .render(canvas);
        }
    }

    /// Choices side by side, each pressable; a dimmed one in quieter ink.
    fn draw_choice_segments(
        &mut self,
        canvas: &Canvas,
        control: Rect,
        choices: &[Choice],
        chosen: Option<usize>,
        viewport: Rect,
        theme: &Theme,
    ) {
        // Measured at the chosen weight, the wider of the two.
        let font = segmented::LABEL_CHOSEN.font();
        let widest = choices
            .iter()
            .map(|c| font.measure_str(&c.label, None).0 + SEGMENT_PAD)
            .fold(0.0, f32::max);
        let track_w = (widest * choices.len() as f32).min(control.width());
        let track = Rect::from_xywh(
            control.left,
            control.center_y() - segmented::HEIGHT / 2.0,
            track_w,
            segmented::HEIGHT,
        );
        // The track and the chosen pill come from the kit; the labels are
        // drawn here so an unplugged port can be dimmed on its own.
        let blank: Vec<Segment<'_>> = choices.iter().map(|_| Segment::Text("")).collect();
        segmented::draw(
            canvas,
            track,
            &blank,
            chosen,
            SegmentedInteraction::Normal,
            theme,
        );
        for (index, choice) in choices.iter().enumerate() {
            let cell = segmented::segment_rect(track, choices.len(), index);
            let is_chosen = Some(index) == chosen;
            let color = if !is_chosen && choice.dimmed {
                theme.text_tertiary
            } else {
                segmented::label_color(theme, is_chosen, SegmentedInteraction::Normal)
            };
            let style = segmented::label_style(is_chosen);
            Label::new(ellipsize(&style.font(), &choice.label, cell.width() - 8.0))
                .with_style(style)
                .with_color(color)
                .centered_at(cell.center_x(), cell.center_y())
                .render(canvas);
            if Some(index) != chosen {
                self.push_visible_target(cell, choice.action, viewport);
            }
        }
    }

    /// The chosen value in a pop-up button that lists every choice.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call shape for both controls"
    )]
    fn draw_choice_field(
        &mut self,
        canvas: &Canvas,
        control: Rect,
        choices: &[Choice],
        chosen: Option<usize>,
        menu: Action,
        viewport: Rect,
        theme: &Theme,
    ) {
        let label = chosen
            .and_then(|i| choices.get(i))
            .map_or("", |c| c.label.as_str());
        let interaction = if self.menu_open == Some(menu) {
            DropdownInteraction::Open
        } else if self.pressed == Some(menu) {
            DropdownInteraction::Pressed
        } else if choices.len() < 2 {
            DropdownInteraction::Disabled
        } else {
            DropdownInteraction::Normal
        };
        let field = Rect::from_xywh(
            control.left,
            control.top,
            control.width().min(300.0),
            control.height(),
        );
        dropdown::draw(canvas, field, label, interaction, theme);
        if choices.len() >= 2 {
            self.push_visible_target(field, menu, viewport);
        }
    }

    /// Record a control in a scrolled area, only as far as it is in view.
    fn push_visible_target(&mut self, rect: Rect, action: Action, viewport: Rect) {
        let mut visible = rect;
        if visible.intersect(viewport) {
            self.targets.push((visible, action));
        }
    }

    /// Open the pop-up `action` names, anchored to `rect`; false when the
    /// action opens none.
    pub(super) fn open_device_menu(&mut self, action: Action, rect: Rect, serial: u32) -> bool {
        let Action::DeviceMenu { device, row } = action else {
            return false;
        };
        let Some(card) = self.graph.device(device).map(|d| self.device_card(d)) else {
            return true;
        };
        let Some(Row::Pick {
            choices, chosen, ..
        }) = card.rows.get(row)
        else {
            return true;
        };
        self.menu_choices = choices.iter().map(|c| c.action).collect();
        self.menu_open = Some(action);
        self.menu_request = Some(MenuRequest {
            rect,
            serial,
            options: choices.iter().map(|c| c.label.clone()).collect(),
            selected: *chosen,
        });
        true
    }

    /// What a device's card shows, read from the device and its nodes.
    fn device_card(&self, device: &Device) -> DeviceCard {
        let mut rows = Vec::new();
        let mut note = None;
        if device.is_bluetooth() {
            if let Some(row) = bluetooth_profile_row(device, rows.len()) {
                rows.push(row);
            }
            let node_codec = self
                .nodes_of(device)
                .find_map(|n| n.prop("api.bluez5.codec"));
            if let Some(row) = codec_row(device, node_codec, rows.len()) {
                rows.push(row);
                note = Some(t!("audio-devices-codec-note").to_string());
            }
        } else if let Some(row) = profile_row(device, rows.len()) {
            rows.push(row);
        }
        // Outputs first, then inputs, as the ports are read.
        let mut active_routes: Vec<_> = device.active_routes.iter().collect();
        active_routes.sort_by_key(|r| r.direction != Some(Direction::Output));
        for active in active_routes {
            let (Some(card_device), Some(direction)) = (active.device, active.direction) else {
                continue;
            };
            let offered = device.routes_for(active);
            let choices: Vec<Choice> = offered
                .iter()
                .map(|route| {
                    let unplugged = route.available == Availability::No;
                    Choice {
                        label: if unplugged {
                            t_owned!("audio-devices-unplugged", port = route.description.clone())
                        } else {
                            route.description.clone()
                        },
                        action: Action::Port {
                            device: device.id,
                            index: route.index,
                            card_device,
                        },
                        dimmed: unplugged,
                    }
                })
                .collect();
            if choices.is_empty() {
                continue;
            }
            let chosen = offered.iter().position(|r| r.index == active.index);
            let label = match direction {
                Direction::Output => t!("audio-devices-output-port"),
                Direction::Input => t!("audio-devices-input-port"),
            };
            rows.push(Row::Pick {
                label: label.to_string(),
                choices,
                chosen,
                menu: Action::DeviceMenu {
                    device: device.id,
                    row: rows.len(),
                },
                segments: true,
            });
        }
        if let Some(value) = volume_control(device) {
            rows.push(Row::Fact {
                label: t!("audio-devices-volume").to_string(),
                value,
            });
        }
        let off = device.profile.as_ref().is_some_and(Profile::is_off);
        let badge = if off {
            Some(t!("audio-devices-off").to_string())
        } else {
            self.channel_badge(device)
        };
        DeviceCard {
            title: device.title().to_string(),
            kind: device_kind(device).to_string(),
            badge,
            rows,
            note,
        }
    }

    /// The nodes this device has now.
    fn nodes_of<'a>(
        &'a self,
        device: &'a Device,
    ) -> impl Iterator<Item = &'a otto_audio_graph::Node> + 'a {
        let id = device.id.to_string();
        self.graph
            .nodes()
            .filter(move |n| n.prop("device.id") == Some(id.as_str()))
    }

    /// "2 in · 2 out": the channels its sources and sinks carry now.
    fn channel_badge(&self, device: &Device) -> Option<String> {
        let (mut inputs, mut outputs) = (0_u32, 0_u32);
        for node in self.nodes_of(device) {
            let channels: u32 = node
                .prop("audio.channels")
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            match node.media_class() {
                "Audio/Source" => inputs += channels,
                "Audio/Sink" => outputs += channels,
                _ => {}
            }
        }
        let mut parts = Vec::new();
        if inputs > 0 {
            parts.push(t_owned!("audio-devices-inputs", count = inputs));
        }
        if outputs > 0 {
            parts.push(t_owned!("audio-devices-outputs", count = outputs));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// What the AirPlay card shows: the switch, what discovery lacks, and the
    /// speakers found.
    fn airplay_card(&self) -> AirPlayCard {
        let mut speakers: Vec<(String, bool)> = self
            .graph
            .airplay_sinks()
            .map(|n| (n.title().to_string(), n.state == NodeState::Running))
            .collect();
        speakers.sort();
        AirPlayCard {
            on: self.routing.airplay,
            missing: self.airplay_missing,
            service_down: self.service_down(),
            speakers,
        }
    }

    fn draw_airplay_card(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        card: &AirPlayCard,
        viewport: Rect,
        theme: &Theme,
    ) {
        let first_target = self.targets.len();
        let rr = RRect::new_rect_xy(rect, 12.0, 12.0);
        canvas.draw_rrect(rr, &fill(graph_view::card_ground(theme)));
        canvas.draw_rrect(rr, &stroke(theme.hairline, 1.0));
        let left = rect.left + DEVICE_PAD;
        let right = rect.right - DEVICE_PAD;
        let width = right - left;

        let switch = Rect::from_xywh(
            right - toggle::WIDTH,
            rect.top + 28.0 - toggle::HEIGHT / 2.0,
            toggle::WIDTH,
            toggle::HEIGHT,
        );
        let enabled = card.on || card.missing.is_none();
        let interaction = if !enabled {
            ToggleInteraction::Disabled
        } else if self.pressed == Some(Action::AirPlay) {
            ToggleInteraction::Pressed
        } else {
            ToggleInteraction::Normal
        };
        toggle::draw(
            canvas,
            switch,
            toggle::knob_fraction_for(card.on),
            interaction,
            theme,
        );
        if enabled {
            self.targets.push((switch, Action::AirPlay));
        }
        let title_font = styles::HEADLINE_EMPHASIZED.font();
        Label::new(ellipsize(
            &title_font,
            t!("audio-airplay-title"),
            switch.left - left - 12.0,
        ))
        .with_style(styles::HEADLINE_EMPHASIZED)
        .with_color(theme.text_primary)
        .centered_on(left, rect.top + 28.0)
        .render(canvas);
        let kind_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(
            &kind_font,
            t!("audio-airplay-intro"),
            switch.left - left - 12.0,
        ))
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_on(left, rect.top + 48.0)
        .render(canvas);

        let mut y = rect.top + DEVICE_HEAD_H;
        if card.service_down {
            self.draw_service_notice(
                canvas,
                Rect::from_xywh(left, y, width, NOTICE_H),
                t!("audio-airplay-service-down"),
                theme,
            );
            y += NOTICE_H + 12.0;
        }
        if let Some(missing) = card.missing {
            let text = match missing {
                AirPlayMissing::Module => {
                    t_owned!("audio-airplay-missing-module", package = AIRPLAY_PACKAGE)
                }
                AirPlayMissing::Avahi => t!("audio-airplay-missing-avahi").to_string(),
            };
            super::effects_pane::draw_missing(
                canvas,
                Rect::from_xywh(left, y, width, NOTICE_H),
                &text,
                theme,
            );
            y += NOTICE_H + 12.0;
        }
        if card.lists_speakers() {
            let font = styles::BODY.font();
            if card.speakers.is_empty() {
                caption(
                    canvas,
                    t!("audio-airplay-searching"),
                    left,
                    y + SPEAKER_ROW_H / 2.0,
                    theme,
                );
                y += SPEAKER_ROW_H;
            }
            for (index, (name, playing)) in card.speakers.iter().enumerate() {
                if index > 0 {
                    canvas.draw_line(
                        Point::new(left, y),
                        Point::new(right, y),
                        &stroke(theme.hairline, 1.0),
                    );
                }
                let cy = y + SPEAKER_ROW_H / 2.0;
                let tag = if *playing {
                    t!("audio-airplay-playing")
                } else {
                    t!("audio-airplay-tag")
                };
                let tag_w = styles::SUBHEADLINE.font().measure_str(tag, None).0;
                Label::new(tag)
                    .with_style(styles::SUBHEADLINE)
                    .with_color(theme.text_secondary)
                    .centered_on(right - tag_w, cy)
                    .render(canvas);
                Label::new(ellipsize(&font, name, width - tag_w - 12.0))
                    .with_style(styles::BODY)
                    .with_color(theme.text_primary)
                    .centered_on(left, cy)
                    .render(canvas);
                y += SPEAKER_ROW_H;
            }
            let font = styles::SUBHEADLINE.font();
            Label::new(ellipsize(&font, t!("audio-airplay-delay"), width))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(left, y + 12.0)
                .render(canvas);
        }
        // The card scrolls: only what of its controls is in view presses.
        let drawn: Vec<(Rect, Action)> = self.targets.drain(first_target..).collect();
        for (rect, action) in drawn {
            self.push_visible_target(rect, action, viewport);
        }
    }
}

/// The AirPlay card under the device cards.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AirPlayCard {
    /// Whether the routing file has AirPlay discovery on.
    on: bool,
    /// What discovery lacks here, if anything.
    missing: Option<AirPlayMissing>,
    /// The audio service, which runs discovery, is not running.
    service_down: bool,
    /// The speakers found, by name, and whether each is playing.
    speakers: Vec<(String, bool)>,
}

impl AirPlayCard {
    /// Whether the speakers found are listed: while it is on and able to
    /// work, or whenever there are some.
    fn lists_speakers(&self) -> bool {
        (self.on && self.missing.is_none()) || !self.speakers.is_empty()
    }

    pub(super) fn height(&self) -> f32 {
        let mut h = DEVICE_HEAD_H;
        if self.service_down {
            h += NOTICE_H + 12.0;
        }
        if self.missing.is_some() {
            h += NOTICE_H + 12.0;
        }
        if self.lists_speakers() {
            h += self.speakers.len().max(1) as f32 * SPEAKER_ROW_H + DEVICE_NOTE_H;
        }
        h + DEVICE_PAD - 6.0
    }
}

/// A Bluetooth device's profiles, one per family: playback, headset,
/// off. Its codecs are the next row.
fn bluetooth_profile_row(device: &Device, row: usize) -> Option<Row> {
    let active = device.profile.as_ref()?;
    let mut families: Vec<&str> = Vec::new();
    for profile in &device.profiles {
        let family = profile.family();
        if profile.available != Availability::No && !families.contains(&family) {
            families.push(family);
        }
    }
    if !families.contains(&active.family()) {
        families.push(active.family());
    }
    let mut choices = Vec::new();
    let mut chosen = None;
    for family in families {
        let members: Vec<&Profile> = device
            .profiles
            .iter()
            .filter(|p| p.family() == family)
            .collect();
        // The active profile stands for its family; else the codec-less
        // one, which lets BlueZ choose; else the one it prefers.
        let Some(pick) = members
            .iter()
            .find(|p| p.index == active.index)
            .or_else(|| members.iter().find(|p| p.name == family))
            .or_else(|| {
                members
                    .iter()
                    .filter(|p| p.available != Availability::No)
                    .max_by_key(|p| p.priority)
            })
            .copied()
        else {
            continue;
        };
        if family == active.family() {
            chosen = Some(choices.len());
        }
        choices.push(Choice {
            label: without_codec(&pick.description),
            action: Action::Profile {
                device: device.id,
                index: pick.index,
            },
            dimmed: false,
        });
    }
    Some(Row::Pick {
        label: t!("audio-devices-profile").to_string(),
        choices,
        chosen,
        menu: Action::DeviceMenu {
            device: device.id,
            row,
        },
        segments: false,
    })
}

/// The codecs the running profile's family offers, the one in use chosen.
fn codec_row(device: &Device, node_codec: Option<&str>, row: usize) -> Option<Row> {
    let active = device.profile.as_ref()?;
    let family = active.family();
    let offered: Vec<&Profile> = device
        .profiles
        .iter()
        .filter(|p| p.family() == family && p.available != Availability::No)
        .filter(|p| p.codec().is_some())
        .collect();
    if offered.is_empty() {
        return None;
    }
    // The profile names its codec, unless BlueZ chose it: then the
    // device's nodes say which, as `node_codec`.
    let in_use = active
        .codec()
        .map(|(_, c)| c)
        .or_else(|| node_codec.and_then(codec_named));
    let chosen = offered
        .iter()
        .position(|p| p.codec().map(|(_, c)| c) == in_use);
    let choices = offered
        .iter()
        .filter_map(|p| {
            let (_, codec) = p.codec()?;
            Some(Choice {
                label: codec.label.to_string(),
                action: Action::Profile {
                    device: device.id,
                    index: p.index,
                },
                dimmed: false,
            })
        })
        .collect();
    Some(Row::Pick {
        label: t!("audio-devices-codec").to_string(),
        choices,
        chosen,
        menu: Action::DeviceMenu {
            device: device.id,
            row,
        },
        segments: true,
    })
}

/// A card's profiles: those it can run now, and the one it runs.
fn profile_row(device: &Device, row: usize) -> Option<Row> {
    let active = device.profile.as_ref()?;
    let offered: Vec<&Profile> = device
        .profiles
        .iter()
        .filter(|p| p.available != Availability::No || p.index == active.index)
        .collect();
    let chosen = offered.iter().position(|p| p.index == active.index);
    Some(Row::Pick {
        label: t!("audio-devices-profile").to_string(),
        choices: offered
            .iter()
            .map(|p| Choice {
                label: p.description.clone(),
                action: Action::Profile {
                    device: device.id,
                    index: p.index,
                },
                dimmed: false,
            })
            .collect(),
        chosen,
        menu: Action::DeviceMenu {
            device: device.id,
            row,
        },
        segments: false,
    })
}

/// Whether every choice's label fits its share of `width` as a segment.
fn fits_as_segments(choices: &[Choice], width: f32) -> bool {
    if choices.is_empty() || choices.len() > MAX_SEGMENTS {
        return false;
    }
    let font = styles::BODY.font();
    let widest = choices
        .iter()
        .map(|c| font.measure_str(&c.label, None).0 + SEGMENT_PAD)
        .fold(0.0, f32::max);
    widest * choices.len() as f32 <= width
}

/// Where the volume is set, from what the ports in use say: on the
/// hardware's own mixer, in software, or each for one direction.
fn volume_control(device: &Device) -> Option<String> {
    let said: Vec<(Direction, bool)> = device
        .active_routes
        .iter()
        .filter_map(|r| Some((r.direction?, r.hardware_volume()?)))
        .collect();
    let first = said.first()?.1;
    if said.iter().all(|(_, hw)| *hw == first) {
        return Some(volume_word(first).to_string());
    }
    let mut parts = Vec::new();
    for direction in [Direction::Output, Direction::Input] {
        if let Some((_, hw)) = said.iter().find(|(d, _)| *d == direction) {
            let word = volume_word(*hw);
            parts.push(match direction {
                Direction::Output => t_owned!("audio-devices-volume-output", control = word),
                Direction::Input => t_owned!("audio-devices-volume-input", control = word),
            });
        }
    }
    Some(parts.join(" · "))
}

fn volume_word(hardware: bool) -> &'static str {
    if hardware {
        t!("audio-devices-volume-hardware")
    } else {
        t!("audio-devices-volume-software")
    }
}

/// What kind of device it is, from how it is attached.
fn device_kind(device: &Device) -> &'static str {
    if device.is_bluetooth() {
        return match device.prop("device.form-factor") {
            Some("headphone") => t!("audio-devices-kind-bluetooth-headphones"),
            Some("headset" | "hands-free") => t!("audio-devices-kind-bluetooth-headset"),
            Some("speaker") => t!("audio-devices-kind-bluetooth-speaker"),
            _ => t!("audio-devices-kind-bluetooth"),
        };
    }
    match (device.prop("device.bus"), device.prop("device.form-factor")) {
        (Some("usb"), _) => t!("audio-devices-kind-usb"),
        (_, Some("internal")) | (Some("pci"), _) => t!("audio-devices-kind-built-in"),
        _ => t!("audio-devices-kind-other"),
    }
}

/// A Bluetooth profile's description without the codec it names:
/// "High Fidelity Playback (A2DP Sink, codec LDAC)" reads as
/// "High Fidelity Playback (A2DP Sink)". The server translates the words,
/// so this goes by the punctuation alone.
fn without_codec(description: &str) -> String {
    let Some(open) = description.rfind('(') else {
        return description.to_string();
    };
    match description[open..].rfind(", ") {
        Some(comma) if description.ends_with(')') => {
            format!("{})", &description[..open + comma])
        }
        _ => description.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_codec_comes_off_a_bluetooth_profile_s_description() {
        assert_eq!(
            without_codec("High Fidelity Playback (A2DP Sink, codec LDAC)"),
            "High Fidelity Playback (A2DP Sink)"
        );
        assert_eq!(without_codec("Off"), "Off");
        assert_eq!(
            without_codec("Headset Head Unit (HSP/HFP)"),
            "Headset Head Unit (HSP/HFP)"
        );
    }

    fn headphones() -> Device {
        let profile = |index, name: &str, description: &str, available| Profile {
            index,
            name: name.into(),
            description: description.into(),
            available,
            priority: index,
        };
        let ldac = profile(
            3,
            "a2dp-sink-ldac",
            "High Fidelity Playback (A2DP Sink, codec LDAC)",
            Availability::Yes,
        );
        Device {
            id: 9,
            props: [("device.api".to_string(), "bluez5".to_string())].into(),
            profiles: vec![
                profile(0, "off", "Off", Availability::Yes),
                profile(
                    1,
                    "a2dp-sink-sbc",
                    "High Fidelity Playback (A2DP Sink, codec SBC)",
                    Availability::Yes,
                ),
                profile(
                    2,
                    "a2dp-sink-aac",
                    "High Fidelity Playback (A2DP Sink, codec AAC)",
                    Availability::No,
                ),
                ldac.clone(),
                profile(
                    4,
                    "headset-head-unit-msbc",
                    "Headset Head Unit (HSP/HFP, codec mSBC)",
                    Availability::Yes,
                ),
            ],
            profile: Some(ldac),
            ..Device::default()
        }
    }

    fn labels(row: &Row) -> (Vec<&str>, Option<usize>) {
        match row {
            Row::Pick {
                choices, chosen, ..
            } => (choices.iter().map(|c| c.label.as_str()).collect(), *chosen),
            Row::Fact { .. } => panic!("not a pick"),
        }
    }

    #[test]
    fn a_bluetooth_device_offers_one_profile_per_family() {
        let row = bluetooth_profile_row(&headphones(), 0).expect("a row");
        assert_eq!(
            labels(&row),
            (
                vec![
                    "Off",
                    "High Fidelity Playback (A2DP Sink)",
                    "Headset Head Unit (HSP/HFP)"
                ],
                Some(1)
            )
        );
    }

    #[test]
    fn the_codecs_offered_are_the_running_family_s_usable_ones() {
        let row = codec_row(&headphones(), None, 1).expect("a row");
        assert_eq!(labels(&row), (vec!["SBC", "LDAC"], Some(1)));
        let Row::Pick { choices, .. } = &row else {
            unreachable!()
        };
        assert_eq!(
            choices[0].action,
            Action::Profile {
                device: 9,
                index: 1
            }
        );
    }

    #[test]
    fn a_codec_bluez_chose_is_read_from_the_node() {
        let mut device = headphones();
        device.profiles.push(Profile {
            index: 5,
            name: "a2dp-sink".into(),
            description: "High Fidelity Playback (A2DP Sink)".into(),
            available: Availability::Yes,
            priority: 9,
        });
        device.profile = device.profiles.last().cloned();
        let row = codec_row(&device, Some("sbc"), 1).expect("a row");
        assert_eq!(labels(&row), (vec!["SBC", "LDAC"], Some(0)));
    }
}
