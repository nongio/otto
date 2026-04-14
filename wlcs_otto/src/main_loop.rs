use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use smithay::{
    backend::input::ButtonState,
    input::pointer::{ButtonEvent, CursorImageStatus, MotionEvent, RelativeMotionEvent},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            channel::{Channel, Event as ChannelEvent},
            EventLoop,
        },
        wayland_server::{protocol::wl_surface, Client, Display, Resource},
    },
    utils::SERIAL_COUNTER as SCOUNTER,
};

use otto::{state::Backend, ClientState, Otto};

use crate::WlcsEvent;

const OUTPUT_NAME: &str = "otto";

struct TestState {
    clients: HashMap<i32, Client>,
}

impl Backend for TestState {
    fn seat_name(&self) -> String {
        "otto_wlcs".into()
    }

    fn backend_name(&self) -> &'static str {
        "wlcs"
    }

    fn reset_buffers(&mut self, _output: &Output) {}
    fn early_import(&mut self, _surface: &wl_surface::WlSurface) {}
    fn texture_for_surface(
        &self,
        _surface: &smithay::backend::renderer::utils::RendererSurfaceState,
    ) -> Option<otto::skia_renderer::SkiaTextureImage> {
        None
    }
    fn renderer_context(&mut self) -> Option<layers::skia::gpu::DirectContext> {
        None
    }
    fn set_cursor(&mut self, _image: &CursorImageStatus) {}
}

pub fn run(channel: Channel<WlcsEvent>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        run_inner(channel);
    });
}

fn run_inner(channel: Channel<WlcsEvent>) {
    let mut event_loop = EventLoop::try_new().unwrap();

    let display = Display::new().expect("Failed to init display");
    let test_state = TestState {
        clients: HashMap::new(),
    };

    let mut state = Otto::init(display, event_loop.handle(), test_state, false);

    event_loop
        .handle()
        .insert_source(channel, |event, &mut (), state| match event {
            ChannelEvent::Msg(msg) => handle_event(msg, state),
            ChannelEvent::Closed => {}
        })
        .unwrap();

    let mode = Mode {
        size: (800, 600).into(),
        refresh: 60_000,
    };

    let output = Output::new(
        OUTPUT_NAME.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Otto".into(),
            model: "WLCS".into(),
            serial_number: String::new(),
        },
    );
    let _global = output.create_global::<Otto<TestState>>(&state.display_handle);
    output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.workspaces.map_output(&output, (0, 0));

    while state.running.load(Ordering::SeqCst) {
        // Send frame events so that clients start drawing their next frame
        if let Some(space) = state.workspaces.space() {
            space.elements().for_each(|window| {
                window.send_frame(&output, state.clock.now(), Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                })
            });
        }

        if event_loop
            .dispatch(Some(Duration::from_millis(16)), &mut state)
            .is_err()
        {
            state.running.store(false, Ordering::SeqCst);
        } else {
            state.workspaces.refresh_space();
            state.popups.cleanup();
            state.display_handle.flush_clients().unwrap();
            state.scene_element.update();
            state.update_dnd();
        }
    }
}

fn handle_event(event: WlcsEvent, state: &mut Otto<TestState>) {
    match event {
        WlcsEvent::Exit => state.running.store(false, Ordering::SeqCst),
        WlcsEvent::NewClient { stream, client_id } => {
            let client = state
                .display_handle
                .insert_client(stream, Arc::new(ClientState::default()))
                .unwrap();
            state.backend_data.clients.insert(client_id, client);
        }
        WlcsEvent::PositionWindow {
            client_id,
            surface_id,
            location,
        } => {
            let client = state.backend_data.clients.get(&client_id);
            let toplevel = state.workspaces.space().and_then(|space| {
                space
                    .elements()
                    .find(|w: &&otto::shell::WindowElement| {
                        if let Some(surface) = w.wl_surface() {
                            state.display_handle.get_client(surface.id()).ok().as_ref() == client
                                && surface.id().protocol_id() == surface_id
                        } else {
                            false
                        }
                    })
                    .cloned()
            });
            if let Some(toplevel) = toplevel {
                state.workspaces.map_window(&toplevel, location, false, None);
            }
        }
        // pointer inputs
        WlcsEvent::NewPointer { .. } => {}
        WlcsEvent::PointerMoveAbsolute {
            device_id: _,
            location,
        } => {
            let serial = SCOUNTER.next_serial();
            let under = state.surface_under(location);
            let ptr = state.seat.get_pointer().unwrap();
            ptr.motion(
                state,
                under.clone(),
                &MotionEvent {
                    location,
                    serial,
                    time: 0,
                },
            );
            ptr.relative_motion(
                state,
                under,
                &RelativeMotionEvent {
                    delta: (0.0, 0.0).into(),
                    delta_unaccel: (0.0, 0.0).into(),
                    utime: 0,
                },
            );
        }
        WlcsEvent::PointerMoveRelative {
            device_id: _,
            delta,
        } => {
            let pointer = state.seat.get_pointer().unwrap();
            let location = pointer.current_location() + delta;
            let serial = SCOUNTER.next_serial();
            let under = state.surface_under(location);
            pointer.motion(
                state,
                under.clone(),
                &MotionEvent {
                    location,
                    serial,
                    time: 0,
                },
            );
            pointer.relative_motion(
                state,
                under,
                &RelativeMotionEvent {
                    delta,
                    delta_unaccel: delta,
                    utime: 0,
                },
            );
        }
        WlcsEvent::PointerButtonDown {
            device_id: _,
            button_id,
        } => {
            let serial = SCOUNTER.next_serial();
            let ptr = state.seat.get_pointer().unwrap();
            if !ptr.is_grabbed() {
                let window = state
                    .workspaces
                    .element_under(ptr.current_location())
                    .map(|(w, _)| w.clone());
                if let Some(ref w) = window {
                    state.workspaces.focus_app_with_window(&w.id());
                }
                state
                    .seat
                    .get_keyboard()
                    .unwrap()
                    .set_focus(state, window.map(|w| w.into()), serial);
            }
            ptr.button(
                state,
                &ButtonEvent {
                    button: button_id as u32,
                    state: ButtonState::Pressed,
                    serial,
                    time: 0,
                },
            );
        }
        WlcsEvent::PointerButtonUp {
            device_id: _,
            button_id,
        } => {
            let serial = SCOUNTER.next_serial();
            let ptr = state.seat.get_pointer().unwrap();
            ptr.button(
                state,
                &ButtonEvent {
                    button: button_id as u32,
                    state: ButtonState::Released,
                    serial,
                    time: 0,
                },
            );
        }
        WlcsEvent::PointerRemoved { .. } => {}
        WlcsEvent::NewTouch { .. } => {}
        WlcsEvent::TouchDown { .. } => {}
        WlcsEvent::TouchMove { .. } => {}
        WlcsEvent::TouchUp { .. } => {}
        WlcsEvent::TouchRemoved { .. } => {}
    }
}
