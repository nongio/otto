//! The sound pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.
//!
//! Below the interface-sound settings sits a mixer laid out after
//! pavucontrol, the reference for what it covers: a Show pop-up stands in
//! for pavucontrol's tabs — Playback, Recording, Output devices, Input
//! devices, Configuration — and the groups under it are that tab's. Each app
//! playing or recording has its volume, mute and device; each device its
//! port, volume, mute and whether it is the default; each card its profile.
//!
//! None of that is a setting. It belongs to the sound server, which
//! remembers it itself, and is read and written through [`crate::pulse`] on
//! threads of their own, the way the Privacy pane treats the permission
//! store. The mixer's identifiers live under `sound.`, which
//! `org.otto.Settings` serves nothing under, and `main.rs` hands them to this
//! module before the bus ever sees them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::Duration;

use crate::discovery::Choice;
use crate::model::{group, untitled, Control, Group, Pane, Row};
use crate::pulse::{self, Direction, Graph};
use crate::settings_client::{self, Value};

/// The Show pop-up: which of pavucontrol's tabs the mixer shows.
const VIEW_ID: &str = "sound.view";

/// pavucontrol's tabs, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Playback,
    Recording,
    Output,
    Input,
    Configuration,
}

impl View {
    const ALL: [View; 5] = [
        View::Playback,
        View::Recording,
        View::Output,
        View::Input,
        View::Configuration,
    ];

    fn token(self) -> &'static str {
        match self {
            View::Playback => "playback",
            View::Recording => "recording",
            View::Output => "output",
            View::Input => "input",
            View::Configuration => "configuration",
        }
    }

    fn label(self) -> &'static str {
        match self {
            View::Playback => otto_kit::t!("settings-sound-view-playback"),
            View::Recording => otto_kit::t!("settings-sound-view-recording"),
            View::Output => otto_kit::t!("settings-group-sound-output"),
            View::Input => otto_kit::t!("settings-group-sound-input"),
            View::Configuration => otto_kit::t!("settings-sound-view-configuration"),
        }
    }

    fn from_token(token: &str) -> Option<View> {
        View::ALL.into_iter().find(|view| view.token() == token)
    }
}

/// Which tab is shown. Output devices first: what a Settings pane is opened
/// for far more often than to move one app's sound.
static VIEW: Mutex<View> = Mutex::new(View::Output);

/// What the pane last learned from the sound server.
enum Snapshot {
    /// Not asked yet.
    Pending,
    /// Nothing answered: no server, or no `pactl` to ask it with.
    Unavailable,
    Ready(Graph),
}

static SNAPSHOT: RwLock<Snapshot> = RwLock::new(Snapshot::Pending);
static SHOWN: AtomicBool = AtomicBool::new(false);
static WATCHING: AtomicBool = AtomicBool::new(false);
static RELOAD_QUEUED: AtomicBool = AtomicBool::new(false);

/// Read the server and keep what it says, then have the pane redrawn.
fn reload() {
    let snapshot = match pulse::read() {
        Ok(graph) => Snapshot::Ready(graph),
        Err(why) => {
            eprintln!("sound: {why}");
            Snapshot::Unavailable
        }
    };
    *SNAPSHOT.write().unwrap() = snapshot;
    settings_client::request_redraw();
}

/// Read the server again shortly. A volume drag or a device plugged in sets
/// off a burst of events; they share one read.
fn reload_soon() {
    if RELOAD_QUEUED.swap(true, Ordering::Relaxed) {
        return;
    }
    in_background("sound-read", || {
        std::thread::sleep(Duration::from_millis(80));
        RELOAD_QUEUED.store(false, Ordering::Relaxed);
        reload();
    });
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("sound: could not start {name}: {err}");
    }
}

/// Follow the server's events while the app runs, so an app starting to
/// play, a headset plugged in or a volume key pressed shows up here. A
/// server that goes away ends the watch; the next time the pane is shown
/// starts it again.
fn watch() {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    in_background("sound-watch", || {
        let outcome = pulse::subscribe(|| {
            if SHOWN.load(Ordering::Relaxed) {
                reload_soon();
            }
        });
        if let Err(why) = outcome {
            eprintln!("sound: cannot follow the sound server: {why}");
        }
        WATCHING.store(false, Ordering::Relaxed);
        if SHOWN.load(Ordering::Relaxed) {
            reload_soon();
        }
    });
}

/// Tell the pane whether it is on screen. Coming on screen reads the server
/// afresh, showing what was read last until the answer is in.
pub fn set_shown(shown: bool) {
    let was = SHOWN.swap(shown, Ordering::Relaxed);
    if shown && !was {
        watch();
        in_background("sound-read", reload);
    }
}

/// One change for the server.
enum Write {
    Default(Direction, String),
    Port(Direction, String, String),
    Volume(Direction, String, u32),
    Mute(Direction, String, bool),
    Move(Direction, u32, String),
    StreamVolume(Direction, u32, u32),
    StreamMute(Direction, u32, bool),
    Profile(String, String),
}

impl Write {
    /// Two writes with the same key: only the later one matters.
    fn key(&self) -> String {
        match self {
            Write::Default(d, _) => format!("default {d:?}"),
            Write::Port(d, name, _) => format!("port {d:?} {name}"),
            Write::Volume(d, name, _) => format!("volume {d:?} {name}"),
            Write::Mute(d, name, _) => format!("mute {d:?} {name}"),
            Write::Move(d, index, _) => format!("move {d:?} {index}"),
            Write::StreamVolume(d, index, _) => format!("stream-volume {d:?} {index}"),
            Write::StreamMute(d, index, _) => format!("stream-mute {d:?} {index}"),
            Write::Profile(card, _) => format!("profile {card}"),
        }
    }

    fn run(self) -> Result<(), String> {
        match self {
            Write::Default(d, name) => pulse::set_default(d, &name),
            Write::Port(d, name, port) => pulse::set_port(d, &name, &port),
            Write::Volume(d, name, percent) => pulse::set_volume(d, &name, percent),
            Write::Mute(d, name, muted) => pulse::set_mute(d, &name, muted),
            Write::Move(d, index, device) => pulse::move_stream(d, index, &device),
            Write::StreamVolume(d, index, percent) => pulse::set_stream_volume(d, index, percent),
            Write::StreamMute(d, index, muted) => pulse::set_stream_mute(d, index, muted),
            Write::Profile(card, profile) => pulse::set_profile(&card, &profile),
        }
    }
}

/// Hand a write to the one thread that makes them, in order.
///
/// A slider drag asks for a new volume on every pointer motion, far faster
/// than `pactl` starts. The writer takes everything queued since its last
/// round and runs only the latest of each kind, so the volume follows the
/// pointer instead of trailing it through every value it crossed.
fn write(change: Write) {
    static WRITER: OnceLock<Mutex<Sender<Write>>> = OnceLock::new();
    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        in_background("sound-write", move || writer(receiver));
        Mutex::new(sender)
    });
    let _ = sender.lock().unwrap().send(change);
}

fn writer(receiver: Receiver<Write>) {
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        batch.extend(receiver.try_iter());
        let mut latest: Vec<Write> = Vec::new();
        for change in batch {
            let key = change.key();
            latest.retain(|kept| kept.key() != key);
            latest.push(change);
        }
        for change in latest {
            if let Err(why) = change.run() {
                eprintln!("sound: {why}");
            }
        }
        // The server's own event would bring the answer too, but only while
        // the watch runs; reading back here puts a refused change right.
        reload_soon();
    }
}

/// `text` as a `&'static str`, kept once however often it is asked for.
///
/// Rows want `'static` identifiers, and the mixer's are made from the
/// device names and stream numbers the server hands out. They are few and
/// repeat from one read to the next.
fn intern(text: String) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut interned = INTERNED.get_or_init(Default::default).lock().unwrap();
    interned
        .entry(text)
        .or_insert_with_key(|text| text.clone().leak())
}

fn side(direction: Direction) -> &'static str {
    match direction {
        Direction::Output => "out",
        Direction::Input => "in",
    }
}

/// What a switch or slider of the mixer edits, as its identifier spells it:
/// `sound.device.<out|in>.<name>.<field>` or
/// `sound.stream.<out|in>.<index>.<field>`. A device name has dots in it,
/// so the field is split off the end.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Device(Direction, String, Field),
    Stream(Direction, u32, Field),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Volume,
    Mute,
    Default,
}

impl Field {
    fn word(self) -> &'static str {
        match self {
            Field::Volume => "volume",
            Field::Mute => "mute",
            Field::Default => "default",
        }
    }
}

fn device_control(direction: Direction, name: &str, field: Field) -> &'static str {
    intern(format!(
        "sound.device.{}.{name}.{}",
        side(direction),
        field.word()
    ))
}

fn stream_control(direction: Direction, index: u32, field: Field) -> &'static str {
    intern(format!(
        "sound.stream.{}.{index}.{}",
        side(direction),
        field.word()
    ))
}

fn parse_control(id: &str) -> Option<Target> {
    let (kind, rest) = id.strip_prefix("sound.")?.split_once('.')?;
    let (direction, rest) = rest.split_once('.')?;
    let direction = match direction {
        "out" => Direction::Output,
        "in" => Direction::Input,
        _ => return None,
    };
    let (target, field) = rest.rsplit_once('.')?;
    let field = match field {
        "volume" => Field::Volume,
        "mute" => Field::Mute,
        "default" => Field::Default,
        _ => return None,
    };
    match kind {
        "device" => Some(Target::Device(direction, target.to_string(), field)),
        "stream" => Some(Target::Stream(direction, target.parse().ok()?, field)),
        _ => None,
    }
}

/// What one of the mixer's pop-ups picks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Menu {
    /// A device's port.
    Port(Direction, String),
    /// The device an app's stream plays on or records from.
    StreamDevice(Direction, u32),
    /// A card's profile.
    Profile(String),
}

/// How many pop-ups the mixer can show at once. The menus are made at
/// startup, before anything is known of the server, so there is a pool of
/// them and the pane hands them out each time it is built. A row past the
/// last says its choice as text.
const SLOTS: usize = 32;

fn menu_slots() -> &'static [&'static str] {
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| {
        (0..SLOTS)
            .map(|i| &*format!("sound.menu.{i}").leak())
            .collect()
    })
}

/// Which pop-up each slot was given when the pane was last built.
static SLOT_TARGETS: Mutex<Vec<Menu>> = Mutex::new(Vec::new());

fn slot_target(id: &str) -> Option<Menu> {
    let index = menu_slots().iter().position(|slot| *slot == id)?;
    SLOT_TARGETS.lock().unwrap().get(index).cloned()
}

/// The pop-ups this module owns, for the menu pool built at startup.
pub fn slot_ids() -> Vec<&'static str> {
    let mut ids = vec![VIEW_ID];
    ids.extend_from_slice(menu_slots());
    ids
}

/// The choices of a pop-up, as (value, label).
fn choices(graph: &Graph, menu: &Menu) -> Vec<(String, String)> {
    match menu {
        Menu::Port(direction, name) => graph
            .device(*direction, name)
            .map(|device| {
                device
                    .ports
                    .iter()
                    .map(|port| (port.name.clone(), port.description.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        Menu::StreamDevice(direction, _) => graph
            .devices(*direction)
            .iter()
            .map(|device| (device.name.clone(), device.description.clone()))
            .collect(),
        Menu::Profile(card) => graph
            .cards
            .iter()
            .find(|c| c.name == *card)
            .map(|card| {
                card.profiles
                    .iter()
                    .map(|profile| (profile.name.clone(), profile.description.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The choices one of this module's pop-ups offers. `None` for any other.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    if id == VIEW_ID {
        return Some(
            View::ALL
                .into_iter()
                .map(|view| Choice {
                    label: view.label().to_string(),
                    value: view.token().to_string(),
                })
                .collect(),
        );
    }
    let menu = slot_target(id)?;
    let Snapshot::Ready(graph) = &*SNAPSHOT.read().unwrap() else {
        return Some(Vec::new());
    };
    Some(
        choices(graph, &menu)
            .into_iter()
            .map(|(value, label)| Choice { label, value })
            .collect(),
    )
}

/// What one of this module's pop-ups shows for `value`.
pub fn display(id: &str, value: &str) -> Option<String> {
    if id == VIEW_ID {
        return View::from_token(value).map(|view| view.label().to_string());
    }
    let menu = slot_target(id)?;
    let Snapshot::Ready(graph) = &*SNAPSHOT.read().unwrap() else {
        return Some(String::new());
    };
    Some(
        choices(graph, &menu)
            .into_iter()
            .find(|(v, _)| v == value)
            .map_or_else(|| value.to_string(), |(_, label)| label),
    )
}

/// A choice was picked in one of this module's pop-ups. Whether `id` is one
/// of them. The row shows the choice straight away; the server is written
/// on the writer thread and read back after.
pub fn choose(id: &str, value: &str) -> bool {
    if id == VIEW_ID {
        if let Some(view) = View::from_token(value) {
            *VIEW.lock().unwrap() = view;
        }
        return true;
    }
    let Some(menu) = slot_target(id) else {
        return false;
    };
    let mut snapshot = SNAPSHOT.write().unwrap();
    let Snapshot::Ready(graph) = &mut *snapshot else {
        return true;
    };
    match menu {
        Menu::Port(direction, name) => {
            if let Some(device) = graph.device_mut(direction, &name) {
                device.active_port = Some(value.to_string());
            }
            write(Write::Port(direction, name, value.to_string()));
        }
        Menu::StreamDevice(direction, index) => {
            let Some(target) = graph.device(direction, value).map(|d| d.index) else {
                return true;
            };
            if let Some(stream) = graph.stream_mut(direction, index) {
                stream.device = target;
            }
            write(Write::Move(direction, index, value.to_string()));
        }
        Menu::Profile(card) => {
            if let Some(shown) = graph.cards.iter_mut().find(|c| c.name == card) {
                shown.active_profile = value.to_string();
            }
            write(Write::Profile(card, value.to_string()));
        }
    }
    true
}

/// A slider moved or a switch flipped in the mixer. Whether `id` is one of
/// this module's. The row shows the new value straight away; the server is
/// written on the writer thread and read back after.
pub fn apply(id: &str, value: &Value) -> bool {
    let Some(target) = parse_control(id) else {
        return false;
    };
    let mut snapshot = SNAPSHOT.write().unwrap();
    let Snapshot::Ready(graph) = &mut *snapshot else {
        return true;
    };
    let percent = value
        .as_f32()
        .map(|percent| percent.round().clamp(0.0, 100.0) as u32);
    let on = match *value {
        Value::Bool(on) => Some(on),
        _ => None,
    };
    match target {
        Target::Device(direction, name, field) => match field {
            Field::Volume => {
                let (Some(percent), Some(device)) = (percent, graph.device_mut(direction, &name))
                else {
                    return true;
                };
                device.volume = percent;
                write(Write::Volume(direction, name, percent));
            }
            Field::Mute => {
                let (Some(muted), Some(device)) = (on, graph.device_mut(direction, &name)) else {
                    return true;
                };
                device.muted = muted;
                write(Write::Mute(direction, name, muted));
            }
            // As pavucontrol's fallback button: switching one on makes it the
            // default. The default cannot be switched off, only replaced, so
            // switching it off leaves it on.
            Field::Default => {
                if on == Some(true) {
                    graph.set_default_name(direction, &name);
                    write(Write::Default(direction, name));
                }
            }
        },
        Target::Stream(direction, index, field) => match field {
            Field::Volume => {
                let (Some(percent), Some(stream)) = (percent, graph.stream_mut(direction, index))
                else {
                    return true;
                };
                stream.volume = percent;
                write(Write::StreamVolume(direction, index, percent));
            }
            Field::Mute => {
                let (Some(muted), Some(stream)) = (on, graph.stream_mut(direction, index)) else {
                    return true;
                };
                stream.muted = muted;
                write(Write::StreamMute(direction, index, muted));
            }
            Field::Default => {}
        },
    }
    true
}

/// A row that says something rather than holding a value.
fn note(text: &'static str) -> Row {
    Row::new(text, Control::Value(String::new()))
}

fn bound(mut row: Row, id: &'static str) -> Row {
    row.id = Some(id);
    row
}

/// Volume runs to normal, 100 %. A device or app boosted past it shows at
/// the end of the track, and touching the slider brings it back to normal.
fn volume_row(id: &'static str, volume: u32) -> Row {
    let volume = volume.min(100);
    bound(
        Row::new(
            otto_kit::t!("settings-sound-volume"),
            Control::Slider {
                value: volume as f32,
                min: 0.0,
                max: 100.0,
                readout: format!("{volume}%"),
            },
        ),
        id,
    )
}

fn mute_row(id: &'static str, muted: bool) -> Row {
    bound(
        Row::new(otto_kit::t!("settings-sound-mute"), Control::Toggle(muted)),
        id,
    )
}

/// Hands out the pool's pop-ups while the pane is built, and remembers what
/// each was given.
struct Menus {
    targets: Vec<Menu>,
}

impl Menus {
    /// A pop-up row for `menu` showing `current`, or the choice as plain
    /// text, `shown`, once the pool has run out.
    fn row(&mut self, label: &'static str, menu: Menu, current: String, shown: String) -> Row {
        match menu_slots().get(self.targets.len()) {
            Some(slot) => {
                self.targets.push(menu);
                bound(Row::new(label, Control::Select(current)), slot)
            }
            None => Row::new(label, Control::Value(shown)),
        }
    }
}

/// One group per app playing (or recording): its volume, mute, and the
/// device it plays on.
fn stream_groups(graph: &Graph, direction: Direction, menus: &mut Menus) -> Vec<Group> {
    let streams = graph.streams(direction);
    if streams.is_empty() {
        return vec![untitled(vec![note(match direction {
            Direction::Output => otto_kit::t!("settings-sound-no-playback"),
            Direction::Input => otto_kit::t!("settings-sound-no-recording"),
        })])];
    }
    let device_label = match direction {
        Direction::Output => otto_kit::t!("settings-sound-output-device"),
        Direction::Input => otto_kit::t!("settings-sound-input-device"),
    };
    streams
        .iter()
        .map(|stream| {
            let title = if stream.media.is_empty() {
                stream.app.clone()
            } else {
                format!("{} — {}", stream.app, stream.media)
            };
            let device = graph
                .devices(direction)
                .iter()
                .find(|device| device.index == stream.device);
            let rows = vec![
                volume_row(
                    stream_control(direction, stream.index, Field::Volume),
                    stream.volume,
                ),
                mute_row(
                    stream_control(direction, stream.index, Field::Mute),
                    stream.muted,
                ),
                menus.row(
                    device_label,
                    Menu::StreamDevice(direction, stream.index),
                    device.map(|d| d.name.clone()).unwrap_or_default(),
                    device.map(|d| d.description.clone()).unwrap_or_default(),
                ),
            ];
            group(title, rows)
        })
        .collect()
}

/// One group per device: its port, volume, mute, and whether it is the
/// default.
fn device_groups(graph: &Graph, direction: Direction, menus: &mut Menus) -> Vec<Group> {
    let devices = graph.devices(direction);
    if devices.is_empty() {
        return vec![untitled(vec![note(match direction {
            Direction::Output => otto_kit::t!("settings-sound-no-outputs"),
            Direction::Input => otto_kit::t!("settings-sound-no-inputs"),
        })])];
    }
    devices
        .iter()
        .map(|device| {
            let mut rows = Vec::new();
            if !device.ports.is_empty() {
                let current = device.active_port.clone().unwrap_or_default();
                let shown = device
                    .ports
                    .iter()
                    .find(|port| port.name == current)
                    .map(|port| port.description.clone())
                    .unwrap_or_default();
                rows.push(menus.row(
                    otto_kit::t!("settings-sound-port"),
                    Menu::Port(direction, device.name.clone()),
                    current,
                    shown,
                ));
            }
            rows.push(volume_row(
                device_control(direction, &device.name, Field::Volume),
                device.volume,
            ));
            rows.push(mute_row(
                device_control(direction, &device.name, Field::Mute),
                device.muted,
            ));
            rows.push(bound(
                Row::new(
                    otto_kit::t!("settings-sound-default"),
                    Control::Toggle(device.name == graph.default_name(direction)),
                ),
                device_control(direction, &device.name, Field::Default),
            ));
            group(device.description.clone(), rows)
        })
        .collect()
}

/// One group per card: the profile it runs in.
fn card_groups(graph: &Graph, menus: &mut Menus) -> Vec<Group> {
    if graph.cards.is_empty() {
        return vec![untitled(vec![note(otto_kit::t!(
            "settings-sound-no-cards"
        ))])];
    }
    graph
        .cards
        .iter()
        .map(|card| {
            let shown = card
                .profiles
                .iter()
                .find(|profile| profile.name == card.active_profile)
                .map(|profile| profile.description.clone())
                .unwrap_or_default();
            group(
                card.description.clone(),
                vec![menus.row(
                    otto_kit::t!("settings-sound-profile"),
                    Menu::Profile(card.name.clone()),
                    card.active_profile.clone(),
                    shown,
                )],
            )
        })
        .collect()
}

/// The groups of `view`.
fn mixer_groups(graph: &Graph, view: View, menus: &mut Menus) -> Vec<Group> {
    match view {
        View::Playback => stream_groups(graph, Direction::Output, menus),
        View::Recording => stream_groups(graph, Direction::Input, menus),
        View::Output => device_groups(graph, Direction::Output, menus),
        View::Input => device_groups(graph, Direction::Input, menus),
        View::Configuration => card_groups(graph, menus),
    }
}

pub fn build() -> Pane {
    let mut groups = vec![untitled(vec![
        Row::new(
            otto_kit::t!("settings-interface-sounds"),
            Control::Toggle(true),
        )
        .id("audio.sound_enabled"),
        Row::new(
            otto_kit::t!("settings-sound-theme"),
            Control::Select("Auto".into()),
        )
        .id("audio.sound_theme"),
    ])];

    let view = *VIEW.lock().unwrap();
    let mut menus = Menus {
        targets: Vec::new(),
    };
    match &*SNAPSHOT.read().unwrap() {
        // The first read is a moment away. Saying so would only flash; the
        // interface rows stand on their own until it is in.
        Snapshot::Pending => {}
        Snapshot::Unavailable => {
            groups.push(untitled(vec![note(otto_kit::t!(
                "settings-sound-unavailable"
            ))]));
        }
        Snapshot::Ready(graph) => {
            groups.push(untitled(vec![bound(
                Row::new(
                    otto_kit::t!("settings-sound-show"),
                    Control::Select(view.token().to_string()),
                ),
                VIEW_ID,
            )]));
            groups.extend(mixer_groups(graph, view, &mut menus));
        }
    }
    *SLOT_TARGETS.lock().unwrap() = menus.targets;

    Pane {
        name: otto_kit::t!("settings-pane-sound"),
        icon: "sound",
        intro: None,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pulse::{Card, Device, Port, Profile, Stream};

    fn device(index: u32, name: &str) -> Device {
        Device {
            index,
            name: name.into(),
            description: format!("{name} speakers"),
            volume: 40,
            muted: false,
            ports: Vec::new(),
            active_port: None,
        }
    }

    fn graph() -> Graph {
        let mut laptop = device(55, "alsa.laptop");
        laptop.ports = vec![
            Port {
                name: "speaker".into(),
                description: "Speakers".into(),
            },
            Port {
                name: "headphones".into(),
                description: "Headphones".into(),
            },
        ];
        laptop.active_port = Some("speaker".into());
        Graph {
            outputs: vec![laptop, device(60, "usb")],
            playback: vec![Stream {
                index: 7,
                app: "Firefox".into(),
                media: "Tiny Desk".into(),
                device: 60,
                volume: 140,
                muted: true,
            }],
            cards: vec![Card {
                name: "card0".into(),
                description: "Built-in Audio".into(),
                profiles: vec![Profile {
                    name: "duplex".into(),
                    description: "Analog Stereo Duplex".into(),
                }],
                active_profile: "duplex".into(),
            }],
            default_output: "alsa.laptop".into(),
            ..Default::default()
        }
    }

    fn menus() -> Menus {
        Menus {
            targets: Vec::new(),
        }
    }

    #[test]
    fn identifiers_survive_dotted_device_names() {
        let name = "alsa_output.pci-0000_00_1f.3.analog-stereo";
        assert_eq!(
            parse_control(device_control(Direction::Output, name, Field::Mute)),
            Some(Target::Device(Direction::Output, name.into(), Field::Mute))
        );
        assert_eq!(
            parse_control(stream_control(Direction::Input, 42, Field::Volume)),
            Some(Target::Stream(Direction::Input, 42, Field::Volume))
        );
        assert_eq!(parse_control("audio.sound_enabled"), None);
        assert_eq!(parse_control(VIEW_ID), None);
        assert_eq!(parse_control("sound.menu.3"), None);
    }

    #[test]
    fn each_device_has_its_port_volume_mute_and_default() {
        let mut menus = menus();
        let groups = device_groups(&graph(), Direction::Output, &mut menus);
        assert_eq!(groups.len(), 2);
        let laptop = &groups[0].rows;
        assert!(matches!(&laptop[0].control, Control::Select(port) if port == "speaker"));
        assert!(matches!(laptop[3].control, Control::Toggle(true)));
        // No ports, no port row; not the default.
        let usb = &groups[1].rows;
        assert_eq!(usb.len(), 3);
        assert!(matches!(usb[2].control, Control::Toggle(false)));
        assert_eq!(
            menus.targets,
            [Menu::Port(Direction::Output, "alsa.laptop".into())]
        );
    }

    #[test]
    fn each_stream_has_its_volume_mute_and_device() {
        let mut menus = menus();
        let groups = stream_groups(&graph(), Direction::Output, &mut menus);
        assert_eq!(groups[0].title.as_deref(), Some("Firefox — Tiny Desk"));
        let rows = &groups[0].rows;
        // Boosted past normal, the slider sits at its end.
        assert!(matches!(&rows[0].control, Control::Slider { value, .. } if *value == 100.0));
        assert!(matches!(rows[1].control, Control::Toggle(true)));
        assert!(matches!(&rows[2].control, Control::Select(device) if device == "usb"));
        assert_eq!(choices(&graph(), &menus.targets[0]).len(), 2);
    }

    #[test]
    fn nothing_recording_says_so() {
        let groups = stream_groups(&graph(), Direction::Input, &mut menus());
        assert_eq!(groups.len(), 1);
        assert!(groups[0].rows[0].id.is_none());
    }

    #[test]
    fn a_card_offers_its_profiles() {
        let mut menus = menus();
        let groups = card_groups(&graph(), &mut menus);
        assert_eq!(groups[0].title.as_deref(), Some("Built-in Audio"));
        assert!(matches!(&groups[0].rows[0].control, Control::Select(p) if p == "duplex"));
        assert_eq!(menus.targets, [Menu::Profile("card0".into())]);
    }

    #[test]
    fn past_the_pool_a_choice_is_text() {
        let mut menus = menus();
        for _ in 0..SLOTS {
            menus.row("x", Menu::Profile("c".into()), "a".into(), "A".into());
        }
        let row = menus.row("x", Menu::Profile("c".into()), "a".into(), "A".into());
        assert!(matches!(&row.control, Control::Value(text) if text == "A"));
    }
}
