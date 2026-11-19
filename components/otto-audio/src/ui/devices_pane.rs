//! The Devices section: one card per sound device, with its profile, the
//! port each of its outputs and inputs uses, and a Bluetooth device's codec.
//!
//! Every choice is a device param the server keeps, so nothing here is
//! written to the routing file.

// Rust guideline compliant 2026-02-21

use otto_audio_graph::{codec_named, Availability, Device, Direction, Profile};
use otto_kit::components::dropdown::{field as dropdown, DropdownInteraction};

use super::*;

/// The narrowest a device card goes before the grid drops a column.
const DEVICE_CARD_MIN_W: f32 = 420.0;
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
/// The controls' height.
const DEVICE_CONTROL_H: f32 = 28.0;
/// The title and intro above the cards, which stay put while they scroll.
const DEVICES_HEADER_H: f32 = 72.0;
/// The most choices a segmented control holds before a pop-up takes over.
const MAX_SEGMENTS: usize = 5;
/// A segment's room around its label.
const SEGMENT_PAD: f32 = 20.0;

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
struct DeviceCard {
    title: String,
    kind: String,
    /// On the right of the title: what it offers now, or that it is off.
    badge: Option<String>,
    rows: Vec<Row>,
    note: Option<String>,
}

impl DeviceCard {
    fn height(&self) -> f32 {
        let note = if self.note.is_some() {
            DEVICE_NOTE_H
        } else {
            0.0
        };
        DEVICE_HEAD_H + self.rows.len() as f32 * DEVICE_ROW_H + note + DEVICE_PAD - 6.0
    }
}

impl Ui {
    /// The cards' scrolling area, under the section's title.
    pub(super) fn devices_viewport(&self) -> Rect {
        let content = self.content();
        Rect::from_ltrb(
            content.left,
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

    pub(super) fn draw_devices(&mut self, canvas: &Canvas, theme: &Theme) {
        let content = self.content();
        canvas.draw_rect(content, &fill(graph_view::canvas_ground(theme)));
        let left = content.left + 28.0;
        let right = content.right - 28.0;
        let width = (right - left).max(1.0);
        let top = content.top + 24.0;
        Label::new(t!("audio-devices-title"))
            .with_style(styles::TITLE_2_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, top + 12.0)
            .render(canvas);
        let intro_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&intro_font, t!("audio-devices-intro"), width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, top + 38.0)
            .render(canvas);

        let cards: Vec<(u32, DeviceCard)> = self
            .sound_devices()
            .into_iter()
            .map(|device| (device.id, self.device_card(device)))
            .collect();
        let viewport = self.devices_viewport();
        if cards.is_empty() {
            if self.graph.is_synced() {
                Label::new(t!("audio-devices-empty"))
                    .with_style(styles::BODY)
                    .with_color(theme.text_secondary)
                    .centered_at(viewport.center_x(), viewport.center_y())
                    .render(canvas);
            }
            return;
        }

        // A grid of columns at least `DEVICE_CARD_MIN_W` wide; each row of
        // cards is as tall as its tallest.
        let columns = ((width + DEVICE_GAP) / (DEVICE_CARD_MIN_W + DEVICE_GAP))
            .floor()
            .max(1.0) as usize;
        let card_w = (width - (columns - 1) as f32 * DEVICE_GAP) / columns as f32;
        let mut placed = Vec::with_capacity(cards.len());
        let mut y = 4.0;
        for row in cards.chunks(columns) {
            let row_h = row.iter().map(|(_, c)| c.height()).fold(0.0, f32::max);
            for (col, (_, card)) in row.iter().enumerate() {
                let x = left + col as f32 * (card_w + DEVICE_GAP);
                placed.push(Rect::from_xywh(x, y, card_w, card.height()));
            }
            y += row_h + DEVICE_GAP;
        }
        self.devices_scroll.set_viewport(viewport);
        self.devices_scroll.set_content_length(y + 8.0);
        self.devices_scroll.tick();
        let shift = viewport.top - self.devices_scroll.offset();

        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));
        for ((_, card), at) in cards.iter().zip(placed) {
            let rect = at.with_offset((0.0, shift));
            if rect.bottom < viewport.top || rect.top > viewport.bottom {
                continue;
            }
            self.draw_device_card(canvas, rect, card, viewport, theme);
        }
        canvas.restore();
        self.devices_scroll.render(canvas, theme, |_, _| {});
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
            let font = styles::FOOTNOTE.font();
            let w = font.measure_str(badge, None).0;
            Label::new(badge)
                .with_style(styles::FOOTNOTE)
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
        let kind_font = styles::FOOTNOTE.font();
        Label::new(ellipsize(&kind_font, &card.kind, right - left))
            .with_style(styles::FOOTNOTE)
            .with_color(theme.text_secondary)
            .centered_on(left, rect.top + 48.0)
            .render(canvas);

        let label_font = styles::CALLOUT.font();
        let control_left = left + DEVICE_LABEL_W;
        let control_w = (right - control_left).max(1.0);
        for (index, row) in card.rows.iter().enumerate() {
            let cy = rect.top + DEVICE_HEAD_H + index as f32 * DEVICE_ROW_H + DEVICE_ROW_H / 2.0;
            let label = match row {
                Row::Pick { label, .. } | Row::Fact { label, .. } => label,
            };
            Label::new(ellipsize(&label_font, label, DEVICE_LABEL_W - 12.0))
                .with_style(styles::CALLOUT)
                .with_color(theme.text_secondary)
                .centered_on(left, cy)
                .render(canvas);
            let control = Rect::from_xywh(
                control_left,
                cy - DEVICE_CONTROL_H / 2.0,
                control_w,
                DEVICE_CONTROL_H,
            );
            match row {
                Row::Fact { value, .. } => {
                    Label::new(ellipsize(&label_font, value, control_w))
                        .with_style(styles::CALLOUT)
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
            let font = styles::FOOTNOTE.font();
            Label::new(ellipsize(&font, note, right - left))
                .with_style(styles::FOOTNOTE)
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
        let font = styles::CALLOUT.font();
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
            let color = if Some(index) == chosen {
                theme.text_primary
            } else if choice.dimmed {
                theme.text_tertiary
            } else {
                theme.text_secondary
            };
            Label::new(ellipsize(&font, &choice.label, cell.width() - 8.0))
                .with_style(styles::CALLOUT)
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
    let font = styles::CALLOUT.font();
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
