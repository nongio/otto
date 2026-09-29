//! The `otto-canvas-v1` globals: turning requests into canvas state changes.

// Rust guideline compliant 2026-02-21

use smithay::reexports::wayland_server::{
    backend::ClientId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle,
    GlobalDispatch, New, Resource,
};

use crate::otto_canvas::protocol::{
    gen::{otto_canvas_item_v1, otto_canvas_manager_v1},
    OttoCanvasItemV1, OttoCanvasManagerV1,
};
use crate::otto_canvas::ItemKeyboard;
use crate::state::{Backend, Otto};

/// The role a surface takes when it becomes a canvas item.
///
/// A surface keeps its role for life, so an item that is destroyed leaves the
/// surface able to become a canvas item again and nothing else.
pub const CANVAS_ITEM_ROLE: &str = "otto_canvas_item_v1";

/// What each `otto_canvas_item_v1` resource carries: the surface it places.
#[derive(Debug, Clone)]
pub struct CanvasItemData {
    pub surface: WlSurface,
}

/// The version of `otto-canvas-v1` advertised.
const VERSION: u32 = 3;

/// Owner of the `otto_canvas_manager_v1` global.
#[derive(Debug)]
pub struct CanvasGlobal;

impl CanvasGlobal {
    /// Advertise the manager global on `display`.
    pub fn create<D>(display: &DisplayHandle)
    where
        D: GlobalDispatch<OttoCanvasManagerV1, ()>
            + Dispatch<OttoCanvasManagerV1, ()>
            + Dispatch<OttoCanvasItemV1, CanvasItemData>
            + 'static,
    {
        display.create_global::<D, OttoCanvasManagerV1, ()>(VERSION, ());
    }
}

impl<B: Backend> GlobalDispatch<OttoCanvasManagerV1, (), Otto<B>> for CanvasGlobal {
    fn bind(
        _state: &mut Otto<B>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OttoCanvasManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<B>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<B: Backend> Dispatch<OttoCanvasManagerV1, (), Otto<B>> for CanvasGlobal {
    fn request(
        state: &mut Otto<B>,
        _client: &Client,
        manager: &OttoCanvasManagerV1,
        request: otto_canvas_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<B>>,
    ) {
        match request {
            otto_canvas_manager_v1::Request::GetCanvasItem { id, surface } => {
                if smithay::wayland::compositor::give_role(&surface, CANVAS_ITEM_ROLE).is_err() {
                    manager.post_error(
                        otto_canvas_manager_v1::Error::Role,
                        "the surface already has another role",
                    );
                    return;
                }
                let item = data_init.init(
                    id,
                    CanvasItemData {
                        surface: surface.clone(),
                    },
                );
                state.canvas_item_created(item, surface);
            }
            otto_canvas_manager_v1::Request::Destroy => {}
        }
    }
}

impl<B: Backend> Dispatch<OttoCanvasItemV1, CanvasItemData, Otto<B>> for CanvasGlobal {
    fn request(
        state: &mut Otto<B>,
        _client: &Client,
        item: &OttoCanvasItemV1,
        request: otto_canvas_item_v1::Request,
        _data: &CanvasItemData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<B>>,
    ) {
        match request {
            otto_canvas_item_v1::Request::AckConfigure { serial } => {
                state.canvas_item_acked(item, serial);
            }
            otto_canvas_item_v1::Request::SetKeyboardInteractivity { interactivity } => {
                use otto_canvas_item_v1::KeyboardInteractivity;
                let keyboard = match interactivity.into_result() {
                    Ok(KeyboardInteractivity::None) => ItemKeyboard::OnPress,
                    Ok(KeyboardInteractivity::OnShow) => ItemKeyboard::OnShow,
                    // `never` came with version 3.
                    Ok(KeyboardInteractivity::Never) if item.version() >= 3 => ItemKeyboard::Never,
                    _ => {
                        item.post_error(
                            otto_canvas_item_v1::Error::InvalidKeyboardInteractivity,
                            "keyboard interactivity is not in the enum",
                        );
                        return;
                    }
                };
                state.canvas_item_set_keyboard(item, keyboard);
            }
            otto_canvas_item_v1::Request::Show => state.canvas_item_show(item),
            otto_canvas_item_v1::Request::SetOrder { order } => {
                state.canvas_item_set_order(item, order);
            }
            otto_canvas_item_v1::Request::Dismiss => state.canvas_hide(),
            // The item leaves the canvas in `destroyed`, which also covers a
            // client that goes away without asking.
            otto_canvas_item_v1::Request::Destroy => {}
        }
    }

    fn destroyed(
        state: &mut Otto<B>,
        _client: ClientId,
        item: &OttoCanvasItemV1,
        _data: &CanvasItemData,
    ) {
        state.canvas_item_destroyed(&item.id());
    }
}
