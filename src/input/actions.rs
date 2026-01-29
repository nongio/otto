use std::{fs, process::Command, sync::atomic::Ordering};

use smithay::{
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1,
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};
use tracing::{error, info};

use crate::{
    config::{
        default_apps,
        shortcuts::{BuiltinAction, ShortcutAction},
        Config,
    },
    state::Backend,
    Otto,
};

/// Possible results of a keyboard action
#[allow(dead_code)]
#[derive(Debug)]
pub enum KeyAction {
    /// Quit the compositor
    Quit,
    /// Trigger a vt-switch
    VtSwitch(i32),
    /// run a command
    Run((String, Vec<String>)),
    /// Switch the current screen
    Screen(usize),
    ScaleUp,
    ScaleDown,
    RotateOutput,
    ToggleDecorations,
    ApplicationSwitchNext,
    ApplicationSwitchPrev,
    ApplicationSwitchQuit,
    ToggleMaximize,
    CloseWindow,
    ApplicationSwitchNextWindow,
    ExposeShowDesktop,
    ExposeShowAll,
    WorkspaceNum(usize),
    SceneSnapshot,
    /// Do nothing more
    None,
}

impl<BackendData: Backend> Otto<BackendData> {
    pub fn launch_program(&mut self, cmd: String, args: Vec<String>) {
        info!(program = %cmd, args = ?args, "Starting program");

        if let Err(e) = Command::new(&cmd)
            .args(&args)
            .envs(
                self.socket_name
                    .clone()
                    .map(|v| ("WAYLAND_DISPLAY", v))
                    .into_iter()
                    .chain(
                        #[cfg(feature = "xwayland")]
                        self.xdisplay.map(|v| ("DISPLAY", format!(":{}", v))),
                        #[cfg(not(feature = "xwayland"))]
                        None,
                    ),
            )
            .spawn()
        {
            error!(program = %cmd, err = %e, "Failed to start program");
        }
    }

    pub(crate) fn process_common_key_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::None => (),

            KeyAction::Quit => {
                info!("Quitting.");
                self.running.store(false, Ordering::SeqCst);
            }

            KeyAction::Run((cmd, args)) => {
                self.launch_program(cmd, args);
            }

            KeyAction::ToggleDecorations => {
                for element in self.workspaces.spaces_elements() {
                    #[allow(irrefutable_let_patterns)]
                    if let Some(toplevel) = element.toplevel() {
                        let mode_changed = toplevel.with_pending_state(|state| {
                            if let Some(current_mode) = state.decoration_mode {
                                let new_mode = if current_mode
                                    == zxdg_toplevel_decoration_v1::Mode::ClientSide
                                {
                                    zxdg_toplevel_decoration_v1::Mode::ServerSide
                                } else {
                                    zxdg_toplevel_decoration_v1::Mode::ClientSide
                                };
                                state.decoration_mode = Some(new_mode);
                                true
                            } else {
                                false
                            }
                        });
                        let initial_configure_sent = with_states(toplevel.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<XdgToplevelSurfaceData>()
                                .unwrap()
                                .lock()
                                .unwrap()
                                .initial_configure_sent
                        });
                        if mode_changed && initial_configure_sent {
                            toplevel.send_pending_configure();
                        }
                    }
                }
            }

            KeyAction::SceneSnapshot => {
                let scene = self.layers_engine.scene();

                match scene.serialize_state_pretty() {
                    Ok(json) => {
                        if let Err(err) = fs::write("scene.json", json) {
                            error!(?err, "Failed to write scene snapshot");
                        } else {
                            info!("Scene snapshot saved to scene.json");
                        }
                    }
                    Err(err) => error!(?err, "Failed to serialize scene snapshot"),
                }
            }

            _ => unreachable!(
                "Common key action handler encountered backend specific action {:?}",
                action
            ),
        }
    }

    // Common action handlers shared across all backends

    pub(crate) fn handle_app_switcher_next(&mut self) {
        if self.workspaces.get_show_all() {
            self.workspaces.expose_set_visible(false);
        }
        self.workspaces.app_switcher.next();
    }

    pub(crate) fn handle_app_switcher_prev(&mut self) {
        if self.workspaces.get_show_all() {
            self.workspaces.expose_set_visible(false);
        }
        self.workspaces.app_switcher.previous();
    }

    pub(crate) fn handle_app_switcher_quit(&mut self) {
        self.workspaces.quit_appswitcher_app();
    }

    pub(crate) fn handle_toggle_maximize(&mut self) {
        self.toggle_maximize_focused_window();
    }

    pub(crate) fn handle_close_window(&mut self) {
        self.close_focused_window();
    }

    pub(crate) fn handle_app_switcher_next_window(&mut self) {
        if let Some(wid) = self.workspaces.raise_next_app_window() {
            self.set_keyboard_focus_on_surface(&wid);
        }
    }

    pub(crate) fn handle_expose_show_desktop(&mut self) {
        if self.workspaces.get_show_desktop() {
            self.workspaces.expose_show_desktop(-1.0, true);
        } else {
            self.workspaces.expose_show_desktop(1.0, true);
        }
    }

    pub(crate) fn handle_expose_show_all(&mut self) {
        if self.workspaces.get_show_all() {
            self.workspaces.expose_set_visible(false);
        } else {
            // Exit show desktop mode if active, don't enter expose yet
            if self.workspaces.get_show_desktop() {
                self.workspaces.expose_show_desktop(-1.0, true);
                return;
            }
            // Dismiss all popups before entering expose mode
            // to release pointer grabs that would intercept events
            self.dismiss_all_popups();
            self.workspaces.expose_set_visible(true);
        }
    }

    pub(crate) fn handle_workspace_num(&mut self, n: usize) {
        self.set_current_workspace_index(n);
    }
}

pub fn resolve_shortcut_action(config: &Config, action: &ShortcutAction) -> Option<KeyAction> {
    match action {
        ShortcutAction::Builtin(builtin) => match builtin {
            BuiltinAction::Quit => Some(KeyAction::Quit),
            BuiltinAction::Screen { index } => Some(KeyAction::Screen(*index)),
            BuiltinAction::ScaleUp => Some(KeyAction::ScaleUp),
            BuiltinAction::ScaleDown => Some(KeyAction::ScaleDown),
            BuiltinAction::RotateOutput => Some(KeyAction::RotateOutput),
            BuiltinAction::ToggleDecorations => Some(KeyAction::ToggleDecorations),
            BuiltinAction::ApplicationSwitchNext => Some(KeyAction::ApplicationSwitchNext),
            BuiltinAction::ApplicationSwitchPrev => Some(KeyAction::ApplicationSwitchPrev),
            BuiltinAction::ApplicationSwitchQuit => Some(KeyAction::ApplicationSwitchQuit),
            BuiltinAction::ToggleMaximizeWindow => Some(KeyAction::ToggleMaximize),
            BuiltinAction::CloseWindow => Some(KeyAction::CloseWindow),
            BuiltinAction::ApplicationSwitchNextWindow => {
                Some(KeyAction::ApplicationSwitchNextWindow)
            }
            BuiltinAction::ExposeShowDesktop => Some(KeyAction::ExposeShowDesktop),
            BuiltinAction::ExposeShowAll => Some(KeyAction::ExposeShowAll),
            BuiltinAction::WorkspaceNum { index } => Some(KeyAction::WorkspaceNum(*index)),
            BuiltinAction::SceneSnapshot => Some(KeyAction::SceneSnapshot),
        },
        ShortcutAction::RunCommand(run) => {
            Some(KeyAction::Run((run.cmd.clone(), run.args.clone())))
        }
        ShortcutAction::OpenDefaultApp { role, fallback } => {
            match default_apps::resolve(role, fallback.as_deref(), config) {
                Some((cmd, args)) => Some(KeyAction::Run((cmd, args))),
                None => {
                    tracing::warn!(
                        role,
                        "no default application found for role; ignoring shortcut action"
                    );
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::shortcuts::RunCommandConfig;

    #[test]
    fn builtin_quit_maps_to_key_action() {
        let config = Config::default();
        let action = ShortcutAction::Builtin(BuiltinAction::Quit);
        assert!(matches!(
            resolve_shortcut_action(&config, &action),
            Some(KeyAction::Quit)
        ));
    }

    #[test]
    fn run_command_maps_to_key_action() {
        let config = Config::default();
        let action = ShortcutAction::RunCommand(RunCommandConfig {
            cmd: "echo".into(),
            args: vec!["hello".into()],
        });
        let result = resolve_shortcut_action(&config, &action).expect("command resolved");
        match result {
            KeyAction::Run((cmd, args)) => {
                assert_eq!(cmd, "echo");
                assert_eq!(args, vec!["hello".to_string()]);
            }
            other => panic!("unexpected key action: {:?}", other),
        }
    }

    #[test]
    fn open_default_uses_fallback_when_unknown() {
        let config = Config::default();
        let action = ShortcutAction::OpenDefaultApp {
            role: "nonexistent-role".into(),
            fallback: Some("xterm".into()),
        };
        let result = resolve_shortcut_action(&config, &action).expect("fallback resolved");
        match result {
            KeyAction::Run((cmd, args)) => {
                assert_eq!(cmd, "xterm");
                assert!(args.is_empty());
            }
            other => panic!("unexpected key action: {:?}", other),
        }
    }
}
