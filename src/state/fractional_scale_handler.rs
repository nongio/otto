use smithay::{
    desktop::utils::surface_primary_scanout_output,
    reexports::wayland_server::Resource,
    wayland::{
        compositor::{get_parent, with_states},
        fractional_scale::{with_fractional_scale, FractionalScaleHandler},
    },
};

use super::{Backend, Otto};

impl<BackendData: Backend> FractionalScaleHandler for Otto<BackendData> {
    fn new_fractional_scale(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        // Here we can set the initial fractional scale
        //
        // First we look if the surface already has a primary scan-out output, if not
        // we test if the surface is a subsurface and try to use the primary scan-out output
        // of the root surface. If the root also has no primary scan-out output we just try
        // to use the first output of the toplevel.
        // If the surface is the root we also try to use the first output of the toplevel.
        //
        // If all the above tests do not lead to a output we just use the first output
        // of the space (which in case of anvil will also be the output a toplevel will
        // initially be placed on)
        #[allow(clippy::redundant_clone)]
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        with_states(&surface, |states| {
            let primary_scanout_output = surface_primary_scanout_output(&surface, states)
                .or_else(|| {
                    if root != surface {
                        with_states(&root, |states| {
                            surface_primary_scanout_output(&root, states).or_else(|| {
                                self.workspaces.get_window_for_surface(&root.id()).and_then(
                                    |window| {
                                        self.workspaces.outputs_for_element(window).first().cloned()
                                    },
                                )
                            })
                        })
                    } else {
                        self.workspaces
                            .get_window_for_surface(&root.id())
                            .and_then(|window| {
                                self.workspaces.outputs_for_element(window).first().cloned()
                            })
                    }
                })
                .or_else(|| self.workspaces.outputs().next().cloned());
            if let Some(output) = primary_scanout_output {
                with_fractional_scale(states, |fractional_scale| {
                    fractional_scale.set_preferred_scale(output.current_scale().fractional_scale());
                });
            }
        });
    }
}
