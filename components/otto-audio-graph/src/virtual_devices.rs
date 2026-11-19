//! Virtual devices: outputs, inputs and loopbacks the service makes.
//!
//! Each is kept in the routing file as a [`VirtualDevice`] and made in
//! PipeWire while the program that asked for it stays connected:
//!
//! - an **output** is a null sink (`Audio/Sink`): apps play into it and its
//!   monitor ports carry what they play;
//! - an **input** is a virtual source (`Audio/Source/Virtual`): what is linked
//!   into it is what apps recording from it hear;
//! - a **loopback** is a pair of streams, one recording and one playing, that
//!   passes whatever reaches its input on to its output. It starts unlinked.
//!
//! Every node is named after the device's [`VirtualDevice::id`], which never
//! changes, so saved links to it survive a rename and a restart. Renaming
//! changes only the description apps and the Sound pane show.
//!
//! ```toml
//! [[virtual]]
//! id = "stream-mix"
//! name = "Stream mix"
//! kind = "output"
//! channels = 2
//! ```

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::ffi::CString;

use pipewire as pw;
use serde::{Deserialize, Serialize};

use crate::identity::{NodeIdentity, PortRef};
use crate::model::Node;
use crate::routing::{Routing, SavedLink};

/// What every virtual device's node names start with.
const NODE_PREFIX: &str = "otto-virtual.";

/// The factory that wraps a SPA node, such as the null sink, as a node.
const ADAPTER_FACTORY: &str = "adapter";

/// The PipeWire module a loopback is made with.
const LOOPBACK_MODULE: &str = "libpipewire-module-loopback";

/// The most channels a virtual device can have: 7.1.
pub const MAX_CHANNELS: u32 = 8;

/// What a virtual device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VirtualKind {
    /// A null sink: an output that goes nowhere but its monitor.
    Output,
    /// A virtual source: an input apps can record from.
    Input,
    /// Passes what reaches its input on to its output.
    Loopback,
}

/// One virtual device, as the routing file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDevice {
    /// What its node names are made from; never changes once made.
    pub id: String,
    /// The name apps and the Sound pane show.
    pub name: String,
    pub kind: VirtualKind,
    /// How many channels, from 1 to [`MAX_CHANNELS`].
    #[serde(default = "stereo")]
    pub channels: u32,
}

fn stereo() -> u32 {
    2
}

impl VirtualDevice {
    /// The `node.name` of each node it shows as: one for an output or an
    /// input, the recording side then the playing side for a loopback.
    #[must_use]
    pub fn node_names(&self) -> Vec<String> {
        let base = format!("{NODE_PREFIX}{}", self.id);
        match self.kind {
            VirtualKind::Output | VirtualKind::Input => vec![base],
            VirtualKind::Loopback => vec![format!("{base}.in"), format!("{base}.out")],
        }
    }

    /// Whether `node` is one of its nodes.
    #[must_use]
    pub fn owns(&self, node: &Node) -> bool {
        self.node_names().iter().any(|n| n == node.name())
    }

    /// The channel positions, as PipeWire names them.
    #[must_use]
    pub fn positions(&self) -> &'static [&'static str] {
        positions(self.channels)
    }

    /// The identity a saved link names each of its nodes by.
    fn identities(&self) -> Vec<NodeIdentity> {
        self.node_names()
            .into_iter()
            .map(|name| match self.kind {
                VirtualKind::Output | VirtualKind::Input => NodeIdentity::Node { name },
                // A loopback's sides are streams with no app, so they go by
                // their node names.
                VirtualKind::Loopback => NodeIdentity::Stream {
                    direction: if name.ends_with(".in") {
                        crate::identity::StreamDirection::Capture
                    } else {
                        crate::identity::StreamDirection::Playback
                    },
                    app: name,
                    role: None,
                },
            })
            .collect()
    }
}

/// The channel positions for `channels`: mono, stereo, quad, 5.1 or 7.1, and
/// the auxiliary channels PipeWire numbers for any other count.
#[must_use]
pub fn positions(channels: u32) -> &'static [&'static str] {
    const AUX: [&str; MAX_CHANNELS as usize] = [
        "AUX0", "AUX1", "AUX2", "AUX3", "AUX4", "AUX5", "AUX6", "AUX7",
    ];
    match channels {
        1 => &["MONO"],
        2 => &["FL", "FR"],
        4 => &["FL", "FR", "RL", "RR"],
        6 => &["FL", "FR", "FC", "LFE", "RL", "RR"],
        8 => &["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"],
        n => &AUX[..n.clamp(1, MAX_CHANNELS) as usize],
    }
}

/// An id for a device called `name`: its letters and digits in lower case,
/// words joined by dashes, and a number after it when `taken` has it.
fn id_for(name: &str, taken: &[&str]) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "device" } else { slug };
    let mut id = slug.to_string();
    let mut n = 2;
    while taken.contains(&id.as_str()) {
        id = format!("{slug}-{n}");
        n += 1;
    }
    id
}

impl Routing {
    /// The virtual devices, in the order they were added.
    #[must_use]
    pub fn virtual_devices(&self) -> &[VirtualDevice] {
        &self.virtual_devices
    }

    /// Add a virtual device; returns its id.
    pub fn add_virtual(&mut self, name: &str, kind: VirtualKind, channels: u32) -> String {
        let taken: Vec<&str> = self.virtual_devices.iter().map(|d| d.id.as_str()).collect();
        let id = id_for(name, &taken);
        self.virtual_devices.push(VirtualDevice {
            id: id.clone(),
            name: name.trim().to_string(),
            kind,
            channels: channels.clamp(1, MAX_CHANNELS),
        });
        id
    }

    /// Rename the virtual device `id`; `false` when there is none or the name
    /// is the same.
    pub fn rename_virtual(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        match self.virtual_devices.iter_mut().find(|d| d.id == id) {
            Some(device) if device.name != name && !name.is_empty() => {
                device.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Remove the virtual device `id`, and forget every saved link to it,
    /// since nothing will ever be at its end again. `false` when there is
    /// none.
    pub fn remove_virtual(&mut self, id: &str) -> bool {
        let Some(at) = self.virtual_devices.iter().position(|d| d.id == id) else {
            return false;
        };
        let device = self.virtual_devices.remove(at);
        let identities = device.identities();
        let names = |r: &PortRef| identities.contains(&r.node);
        let gone: Vec<SavedLink> = self
            .links()
            .iter()
            .filter(|l| names(&l.output) || names(&l.input))
            .cloned()
            .collect();
        for link in &gone {
            self.remove(link);
        }
        true
    }
}

/// Quote `value` as a SPA JSON string.
fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The arguments `libpipewire-module-loopback` takes for `device`.
fn loopback_args(device: &VirtualDevice) -> String {
    let names = device.node_names();
    let positions = device.positions().join(" ");
    let name = quoted(&device.name);
    // Unlinked until the person links it: a loopback from the default
    // microphone to the default speakers would howl.
    let side = |node_name: &str| {
        format!(
            "{{ node.name = {} node.description = {name} node.autoconnect = false \
             node.virtual = true }}",
            quoted(node_name)
        )
    };
    format!(
        "{{ node.description = {name} audio.channels = {} audio.position = [ {positions} ] \
         capture.props = {} playback.props = {} }}",
        device.channels,
        side(&names[0]),
        side(&names[1]),
    )
}

/// A loopback module loaded into this program's context.
struct LoadedModule(std::ptr::NonNull<pw::sys::pw_impl_module>);

impl Drop for LoadedModule {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `pw_context_load_module` and is
        // destroyed only here, once; the context it lives in outlives it
        // (see `Made`).
        unsafe { pw::sys::pw_impl_module_destroy(self.0.as_ptr()) };
    }
}

/// What makes one device exist; dropping it removes the device.
enum Handle {
    Node(#[expect(dead_code, reason = "held for its lifetime only")] pw::node::Node),
    Module(#[expect(dead_code, reason = "held for its lifetime only")] LoadedModule),
}

/// The virtual devices made on one connection.
///
/// Must be dropped, or [`Made::clear`]ed, before the context it loaded
/// modules into.
#[derive(Default)]
pub(crate) struct Made {
    made: HashMap<String, (VirtualDevice, Handle)>,
}

impl Made {
    /// Make every device in `wanted` that is not made, remove every made one
    /// that is not wanted, and make again any whose name, kind or channels
    /// changed.
    pub(crate) fn sync(
        &mut self,
        core: &pw::core::CoreRc,
        context: &pw::context::ContextRc,
        wanted: &[VirtualDevice],
    ) {
        self.made.retain(|id, (device, _)| {
            let keep = wanted.iter().any(|w| w == device);
            if !keep {
                tracing::info!(id, "removing a virtual device");
            }
            keep
        });
        for device in wanted {
            if self.made.contains_key(&device.id) {
                continue;
            }
            tracing::info!(
                id = device.id,
                name = device.name,
                "making a virtual device"
            );
            let handle = match device.kind {
                VirtualKind::Output | VirtualKind::Input => make_node(core, device),
                VirtualKind::Loopback => load_loopback(context, device),
            };
            if let Some(handle) = handle {
                self.made
                    .insert(device.id.clone(), (device.clone(), handle));
            }
        }
    }

    /// Remove every device made.
    pub(crate) fn clear(&mut self) {
        self.made.clear();
    }
}

fn make_node(core: &pw::core::CoreRc, device: &VirtualDevice) -> Option<Handle> {
    let class = match device.kind {
        VirtualKind::Output => "Audio/Sink",
        _ => "Audio/Source/Virtual",
    };
    let mut props = pw::properties::PropertiesBox::new();
    props.insert("factory.name", "support.null-audio-sink");
    props.insert("node.name", device.node_names().remove(0));
    props.insert("node.description", device.name.as_str());
    props.insert("media.class", class);
    props.insert("audio.channels", device.channels.to_string());
    props.insert("audio.position", device.positions().join(","));
    props.insert("monitor.channel-volumes", "true");
    props.insert("monitor.passthrough", "true");
    // Gone with this program, as the spec has virtual devices go with the
    // service.
    props.insert("object.linger", "false");
    match core.create_object::<pw::node::Node>(ADAPTER_FACTORY, &props) {
        Ok(node) => Some(Handle::Node(node)),
        Err(error) => {
            tracing::warn!(%error, id = device.id, "making a virtual device failed");
            None
        }
    }
}

fn load_loopback(context: &pw::context::ContextRc, device: &VirtualDevice) -> Option<Handle> {
    let (Ok(name), Ok(args)) = (
        CString::new(LOOPBACK_MODULE),
        CString::new(loopback_args(device)),
    ) else {
        return None;
    };
    // SAFETY: the context is live for the call, both strings are valid and
    // NUL-terminated for its whole length, and a null properties pointer is
    // allowed. The module is destroyed before the context (see `Made`).
    let module = unsafe {
        pw::sys::pw_context_load_module(
            context.as_raw_ptr(),
            name.as_ptr(),
            args.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    match std::ptr::NonNull::new(module) {
        Some(module) => Some(Handle::Module(LoadedModule(module))),
        None => {
            tracing::warn!(
                error = %std::io::Error::last_os_error(),
                id = device.id,
                "loading a loopback failed"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_slugs_and_never_repeat() {
        assert_eq!(id_for("Stream Mix!", &[]), "stream-mix");
        assert_eq!(id_for("  ", &[]), "device");
        assert_eq!(
            id_for("Stream mix", &["stream-mix", "stream-mix-2"]),
            "stream-mix-3"
        );
    }

    #[test]
    fn a_loopback_shows_as_two_nodes() {
        let mut routing = Routing::default();
        let id = routing.add_virtual("Mic monitor", VirtualKind::Loopback, 1);
        let device = &routing.virtual_devices()[0];
        assert_eq!(id, "mic-monitor");
        assert_eq!(
            device.node_names(),
            [
                "otto-virtual.mic-monitor.in",
                "otto-virtual.mic-monitor.out"
            ]
        );
        let args = loopback_args(device);
        assert!(args.contains("audio.position = [ MONO ]"), "{args}");
        assert!(args.contains("node.autoconnect = false"), "{args}");
    }

    #[test]
    fn renaming_keeps_the_id() {
        let mut routing = Routing::default();
        let id = routing.add_virtual("Mix", VirtualKind::Output, 2);
        assert!(routing.rename_virtual(&id, "Stream \"mix\""));
        assert!(!routing.rename_virtual(&id, "   "));
        let device = &routing.virtual_devices()[0];
        assert_eq!(device.id, "mix");
        assert_eq!(device.node_names(), ["otto-virtual.mix"]);
        assert_eq!(quoted(&device.name), r#""Stream \"mix\"""#);
    }

    #[test]
    fn removing_forgets_its_saved_links() {
        let mut routing = Routing::default();
        let id = routing.add_virtual("Mix", VirtualKind::Output, 2);
        let to_mix = SavedLink {
            output: PortRef {
                node: NodeIdentity::Node { name: "mic".into() },
                port: "capture_FL".into(),
            },
            input: PortRef {
                node: NodeIdentity::Node {
                    name: "otto-virtual.mix".into(),
                },
                port: "playback_FL".into(),
            },
        };
        let other = SavedLink {
            input: PortRef {
                node: NodeIdentity::Node {
                    name: "speakers".into(),
                },
                port: "playback_FL".into(),
            },
            ..to_mix.clone()
        };
        routing.add(to_mix);
        routing.add(other.clone());
        assert!(routing.remove_virtual(&id));
        assert!(!routing.remove_virtual(&id));
        assert_eq!(routing.links(), [other]);
        assert!(routing.virtual_devices().is_empty());
    }

    #[test]
    fn the_file_keeps_virtual_devices_and_old_files_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing.toml");
        let mut routing = Routing::default();
        routing.add_virtual("Mix", VirtualKind::Input, 6);
        routing.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[virtual]]"), "{text}");
        let back = Routing::load_from(&path).unwrap();
        assert_eq!(back.virtual_devices(), routing.virtual_devices());
        assert_eq!(back.virtual_devices()[0].positions().len(), 6);

        let old: Routing = toml::from_str("version = 1\n[engine]\nforce_quantum = 256\n").unwrap();
        assert!(old.virtual_devices().is_empty());
        let no_channels: Routing =
            toml::from_str("[[virtual]]\nid = \"a\"\nname = \"A\"\nkind = \"output\"\n").unwrap();
        assert_eq!(no_channels.virtual_devices()[0].channels, 2);
    }

    #[test]
    fn odd_channel_counts_are_aux_channels() {
        assert_eq!(positions(3), ["AUX0", "AUX1", "AUX2"]);
        assert_eq!(positions(0), ["AUX0"]);
        assert_eq!(positions(20).len(), 8);
    }
}
