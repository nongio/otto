// org_kde_kwin_appmenu protocol implementation
//
// How a Wayland-native app says where its global menu lives: a com.canonical.dbusmenu
// object, named by D-Bus service and object path, tied to one wl_surface. Qt apps
// running with KDE's platform theme (plasma-integration) send it; on Wayland
// they have no X11 window id to give `com.canonical.AppMenu.Registrar`, so this
// is the only way otto-bar can find their menu. The address is kept on the
// surface and reported in the window's `GetTree` node as `otto_appmenu`
// (docs/developer/shell-dbus-api.md).

use std::sync::Mutex;

use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch,
    New, Resource,
};
use smithay::wayland::compositor::with_states;

pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;

    wayland_scanner::generate_interfaces!("./protocols/kde-appmenu.xml");
    wayland_scanner::generate_server_code!("./protocols/kde-appmenu.xml");
}

use gen::org_kde_kwin_appmenu::{self, OrgKdeKwinAppmenu};
use gen::org_kde_kwin_appmenu_manager::{self, OrgKdeKwinAppmenuManager};

use crate::state::{Backend, Otto};

/// Where a surface's dbusmenu lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMenuAddress {
    pub service: String,
    pub object_path: String,
}

type SurfaceAppMenu = Mutex<Option<AppMenuAddress>>;

/// The menu address a surface was given, if any.
pub fn appmenu_address(surface: &WlSurface) -> Option<AppMenuAddress> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<SurfaceAppMenu>()
            .and_then(|slot| slot.lock().unwrap().clone())
    })
}

fn set_appmenu_address(surface: &WlSurface, address: Option<AppMenuAddress>) {
    with_states(surface, |states| {
        states
            .data_map
            .insert_if_missing_threadsafe(SurfaceAppMenu::default);
        *states
            .data_map
            .get::<SurfaceAppMenu>()
            .unwrap()
            .lock()
            .unwrap() = address;
    });
}

/// Dispatch target for both interfaces; the state lives on the surfaces.
pub struct KdeAppMenuState;

impl<BackendData: Backend> GlobalDispatch<OrgKdeKwinAppmenuManager, (), Otto<BackendData>>
    for KdeAppMenuState
{
    fn bind(
        _state: &mut Otto<BackendData>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinAppmenuManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData: Backend> Dispatch<OrgKdeKwinAppmenuManager, (), Otto<BackendData>>
    for KdeAppMenuState
{
    fn request(
        _state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &OrgKdeKwinAppmenuManager,
        request: org_kde_kwin_appmenu_manager::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            org_kde_kwin_appmenu_manager::Request::Create { id, surface } => {
                data_init.init(id, surface);
            }
            org_kde_kwin_appmenu_manager::Request::Release => {}
        }
    }
}

impl<BackendData: Backend> Dispatch<OrgKdeKwinAppmenu, WlSurface, Otto<BackendData>>
    for KdeAppMenuState
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &OrgKdeKwinAppmenu,
        request: org_kde_kwin_appmenu::Request,
        surface: &WlSurface,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let address = match request {
            org_kde_kwin_appmenu::Request::SetAddress {
                service_name,
                object_path,
            } => Some(AppMenuAddress {
                service: service_name,
                object_path,
            }),
            // "If not applicable, clients should remove this object."
            org_kde_kwin_appmenu::Request::Release => None,
        };
        if !surface.is_alive() || appmenu_address(surface) == address {
            return;
        }
        set_appmenu_address(surface, address);

        // The menu usually arrives after the window was mapped and focused,
        // so tell a bar already showing this window to look again.
        let focused = state
            .focused_window()
            .and_then(|window| window.wl_surface().map(|s| s.into_owned()));
        if focused.as_ref() == Some(surface) {
            state.announce_window_change("otto_appmenu");
        }
    }
}
