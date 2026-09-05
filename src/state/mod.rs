use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Debug,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use layers::{engine::Engine, prelude::taffy};
use sd_notify::NotifyState;
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
        utils::{
            surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
            update_surface_primary_scanout_output, with_surfaces_surface_tree,
            OutputPresentationFeedback,
        },
        PopupManager,
    },
    input::{
        keyboard::{Keysym, ModifiersState, XkbConfig},
        pointer::{CursorIcon, CursorImageStatus, PointerHandle},
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
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as KdeDefaultDecorationMode,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason, ObjectId},
            protocol::{wl_data_device_manager::DndAction, wl_surface::WlSurface},
            Display, DisplayHandle, Resource,
        },
    },
    utils::{self, Clock, Monotonic, SERIAL_COUNTER},
    wayland::{
        compositor::{
            CompositorClientState, CompositorState, SurfaceAttributes, SurfaceData, TraversalAction,
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
            data_device::DataDeviceState,
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::PrimarySelectionState, wlr_data_control::DataControlState,
        },
        shell::{
            kde::decoration::KdeDecorationState,
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

#[cfg(feature = "xwayland")]
use crate::cursor::Cursor;
use crate::cursor::{CursorManager, CursorTextureCache};
use crate::{
    audio::{AudioManager, SoundPlayer},
    config::Config,
    render_elements::scene_element::SceneElement,
    shell::{LayerShellSurface, WindowElement},
    skia_renderer::SkiaTextureImage,
    workspaces::{WindowViewBaseModel, WindowViewSurface, Workspaces},
};
#[cfg(feature = "xwayland")]
use smithay::{
    utils::{Point, Size},
    wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState,
    wayland::xwayland_shell,
    xwayland::{X11Wm, XWayland, XWaylandClientData, XWaylandEvent},
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
    /// Whether the fullscreen chrome is currently held visible for a modal
    /// overlay layer surface (a portal Access dialog, say). Fullscreen hides
    /// the layer-shell top/overlay layers and scans the window out directly,
    /// so a dialog that appears afterwards has to undo both — see
    /// `Otto::refresh_modal_overlay`.
    pub modal_overlay_shown: bool,
    /// Session lock state — see `src/lock.rs`. Locked-ness lives here rather
    /// than with the client, so a locker that dies leaves the session locked.
    pub lock_state: crate::lock::LockState,
    /// The locker's surfaces, keyed by output name.
    pub lock_surfaces: crate::lock::LockSurfaces,
    /// Keyboard focus at the moment the lock began, restored on unlock.
    pub lock_previous_focus: Option<crate::focus::KeyboardFocusTarget<BackendData>>,
    /// Set once the locker has mapped a surface. Losing every surface after
    /// that means the locker died, which is what a respawn keys off.
    pub lock_locker_seen: bool,
    /// When the locker was last (re)launched, so a locker that crashes on
    /// startup cannot be respawned in a tight loop.
    pub lock_last_spawn: Option<std::time::Instant>,
    /// When the blank finishes going back up after an unlock. The session is
    /// unlocked for every other purpose from the moment the request arrives,
    /// but the shade is still on screen until this passes and the frame has to
    /// be composited as a whole to carry it — see `Otto::lock_blank_on_screen`.
    pub lock_shade_until: Option<std::time::Instant>,
    /// When the user last did anything. Auto-lock (`lock.auto_lock_timeout`)
    /// measures idleness from here — see `Otto::note_input_activity`.
    pub lock_last_activity: std::time::Instant,
    /// The auto-lock timer's event source, kept so that changing the timeout
    /// can drop the old one: the timer holds its interval, so re-arming is the
    /// only way to follow a new `lock.auto_lock_timeout`. `None` while
    /// auto-locking is off, which is also its state when the timeout is 0.
    pub auto_lock_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    pub workspaces: Workspaces,

    // smithay state
    pub compositor_state: CompositorState,
    pub data_device_state: DataDeviceState,
    pub layer_shell_state: WlrLayerShellState,
    pub session_lock_manager_state: smithay::wayland::session_lock::SessionLockManagerState,
    #[allow(dead_code)]
    pub idle_inhibit_manager_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    /// Surfaces holding an `idle-inhibit-unstable-v1` inhibitor — a client
    /// saying "don't consider the session idle" (a video playing, a
    /// presentation). Consulted by auto-lock; see `Otto::idle_inhibited`.
    pub idle_inhibitors: HashSet<WlSurface>,
    /// The window the pointer last scrolled, and when.
    ///
    /// A window being scrolled is being used, whether or not it holds
    /// keyboard focus, so it keeps full-rate frame callbacks for a moment
    /// afterwards — see [`crate::state::window_throttle`]. Without this a
    /// click-to-focus desktop delivers a scroll to a `Secondary` window and
    /// then feeds it frames at 30 Hz while the user watches it move.
    pub pointer_interaction: Option<(ObjectId, std::time::Instant)>,
    pub output_manager_state: OutputManagerState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    /// `ext-data-control-v1`, the standardised successor to
    /// `zwlr-data-control`. Both are advertised: older clipboard managers only
    /// know the wlr one, newer ones only the ext one, and neither is a superset
    /// of the other in the wild.
    pub ext_data_control_state: ExtDataControlState,
    pub seat_state: SeatState<Otto<BackendData>>,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub shm_state: ShmState,
    pub viewporter_state: ViewporterState,
    pub xdg_activation_state: XdgActivationState,
    pub xdg_decoration_state: XdgDecorationState,
    /// KDE's legacy server-decoration protocol. GTK apps that offer a
    /// server-side mode (ghostty) only look for this one, never
    /// `xdg-decoration`.
    pub kde_decoration_state: KdeDecorationState,
    /// Decoration modes negotiated over the KDE protocol before the surface
    /// had an `xdg_toplevel`. GTK asks for its mode on the bare `wl_surface`,
    /// one request ahead of `get_toplevel`, so there is no window to flag yet
    /// — `new_toplevel` replays what landed here.
    pub pending_kde_decorations: HashMap<ObjectId, bool>,
    pub xdg_shell_state: XdgShellState,
    pub presentation_state: PresentationState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub xdg_foreign_state: XdgForeignState,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    pub wlr_foreign_toplevel_state: wlr_foreign_toplevel::WlrForeignToplevelManagerState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    pub virtual_keyboard_manager_state: VirtualKeyboardManagerState,
    pub screencopy_manager_state: screencopy::ScreencopyManagerState,
    pub pending_screencopy_frames: Vec<screencopy::PendingScreencopy>,
    pub virtual_pointer_manager_state: virtual_pointer::VirtualPointerManagerState,

    #[cfg(feature = "xwayland")]
    pub xwayland_shell_state: xwayland_shell::XWaylandShellState,

    pub dnd_icon: Option<WlSurface>,
    /// Every surface a drag icon has had a layer built for, so the next drag
    /// can sweep them whether or not those surfaces are still alive.
    pub dnd_layer_ids: Vec<ObjectId>,
    /// The icon surface of a refused drag, whose layers are kept alive while it
    /// flies back to where the drag started and are swept up when the next drag
    /// begins. See [`crate::state::dnd_grab_handler`].
    pub pending_dnd_cleanup: Option<WlSurface>,
    /// Where the drag icon sits relative to the cursor.
    ///
    /// A client anchors the icon by the point it was grabbed by — `wl_surface
    /// .offset`, or the deprecated x and y of `attach` — which arrives as
    /// `buffer_delta` on each commit and is *relative*, so it accumulates over
    /// the drag rather than replacing what came before. Without this the icon
    /// hangs by its top-left corner whatever the client asks for.
    pub dnd_icon_offset: utils::Point<i32, utils::Logical>,

    // input-related fields
    pub suppressed_keys: Vec<Keysym>,
    pub current_modifiers: ModifiersState,
    /// Physical Cmd keys (`<LWIN>`/`<RWIN>`) currently held down.
    ///
    /// `altwin:ctrl_win` maps those keys onto the Control modifier, so by the
    /// time a key reaches us Cmd+C and Ctrl+C are the same event. The keycode
    /// is the one thing xkb leaves untouched, so tracking it here is what lets
    /// shortcut matching tell the two apart. See
    /// [`crate::input::keyboard::shortcut_modifiers`].
    pub pressed_cmd_keys: HashSet<u32>,
    pub app_switcher_hold_modifiers: Option<ModifiersState>,
    pub cursor_status: Arc<Mutex<CursorImageStatus>>,
    pub cursor_manager: CursorManager,
    pub cursor_texture_cache: CursorTextureCache,
    pub seat_name: String,
    pub seat: Seat<Otto<BackendData>>,
    pub clock: Clock<Monotonic>,
    pub pointer: PointerHandle<Otto<BackendData>>,
    /// Cached pointer location (logical) to avoid deadlock when accessing during button events
    pub last_pointer_location: (f64, f64),
    /// When and where the last press on a server-side titlebar landed, for
    /// double-click detection. Kept here rather than on the decoration view:
    /// that view is rebuilt on every hit test, so anything it remembers is
    /// gone by the next event.
    /// `(when, titlebar-local point, window key)`.
    pub last_titlebar_press: Option<(std::time::Instant, (f32, f32), usize)>,
    /// Cached pointer position in physical pixels, updated on every pointer move.
    /// Use `get_cursor_position()` to read. This avoids the deadlock that occurs
    /// when locking `cursor_status` from button/DnD handlers.
    pub cursor_physical_position: (f64, f64),

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
    /// The XWayland client connection, kept so its `client_scale` can be updated
    /// when the output scale changes (see `update_xwayland_scale`).
    #[cfg(feature = "xwayland")]
    pub xwayland_client: Option<smithay::reexports::wayland_server::Client>,

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

    /// Virtual outputs defined in config, each streamed via PipeWire.
    pub virtual_outputs: Vec<crate::virtual_output::VirtualOutputState>,

    /// Accessibility — the keyboard monitor assistive technologies grab keys
    /// through, and the D-Bus half of it until the service thread takes it.
    pub a11y: crate::a11y::A11yState,

    // foreign toplevel list - maps surface ObjectId to unified toplevel handles (both protocols)
    pub foreign_toplevels: HashMap<ObjectId, foreign_toplevel_shared::ForeignToplevelHandles>,

    /// Toplevels mapped but not yet placed against their real size, and the
    /// size the last commit reported for each (`None` until the first sized
    /// commit).
    ///
    /// Initial placement runs before a client has configured, so it can only
    /// guess at dimensions. Windows land here at map time and are re-placed as
    /// their real size arrives — see `Otto::settle_initial_placement`.
    pub pending_initial_placement:
        HashMap<ObjectId, Option<smithay::utils::Size<i32, smithay::utils::Logical>>>,

    // surface style protocol
    // Map from surface ID to list of surface styles augmenting that surface
    pub surfaces_style: HashMap<ObjectId, Vec<crate::surface_style::SurfaceStyle>>,
    pub style_transactions: HashMap<ObjectId, crate::surface_style::StyleTransaction>,
    /// `ext-background-effect-v1`: surfaces whose scene layer this compositor
    /// switched to `BackgroundBlur` because the client committed a blur
    /// region. Keyed by surface, present only while the blur is on — see
    /// `Otto::apply_background_effect`.
    pub background_effects:
        HashMap<ObjectId, (smithay::utils::Rectangle<i32, smithay::utils::Logical>, i32)>,
    // Map from surface ID to its rendering layer in the scene graph
    pub surface_layers: HashMap<ObjectId, layers::prelude::Layer>,
    /// Tracks which parent ObjectId each surface layer was last appended to,
    /// so we can skip redundant append_layer calls that cause flicker.
    pub surface_layer_parents: HashMap<ObjectId, ObjectId>,
    /// The sibling order each parent's children were last appended in,
    /// bottom to top.
    ///
    /// `append_layer` puts a layer last, so the scene's sibling order is
    /// whatever order the layers were first appended in — creation order. That
    /// makes `wl_subsurface.place_above` invisible: a subsurface created later
    /// stays on top of one a client has since raised above it. Re-appending
    /// every commit would fix the order and cause flicker, so the order is
    /// remembered and the children are re-appended only when it changes.
    pub surface_children_order: HashMap<ObjectId, Vec<ObjectId>>,
    // Pre-warmed View caches: surface_id -> (layer_key -> NodeRef)
    // Built during surface creation, moved into Views when they're created
    pub view_warm_cache:
        HashMap<ObjectId, HashMap<String, std::collections::VecDeque<layers::prelude::NodeRef>>>,

    // otto_dock protocol
    pub otto_dock: crate::otto_dock::handlers::OttoDockState,
    pub dock_item_surfaces: HashMap<ObjectId, crate::otto_dock::DockItem>,
    // Rendering metrics
    #[cfg(feature = "metrics")]
    pub render_metrics: Arc<crate::render_metrics::RenderMetrics>,
}

pub mod app_management;
pub mod data_device_handler;
pub mod dnd_grab_handler;
pub mod foreign_toplevel_list_handler;
pub mod foreign_toplevel_shared;
pub mod fractional_scale_handler;
pub mod gamma_control;
pub mod input_method_handler;
pub mod screencopy;
pub mod seat_handler;
pub mod security_context_handler;
pub mod selection_handler;
pub mod session_lock_handler;
pub mod virtual_keyboard_handler;
pub mod virtual_pointer;
pub mod window_throttle;
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
    Detecting {
        accumulated: (f64, f64),
    },
    WorkspaceSwitching {
        velocity_samples: Vec<f64>,
        /// Name of the output this swipe targets (output under pointer at gesture start)
        output_name: String,
    },
    Expose {
        velocity_samples: Vec<f64>,
    },
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

// wlr-virtual-pointer-unstable-v1 delegates. Hand-rolled because Smithay
// doesn't ship a virtual-pointer module; the impls live in
// `state::virtual_pointer` and we dispatch to them here.
smithay::reexports::wayland_server::delegate_global_dispatch!(
    @<BackendData: Backend + 'static> Otto<BackendData>:
    [smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1: ()]
    => virtual_pointer::VirtualPointerManagerState
);
smithay::reexports::wayland_server::delegate_dispatch!(
    @<BackendData: Backend + 'static> Otto<BackendData>:
    [smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1: ()]
    => virtual_pointer::VirtualPointerManagerState
);
smithay::reexports::wayland_server::delegate_dispatch!(
    @<BackendData: Backend + 'static> Otto<BackendData>:
    [smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1: virtual_pointer::VirtualPointerUserData]
    => virtual_pointer::VirtualPointerManagerState
);
delegate_pointer_gestures!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_relative_pointer!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_viewporter!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_xdg_shell!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_layer_shell!(@<BackendData: Backend + 'static> Otto<BackendData>);
smithay::delegate_session_lock!(@<BackendData: Backend + 'static> Otto<BackendData>);
smithay::delegate_idle_inhibit!(@<BackendData: Backend + 'static> Otto<BackendData>);

impl<BackendData: Backend + 'static> smithay::wayland::idle_inhibit::IdleInhibitHandler
    for Otto<BackendData>
{
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.insert(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.remove(&surface);
    }
}
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
    crate::otto_dock::protocol::gen::otto_dock_manager_v1::OttoDockManagerV1: ()
] => crate::otto_dock::handlers::OttoDockState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    crate::otto_dock::protocol::gen::otto_dock_manager_v1::OttoDockManagerV1: ()
] => crate::otto_dock::handlers::OttoDockState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    crate::otto_dock::protocol::gen::otto_dock_item_v1::OttoDockItemV1: crate::otto_dock::protocol::DockItem
] => crate::otto_dock::handlers::OttoDockState);

smithay::reexports::wayland_server::delegate_global_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: ()
] => screencopy::ScreencopyManagerState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: ()
] => screencopy::ScreencopyManagerState);

smithay::reexports::wayland_server::delegate_dispatch!(@<BackendData: Backend + 'static> Otto<BackendData>: [
    smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1: screencopy::ScreencopyFrameData
] => screencopy::ScreencopyManagerState);

impl<BackendData: Backend + 'static> Otto<BackendData> {
    pub fn init(
        display: Display<Otto<BackendData>>,
        handle: LoopHandle<'static, Otto<BackendData>>,
        backend_data: BackendData,
        listen_on_socket: bool,
    ) -> Otto<BackendData> {
        let dh = display.handle();

        // otto-kit draws the window decorations and reads the accent from its
        // own store; fill it before anything renders so the titlebar controls
        // start out the configured colour rather than otto-kit's fallback.
        crate::theme::publish_accent();

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

            // Export WAYLAND_DISPLAY so bus-activated helpers — the portal
            // backend, the file picker, the islands — can find us. Without it
            // they are started by the systemd user manager with whatever
            // WAYLAND_DISPLAY it happens to hold and fail with `NoCompositor`.
            //
            // Only the backend that *is* the session does this. A nested Otto
            // (`--winit`, `--x11`) is a client of somebody else's compositor,
            // and its socket is not where the session's helpers should
            // connect; exporting from one leaves the value pointing at a
            // socket that disappears when the dev run ends, which breaks
            // every bus-activated helper in the real session until the next
            // login. Both are also handed to dbus, because a session whose
            // dbus-daemon is not the systemd one activates from its own
            // environment and never reads systemd's.
            let owns_the_session = matches!(backend_data.backend_name(), "udev");
            if !owns_the_session {
                info!(
                    backend = backend_data.backend_name(),
                    name = socket_name,
                    "Nested backend: not exporting WAYLAND_DISPLAY to the session"
                );
            } else {
                // The language goes out with the socket: a bus-activated
                // service is not Otto's child and inherits nothing from it, so
                // without this a portal or a helper started on demand would
                // run in the session's `LANG` while everything Otto spawned
                // itself runs in the configured one.
                let mut assignments = vec![format!("WAYLAND_DISPLAY={socket_name}")];
                assignments.extend(crate::locale_env::published().iter().cloned());
                assignments.push(crate::export_rounded_corners());
                assignments.push(crate::export_window_controls_side());
                assignments.push(crate::export_maximize_button());
                assignments.push(crate::export_color_scheme());
                let args: Vec<&str> = assignments.iter().map(String::as_str).collect();

                let mut systemctl = vec!["--user", "set-environment"];
                systemctl.extend_from_slice(&args);
                let mut dbus = vec!["--systemd"];
                dbus.extend_from_slice(&args);
                let exports = [
                    ("systemctl", systemctl),
                    ("dbus-update-activation-environment", dbus),
                ];
                for (program, args) in exports {
                    match std::process::Command::new(program).args(&args).output() {
                        Ok(out) if out.status.success() => {}
                        Ok(out) => warn!(
                            program,
                            status = ?out.status,
                            stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                            "Failed to export the session environment"
                        ),
                        Err(e) => {
                            warn!(program, error = ?e, "Failed to export the session environment")
                        }
                    }
                }
                info!(
                    name = socket_name,
                    locale = ?crate::locale_env::published(),
                    "Exported the session environment to systemd and dbus"
                );
            }

            // Notify systemd that the compositor is ready (opt-in via --systemd-notify or config)
            let systemd_notify_enabled =
                Config::with(|c| c.systemd_notify) || std::env::var("OTTO_SYSTEMD_NOTIFY").is_ok();
            if systemd_notify_enabled {
                if let Err(e) = sd_notify::notify(false, &[NotifyState::Ready]) {
                    warn!(error = ?e, "Failed to send sd_notify READY=1");
                } else {
                    info!("Sent sd_notify READY=1 to systemd");
                }
                // Activate graphical-session.target so dependent user services can start
                if let Err(e) = std::process::Command::new("systemctl")
                    .args(["--user", "start", "graphical-session.target"])
                    .output()
                {
                    warn!(error = ?e, "Failed to start graphical-session.target");
                } else {
                    info!("Started systemd user graphical-session.target");
                }
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
        let session_lock_manager_state =
            smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        let idle_inhibit_manager_state =
            smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let mut seat_state = SeatState::new();
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        // Advertise server-side as the default mode, matching what Otto
        // answers over `xdg-decoration`.
        let kde_decoration_state =
            KdeDecorationState::new::<Self>(&dh, KdeDefaultDecorationMode::Server);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let presentation_state = PresentationState::new::<Self>(&dh, clock.id() as u32);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        TextInputManagerState::new::<Self>(&dh);
        InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        let virtual_keyboard_manager_state =
            VirtualKeyboardManagerState::new::<Self, _>(&dh, |_client| true);
        let screencopy_manager_state = screencopy::ScreencopyManagerState::new::<BackendData>(&dh);
        let virtual_pointer_manager_state =
            virtual_pointer::VirtualPointerManagerState::new::<BackendData>(&dh);
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
        crate::surface_style::create_style_manager_global::<BackendData>(&dh);

        // ext-background-effect-v1: the standard way for a terminal or panel
        // to ask for the frost otto-surface-style gives otto-kit apps.
        crate::background_effect::BackgroundEffectState::new::<Self>(&dh);

        // Create otto_dock protocol global
        let otto_dock = crate::otto_dock::handlers::OttoDockState::new::<Self>(&dh);

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
        root_layer.set_key("otto_root");
        root_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        let _ = layers_engine.add_layer(&root_layer);
        let scene_element = SceneElement::with_engine(layers_engine.clone());
        let (workspaces, remove_workspace_receiver, rename_workspace_receiver) =
            Workspaces::new(layers_engine.clone(), dh.clone());
        handle
            .insert_source(remove_workspace_receiver, |event, _, otto| {
                // Channel carries (Option<output_name>, position). Some(name)
                // = per-output removal (a selector's remove button, workspaces
                // being independent per output); None = lockstep removal across
                // all outputs (fullscreen-close, whose workspace is created
                // lockstep).
                if let ChannelEvent::Msg((output_name, pos)) = event {
                    match output_name {
                        Some(name) => otto.workspaces.remove_workspace_from_output(&name, pos),
                        None => otto.workspaces.remove_workspace_at(pos),
                    }
                }
            })
            .expect("Failed to register workspace remove channel");
        handle
            .insert_source(rename_workspace_receiver, |event, _, otto| {
                // A rename that ended without a `&mut Otto` at hand — the
                // selector lost keyboard focus mid-edit.
                if let ChannelEvent::Msg((output_name, index, name)) = event {
                    otto.workspaces
                        .rename_workspace(&output_name, index, Some(name));
                }
            })
            .expect("Failed to register workspace rename channel");

        #[cfg(feature = "debugger")]
        layers_engine.start_debugger();

        // `None` unless `lock.auto_lock_timeout` is set.
        let auto_lock_timer = Self::start_auto_lock_timer(&handle);

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
            modal_overlay_shown: false,
            compositor_state,
            data_device_state,
            layer_shell_state,
            session_lock_manager_state,
            idle_inhibit_manager_state,
            idle_inhibitors: HashSet::new(),
            pointer_interaction: None,
            lock_state: crate::lock::LockState::Unlocked,
            lock_surfaces: Default::default(),
            lock_previous_focus: None,
            lock_locker_seen: false,
            lock_last_spawn: None,
            lock_shade_until: None,
            lock_last_activity: std::time::Instant::now(),
            auto_lock_timer,
            output_manager_state,
            primary_selection_state,
            data_control_state,
            ext_data_control_state,
            seat_state,
            keyboard_shortcuts_inhibit_state,
            shm_state,
            viewporter_state,
            xdg_activation_state,
            xdg_decoration_state,
            kde_decoration_state,
            pending_kde_decorations: HashMap::new(),
            xdg_shell_state,
            presentation_state,
            fractional_scale_manager_state,
            xdg_foreign_state,
            foreign_toplevel_list_state,
            wlr_foreign_toplevel_state,
            cursor_shape_manager_state,
            virtual_keyboard_manager_state,
            screencopy_manager_state,
            pending_screencopy_frames: Vec::new(),
            virtual_pointer_manager_state,
            dnd_icon: None,
            dnd_layer_ids: Vec::new(),
            pending_dnd_cleanup: None,
            dnd_icon_offset: (0, 0).into(),
            suppressed_keys: Vec::new(),
            current_modifiers: ModifiersState::default(),
            pressed_cmd_keys: HashSet::new(),
            app_switcher_hold_modifiers: None,
            cursor_status,
            cursor_manager,
            cursor_texture_cache,
            seat_name,
            seat,
            pointer,
            last_pointer_location: (0.0, 0.0),
            last_titlebar_press: None,
            cursor_physical_position: (0.0, 0.0),
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
            #[cfg(feature = "xwayland")]
            xwayland_client: None,
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
            virtual_outputs: Vec::new(),

            // accessibility
            a11y: crate::a11y::A11yState::new(),

            // foreign toplevel list
            foreign_toplevels: HashMap::new(),
            pending_initial_placement: HashMap::new(),

            // Surface style protocol
            surfaces_style: HashMap::new(),
            style_transactions: HashMap::new(),
            background_effects: HashMap::new(),
            surface_layer_parents: HashMap::new(),
            surface_children_order: HashMap::new(),
            surface_layers: HashMap::new(),
            view_warm_cache: HashMap::new(),

            // otto_dock protocol
            otto_dock,
            dock_item_surfaces: HashMap::new(),
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

    pub fn usable_zone(&self, output: &Output) -> utils::Rectangle<i32, utils::Logical> {
        let output_geom = self.workspaces.output_geometry(output).unwrap();
        let zones = self
            .exclusive_zones
            .get(&output.name())
            .cloned()
            .unwrap_or_default();
        let mut usable_zone = zones.apply_to_output(output_geom);

        // When autohide is enabled the dock slides away, so tiled/maximized
        // windows can use the full height; otherwise stop above the dock.
        // The dock is rendered on the primary output only — other outputs
        // keep their full height.
        let is_primary = self
            .workspaces
            .primary_output()
            .is_some_and(|p| p.name() == output.name());
        //  itself skips an autohidden dock.
        if is_primary {
            self.workspaces.subtract_dock(&mut usable_zone);
        }

        usable_zone
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

        // Seed the XWayland client's `client_scale` from the primary output's
        // integer scale, BEFORE XWayland binds wl_output. smithay sends
        // `wl_output.scale = integer_scale / client_scale`, so with
        // client_scale == integer_scale XWayland receives scale 1 and builds an
        // X screen at the native PHYSICAL resolution (e.g. 2880x1920) instead of
        // the downscaled logical size (1440x960). This makes XRandR report the
        // correct native modes to X11 clients (Unity/Proton games query these to
        // pick a fullscreen resolution); smithay transparently scales the X11
        // surfaces back into logical space. Without this the game reads back a
        // half-size screen and bails out of fullscreen.
        self.set_xwayland_client_scale(&client);
        self.xwayland_client = Some(client.clone());

        let ret = self
            .handle
            .insert_source(xwayland, move |event, _, data| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    let mut wm = X11Wm::start_wm(
                        data.handle.clone(),
                        &data.display_handle,
                        x11_socket,
                        client.clone(),
                    )
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
                    // The native-resolution X screen (see client_scale above)
                    // leaves X11 apps rendering at scale 1 — publish the scale
                    // via XSETTINGS so toolkits (GTK, Chromium/CEF — e.g. the
                    // Steam UI) scale themselves. Games ignore XSETTINGS and
                    // keep the native resolution.
                    data.apply_xwayland_xsettings();
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

    /// The integer scale XWayland should run its X screen at — taken from the
    /// primary (first) output. XWayland's `client_scale` is a single global
    /// value, so multi-output mixed-DPI is a known compromise (as in mutter).
    #[cfg(feature = "xwayland")]
    fn xwayland_target_scale(&self) -> f64 {
        self.workspaces
            .outputs()
            .next()
            .map(|o| o.current_scale().integer_scale() as f64)
            .unwrap_or(1.0)
    }

    /// Apply the primary output's integer scale to the given XWayland client's
    /// `client_scale`. See `start_xwayland` for why this yields a native-resolution
    /// X screen.
    #[cfg(feature = "xwayland")]
    fn set_xwayland_client_scale(&self, client: &smithay::reexports::wayland_server::Client) {
        let scale = self.xwayland_target_scale();
        if let Some(data) = client.get_data::<XWaylandClientData>() {
            data.compositor_state.set_client_scale(scale);
            tracing::info!(
                scale,
                "set XWayland client_scale (native-resolution X screen)"
            );
        }
    }

    /// Re-apply the XWayland `client_scale` after the output scale changes at
    /// runtime, so the X screen tracks the new native resolution.
    #[cfg(feature = "xwayland")]
    pub fn update_xwayland_scale(&mut self) {
        if let Some(client) = self.xwayland_client.clone() {
            self.set_xwayland_client_scale(&client);
        }
        self.apply_xwayland_xsettings();
    }

    /// Publish the primary output's scale to X11 clients via XSETTINGS.
    ///
    /// The native-resolution X screen (`client_scale`, see `start_xwayland`)
    /// makes X11 apps render at scale 1; well-behaved toolkits (GTK,
    /// Chromium/CEF — e.g. the Steam UI) read `Gdk/WindowScalingFactor` and
    /// `Xft/DPI` from XSETTINGS and scale themselves back up. Games ignore
    /// XSETTINGS and keep rendering at the native resolution. Same approach
    /// as mutter's xwayland-native-scaling.
    #[cfg(feature = "xwayland")]
    pub fn apply_xwayland_xsettings(&mut self) {
        use smithay::xwayland::xwm::settings::Value;
        let scale = self.xwayland_target_scale();
        let Some(wm) = self.xwm.as_mut() else { return };
        let settings = [
            (
                "Gdk/WindowScalingFactor".to_string(),
                Value::Integer(scale as i32),
            ),
            // Base DPI before the window scaling factor (96 in 1024ths).
            ("Gdk/UnscaledDPI".to_string(), Value::Integer(96 * 1024)),
            // Effective DPI (in 1024ths) for toolkits without integer-scale
            // support (Xft consumers, Chromium/CEF).
            (
                "Xft/DPI".to_string(),
                Value::Integer((96.0 * scale * 1024.0) as i32),
            ),
        ];
        match wm.set_xsettings(settings.into_iter()) {
            Ok(()) => tracing::info!(scale, "published XWayland XSETTINGS scale"),
            Err(err) => tracing::warn!(?err, "failed to set XWayland XSETTINGS"),
        }
    }

    pub fn set_cursor(&mut self, image: &CursorImageStatus) {
        *self.cursor_status.lock().unwrap() = image.clone();
        self.cursor_manager.set_cursor_image(image.clone());
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
            // No action the target will take — say so, rather than saying
            // nothing. Hiding the cursor here left the user dragging with no
            // pointer at all for most of the screen, since every surface that
            // is not a drop target lands in this branch.
            CursorImageStatus::Named(CursorIcon::NoDrop)
        };
        self.set_cursor(&cursor);
    }

    /// Returns the cached cursor position in physical pixels.
    ///
    /// This is updated on every pointer move event and is lock-free,
    /// so it can safely be called from any handler (including button
    /// and DnD handlers) without risking a deadlock.
    pub fn get_cursor_position(&self) -> utils::Point<f64, utils::Physical> {
        (
            self.cursor_physical_position.0,
            self.cursor_physical_position.1,
        )
            .into()
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

            // Track per-parent child ordering for subsurface reordering
            #[allow(clippy::mutable_key_type)]
            let mut children_order: std::collections::HashMap<ObjectId, Vec<ObjectId>> =
                std::collections::HashMap::new();

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
                        let sid = surface.id();
                        surface_info.insert(sid.clone(), (surface.clone(), parent_id.clone()));
                        if let Some(pid) = parent_id {
                            children_order.entry(pid.clone()).or_default().push(sid);
                        }
                    }
                },
                |_, _, _| true,
            );

            // Now build layers from surface_info (like windows)
            for (surface_id, (surface, parent_id)) in surface_info.iter() {
                let layer = self.get_or_create_layer_for_surface(surface);
                // Remembered so the next drag can take it down even if the
                // client that owned it is gone by then.
                if !self.dnd_layer_ids.contains(surface_id) {
                    self.dnd_layer_ids.push(surface_id.clone());
                }

                // Configure layer with all properties
                if let Some(wvs) = render_elements.iter().find(|e| &e.id == surface_id) {
                    let style = self.surfaces_style.get(surface_id).and_then(|v| v.first());
                    let gravity = style.map(|s| s.contents_gravity).unwrap_or_default();
                    let client_owns_size = style.map(|s| s.client_owns_size).unwrap_or(false);
                    let shared_gravity = style.map(|s| s.shared_gravity.clone());
                    crate::workspaces::utils::configure_surface_layer(
                        &layer,
                        wvs,
                        gravity,
                        client_owns_size,
                        shared_gravity,
                    );

                    // Build parent-child hierarchy
                    if let Some(parent_id) = parent_id {
                        if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                            let _ = self.layers_engine.append_layer(&layer, parent_layer.id());
                        }
                    } else {
                        // Root surface - attach to DnD content layer
                        let _ = self
                            .layers_engine
                            .append_layer(&layer, self.workspaces.dnd_view.content_layer.id());
                    }
                }
            }

            // Re-append children in Smithay's subsurface order
            for (parent_id, child_ids) in children_order.iter() {
                if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                    let parent_node = parent_layer.id();
                    for child_id in child_ids {
                        if let Some(child_layer) = self.surface_layers.get(child_id) {
                            let _ = self.layers_engine.append_layer(child_layer, parent_node);
                        }
                    }
                }
            }

            // The cursor, shifted by wherever the client anchored the icon.
            let anchor = self.dnd_icon_offset.to_f64().to_physical(scale);
            self.workspaces.dnd_view.layer.set_position(
                (
                    (cursor_position.x + anchor.x) as f32,
                    (cursor_position.y + anchor.y) as f32,
                ),
                None,
            );
        }
    }

    /// Take down every layer any drag icon has ever put in the scene.
    ///
    /// [`Self::cleanup_dnd_layers`] walks a *live* surface tree, so it sweeps
    /// nothing once the client is gone — and a drag icon outlives its drag by
    /// design, so the client exiting between drags is the ordinary case, not a
    /// rare one. What is left behind stays parented under the view and is shown
    /// again the moment the next drag makes that view visible: the previous
    /// drag's picture, drawn behind the current one.
    ///
    /// Tracked by id rather than by surface for the same reason: the ids are
    /// ours and stay valid, while the surfaces they came from may not.
    fn sweep_dnd_layers(&mut self) {
        for surface_id in std::mem::take(&mut self.dnd_layer_ids) {
            if let Some(layer) = self.surface_layers.remove(&surface_id) {
                layer.remove();
            }
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

    /// Immediately demote `window` from direct scanout (if promoted) and
    /// re-import its buffer so the scene shows current content. Call before
    /// starting an animation that renders the window's scene content (e.g.
    /// the minimize genie): while promoted, the content layer is blanked and
    /// commits skip the scene import, so the animation would otherwise run
    /// on stale or empty content until the next frame demotes the window.
    pub fn demote_scanout_window(&mut self, window: &WindowElement) {
        let id = window.id();
        #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
        let ids = self.workspaces.scanout_window_ids();
        if ids.contains(&id) {
            tracing::info!(target: "otto::planes", "demoting {:?} from scanout (pre-animation)", id);
            self.workspaces.remove_scanout_window(&id);
            self.update_window_view(window);
            // The unhide + re-import above mutated the scene AFTER this
            // frame's vblank-prefetched engine update. Drawing from the
            // stale prefetch drops the client plane while the windows plane
            // still shows the blanked pre-demotion state — a one-frame
            // flash of missing/stale window content (racy: only when the
            // demote lands inside the prefetch window).
            self.backend_data.invalidate_scene_prefetch();
            self.backend_data.request_redraw();
        }
    }

    /// Demote every promoted window and re-import its buffer. Used when
    /// entering the expose overview: mirrors draw the scene content, which is
    /// blanked while a window sits on a scanout plane.
    #[allow(clippy::mutable_key_type)]
    pub fn demote_all_scanout_windows(&mut self) {
        let ids: Vec<_> = self.workspaces.scanout_window_ids().into_iter().collect();
        for id in ids {
            if let Some(w) = self.workspaces.get_window_for_surface(&id).cloned() {
                self.demote_scanout_window(&w);
            }
        }
    }

    /// Full re-sync of a window's surface tree and all of its popups.
    pub fn update_window_view(&mut self, window: &WindowElement) {
        self.update_window_view_for_commit(window, None);
    }

    /// Re-sync a window's scene layers after a commit.
    ///
    /// `committed` is the surface whose commit triggered this, when there is
    /// one. It scopes the POPUP half of the sync: popups are children of the
    /// window for bookkeeping, but a commit on the window's own content says
    /// nothing about them, and walking their surface trees to write back
    /// identical values is how a client repainting at frame rate ended up
    /// dirtying its own tooltip at frame rate — which then drove a full-screen
    /// backdrop rebuild per commit (see `udev::backdrop`). A popup is re-synced
    /// only when the commit is its own, when it moved, or when `committed` is
    /// `None` (an unscoped caller: geometry change, scanout demote, teardown).
    ///
    /// The window's own surface tree is always walked — the commit could have
    /// landed on any surface in it — but `configure_surface_layer` skips
    /// surfaces whose configuration is unchanged, so only what actually moved
    /// or redrew reaches the scene.
    pub fn update_window_view_for_commit(
        &mut self,
        window: &WindowElement,
        committed: Option<&WlSurface>,
    ) {
        let scale_factor = Config::with(|c| c.screen_scale);
        // Which popup, if any, the commit belongs to: subsurfaces of a popup
        // have that popup's surface as their root ancestor.
        let committed_root: Option<ObjectId> = committed.map(|s| {
            let mut root = s.clone();
            while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
                root = parent;
            }
            root.id()
        });
        if let Some(window_surface) = window.wl_surface() {
            let id = window_surface.id();

            // The shadow's `active` look must track real keyboard focus. Rebuilding
            // it as inactive unconditionally makes the shadow flick for one frame
            // whenever this runs on a still-focused window (e.g. on scanout demote,
            // where the window is demoted but keeps focus).
            //
            // Read through `focused_window_surface` so that a popup counts as
            // focus on the window that opened it: a menu takes the keyboard
            // for as long as it is up, and matching the focus target directly
            // would gray out the titlebar and lighten the shadow of the very
            // window being used.
            let is_focused = self
                .seat
                .get_keyboard()
                .and_then(|k| k.current_focus())
                .and_then(|focus| crate::state::seat_handler::focused_window_surface(Some(&focus)))
                .is_some_and(|surface| window.wl_surface().as_deref() == Some(&surface));

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
            let fullscreen = window.is_fullscreen();
            // Otto owns the titlebar, so the client's surface tree starts one
            // bar below the window's frame origin — `content_layer` is offset
            // by exactly this. A popup's offset is measured from the client's
            // window geometry, so it has to be lifted by the same amount or it
            // is painted a bar too high, while the hit test (which lifts the
            // point through `WindowElement::surface_under`) still resolves it
            // where it belongs. Snapped from the same value as the bar itself
            // so the two stay on the same pixel at a fractional scale.
            let decoration_offset_px = crate::workspaces::utils::snap_extent_px(
                0.0,
                window.decoration_height() as f32 * scale_factor as f32,
            );

            let mut render_elements = VecDeque::new();

            // Collect popup surfaces and send them to the popup overlay layer.
            // Two passes: `popups_for_surface` yields children BEFORE their
            // parent, so gather every popup's accumulated offset first, then
            // decide per popup whether its own position is resolved yet.
            let popups: Vec<(
                smithay::desktop::PopupKind,
                smithay::utils::Point<i32, smithay::utils::Logical>,
            )> = PopupManager::popups_for_surface(&window_surface).collect();
            #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
            let popup_offsets: std::collections::HashMap<
                smithay::reexports::wayland_server::backend::ObjectId,
                smithay::utils::Point<i32, smithay::utils::Logical>,
            > = popups
                .iter()
                .map(|(p, o)| (p.wl_surface().id(), *o))
                .collect();
            for (popup, popup_offset) in &popups {
                let popup_offset = *popup_offset;
                let popup_surface = popup.wl_surface();
                let popup_id = popup_surface.id();

                // A nested popup whose accumulated offset equals its parent
                // popup's offset has an unresolved (0,0) own position: its
                // committed geometry hasn't landed yet (the initial configure
                // round-trip; `PopupKind::location()` reads committed state).
                // Drawing it now places it on top of its parent — skip it; a
                // later frame re-runs this with the real offset once its
                // geometry commits.
                let parent_surface = match popup {
                    smithay::desktop::PopupKind::Xdg(s) => s.get_parent_surface(),
                    _ => None,
                };
                let degenerate_nested = parent_surface
                    .and_then(|p| popup_offsets.get(&p.id()).copied())
                    .is_some_and(|parent_off| parent_off == popup_offset);
                if degenerate_nested {
                    continue;
                }

                // Smithay resolves a click on a popup against
                // `window_geometry_origin + popup_location - popup.geometry().loc`
                // (`Window::surface_under`), and `element_location` is that
                // geometry origin. The `- popup.geometry().loc` half is
                // already applied to the popup's own surface layers by
                // `window_view_for_surface` (`phy_dst` subtracts each
                // surface's geometry origin), so the layer belongs at the
                // popup's *geometry* origin: subtracting the shadow margin
                // here too would slide a menu that pads its buffer with a
                // drop shadow — a GTK one — up and left of where it responds.
                let offset: smithay::utils::Point<f64, smithay::utils::Physical> =
                    popup_offset.to_physical_precise_round(scale_factor);

                // Calculate absolute popup position (window position + popup offset)
                let popup_position = layers::types::Point {
                    x: location.x as f32 + offset.x as f32,
                    y: location.y as f32 + decoration_offset_px + offset.y as f32,
                };

                // Nothing to do unless this popup itself committed or moved —
                // see `update_window_view_for_commit`.
                let popup_committed =
                    committed_root.is_none() || committed_root.as_ref() == Some(&popup_id);
                if !self.workspaces.popup_overlay.needs_sync(
                    &popup_id,
                    popup_position,
                    popup_committed,
                ) {
                    continue;
                }

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
                    } else {
                        tracing::debug!(
                            target: "otto::popups",
                            "popup surface {:?} produced no view (no buffer/texture?)",
                            surface.id()
                        );
                    }
                });

                tracing::debug!(
                    target: "otto::popups",
                    "update_popup {:?} root={:?} pos=({}, {}) surfaces={}",
                    popup_id,
                    id,
                    popup_position.x,
                    popup_position.y,
                    popup_surfaces.len()
                );

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
            }

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
            #[allow(clippy::mutable_key_type)]
            let mut children_order: std::collections::HashMap<ObjectId, Vec<ObjectId>> =
                std::collections::HashMap::new();

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
                        if let Some(parent_id) = parent_id.clone() {
                            // `with_surface_tree_downward` walks the tree from
                            // the screen down — nearest sibling first — and it
                            // honours place_above / place_below. Reversing it
                            // gives the order layers have to be appended in,
                            // since `append_layer` puts a layer last and last
                            // is topmost.
                            children_order
                                .entry(parent_id)
                                .or_default()
                                .insert(0, surface.id());
                        }
                        surface_info.insert(
                            surface.id(),
                            (surface.clone(), *location, parent_id.clone()),
                        );
                    } else {
                        // Surface committed a null buffer (unmapped subsurface) — hide its layer
                        if let Some(layer) = self.surface_layers.get(&surface.id()) {
                            layer.set_hidden(true);
                        }
                    }
                },
                |_, _, _| true,
            );

            // Now sync the layer hierarchy to match the surface tree
            for (surface_id, (surface, _pos, parent_id)) in surface_info.iter() {
                let layer = self.get_or_create_layer_for_surface(surface);

                // Configure layer with all properties and draw callback
                if let Some(wvs) = render_elements.iter().find(|e| &e.id == surface_id) {
                    layer.set_hidden(false);
                    let style = self.surfaces_style.get(surface_id).and_then(|v| v.first());
                    let gravity = style.map(|s| s.contents_gravity).unwrap_or_default();
                    let client_owns_size = style.map(|s| s.client_owns_size).unwrap_or(false);
                    let shared_gravity = style.map(|s| s.shared_gravity.clone());
                    crate::workspaces::utils::configure_surface_layer(
                        &layer,
                        wvs,
                        gravity,
                        client_owns_size,
                        shared_gravity,
                    );

                    // Set up parent-child relationship using layers_engine.
                    // Only re-parent if the parent changed — re-appending on
                    // every commit detaches and re-attaches the node, which
                    // causes flicker.
                    if let Some(parent_id) = parent_id {
                        let needs_reparent =
                            self.surface_layer_parents.get(surface_id) != Some(parent_id);
                        if needs_reparent {
                            if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                                let _ = self.layers_engine.append_layer(&layer, parent_layer.id());
                                self.surface_layer_parents
                                    .insert(surface_id.clone(), parent_id.clone());
                            }
                        }
                    }
                }
            }

            // Sibling order, when it has changed. `place_above` reorders
            // Smithay's tree but nothing in the scene, because a layer is only
            // appended when its *parent* changes — so without this a Quick View
            // panel raised above a column stays underneath it.
            for (parent_id, child_ids) in children_order.iter() {
                if self.surface_children_order.get(parent_id) == Some(child_ids) {
                    continue;
                }
                let Some(parent_layer) = self.surface_layers.get(parent_id).cloned() else {
                    continue;
                };
                let parent_node = parent_layer.id();
                for child_id in child_ids {
                    if let Some(child_layer) = self.surface_layers.get(child_id) {
                        let _ = self.layers_engine.append_layer(child_layer, parent_node);
                    }
                }
                self.surface_children_order
                    .insert(parent_id.clone(), child_ids.clone());
            }

            if let Some(window_view) = self.workspaces.get_window_view(&id) {
                // Snap the window box onto the physical pixel grid: both
                // terms are logical integers multiplied by the output scale,
                // so on a fractional scale every edge lands mid-pixel and the
                // shadow and decoration drawn from this model are resampled.
                let snapped = crate::workspaces::utils::snap_position_px(location.x, location.y);
                let model = WindowViewBaseModel {
                    x: snapped.x,
                    y: snapped.y,
                    w: crate::workspaces::utils::snap_extent_px(
                        location.x as f32,
                        window_geometry.size.w as f32,
                    ),
                    h: crate::workspaces::utils::snap_extent_px(
                        location.y as f32,
                        window_geometry.size.h as f32,
                    ),
                    title,
                    fullscreen,
                    active: is_focused,
                };
                window_view.view_base.update_state(&model);

                // Server-side decoration: the titlebar occupies the top strip
                // of the window and the client's surfaces start below it.
                let decoration_height = window.decoration_height();
                window_view.set_decorated(window.is_decorated());
                // A fullscreen window covers the output: no bar, no shadow.
                window_view.set_shadow_hidden(fullscreen);
                if window.is_decorated() {
                    let model = self.decoration_model_for(
                        window,
                        &window_view,
                        window_geometry.size.w as f32,
                        is_focused,
                        fullscreen,
                        scale_factor,
                    );
                    window_view.update_decoration(model);
                }

                // Directly add root surface layer to content layer without using LayerTreeBuilder
                let content_layer = &window_view.content_layer;
                content_layer.set_position(
                    layers::prelude::Point {
                        x: 0.0,
                        // Rounded to match the snapped bar height in
                        // `view_window_decoration`, so the client's subtree
                        // starts on a whole pixel instead of on 34 x 1.75.
                        y: crate::workspaces::utils::snap_extent_px(
                            0.0,
                            decoration_height as f32 * scale_factor as f32,
                        ),
                    },
                    None,
                );

                if let Some(root_layer) = self.surface_layers.get(&id) {
                    // Use layers_engine to set parent-child relationship
                    let _ = self
                        .layers_engine
                        .append_layer(root_layer, content_layer.id());
                }

                // Keep the expose preview live. The preview mirror is a
                // *follower* of the window's base layer, and lay-rs only
                // propagates NEEDS_PAINT from the leader node itself — this
                // commit repaints a surface layer deeper in the tree, which
                // never reaches the mirror. Report the new content on the
                // leader so the follower repaints; otherwise the previews
                // freeze on whatever was on screen when expose opened (a
                // playing video looks stuck). Only while expose is up: the
                // mirrors aren't rendered otherwise, and the real window layer
                // repaints through the normal path.
                if self.workspaces.get_show_all() || self.workspaces.is_expose_transitioning() {
                    window.base_layer().add_damage(layers::skia::Rect::from_wh(
                        window_geometry.size.w as f32,
                        window_geometry.size.h as f32,
                    ));
                    // Same propagation, one level up: the workspace-selector
                    // thumbnail follows the workspace's windows CONTAINER, so
                    // the container has to be told too or the thumb of a
                    // non-current workspace stays frozen while its windows
                    // keep committing (see `damage_workspace_thumbnail`).
                    self.workspaces.damage_workspace_thumbnail(&window.id());
                }

                self.workspaces.expose_update_if_needed();
            }
        }
    }

    /// Build the titlebar model for `window` at a given decorated width.
    ///
    /// `width_px` is the window's width in physical pixels; the bar itself is
    /// described in logical points. Shared by the full commit import and the
    /// scanout refresh below, so the bar a promoted window wears is built the
    /// same way as everyone else's.
    fn decoration_model_for(
        &self,
        window: &WindowElement,
        window_view: &crate::workspaces::WindowView,
        width_px: f32,
        is_focused: bool,
        fullscreen: bool,
        scale_factor: f64,
    ) -> crate::workspaces::WindowDecorationModel {
        crate::workspaces::WindowDecorationModel {
            width: width_px / scale_factor as f32,
            height: window.decoration_height() as f32,
            title: window.xdg_title(),
            active: is_focused,
            dark: Config::with(|c| matches!(c.theme_scheme, crate::theme::ThemeScheme::Dark)),
            // Maximized and fullscreen windows sit flush against the screen
            // edges, so their frame — and with it the bar — squares off.
            corner_radius: if window.is_maximized() || fullscreen {
                0.0
            } else {
                otto_kit::corners::radius(12.0)
            },
            controls_hovered: window_view.decoration_state().controls_hovered,
            pressed: window_view.decoration_state().pressed,
            sharing: crate::screenshare::is_window_screencast(
                &self.screenshare_sessions,
                &self.workspaces,
                &self.foreign_toplevels,
                &window.id(),
            ),
            fixed_size: !window.is_resizable(),
            scale: scale_factor as f32,
        }
    }

    /// Rebuild every server-side titlebar from the current configuration.
    ///
    /// The colour scheme, corner rounding, the side the controls sit at and
    /// whether the zoom dot is drawn are all read while a bar is built, so a
    /// change to any of them has to reach the bars that are already up. The
    /// model is recomputed rather than nudged, because two of those four live
    /// in it (`dark`, `corner_radius`) and two do not — and a model that hashed
    /// the same would skip the repaint the other two need.
    pub fn refresh_window_decorations(&mut self) {
        let scale_factor = Config::with(|c| c.screen_scale);
        let windows: Vec<_> = self.workspaces.spaces_elements().cloned().collect();
        for window in windows {
            if !window.is_decorated() {
                continue;
            }
            let Some(id) = window.wl_surface().map(|s| s.id()) else {
                continue;
            };
            let Some(window_view) = self.workspaces.get_window_view(&id) else {
                continue;
            };
            let base = window_view.view_base.get_state();
            let width_px = self
                .workspaces
                .element_geometry(&window)
                .unwrap_or_default()
                .to_f64()
                .to_physical(scale_factor)
                .size
                .w as f32;
            let model = self.decoration_model_for(
                &window,
                &window_view,
                width_px,
                base.active,
                base.fullscreen,
                scale_factor,
            );
            window_view.update_decoration(model);
            // The controls' side and the zoom dot are read inside the render
            // function rather than carried in the model, so a bar whose model
            // did not move still has to be redrawn.
            window_view.rerender_decoration();
        }
    }

    /// Refresh the chrome Otto draws around a window — its drop shadow and its
    /// server-side titlebar — from the window's geometry, without re-importing
    /// its surface tree.
    ///
    /// Used for scanned-out windows: their client buffer is pushed straight to
    /// a KMS plane, so the full `update_window_view` import is skipped on every
    /// root commit (see `shell::mod` commit handler). But the shadow and the
    /// titlebar still render in the windows plane, so a geometry change while
    /// promoted (tile, maximize, resize) must be reflected here or the shadow
    /// ghosts at the pre-change size and the bar keeps the width it had before
    /// — a tiled window maximized on a plane wearing a half-screen titlebar.
    pub fn refresh_window_chrome_geometry(&mut self, window: &WindowElement) {
        let scale_factor = Config::with(|c| c.screen_scale);
        let Some(id) = window.wl_surface().map(|s| s.id()) else {
            return;
        };
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
        if let Some(window_view) = self.workspaces.get_window_view(&id) {
            let current = window_view.view_base.get_state();
            // Snapped for the same reason as the commit path above; the
            // early-out below then compares snapped values, so a sub-pixel
            // jitter that rounds to the same box costs nothing.
            let snapped = crate::workspaces::utils::snap_position_px(location.x, location.y);
            let (x, y) = (snapped.x, snapped.y);
            let (w, h) = (
                crate::workspaces::utils::snap_extent_px(
                    location.x as f32,
                    window_geometry.size.w as f32,
                ),
                crate::workspaces::utils::snap_extent_px(
                    location.y as f32,
                    window_geometry.size.h as f32,
                ),
            );
            // Skip the state churn when nothing moved — most scanout commits
            // are same-geometry buffer swaps (video, games) firing at display
            // rate.
            if current.x == x && current.y == y && current.w == w && current.h == h {
                return;
            }
            let model = WindowViewBaseModel {
                x,
                y,
                w,
                h,
                title: current.title.clone(),
                fullscreen: current.fullscreen,
                active: current.active,
            };
            window_view.view_base.update_state(&model);

            // The bar is sized from the same geometry, and it is drawn in the
            // scene even while the client's content scans out.
            window_view.set_decorated(window.is_decorated());
            window_view.set_shadow_hidden(current.fullscreen);
            if window.is_decorated() {
                let decoration = self.decoration_model_for(
                    window,
                    &window_view,
                    window_geometry.size.w as f32,
                    current.active,
                    current.fullscreen,
                    scale_factor,
                );
                window_view.update_decoration(decoration);
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

    pub fn send_foreign_toplevel_state(&self, wid: &ObjectId, activated: bool) {
        if let Some(handles) = self.foreign_toplevels.get(wid) {
            if let Some(window) = self.workspaces.get_window_for_surface(wid) {
                let minimized = window.is_minimised();
                let maximized = window
                    .toplevel()
                    .map(|t| {
                        t.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized))
                    })
                    .unwrap_or(false);
                let fullscreen = window.is_fullscreen();
                handles.send_state(activated, minimized, maximized, fullscreen);
            }
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

    /// Dismiss all active popups and release any pointer/keyboard grabs.
    ///
    /// Must NOT be called from within a keyboard grab callback (e.g. `KeyboardTarget::leave`)
    /// or while smithay's input_method mutex is held, as it calls `keyboard.is_grabbed()` /
    /// `pointer.is_grabbed()` which may re-acquire those mutexes and deadlock.
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

        self.restore_pointer_focus();
    }

    /// Restore pointer focus to the surface currently under the cursor.
    ///
    /// Unlike `dismiss_all_popups`, this does not touch the keyboard handle and is safe to call
    /// from within keyboard grab callbacks where the keyboard mutex is already held.
    pub fn restore_pointer_focus(&mut self) {
        let pointer = self.pointer.clone();
        let pointer_location = pointer.current_location();
        let under = self.surface_under(pointer_location);
        let serial = SERIAL_COUNTER.next_serial();
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location: pointer_location,
                serial,
                time: 0,
            },
        );
        pointer.frame(self);
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
#[allow(clippy::mutable_key_type)] // ObjectId as HashMap key — see window_throttle.rs
pub fn post_repaint<'a>(
    output: &Output,
    render_element_states: &RenderElementStates,
    window_elements: &[&WindowElement],
    dmabuf_feedback: Option<SurfaceDmabufFeedback<'_>>,
    time: impl Into<Duration>,
    window_throttle_states: &std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        window_throttle::WindowThrottleState,
    >,
    occluded_layer_ids: &std::collections::HashSet<
        smithay::reexports::wayland_server::backend::ObjectId,
    >,
) {
    let time = time.into();
    let default_throttle = Duration::ZERO;

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

        // Per-window throttle based on user-visibility classification. Missing
        // entries (should be rare) fall through to full-rate, matching the
        // previous behaviour.
        let throttle = window_throttle_states
            .get(&window.id())
            .map(|s| s.throttle())
            .unwrap_or(default_throttle);
        window.send_frame(output, time, Some(throttle), |_, _| Some(output.clone()));
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

        // Background/bottom surfaces hidden behind a window get the same 2 Hz
        // trickle as an occluded window (see `occluded_layer_surface_ids`);
        // everything else paints at full rate.
        let layer_throttle = if occluded_layer_ids.contains(&layer_surface.wl_surface().id()) {
            window_throttle::WindowThrottleState::Occluded.throttle()
        } else {
            Duration::ZERO
        };
        layer_surface.send_frame(
            output,
            time,
            Some(layer_throttle),
            surface_primary_scanout_output,
        );
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
    /// Push the keyboard LED state (Caps/Num/Scroll Lock) to the hardware.
    /// Only a backend that owns the input devices has anything to do; under
    /// a host compositor the host owns the LEDs.
    fn update_keyboard_leds(&mut self, _led_state: smithay::input::keyboard::LedState) {}
    /// Invalidate any pre-computed (vblank-prefetched) scene update. Called
    /// on client commits: a commit can create scene layers (e.g. a popup)
    /// AFTER the prefetch ran, and drawing from the stale prefetch renders
    /// them without a layout pass — visibly flashing at (0,0) for a frame.
    fn invalidate_scene_prefetch(&mut self) {}
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

    /// Re-apply the current `input.*` configuration to every connected input
    /// device.
    ///
    /// Only the udev backend owns libinput devices; the windowed backends get
    /// cooked events from their host compositor, which applies its own device
    /// configuration, so there is nothing here for them to do.
    fn reconfigure_input_devices(&mut self) {
        tracing::debug!(
            backend = self.backend_name(),
            "Input settings stored; this backend has no libinput devices to reconfigure"
        );
    }
}
