//! Capturing one workspace to a PNG, whether it is on screen or not.
//!
//! A workspace that is not shown is not hidden in the scene: it sits beside
//! the shown one on the strip, its windows keep their buffers, and their
//! applications keep drawing (see `crate::state::agent_seats` for why an
//! agent's hidden workspace draws at full rate). So a capture draws the
//! workspace's own layers — the wallpaper, then the windows — off screen,
//! at its output's size, with the renderer's GPU context when there is one.
//!
//! Left out: a window promoted to its own plane or scanned out directly on
//! the shown workspace, whose content is not in the workspace's layers at
//! that moment, and the layer-shell chrome (bars, dock), which belongs to
//! the output rather than to the workspace.

use std::path::PathBuf;

use layers::{drawing::render_node_tree, skia};

use super::{Backend, Otto};

/// A workspace picked for capture: where it is and what it is called.
struct Target {
    output: String,
    view: std::sync::Arc<crate::workspaces::workspace::WorkspaceView>,
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Capture the workspace `selector` names — its id as `GetWorkspaces`
    /// lists it, or its name — to a PNG under
    /// `$XDG_RUNTIME_DIR/otto/captures`, and return the file's path.
    pub fn capture_workspace(&mut self, selector: &str) -> Result<PathBuf, String> {
        let target = self.find_workspace(selector)?;
        let output = self
            .workspaces
            .outputs()
            .find(|output| output.name() == target.output)
            .cloned()
            .ok_or("the workspace's output is gone")?;
        let mode = output.current_mode().ok_or("the output has no mode")?;
        let (width, height) = (mode.size.w, mode.size.h);

        let info = skia::ImageInfo::new(
            (width, height),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        let mut context = self.backend_data.renderer_context();
        // Client windows are textures on the renderer's context; without one
        // (the headless backend) only what Skia can draw on the CPU shows.
        let mut surface = match context.as_mut() {
            Some(context) => skia::gpu::surfaces::render_target(
                context,
                skia::gpu::Budgeted::No,
                &info,
                None,
                skia::gpu::SurfaceOrigin::TopLeft,
                None,
                false,
                false,
            ),
            None => skia::surfaces::raster(&info, None, None),
        }
        .ok_or("could not make a surface to draw into")?;

        let canvas = surface.canvas();
        canvas.clear(skia::Color::BLACK);
        let scene = self.layers_engine.scene();
        scene.with_arena(|arena| {
            scene.with_renderable_arena(|renderables| {
                // Each root lands at the canvas origin: its own position on
                // the strip is not applied.
                for root in [target.view.wallpaper_group.id, target.view.windows_layer.id] {
                    render_node_tree(root, arena, renderables, canvas, 1.0, None, None, None);
                }
            });
        });
        if let Some(context) = context.as_mut() {
            context.flush_and_submit();
        }

        let image = surface.image_snapshot();
        let png = image
            .encode(context.as_mut(), skia::EncodedImageFormat::PNG, None)
            .ok_or("could not encode the capture")?;

        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("otto/captures");
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let path = dir.join(format!("workspace-{}-{millis}.png", target.view.index));
        std::fs::write(&path, png.as_bytes()).map_err(|err| err.to_string())?;
        tracing::info!(
            workspace = target.view.display_name(),
            output = target.output,
            path = %path.display(),
            "Workspace captured"
        );
        Ok(path)
    }

    /// The workspace `selector` names: an id from `GetWorkspaces`, or a name
    /// (any case). A name more than one output has is refused rather than
    /// guessed.
    fn find_workspace(&self, selector: &str) -> Result<Target, String> {
        let selector = selector.trim();
        let id: Option<u64> = selector.parse().ok();
        let mut found: Vec<Target> = Vec::new();
        for (output, ows) in &self.workspaces.output_workspaces {
            for (position, view) in ows.workspace_views.iter().enumerate() {
                let by_id = id == Some(crate::shell::commands::workspace_id(output, position));
                let by_name = view.display_name().eq_ignore_ascii_case(selector);
                if by_id || by_name {
                    found.push(Target {
                        output: output.clone(),
                        view: view.clone(),
                    });
                }
            }
        }
        match found.len() {
            0 => Err(format!("no workspace {selector:?}")),
            1 => Ok(found.remove(0)),
            _ => Err(format!(
                "{selector:?} names a workspace on {} outputs; use its id",
                found.len()
            )),
        }
    }
}
