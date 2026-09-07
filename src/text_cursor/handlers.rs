//! The `otto-text-cursor-v1` globals, and the caret arithmetic behind them.

use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::text_input::TextInputSeat;

use crate::state::{Backend, Otto};
use crate::text_cursor::protocol::{
    gen::{otto_text_cursor_manager_v1, otto_text_cursor_v1},
    OttoTextCursorManagerV1, OttoTextCursorV1,
};

/// Every live text-cursor object, so a moved caret can be announced to all of
/// them.
#[derive(Debug, Default)]
pub struct TextCursorState {
    watchers: Vec<OttoTextCursorV1>,
    /// The last rectangle announced, so an unchanged caret is not re-sent on
    /// every commit of the surface that owns it.
    announced: Option<Option<Rectangle<i32, Logical>>>,
}

impl TextCursorState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<OttoTextCursorManagerV1, ()>
            + Dispatch<OttoTextCursorManagerV1, ()>
            + Dispatch<OttoTextCursorV1, ()>
            + 'static,
    {
        display.create_global::<D, OttoTextCursorManagerV1, ()>(1, ());
        Self::default()
    }

    /// Drop objects whose client has gone.
    fn retain_alive(&mut self) {
        self.watchers.retain(|watcher| watcher.is_alive());
    }
}

/// Send one answer to one watcher.
fn announce(watcher: &OttoTextCursorV1, caret: Option<Rectangle<i32, Logical>>) {
    match caret {
        Some(rect) => watcher.position(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
        None => watcher.unavailable(),
    }
}

impl<BackendData: Backend> Otto<BackendData> {
    /// Where the text cursor is, in layout coordinates.
    ///
    /// The rectangle applications report is relative to their own surface, so
    /// it is offset by wherever that surface's window currently sits. A caret
    /// whose window has since closed, or which was never reported at all, is
    /// `None`.
    pub fn text_cursor_rectangle(&self) -> Option<Rectangle<i32, Logical>> {
        let (surface, rect) = self.seat.text_input().cursor_rectangle()?;
        let window = self
            .workspaces
            .spaces_elements()
            .find(|window| window.wl_surface().as_deref() == Some(&surface))?;
        let location: Point<i32, Logical> = self.workspaces.element_location(window)?;
        // The window's geometry origin is not always its surface origin — a
        // client with shadows in its buffer puts the visible top-left some way
        // in — and the caret is relative to the surface, so the geometry offset
        // has to come back off.
        let geometry = window.geometry();
        Some(Rectangle::new(
            location + rect.loc - geometry.loc,
            rect.size,
        ))
    }

    /// Tell every watcher where the caret is now, if it has moved since the
    /// last time they were told.
    pub fn refresh_text_cursor(&mut self) {
        self.text_cursor.retain_alive();
        if self.text_cursor.watchers.is_empty() {
            return;
        }
        let caret = self.text_cursor_rectangle();
        if self.text_cursor.announced == Some(caret) {
            return;
        }
        self.text_cursor.announced = Some(caret);
        for watcher in &self.text_cursor.watchers {
            announce(watcher, caret);
        }
    }
}

impl<BackendData: Backend> GlobalDispatch<OttoTextCursorManagerV1, (), Otto<BackendData>>
    for TextCursorState
{
    fn bind(
        _state: &mut Otto<BackendData>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OttoTextCursorManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData: Backend> Dispatch<OttoTextCursorManagerV1, (), Otto<BackendData>>
    for TextCursorState
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _manager: &OttoTextCursorManagerV1,
        request: otto_text_cursor_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            otto_text_cursor_manager_v1::Request::GetTextCursor { id, seat: _ } => {
                let watcher = data_init.init(id, ());
                // Answered at once: a client creating this object is about to
                // decide where to put itself, and has nothing to wait for.
                announce(&watcher, state.text_cursor_rectangle());
                state.text_cursor.watchers.push(watcher);
            }
            otto_text_cursor_manager_v1::Request::Destroy => {}
        }
    }
}

impl<BackendData: Backend> Dispatch<OttoTextCursorV1, (), Otto<BackendData>> for TextCursorState {
    fn request(
        _state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &OttoTextCursorV1,
        request: otto_text_cursor_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            otto_text_cursor_v1::Request::Destroy => {}
        }
    }

    fn destroyed(
        state: &mut Otto<BackendData>,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &OttoTextCursorV1,
        _data: &(),
    ) {
        state
            .text_cursor
            .watchers
            .retain(|watcher| watcher != resource);
    }
}
