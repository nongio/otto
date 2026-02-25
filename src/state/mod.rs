use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use layers::{engine::Engine, prelude::taffy};
use tracing::{info, warn};

use smithay::{
    backend::renderer::{
        element::{
            default_primary_scanout_output_compare, utils::select_dmabuf_feedback,
            RenderElementStates,
        },
        utils::{RendererSurfaceState, RendererSurfaceStateUserData},
    },
    delegate_compositor, delegate_cursor_shape, delegate_keyboard_shortcuts_inhibit,
    delegate_layer_shell, delegate_output, delegate_pointer_gestures, delegate_presentation,
    delegate_relative_pointer, delegate_shm, delegate_text_input_manager, delegate_viewporter,
    delegate_virtual_keyboard_manager, delegate_xdg_foreign, delegate_xdg_shell,
    desktop::{
        space::SpaceElement,
        utils::{
            surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
            update_surface_primary_scanout_output, with_surfaces_surface_tree,
            OutputPresentationFeedback,
        },
        PopupManager,
    },
    input::{
        keyboard::{Keysym, ModifiersState, XkbConfig},
        pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus, PointerHandle},
        Seat, SeatState,
    },
    output::Output,
    reexports::{
        calloop::{
            channel::{channel, Event as ChannelEvent, Sender as ChannelSender},
            generic::Generic,
            Interest, LoopHandle, Mode, PostAction,
        },
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason, ObjectId},
            protocol::{wl_data_device_manager::DndAction, wl_surface::WlSurface},
            Display, DisplayHandle, Resource,
        },
    },
    utils::{self, Clock, Monotonic, SERIAL_COUNTER},
    wayland::{
        compositor::{
            self, CompositorClientState, CompositorState, SurfaceAttributes, SurfaceData,
            TraversalAction,
        },
        cursor_shape::CursorShapeManagerState,
        dmabuf::DmabufFeedback,
        foreign_toplevel_list::ForeignToplevelListState,
        fractional_scale::{with_fractional_scale, FractionalScaleManagerState},
        input_method::InputMethodManagerState,
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
        output::{OutputHandler, OutputManagerState},
        pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        security_context::{SecurityContext, SecurityContextState},
        selection::{
            data_device::DataDeviceState, primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, SurfaceCachedState, XdgShellState},
        },
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
        tablet_manager::TabletManagerState,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        xdg_activation::XdgActivationState,
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

use crate::cursor::{CursorManager, CursorTextureCache};
use crate::{
    audio::{AudioManager, SoundPlayer},
    config::Config,
    focus::KeyboardFocusTarget,
    render_elements::scene_element::SceneElement,
    shell::{LayerShellSurface, WindowElement},
    skia_renderer::SkiaTextureImage,
    workspaces::{WindowViewBaseModel, WindowViewSurface, Workspaces},
};
#[cfg(feature = "xwayland")]
use smithay::{
    delegate_xwayland_keyboard_grab,
    utils::{Point, Size},
    wayland::selection::{SelectionSource, SelectionTarget},
    wayland::xwayland_keyboard_grab::{XWaylandKeyboardGrabHandler, XWaylandKeyboardGrabState},
    xwayland::{X11Wm, XWayland, XWaylandEvent},
};

pub struct CalloopData<BackendData: Backend + 'static> {
    pub state: Otto<BackendData>,
    pub display_handle: DisplayHandle,
}

#[derive(Debug, Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    pub security_context: Option<SecurityContext>,
}
impl ClientData for ClientState {
    /// Notification that a client was initialized
    fn initialized(&self, _client_id: ClientId) {}
    /// Notification that a client is disconnected
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// Tracks reserved space on each edge of an output from layer shell exclusive zones
#[derive(Debug, Clone, Default)]
pub struct ExclusiveZones {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

impl ExclusiveZones {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the usable area after applying exclusive zones
    pub fn apply_to_output(
        &self,
        output_geometry: utils::Rectangle<i32, utils::Logical>,
    ) -> utils::Rectangle<i32, utils::Logical> {
        let loc_x = output_geometry.loc.x + self.left;
        let loc_y = output_geometry.loc.y + self.top;
        let width = output_geometry.size.w - self.left - self.right;
        let height = output_geometry.size.h - self.top - self.bottom;

        utils::Rectangle::new((loc_x, loc_y).into(), (width, height).into())
    }
}

pub struct Otto<BackendData: Backend + 'static> {
    pub backend_data: BackendData,
    pub socket_name: Option<String>,
    pub display_handle: DisplayHandle,
    pub running: Arc<AtomicBool>,
    pub handle: LoopHandle<'static, Otto<BackendData>>,
    pub loop_wakeup_sender: ChannelSender<()>,
    pub loop_wakeup_pending: Arc<AtomicBool>,

    // desktop
    pub popups: PopupManager,
    /// Cache mapping popup surface IDs to their root window surface IDs
    /// for fast lookup during commit/destroy without re-traversing the popup tree
    pub popup_root_cache: HashMap<ObjectId, ObjectId>,
    /// Compositor-owned layer shell surfaces, keyed by surface ObjectId
    pub layer_surfaces: HashMap<ObjectId, LayerShellSurface>,
    /// Tracked exclusive zones per output (reserved space on each edge)
    pub exclusive_zones: HashMap<String, ExclusiveZones>,
    pub workspaces: Workspaces,

    // smithay state
    pub compositor_state: CompositorState,
    pub data_device_state: DataDeviceState,
    pub layer_shell_state: WlrLayerShellState,
    pub output_manager_state: OutputManagerState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub seat_state: SeatState<Otto<BackendData>>,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub shm_state: ShmState,
    pub viewporter_state: ViewporterState,
    pub xdg_activation_state: XdgActivationState,
    pub xdg_decoration_state: XdgDecorationState,
    pub xdg_shell_state: XdgShellState,
    pub presentation_state: PresentationState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub xdg_foreign_state: XdgForeignState,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    pub wlr_foreign_toplevel_state: wlr_foreign_toplevel::WlrForeignToplevelManagerState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    pub virtual_keyboard_manager_state: VirtualKeyboardManagerState,

    #[cfg(feature = "xwayland")]
    pub xwayland_shell_state: xwayland_shell::XWaylandShellState,

    pub dnd_icon: Option<WlSurface>,

    // input-related fields
    pub suppressed_keys: Vec<Keysym>,
    pub current_modifiers: ModifiersState,
    pub app_switcher_hold_modifiers: Option<ModifiersState>,
    pub cursor_status: Arc<Mutex<CursorImageStatus>>,
    pub cursor_manager: CursorManager,
    pub cursor_texture_cache: CursorTextureCache,
    pub seat_name: String,
    pub seat: Seat<Otto<BackendData>>,
    pub clock: Clock<Monotonic>,
    pub pointer: PointerHandle<Otto<BackendData>>,

    pub gamma_control_manager: gamma_control::GammaControlManagerState,
    pub audio_manager: Option<crate::audio::AudioManager>,
    pub sound_player: Option<crate::audio::SoundPlayer>,

    // gamma animation state
    /// Active gamma transitions: (output_name, from_lut, to_lut, start_time, duration)
    #[allow(clippy::type_complexity)]
    pub gamma_transitions: HashMap<
        String,
        (
            Vec<u16>,
            Vec<u16>,
            Vec<u16>,
            Vec<u16>,
            Vec<u16>,
            Vec<u16>,
            std::time::Instant,
            std::time::Duration,
        ),
    >,
    /// Currently applied gamma per output: (output_name, red_lut, green_lut, blue_lut)
    #[allow(clippy::type_complexity)]
    pub current_gamma: HashMap<String, (Vec<u16>, Vec<u16>, Vec<u16>)>,

    #[cfg(feature = "xwayland")]
    pub xwm: Option<X11Wm>,
    #[cfg(feature = "xwayland")]
    pub xdisplay: Option<u32>,

    #[cfg(feature = "debug")]
    pub renderdoc: Option<renderdoc::RenderDoc<renderdoc::V141>>,

    pub scene_element: SceneElement,

    // layers
    pub layers_engine: Arc<Engine>,

    pub show_desktop: bool,
    pub swipe_gesture: SwipeGestureState,
    pub is_pinching: bool,
    pub pinch_last_scale: f64,
    pub is_resizing: bool,

    // power management
    pub is_lid_closed: bool,

    // screenshare
    pub screenshare_sessions: HashMap<String, crate::screenshare::ScreencastSession>,
    /// Manager for the screenshare D-Bus service (started lazily when needed).
    pub screenshare_manager: Option<crate::screenshare::ScreenshareManager>,

    // foreign toplevel list - maps surface ObjectId to unified toplevel handles (both protocols)
    pub foreign_toplevels: HashMap<ObjectId, foreign_toplevel_shared::ForeignToplevelHandles>,

    // sc_layer protocol
    // Map from surface ID to list of sc-layers augmenting that surface
    pub sc_layers: HashMap<ObjectId, Vec<crate::sc_layer_shell::ScLayer>>,
    pub sc_transactions: HashMap<ObjectId, crate::sc_layer_shell::ScTransaction>,
    // Map from surface ID to its rendering layer in the scene graph
    pub surface_layers: HashMap<ObjectId, layers::prelude::Layer>,
    // Pre-warmed View caches: surface_id -> (layer_key -> NodeRef)
    // Built during surface creation, moved into Views when they're created
    pub view_warm_cache:
        HashMap<ObjectId, HashMap<String, std::collections::VecDeque<layers::prelude::NodeRef>>>,

    // otto_dock protocol
    pub otto_dock: crate::otto_dock::OttoDockState,

    // Rendering metrics
    #[cfg(feature = "metrics")]
    pub render_metrics: Arc<crate::render_metrics::RenderMetrics>,
}

pub mod data_device_handler;
pub mod dnd_grab_handler;
pub mod foreign_toplevel_list_handler;
pub mod foreign_toplevel_shared;
pub mod fractional_scale_handler;
pub mod gamma_control;
pub mod input_method_handler;
pub mod seat_handler;
pub mod security_context_handler;
pub mod selection_handler;
pub mod virtual_keyboard_handler;
pub mod wlr_foreign_toplevel;
pub mod xdg_activation_handler;
pub mod xdg_decoration_handler;
pub mod xwayland_handler;

// Gesture constants
pub const DIRECTION_THRESHOLD: f64 = 5.0;
pub const EXPOSE_DELTA_MULTIPLIER: f64 = 500.0;
pub const VELOCITY_SAMPLE_COUNT: usize = 4;

/// Swipe gesture direction detected from accumulated deltas
#[derive(Debug, Clone, Copy)]
pub enum SwipeDirection {
    Horizontal(f64),
    Vertical(f64),
    Undetermined,
}

impl SwipeDirection {
    pub fn from_accumulated(horiz: f64, vert: f64) -> Self {
        if horiz > DIRECTION_THRESHOLD && horiz > vert {
            Self::Horizontal(horiz)
        } else if vert > DIRECTION_THRESHOLD {
            Self::Vertical(vert)
        } else {
            Self::Undetermined
        }
    }

    pub fn to_expose_delta(&self) -> Option<f32> {
        match self {
            Self::Vertical(delta) => Some(*delta as f32 / EXPOSE_DELTA_MULTIPLIER as f32),
            _ => None,
        }
    }
}

/// State machine for 3-finger swipe gestures
#[derive(Debug, Clone)]
pub enum SwipeGestureState {
    Idle,
    Detecting { accumulated: (f64, f64) },
    WorkspaceSwitching { velocity_samples: Vec<f64> },
    Expose { velocity_samples: Vec<f64> },
}

impl SwipeGestureState {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn is_expose(&self) -> bool {
        matches!(self, Self::Expose { .. })
    }
}

impl<BackendData: Backend> OutputHandler for Otto<BackendData> {}

impl<BackendData: Backend> ShmHandler for Otto<BackendData> {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl<BackendData: Backend> KeyboardShortcutsInhibitHandler for Otto<BackendData> {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Just grant the wish for everyone
        inhibitor.activate();
    }
}

impl<BackendData: Backend> XdgForeignHandler for Otto<BackendData> {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}

delegate_compositor!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_output!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_shm!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_cursor_shape!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_text_input_manager!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_keyboard_shortcuts_inhibit!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_virtual_keyboard_manager!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_pointer_gestures!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_relative_pointer!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_viewporter!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_xdg_shell!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_layer_shell!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_presentation!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_xdg_foreign!(@<BackendData: Backend + 'static> Otto<BackendData>);

// Gamma control protocol delegation
smithay::reexports::wayland_server::delegate_global_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    gamma_control::gen::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: ()
] => gamma_control::GammaControlManagerState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    gamma_control::gen::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: ()
] => gamma_control::GammaControlManagerState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    gamma_control::gen::zwlr_gamma_control_v1::ZwlrGammaControlV1: gamma_control::GammaControlState
] => gamma_control::GammaControlManagerState);

// otto_dock protocol delegates
smithay::reexports::wayland_server::delegate_global_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    crate::otto_dock::OttoDockManagerV1: ()
] => crate::otto_dock::OttoDockState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    crate::otto_dock::OttoDockManagerV1: ()
] => crate::otto_dock::OttoDockState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    crate::otto_dock::OttoDockItemV1: crate::otto_dock::DockItem
] => crate::otto_dock::OttoDockState);

impl<BackendData: Backend + 'static> Otto<BackendData> {
    pub fn init(
        display: Display<Otto<BackendData>>,
        handle: LoopHandle<'static, Otto<BackendData>>,
        backend_data: BackendData,
        listen_on_socket: bool,
    ) -> Otto<BackendData> {
        let dh = display.handle();

        let clock = Clock::new();

        // init wayland clients
        let socket_name = if listen_on_socket {
            let source = ListeningSocketSource::new_auto().unwrap();
            let socket_name = source.socket_name().to_string_lossy().into_owned();
            handle
                .insert_source(source, |client_stream, _, data| {
                    if let Ok(_client) = data
                        .display_handle
                        .insert_client(client_stream, Arc::new(ClientState::default()))
                    {
                        // warn!("Error adding wayland client: {}", err);
                    };
                })
                .expect("Failed to init wayland socket source");
            info!(name = socket_name, "Listening on wayland socket");

            // Export WAYLAND_DISPLAY to systemd user session for portal services
            if let Err(e) = std::process::Command::new("systemctl")
                .args([
                    "--user",
                    "set-environment",
                    &format!("WAYLAND_DISPLAY={}", socket_name),
                ])
                .output()
            {
                warn!(error = ?e, "Failed to export WAYLAND_DISPLAY to systemd");
            } else {
                info!(
                    name = socket_name,
                    "Exported WAYLAND_DISPLAY to systemd user session"
                );
            }

            Some(socket_name)
        } else {
            None
        };
        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data| {
                    profiling::scope!("dispatch_clients");
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(data).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("Failed to init wayland server source");

        let (loop_wakeup_sender, loop_wakeup_channel) = channel::<()>();
        let loop_wakeup_pending = Arc::new(AtomicBool::new(false));
        let pending_flag = loop_wakeup_pending.clone();
        handle
            .insert_source(loop_wakeup_channel, move |event, _, _| {
                if matches!(event, ChannelEvent::Msg(_) | ChannelEvent::Closed) {
                    pending_flag.store(false, Ordering::Release);
                }
            })
            .expect("Failed to insert loop wake channel");

        // init globals
        let compositor_state = CompositorState::new::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let mut seat_state = SeatState::new();
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let presentation_state = PresentationState::new::<Self>(&dh, clock.id() as u32);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        TextInputManagerState::new::<Self>(&dh);
        InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        let virtual_keyboard_manager_state =
            VirtualKeyboardManagerState::new::<Self, _>(&dh, |_client| true);
        // Expose global only if backend supports relative motion events
        if BackendData::HAS_RELATIVE_MOTION {
            RelativePointerManagerState::new::<Self>(&dh);
        }
        PointerConstraintsState::new::<Self>(&dh);
        if BackendData::HAS_GESTURES {
            PointerGesturesState::new::<Self>(&dh);
        }
        TabletManagerState::new::<Self>(&dh);
        SecurityContextState::new::<Self, _>(&dh, |client| {
            client
                .get_data::<ClientState>()
                .is_none_or(|client_state| client_state.security_context.is_none())
        });
        let xdg_foreign_state = XdgForeignState::new::<Self>(&dh);
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<Self>(&dh);
        let wlr_foreign_toplevel_state =
            wlr_foreign_toplevel::WlrForeignToplevelManagerState::new::<Self>(&dh);
        let gamma_control_manager = gamma_control::GammaControlManagerState::new();

        // Register gamma control global
        dh.create_global::<Self, gamma_control::gen::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1, _>(
            1,
            (),
        );

        // Create minimal sc_layer shell global
        crate::sc_layer_shell::create_layer_shell_global::<BackendData>(&dh);

        // Create otto_dock protocol global
        let otto_dock = crate::otto_dock::OttoDockState::new::<Self>(&dh);

        // init input
        let seat_name = backend_data.seat_name();
        let mut seat = seat_state.new_wl_seat(&dh, seat_name.clone());

        let cursor_status = Arc::new(Mutex::new(CursorImageStatus::default_named()));
        let (cursor_theme, cursor_size) = Config::with(|c| (c.cursor_theme.clone(), c.cursor_size));
        let cursor_manager = CursorManager::new(&cursor_theme, cursor_size as u8);
        let cursor_texture_cache = CursorTextureCache::default();
        let pointer = seat.add_pointer();
        let (layout, variant, options, repeat_delay, repeat_rate) = Config::with(|c| {
            let layout = c.input.xkb_layout.clone().unwrap_or_default();
            let variant = c.input.xkb_variant.clone().unwrap_or_default();
            let options = if c.input.xkb_options.is_empty() {
                None
            } else {
                Some(c.input.xkb_options.join(","))
            };
            (
                layout,
                variant,
                options,
                c.keyboard_repeat_delay,
                c.keyboard_repeat_rate,
            )
        });
        let xkb_config = XkbConfig {
            layout: &layout,
            variant: &variant,
            options,
            ..Default::default()
        };
        seat.add_keyboard(xkb_config, repeat_delay, repeat_rate)
            .expect("Failed to initialize the keyboard");

        let keyboard_shortcuts_inhibit_state = KeyboardShortcutsInhibitState::new::<Self>(&dh);
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&dh);

        #[cfg(feature = "xwayland")]
        let xwayland_shell_state = xwayland_shell::XWaylandShellState::new::<Self>(&dh.clone());

        #[cfg(feature = "xwayland")]
        XWaylandKeyboardGrabState::new::<Self>(&dh.clone());

        let layers_engine = Engine::create(500.0, 500.0);
        let root_layer = layers_engine.new_layer();
        root_layer.set_key("screen_composer_root");
        root_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layers_engine.add_layer(&root_layer);
        let scene_element = SceneElement::with_engine(layers_engine.clone());
        let workspaces = Workspaces::new(layers_engine.clone(), dh.clone());

        #[cfg(feature = "debugger")]
        layers_engine.start_debugger();

        // Get backend name before moving backend_data
        #[cfg(feature = "metrics")]
        let backend_name = backend_data.backend_name();

        Otto {
            backend_data,
            display_handle: dh,
            socket_name,
            running: Arc::new(AtomicBool::new(true)),
            handle,
            loop_wakeup_sender,
            loop_wakeup_pending,

            popups: PopupManager::default(),
            popup_root_cache: HashMap::new(),
            layer_surfaces: HashMap::new(),
            exclusive_zones: HashMap::new(),
            compositor_state,
            data_device_state,
            layer_shell_state,
            output_manager_state,
            primary_selection_state,
            data_control_state,
            seat_state,
            keyboard_shortcuts_inhibit_state,
            shm_state,
            viewporter_state,
            xdg_activation_state,
            xdg_decoration_state,
            xdg_shell_state,
            presentation_state,
            fractional_scale_manager_state,
            xdg_foreign_state,
            foreign_toplevel_list_state,
            wlr_foreign_toplevel_state,
            cursor_shape_manager_state,
            virtual_keyboard_manager_state,
            dnd_icon: None,
            suppressed_keys: Vec::new(),
            current_modifiers: ModifiersState::default(),
            app_switcher_hold_modifiers: None,
            cursor_status,
            cursor_manager,
            cursor_texture_cache,
            seat_name,
            seat,
            pointer,
            clock,
            gamma_control_manager,
            audio_manager: AudioManager::new().ok(),
            sound_player: SoundPlayer::new().ok(),
            gamma_transitions: HashMap::new(),
            current_gamma: HashMap::new(),
            #[cfg(feature = "xwayland")]
            xwayland_shell_state,
            #[cfg(feature = "xwayland")]
            xwm: None,
            #[cfg(feature = "xwayland")]
            xdisplay: None,
            #[cfg(feature = "debug")]
            renderdoc: renderdoc::RenderDoc::new().ok(),

            workspaces,
            layers_engine,
            scene_element,

            show_desktop: false,
            // support variables for gestures
            swipe_gesture: SwipeGestureState::Idle,
            is_pinching: false,
            pinch_last_scale: 1.0,
            is_resizing: false,

            // power management
            is_lid_closed: false,

            // screenshare
            screenshare_sessions: HashMap::new(),
            screenshare_manager: None,

            // foreign toplevel list
            foreign_toplevels: HashMap::new(),

            // sc_layer minimal protocol
            sc_layers: HashMap::new(),
            sc_transactions: HashMap::new(),
            surface_layers: HashMap::new(),
            view_warm_cache: HashMap::new(),

            // otto_dock protocol
            otto_dock,

            // render metrics
            #[cfg(feature = "metrics")]
            render_metrics: Arc::new(crate::render_metrics::RenderMetrics::new(backend_name)),
        }
    }

    /// Recalculate exclusive zones for an output from its layer shell surfaces
    pub fn recalculate_exclusive_zones(&mut self, output: &Output) {
        use smithay::desktop::layer_map_for_output;
        use smithay::wayland::shell::wlr_layer::{Anchor, ExclusiveZone};

        let output_name = output.name();
        let layer_map = layer_map_for_output(output);

        let mut zones = ExclusiveZones::new();
        let layer_config = Config::with(|c| c.layer_shell.clone());
        let scale = Config::with(|c| c.screen_scale);

        // Calculate scaled max limits
        let max_top = (layer_config.max_top as f64 * scale) as i32;
        let max_bottom = (layer_config.max_bottom as f64 * scale) as i32;
        let max_left = (layer_config.max_left as f64 * scale) as i32;
        let max_right = (layer_config.max_right as f64 * scale) as i32;

        for layer_surface in layer_map.layers() {
            let anchor =
                smithay::wayland::compositor::with_states(layer_surface.wl_surface(), |states| {
                    states
                        .cached_state
                        .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState>()
                        .current()
                        .anchor
                });

            let exclusive_zone =
                smithay::wayland::compositor::with_states(layer_surface.wl_surface(), |states| {
                    states
                        .cached_state
                        .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState>()
                        .current()
                        .exclusive_zone
                });

            match exclusive_zone {
                ExclusiveZone::Exclusive(size) if size > 0 => {
                    let size = size as i32;
                    // Apply to appropriate edge based on anchor
                    if anchor.contains(Anchor::TOP) && !anchor.contains(Anchor::BOTTOM) {
                        let clamped = if max_top > 0 { size.min(max_top) } else { size };
                        zones.top += clamped;
                    } else if anchor.contains(Anchor::BOTTOM) && !anchor.contains(Anchor::TOP) {
                        let clamped = if max_bottom > 0 {
                            size.min(max_bottom)
                        } else {
                            size
                        };
                        zones.bottom += clamped;
                    }

                    if anchor.contains(Anchor::LEFT) && !anchor.contains(Anchor::RIGHT) {
                        let clamped = if max_left > 0 {
                            size.min(max_left)
                        } else {
                            size
                        };
                        zones.left += clamped;
                    } else if anchor.contains(Anchor::RIGHT) && !anchor.contains(Anchor::LEFT) {
                        let clamped = if max_right > 0 {
                            size.min(max_right)
                        } else {
                            size
                        };
                        zones.right += clamped;
                    }
                }
                _ => {}
            }
        }

        self.exclusive_zones.insert(output_name.clone(), zones);
    }

    pub fn schedule_event_loop_dispatch(&self) {
        if !self.loop_wakeup_pending.swap(true, Ordering::AcqRel)
            && self.loop_wakeup_sender.send(()).is_err()
        {
            self.loop_wakeup_pending.store(false, Ordering::Release);
        }
    }

    #[cfg(feature = "xwayland")]
    pub fn start_xwayland(&mut self) {
        use std::process::Stdio;

        let (xwayland, client) = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        )
        .expect("failed to start XWayland");

        let ret = self
            .handle
            .insert_source(xwayland, move |event, _, data| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    let mut wm = X11Wm::start_wm(data.handle.clone(), x11_socket, client.clone())
                        .expect("Failed to attach X11 Window Manager");

                    let cursor = Cursor::load();
                    let image = cursor.get_image(1, Duration::ZERO);
                    wm.set_cursor(
                        &image.pixels_rgba,
                        Size::from((image.width as u16, image.height as u16)),
                        Point::from((image.xhot as u16, image.yhot as u16)),
                    )
                    .expect("Failed to set xwayland default cursor");
                    data.xwm = Some(wm);
                    data.xdisplay = Some(display_number);
                }
                XWaylandEvent::Error => {
                    warn!("XWayland crashed on startup");
                }
            });
        if let Err(e) = ret {
            tracing::error!(
                "Failed to insert the XWaylandSource into the event loop: {}",
                e
            );
        }
    }
    pub fn set_cursor(&mut self, image: &CursorImageStatus) {
        *self.cursor_status.lock().unwrap() = image.clone();
        self.backend_data.set_cursor(image);
    }

    pub fn load_cursor_for_action(
        &mut self,
        action: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
    ) {
        let cursor = if action == DndAction::Copy {
            CursorImageStatus::Named(CursorIcon::Copy)
        } else if action == DndAction::Move {
            CursorImageStatus::Named(CursorIcon::Move)
        } else if action == DndAction::Ask {
            CursorImageStatus::Named(CursorIcon::Help)
        } else {
            CursorImageStatus::Hidden
        };
        self.set_cursor(&cursor);
    }

    pub fn get_cursor_position(&self) -> utils::Point<f64, utils::Physical> {
        let cursor_guard: std::sync::MutexGuard<CursorImageStatus> =
            self.cursor_status.lock().unwrap();
        let cursor_hotspot = if let CursorImageStatus::Surface(ref surface) = *cursor_guard {
            compositor::with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .hotspot
            })
        } else {
            (0, 0).into()
        };

        let cursor_pos = self.pointer.current_location() - cursor_hotspot.to_f64();
        let output = self
            .workspaces
            .output_under(cursor_pos)
            .next()
            .cloned()
            .unwrap();
        let scale = output.current_scale().fractional_scale();

        cursor_pos.to_physical(scale).to_f64()
    }

    pub fn get_render_elements(
        &self,
        surface: &WlSurface,
        scale_factor: f64,
    ) -> VecDeque<WindowViewSurface> {
        let initial_location: smithay::utils::Point<f64, smithay::utils::Physical> =
            (0.0, 0.0).into();
        let mut render_elements = VecDeque::new();

        // Track parent through traversal context: (absolute_location, parent_location, parent_id)
        // parent_location is used to compute relative offsets for child surfaces
        let initial_context = (initial_location, initial_location, None);

        smithay::wayland::compositor::with_surface_tree_downward(
            surface,
            initial_context,
            |surface, states, (location, _parent_location, _parent_id)| {
                let mut location = *location;
                let data = states.data_map.get::<RendererSurfaceStateUserData>();
                let mut cached_state = states.cached_state.get::<SurfaceCachedState>();
                let cached_state = cached_state.current();
                let surface_geometry = cached_state.geometry.unwrap_or_default();

                if let Some(data) = data {
                    let data = data.lock().unwrap();

                    if let Some(view) = data.view() {
                        location += view.offset.to_f64().to_physical(scale_factor);
                        location -= surface_geometry.loc.to_f64().to_physical(scale_factor);
                        // Pass current location as parent location for children, and current surface as parent ID
                        TraversalAction::DoChildren((location, location, Some(surface.id())))
                    } else {
                        TraversalAction::SkipChildren
                    }
                } else {
                    TraversalAction::SkipChildren
                }
            },
            |surface, states, (location, parent_location, parent_id)| {
                // Compute relative offset from parent for child surfaces
                let relative_offset = if parent_id.is_some() {
                    *location - *parent_location
                } else {
                    *location
                };

                if let Some(window_view) = self.window_view_for_surface(
                    surface,
                    states,
                    &relative_offset,
                    scale_factor,
                    parent_id.clone(),
                ) {
                    render_elements.push_front(window_view);
                }
            },
            |_, _, _| true,
        );
        render_elements
    }

    pub fn update_dnd(&mut self) {
        let dnd_surface = self.dnd_icon.as_ref().cloned();
        if let Some(dnd_surface) = dnd_surface {
            profiling::scope!("update_dnd_icon");
            let cursor_position = self.get_cursor_position();

            let scale = Config::with(|c| c.screen_scale);

            // Build both render_elements and surface_info (like windows do)
            let mut render_elements = VecDeque::new();
            #[allow(clippy::mutable_key_type)]
            let mut surface_info: std::collections::HashMap<
                ObjectId,
                (WlSurface, Option<ObjectId>),
            > = std::collections::HashMap::new();

            let initial_location: smithay::utils::Point<f64, smithay::utils::Physical> =
                (0.0, 0.0).into();
            let initial_context = (initial_location, initial_location, None);

            smithay::wayland::compositor::with_surface_tree_downward(
                &dnd_surface,
                initial_context,
                |surface, states, (location, _parent_location, _parent_id)| {
                    let mut location = *location;
                    let data = states.data_map.get::<RendererSurfaceStateUserData>();
                    let mut cached_state = states.cached_state.get::<SurfaceCachedState>();
                    let cached_state = cached_state.current();
                    let surface_geometry = cached_state.geometry.unwrap_or_default();

                    if let Some(data) = data {
                        let data = data.lock().unwrap();
                        if let Some(view) = data.view() {
                            location += view.offset.to_f64().to_physical(scale);
                            location -= surface_geometry.loc.to_f64().to_physical(scale);
                            TraversalAction::DoChildren((location, location, Some(surface.id())))
                        } else {
                            TraversalAction::SkipChildren
                        }
                    } else {
                        TraversalAction::SkipChildren
                    }
                },
                |surface, states, (location, parent_location, parent_id)| {
                    let relative_offset = if parent_id.is_some() {
                        *location - *parent_location
                    } else {
                        *location
                    };

                    if let Some(wvs) = self.window_view_for_surface(
                        surface,
                        states,
                        &relative_offset,
                        scale,
                        parent_id.clone(),
                    ) {
                        render_elements.push_front(wvs);
                        surface_info.insert(surface.id(), (surface.clone(), parent_id.clone()));
                    }
                },
                |_, _, _| true,
            );

            // Now build layers from surface_info (like windows)
            for (surface_id, (surface, parent_id)) in surface_info.iter() {
                let layer = self.get_or_create_layer_for_surface(surface);

                // Configure layer with all properties
                if let Some(wvs) = render_elements.iter().find(|e| &e.id == surface_id) {
                    crate::workspaces::utils::configure_surface_layer(&layer, wvs);

                    // Build parent-child hierarchy
                    if let Some(parent_id) = parent_id {
                        if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                            self.layers_engine.append_layer(&layer, parent_layer.id());
                        }
                    } else {
                        // Root surface - attach to DnD content layer
                        self.layers_engine
                            .append_layer(&layer, self.workspaces.dnd_view.content_layer.id());
                    }
                }
            }

            self.workspaces
                .dnd_view
                .layer
                .set_position((cursor_position.x as f32, cursor_position.y as f32), None);
        }
    }

    pub fn cleanup_dnd_layers(&mut self, dnd_surface: &WlSurface) {
        // Remove all layers created for this DnD surface tree
        let mut to_remove = Vec::new();
        smithay::wayland::compositor::with_surface_tree_downward(
            dnd_surface,
            (),
            |_surface, _states, _| TraversalAction::DoChildren(()),
            |surface, _states, _| {
                to_remove.push(surface.id());
            },
            |_, _, _| true,
        );

        for surface_id in to_remove {
            if let Some(layer) = self.surface_layers.remove(&surface_id) {
                layer.remove();
            }
        }
    }

    #[profiling::function]
    pub fn window_view_for_surface(
        &self,
        surface: &WlSurface,
        states: &SurfaceData,
        location: &smithay::utils::Point<f64, smithay::utils::Physical>,
        scale: f64,
        parent_id: Option<smithay::reexports::wayland_server::backend::ObjectId>,
    ) -> Option<WindowViewSurface> {
        let id = surface.id();
        let mut cached_state = states.cached_state.get::<SurfaceCachedState>();
        let cached_state = cached_state.current();
        let surface_geometry = cached_state
            .geometry
            .unwrap_or_default()
            .to_f64()
            .to_physical(scale);
        let mut surface_attributes = states.cached_state.get::<SurfaceAttributes>();
        let surface_attributes = surface_attributes.current();
        if let Some(render_surface) = states.data_map.get::<RendererSurfaceStateUserData>() {
            let render_surface: std::sync::MutexGuard<RendererSurfaceState> =
                render_surface.lock().unwrap();

            if let Some(view) = render_surface.view() {
                let mut texture_id = None;
                if let Some(t) = self.backend_data.texture_for_surface(&render_surface) {
                    // Store for debug comparison rendering (unique per surface id)
                    texture_id = Some(t.tid);
                    crate::textures_storage::set(&id, t);
                }
                let wvs = WindowViewSurface {
                    parent_id, // Track parent for hierarchy
                    id: id.clone(),
                    log_offset_x: location.x as f32,
                    log_offset_y: location.y as f32,

                    phy_src_x: view.src.loc.x as f32 * surface_attributes.buffer_scale as f32,
                    phy_src_y: view.src.loc.y as f32 * surface_attributes.buffer_scale as f32,
                    phy_src_w: view.src.size.w as f32 * surface_attributes.buffer_scale as f32,
                    phy_src_h: view.src.size.h as f32 * surface_attributes.buffer_scale as f32,

                    phy_dst_x: view.offset.x as f32 * scale as f32 - surface_geometry.loc.x as f32,
                    phy_dst_y: view.offset.y as f32 * scale as f32 - surface_geometry.loc.y as f32,
                    phy_dst_w: view.dst.w as f32 * scale as f32,
                    phy_dst_h: view.dst.h as f32 * scale as f32,
                    texture_id,
                    commit: render_surface.current_commit(),
                    transform: surface_attributes.buffer_transform.into(),
                };
                return Some(wvs);
            }
        };
        None
    }

    pub fn update_window_view(&mut self, window: &WindowElement) {
        let scale_factor = Config::with(|c| c.screen_scale);
        if let Some(window_surface) = window.wl_surface() {
            let id = window_surface.id();

            // Ensure all surfaces in the tree have rendering layers before building render elements
            // This only creates layers for surfaces that don't already have them
            self.ensure_surface_tree_layers(&window_surface);

            let location = self
                .workspaces
                .element_location(window)
                .unwrap_or((0, 0).into())
                .to_f64()
                .to_physical(scale_factor);
            let window_geometry = self
                .workspaces
                .element_geometry(window)
                .unwrap_or_default()
                .to_f64()
                .to_physical(scale_factor);
            let title = window.xdg_title();
            let fullscreen = window.xdg_is_fullscreen();

            let mut render_elements = VecDeque::new();

            // Collect popup surfaces and send them to the popup overlay layer
            PopupManager::popups_for_surface(&window_surface).for_each(|(popup, popup_offset)| {
                let offset: smithay::utils::Point<f64, smithay::utils::Physical> =
                    popup_offset.to_physical_precise_round(scale_factor);
                let popup_surface = popup.wl_surface();
                let popup_id = popup_surface.id();

                // Calculate absolute popup position (window position + popup offset)
                let popup_position = layers::types::Point {
                    x: location.x as f32 + offset.x as f32,
                    y: location.y as f32 + offset.y as f32,
                };

                // Collect surfaces for this popup
                let mut popup_surfaces = Vec::new();
                let popup_origin: smithay::utils::Point<f64, smithay::utils::Physical> =
                    (0.0, 0.0).into();
                with_surfaces_surface_tree(popup_surface, |surface, states| {
                    // For popups, parent tracking is simpler - just use None for root
                    // The popup itself is the root of its own surface tree
                    if let Some(window_view) = self.window_view_for_surface(
                        surface,
                        states,
                        &popup_origin,
                        scale_factor,
                        None,
                    ) {
                        popup_surfaces.push(window_view);
                    }
                });

                // Send popup to the overlay layer and register its surface layers
                #[allow(clippy::mutable_key_type)]
                let popup_layers = self.workspaces.popup_overlay.update_popup(
                    &popup_id,
                    &id,
                    popup_position,
                    popup_surfaces,
                    None, // No warm cache needed anymore
                    &self.layers_engine,
                    &self.surface_layers,
                );

                self.surface_layers.extend(popup_layers);
            });

            let initial_location: smithay::utils::Point<f64, smithay::utils::Physical> =
                (0.0, 0.0).into();

            // Track parent through traversal context: (location, parent_location, parent_id)
            let initial_context = (initial_location, initial_location, None);

            // Collect all surfaces and build parent-child map
            #[allow(clippy::mutable_key_type, clippy::type_complexity)]
            let mut surface_info: std::collections::HashMap<
                ObjectId,
                (
                    WlSurface,
                    smithay::utils::Point<f64, smithay::utils::Physical>,
                    Option<ObjectId>,
                ),
            > = std::collections::HashMap::new();

            smithay::wayland::compositor::with_surface_tree_downward(
                &window_surface,
                initial_context,
                |surface, states, (location, _parent_location, _parent_id)| {
                    profiling::scope!("surface_tree_downward");
                    let mut location = *location;
                    let data = states.data_map.get::<RendererSurfaceStateUserData>();
                    let mut cached_state = states.cached_state.get::<SurfaceCachedState>();
                    let cached_state = cached_state.current();
                    let surface_geometry = cached_state.geometry.unwrap_or_default();

                    if let Some(data) = data {
                        let data = data.lock().unwrap();

                        if let Some(view) = data.view() {
                            location += view.offset.to_f64().to_physical(scale_factor);
                            location -= surface_geometry.loc.to_f64().to_physical(scale_factor);
                            TraversalAction::DoChildren((location, location, Some(surface.id())))
                        } else {
                            TraversalAction::SkipChildren
                        }
                    } else {
                        TraversalAction::SkipChildren
                    }
                },
                |surface, states, (location, parent_location, parent_id)| {
                    let relative_offset = if parent_id.is_some() {
                        *location - *parent_location
                    } else {
                        *location
                    };

                    if let Some(window_view) = self.window_view_for_surface(
                        surface,
                        states,
                        &relative_offset,
                        scale_factor,
                        parent_id.clone(),
                    ) {
                        render_elements.push_front(window_view.clone());
                        surface_info.insert(
                            surface.id(),
                            (surface.clone(), *location, parent_id.clone()),
                        );
                    }
                },
                |_, _, _| true,
            );

            // Now sync the layer hierarchy to match the surface tree
            for (surface_id, (surface, _pos, parent_id)) in surface_info.iter() {
                let layer = self.get_or_create_layer_for_surface(surface);

                // Configure layer with all properties and draw callback
                if let Some(wvs) = render_elements.iter().find(|e| &e.id == surface_id) {
                    crate::workspaces::utils::configure_surface_layer(&layer, wvs);

                    // Set up parent-child relationship using layers_engine
                    if let Some(parent_id) = parent_id {
                        if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                            self.layers_engine.append_layer(&layer, parent_layer.id());
                        }
                    }
                }
            }

            if let Some(window_view) = self.workspaces.get_window_view(&id) {
                let model = WindowViewBaseModel {
                    x: location.x as f32,
                    y: location.y as f32,
                    w: window_geometry.size.w as f32,
                    h: window_geometry.size.h as f32,
                    title,
                    fullscreen,
                    active: false,
                };
                window_view.view_base.update_state(&model);

                // Directly add root surface layer to content layer without using LayerTreeBuilder
                let content_layer = &window_view.content_layer;

                if let Some(root_layer) = self.surface_layers.get(&id) {
                    // Use layers_engine to set parent-child relationship
                    self.layers_engine
                        .append_layer(root_layer, content_layer.id());
                }

                self.workspaces.expose_update_if_needed();
            }
        }
    }
    // Commented out - update_layer_surface is no longer used
    // pub fn update_layer_surface(&mut self, surface_id: &ObjectId) {
    //     let Some(layer_shell_surface) = self.layer_surfaces.get(surface_id) else {
    //         return;
    //     };

    //     let scale_factor = Config::with(|c| c.screen_scale);
    //     let wl_surface = layer_shell_surface.layer_surface().wl_surface();

    //     // Get the output geometry to compute surface placement
    //     let output_geometry = self
    //         .workspaces
    //         .output_geometry(layer_shell_surface.output())
    //         .unwrap_or_default();

    //     // Compute the layer surface geometry based on anchors/margins
    //     let geometry = layer_shell_surface.compute_geometry(output_geometry);

    //     // Collect render elements from the surface tree
    //     let mut render_elements: Vec<WindowViewSurface> = Vec::new();
    //     let initial_location: smithay::utils::Point<f64, smithay::utils::Physical> =
    //         (0.0, 0.0).into();

    //     smithay::wayland::compositor::with_surface_tree_downward(
    //         wl_surface,
    //         initial_location,
    //         |_, states, location| {
    //             let mut location = *location;
    //             let data = states
    //                 .data_map
    //                 .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>(
    //             );
    //             let mut cached_state = states.cached_state.get::<SurfaceCachedState>();
    //             let cached_state = cached_state.current();
    //             let surface_geometry = cached_state.geometry.unwrap_or_default();

    //             if let Some(data) = data {
    //                 let data = data.lock().unwrap();
    //                 if let Some(view) = data.view() {
    //                     location += view.offset.to_f64().to_physical(scale_factor);
    //                     location -= surface_geometry.loc.to_f64().to_physical(scale_factor);
    //                     TraversalAction::DoChildren(location)
    //                 } else {
    //                     TraversalAction::SkipChildren
    //                 }
    //             } else {
    //                 TraversalAction::SkipChildren
    //             }
    //         },
    //         |surface, states, location| {
    //             if let Some(wvs) =
    //                 self.window_view_for_surface(surface, states, location, scale_factor)
    //             {
    //                 render_elements.push(wvs);
    //             }
    //         },
    //         |_, _, _| true,
    //     );

    //     // Update the lay_rs layer position and size
    //     let layer = &layer_shell_surface.layer;
    //     layer.set_position(
    //         layers::types::Point {
    //             x: (geometry.loc.x as f64 * scale_factor) as f32,
    //             y: (geometry.loc.y as f64 * scale_factor) as f32,
    //         },
    //         None,
    //     );
    //     layer.set_size(
    //         layers::types::Size::points(
    //             (geometry.size.w as f64 * scale_factor) as f32,
    //             (geometry.size.h as f64 * scale_factor) as f32,
    //         ),
    //         None,
    //     );

    //     // If we have render elements, set up the drawing
    //     if !render_elements.is_empty() {
    //         // Clone what we need for the draw closure
    //         let elements = render_elements.clone();
    //         let width = (geometry.size.w as f64 * scale_factor) as f32;
    //         let height = (geometry.size.h as f64 * scale_factor) as f32;

    //         layer.set_draw_content(move |canvas: &layers::skia::Canvas, _w, _h| {
    //             for wvs in &elements {
    //                 if wvs.phy_dst_w <= 0.0 || wvs.phy_dst_h <= 0.0 {
    //                     continue;
    //                 }
    //                 let tex = crate::textures_storage::get(&wvs.id);
    //                 if let Some(tex) = tex {
    //                     let src_h = (wvs.phy_src_h - wvs.phy_src_y).max(1.0);
    //                     let src_w = (wvs.phy_src_w - wvs.phy_src_x).max(1.0);
    //                     let scale_y = wvs.phy_dst_h / src_h;
    //                     let scale_x = wvs.phy_dst_w / src_w;
    //                     let mut matrix = layers::skia::Matrix::new_identity();
    //                     matrix.pre_translate((-wvs.phy_src_x, -wvs.phy_src_y));
    //                     matrix.pre_scale((scale_x, scale_y), None);

    //                     let sampling = layers::skia::SamplingOptions::from(
    //                         layers::skia::CubicResampler::catmull_rom(),
    //                     );
    //                     let mut paint = layers::skia::Paint::new(
    //                         layers::skia::Color4f::new(1.0, 1.0, 1.0, 1.0),
    //                         None,
    //                     );
    //                     paint.set_shader(tex.image.to_shader(
    //                         (layers::skia::TileMode::Clamp, layers::skia::TileMode::Clamp),
    //                         sampling,
    //                         &matrix,
    //                     ));

    //                     let dst_rect = layers::skia::Rect::from_xywh(
    //                         wvs.phy_dst_x,
    //                         wvs.phy_dst_y,
    //                         wvs.phy_dst_w,
    //                         wvs.phy_dst_h,
    //                     );
    //                     canvas.draw_rect(dst_rect, &paint);
    //                 }
    //             }
    //             layers::skia::Rect::from_xywh(0.0, 0.0, width, height)
    //         });
    //     }
    // }

    pub fn quit_appswitcher_app(&mut self) {
        self.workspaces.quit_appswitcher_app();
        // FIXME focus the previous window
    }
    pub fn toggle_maximize_focused_window(&mut self) {
        let Some(window) = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| match focus {
                KeyboardFocusTarget::Window(window) => Some(window),
                _ => None,
            })
        else {
            return;
        };

        match window.underlying_surface() {
            smithay::desktop::WindowSurface::Wayland(_) => {
                if let Some(toplevel) = window.toplevel() {
                    let toplevel = toplevel.clone();
                    let is_maximized = toplevel.with_pending_state(|state| {
                        state.states.contains(xdg_toplevel::State::Maximized)
                    });
                    if is_maximized {
                        <Self as smithay::wayland::shell::xdg::XdgShellHandler>::unmaximize_request(
                            self, toplevel,
                        );
                    } else {
                        <Self as smithay::wayland::shell::xdg::XdgShellHandler>::maximize_request(
                            self, toplevel,
                        );
                    }
                }
            }
            #[cfg(feature = "xwayland")]
            smithay::desktop::WindowSurface::X11(surface) => {
                if surface.is_maximized() {
                    self.unmaximize_request_x11(&surface);
                } else {
                    self.maximize_request_x11(&surface);
                }
            }
        }
    }
    pub fn close_focused_window(&mut self) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            if let Some(KeyboardFocusTarget::Window(window)) = keyboard.current_focus() {
                match window.underlying_surface() {
                    smithay::desktop::WindowSurface::Wayland(toplevel) => toplevel.send_close(),
                    #[cfg(feature = "xwayland")]
                    smithay::desktop::WindowSurface::X11(surface) => {
                        let _ = surface.close();
                    }
                }
            }
        }
    }
    pub fn raise_next_app_window(&mut self) {
        if let Some(wid) = self.workspaces.raise_next_app_window() {
            self.set_keyboard_focus_on_surface(&wid);
        }
    }
    pub fn focus_app(&mut self, app_id: &str) {
        if let Some(wid) = self.workspaces.focus_app(app_id) {
            self.set_keyboard_focus_on_surface(&wid);
        }
    }
    pub fn set_current_workspace_index(&mut self, index: usize) {
        self.workspaces.set_current_workspace_index(index, None);
        // Focus the top window of the new workspace, or clear focus if empty
        if let Some(top_wid) = self.workspaces.get_top_window_of_workspace(index) {
            self.set_keyboard_focus_on_surface(&top_wid);
        } else {
            self.clear_keyboard_focus();
        }
    }

    pub fn set_keyboard_focus_on_surface(&mut self, wid: &ObjectId) {
        if let Some(window) = self.workspaces.get_window_for_surface(wid) {
            let keyboard = self.seat.get_keyboard().unwrap();
            let serial = SERIAL_COUNTER.next_serial();

            // Deactivate the previously focused window
            if let Some(old_focus) = keyboard.current_focus() {
                if let crate::focus::KeyboardFocusTarget::Window(old_window) = old_focus {
                    if old_window.wl_surface() != window.wl_surface() {
                        old_window.set_activate(false);
                        // Update shadow for deactivated window
                        if let Some(view) = self.workspaces.get_window_view(&old_window.id()) {
                            view.set_active(false);
                        }
                        if let Some(toplevel) = old_window.toplevel() {
                            toplevel.send_configure();
                        }
                    }
                }
            }

            // Activate the new window and send configure
            window.set_activate(true);
            // Update shadow for activated window
            if let Some(view) = self.workspaces.get_window_view(wid) {
                view.set_active(true);
            }
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_configure();
            }
            keyboard.set_focus(self, Some(window.clone().into()), serial);
        }
    }

    pub fn clear_keyboard_focus(&mut self) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            let serial = SERIAL_COUNTER.next_serial();

            // Deactivate the currently focused window when clearing focus
            if let Some(old_focus) = keyboard.current_focus() {
                if let crate::focus::KeyboardFocusTarget::Window(old_window) = old_focus {
                    old_window.set_activate(false);
                    // Update shadow for deactivated window
                    if let Some(view) = self.workspaces.get_window_view(&old_window.id()) {
                        view.set_active(false);
                    }
                    if let Some(toplevel) = old_window.toplevel() {
                        toplevel.send_configure();
                    }
                }
            }

            keyboard.set_focus(self, None, serial);
        }
    }

    /// Inject pre-created surface layers into a View's cache
    /// This allows the View builder to find existing layers instead of creating new ones
    pub fn inject_surface_layers_into_view<S: std::hash::Hash + Clone>(
        &self,
        surface: &WlSurface,
        view: &layers::prelude::View<S>,
    ) {
        use smithay::wayland::compositor::with_surface_tree_downward;
        use smithay::wayland::compositor::TraversalAction;

        with_surface_tree_downward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |sub_surface, _, _| {
                let sub_id = sub_surface.id();
                if let Some(layer) = self.surface_layers.get(&sub_id) {
                    let key = format!("surface_{:?}", sub_id);
                    view.viewlayer_node_map_insert(key, layer.id);
                    tracing::debug!("Injected layer into view cache for {:?}", sub_id);
                }
            },
            |_, _, _| true,
        );
    }

    /// Dismiss all active popups and release any pointer/keyboard grabs
    pub fn dismiss_all_popups(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();

        // Unset pointer grab if active
        if let Some(pointer) = self.seat.get_pointer() {
            if pointer.is_grabbed() {
                pointer.unset_grab(self, serial, 0);
            }
        }

        // Unset keyboard grab if active
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.is_grabbed() {
                keyboard.unset_grab(self);
            }
        }
    }

    pub fn get_gamma_size(&self, output: &Output) -> Option<u32> {
        #[cfg(feature = "udev")]
        {
            use crate::udev::UdevData;
            if let Some(udev_data) =
                (&self.backend_data as &dyn std::any::Any).downcast_ref::<UdevData>()
            {
                use crate::udev::UdevOutputId;
                let output_id = output.user_data().get::<UdevOutputId>()?;
                let backend = udev_data.backends.get(&output_id.device_id)?;
                let drm_fd = backend.drm.device_fd();
                crate::udev::gamma::get_gamma_size(drm_fd, output_id.crtc).ok()
            } else {
                None
            }
        }
        #[cfg(not(feature = "udev"))]
        {
            let _ = output;
            None
        }
    }

    /// Apply gamma LUT to an output (udev backend only)
    /// This version animates the transition over 500ms
    pub fn apply_gamma(
        &mut self,
        output: &Output,
        red: &[u16],
        green: &[u16],
        blue: &[u16],
    ) -> Result<(), String> {
        #[cfg(feature = "udev")]
        {
            use crate::udev::UdevData;
            if let Some(udev_data) =
                (&self.backend_data as &dyn std::any::Any).downcast_ref::<UdevData>()
            {
                use crate::udev::UdevOutputId;
                let output_id = output
                    .user_data()
                    .get::<UdevOutputId>()
                    .ok_or_else(|| "Output has no UdevOutputId".to_string())?;
                let _ = udev_data
                    .backends
                    .get(&output_id.device_id)
                    .ok_or_else(|| "Backend not found".to_string())?;

                // Get current gamma as starting point
                let gamma_size = red.len();
                let current = if let Some((current_r, current_g, current_b)) =
                    self.current_gamma.get(&output.name())
                {
                    // Use actual current gamma from last apply
                    (current_r.clone(), current_g.clone(), current_b.clone())
                } else if let Some((from_r, from_g, from_b, _, _, _, _, _)) =
                    self.gamma_transitions.get(&output.name())
                {
                    // Use the start of ongoing transition as new start
                    (from_r.clone(), from_g.clone(), from_b.clone())
                } else {
                    // Generate linear gamma as default start (first time only)
                    let linear: Vec<u16> = (0..gamma_size)
                        .map(|i| ((i as f64 / (gamma_size - 1) as f64) * 65535.0) as u16)
                        .collect();
                    (linear.clone(), linear.clone(), linear)
                };

                // Store transition
                self.gamma_transitions.insert(
                    output.name(),
                    (
                        current.0,
                        current.1,
                        current.2,
                        red.to_vec(),
                        green.to_vec(),
                        blue.to_vec(),
                        std::time::Instant::now(),
                        std::time::Duration::from_millis(500),
                    ),
                );

                // Trigger render loop to start animation
                self.schedule_event_loop_dispatch();

                Ok(())
            } else {
                Err("Not a udev backend".to_string())
            }
        }
        #[cfg(not(feature = "udev"))]
        {
            let _ = (output, red, green, blue);
            Err("Gamma control not supported on this backend".to_string())
        }
    }

    /// Apply gamma immediately without animation (for internal use)
    pub fn apply_gamma_immediate(
        &self,
        output: &Output,
        red: &[u16],
        green: &[u16],
        blue: &[u16],
    ) -> Result<(), String> {
        #[cfg(feature = "udev")]
        {
            use crate::udev::UdevData;
            if let Some(udev_data) =
                (&self.backend_data as &dyn std::any::Any).downcast_ref::<UdevData>()
            {
                use crate::udev::UdevOutputId;
                let output_id = output
                    .user_data()
                    .get::<UdevOutputId>()
                    .ok_or_else(|| "Output has no UdevOutputId".to_string())?;
                let backend = udev_data
                    .backends
                    .get(&output_id.device_id)
                    .ok_or_else(|| "Backend not found".to_string())?;
                let drm_fd = backend.drm.device_fd();
                crate::udev::gamma::apply_gamma_lut(drm_fd, output_id.crtc, red, green, blue)
            } else {
                Err("Not a udev backend".to_string())
            }
        }
        #[cfg(not(feature = "udev"))]
        {
            let _ = (output, red, green, blue);
            Err("Gamma control not supported on this backend".to_string())
        }
    }

    /// Reset gamma to neutral for an output (udev backend only)
    pub fn reset_gamma(&mut self, output: &Output) -> Result<(), String> {
        #[cfg(feature = "udev")]
        {
            // Generate neutral 6500K gamma LUT
            let size = self
                .get_gamma_size(output)
                .ok_or("Failed to get gamma size")? as usize;
            let neutral = crate::udev::gamma::generate_gamma_lut(6500, size);
            // Use animated apply_gamma for smooth transition back to neutral
            self.apply_gamma(output, &neutral.0, &neutral.1, &neutral.2)
        }
        #[cfg(not(feature = "udev"))]
        {
            let _ = output;
            Err("Gamma control not supported on this backend".to_string())
        }
    }

    /// Tick gamma transitions (called from render loop)
    pub fn tick_gamma_transitions(&mut self) {
        let mut completed = Vec::new();

        for (output_name, (from_r, from_g, from_b, to_r, to_g, to_b, start, duration)) in
            self.gamma_transitions.iter()
        {
            let elapsed = start.elapsed();
            let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0);

            // Interpolate each color channel
            let current_r: Vec<u16> = from_r
                .iter()
                .zip(to_r.iter())
                .map(|(f, t)| (*f as f64 + (*t as f64 - *f as f64) * progress) as u16)
                .collect();
            let current_g: Vec<u16> = from_g
                .iter()
                .zip(to_g.iter())
                .map(|(f, t)| (*f as f64 + (*t as f64 - *f as f64) * progress) as u16)
                .collect();
            let current_b: Vec<u16> = from_b
                .iter()
                .zip(to_b.iter())
                .map(|(f, t)| (*f as f64 + (*t as f64 - *f as f64) * progress) as u16)
                .collect();

            // Apply to output
            if let Some(output) = self.workspaces.outputs().find(|o| &o.name() == output_name) {
                let _ = self.apply_gamma_immediate(output, &current_r, &current_g, &current_b);

                // Update current_gamma tracker
                self.current_gamma.insert(
                    output_name.clone(),
                    (current_r.clone(), current_g.clone(), current_b.clone()),
                );
            }

            // Mark as completed if done
            if progress >= 1.0 {
                completed.push(output_name.clone());
            }
        }

        // Remove completed transitions
        for name in completed {
            self.gamma_transitions.remove(&name);
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct SurfaceDmabufFeedback<'a> {
    pub render_feedback: &'a DmabufFeedback,
    pub scanout_feedback: &'a DmabufFeedback,
}

#[profiling::function]
pub fn post_repaint<'a>(
    output: &Output,
    render_element_states: &RenderElementStates,
    window_elements: &[&WindowElement],
    dmabuf_feedback: Option<SurfaceDmabufFeedback<'_>>,
    time: impl Into<Duration>,
) {
    let time = time.into();
    let throttle = Some(Duration::from_secs(1));

    window_elements.iter().for_each(|window| {
        window.with_surfaces(|surface, states| {
            let primary_scanout_output = update_surface_primary_scanout_output(
                surface,
                output,
                states,
                render_element_states,
                default_primary_scanout_output_compare,
            );

            if let Some(output) = primary_scanout_output {
                with_fractional_scale(states, |fraction_scale| {
                    fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                });
            }
        });
        window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
            Some(output.clone())
        });
        // Send frame to all windows since we're processing all workspaces
        if let Some(dmabuf_feedback) = dmabuf_feedback {
            window.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                select_dmabuf_feedback(
                    surface,
                    render_element_states,
                    dmabuf_feedback.render_feedback,
                    dmabuf_feedback.scanout_feedback,
                )
            });
        }
    });
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.with_surfaces(|surface, states| {
            let primary_scanout_output = update_surface_primary_scanout_output(
                surface,
                output,
                states,
                render_element_states,
                default_primary_scanout_output_compare,
            );

            if let Some(output) = primary_scanout_output {
                with_fractional_scale(states, |fraction_scale| {
                    fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                });
            }
        });

        layer_surface.send_frame(output, time, throttle, surface_primary_scanout_output);
        if let Some(dmabuf_feedback) = dmabuf_feedback {
            layer_surface.send_dmabuf_feedback(
                output,
                surface_primary_scanout_output,
                |surface, _| {
                    select_dmabuf_feedback(
                        surface,
                        render_element_states,
                        dmabuf_feedback.render_feedback,
                        dmabuf_feedback.scanout_feedback,
                    )
                },
            );
        }
    }
}

#[profiling::function]
pub fn take_presentation_feedback<'a>(
    output: &Output,
    window_elements: &[&WindowElement],
    render_element_states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut output_presentation_feedback = OutputPresentationFeedback::new(output);

    window_elements.iter().for_each(|window| {
        // Process all windows since we're handling all workspaces
        window.take_presentation_feedback(
            &mut output_presentation_feedback,
            surface_primary_scanout_output,
            |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, render_element_states)
            },
        );
    });

    // space.elements().for_each(|window| {
    //     if space.outputs_for_element(window).contains(output) {
    //         window.take_presentation_feedback(
    //             &mut output_presentation_feedback,
    //             surface_primary_scanout_output,
    //             |surface, _| {
    //                 surface_presentation_feedback_flags_from_states(surface, render_element_states)
    //             },
    //         );
    //     }
    // });
    // TODO layers presentation feedback
    // let map = smithay::desktop::layer_map_for_output(output);
    // for layer_surface in map.layers() {
    //     layer_surface.take_presentation_feedback(
    //         &mut output_presentation_feedback,
    //         surface_primary_scanout_output,
    //         |surface, _| {
    //             surface_presentation_feedback_flags_from_states(surface, render_element_states)
    //         },
    //     );
    // }

    output_presentation_feedback
}

pub trait Backend {
    const HAS_RELATIVE_MOTION: bool = false;
    const HAS_GESTURES: bool = false;
    fn seat_name(&self) -> String;
    fn backend_name(&self) -> &'static str;
    fn reset_buffers(&mut self, output: &Output);
    fn early_import(&mut self, surface: &WlSurface);
    fn texture_for_surface(&self, surface: &RendererSurfaceState) -> Option<SkiaTextureImage>;
    fn set_cursor(&mut self, image: &CursorImageStatus); //, renderer: &mut SkiaRenderer);
    fn renderer_context(&mut self) -> Option<layers::skia::gpu::DirectContext>;
    fn request_redraw(&mut self) {}
    /// Get GBM device for DMA-BUF screenshare (None for backends without DMA-BUF support)
    fn gbm_device(
        &self,
    ) -> Option<smithay::backend::allocator::gbm::GbmDevice<smithay::backend::drm::DrmDeviceFd>>
    {
        None
    }
    /// Get render format and modifier for screenshare.
    /// Returns (fourcc, modifier) tuple, or None if not available.
    fn render_format(&mut self) -> Option<(u32, u64)> {
        None
    }
    /// Get all supported modifiers for a given format from the backend.
    /// Used for DMA-BUF format negotiation.
    fn get_format_modifiers(&mut self, _fourcc: smithay::backend::allocator::Fourcc) -> Vec<u64> {
        vec![]
    }
    /// Whether this backend prefers DMA-BUF for screenshare (zero-copy)
    fn prefers_dmabuf_screenshare(&self) -> bool {
        false
    }
}
