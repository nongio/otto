//! The Devices section's cards for how an output sounds: its tuning, and a
//! Bluetooth device's choice between its best sound and its microphone.
//!
//! The Sound card maps onto the effects in the routing file. A tuning is an
//! output equaliser or speaker correction in the device's chain: choosing
//! one moves it onto the device, None takes it off (it stays in the file,
//! on no device, and is not made), Compare is its On switch, Loudness its
//! preamp (or a speaker correction's output gain), and Protect from clipping
//! is a limiter in the chain, after the tuning. The service makes them.

// Rust guideline compliant 2026-02-21

use otto_audio_graph::{
    support, Device, Effect, EffectKind, EffectSide, Endpoint, Node, Profile, Support,
};
use otto_kit::components::dropdown::{field as dropdown, DropdownInteraction};
use otto_kit::components::slider::{self, SliderInteraction};
use otto_kit::components::toggle::{self, ToggleInteraction};

use super::devices_pane::AirPlayCard;
use super::virtual_pane::wrap;
use super::*;

/// The device list beside the detail.
pub(super) const DEVICE_LIST_W: f32 = 232.0;
/// One device in the list.
pub(super) const LIST_ITEM_H: f32 = 50.0;
/// The row that opens a device's profile, codec and ports.
pub(super) const DISCLOSURE_H: f32 = 32.0;
/// A card's inner margin, its heading, and one labelled row.
const PAD: f32 = 20.0;
const HEAD_H: f32 = 62.0;
const ROW_H: f32 = 42.0;
const LABEL_W: f32 = 170.0;
/// A line of explanation.
const LINE_H: f32 = 20.0;
/// The loudness range, in dB: enough to even out tunings, short of what
/// would clip without the limiter.
const LOUDNESS_MIN: f32 = -12.0;
const LOUDNESS_MAX: f32 = 12.0;
/// The readout after the loudness slider.
const READOUT_W: f32 = 64.0;

/// What the Devices section's detail shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shown {
    Device(u32),
    AirPlay,
}

/// One block of the detail, in the order drawn.
pub(super) enum Block {
    AirPlay(AirPlayCard),
    Sound(SoundCard),
    Bluetooth(BluetoothCard),
    Disclosure,
    Details(super::devices_pane::DeviceCard),
}

/// What a control of the Devices section's detail does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DeviceAction {
    Select(u32),
    SelectAirPlay,
    /// Open or close the profile, codec and ports.
    Details,
    TuningMenu,
    /// The tuning choice at this index of the menu just opened.
    Tuning(usize),
    /// Hear the tuning (`true`) or the device without it.
    Compare(bool),
    /// The loudness slider: dragged.
    Loudness,
    Protect,
    /// Edit the tuning in the Effects section.
    EditTuning,
    /// Switch a Bluetooth headset for calls, and back, or not.
    HeadsetSwitch,
}

/// A choice in the tuning menu.
#[derive(Debug, Clone, PartialEq)]
enum TuningChoice {
    None,
    Effect(String),
    NewEqualiser,
}

/// The section's own state.
#[derive(Debug, Default)]
pub(super) struct DevicesState {
    pub(super) selected: Option<Shown>,
    /// Whether the profile, codec and ports are open.
    pub(super) details: bool,
    /// The choices the open tuning menu offers.
    choices: Vec<TuningChoice>,
    /// The loudness slider's track while it is dragged.
    drag: Option<Rect>,
    unsaved: bool,
}

/// An output's Sound card.
pub(super) struct SoundCard {
    /// The output's `node.name`.
    sink: String,
    /// The tuning on it, if any.
    tuning: Option<Effect>,
    /// Whether a limiter is in its chain.
    protected: bool,
    /// Whether limiters can be made here.
    limiter: Support,
}

impl SoundCard {
    pub(super) fn height(&self) -> f32 {
        let note = if matches!(self.limiter, Support::Missing { .. }) {
            LINE_H
        } else {
            0.0
        };
        HEAD_H + 4.0 * ROW_H + note + PAD - 6.0
    }
}

/// A Bluetooth device's sound-or-microphone card.
pub(super) struct BluetoothCard {
    device: u32,
    name: String,
    hifi: Option<Profile>,
    headset: Option<Profile>,
    /// Which of the two runs now.
    on_headset: bool,
    switch_for_calls: bool,
}

impl BluetoothCard {
    pub(super) fn height(&self, width: f32) -> f32 {
        let lines = wrap(
            &styles::SUBHEADLINE.font(),
            t!("audio-devices-bt-about"),
            width,
            4,
        )
        .len();
        HEAD_H - 20.0 + lines as f32 * LINE_H + 2.0 * ROW_H + PAD
    }
}

impl Ui {
    /// The output node a device plays through now.
    fn sink_of(&self, device: &Device) -> Option<&Node> {
        let id = device.id.to_string();
        self.graph
            .nodes()
            .find(|n| n.prop("device.id") == Some(id.as_str()) && Endpoint::Output.matches(n))
    }

    pub(super) fn sound_card(&self, device: &Device) -> Option<SoundCard> {
        let sink = self.sink_of(device)?.name().to_string();
        let chain = self.routing.output_chain(&sink);
        Some(SoundCard {
            tuning: chain.iter().find(|e| e.is_tuning()).map(|e| (*e).clone()),
            protected: chain.iter().any(|e| e.kind == EffectKind::Limiter),
            limiter: support(EffectKind::Limiter),
            sink,
        })
    }

    pub(super) fn bluetooth_card(&self, device: &Device) -> Option<BluetoothCard> {
        if !device.is_bluetooth() {
            return None;
        }
        let hifi = device.preferred_profile(Profile::is_hifi).cloned();
        let headset = device.preferred_profile(Profile::is_headset).cloned();
        if hifi.is_none() && headset.is_none() {
            return None;
        }
        let name = device.prop("device.name").unwrap_or_default().to_string();
        Some(BluetoothCard {
            device: device.id,
            on_headset: device.profile.as_ref().is_some_and(Profile::is_headset),
            switch_for_calls: self.routing.calls.headset.contains(&name),
            name,
            hifi,
            headset,
        })
    }

    pub(super) fn draw_sound_card(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        card: &SoundCard,
        theme: &Theme,
    ) {
        super::mixer_pane::draw_card(canvas, rect, theme);
        let left = rect.left + PAD;
        let right = rect.right - PAD;
        Label::new(t!("audio-devices-sound"))
            .with_style(styles::HEADLINE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, rect.top + 28.0)
            .render(canvas);
        let font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(
            &font,
            t!("audio-devices-sound-intro"),
            right - left,
        ))
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_on(left, rect.top + 48.0)
        .render(canvas);

        let control_left = left + LABEL_W;
        let control_w = (right - control_left).max(1.0);
        let row = |i: usize| rect.top + HEAD_H + i as f32 * ROW_H + ROW_H / 2.0;
        let labels = [
            t!("audio-devices-tuning"),
            t!("audio-devices-compare"),
            t!("audio-devices-loudness"),
            t!("audio-devices-protect"),
        ];
        let body = styles::BODY.font();
        for (i, label) in labels.iter().enumerate() {
            Label::new(ellipsize(&body, label, LABEL_W - 12.0))
                .with_style(styles::BODY)
                .with_color(theme.text_secondary)
                .centered_on(left, row(i))
                .render(canvas);
        }

        // Tuning: a pop-up of the tunings, and Edit for the one in use.
        let menu = Action::Device(DeviceAction::TuningMenu);
        let field = Rect::from_xywh(
            control_left,
            row(0) - dropdown::HEIGHT / 2.0,
            control_w.min(280.0),
            dropdown::HEIGHT,
        );
        let interaction = if self.menu_open == Some(menu) {
            DropdownInteraction::Open
        } else if self.pressed == Some(menu) {
            DropdownInteraction::Pressed
        } else {
            DropdownInteraction::Normal
        };
        let shown = card.tuning.as_ref().map_or_else(
            || t!("audio-devices-tuning-none").to_string(),
            |e| e.name.clone(),
        );
        dropdown::draw(canvas, field, &shown, interaction, theme);
        self.targets.push((field, menu));
        if card.tuning.is_some() {
            let label = t!("audio-devices-tuning-edit");
            let w = push_button::width(label);
            let button = Rect::from_xywh(
                field.right + 10.0,
                row(0) - push_button::HEIGHT / 2.0,
                w,
                push_button::HEIGHT,
            );
            if button.right <= right {
                self.button(
                    canvas,
                    button,
                    label,
                    Action::Device(DeviceAction::EditTuning),
                    true,
                    theme,
                );
            }
        }

        // Compare: the device with the tuning, and without.
        let tuned = card.tuning.as_ref().is_some_and(|e| e.enabled);
        let track = Rect::from_xywh(
            control_left,
            row(1) - segmented::HEIGHT / 2.0,
            control_w.min(200.0),
            segmented::HEIGHT,
        );
        let labels = [
            t!("audio-devices-compare-off").to_string(),
            t!("audio-devices-compare-on").to_string(),
        ];
        if card.tuning.is_some() {
            self.segments(
                canvas,
                track,
                &labels,
                usize::from(tuned),
                &[
                    Action::Device(DeviceAction::Compare(false)),
                    Action::Device(DeviceAction::Compare(true)),
                ],
                theme,
            );
        } else {
            let segments: Vec<Segment<'_>> = labels.iter().map(|l| Segment::Text(l)).collect();
            canvas.save_layer_alpha(None, 0x66);
            segmented::draw(
                canvas,
                track,
                &segments,
                None,
                SegmentedInteraction::Normal,
                theme,
            );
            canvas.restore();
        }

        // Loudness: the tuning's own gain.
        let gain = card.tuning.as_ref().map(loudness_of);
        let track = Rect::from_ltrb(
            control_left + slider::KNOB_RADIUS,
            row(2) - 10.0,
            (right - READOUT_W - slider::KNOB_RADIUS).min(control_left + 300.0),
            row(2) + 10.0,
        );
        let interaction = match (gain, self.devices.drag) {
            (None, _) => SliderInteraction::Disabled,
            (Some(_), Some(_)) => SliderInteraction::Pressed,
            _ => SliderInteraction::Normal,
        };
        slider::draw(
            canvas,
            track,
            gain.unwrap_or(0.0),
            LOUDNESS_MIN,
            LOUDNESS_MAX,
            None,
            interaction,
            theme,
        );
        if let Some(gain) = gain {
            Label::new(t_owned!("audio-effects-db", value = format!("{gain:+.1}")))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(track.right + slider::KNOB_RADIUS + 12.0, row(2))
                .render(canvas);
            self.targets.push((
                track.with_outset((slider::KNOB_RADIUS, 6.0)),
                Action::Device(DeviceAction::Loudness),
            ));
        }

        // Protect from clipping: a limiter after the tuning.
        let switch = Rect::from_xywh(
            control_left,
            row(3) - toggle::HEIGHT / 2.0,
            toggle::WIDTH,
            toggle::HEIGHT,
        );
        let action = Action::Device(DeviceAction::Protect);
        let can = card.protected || matches!(card.limiter, Support::Ready);
        let interaction = if !can {
            ToggleInteraction::Disabled
        } else if self.pressed == Some(action) {
            ToggleInteraction::Pressed
        } else {
            ToggleInteraction::Normal
        };
        toggle::draw(
            canvas,
            switch,
            toggle::knob_fraction_for(card.protected),
            interaction,
            theme,
        );
        if can {
            self.targets.push((switch, action));
        }
        if let Support::Missing { package, .. } = card.limiter {
            let text = t_owned!("audio-effects-missing", package = package);
            Label::new(ellipsize(&font, &text, right - left))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_tertiary)
                .centered_on(left, row(3) + ROW_H / 2.0 + LINE_H / 2.0)
                .render(canvas);
        }
    }

    pub(super) fn draw_bluetooth_card(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        card: &BluetoothCard,
        theme: &Theme,
    ) {
        super::mixer_pane::draw_card(canvas, rect, theme);
        let left = rect.left + PAD;
        let right = rect.right - PAD;
        Label::new(t!("audio-devices-bt-title"))
            .with_style(styles::HEADLINE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left, rect.top + 28.0)
            .render(canvas);
        let font = styles::SUBHEADLINE.font();
        let mut y = rect.top + 48.0;
        for line in wrap(&font, t!("audio-devices-bt-about"), right - left, 4) {
            Label::new(line)
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(left, y)
                .render(canvas);
            y += LINE_H;
        }
        y += ROW_H / 2.0 - LINE_H / 2.0 + 4.0;

        // The two profiles as a choice.
        let labels = [
            t!("audio-devices-bt-best").to_string(),
            t!("audio-devices-bt-headset").to_string(),
        ];
        let font_chosen = segmented::LABEL_CHOSEN.font();
        let widest = labels
            .iter()
            .map(|l| font_chosen.measure_str(l, None).0 + 28.0)
            .fold(0.0, f32::max);
        let track = Rect::from_xywh(
            left,
            y - segmented::HEIGHT / 2.0,
            (widest * 2.0).min(right - left),
            segmented::HEIGHT,
        );
        let profile_action = |p: &Option<Profile>| {
            p.as_ref().map(|p| Action::Profile {
                device: card.device,
                index: p.index,
            })
        };
        match (profile_action(&card.hifi), profile_action(&card.headset)) {
            (Some(hifi), Some(headset)) => self.segments(
                canvas,
                track,
                &labels,
                usize::from(card.on_headset),
                &[hifi, headset],
                theme,
            ),
            _ => {
                let segments: Vec<Segment<'_>> = labels.iter().map(|l| Segment::Text(l)).collect();
                segmented::draw(
                    canvas,
                    track,
                    &segments,
                    Some(usize::from(card.on_headset)),
                    SegmentedInteraction::Locked,
                    theme,
                );
            }
        }
        y += ROW_H;

        // Switching for calls.
        let switch = Rect::from_xywh(
            left,
            y - toggle::HEIGHT / 2.0,
            toggle::WIDTH,
            toggle::HEIGHT,
        );
        let action = Action::Device(DeviceAction::HeadsetSwitch);
        let usable = card.headset.is_some() && !card.name.is_empty();
        let interaction = if !usable {
            ToggleInteraction::Disabled
        } else if self.pressed == Some(action) {
            ToggleInteraction::Pressed
        } else {
            ToggleInteraction::Normal
        };
        toggle::draw(
            canvas,
            switch,
            toggle::knob_fraction_for(card.switch_for_calls),
            interaction,
            theme,
        );
        if usable {
            self.targets.push((switch, action));
        }
        let body = styles::BODY.font();
        Label::new(ellipsize(
            &body,
            t!("audio-devices-bt-switch"),
            right - switch.right - 12.0,
        ))
        .with_style(styles::BODY)
        .with_color(theme.text_primary)
        .centered_on(switch.right + 12.0, y)
        .render(canvas);
    }

    /// Open the tuning menu `action` asks for; `false` when it asks for none.
    pub(super) fn open_tuning_menu(&mut self, action: Action, rect: Rect, serial: u32) -> bool {
        if action != Action::Device(DeviceAction::TuningMenu) {
            return false;
        }
        let Some(card) = self.shown_sound_card() else {
            return true;
        };
        let mut options = vec![t!("audio-devices-tuning-none").to_string()];
        let mut choices = vec![TuningChoice::None];
        let mut selected = Some(0);
        for effect in self.routing.effects().iter().filter(|e| e.is_tuning()) {
            if card.tuning.as_ref().is_some_and(|t| t.id == effect.id) {
                selected = Some(options.len());
            }
            options.push(effect.name.clone());
            choices.push(TuningChoice::Effect(effect.id.clone()));
        }
        options.push(t!("audio-devices-tuning-new").to_string());
        choices.push(TuningChoice::NewEqualiser);
        self.menu_choices = (0..choices.len())
            .map(|i| Action::Device(DeviceAction::Tuning(i)))
            .collect();
        self.devices.choices = choices;
        self.menu_open = Some(action);
        self.menu_request = Some(MenuRequest {
            rect,
            serial,
            options,
            selected,
        });
        true
    }

    fn shown_sound_card(&self) -> Option<SoundCard> {
        let Shown::Device(id) = self.shown_device() else {
            return None;
        };
        self.sound_card(self.graph.device(id)?)
    }

    /// Start the loudness drag; `false` for any other control.
    pub(super) fn press_device(&mut self, action: Action, rect: Rect, p: Point) -> bool {
        if action != Action::Device(DeviceAction::Loudness) {
            return false;
        }
        self.devices.drag = Some(rect.with_inset((slider::KNOB_RADIUS, 6.0)));
        self.drag_device(p);
        true
    }

    /// Follow the loudness drag; `false` when there is none.
    pub(super) fn drag_device(&mut self, p: Point) -> bool {
        let Some(track) = self.devices.drag else {
            return false;
        };
        let Some(tuning) = self.shown_sound_card().and_then(|c| c.tuning) else {
            return false;
        };
        let value = slider::value_at(track, LOUDNESS_MIN, LOUDNESS_MAX, Some(0.5), p.x);
        self.change_effect(&tuning.id, true, |e| set_loudness(e, value));
        self.devices.unsaved = true;
        true
    }

    /// End the loudness drag; `true` when one ended.
    pub(super) fn release_device(&mut self) -> bool {
        if self.devices.drag.take().is_none() {
            return false;
        }
        if std::mem::take(&mut self.devices.unsaved) {
            self.flush_effects();
        }
        true
    }

    pub(super) fn run_device(&mut self, action: DeviceAction) {
        match action {
            DeviceAction::Select(id) => {
                self.devices.selected = Some(Shown::Device(id));
                self.devices_scroll.scroll_to(0.0);
            }
            DeviceAction::SelectAirPlay => {
                self.devices.selected = Some(Shown::AirPlay);
                self.devices_scroll.scroll_to(0.0);
            }
            DeviceAction::Details => self.devices.details = !self.devices.details,
            DeviceAction::Tuning(index) => {
                let Some(choice) = self.devices.choices.get(index).cloned() else {
                    return;
                };
                self.choose_tuning(choice);
            }
            DeviceAction::Compare(on) => {
                if let Some(tuning) = self.shown_sound_card().and_then(|c| c.tuning) {
                    self.change_effect(&tuning.id, false, |e| e.enabled = on);
                }
            }
            DeviceAction::Protect => self.toggle_protect(),
            DeviceAction::EditTuning => {
                if let Some(tuning) = self.shown_sound_card().and_then(|c| c.tuning) {
                    self.select_effect(&tuning.id);
                    self.set_pane(Pane::Effects);
                }
            }
            DeviceAction::HeadsetSwitch => {
                let Shown::Device(id) = self.shown_device() else {
                    return;
                };
                let Some(card) = self.graph.device(id).and_then(|d| self.bluetooth_card(d)) else {
                    return;
                };
                let list = &mut self.routing.calls.headset;
                if card.switch_for_calls {
                    list.retain(|n| *n != card.name);
                } else {
                    list.push(card.name);
                }
                self.save_routing_file();
            }
            // Opened on the press, or dragged.
            DeviceAction::TuningMenu | DeviceAction::Loudness => {}
        }
    }

    fn choose_tuning(&mut self, choice: TuningChoice) {
        let Some(card) = self.shown_sound_card() else {
            return;
        };
        let current = card.tuning.as_ref().map(|e| e.id.clone());
        match choice {
            TuningChoice::None => {
                if let Some(id) = current {
                    self.routing.move_effect(&id, "");
                }
            }
            TuningChoice::Effect(id) => {
                if current.as_deref() == Some(id.as_str()) {
                    return;
                }
                if let Some(old) = current {
                    self.routing.move_effect(&old, "");
                }
                // On top of the chain, so a limiter already there stays
                // after it.
                let entry = self.routing.output_entry(&card.sink);
                self.routing.move_effect(&id, &entry);
            }
            TuningChoice::NewEqualiser => {
                if let Some(old) = current {
                    self.routing.move_effect(&old, "");
                }
                let title = self
                    .graph
                    .nodes()
                    .find(|n| n.name() == card.sink)
                    .map_or_else(|| card.sink.clone(), |n| n.title().to_string());
                let name = t_owned!("audio-devices-tuning-new-name", device = title);
                let entry = self.routing.output_entry(&card.sink);
                let id = self.routing.add_effect(
                    &name,
                    EffectKind::Equaliser,
                    EffectSide::Output,
                    &entry,
                );
                self.select_effect(&id);
            }
        }
        self.save_routing_file();
    }

    fn toggle_protect(&mut self) {
        let Some(card) = self.shown_sound_card() else {
            return;
        };
        if card.protected {
            let limiters: Vec<String> = self
                .routing
                .output_chain(&card.sink)
                .iter()
                .filter(|e| e.kind == EffectKind::Limiter)
                .map(|e| e.id.clone())
                .collect();
            for id in limiters {
                self.routing.remove_effect(&id);
            }
        } else {
            let name = t!("audio-effects-kind-limiter");
            match &card.tuning {
                Some(tuning) => {
                    self.routing
                        .add_effect_after(&tuning.id, name, EffectKind::Limiter);
                }
                None => {
                    // Under whatever else is on the device, nearest it.
                    let id =
                        self.routing
                            .add_effect(name, EffectKind::Limiter, EffectSide::Output, "");
                    self.routing.move_effect(&id, &card.sink);
                }
            }
        }
        self.save_routing_file();
    }
}

/// A tuning's loudness: an equaliser's preamp, a speaker correction's
/// output gain.
fn loudness_of(effect: &Effect) -> f32 {
    match effect.kind {
        EffectKind::SpeakerCorrection => effect.params.get("gain").copied().unwrap_or(0.0),
        _ => effect.preamp,
    }
}

fn set_loudness(effect: &mut Effect, value: f32) {
    match effect.kind {
        EffectKind::SpeakerCorrection => {
            effect.params.insert("gain".into(), value);
        }
        _ => effect.preamp = value,
    }
}
