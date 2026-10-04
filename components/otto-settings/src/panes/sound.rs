//! The sound pane.
//!
//! Rows carrying an `id` are bound to `org.otto.Settings`; rows without one
//! are not wired to the compositor yet.
//!
//! The Output and Input groups are not settings at all: which device plays
//! and records, how loud and whether muted belong to the sound server, which
//! remembers them itself. They are read from and written to the server
//! through [`crate::pulse`], on threads of their own, the way the Privacy pane
//! treats the permission store. Their identifiers live under `sound.`, which
//! `org.otto.Settings` serves nothing under, and `main.rs` hands them to this
//! module before the bus ever sees them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::Duration;

use crate::discovery::Choice;
use crate::model::{group, untitled, Control, Pane, Row};
use crate::pulse::{self, Direction, Graph};
use crate::settings_client::{self, Value};

const OUTPUT_DEVICE: &str = "sound.output.device";
const OUTPUT_VOLUME: &str = "sound.output.volume";
const OUTPUT_MUTE: &str = "sound.output.mute";
const INPUT_DEVICE: &str = "sound.input.device";
const INPUT_VOLUME: &str = "sound.input.volume";
const INPUT_MUTE: &str = "sound.input.mute";

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

fn device_id(direction: Direction) -> &'static str {
    match direction {
        Direction::Output => OUTPUT_DEVICE,
        Direction::Input => INPUT_DEVICE,
    }
}

fn volume_id(direction: Direction) -> &'static str {
    match direction {
        Direction::Output => OUTPUT_VOLUME,
        Direction::Input => INPUT_VOLUME,
    }
}

fn mute_id(direction: Direction) -> &'static str {
    match direction {
        Direction::Output => OUTPUT_MUTE,
        Direction::Input => INPUT_MUTE,
    }
}

fn direction_of_device(id: &str) -> Option<Direction> {
    match id {
        OUTPUT_DEVICE => Some(Direction::Output),
        INPUT_DEVICE => Some(Direction::Input),
        _ => None,
    }
}

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

/// Follow the server's events while the app runs, so a headset plugged in,
/// a volume key pressed or a device picked in another mixer shows up here.
/// A server that goes away ends the watch; the next time the pane is shown
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
    Volume(Direction, String, u32),
    Mute(Direction, String, bool),
}

impl Write {
    /// Two writes with the same key: only the later one matters.
    fn key(&self) -> (u8, Direction) {
        match self {
            Write::Default(d, _) => (0, *d),
            Write::Volume(d, _, _) => (1, *d),
            Write::Mute(d, _, _) => (2, *d),
        }
    }

    fn run(self) -> Result<(), String> {
        match self {
            Write::Default(d, name) => pulse::set_default(d, &name),
            Write::Volume(d, name, percent) => pulse::set_volume(d, &name, percent),
            Write::Mute(d, name, muted) => pulse::set_mute(d, &name, muted),
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
            latest.retain(|kept| kept.key() != change.key());
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

/// The pop-ups this module owns, for the menu pool built at startup. The
/// rows only exist once the server has answered, which is after the pool is
/// made.
pub fn slot_ids() -> &'static [&'static str] {
    &[OUTPUT_DEVICE, INPUT_DEVICE]
}

/// The devices one of this module's pop-ups offers. `None` for any other.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    let direction = direction_of_device(id)?;
    let Snapshot::Ready(graph) = &*SNAPSHOT.read().unwrap() else {
        return Some(Vec::new());
    };
    Some(
        graph
            .devices(direction)
            .iter()
            .map(|device| Choice {
                label: device.description.clone(),
                value: device.name.clone(),
            })
            .collect(),
    )
}

/// What a device pop-up shows for the device `value` names.
pub fn display(id: &str, value: &str) -> Option<String> {
    let direction = direction_of_device(id)?;
    let Snapshot::Ready(graph) = &*SNAPSHOT.read().unwrap() else {
        return Some(value.to_string());
    };
    Some(
        graph
            .devices(direction)
            .iter()
            .find(|device| device.name == value)
            .map_or_else(|| value.to_string(), |device| device.description.clone()),
    )
}

/// A device was picked in one of this module's pop-ups. Whether `id` is one
/// of them.
pub fn choose(id: &str, value: &str) -> bool {
    let Some(direction) = direction_of_device(id) else {
        return false;
    };
    if let Snapshot::Ready(graph) = &mut *SNAPSHOT.write().unwrap() {
        graph.set_default_name(direction, value);
    }
    write(Write::Default(direction, value.to_string()));
    true
}

/// A volume slider moved or a mute switch flipped. Whether `id` is one of
/// this module's. The row shows the new value straight away; the server is
/// written on the writer thread and read back after.
pub fn apply(id: &str, value: &Value) -> bool {
    let (direction, is_volume) = match id {
        OUTPUT_VOLUME => (Direction::Output, true),
        INPUT_VOLUME => (Direction::Input, true),
        OUTPUT_MUTE => (Direction::Output, false),
        INPUT_MUTE => (Direction::Input, false),
        _ => return false,
    };
    let mut snapshot = SNAPSHOT.write().unwrap();
    let Snapshot::Ready(graph) = &mut *snapshot else {
        return true;
    };
    let Some(device) = graph.default_device_mut(direction) else {
        return true;
    };
    let name = device.name.clone();
    if is_volume {
        let Some(percent) = value.as_f32() else {
            return true;
        };
        let percent = percent.round().clamp(0.0, 100.0) as u32;
        device.volume = percent;
        write(Write::Volume(direction, name, percent));
    } else {
        let Value::Bool(muted) = *value else {
            return true;
        };
        device.muted = muted;
        write(Write::Mute(direction, name, muted));
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

/// The device pop-up, volume and mute of one direction.
fn device_rows(graph: &Graph, direction: Direction) -> Vec<Row> {
    let devices = graph.devices(direction);
    if devices.is_empty() {
        return vec![note(match direction {
            Direction::Output => otto_kit::t!("settings-sound-no-outputs"),
            Direction::Input => otto_kit::t!("settings-sound-no-inputs"),
        })];
    }
    let label = match direction {
        Direction::Output => otto_kit::t!("settings-sound-output-device"),
        Direction::Input => otto_kit::t!("settings-sound-input-device"),
    };
    let current = graph
        .default_device(direction)
        .map(|device| device.name.clone())
        .unwrap_or_default();
    let mut rows = vec![bound(
        Row::new(label, Control::Select(current)).inactive(devices.len() < 2),
        device_id(direction),
    )];

    // The server may have no default, or name one it no longer lists, for
    // the moment between a device leaving and another taking over.
    if let Some(device) = graph.default_device(direction) {
        let volume = device.volume.min(100);
        rows.push(bound(
            Row::new(
                otto_kit::t!("settings-sound-volume"),
                Control::Slider {
                    value: volume as f32,
                    min: 0.0,
                    max: 100.0,
                    readout: format!("{volume}%"),
                },
            ),
            volume_id(direction),
        ));
        rows.push(bound(
            Row::new(
                otto_kit::t!("settings-sound-mute"),
                Control::Toggle(device.muted),
            ),
            mute_id(direction),
        ));
    }
    rows
}

pub fn build() -> Pane {
    let mut groups = Vec::new();
    match &*SNAPSHOT.read().unwrap() {
        // The first read is a moment away. Saying so would only flash; the
        // interface rows below stand on their own until it is in.
        Snapshot::Pending => {}
        Snapshot::Unavailable => {
            groups.push(untitled(vec![note(otto_kit::t!(
                "settings-sound-unavailable"
            ))]));
        }
        Snapshot::Ready(graph) => {
            groups.push(group(
                otto_kit::t!("settings-group-sound-output"),
                device_rows(graph, Direction::Output),
            ));
            groups.push(group(
                otto_kit::t!("settings-group-sound-input"),
                device_rows(graph, Direction::Input),
            ));
        }
    }
    groups.push(untitled(vec![
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
    ]));

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
    use crate::pulse::Device;

    fn device(name: &str, volume: u32, muted: bool) -> Device {
        Device {
            name: name.into(),
            description: format!("{name} speakers"),
            volume,
            muted,
        }
    }

    #[test]
    fn rows_follow_the_default_device() {
        let graph = Graph {
            outputs: vec![device("a", 30, false), device("b", 140, true)],
            default_output: "b".into(),
            ..Default::default()
        };
        let rows = device_rows(&graph, Direction::Output);
        assert!(matches!(&rows[0].control, Control::Select(name) if name == "b"));
        assert!(!rows[0].inactive);
        // Boosted past normal, the slider sits at its end.
        assert!(matches!(&rows[1].control, Control::Slider { value, .. } if *value == 100.0));
        assert!(matches!(rows[2].control, Control::Toggle(true)));
    }

    #[test]
    fn one_device_leaves_nothing_to_pick() {
        let graph = Graph {
            inputs: vec![device("mic", 50, false)],
            default_input: "mic".into(),
            ..Default::default()
        };
        let rows = device_rows(&graph, Direction::Input);
        assert!(rows[0].inactive);
        assert_eq!(rows[0].id, Some(INPUT_DEVICE));
    }

    #[test]
    fn no_devices_says_so() {
        let rows = device_rows(&Graph::default(), Direction::Output);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].id.is_none());
    }
}
