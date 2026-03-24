mod animation;
mod app;
mod appmenu;
mod bar;
mod clock;
mod config;
mod dbusmenu;
mod focus;
mod tray;

use app::TopBarApp;
use otto_kit::AppRunner;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "otto_topbar=info".into()),
        )
        .init();

    let app = TopBarApp::new();
    AppRunner::new(app).run()?;
    Ok(())
}
