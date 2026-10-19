use smithay::{
    reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
    wayland::selection::{
        data_device::{DataDeviceHandler, DataDeviceState},
        ext_data_control::{
            DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
        },
        wlr_data_control::{DataControlHandler, DataControlState},
    },
};

use super::{Backend, Otto};

impl<BackendData: Backend> DataDeviceHandler for Otto<BackendData> {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
    fn action_choice(&mut self, available: DndAction, preferred: DndAction) -> DndAction {
        // Read from the keyboard itself, as `ctrl_held` in the window grabs
        // does: the cached `current_modifiers` can lag behind.
        let mods = self
            .seat
            .get_keyboard()
            .map(|keyboard| keyboard.modifier_state())
            .unwrap_or(self.current_modifiers);
        let action = choose_dnd_action(mods.ctrl, mods.shift, available, preferred);
        self.load_cursor_for_action(action);
        action
    }
}

/// The action a drag performs, out of what source and target both allow.
///
/// Modifiers held during the drag decide over the target's preference, the
/// convention every Linux file manager shares: Ctrl copies, Shift moves,
/// Ctrl+Shift makes a link. The protocol has no link action, so a link is
/// carried as Ask, which a file manager that offers it reads as "link here"
/// (see otto-files' `DropOp`). A chord never falls back to a move: one the
/// target cannot take becomes a copy when it was asking for a link, and
/// nothing otherwise.
///
/// With no chord, the target's preference stands. A target that states none
/// is saying it takes this drop only when asked, which is how a file manager
/// refuses a plain drag back into the folder it came from while still taking
/// a copy or a link there. Failing that, the least destructive action left is
/// chosen — Copy before Move — and Ask, being a link, only when nothing else
/// is.
///
/// Asked on every `set_actions`, which a target sends on every motion. A
/// modifier pressed or released during a drag re-sends the motion
/// (`Otto::dnd_active`), so the keys take effect with the pointer held still.
fn choose_dnd_action(
    ctrl: bool,
    shift: bool,
    available: DndAction,
    preferred: DndAction,
) -> DndAction {
    let first_of = |order: &[DndAction]| {
        order
            .iter()
            .copied()
            .find(|action| available.contains(*action))
            .unwrap_or(DndAction::empty())
    };
    match (ctrl, shift) {
        (true, true) => first_of(&[DndAction::Ask, DndAction::Copy]),
        (true, false) => first_of(&[DndAction::Copy]),
        (false, true) => first_of(&[DndAction::Move]),
        (false, false) if preferred.is_empty() => DndAction::empty(),
        (false, false)
            if [DndAction::Move, DndAction::Copy, DndAction::Ask].contains(&preferred)
                && available.contains(preferred) =>
        {
            preferred
        }
        (false, false) => first_of(&[DndAction::Copy, DndAction::Move, DndAction::Ask]),
    }
}

impl<BackendData: Backend> DataControlHandler for Otto<BackendData> {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

/// `ext-data-control-v1`. Same contract as the wlr one it supersedes; a
/// clipboard manager speaks whichever it was built against, so Otto serves
/// both rather than choosing for it.
impl<BackendData: Backend> ExtDataControlHandler for Otto<BackendData> {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: DndAction = DndAction::Copy.union(DndAction::Move).union(DndAction::Ask);

    #[test]
    fn a_chord_decides_over_the_preference() {
        assert_eq!(
            choose_dnd_action(true, false, ALL, DndAction::Move),
            DndAction::Copy
        );
        assert_eq!(
            choose_dnd_action(false, true, ALL, DndAction::Copy),
            DndAction::Move
        );
        assert_eq!(
            choose_dnd_action(true, true, ALL, DndAction::Move),
            DndAction::Ask
        );
    }

    /// A plain drag never becomes a link, so a source offering a copy or an
    /// ask, but no move, gets a copy rather than a link to a file it may be
    /// about to delete.
    #[test]
    fn a_plain_drag_copies_before_it_links() {
        let copy_or_ask = DndAction::Copy | DndAction::Ask;
        assert_eq!(
            choose_dnd_action(false, false, copy_or_ask, DndAction::Move),
            DndAction::Copy
        );
        assert_eq!(
            choose_dnd_action(false, false, DndAction::Ask, DndAction::Move),
            DndAction::Ask
        );
    }

    #[test]
    fn a_chord_the_target_cannot_take_never_moves() {
        let copy_or_move = DndAction::Copy | DndAction::Move;
        assert_eq!(
            choose_dnd_action(true, true, copy_or_move, DndAction::Move),
            DndAction::Copy
        );
        assert_eq!(
            choose_dnd_action(true, true, DndAction::Move, DndAction::Move),
            DndAction::empty()
        );
        assert_eq!(
            choose_dnd_action(true, false, DndAction::Move, DndAction::Move),
            DndAction::empty()
        );
    }

    /// A target with no preference takes the drop only when a chord asks.
    #[test]
    fn no_preference_means_only_when_asked() {
        let copy_or_ask = DndAction::Copy | DndAction::Ask;
        assert_eq!(
            choose_dnd_action(false, false, copy_or_ask, DndAction::empty()),
            DndAction::empty()
        );
        assert_eq!(
            choose_dnd_action(true, false, copy_or_ask, DndAction::empty()),
            DndAction::Copy
        );
        assert_eq!(
            choose_dnd_action(true, true, copy_or_ask, DndAction::empty()),
            DndAction::Ask
        );
    }
}
