use smithay::reexports::wayland_server::{
    backend::ObjectId, Client, DataInit, Dispatch, DisplayHandle,
    GlobalDispatch, New, Resource,
};
use std::collections::HashMap;

use crate::{
    otto_dock::protocol::{
        gen::otto_dock_manager_v1::{self, OttoDockManagerV1},
        DockItem, OttoDockItemV1,
    },
    state::{Backend, Otto},
};

/// Dock item role data stored per resource
#[derive(Debug, Clone)]
pub struct DockItemRole {
    /// Application ID for this dock item
    pub app_id: String,
    /// Reference to the OttoDockItemV1 resource
    pub resource_id: ObjectId,
}

impl DockItemRole {
    pub fn new(app_id: String, resource_id: ObjectId) -> Self {
        Self { app_id, resource_id }
    }
}

/// Global state for otto_dock protocol
pub struct OttoDockState {
    /// Map from OttoDockItemV1 ObjectId to dock item data
    pub dock_items: HashMap<ObjectId, DockItem>,
    /// Map from app_id to OttoDockItemV1 resource for sending events
    pub app_id_to_resource: HashMap<String, OttoDockItemV1>,
}

impl OttoDockState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<OttoDockManagerV1, ()>
            + Dispatch<OttoDockManagerV1, ()>
            + Dispatch<OttoDockItemV1, DockItem>
            + 'static,
    {
        display.create_global::<D, OttoDockManagerV1, ()>(1, ());

        Self {
            dock_items: HashMap::new(),
            app_id_to_resource: HashMap::new(),
        }
    }
}

impl<BackendData: Backend> GlobalDispatch<OttoDockManagerV1, (), Otto<BackendData>>
    for OttoDockState
{
    fn bind(
        _state: &mut Otto<BackendData>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OttoDockManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData: Backend> Dispatch<OttoDockManagerV1, (), Otto<BackendData>> for OttoDockState {
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _manager: &OttoDockManagerV1,
        request: otto_dock_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            otto_dock_manager_v1::Request::GetDockItem { id, app_id } => {
                tracing::info!("get_dock_item: app_id={}", app_id);

                let dock_item = DockItem {
                    wl_surface: None,
                    item_type: crate::otto_dock::protocol::DockItemType::AppElement,
                    app_id: Some(app_id.clone()),
                    badge: None,
                    progress: None,
                    preview_subsurface: None,
                    width: 0,
                    height: 0,
                };

                let wl_item = data_init.init(id, dock_item.clone());
                let resource_id = wl_item.id();

                // Register item in state for subsequent requests (set_badge, etc.)
                state.otto_dock.dock_items.insert(resource_id.clone(), dock_item);
                state.otto_dock.app_id_to_resource.insert(app_id, wl_item);
            }
            otto_dock_manager_v1::Request::Destroy => {
                // Manager destroyed; existing dock items are unaffected.
            }
        }
    }
}