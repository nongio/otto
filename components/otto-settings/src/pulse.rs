//! The sound server as `pactl` sees it.
//!
//! The Sound pane speaks neither PipeWire nor the PulseAudio protocol itself:
//! `pactl` does. It talks to whatever answers the Pulse socket — PulseAudio,
//! or PipeWire through pipewire-pulse, which is how Otto's own sessions run —
//! and has printed JSON since PulseAudio 16. Listing devices, reading volume
//! and mute, changing the default and following changes (`pactl subscribe`)
//! are each one command away, with no audio client library linked into the
//! app and no PipeWire graph to walk by hand.
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
}

/// One sink or source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The server's name for it, which every command addresses it by.
    pub name: String,
    /// What people call it: "Built-in Audio Analog Stereo".
    pub description: String,
    /// Mean of its channels, in percent of normal volume. Over 100 when
    /// something boosted it past normal.
    pub volume: u32,
    pub muted: bool,
}

/// Every device the server offers, and which one is the default each way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
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

    /// The default device, if the server has one and still lists it.
    pub fn default_device(&self, direction: Direction) -> Option<&Device> {
        let name = self.default_name(direction);
        self.devices(direction).iter().find(|d| d.name == name)
    }

    pub fn default_device_mut(&mut self, direction: Direction) -> Option<&mut Device> {
        let name = self.default_name(direction).to_string();
        self.devices_mut(direction)
            .iter_mut()
            .find(|d| d.name == name)
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

/// Read every sink and source and the two defaults.
pub fn read() -> Result<Graph, String> {
    Ok(Graph {
        outputs: parse_devices(&run(&["--format=json", "list", "sinks"])?)?,
        inputs: parse_devices(&run(&["--format=json", "list", "sources"])?)?,
        default_output: run(&["get-default-sink"])?.trim().to_string(),
        default_input: run(&["get-default-source"])?.trim().to_string(),
    })
}

#[derive(Deserialize)]
struct RawDevice {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    volume: HashMap<String, RawVolume>,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RawVolume {
    value: u32,
}

/// What `pactl` calls normal volume, 100 %.
const NORM: f64 = 65536.0;

/// The devices in one `pactl --format=json list` answer.
///
/// A sink's monitor is listed among the sources, but it is the sink's own
/// sound looped back rather than anything that records, so it is left out:
/// picking it as the microphone is never what anyone meant.
fn parse_devices(json: &str) -> Result<Vec<Device>, String> {
    let raw: Vec<RawDevice> =
        serde_json::from_str(json).map_err(|err| format!("unexpected pactl output: {err}"))?;
    Ok(raw
        .into_iter()
        .filter(|device| {
            device
                .properties
                .get("device.class")
                .and_then(|c| c.as_str())
                != Some("monitor")
        })
        .map(|device| {
            let channels = device.volume.len().max(1) as f64;
            let sum: f64 = device.volume.values().map(|v| f64::from(v.value)).sum();
            Device {
                description: if device.description.is_empty() {
                    device.name.clone()
                } else {
                    device.description
                },
                name: device.name,
                volume: (sum / channels / NORM * 100.0).round() as u32,
                muted: device.mute,
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

/// Follow the server's events, calling `changed` for each one that can
/// change what the pane shows — a device coming or going, a volume or mute,
/// a new default. Returns when the server goes away.
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

/// Whether a `pactl subscribe` line is about a device or the defaults.
/// Streams starting and stopping (`sink-input`, `source-output`) are not:
/// they come and go with every notification sound.
fn is_relevant(line: &str) -> bool {
    let Some((_, about)) = line.split_once(" on ") else {
        return false;
    };
    let kind = about.split_whitespace().next().unwrap_or_default();
    matches!(kind, "sink" | "source" | "server" | "card")
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
         "properties":{"device.class":"sound","alsa.card":"0"}}
    ]"#;

    #[test]
    fn monitors_are_not_inputs() {
        let devices = parse_devices(SOURCES).unwrap();
        assert_eq!(devices.len(), 1);
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
    fn a_device_without_a_description_goes_by_its_name() {
        let devices = parse_devices(r#"[{"name":"null-sink","volume":{}}]"#).unwrap();
        assert_eq!(devices[0].description, "null-sink");
        assert_eq!(devices[0].volume, 0);
    }

    #[test]
    fn only_device_events_count() {
        assert!(is_relevant("Event 'change' on sink #55"));
        assert!(is_relevant("Event 'new' on source #60"));
        assert!(is_relevant("Event 'change' on server #4294967295"));
        assert!(is_relevant("Event 'remove' on card #48"));
        assert!(!is_relevant("Event 'new' on sink-input #120"));
        assert!(!is_relevant("Event 'remove' on source-output #121"));
        assert!(!is_relevant("Event 'change' on client #99"));
    }
}
