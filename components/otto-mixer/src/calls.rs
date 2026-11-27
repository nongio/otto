//! What the service does while an app records from a microphone: lower the
//! other apps, and switch a Bluetooth headset to its headset profile.
//!
//! Both follow `[calls]` in the routing file and undo only what they did:
//! a lowered stream gets its volume back only if nobody changed it since,
//! and a headset switches back only if it is still on the profile the
//! service picked.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet};

use otto_audio_graph::{Calls, Endpoint, Graph, Profile, Volume};

/// How close a stream's volume must be to what the service set for it to
/// count as untouched; the server rounds through its cubic volume.
const TOLERANCE: f32 = 0.02;

/// One volume to set: node `node` to `linear`, 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeChange {
    pub node: u32,
    pub linear: f32,
}

/// One profile switch: device `device` to profile `profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSwitch {
    pub device: u32,
    pub profile: u32,
}

/// A stream the service lowered.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lowered {
    /// Its volume before.
    before: f32,
    /// The volume the service set.
    set: f32,
    /// Whether the server has reported the lowered volume yet; until then
    /// the old one it still reports is not a person's change.
    confirmed: bool,
}

/// Lowers other apps' playback while any app records from a microphone.
#[derive(Debug, Default)]
pub struct Ducker {
    lowered: HashMap<u32, Lowered>,
}

impl Ducker {
    /// The server went away: every id is gone, and so are the volumes it
    /// would have restored.
    pub fn disconnected(&mut self) {
        self.lowered.clear();
    }

    /// Whether any stream is lowered now.
    #[must_use]
    pub fn is_lowering(&self) -> bool {
        !self.lowered.is_empty()
    }

    /// The volume changes `calls` asks for in `graph` now.
    pub fn step(&mut self, calls: &Calls, graph: &Graph) -> Vec<VolumeChange> {
        let recording = graph.recording_streams();
        let callers: HashSet<&str> = recording.iter().map(|s| s.app.as_str()).collect();
        let active = calls.lower_others && !recording.is_empty();
        let mut changes = Vec::new();
        let playing = graph.streams(Endpoint::Output);

        if !active {
            for (node, lowered) in self.lowered.drain() {
                let Some(now) = graph.volume(node).map(Volume::linear) else {
                    continue;
                };
                if (now - lowered.set).abs() <= TOLERANCE {
                    changes.push(VolumeChange {
                        node,
                        linear: lowered.before,
                    });
                }
            }
            return changes;
        }

        self.lowered
            .retain(|node, _| playing.iter().any(|s| s.id == *node));
        for stream in &playing {
            let Some(now) = stream.volume.as_ref().map(Volume::linear) else {
                continue;
            };
            if let Some(lowered) = self.lowered.get_mut(&stream.id) {
                if (now - lowered.set).abs() <= TOLERANCE {
                    lowered.confirmed = true;
                } else if lowered.confirmed {
                    // Changed by hand during the call: theirs now.
                    self.lowered.remove(&stream.id);
                }
                continue;
            }
            // The apps in the call keep their volume, and so does silence.
            if callers.contains(stream.app.as_str()) || now <= TOLERANCE {
                continue;
            }
            let set = now * calls.fraction();
            self.lowered.insert(
                stream.id,
                Lowered {
                    before: now,
                    set,
                    confirmed: false,
                },
            );
            changes.push(VolumeChange {
                node: stream.id,
                linear: set,
            });
        }
        changes
    }
}

/// Switches the Bluetooth headsets `[calls]` names to their headset profile
/// while an app records, and back afterwards.
#[derive(Debug, Default)]
pub struct HeadsetSwitcher {
    /// Per device id, the profile it had and the one the service chose.
    switched: HashMap<u32, (u32, u32)>,
}

impl HeadsetSwitcher {
    /// The server went away: every id is gone.
    pub fn disconnected(&mut self) {
        self.switched.clear();
    }

    /// The profile switches `calls` asks for in `graph` now.
    ///
    /// A headset switches when an app records while it plays the default
    /// output in a playback profile, since its microphone is the one a call
    /// wants then.
    pub fn step(&mut self, calls: &Calls, graph: &Graph) -> Vec<ProfileSwitch> {
        let recording = !graph.recording_streams().is_empty();
        let default_device = graph
            .default_node(Endpoint::Output)
            .and_then(|n| n.prop("device.id"))
            .map(str::to_string);
        let mut switches = Vec::new();
        for device in graph.devices().filter(|d| d.is_bluetooth()) {
            let listed = device
                .prop("device.name")
                .is_some_and(|name| calls.headset.iter().any(|h| h == name));
            let active = device.profile.as_ref();
            match self.switched.get(&device.id).copied() {
                Some((before, chosen)) if !recording => {
                    self.switched.remove(&device.id);
                    if active.is_some_and(|p| p.index == chosen) {
                        switches.push(ProfileSwitch {
                            device: device.id,
                            profile: before,
                        });
                    }
                }
                Some(_) => {}
                None if recording && listed => {
                    let in_use = default_device.as_deref() == Some(device.id.to_string().as_str());
                    let Some(active) = active.filter(|p| p.is_hifi()) else {
                        continue;
                    };
                    let Some(headset) = device.preferred_profile(Profile::is_headset) else {
                        continue;
                    };
                    if !in_use {
                        continue;
                    }
                    self.switched
                        .insert(device.id, (active.index, headset.index));
                    switches.push(ProfileSwitch {
                        device: device.id,
                        profile: headset.index,
                    });
                }
                None => {}
            }
        }
        self.switched.retain(|id, _| graph.device(*id).is_some());
        switches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_audio_graph::{Availability, Device, Event, Link, Node, NodeState};

    fn node(id: u32, props: &[(&str, &str)]) -> Event {
        Event::Node(Node {
            id,
            props: props
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            state: NodeState::Running,
        })
    }

    fn link(id: u32, output_node: u32, input_node: u32) -> Event {
        Event::Link(Link {
            id,
            output_node,
            output_port: id * 10,
            input_node,
            input_port: id * 10 + 1,
            client: None,
        })
    }

    fn volume(node: u32, linear: f32) -> Event {
        Event::Volume {
            node,
            volume: Volume {
                channels: vec![otto_audio_graph::to_cubic(linear); 2],
                mute: false,
            },
        }
    }

    /// Headphones (1, device 40) and a microphone (2); Spotify (10) and a
    /// call app (11) playing, the call app not yet recording.
    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        graph.apply(node(
            1,
            &[
                ("media.class", "Audio/Sink"),
                ("node.name", "bluez_output.AA"),
                ("device.id", "40"),
            ],
        ));
        graph.apply(node(
            2,
            &[("media.class", "Audio/Source"), ("node.name", "mic")],
        ));
        for (id, app) in [(10, "Spotify"), (11, "Zoom")] {
            graph.apply(node(
                id,
                &[
                    ("media.class", "Stream/Output/Audio"),
                    ("application.name", app),
                ],
            ));
            graph.apply(volume(id, 0.8));
            graph.apply(link(100 + id, id, 1));
        }
        graph.apply(Event::Default {
            endpoint: Endpoint::Output,
            name: Some("bluez_output.AA".into()),
        });
        graph
    }

    fn start_call(graph: &mut Graph) {
        graph.apply(node(
            12,
            &[
                ("media.class", "Stream/Input/Audio"),
                ("application.name", "Zoom"),
            ],
        ));
        graph.apply(link(112, 2, 12));
    }

    fn end_call(graph: &mut Graph) {
        graph.apply(Event::Removed(12));
        graph.apply(Event::Removed(112));
    }

    fn calls() -> Calls {
        Calls {
            lower_others: true,
            level: 25,
            headset: vec!["bluez_card.AA".into()],
        }
    }

    #[test]
    fn other_apps_are_lowered_during_a_call_and_put_back_after() {
        let mut graph = graph();
        let mut ducker = Ducker::default();
        assert!(ducker.step(&calls(), &graph).is_empty());
        start_call(&mut graph);
        let changes = ducker.step(&calls(), &graph);
        assert_eq!(changes.len(), 1, "only Spotify, not the call app");
        assert_eq!(changes[0].node, 10);
        assert!((changes[0].linear - 0.2).abs() < 1e-4);
        graph.apply(volume(10, changes[0].linear));
        assert!(ducker.step(&calls(), &graph).is_empty());
        end_call(&mut graph);
        let back = ducker.step(&calls(), &graph);
        assert_eq!(back.len(), 1);
        assert!((back[0].linear - 0.8).abs() < 1e-3);
        assert!(!ducker.is_lowering());
    }

    #[test]
    fn a_volume_changed_during_the_call_is_left_alone() {
        let mut graph = graph();
        let mut ducker = Ducker::default();
        start_call(&mut graph);
        let changes = ducker.step(&calls(), &graph);
        graph.apply(volume(10, changes[0].linear));
        ducker.step(&calls(), &graph);
        graph.apply(volume(10, 0.6));
        assert!(ducker.step(&calls(), &graph).is_empty());
        end_call(&mut graph);
        assert!(ducker.step(&calls(), &graph).is_empty());
    }

    #[test]
    fn nothing_is_lowered_while_it_is_off() {
        let mut graph = graph();
        start_call(&mut graph);
        let off = Calls::default();
        assert!(Ducker::default().step(&off, &graph).is_empty());
    }

    fn headphones(active: u32) -> Event {
        let profile = |index, name: &str| Profile {
            index,
            name: name.into(),
            description: name.into(),
            available: Availability::Yes,
            priority: index,
        };
        let profiles = vec![
            profile(0, "off"),
            profile(1, "a2dp-sink-ldac"),
            profile(2, "headset-head-unit-msbc"),
        ];
        Event::Device(Device {
            id: 40,
            props: [
                ("device.api".to_string(), "bluez5".to_string()),
                ("device.name".to_string(), "bluez_card.AA".to_string()),
            ]
            .into(),
            profile: profiles.iter().find(|p| p.index == active).cloned(),
            profiles,
            ..Device::default()
        })
    }

    #[test]
    fn a_listed_headset_switches_for_a_call_and_back() {
        let mut graph = graph();
        graph.apply(headphones(1));
        let mut switcher = HeadsetSwitcher::default();
        assert!(switcher.step(&calls(), &graph).is_empty());
        start_call(&mut graph);
        assert_eq!(
            switcher.step(&calls(), &graph),
            [ProfileSwitch {
                device: 40,
                profile: 2
            }]
        );
        graph.apply(headphones(2));
        assert!(switcher.step(&calls(), &graph).is_empty());
        end_call(&mut graph);
        assert_eq!(
            switcher.step(&calls(), &graph),
            [ProfileSwitch {
                device: 40,
                profile: 1
            }]
        );
    }

    #[test]
    fn a_headset_not_listed_or_switched_by_hand_is_left_alone() {
        let mut graph = graph();
        graph.apply(headphones(1));
        start_call(&mut graph);
        let mut unlisted = calls();
        unlisted.headset.clear();
        assert!(HeadsetSwitcher::default()
            .step(&unlisted, &graph)
            .is_empty());

        let mut switcher = HeadsetSwitcher::default();
        switcher.step(&calls(), &graph);
        // Turned off by hand during the call: stays off.
        graph.apply(headphones(0));
        end_call(&mut graph);
        assert!(switcher.step(&calls(), &graph).is_empty());
    }
}
