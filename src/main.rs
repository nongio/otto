#[cfg(not(feature = "udev"))]
static POSSIBLE_BACKENDS: &[&str] = &[
    #[cfg(feature = "winit")]
    "--winit : Run anvil as a X11 or Wayland client using winit.",
    #[cfg(feature = "udev")]
    "--tty-udev : Run anvil as a tty udev client (requires root if without logind).",
    #[cfg(feature = "udev")]
    "--probe : Probe available displays and resolutions, then exit.",
    #[cfg(feature = "x11")]
    "--x11 : Run anvil as an X11 client.",
];

#[cfg(feature = "profile-with-tracy-mem")]
#[global_allocator]
static GLOBAL: profiling::tracy_client::ProfiledAllocator<std::alloc::System> =
    profiling::tracy_client::ProfiledAllocator::new(std::alloc::System, 10);

#[tokio::main]
async fn main() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("info")
            .compact()
            .init();
    }

    #[cfg(feature = "profile-with-tracy")]
    profiling::tracy_client::Client::start();

    profiling::register_thread!("Main Thread");

    #[cfg(feature = "profile-with-puffin")]
    let _server = puffin_http::Server::new(&format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT));
    #[cfg(feature = "profile-with-puffin")]
    profiling::puffin::set_scopes_on(true);

    let arg = ::std::env::args().nth(1);
    match arg.as_ref().map(|s| &s[..]) {
        #[cfg(feature = "winit")]
        Some("--winit") => {
            tracing::info!("Starting otto with winit backend");
            std::env::set_var("OTTO_BACKEND", "winit");
            otto::winit::run_winit();
        }
        #[cfg(feature = "udev")]
        Some("--tty-udev") => {
            tracing::info!("Starting otto on a tty using udev");
            std::env::set_var("OTTO_BACKEND", "tty-udev");
            otto::udev::run_udev();
        }
        #[cfg(feature = "udev")]
        Some("--probe") => {
            tracing::info!("Probing available displays and resolutions");
            otto::udev::probe_displays();
        }
        #[cfg(feature = "x11")]
        Some("--x11") => {
            tracing::info!("Starting otto with x11 backend");
            std::env::set_var("OTTO_BACKEND", "x11");
            otto::x11::run_x11();
        }
        Some(other) => {
            tracing::error!("Unknown backend: {}", other);
        }
        None => {
            // Auto-detect backend based on environment
            if std::env::var("WAYLAND_DISPLAY").is_ok() {
                // Running inside a Wayland compositor - use winit
                #[cfg(feature = "winit")]
                {
                    tracing::info!("Auto-detected Wayland session, starting with winit backend");
                    std::env::set_var("OTTO_BACKEND", "winit");
                    otto::winit::run_winit();
                }
                #[cfg(not(feature = "winit"))]
                {
                    tracing::error!("WAYLAND_DISPLAY is set but winit feature is not enabled");
                }
            } else {
                // No Wayland session - use tty-udev (bare metal)
                #[cfg(feature = "udev")]
                {
                    tracing::info!("No Wayland session detected, starting with tty-udev backend");
                    std::env::set_var("OTTO_BACKEND", "tty-udev");
                    otto::udev::run_udev();
                }
                #[cfg(not(feature = "udev"))]
                {
                    tracing::error!("No WAYLAND_DISPLAY and udev feature is not enabled");
                    println!("USAGE: otto [--backend]");
                    println!();
                    println!("Possible backends are:");
                    for b in POSSIBLE_BACKENDS {
                        println!("\t{}", b);
                    }
                }
            }
        }
    }
}
