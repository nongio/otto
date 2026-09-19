use std::time::{Duration, Instant};

use smithay::{
    backend::input::{
        ButtonState, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, PointerButtonEvent,
    },
    delegate_xdg_activation,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    wayland::xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};

use super::{Backend, Otto};

/// How long after a press a token without a fresh serial is still honoured.
///
/// Apps that open a link in another app (a terminal spawning `xdg-open`)
/// rarely hand over a token, so the app that opens the link requests its own,
/// with the serial of its last input — older than the focus the user gave the
/// first app. Accepting tokens requested right after a press lets that app come
/// forward, while one asking on its own long after the user did anything
/// still can't take focus.
const PRESS_GRACE: Duration = Duration::from_secs(3);

impl<BackendData: Backend> Otto<BackendData> {
    /// Remember when the user last pressed something — see [`PRESS_GRACE`].
    pub fn note_press<B: InputBackend>(&mut self, event: &InputEvent<B>) {
        let pressed = match event {
            InputEvent::Keyboard { event } => event.state() == KeyState::Pressed,
            InputEvent::PointerButton { event } => event.state() == ButtonState::Pressed,
            InputEvent::TouchDown { .. } => true,
            _ => false,
        };
        if pressed {
            self.last_press = Some(Instant::now());
        }
    }
}

impl<BackendData: Backend> XdgActivationHandler for Otto<BackendData> {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let fresh_serial = data.serial.as_ref().is_some_and(|(serial, seat)| {
            let keyboard = self.seat.get_keyboard().unwrap();
            smithay::input::Seat::from_resource(seat) == Some(self.seat.clone())
                && keyboard
                    .last_enter()
                    .map(|last_enter| serial.is_no_older_than(&last_enter))
                    .unwrap_or(false)
        });
        let after_press = self
            .last_press
            .is_some_and(|press| press.elapsed() < PRESS_GRACE);
        tracing::debug!(fresh_serial, after_press, app_id = ?data.app_id, "xdg-activation token");
        fresh_serial || after_press
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed().as_secs() < 10 {
            self.activate_window(&surface.id());
        }
    }
}
delegate_xdg_activation!(@<BackendData: Backend + 'static> Otto<BackendData>);
