use std::{
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::utils::RendererSurfaceState},
    delegate_dmabuf,
    input::pointer::CursorImageStatus,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        wayland_server::{protocol::wl_surface, Display},
    },
    wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
};
use tracing::info;

use crate::{
    skia_renderer::SkiaTextureImage,
    state::{Backend, Otto},
};

const OUTPUT_NAME: &str = "headless";
const DEFAULT_WIDTH: i32 = 1920;
const DEFAULT_HEIGHT: i32 = 1080;

pub struct HeadlessData {
    dmabuf_state: DmabufState,
    #[allow(dead_code)]
    dmabuf_global: DmabufGlobal,
}

impl Backend for HeadlessData {
    fn seat_name(&self) -> String {
        "headless".into()
    }
    fn backend_name(&self) -> &'static str {
        "headless"
    }
    fn reset_buffers(&mut self, _output: &Output) {}
    fn early_import(&mut self, _surface: &wl_surface::WlSurface) {}
    fn texture_for_surface(&self, _surface: &RendererSurfaceState) -> Option<SkiaTextureImage> {
        None
    }
    fn set_cursor(&mut self, _image: &CursorImageStatus) {}
    fn renderer_context(&mut self) -> Option<layers::skia::gpu::DirectContext> {
        None
    }
}

impl DmabufHandler for Otto<HeadlessData> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.backend_data.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        // No renderer available — always reject dmabuf imports
        notifier.failed();
    }
}
delegate_dmabuf!(Otto<HeadlessData>);

/// Configuration for the headless compositor instance.
pub struct HeadlessConfig {
    pub width: i32,
    pub height: i32,
}

impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

/// Handle to a running headless compositor instance.
///
/// The compositor runs on a background thread. Use this handle to get the
/// Wayland socket name, run queries against compositor state, and stop it.
pub struct HeadlessHandle {
    pub socket_name: String,
    compositor_thread: Option<JoinHandle<()>>,
    running: Arc<std::sync::atomic::AtomicBool>,
    query_tx: Sender<Box<dyn FnOnce(&mut Otto<HeadlessData>) + Send>>,
    result_rx: Receiver<()>,
}

impl HeadlessHandle {
    /// Start a headless compositor with the given configuration.
    ///
    /// Returns a handle once the compositor is ready to accept Wayland clients.
    pub fn start(config: HeadlessConfig) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel::<String>();
        let (query_tx, query_rx) =
            mpsc::channel::<Box<dyn FnOnce(&mut Otto<HeadlessData>) + Send>>();
        let (result_tx, result_rx) = mpsc::channel::<()>();

        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let running_clone = running.clone();

        let compositor_thread = thread::spawn(move || {
            run_headless_loop(config, ready_tx, query_rx, result_tx, running_clone);
        });

        let socket_name = ready_rx.recv().expect("Compositor thread failed to start");

        Self {
            socket_name,
            compositor_thread: Some(compositor_thread),
            running,
            query_tx,
            result_rx,
        }
    }

    /// Execute a closure on the compositor thread with access to the full
    /// `Otto<HeadlessData>` state. Blocks until the closure has run.
    pub fn with_state<F>(&self, f: F)
    where
        F: FnOnce(&mut Otto<HeadlessData>) + Send + 'static,
    {
        self.query_tx.send(Box::new(f)).ok();
        // Wait for the compositor loop to execute it
        let _ = self.result_rx.recv();
    }

    /// Stop the compositor and join the background thread.
    pub fn stop(mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(thread) = self.compositor_thread.take() {
            let _ = thread.join();
        }
    }

    /// Wait for the compositor to process events for the given duration.
    /// Useful after client operations to let the compositor catch up.
    pub fn wait(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    // ── Synthetic gesture input ──────────────────────────────────────────

    /// Simulate a 3-finger swipe begin gesture.
    pub fn swipe_begin(&self) {
        self.with_state(|state| {
            state.gesture_swipe_begin_3finger();
        });
    }

    /// Simulate a swipe gesture update with the given pixel deltas.
    pub fn swipe_update(&self, dx: f64, dy: f64) {
        self.with_state(move |state| {
            state.gesture_swipe_update(dx, dy);
        });
    }

    /// Simulate a swipe gesture ending.
    pub fn swipe_end(&self) {
        self.with_state(|state| {
            state.gesture_swipe_end(false);
        });
    }

    /// Simulate a complete swipe gesture with multiple update frames.
    /// Each element in `deltas` is a (dx, dy) frame.
    pub fn swipe(&self, deltas: &[(f64, f64)]) {
        let deltas = deltas.to_vec();
        self.with_state(move |state| {
            state.gesture_swipe_begin_3finger();
            for (dx, dy) in &deltas {
                state.gesture_swipe_update(*dx, *dy);
            }
            state.gesture_swipe_end(false);
        });
    }

    /// Simulate a 4-finger pinch begin gesture (show desktop).
    pub fn pinch_begin(&self) {
        self.with_state(|state| {
            state.gesture_pinch_begin_4finger();
        });
    }

    /// Simulate a pinch gesture update with the given scale.
    pub fn pinch_update(&self, scale: f64) {
        self.with_state(move |state| {
            state.gesture_pinch_update(scale);
        });
    }

    /// Simulate a pinch gesture ending.
    pub fn pinch_end(&self) {
        self.with_state(|state| {
            state.gesture_pinch_end();
        });
    }

    // ── State queries ────────────────────────────────────────────────────

    /// Query a value from compositor state. Returns the result synchronously.
    pub fn query<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Otto<HeadlessData>) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        self.with_state(move |state| {
            let result = f(state);
            let _ = tx.send(result);
        });
        rx.recv().expect("Failed to receive query result")
    }

    /// Get the current workspace index.
    pub fn current_workspace_index(&self) -> usize {
        self.query(|state| state.workspaces.get_current_workspace_index())
    }

    /// Check if expose mode (show all windows) is active.
    pub fn is_expose_active(&self) -> bool {
        self.query(|state| state.workspaces.get_show_all())
    }

    /// Check if expose mode is in the middle of a transition (animating).
    pub fn is_expose_transitioning(&self) -> bool {
        self.query(|state| state.workspaces.is_expose_transitioning())
    }

    /// Check if show-desktop mode is active.
    pub fn is_show_desktop_active(&self) -> bool {
        self.query(|state| state.workspaces.get_show_desktop())
    }

    /// Check if show-desktop is in the middle of a transition.
    pub fn is_show_desktop_transitioning(&self) -> bool {
        self.query(|state| state.workspaces.is_show_desktop_transitioning())
    }

    /// Get the current swipe gesture state name (for assertions).
    pub fn swipe_gesture_state(&self) -> String {
        self.query(|state| match &state.swipe_gesture {
            crate::state::SwipeGestureState::Idle => "idle".to_string(),
            crate::state::SwipeGestureState::Detecting { .. } => "detecting".to_string(),
            crate::state::SwipeGestureState::WorkspaceSwitching { .. } => {
                "workspace_switching".to_string()
            }
            crate::state::SwipeGestureState::Expose { .. } => "expose".to_string(),
        })
    }

    /// Get the number of windows across all workspaces.
    pub fn window_count(&self) -> usize {
        self.query(|state| state.workspaces.spaces_elements().count())
    }

    /// Programmatically switch to a workspace by index.
    pub fn set_workspace(&self, index: usize) {
        self.with_state(move |state| {
            state.workspaces.set_current_workspace_index(index, None);
        });
    }

    /// Toggle expose mode on/off.
    pub fn toggle_expose(&self) {
        self.with_state(|state| {
            let current = state.workspaces.get_show_all();
            state.workspaces.expose_set_visible(!current);
        });
    }

    /// Get a snapshot of the scene tree (all nodes with keys, bounds, visibility).
    pub fn scene_snapshot(&self) -> layers::engine::scene::SceneSnapshot {
        self.query(|state| state.layers_engine.scene().snapshot())
    }

    /// Serialize the scene tree to pretty JSON (for debugging / golden tests).
    pub fn scene_json(&self) -> String {
        self.query(|state| {
            state
                .layers_engine
                .scene()
                .serialize_state_pretty()
                .unwrap_or_default()
        })
    }

    /// Find a layer by its key and return whether it is hidden.
    /// Returns `None` if no layer with that key exists.
    pub fn is_layer_hidden(&self, key: &str) -> Option<bool> {
        let key = key.to_string();
        self.query(move |state| {
            state
                .layers_engine
                .find_layer_by_key(&key)
                .map(|l| l.hidden())
        })
    }

    /// Find a layer by its key and return its opacity from the scene snapshot.
    /// Returns `None` if no layer with that key exists.
    pub fn layer_opacity(&self, key: &str) -> Option<f32> {
        let key = key.to_string();
        self.query(move |state| {
            let snapshot = state.layers_engine.scene().snapshot();
            find_node_by_key(&snapshot.nodes, &key).map(|n| n.opacity)
        })
    }

    /// Check whether any layer is currently animating (scene has pending damage).
    pub fn scene_has_damage(&self) -> bool {
        self.query(|state| state.scene_element.update())
    }

    /// Advance the scene graph by one frame with the given delta time (seconds).
    /// Returns true if the frame produced damage (i.e. animations are still running).
    pub fn tick(&self, dt: f32) -> bool {
        self.query(move |state| state.layers_engine.update(dt))
    }

    /// Run the scene graph at 60fps until animations finish or `max_frames` is reached.
    ///
    /// Advances the engine timer by 16ms per frame (deterministic, no wall-clock sleep).
    /// Returns the number of frames that had damage.
    pub fn settle(&self, max_frames: usize) -> usize {
        const DT: f32 = 1.0 / 60.0;
        let mut frames_with_damage = 0;
        for _ in 0..max_frames {
            let has_damage = self.query(move |state| state.layers_engine.update(DT));
            if has_damage {
                frames_with_damage += 1;
            } else {
                break;
            }
        }
        frames_with_damage
    }

    // ── Synthetic pointer input ──────────────────────────────────────────

    /// Move the pointer to an absolute position (logical pixels).
    pub fn pointer_move(&self, x: f64, y: f64) {
        self.with_state(move |state| {
            state.synthetic_pointer_move(x, y);
        });
    }

    /// Simulate a left-button click (press + release) at the current pointer position.
    pub fn pointer_click(&self) {
        self.with_state(|state| {
            state.synthetic_pointer_button(true);
            state.synthetic_pointer_button(false);
        });
    }

    /// Return the title of the currently hovered window in expose, if any.
    pub fn expose_selected_title(&self) -> Option<String> {
        self.query(|state| {
            let ws_index = state.workspaces.get_current_workspace_index();
            let workspace = state.workspaces.get_workspace_at(ws_index)?;
            let selector_state = workspace.window_selector_view.view.get_state();
            let index = selector_state.current_selection?;
            Some(selector_state.rects.get(index)?.window_title.clone())
        })
    }

    /// Return the expose window rects for the current workspace: (title, x, y, w, h)
    /// in physical pixels (same coordinate space as the window selector).
    pub fn expose_window_rects(&self) -> Vec<(String, f32, f32, f32, f32)> {
        self.query(|state| {
            let ws_index = state.workspaces.get_current_workspace_index();
            let Some(workspace) = state.workspaces.get_workspace_at(ws_index) else {
                return Vec::new();
            };
            let selector_state = workspace.window_selector_view.view.get_state();
            selector_state
                .rects
                .iter()
                .map(|r| (r.window_title.clone(), r.x, r.y, r.w, r.h))
                .collect()
        })
    }
}

impl Otto<HeadlessData> {
    /// Move pointer to (x, y) in logical pixels, updating focus and layers engine.
    fn synthetic_pointer_move(&mut self, x: f64, y: f64) {
        use smithay::{
            input::pointer::MotionEvent,
            utils::{Logical, Point, SERIAL_COUNTER},
        };

        let pos: Point<f64, Logical> = (x, y).into();
        let serial = SERIAL_COUNTER.next_serial();

        let under = self.surface_under(pos);
        let pointer = self.pointer.clone();
        self.last_pointer_location = (pos.x, pos.y);

        {
            let focused = self.workspaces.output_under(pos).next().cloned();
            self.workspaces.set_focused_output(focused.as_ref());
        }

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time: 0,
            },
        );
        pointer.frame(self);

        // Also update the layers engine in physical pixels.
        let scale = self
            .workspaces
            .outputs()
            .next()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0);
        let phys = pos.to_physical(scale);
        self.cursor_physical_position = (phys.x, phys.y);
        self.layers_engine
            .pointer_move(&(phys.x as f32, phys.y as f32).into(), None);
    }

    /// Press or release the left pointer button.
    fn synthetic_pointer_button(&mut self, pressed: bool) {
        use smithay::{
            backend::input::ButtonState, input::pointer::ButtonEvent, utils::SERIAL_COUNTER,
        };

        let serial = SERIAL_COUNTER.next_serial();
        let button_state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };

        // BTN_LEFT = 0x110
        if !self.workspaces.get_show_all() && pressed {
            self.focus_window_under_cursor(serial);
        }
        let pointer = self.pointer.clone();
        pointer.button(
            self,
            &ButtonEvent {
                button: 0x110,
                state: button_state,
                serial,
                time: 0,
            },
        );
        pointer.frame(self);
        match button_state {
            ButtonState::Pressed => self.layers_engine.pointer_button_down(),
            ButtonState::Released => self.layers_engine.pointer_button_up(),
        }
    }
}

impl Drop for HeadlessHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(thread) = self.compositor_thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_headless_loop(
    config: HeadlessConfig,
    ready_tx: Sender<String>,
    query_rx: Receiver<Box<dyn FnOnce(&mut Otto<HeadlessData>) + Send>>,
    result_tx: Sender<()>,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    // A tokio runtime is needed because some compositor subsystems (e.g. dock)
    // spawn async tasks internally.
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let mut display_handle = display.handle();

    let mut dmabuf_state = DmabufState::new();
    let dmabuf_global = dmabuf_state.create_global::<Otto<HeadlessData>>(&display.handle(), vec![]);

    let data = HeadlessData {
        dmabuf_state,
        dmabuf_global,
    };

    let mut state = Otto::init(display, event_loop.handle(), data, true);
    state.running = running;

    // Create a virtual output
    let mode = Mode {
        size: (config.width, config.height).into(),
        refresh: 60_000,
    };
    let output = Output::new(
        OUTPUT_NAME.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Otto".into(),
            model: "Headless".into(),
            serial_number: "0".into(),
        },
    );
    let _global = output.create_global::<Otto<HeadlessData>>(&state.display_handle);

    let config_screen_scale = crate::config::Config::with(|c| c.screen_scale);
    output.change_current_state(
        Some(mode),
        Some(smithay::utils::Transform::Flipped180),
        Some(smithay::output::Scale::Fractional(config_screen_scale)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    // Set up scene dimensions and map output
    let root = state.scene_element.root_layer().unwrap();
    state
        .layers_engine
        .scene_set_size(config.width as f32, config.height as f32);
    root.set_size(
        layers::types::Size::points(config.width as f32, config.height as f32),
        None,
    );

    state
        .workspaces
        .set_screen_dimension(config.width, config.height);
    state.workspaces.map_output(&output, (0, 0));

    let socket_name = state
        .socket_name
        .clone()
        .expect("Headless compositor must have a socket");
    info!(name = %socket_name, "Headless compositor ready");

    // Signal readiness
    ready_tx.send(socket_name).unwrap();

    // Main dispatch loop — no rendering, just protocol dispatch and scene updates
    while state.running.load(Ordering::SeqCst) {
        // Process any pending state queries from the test thread
        while let Ok(query) = query_rx.try_recv() {
            query(&mut state);
            let _ = result_tx.send(());
        }

        // Update the scene graph (layout computation, no GPU)
        state.scene_element.update();

        // Dispatch Wayland clients and calloop sources
        let result = event_loop.dispatch(Some(Duration::from_millis(16)), &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
        } else {
            state.workspaces.refresh_space();
            state.popups.cleanup();
            display_handle.flush_clients().unwrap();
        }
    }

    info!("Headless compositor stopped");
}

/// Convenience entry point for running headless from the CLI (mainly for
/// manual testing). Blocks until the compositor is stopped.
pub fn run_headless() {
    let handle = HeadlessHandle::start(HeadlessConfig::default());
    info!(
        socket = %handle.socket_name,
        "Headless compositor running. Set WAYLAND_DISPLAY={} to connect clients.",
        handle.socket_name
    );

    // Block until interrupted
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if !handle.running.load(Ordering::SeqCst) {
            break;
        }
    }
}

/// Recursively search for a node by key in the scene snapshot tree.
fn find_node_by_key(
    nodes: &[layers::engine::scene::SceneNodeSnapshot],
    key: &str,
) -> Option<layers::engine::scene::SceneNodeSnapshot> {
    for node in nodes {
        if node.key == key {
            return Some(node.clone());
        }
        if let Some(found) = find_node_by_key(&node.children, key) {
            return Some(found);
        }
    }
    None
}
