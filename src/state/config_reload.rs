//! Live configuration reload.
//!
//! Most of the compositor reads its settings through `Config::with` every time
//! it needs them, so swapping the published config (see
//! [`crate::config::Config::reload`]) is enough for them. This module covers
//! the rest: the values something copied out of the config at startup and now
//! owns — input devices, the keyboard map, the cursor theme, the dock, the
//! wallpaper — plus the redraw that makes a purely visual change show up.

use smithay::input::keyboard::XkbConfig;
use smithay::reexports::calloop::{
    timer::{TimeoutAction, Timer},
    LoopHandle,
};
use tracing::{info, warn};

use crate::config::{section_changed, watcher::ConfigWatcher, Config};
use crate::state::{Backend, Otto};

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Watch the config files and re-apply them when they change.
    pub fn start_config_watcher(handle: &LoopHandle<'static, Self>) {
        let mut watcher = ConfigWatcher::new();
        let interval = crate::config::watcher::POLL_INTERVAL;

        if handle
            .insert_source(Timer::from_duration(interval), move |_, _, data| {
                if watcher.poll() {
                    data.reload_config();
                }
                TimeoutAction::ToDuration(interval)
            })
            .is_err()
        {
            warn!("Failed to start the config watcher; config changes need a restart");
        }
    }

    /// Re-read the config files and apply what changed. A no-op when the
    /// merged result is the same as the running one — a file can be touched,
    /// or rewritten by Otto itself (the dock persists its own settings),
    /// without anything actually moving.
    pub fn reload_config(&mut self) {
        let Some((previous, config)) = Config::reload() else {
            return;
        };
        info!("Config changed on disk, applying it to the running session");

        if section_changed(&previous.input, &config.input) {
            self.backend_data.apply_input_config(&config);
            self.apply_keyboard_config(&config);
        } else if previous.keyboard_repeat_delay != config.keyboard_repeat_delay
            || previous.keyboard_repeat_rate != config.keyboard_repeat_rate
        {
            self.apply_keyboard_config(&config);
        }

        if previous.cursor_theme != config.cursor_theme
            || previous.cursor_size != config.cursor_size
        {
            self.cursor_manager
                .reload(&config.cursor_theme, config.cursor_size as u8);
            self.cursor_texture_cache.clear();
        }

        self.workspaces.dock.apply_config(&config.dock);

        if previous.background_image != config.background_image
            || previous.background_color != config.background_color
        {
            self.apply_background_config(&config);
        }

        if section_changed(&previous.layer_shell, &config.layer_shell) {
            for output in self.workspaces.outputs().cloned().collect::<Vec<_>>() {
                self.recalculate_exclusive_zones(&output);
            }
        }

        if previous.screen_scale != config.screen_scale {
            self.apply_screen_scale(config.screen_scale);
        }

        // Fonts, theme colours and the rest are read while drawing: they only
        // need the scene to be drawn again.
        self.backend_data.request_redraw();
    }

    fn apply_keyboard_config(&mut self, config: &Config) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };

        keyboard.change_repeat_info(config.keyboard_repeat_rate, config.keyboard_repeat_delay);

        let layout = config.input.xkb_layout.clone().unwrap_or_default();
        let variant = config.input.xkb_variant.clone().unwrap_or_default();
        let options = if config.input.xkb_options.is_empty() {
            None
        } else {
            Some(config.input.xkb_options.join(","))
        };
        let xkb_config = XkbConfig {
            layout: &layout,
            variant: &variant,
            options,
            ..Default::default()
        };
        if let Err(err) = keyboard.set_xkb_config(self, xkb_config) {
            warn!("Keeping the previous keyboard layout, the new one failed: {err}");
        }
    }

    fn apply_background_config(&mut self, config: &Config) {
        let color = crate::utils::parse_hex_color(&config.background_color);
        let image = crate::utils::image_from_path(&config.background_image, (2048, 2048));
        if image.is_none() && !config.background_image.is_empty() {
            warn!(
                "Failed to load background image from path: {}",
                config.background_image
            );
        }

        for output_workspaces in self.workspaces.output_workspaces.values() {
            for workspace in output_workspaces.workspace_views.iter() {
                workspace
                    .background_view
                    .set_background(image.clone(), color);
            }
        }
    }

    /// Apply a new global scale to every output, the same way the scale
    /// keybinding does.
    fn apply_screen_scale(&mut self, scale: f64) {
        use smithay::output::Scale;

        let outputs: Vec<_> = self.workspaces.outputs().cloned().collect();
        for output in &outputs {
            output.change_current_state(None, None, Some(Scale::Fractional(scale)), None);
        }
        let pointer_location = self.pointer.current_location();
        crate::shell::fixup_positions(&mut self.workspaces, pointer_location);

        // The workspaces model caches the scale alongside the physical screen
        // size, and the size did not change — re-set it so the scene is laid
        // out at the new scale.
        let (width, height) = self
            .workspaces
            .with_model(|model| (model.width, model.height));
        if width > 0 && height > 0 {
            self.workspaces.set_screen_dimension(width, height);
        }

        for output in &outputs {
            self.backend_data.reset_buffers(output);
        }
        #[cfg(feature = "xwayland")]
        self.update_xwayland_scale();
    }
}
