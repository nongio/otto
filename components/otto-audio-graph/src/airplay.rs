//! AirPlay speakers: PipeWire's RAOP discovery, kept loaded by the
//! connection that asks for it, and the checks for what it needs.
//!
//! `libpipewire-module-raop-discover` browses the network through Avahi and
//! loads a `libpipewire-module-raop-sink` for every AirPlay receiver it
//! finds, into the same context. Unloading it unloads those, so the sinks
//! go when discovery is turned off or the program that loaded it stops.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use pipewire as pw;

use crate::model::Node;
use crate::module::LoadedModule;

/// The module that finds AirPlay receivers and makes a sink for each.
const DISCOVER_MODULE: &str = "libpipewire-module-raop-discover";

/// The `node.name` PipeWire's RAOP sink gives itself, before the host.
pub const AIRPLAY_SINK_PREFIX: &str = "raop_sink.";

/// Where PipeWire's modules are looked for, after `PIPEWIRE_MODULE_DIR` and
/// the directory beside the `libpipewire` this program runs with.
const MODULE_DIRS: [&str; 4] = [
    "/usr/lib/pipewire-0.3",
    "/usr/lib64/pipewire-0.3",
    "/usr/lib/x86_64-linux-gnu/pipewire-0.3",
    "/usr/lib/aarch64-linux-gnu/pipewire-0.3",
];

/// The socket `avahi-daemon` listens on while it runs, under `/run` or the
/// older `/var/run`.
const AVAHI_SOCKETS: [&str; 2] = ["/run/avahi-daemon/socket", "/var/run/avahi-daemon/socket"];

/// The package most distributions ship the RAOP modules in.
pub const AIRPLAY_PACKAGE: &str = "pipewire-zeroconf";

/// What AirPlay discovery is missing to work here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirPlayMissing {
    /// PipeWire's RAOP modules are not installed ([`AIRPLAY_PACKAGE`]).
    Module,
    /// `avahi-daemon` is not running, so nothing on the network is found.
    Avahi,
}

/// Where AirPlay discovery stands on the connection that keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AirPlay {
    /// Not asked for.
    #[default]
    Off,
    /// Loaded and looking for receivers.
    On,
    /// Asked for, but it cannot work: the module would not load, or Avahi
    /// is not running. With Avahi missing the module stays loaded and finds
    /// receivers once Avahi starts.
    Unavailable(AirPlayMissing),
}

/// What AirPlay discovery is missing on this system, if anything. Reads two
/// paths and, when it has to, `/proc/self/maps`; touches nothing.
#[must_use]
pub fn airplay_missing() -> Option<AirPlayMissing> {
    if !module_installed() {
        return Some(AirPlayMissing::Module);
    }
    if !avahi_running() {
        return Some(AirPlayMissing::Avahi);
    }
    None
}

/// Whether `node` is a sink PipeWire's RAOP module made for an AirPlay
/// receiver; see [`Node::is_airplay`].
pub(crate) fn is_airplay(node: &Node) -> bool {
    node.name().starts_with(AIRPLAY_SINK_PREFIX) || node.prop("raop.ip").is_some()
}

fn avahi_running() -> bool {
    AVAHI_SOCKETS.iter().any(|path| Path::new(path).exists())
}

fn module_installed() -> bool {
    let file = format!("{DISCOVER_MODULE}.so");
    module_dirs().iter().any(|dir| dir.join(&file).is_file())
}

/// The directories PipeWire loads modules from: `PIPEWIRE_MODULE_DIR` when
/// set, else the one beside the `libpipewire` mapped into this process and
/// the usual ones.
fn module_dirs() -> Vec<PathBuf> {
    if let Some(dirs) = std::env::var_os("PIPEWIRE_MODULE_DIR").filter(|d| !d.is_empty()) {
        return std::env::split_paths(&dirs).collect();
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_to_string("/proc/self/maps")
        .ok()
        .and_then(|maps| beside_libpipewire(&maps))
        .into_iter()
        .collect();
    dirs.extend(MODULE_DIRS.iter().map(PathBuf::from));
    dirs
}

/// The module directory beside the `libpipewire-0.3` named in `maps` (the
/// text of `/proc/<pid>/maps`): `<libdir>/pipewire-0.3`.
fn beside_libpipewire(maps: &str) -> Option<PathBuf> {
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .map(Path::new)
        .find(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libpipewire-0.3.so"))
        })
        .and_then(Path::parent)
        .map(|dir| dir.join("pipewire-0.3"))
}

/// Discovery on one connection, on the PipeWire thread.
///
/// Must be dropped, or [`Discovery::clear`]ed, before the context it loaded
/// its module into.
#[derive(Default)]
pub(crate) struct Discovery {
    module: Option<LoadedModule>,
    status: AirPlay,
}

impl Discovery {
    /// Load the discovery module when `wanted` and not loaded (or gone by
    /// itself), unload it when not `wanted`. Returns the new status when it
    /// changed.
    pub(crate) fn sync(
        &mut self,
        context: &pw::context::ContextRc,
        wanted: bool,
    ) -> Option<AirPlay> {
        if self.module.as_ref().is_some_and(|m| !m.is_alive()) {
            tracing::info!("AirPlay discovery went away by itself, loading it again");
            self.module = None;
        }
        let status = if wanted {
            if self.module.is_none() {
                tracing::info!("loading AirPlay discovery");
                self.module = crate::module::load(context, DISCOVER_MODULE, "{ }");
            }
            match &self.module {
                None => AirPlay::Unavailable(airplay_missing().unwrap_or(AirPlayMissing::Module)),
                Some(_) if !avahi_running() => AirPlay::Unavailable(AirPlayMissing::Avahi),
                Some(_) => AirPlay::On,
            }
        } else {
            if self.module.take().is_some() {
                tracing::info!("unloading AirPlay discovery");
            }
            AirPlay::Off
        };
        (status != self.status).then(|| {
            self.status = status;
            status
        })
    }

    /// Unload the module, and the sinks it made.
    pub(crate) fn clear(&mut self) {
        self.module = None;
        self.status = AirPlay::Off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeState;

    fn node(props: &[(&str, &str)]) -> Node {
        Node {
            id: 1,
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            state: NodeState::Idle,
        }
    }

    #[test]
    fn a_raop_sink_is_airplay() {
        let sink = node(&[
            ("media.class", "Audio/Sink"),
            ("node.name", "raop_sink.Living-Room.local.192.168.1.20.7000"),
        ]);
        assert!(is_airplay(&sink));
        let by_prop = node(&[("media.class", "Audio/Sink"), ("raop.ip", "192.168.1.20")]);
        assert!(is_airplay(&by_prop));
    }

    #[test]
    fn other_sinks_are_not_airplay() {
        let sink = node(&[
            ("media.class", "Audio/Sink"),
            ("node.name", "alsa_output.pci-0000_00_1f.3.analog-stereo"),
        ]);
        assert!(!is_airplay(&sink));
        assert!(!is_airplay(&node(&[("node.name", "otto-virtual.raop")])));
    }

    #[test]
    fn the_module_directory_is_beside_libpipewire() {
        let maps = "\
7f00-7f01 r--p 00000000 103:02 1 /usr/lib/libc.so.6
7f02-7f03 r-xp 00001000 103:02 2 /usr/lib/x86_64-linux-gnu/libpipewire-0.3.so.0.1409.0
7f04-7f05 rw-p 00000000 00:00 0
";
        assert_eq!(
            beside_libpipewire(maps),
            Some(PathBuf::from("/usr/lib/x86_64-linux-gnu/pipewire-0.3"))
        );
        assert_eq!(
            beside_libpipewire("7f04-7f05 rw-p 00000000 00:00 0\n"),
            None
        );
    }
}
