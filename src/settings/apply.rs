//! Reconciling the running compositor with a changed configuration.
//!
//! The appearance settings, the dock, the switcher, the pointer, the icon
//! theme and `input.*` are reconciled today, and the schema says so: a setting
//! marked `live` here must genuinely take effect, and everything else is
//! marked `restart` rather than being quietly accepted and ignored.

use crate::config::Config;
use crate::state::{Backend, Otto};

/// Whether `apply_live` knows how to apply this identifier.
///
/// The schema and this module have to agree exactly — a `live` setting with no
/// apply path is the dishonesty the spec forbids — and the test below holds
/// them together.
pub fn is_applied_live(id: &str) -> bool {
    matches!(
        id,
        "dock.size"
            | "dock.position"
            | "dock.autohide"
            | "dock.magnification"
            | "dock.genie_scale"
            | "dock.genie_span"
            | "dock.colorize_icons"
            | "dock.colorize_color"
            | "dock.colorize_intensity"
            | "appswitcher.colorize_icons"
            | "appswitcher.follow_cursor"
            | "accent_color"
            | "background_image"
            | "background_color"
            | "theme_scheme"
            | "rounded_corners"
            | "window_controls_side"
            | "show_maximize_button"
            | "cursor_theme"
            | "cursor_size"
            | "icon_theme"
    ) || is_input_id(id)
}

/// Whether `id` is one of the libinput/pointer settings applied by
/// reconfiguring the connected devices.
fn is_input_id(id: &str) -> bool {
    matches!(
        id,
        "input.tap_enabled"
            | "input.tap_drag_enabled"
            | "input.tap_drag_lock_enabled"
            | "input.touchpad_click_method"
            | "input.touchpad_dwt_enabled"
            | "input.touchpad_natural_scroll_enabled"
            | "input.touchpad_left_handed"
            | "input.touchpad_middle_emulation_enabled"
            | "input.pointer_accel_speed"
            | "input.pointer_accel_profile"
            | "input.scroll_speed"
    )
}

/// Bring the running system in line with the current configuration for `id`.
///
/// The new value is read from the live configuration rather than passed in, so
/// this is the same operation whether the change came from the bus, from a dock
/// interaction, or from a file reload.
pub fn apply_live<B: Backend + 'static>(state: &mut Otto<B>, id: &str) -> Result<(), String> {
    match id {
        // `render_dock` rebuilds the icon strip from the live configuration —
        // slot sizes, the icon colour filter — and then re-applies
        // magnification at the pointer's current position, which is where the
        // genie values are read. So every dock setting that only changes how
        // the strip is drawn lands with the same call.
        "dock.size" | "dock.genie_scale" | "dock.genie_span" => {
            state.workspaces.dock.render_dock();
            Ok(())
        }
        // The tint belongs to the icons rather than to the dock: the app
        // switcher mirrors the same sources and applies the same filter, so
        // both have to be redrawn. Neither reads the tint from view state, so
        // the switcher needs a forced render rather than a state update.
        "dock.colorize_icons" | "dock.colorize_color" | "dock.colorize_intensity" => {
            state.workspaces.dock.render_dock();
            state.workspaces.app_switcher.rerender();
            Ok(())
        }
        // Whether the switcher joins in that tint. The dock is unaffected —
        // its own icons, and its drag ghost, follow `dock.colorize_icons` —
        // so only the switcher has to be redrawn.
        "appswitcher.colorize_icons" => {
            state.workspaces.app_switcher.rerender();
            Ok(())
        }
        "dock.position" => {
            state.workspaces.dock.apply_dock_position();
            // The dock reserves screen space on a different edge now.
            state.remaximize_maximized_windows();
            Ok(())
        }
        "dock.autohide" => {
            let dock = state.workspaces.dock.clone();
            dock.apply_autohide();
            // With autohide off the dock is permanently visible, so maximized
            // windows have to shrink to make room for it.
            state.remaximize_maximized_windows();
            Ok(())
        }
        "dock.magnification" => {
            state.workspaces.dock.apply_magnification();
            Ok(())
        }
        // The accent is read inside render functions rather than held in view
        // state, so the views have to be re-rendered rather than updated: an
        // unchanged state hash would make `update_state` a no-op. Publishing
        // comes first — the render functions read the store, not the config.
        "accent_color" => {
            crate::theme::publish_accent();
            state.workspaces.rerender_accent_colored_views();
            Ok(())
        }
        // Both background settings are read together by `reload_background`:
        // the colour is the gradient behind a wallpaper that is absent or
        // unreadable, so changing either one is the same reconciliation.
        "background_image" | "background_color" => state.workspaces.reload_background(),
        // Light against dark changes the palette every render function reads,
        // so all of Otto's own chrome is repainted. The accent is republished
        // first: it is usually a palette *name*, and the colour that name
        // stands for is different under the other scheme.
        //
        // Client applications hear about it from the Settings portal, which
        // relays Otto's `Changed` signal as `color-scheme`; the environment is
        // updated too, so a process started after this point agrees with the
        // ones already running even where no portal is answering.
        "theme_scheme" => {
            crate::export_color_scheme();
            crate::theme::publish_accent();
            state.workspaces.rerender_chrome();
            state.refresh_window_decorations();
            Ok(())
        }
        // Corner rounding is read through `otto_kit::corners` by every draw
        // routine, in this process and in the ones drawing the bar and their
        // own titlebars. Publishing updates this process and the session
        // environment; the portal's `org.otto.desktop` namespace is what
        // carries it to the windows already on screen.
        "rounded_corners" => {
            crate::export_rounded_corners();
            state.workspaces.rerender_chrome();
            state.refresh_window_decorations();
            Ok(())
        }
        // Both travel the same way, and both are read while a titlebar is
        // drawn rather than being held in its model — so only the bars have to
        // be rebuilt, not the rest of the chrome.
        "window_controls_side" => {
            crate::export_window_controls_side();
            state.refresh_window_decorations();
            Ok(())
        }
        "show_maximize_button" => {
            crate::export_maximize_button();
            state.refresh_window_decorations();
            Ok(())
        }
        // The pointer's own theme and size. `reload` drops the loaded theme
        // and every cursor resolved from it; the texture cache is keyed by
        // icon and scale rather than by theme, so it has to go as well or the
        // old bitmaps are re-uploaded for the new names.
        "cursor_theme" | "cursor_size" => {
            let (theme, size) = Config::with(|c| (c.cursor_theme.clone(), c.cursor_size));
            state
                .cursor_manager
                .reload(&theme, size.clamp(1, 255) as u8);
            state.cursor_texture_cache.clear();
            // Nothing else invalidates on a pointer that is not moving.
            state.backend_data.request_redraw();
            Ok(())
        }
        // Icons are decoded once and kept, so the theme reaches nothing until
        // the caches are dropped and the dock's applications are looked up
        // again. Chrome icons (the workspace selector's) come out of the same
        // otto-kit cache, so they are repainted here too; the app switcher
        // resolves its applications when it is next opened.
        "icon_theme" => {
            state.workspaces.dock.reload_icons();
            state.workspaces.rerender_chrome();
            Ok(())
        }
        // Read from the live configuration each time the switcher is shown
        // (`Workspaces::app_switcher_output_for`), so the next Alt-Tab already
        // uses the new value.
        "appswitcher.follow_cursor" => Ok(()),
        // Otto's own scroll handling multiplies by the live configuration on
        // every axis event (`input::pointer`), so there is nothing to push
        // anywhere — the next scroll already uses the new value.
        "input.scroll_speed" => Ok(()),
        // The rest of `input.*` is libinput device state, and one device is as
        // cheap to reconfigure as all of them, so the whole set is re-pushed
        // rather than mapping each identifier to its own setter.
        id if is_input_id(id) => {
            state.backend_data.reconfigure_input_devices();
            Ok(())
        }
        other => Err(format!("`{other}` cannot be applied while Otto is running")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::schema::{self, Apply};

    #[test]
    fn every_live_setting_has_an_apply_path() {
        for spec in schema::SETTINGS {
            if spec.apply == Apply::Live {
                assert!(
                    is_applied_live(spec.id),
                    "`{}` is declared live but apply_live does not handle it",
                    spec.id
                );
            } else {
                assert!(
                    !is_applied_live(spec.id),
                    "`{}` has an apply path but is not declared live",
                    spec.id
                );
            }
        }
    }

    /// The appearance settings a user changes in the settings app and expects
    /// to see at once, plus the dock's magnification numbers. Each of these
    /// needed an apply path written for it, and putting one back on `Restart`
    /// would silently reintroduce "log out for this to take effect".
    #[test]
    fn the_appearance_settings_apply_live() {
        for id in [
            "theme_scheme",
            "rounded_corners",
            "window_controls_side",
            "show_maximize_button",
            "cursor_theme",
            "cursor_size",
            "icon_theme",
            "appswitcher.follow_cursor",
            "dock.genie_scale",
            "dock.genie_span",
        ] {
            assert_eq!(
                schema::lookup(id).expect("in schema").apply,
                Apply::Live,
                "`{id}` should apply live"
            );
        }
        // The display scale is deliberately not among them: nothing
        // reconciles the outputs, the bar and maximized geometry against a new
        // one, so the schema says so rather than half-applying it.
        assert_eq!(
            schema::lookup("screen_scale").expect("in schema").apply,
            Apply::Restart
        );
    }

    #[test]
    fn every_input_setting_that_reaches_a_device_is_live() {
        // The touchpad and pointer settings are the ones a user expects to
        // feel immediately; the keyboard ones still need a restart.
        for id in [
            "input.tap_enabled",
            "input.touchpad_click_method",
            "input.pointer_accel_speed",
            "input.scroll_speed",
        ] {
            assert_eq!(
                schema::lookup(id).expect("in schema").apply,
                Apply::Live,
                "`{id}` should apply live"
            );
        }
        assert_eq!(
            schema::lookup("input.xkb_layout").expect("in schema").apply,
            Apply::Restart
        );
    }
}
