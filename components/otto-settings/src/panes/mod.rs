//! One module per settings pane.
//!
//! Each owns its own rows and their bindings, so panes can be worked on
//! independently.

pub mod about;
pub mod account;
pub mod agents;
pub mod appearance;
pub mod desk;
pub mod displays;
pub mod dock;
pub mod general;
pub mod keyboard;
pub mod keyboard_layouts;
pub mod lock_and_login;
pub mod pointing;
pub mod power;
pub mod privacy;
pub mod search;
pub mod sound;
pub mod tiling;

/// Hand a file to the application the desktop opens its type with.
///
/// Resolved through the same associations Files uses, so a TOML file opens in
/// whatever the user chose for it. On a thread of its own, spawned and
/// forgotten: reading the associations walks the application directories,
/// and the editor outlives this app, so neither may freeze the pane.
pub(crate) fn open_in_default_app(path: &std::path::Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let associations = otto_kit::mime_apps::Associations::load();
        let chain = otto_kit::filetype::ancestors(otto_kit::filetype::for_file(&path));
        let Some(app) = associations.default_for(&chain) else {
            eprintln!("settings: nothing opens {}", path.display());
            return;
        };
        if let Err(err) = otto_kit::mime_apps::open(app, std::slice::from_ref(&path)) {
            eprintln!("settings: could not open {}: {err}", path.display());
        }
    });
}
