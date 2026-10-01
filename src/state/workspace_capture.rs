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

use std::{os::unix::fs::DirBuilderExt, path::PathBuf};

use layers::{drawing::render_node_tree, skia};

use super::{agent_seats::Reach, Backend, Otto};

/// A workspace picked for capture: where it is and what it is called.
pub(crate) struct Target {
    pub output: String,
    pub view: std::sync::Arc<crate::workspaces::workspace::WorkspaceView>,
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Capture the workspace `selector` names — its id as `GetWorkspaces`
    /// lists it, or its name, or nothing for the caller's own — to a PNG under
    /// `$XDG_RUNTIME_DIR/otto/captures`, and return the file's path.
    ///
    /// Only for `owner`'s agent, and only of the workspace it is granted;
    /// never while the session is locked (`specs/permissions.md`).
    pub fn capture_workspace(&mut self, owner: &str, selector: &str) -> Result<PathBuf, String> {
        if self.is_session_locked() {
            return Err("the session is locked".into());
        }
        let target = if selector.trim().is_empty() {
            self.own_workspace(owner)?
        } else {
            self.find_workspace(selector)?
        };
        let granted = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .map(|agent| agent.reach(&self.workspaces));
        match granted {
            Some(Reach::Workspace { output, workspace })
                if output == target.output && workspace == target.view.index => {}
            _ => return Err(format!("{selector:?} is not the caller's own workspace")),
        }
        let output = self
            .workspaces
            .outputs()
            .find(|output| output.name() == target.output)
            .cloned()
            .ok_or("the workspace's output is gone")?;
        let mode = output.current_mode().ok_or("the output has no mode")?;
        let (width, height) = (mode.size.w, mode.size.h);

        let roots = [target.view.wallpaper_group.id, target.view.windows_layer.id];
        let mut surface = self.render_layers(&roots, width, height)?;
        let mut context = self.backend_data.renderer_context();

        let image = surface.image_snapshot();
        let png = image
            .encode(context.as_mut(), skia::EncodedImageFormat::PNG, None)
            .ok_or("could not encode the capture")?;

        // Never a shared directory such as /tmp: other users could read it.
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or("XDG_RUNTIME_DIR is not set")?
            .join("otto/captures");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|err| err.to_string())?;
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
    /// Draw the layer trees `roots` off screen, each at the canvas origin,
    /// at `width` by `height` pixels, with the renderer's GPU context when
    /// there is one. Client windows are textures on that context; without
    /// one (the headless backend) only what Skia can draw on the CPU shows.
    pub(crate) fn render_layers(
        &mut self,
        roots: &[layers::prelude::NodeRef],
        width: i32,
        height: i32,
    ) -> Result<skia::Surface, String> {
        let info = skia::ImageInfo::new(
            (width, height),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        let mut context = self.backend_data.renderer_context();
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
                for root in roots {
                    render_node_tree(*root, arena, renderables, canvas, 1.0, None, None, None);
                }
            });
        });
        if let Some(context) = context.as_mut() {
            context.flush_and_submit();
        }
        Ok(surface)
    }

    /// The workspace the agent seat `seat_name` is granted.
    pub(crate) fn workspace_of_seat(&self, seat_name: &str) -> Result<Target, String> {
        let reach = self
            .agent_seat(seat_name)
            .map(|agent| agent.reach(&self.workspaces));
        self.granted_target(reach)
    }

    /// The workspace `owner`'s agent is granted, for a capture that names
    /// none.
    fn own_workspace(&self, owner: &str) -> Result<Target, String> {
        let reach = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .map(|agent| agent.reach(&self.workspaces));
        self.granted_target(reach)
    }

    fn granted_target(&self, reach: Option<Reach>) -> Result<Target, String> {
        let Some(Reach::Workspace { output, workspace }) = reach else {
            return Err("the caller has no workspace of its own".into());
        };
        self.workspaces
            .output_workspaces
            .get(&output)
            .and_then(|ows| {
                ows.workspace_views
                    .iter()
                    .find(|view| view.index == workspace)
            })
            .map(|view| Target {
                output: output.clone(),
                view: view.clone(),
            })
            .ok_or_else(|| "the caller's workspace is gone".into())
    }

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
            count => Err(format!("{selector:?} names {count} workspaces; use its id")),
        }
    }
}
