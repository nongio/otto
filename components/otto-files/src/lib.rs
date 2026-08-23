//! Otto's file browser and file picker — see `specs/file-browser.md` and
//! `specs/file-picker.md`.
//!
//! One crate, two shells over one view layer. The browser is a document
//! window the user opens; the picker is a transient serving somebody else's
//! application through the XDG desktop portal. Below the chrome they are the
//! same code: the same directory model, the same async reads, the same
//! list/grid/column presentations, the same Quick View.
//!
//! ```sh
//! cargo run -p otto-files            # browse $HOME
//! cargo run -p otto-files -- /etc    # browse somewhere else
//! cargo run -p otto-files -- --picker  # serve org.otto.FilePicker1
//! ```

#[cfg(test)]
mod bench;

pub mod app;
pub mod dbus;
pub mod model;
pub mod pane_surfaces;
pub mod perf;
pub mod picker;
pub mod quickview;
pub mod scene;
pub mod thumbcache;
pub mod thumbnails;
pub mod view;
pub mod watch;
