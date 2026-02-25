use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use crate::{
    otto_dock::protocol::{
        gen::otto_dock_item_v1::{self, OttoDockItemV1},
        DockItem,
    },
    state::{Backend, Otto},
};

impl<BackendData: Backend> Dispatch<OttoDockItemV1, DockItem, Otto<BackendData>>
    for crate::otto_dock::handlers::OttoDockState
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        item: &OttoDockItemV1,
        request: otto_dock_item_v1::Request,
        data: &DockItem,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let app_id = match data.app_id.as_deref() {
            Some(id) => id.to_string(),
            None => {
                tracing::warn!("otto_dock_item_v1 request on item with no app_id");
                return;
            }
        };

        match request {
            otto_dock_item_v1::Request::SetPreview { surface } => {
                tracing::info!(
                    "set_preview: app_id={} surface={:?}",
                    app_id,
                    surface.as_ref().map(|s| s.id())
                );

                // Persist the surface reference in dock state
                if let Some(dock_item) = state.otto_dock.dock_items.get_mut(&item.id()) {
                    dock_item.wl_surface = surface;
                }
                // TODO: render the provided wl_surface as the dock icon.
                // This requires bridging the Wayland surface into the lay-rs scene graph
                // (e.g. via a GlesTexture → Skia image), which is left for a follow-up.
            }

            otto_dock_item_v1::Request::SetBadge { text } => {
                tracing::info!("set_badge: app_id={} text={:?}", app_id, text);

                // Persist
                if let Some(dock_item) = state.otto_dock.dock_items.get_mut(&item.id()) {
                    dock_item.badge = text.clone();
                }

                // Update dock icon badge overlay (app-switcher mirrors reflect this automatically)
                state.workspaces.dock.update_badge_for_app(&app_id, text);
            }

            otto_dock_item_v1::Request::SetProgress { value } => {
                let progress_value: f64 = value.into();
                tracing::info!("set_progress: app_id={} value={}", app_id, progress_value);

                let opt_value = if progress_value < 0.0 {
                    None
                } else {
                    Some(progress_value.clamp(0.0, 1.0))
                };

                // Persist
                if let Some(dock_item) = state.otto_dock.dock_items.get_mut(&item.id()) {
                    dock_item.progress = opt_value;
                }

                // Update dock icon progress bar overlay (app-switcher mirrors reflect this automatically)
                state
                    .workspaces
                    .dock
                    .update_progress_for_app(&app_id, opt_value);
            }
        }
    }
}
