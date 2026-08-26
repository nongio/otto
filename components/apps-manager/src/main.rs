use std::collections::HashMap;
use wayland_client::{protocol::wl_registry, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};

#[derive(Debug, Clone)]
struct ToplevelInfo {
    title: Option<String>,
    app_id: Option<String>,
    identifier: Option<String>,
}

struct AppState {
    toplevels: HashMap<u32, ToplevelInfo>,
}

impl AppState {
    fn new() -> Self {
        Self {
            toplevels: HashMap::new(),
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        _state: &mut Self,
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
            if &interface[..] == "ext_foreign_toplevel_list_v1" {
                println!("Found ext_foreign_toplevel_list_v1 v{}", version);
                registry.bind::<ExtForeignToplevelListV1, _, _>(name, version.min(1), qh, ());
            }
        }
    }
}
impl Dispatch<ExtForeignToplevelListV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                // Use the Wayland object ID as our ID
                let id = toplevel.id().protocol_id();
                println!("New toplevel #{}", id);
                state.toplevels.insert(
                    id,
                    ToplevelInfo {
                        title: None,
                        app_id: None,
                        identifier: None,
                    },
                );
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                println!("Foreign toplevel list finished");
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(AppState, ExtForeignToplevelListV1, [
        0 => (ExtForeignToplevelHandleV1, 0u32)
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, u32> for AppState {
    fn event(
        state: &mut Self,
        proxy: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _udata: &u32,
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Use the proxy's Wayland object ID
        let id = proxy.id().protocol_id();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(info) = state.toplevels.remove(&id) {
                    println!(
                        "Toplevel #{} closed: {} ({})",
                        id,
                        info.title.as_deref().unwrap_or("<no title>"),
                        info.app_id.as_deref().unwrap_or("<no app_id>")
                    );
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                if let Some(info) = state.toplevels.get(&id) {
                    println!(
                        "Toplevel #{}: title='{}' app_id='{}' identifier='{}'",
                        id,
                        info.title.as_deref().unwrap_or("<none>"),
                        info.app_id.as_deref().unwrap_or("<none>"),
                        info.identifier.as_deref().unwrap_or("<none>")
                    );
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(info) = state.toplevels.get_mut(&id) {
                    info.title = Some(title);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(info) = state.toplevels.get_mut(&id) {
                    info.app_id = Some(app_id);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                if let Some(info) = state.toplevels.get_mut(&id) {
                    info.identifier = Some(identifier);
                }
            }
            _ => {}
        }
    }
}
fn main() {
    println!("Foreign Toplevel List Debug Tool");
    println!("Connecting to Wayland compositor...\n");
    let conn = Connection::connect_to_env().expect("Failed to connect to Wayland");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let _registry = display.get_registry(&qh, ());
    let mut state = AppState::new();
    println!("Waiting for events... (Press Ctrl+C to exit)\n");
    loop {
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}
