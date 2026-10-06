//! The sound server as `pactl` sees it.
//!
//! The Sound pane speaks neither PipeWire nor the PulseAudio protocol itself:
//! `pactl` does. It talks to whatever answers the Pulse socket — PulseAudio,
//! or PipeWire through pipewire-pulse, which is how Otto's own sessions run —
//! and has printed JSON since PulseAudio 16. Listing devices, app streams and
//! cards, reading volume and mute, changing the default, moving a stream,
//! switching a card's profile and following changes (`pactl subscribe`) are
//! each one command away, with no audio client library linked into the app
//! and no PipeWire graph to walk by hand. What it covers is what pavucontrol
//! covers, which is the pane's reference.
//!
//! Every call blocks on a child process, so callers run them off the draw
//! thread.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use serde::Deserialize;

/// Which way the sound goes: a sink plays it, a source records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Output,
    Input,
}

impl Direction {
    /// The word `pactl` uses for this kind of device in its subcommands.
    fn noun(self) -> &'static str {
        match self {
            Direction::Output => "sink",
            Direction::Input => "source",
        }
    }

    /// The word `pactl` uses for an app's stream to or from such a device.
    fn stream_noun(self) -> &'static str {
        match self {
            Direction::Output => "sink-input",
            Direction::Input => "source-output",
        }
    }
}

/// One sink or source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The server's number for it, which a stream names its device by.
    pub index: u32,
    /// The server's name for it, which every command addresses it by.
    pub name: String,
    /// What people call it: "Built-in Audio Analog Stereo".
    pub description: String,
    /// Mean of its channels, in percent of normal volume. Over 100 when
    /// something boosted it past normal.
    pub volume: u32,
    pub muted: bool,
    /// Where on the device the sound goes or comes from — a laptop's one
    /// analog device has its speakers and its headphone jack — leaving out
    /// the ones with nothing plugged in.
    pub ports: Vec<Port>,
    /// The port in use, if the device has any.
    pub active_port: Option<String>,
}

/// One connector or transducer on a device: "Speakers", "Headphones",
/// "Internal Microphone".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub description: String,
    /// False for a jack that senses nothing plugged in. Such a port is
    /// still listed, as pavucontrol lists it, so headphones can be picked
    /// before they are plugged in.
    pub available: bool,
}

/// One app's sound on its way to a sink, or from a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub index: u32,
    /// The app: "Firefox".
    pub app: String,
    /// What it plays or records: a tab's title, a track. Empty when the app
    /// does not say.
    pub media: String,
    /// The [`Device::index`] it plays on or records from.
    pub device: u32,
    pub volume: u32,
    pub muted: bool,
}

/// One sound card, and the profiles it can run in: which of its outputs
/// and inputs exist at all. A laptop's HDMI audio is a profile of the same
/// card as its speakers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub name: String,
    pub description: String,
    /// Best first, as the server ranks them, leaving out the ones that
    /// cannot run now — HDMI with no display plugged in.
    pub profiles: Vec<Profile>,
    pub active_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub description: String,
}

/// Everything the server offers, and which device is the default each way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub playback: Vec<Stream>,
    pub recording: Vec<Stream>,
    pub cards: Vec<Card>,
    pub default_output: String,
    pub default_input: String,
}

impl Graph {
    pub fn devices(&self, direction: Direction) -> &[Device] {
        match direction {
            Direction::Output => &self.outputs,
            Direction::Input => &self.inputs,
        }
    }

    pub fn devices_mut(&mut self, direction: Direction) -> &mut Vec<Device> {
        match direction {
            Direction::Output => &mut self.outputs,
            Direction::Input => &mut self.inputs,
        }
    }

    pub fn streams(&self, direction: Direction) -> &[Stream] {
        match direction {
            Direction::Output => &self.playback,
            Direction::Input => &self.recording,
        }
    }

    pub fn streams_mut(&mut self, direction: Direction) -> &mut Vec<Stream> {
        match direction {
            Direction::Output => &mut self.playback,
            Direction::Input => &mut self.recording,
        }
    }

    pub fn default_name(&self, direction: Direction) -> &str {
        match direction {
            Direction::Output => &self.default_output,
            Direction::Input => &self.default_input,
        }
    }

    pub fn set_default_name(&mut self, direction: Direction, name: &str) {
        match direction {
            Direction::Output => self.default_output = name.to_string(),
            Direction::Input => self.default_input = name.to_string(),
        }
    }

    pub fn device(&self, direction: Direction, name: &str) -> Option<&Device> {
        self.devices(direction).iter().find(|d| d.name == name)
    }

    pub fn device_mut(&mut self, direction: Direction, name: &str) -> Option<&mut Device> {
        self.devices_mut(direction)
            .iter_mut()
            .find(|d| d.name == name)
    }

    pub fn stream_mut(&mut self, direction: Direction, index: u32) -> Option<&mut Stream> {
        self.streams_mut(direction)
            .iter_mut()
            .find(|s| s.index == index)
    }
}

/// `pactl` with the C locale: its event lines are translated otherwise, and
/// [`subscribe`] reads them.
fn pactl() -> Command {
    let mut command = Command::new("pactl");
    command.env("LC_ALL", "C");
    command
}

fn run(args: &[&str]) -> Result<String, String> {
    let output = pactl()
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("cannot run pactl: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pactl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn list(what: &str) -> Result<String, String> {
    run(&["--format=json", "list", what])
}

/// Read every device, stream and card, and the two defaults.
pub fn read() -> Result<Graph, String> {
    let info: RawInfo = parse(&run(&["--format=json", "info"])?)?;
    Ok(Graph {
        outputs: parse_devices(&list("sinks")?)?,
        inputs: parse_devices(&list("sources")?)?,
        playback: parse_streams(&list("sink-inputs")?)?,
        recording: parse_streams(&list("source-outputs")?)?,
        cards: parse_cards(&list("cards")?)?,
        default_output: info.default_sink_name,
        default_input: info.default_source_name,
    })
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|err| format!("unexpected pactl output: {err}"))
}

#[derive(Deserialize)]
struct RawInfo {
    #[serde(default)]
    default_sink_name: String,
    #[serde(default)]
    default_source_name: String,
}

#[derive(Deserialize)]
struct RawDevice {
    index: u32,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    volume: HashMap<String, RawVolume>,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
    #[serde(default)]
    ports: Vec<RawPort>,
    #[serde(default)]
    active_port: Option<String>,
}

#[derive(Deserialize)]
struct RawPort {
    name: String,
    #[serde(default)]
    description: String,
    /// "available", "not available" or "availability unknown". A jack that
    /// cannot sense a plug says the last, and is kept.
    #[serde(default)]
    availability: String,
}

#[derive(Deserialize)]
struct RawVolume {
    value: u32,
}

#[derive(Deserialize)]
struct RawStream {
    index: u32,
    /// The device, under the name of its kind: `sink` for a playback
    /// stream, `source` for a recording one.
    #[serde(alias = "sink", alias = "source")]
    device: u32,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    volume: HashMap<String, RawVolume>,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RawCard {
    name: String,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
    #[serde(default)]
    profiles: HashMap<String, RawProfile>,
    #[serde(default)]
    active_profile: String,
}

#[derive(Deserialize)]
struct RawProfile {
    #[serde(default)]
    description: String,
    #[serde(default)]
    priority: u32,
    #[serde(default = "yes")]
    available: bool,
}

fn yes() -> bool {
    true
}

/// What `pactl` calls normal volume, 100 %.
const NORM: f64 = 65536.0;

fn percent(volume: &HashMap<String, RawVolume>) -> u32 {
    let channels = volume.len().max(1) as f64;
    let sum: f64 = volume.values().map(|v| f64::from(v.value)).sum();
    (sum / channels / NORM * 100.0).round() as u32
}

fn property<'a>(properties: &'a HashMap<String, serde_json::Value>, key: &str) -> Option<&'a str> {
    properties
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
}

fn or_name(description: String, name: &str) -> String {
    if description.is_empty() {
        name.to_string()
    } else {
        description
    }
}

/// The devices in one `pactl --format=json list sinks|sources` answer.
///
/// A sink's monitor is listed among the sources, but it is the sink's own
/// sound looped back rather than anything that records, so it is left out,
/// as pavucontrol leaves it out by default.
fn parse_devices(json: &str) -> Result<Vec<Device>, String> {
    let raw: Vec<RawDevice> = parse(json)?;
    Ok(raw
        .into_iter()
        .filter(|device| property(&device.properties, "device.class") != Some("monitor"))
        .map(|device| Device {
            index: device.index,
            volume: percent(&device.volume),
            muted: device.mute,
            ports: device
                .ports
                .into_iter()
                .map(|port| Port {
                    description: or_name(port.description, &port.name),
                    available: port.availability != "not available",
                    name: port.name,
                })
                .collect(),
            active_port: device.active_port,
            description: or_name(device.description, &device.name),
            name: device.name,
        })
        .collect())
}

/// The streams in one `pactl --format=json list sink-inputs|source-outputs`
/// answer.
///
/// Peak meters are left out: pavucontrol's own, and any stream that says it
/// is one. They record to draw a level, not to keep anything, and listing
/// them would show the mixer recording itself.
fn parse_streams(json: &str) -> Result<Vec<Stream>, String> {
    let raw: Vec<RawStream> = parse(json)?;
    Ok(raw
        .into_iter()
        .filter(|stream| {
            property(&stream.properties, "application.id") != Some("org.PulseAudio.pavucontrol")
                && property(&stream.properties, "media.name") != Some("Peak detect")
        })
        .map(|stream| {
            let app = property(&stream.properties, "application.name")
                .or_else(|| property(&stream.properties, "application.process.binary"))
                .unwrap_or("?")
                .to_string();
            let media = property(&stream.properties, "media.name")
                .filter(|media| *media != app)
                .unwrap_or_default()
                .to_string();
            Stream {
                index: stream.index,
                app,
                media,
                device: stream.device,
                volume: percent(&stream.volume),
                muted: stream.mute,
            }
        })
        .collect())
}

/// The cards in one `pactl --format=json list cards` answer.
fn parse_cards(json: &str) -> Result<Vec<Card>, String> {
    let raw: Vec<RawCard> = parse(json)?;
    Ok(raw
        .into_iter()
        .map(|card| {
            let mut profiles: Vec<(String, RawProfile)> = card
                .profiles
                .into_iter()
                .filter(|(name, profile)| profile.available || *name == card.active_profile)
                .collect();
            // Best first, the way pavucontrol lists them; by name among
            // equals, so the order holds from one read to the next.
            profiles.sort_by(|(a, pa), (b, pb)| pb.priority.cmp(&pa.priority).then(a.cmp(b)));
            Card {
                description: or_name(
                    property(&card.properties, "device.description")
                        .unwrap_or_default()
                        .to_string(),
                    &card.name,
                ),
                profiles: profiles
                    .into_iter()
                    .map(|(name, profile)| Profile {
                        description: or_name(profile.description, &name),
                        name,
                    })
                    .collect(),
                active_profile: card.active_profile,
                name: card.name,
            }
        })
        .collect())
}

/// Make `name` where new sound goes (an output) or comes from (an input).
/// The server moves the streams already playing along with it.
pub fn set_default(direction: Direction, name: &str) -> Result<(), String> {
    run(&[&format!("set-default-{}", direction.noun()), name]).map(drop)
}

/// Set every channel of `name` to `percent` of normal volume.
pub fn set_volume(direction: Direction, name: &str, percent: u32) -> Result<(), String> {
    run(&[
        &format!("set-{}-volume", direction.noun()),
        name,
        &format!("{percent}%"),
    ])
    .map(drop)
}

pub fn set_mute(direction: Direction, name: &str, muted: bool) -> Result<(), String> {
    run(&[
        &format!("set-{}-mute", direction.noun()),
        name,
        if muted { "1" } else { "0" },
    ])
    .map(drop)
}

/// Send `name`'s sound through `port`, or take it from there.
pub fn set_port(direction: Direction, name: &str, port: &str) -> Result<(), String> {
    run(&[&format!("set-{}-port", direction.noun()), name, port]).map(drop)
}

/// Move one app's stream to another device.
pub fn move_stream(direction: Direction, index: u32, device: &str) -> Result<(), String> {
    run(&[
        &format!("move-{}", direction.stream_noun()),
        &index.to_string(),
        device,
    ])
    .map(drop)
}

pub fn set_stream_volume(direction: Direction, index: u32, percent: u32) -> Result<(), String> {
    run(&[
        &format!("set-{}-volume", direction.stream_noun()),
        &index.to_string(),
        &format!("{percent}%"),
    ])
    .map(drop)
}

pub fn set_stream_mute(direction: Direction, index: u32, muted: bool) -> Result<(), String> {
    run(&[
        &format!("set-{}-mute", direction.stream_noun()),
        &index.to_string(),
        if muted { "1" } else { "0" },
    ])
    .map(drop)
}

/// Run `card` in `profile`, which brings its devices for that profile into
/// being and takes the others away.
pub fn set_profile(card: &str, profile: &str) -> Result<(), String> {
    run(&["set-card-profile", card, profile]).map(drop)
}

/// Follow the server's events, calling `changed` for each one that can
/// change what the pane shows. Returns when the server goes away.
pub fn subscribe(mut changed: impl FnMut()) -> Result<(), String> {
    let mut child: Child = pactl()
        .arg("subscribe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("cannot run pactl: {err}"))?;
    let stdout = child.stdout.take().ok_or("pactl subscribe has no output")?;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if is_relevant(&line) {
            changed();
        }
    }
    let _ = child.wait();
    Ok(())
}

/// Whether a `pactl subscribe` line is about something the pane shows:
/// devices, app streams, cards or the defaults. Clients and modules come
/// and go without changing any of those.
fn is_relevant(line: &str) -> bool {
    let Some((_, about)) = line.split_once(" on ") else {
        return false;
    };
    let kind = about.split_whitespace().next().unwrap_or_default();
    matches!(
        kind,
        "sink" | "source" | "sink-input" | "source-output" | "server" | "card"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: &str = r#"[
        {"index":54,"name":"alsa_output.pci.analog-stereo.monitor","description":"Monitor of Built-in Audio","mute":false,
         "volume":{"front-left":{"value":65536,"value_percent":"100%","db":"0.00 dB"}},
         "properties":{"device.class":"monitor"}},
        {"index":56,"name":"alsa_input.pci.analog-stereo","description":"Built-in Audio Analog Stereo","mute":true,
         "volume":{"front-left":{"value":26214},"front-right":{"value":39322}},
         "properties":{"device.class":"sound","alsa.card":"0"},
         "active_port":"analog-input-internal-mic",
         "ports":[
            {"name":"analog-input-internal-mic","description":"Internal Microphone","availability":"availability unknown"},
            {"name":"analog-input-mic","description":"Microphone","availability":"not available"},
            {"name":"analog-input-dock","description":"Dock Microphone","availability":"available"}]}
    ]"#;

    #[test]
    fn monitors_are_not_inputs() {
        let devices = parse_devices(SOURCES).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].index, 56);
        assert_eq!(devices[0].name, "alsa_input.pci.analog-stereo");
        assert_eq!(devices[0].description, "Built-in Audio Analog Stereo");
        assert!(devices[0].muted);
    }

    #[test]
    fn volume_is_the_mean_of_the_channels() {
        // 40 % and 60 %.
        assert_eq!(parse_devices(SOURCES).unwrap()[0].volume, 50);
    }

    #[test]
    fn unplugged_ports_are_kept_and_marked() {
        let device = &parse_devices(SOURCES).unwrap()[0];
        let ports: Vec<_> = device
            .ports
            .iter()
            .map(|p| (p.description.as_str(), p.available))
            .collect();
        assert_eq!(
            ports,
            [
                ("Internal Microphone", true),
                ("Microphone", false),
                ("Dock Microphone", true)
            ]
        );
        assert_eq!(
            device.active_port.as_deref(),
            Some("analog-input-internal-mic")
        );
    }

    #[test]
    fn a_device_without_a_description_goes_by_its_name() {
        let devices = parse_devices(r#"[{"index":1,"name":"null-sink","volume":{}}]"#).unwrap();
        assert_eq!(devices[0].description, "null-sink");
        assert_eq!(devices[0].volume, 0);
    }

    #[test]
    fn streams_name_their_app_and_device() {
        let playback = parse_streams(
            r#"[{"index":7,"sink":55,"mute":false,"owner_module":null,
                 "volume":{"front-left":{"value":65536}},
                 "properties":{"application.name":"Firefox","media.name":"Tiny Desk Concert"}}]"#,
        )
        .unwrap();
        assert_eq!(
            playback,
            [Stream {
                index: 7,
                app: "Firefox".into(),
                media: "Tiny Desk Concert".into(),
                device: 55,
                volume: 100,
                muted: false,
            }]
        );
        let recording = parse_streams(
            r#"[{"index":8,"source":56,"properties":{"application.process.binary":"arecord"}},
                {"index":9,"source":56,"properties":{"application.id":"org.PulseAudio.pavucontrol","media.name":"Peak detect"}}]"#,
        )
        .unwrap();
        assert_eq!(recording.len(), 1);
        assert_eq!(recording[0].app, "arecord");
        assert_eq!(recording[0].device, 56);
    }

    #[test]
    fn profiles_that_cannot_run_are_left_out_best_first() {
        let cards = parse_cards(
            r#"[{"name":"alsa_card.pci","properties":{"device.description":"Built-in Audio"},
                 "active_profile":"output:analog-stereo",
                 "profiles":{
                    "off":{"description":"Off","priority":0,"available":true},
                    "output:analog-stereo":{"description":"Analog Stereo Output","priority":6500,"available":true},
                    "output:hdmi-stereo":{"description":"Digital Stereo (HDMI) Output","priority":5900,"available":false},
                    "output:analog-stereo+input:analog-stereo":{"description":"Analog Stereo Duplex","priority":6565,"available":true}}}]"#,
        )
        .unwrap();
        assert_eq!(cards[0].description, "Built-in Audio");
        let names: Vec<_> = cards[0]
            .profiles
            .iter()
            .map(|p| p.description.as_str())
            .collect();
        assert_eq!(
            names,
            ["Analog Stereo Duplex", "Analog Stereo Output", "Off"]
        );
    }

    #[test]
    fn only_events_about_what_the_pane_shows_count() {
        assert!(is_relevant("Event 'change' on sink #55"));
        assert!(is_relevant("Event 'new' on source #60"));
        assert!(is_relevant("Event 'change' on server #4294967295"));
        assert!(is_relevant("Event 'remove' on card #48"));
        assert!(is_relevant("Event 'new' on sink-input #120"));
        assert!(is_relevant("Event 'remove' on source-output #121"));
        assert!(!is_relevant("Event 'change' on client #99"));
        assert!(!is_relevant("Event 'new' on module #12"));
    }
}
