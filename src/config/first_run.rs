//! The first session's configuration.
//!
//! When Otto starts and the user has no configuration of their own, it writes
//! one with the icon theme the user's other desktops point at. It happens
//! once: from then on the file is the user's, and nothing here runs again.

use super::{file, user_config_file};

/// Write the user's first configuration, if they have none yet.
///
/// Must run before the configuration is first loaded, so the session starts
/// with what is written here.
pub fn prepare() {
    // A test session writes to a throwaway directory and must not depend on
    // what the host has installed.
    if super::isolated_config_file().is_some() {
        return;
    }
    let Some(path) = user_config_file() else {
        return;
    };
    if path.exists() {
        return;
    }
    // Nothing found means nothing to write: the system configuration applies.
    let Some(icon_theme) = otto_kit::icon_theme::detect_icon_theme() else {
        return;
    };

    let mut doc = toml_edit::DocumentMut::new();
    set(&mut doc, "icon_theme", toml::Value::String(icon_theme.clone()));

    match file::store_document(&path, &doc) {
        Ok(()) => tracing::info!(
            path = %path.display(),
            icon_theme,
            "wrote the first configuration"
        ),
        Err(err) => tracing::warn!("could not write the first configuration: {err}"),
    }
}

fn set(doc: &mut toml_edit::DocumentMut, key: &str, value: toml::Value) {
    if let Some(value) = file::to_edit_value(&value) {
        if let Err(err) = file::set_key(doc, key, value) {
            tracing::warn!("first configuration: cannot set `{key}`: {err}");
        }
    }
}
