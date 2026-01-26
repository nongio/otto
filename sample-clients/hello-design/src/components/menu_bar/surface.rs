use smithay_client_toolkit::{
    compositor::CompositorState,
    reexports::client::{protocol::wl_surface, QueueHandle},
};
use wayland_client::{protocol::wl_subcompositor, Dispatch};

use crate::components::menu::{sc_layer_shell_v1, sc_layer_v1};
use crate::surfaces::{SubsurfaceSurface, Surface};

use super::MenuBar;

/// Surface manager for MenuBar component
///
/// This component uses SubsurfaceSurface internally to manage
/// the Wayland subsurface and Skia rendering.
pub struct MenuBarSurface {
    /// The subsurface wrapper that handles rendering
    subsurface: SubsurfaceSurface,
    /// The menu bar component
    menu_bar: MenuBar,
}

impl MenuBarSurface {
    /// Create a new menu bar surface as a subsurface of the parent
    pub fn new<D>(
        parent_surface: &wl_surface::WlSurface,
        menu_bar: MenuBar,
        width: i32,
        compositor: &CompositorState,
        subcompositor: &wl_subcompositor::WlSubcompositor,
        qh: &QueueHandle<D>,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        D: Dispatch<wl_surface::WlSurface, smithay_client_toolkit::compositor::SurfaceData>
            + Dispatch<wayland_client::protocol::wl_subsurface::WlSubsurface, ()>
            + Dispatch<sc_layer_v1::ScLayerV1, ()>
            + Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
            + 'static,
    {
        let height = menu_bar.height() as i32;

        // Create the subsurface using SubsurfaceSurface component
        let subsurface = SubsurfaceSurface::new(
            parent_surface,
            0, // x position (top-left)
            0, // y position (top-left)
            width,
            height,
            compositor,
            subcompositor,
            qh,
        )?;

        let mut menu_bar_surface = Self {
            subsurface,
            menu_bar,
        };

        // Initial render
        menu_bar_surface.render();

        Ok(menu_bar_surface)
    }

    /// Render the menu bar
    pub fn render(&mut self) {
        let width = self.subsurface.dimensions().0;
        self.subsurface.draw(|canvas| {
            self.menu_bar.render(canvas, width as f32);
        });
    }

    /// Handle a click at the given position (in surface-local coordinates)
    /// Returns (label, x_position) of the menu that was toggled, if any
    pub fn handle_click(&mut self, x: f32, y: f32) -> Option<(String, f32)> {
        let result = self.menu_bar.handle_click(x, y);
        if result.is_some() {
            self.render();
        }
        result
    }

    /// Handle hover at the given position (in surface-local coordinates)
    /// Returns (label, x_position, changed) if hovering over a menubar item
    /// changed is true if the active menu was switched
    pub fn handle_hover(&mut self, x: f32, y: f32) -> Option<(String, f32, bool)> {
        if let Some((label, x_pos, changed)) = self.menu_bar.handle_hover(x, y) {
            if changed {
                self.render();
            }
            Some((label, x_pos, changed))
        } else {
            None
        }
    }

    /// Get the menu bar component
    pub fn menu_bar(&self) -> &MenuBar {
        &self.menu_bar
    }

    /// Get mutable menu bar component
    pub fn menu_bar_mut(&mut self) -> &mut MenuBar {
        &mut self.menu_bar
    }

    /// Get the surface
    pub fn surface(&self) -> &wl_surface::WlSurface {
        self.subsurface.wl_surface()
    }

    /// Resize the surface
    pub fn resize(&mut self, width: i32) {
        let (current_width, _) = self.subsurface.dimensions();
        if current_width != width {
            // SubsurfaceSurface doesn't have a public resize method yet,
            // but for now the menu bar will just re-render with the current dimensions
            // TODO: Add resize support to SubsurfaceSurface
            self.render();
        }
    }
}
