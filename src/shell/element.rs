use std::{
    borrow::Cow,
    fs,
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        Arc,
    },
    time::Duration,
};

use layers::prelude::Layer;
use smithay::{
    backend::renderer::{
        element::{
            solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement, AsRenderElements,
        },
        ImportAll, ImportMem, Renderer, RendererSuper, Texture,
    },
    desktop::{
        space::SpaceElement, utils::OutputPresentationFeedback, Window, WindowSurface,
        WindowSurfaceType,
    },
    output::Output,
    reexports::{
        wayland_protocols::{
            wp::presentation_time::server::wp_presentation_feedback,
            xdg::shell::server::xdg_toplevel,
        },
        wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface, Resource},
    },
    render_elements,
    utils::{user_data::UserDataMap, IsAlive, Logical, Physical, Point, Rectangle, Scale},
    wayland::{
        compositor::SurfaceData as WlSurfaceData,
        dmabuf::DmabufFeedback,
        seat::WaylandFocus,
        shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData},
    },
};
use wayland_server::DisplayHandle;

use crate::{focus::PointerFocusTarget, state::Backend};

#[derive(Debug, Clone)]
pub struct WindowElement(pub Arc<WindowElementInner>);

#[derive(Debug)]
pub struct WindowElementInner {
    window: Window,
    pub is_maximized: AtomicBool,
    pub is_minimized: AtomicBool,
    pub is_fullscreen: AtomicBool,
    pub app_id: String,
    pub base_layer: Layer,
    pub mirror_layer: Layer,
    pub workspace_index: AtomicUsize,
    pub fullscreen_workspace_index: AtomicUsize,
}

impl PartialEq for WindowElement {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl WindowElement {
    pub fn new(window: Window, base_layer: Layer, mirror_layer: Layer) -> Self {
        Self(Arc::new(WindowElementInner {
            window,
            is_maximized: AtomicBool::new(false),
            is_minimized: AtomicBool::new(false),
            is_fullscreen: AtomicBool::new(false),
            workspace_index: AtomicUsize::new(0),
            fullscreen_workspace_index: AtomicUsize::new(0),
            app_id: "".to_string(),
            base_layer,
            mirror_layer,
        }))
    }
    pub fn id(&self) -> ObjectId {
        self.0.window.wl_surface().unwrap().as_ref().id()
    }
    pub fn surface_under<B: Backend>(
        &self,
        location: Point<f64, Logical>,
        window_type: WindowSurfaceType,
    ) -> Option<(PointerFocusTarget<B>, Point<i32, Logical>)> {
        // let state = self.decoration_state();

        // let offset = if state.is_ssd {
        //     Point::from((0, HEADER_BAR_HEIGHT))
        // } else {
        //     Point::default()
        // };
        let offset = Point::default();

        let surface_under = self
            .0
            .window
            .surface_under(location - offset.to_f64(), window_type);
        let (under, loc) = match self.0.window.underlying_surface() {
            WindowSurface::Wayland(_) => {
                surface_under.map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc))
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(s) => {
                surface_under.map(|(_, loc)| (PointerFocusTarget::X11Surface(s.clone()), loc))
            }
        }?;
        Some((under, loc + offset))
    }

    pub fn with_surfaces<F>(&self, processor: F)
    where
        F: FnMut(&WlSurface, &WlSurfaceData),
    {
        self.0.window.with_surfaces(processor);
    }

    pub fn send_frame<T, F>(
        &self,
        output: &Output,
        time: T,
        throttle: Option<Duration>,
        primary_scan_out_output: F,
    ) where
        T: Into<Duration>,
        F: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
    {
        self.0
            .window
            .send_frame(output, time, throttle, primary_scan_out_output)
    }

    pub fn send_dmabuf_feedback<'a, P, F>(
        &self,
        output: &Output,
        primary_scan_out_output: P,
        select_dmabuf_feedback: F,
    ) where
        P: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F: Fn(&WlSurface, &WlSurfaceData) -> &'a DmabufFeedback + Copy,
    {
        self.0
            .window
            .send_dmabuf_feedback(output, primary_scan_out_output, select_dmabuf_feedback)
    }

    pub fn take_presentation_feedback<F1, F2>(
        &self,
        output_feedback: &mut OutputPresentationFeedback,
        primary_scan_out_output: F1,
        presentation_feedback_flags: F2,
    ) where
        F1: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F2: FnMut(&WlSurface, &WlSurfaceData) -> wp_presentation_feedback::Kind + Copy,
    {
        self.0.window.take_presentation_feedback(
            output_feedback,
            primary_scan_out_output,
            presentation_feedback_flags,
        )
    }

    #[cfg(feature = "xwayland")]
    #[inline]
    pub fn is_x11(&self) -> bool {
        self.window.is_x11()
    }

    #[inline]
    pub fn is_wayland(&self) -> bool {
        self.0.window.is_wayland()
    }

    #[inline]
    pub fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.0.window.wl_surface()
    }

    #[inline]
    pub fn toplevel(&self) -> Option<&ToplevelSurface> {
        self.0.window.toplevel()
    }

    #[inline]
    pub fn user_data(&self) -> &UserDataMap {
        self.0.window.user_data()
    }

    pub fn underlying_surface(&self) -> &WindowSurface {
        self.0.window.underlying_surface()
    }

    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        self.0.window.geometry()
    }

    pub fn bbox(&self) -> Rectangle<i32, Logical> {
        self.0.window.bbox()
    }

    pub fn on_commit(&self) {
        self.0.window.on_commit()
    }

    pub fn base_layer(&self) -> &Layer {
        &self.0.base_layer
    }

    pub fn layer(&self) -> &Layer {
        &self.0.base_layer
    }

    pub fn mirror_layer(&self) -> &Layer {
        &self.0.mirror_layer
    }

    pub fn xdg_app_id(&self) -> String {
        if self.is_wayland() {
            let surface = self.wl_surface().unwrap();
            smithay::wayland::compositor::with_states(&surface, |states| {
                let attributes: std::sync::MutexGuard<
                    '_,
                    smithay::wayland::shell::xdg::XdgToplevelSurfaceRoleAttributes,
                > = states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap();

                attributes.app_id.clone().unwrap_or_default()
            })
        } else {
            "".to_string()
        }
    }

    /// Get the app_id to display in dock/app switcher
    /// Uses PID resolution as fallback when XDG app_id is missing
    pub fn display_app_id(&self, display_handle: &DisplayHandle) -> String {
        let raw_app_id = self.xdg_app_id();

        // If we have a valid XDG app_id, use it
        if !raw_app_id.is_empty() {
            return raw_app_id;
        }

        // Only try PID resolution as fallback when app_id is missing
        if let Some(resolved_id) = self.resolve_app_id_from_pid(display_handle) {
            return resolved_id;
        }

        // Last resort: empty string
        String::new()
    }

    /// Resolve the actual app_id by examining the client's PID
    fn resolve_app_id_from_pid(&self, display_handle: &DisplayHandle) -> Option<String> {
        let surface = self.wl_surface()?;

        // Get the client from the surface
        let client = display_handle.get_client(surface.id()).ok()?;

        // Get client PID from credentials
        let credentials = client.get_credentials(display_handle).ok()?;
        let pid = credentials.pid;


        // Read /proc/PID/exe to get the executable path
        let exe_path = fs::read_link(format!("/proc/{}/exe", pid)).ok()?;
        let exe_name = exe_path.file_name()?.to_str()?.to_string();


        // Try to find matching desktop entry
        if let Some(desktop_id) = Self::find_desktop_entry_for_exe(&exe_name, &exe_path) {
            return Some(desktop_id);
        }

        // Fall back to executable name
        tracing::trace!(
            "[resolve_app_id_from_pid] No desktop entry found, using exe name: {}",
            exe_name
        );
        Some(exe_name)
    }

    /// Find a desktop entry matching the executable
    fn find_desktop_entry_for_exe(exe_name: &str, exe_path: &std::path::Path) -> Option<String> {
        use freedesktop_desktop_entry::{DesktopEntry, Iter};

        let exe_path_str = exe_path.to_str()?;

        // Iterate through all desktop entries
        for path in Iter::new(freedesktop_desktop_entry::default_paths()) {
            if let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) {
                // Check Exec field
                if let Some(exec) = entry.exec() {
                    let exec_parts: Vec<&str> = exec.split_whitespace().collect();
                    if let Some(exec_binary) = exec_parts.first() {
                        // Extract basename from exec
                        let exec_basename = std::path::Path::new(exec_binary)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(exec_binary);

                        // Match by executable name
                        if exec_basename == exe_name || exec_binary.contains(exe_name) {
                            let desktop_id = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .map(|s| s.to_string())?;
                            return Some(desktop_id);
                        }
                    }
                }

                // Check TryExec field
                if let Some(try_exec) = entry.try_exec() {
                    if try_exec.contains(exe_name) || try_exec == exe_path_str {
                        let desktop_id = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())?;
                        return Some(desktop_id);
                    }
                }
            }
        }

        None
    }

    pub fn is_minimised(&self) -> bool {
        self.0
            .is_minimized
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_is_minimised(&self, is_minimized: bool) {
        self.0
            .is_minimized
            .store(is_minimized, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_maximized(&self) -> bool {
        self.0
            .is_maximized
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_is_maximized(&self, is_maximized: bool) {
        self.0
            .is_maximized
            .store(is_maximized, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn set_workspace(&self, index: usize) {
        self.0
            .workspace_index
            .store(index, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn get_workspace(&self) -> usize {
        self.0
            .workspace_index
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get_fullscreen_workspace(&self) -> usize {
        self.0
            .fullscreen_workspace_index
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_fullscreen(&self, fullscreen: bool, workspace_index: usize) {
        self.0
            .is_fullscreen
            .store(fullscreen, std::sync::atomic::Ordering::Relaxed);
        self.0
            .fullscreen_workspace_index
            .store(workspace_index, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_fullscreen(&self) -> bool {
        self.0
            .is_fullscreen
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn xdg_is_fullscreen(&self) -> bool {
        self.toplevel()
            .map(|toplevel| {
                toplevel.with_committed_state(|current| {
                    current.is_some_and(|s| s.states.contains(xdg_toplevel::State::Fullscreen))
                })
            })
            .unwrap_or(false)
    }

    pub fn xdg_title(&self) -> String {
        self.wl_surface()
            .map(|window_surface| {
                smithay::wayland::compositor::with_states(&window_surface, |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .title
                        .clone()
                })
                .unwrap_or_default()
            })
            .unwrap_or_default()
    }
}

impl IsAlive for WindowElement {
    #[inline]
    fn alive(&self) -> bool {
        self.0.window.alive()
    }
}

impl SpaceElement for WindowElement {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        // if self.decoration_state().is_ssd {
        //     geo.size.h += HEADER_BAR_HEIGHT;
        // }
        SpaceElement::geometry(&self.0.window)
    }
    fn bbox(&self) -> Rectangle<i32, Logical> {
        // if self.decoration_state().is_ssd {
        //     bbox.size.h += HEADER_BAR_HEIGHT;
        // }
        SpaceElement::bbox(&self.0.window)
    }
    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        // if self.decoration_state().is_ssd {
        //     point.y < HEADER_BAR_HEIGHT as f64
        //         || SpaceElement::is_in_input_region(
        //             &self.window,
        //             &(*point - Point::from((0.0, HEADER_BAR_HEIGHT as f64))),
        //         )
        // } else {
        SpaceElement::is_in_input_region(&self.0.window, point)
        // }
    }
    fn z_index(&self) -> u8 {
        SpaceElement::z_index(&self.0.window)
    }

    fn set_activate(&self, activated: bool) {
        SpaceElement::set_activate(&self.0.window, activated);
    }
    fn output_enter(&self, output: &Output, overlap: Rectangle<i32, Logical>) {
        SpaceElement::output_enter(&self.0.window, output, overlap);
    }
    fn output_leave(&self, output: &Output) {
        SpaceElement::output_leave(&self.0.window, output);
    }
    #[profiling::function]
    fn refresh(&self) {
        SpaceElement::refresh(&self.0.window);
    }
}

render_elements!(
    pub WindowRenderElement<R> where R: ImportAll + ImportMem;
    Window=WaylandSurfaceRenderElement<R>,
    Decoration=SolidColorRenderElement,
);

impl<R: Renderer> std::fmt::Debug for WindowRenderElement<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(arg0) => f.debug_tuple("Window").field(arg0).finish(),
            Self::Decoration(arg0) => f.debug_tuple("Decoration").field(arg0).finish(),
            Self::_GenericCatcher(arg0) => f.debug_tuple("_GenericCatcher").field(arg0).finish(),
        }
    }
}

impl<R> AsRenderElements<R> for WindowElement
where
    R: Renderer + ImportAll + ImportMem,
    <R as RendererSuper>::TextureId: Clone + Texture + 'static,
{
    type RenderElement = WindowRenderElement<R>;

    fn render_elements<C: From<Self::RenderElement>>(
        &self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        let _window_bbox = SpaceElement::bbox(&self.0.window);

        // if self.decoration_state().is_ssd && !window_bbox.is_empty() {
        //     let window_geo = SpaceElement::geometry(&self.window);

        //     let mut state = self.decoration_state();
        //     let width = window_geo.size.w;
        //     state.header_bar.redraw(width as u32);
        //     let mut vec = AsRenderElements::<R>::render_elements::<WindowRenderElement<R>>(
        //         &state.header_bar,
        //         renderer,
        //         location,
        //         scale,
        //         alpha,
        //     );

        //     location.y += (scale.y * HEADER_BAR_HEIGHT as f64) as i32;

        //     let window_elements =
        //         AsRenderElements::render_elements(&self.window, renderer, location, scale, alpha);
        //     vec.extend(window_elements);
        //     vec.into_iter().map(C::from).collect()
        // } else {
        AsRenderElements::render_elements(&self.0.window, renderer, location, scale, alpha)
            .into_iter()
            .map(C::from)
            .collect()
        // }
    }
}
