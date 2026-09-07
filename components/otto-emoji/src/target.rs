//! Which application the pick is going to.
//!
//! The picker cannot ask the window under it what it is, but the compositor
//! lists every toplevel over `zwlr-foreign-toplevel-management-v1`, with the
//! activated one marked, and that is the window that had the keyboard before
//! the picker took it. Asked once at startup, before the picker's own surface
//! exists to muddy the answer.
//!
//! Its own connection, for the same reason the launcher's window source has
//! one: otto-kit's runner owns the app's queue and the set of protocols it
//! dispatches, and this one is not among them.

use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, State, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// Applications whose text goes in by typed keys rather than a paste.
///
/// Terminals decode a key's keysym correctly, whatever it is, so keys are the
/// right delivery there — and the wrong one is worse than usual, because
/// Ctrl+V in a terminal is not paste but a control character. Anything not
/// listed here is assumed to want a paste: that covers every Chromium-based
/// application, which is where key delivery breaks (see [`crate::typing`]).
const TERMINALS: [&str; 14] = [
    "foot",
    "footclient",
    "Alacritty",
    "alacritty",
    "kitty",
    "org.wezfurlong.wezterm",
    "wezterm",
    "org.gnome.Terminal",
    "org.gnome.Ptyxis",
    "org.kde.konsole",
    "konsole",
    "xterm",
    "com.mitchellh.ghostty",
    "st",
];

/// What the focused application is, as far as delivery cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A terminal: type keys, never paste.
    Terminal,
    /// Anything else, or nothing known: paste.
    Other,
}

#[derive(Default)]
struct Registry {
    seat: Option<WlSeat>,
    manager: Option<ZwlrForeignToplevelManagerV1>,
    /// The app_id of every window that has said it is activated. The
    /// compositor marks exactly one, but the events arrive per window.
    activated: Vec<String>,
    /// app_ids by handle id, so an `activated` state can be matched to a name
    /// that arrived in an earlier event.
    app_ids: Vec<(ZwlrForeignToplevelHandleV1, String)>,
    pending_activated: Vec<ZwlrForeignToplevelHandleV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Registry {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Registry {
    fn event(
        _: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        _: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Each `toplevel` event creates a handle whose own events carry the
        // details; nothing to do with the manager's.
    }

    // The manager's `toplevel` event carries a new object, and the library
    // has to be told what to make of it or it aborts on the first one.
    wayland_client::event_created_child!(Registry, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Registry {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.app_ids.push((handle.clone(), app_id));
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                let activated = bytes
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .any(|s| s == State::Activated as u32);
                if activated {
                    state.pending_activated.push(handle.clone());
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                // `done` closes a description: only now are app_id and state
                // both known for this handle.
                let handles = std::mem::take(&mut state.pending_activated);
                for active in handles {
                    if let Some((_, app_id)) = state.app_ids.iter().find(|(h, _)| *h == active) {
                        state.activated.push(app_id.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(Registry: ignore WlSeat);

/// The kind of application that has the keyboard right now.
///
/// `Other` when the compositor does not offer the protocol, or nothing is
/// focused, or the focused window never said what it is: paste is the
/// delivery that works in the most places, so it is the one to fall back to.
pub fn focused_kind() -> Kind {
    let Some(app_id) = focused_app_id() else {
        return Kind::Other;
    };
    if TERMINALS.iter().any(|t| *t == app_id) {
        Kind::Terminal
    } else {
        Kind::Other
    }
}

/// The app_id of the activated toplevel, if the compositor will say.
pub fn focused_app_id() -> Option<String> {
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut registry = Registry::default();
    // One roundtrip binds the manager, which then announces every toplevel;
    // a second collects those announcements.
    queue.roundtrip(&mut registry).ok()?;
    registry.manager.as_ref()?;
    queue.roundtrip(&mut registry).ok()?;
    let app_id = registry.activated.last()?.clone();
    tracing::debug!(%app_id, "the pick is going to");
    Some(app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_known_by_app_id() {
        assert!(TERMINALS.contains(&"foot"));
        assert!(TERMINALS.contains(&"kitty"));
        assert!(!TERMINALS.contains(&"google-chrome"));
    }
}
