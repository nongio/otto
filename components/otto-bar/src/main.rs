mod app;
mod appmenu;
mod bar;
mod battery;
mod clock;
mod config;
mod dbusmenu;
mod keyboard_layout;
mod logout;
mod power;
mod tray;

use app::TopBarApp;
use otto_kit::AppRunner;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    otto_kit::logging::init("info");

    // Before any string is read — the clock's format comes from the
    // catalogue. Asks the compositor rather than reading LANG, so that
    // "Preferred languages" moves the bar too.
    otto_kit::i18n::init_from_desktop();

    let app = TopBarApp::new();
    AppRunner::new(app).run()?;
    Ok(())
}
