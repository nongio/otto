use layers::prelude::Transition;
use smithay::{
    input::{dnd::DndGrabHandler, Seat},
    utils::{Logical, Point},
};

use super::{Backend, Otto};

impl<BackendData: Backend> DndGrabHandler for Otto<BackendData> {
    fn dropped(
        &mut self,
        _target: Option<smithay::input::dnd::DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        self.dnd_icon = None;
        self.workspaces
            .dnd_view
            .layer
            .set_opacity(0.0, Some(Transition::default()));
        self.workspaces
            .dnd_view
            .layer
            .set_scale((1.2, 1.2), Some(Transition::default()));
    }
}
