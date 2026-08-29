//! Open windows, over `zwlr-foreign-toplevel-management-v1`.
//!
//! This source keeps its own Wayland connection. otto-kit's runner owns the
//! app's event queue and the set of protocols it dispatches, and a client
//! cannot add a `Dispatch` impl to a type it does not own — so rather than
//! widen otto-kit for one source, the toplevel list gets a second connection
//! whose file descriptor the launcher hands to
//! [`App::poll_fds`](otto_kit::App::poll_fds). The runner then wakes for a
//! window opening or closing exactly as it does for its own events.

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, RawFd};

use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, State as ToplevelState, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use crate::source::{Item, Origin, Source};

/// One window as the compositor describes it.
///
/// Titles arrive in pieces — a `title`, an `app_id`, a `state` — and only
/// become a window when `done` says the description is complete.
#[derive(Clone, Default)]
struct Toplevel {
    handle: Option<ZwlrForeignToplevelHandleV1>,
    title: String,
    app_id: String,
    minimized: bool,
    activated: bool,
    /// When this window was last focused, as a counter. Orders the list so the
    /// window someone was just in is the one under the cursor keys.
    touched: u64,
}

#[derive(Default)]
struct Registry {
    seat: Option<WlSeat>,
    toplevels: HashMap<u32, Toplevel>,
    /// Insertion order, so windows that have never been focused keep a stable
    /// place instead of shuffling on every event.
    order: Vec<u32>,
    clock: u64,
    changed: bool,
}

pub struct Windows {
    index: usize,
    /// Whether this run is a window switcher. It changes what an empty query
    /// shows: browsing is the point of the switcher, and beside the point of
    /// the launcher.
    switcher: bool,
    connection: Connection,
    queue: EventQueue<Registry>,
    registry: Registry,
    /// The list as it was last handed out, so `activate` can map a row back to
    /// a handle even after the compositor has changed the set of windows.
    listed: Vec<u32>,
}

impl Windows {
    /// Connect and wait for the compositor to describe every window it already
    /// has. Returns `None` when the compositor does not offer the protocol.
    pub fn connect(index: usize, switcher: bool) -> Option<Self> {
        let connection = Connection::connect_to_env().ok()?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());

        let mut registry = Registry::default();
        // Two roundtrips: the first binds the globals, the second collects the
        // toplevels the manager announces as soon as it is bound.
        queue.roundtrip(&mut registry).ok()?;
        queue.roundtrip(&mut registry).ok()?;

        if registry.toplevels.is_empty() && registry.seat.is_none() {
            return None;
        }
        registry.changed = false;

        Some(Self {
            index,
            switcher,
            connection,
            queue,
            registry,
            listed: Vec::new(),
        })
    }
}

impl Source for Windows {
    fn label(&self) -> &'static str {
        otto_kit::t!("launcher-badge-window")
    }

    fn items(&mut self) -> Vec<Item> {
        let mut ids: Vec<u32> = self
            .registry
            .order
            .iter()
            .copied()
            .filter(|id| {
                self.registry
                    .toplevels
                    .get(id)
                    .is_some_and(|toplevel| !toplevel.title.is_empty())
            })
            .collect();

        // Most recently focused first, and among windows that have never been
        // focused, the order they appeared in.
        ids.sort_by_key(|id| {
            std::cmp::Reverse(self.registry.toplevels.get(id).map_or(0, |t| t.touched))
        });

        self.listed = ids.clone();
        ids.iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let toplevel = self.registry.toplevels.get(id)?;
                let app = otto_kit::desktop_entry::display_name_for_app(&toplevel.app_id);
                Some(Item {
                    title: toplevel.title.clone(),
                    subtitle: Some(if toplevel.minimized {
                        format!("{app} — minimised")
                    } else {
                        app
                    }),
                    icon: Some(toplevel.app_id.clone()),
                    search_terms: vec![toplevel.app_id.clone()],
                    origin: Origin {
                        source: self.index,
                        index,
                    },
                })
            })
            .collect()
    }

    /// Every window, but only in the switcher. In the launcher an empty query
    /// is the resting state, and the resting state is the last few things
    /// launched — not a list of what happens to be open.
    fn resting(&mut self) -> Vec<Item> {
        if self.switcher {
            self.items()
        } else {
            Vec::new()
        }
    }

    fn activate(&mut self, index: usize) -> Result<(), String> {
        let id = self.listed.get(index).ok_or("no such window")?;
        let toplevel = self
            .registry
            .toplevels
            .get(id)
            .ok_or("that window has closed")?;
        let handle = toplevel.handle.as_ref().ok_or("that window has closed")?;
        let seat = self.registry.seat.as_ref().ok_or("no seat")?;

        // A minimised window has to be brought back before it can be focused;
        // activating one that is still minimised would only mark it.
        if toplevel.minimized {
            handle.unset_minimized();
        }
        handle.activate(seat);
        self.connection.flush().map_err(|err| err.to_string())?;
        Ok(())
    }

    fn changed(&mut self) -> bool {
        std::mem::take(&mut self.registry.changed)
    }

    fn poll_fd(&self) -> Option<RawFd> {
        Some(self.connection.as_fd().as_raw_fd())
    }

    /// Read whatever the compositor has sent. Never blocks: the runner calls
    /// this on every loop iteration, not only when this socket is readable.
    fn pump(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.registry);
        let _ = self.connection.flush();
        if let Some(guard) = self.connection.prepare_read() {
            // `WouldBlock` is the normal answer — this is a poll, not a wait.
            let _ = guard.read();
            let _ = self.queue.dispatch_pending(&mut self.registry);
        }
    }
}

// ---------------------------------------------------------------------------
// Wayland plumbing
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for Registry {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "zwlr_foreign_toplevel_manager_v1" => {
                registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
            }
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(7), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Registry {
    fn event(
        state: &mut Self,
        _manager: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            let id = toplevel.id().protocol_id();
            state.order.push(id);
            state.toplevels.insert(
                id,
                Toplevel {
                    handle: Some(toplevel),
                    ..Default::default()
                },
            );
        }
    }

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
        let id = handle.id().protocol_id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(toplevel) = state.toplevels.get_mut(&id) {
                    toplevel.title = title;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(toplevel) = state.toplevels.get_mut(&id) {
                    toplevel.app_id = app_id;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: flags } => {
                // The state arrives as a raw array of enum values, in the
                // wire's native byte order.
                let states: Vec<ToplevelState> = flags
                    .chunks_exact(4)
                    .map(|bytes| u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                    .filter_map(|value| ToplevelState::try_from(value).ok())
                    .collect();
                let activated = states.contains(&ToplevelState::Activated);
                let minimized = states.contains(&ToplevelState::Minimized);

                state.clock += 1;
                let clock = state.clock;
                if let Some(toplevel) = state.toplevels.get_mut(&id) {
                    if activated && !toplevel.activated {
                        toplevel.touched = clock;
                    }
                    toplevel.activated = activated;
                    toplevel.minimized = minimized;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                handle.destroy();
                state.toplevels.remove(&id);
                state.order.retain(|other| *other != id);
                state.changed = true;
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.changed = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(Registry: ignore WlSeat);
