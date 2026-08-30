static POSSIBLE_BACKENDS: &[&str] = &[
    #[cfg(feature = "winit")]
    "--winit      Run otto as a X11 or Wayland client using winit.",
    #[cfg(feature = "udev")]
    "--tty-udev   Run otto on a tty using udev (requires root or logind).",
    #[cfg(feature = "udev")]
    "--probe      Probe available displays and resolutions, then exit.",
    #[cfg(feature = "x11")]
    "--x11        Run otto as an X11 client.",
];

fn print_help() {
    println!("otto {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("    otto [OPTION] [FLAGS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help         Print this help message");
    println!("    --version          Print version information");
    for b in POSSIBLE_BACKENDS {
        println!("    {}", b);
    }
    println!();
    println!("FLAGS:");
    println!(
        "    --systemd-notify   Send sd_notify(READY=1) and activate graphical-session.target"
    );
    println!(
        "    --login            Run as a greeter host: primary output only, no dock/switcher,"
    );
    println!("                       launches the greeter client instead of autostart");
    println!();
    println!("If no backend is specified, otto auto-detects based on the environment.");
}

#[cfg(feature = "profile-with-tracy-mem")]
#[global_allocator]
static GLOBAL: profiling::tracy_client::ProfiledAllocator<std::alloc::System> =
    profiling::tracy_client::ProfiledAllocator::new(std::alloc::System, 10);

#[tokio::main]
async fn main() {
    // Handle informational flags before any side-effectful initialization.
    match std::env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("otto {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--help") | Some("-h") => {
            print_help();
            return;
        }
        Some(other)
            if !other.starts_with("--winit")
                && !other.starts_with("--tty-udev")
                && !other.starts_with("--probe")
                && !other.starts_with("--x11")
                && !other.starts_with("--headless")
                && !other.starts_with("--login")
                && !other.starts_with("--systemd-notify") =>
        {
            eprintln!("Unknown argument: {}", other);
            eprintln!();
            print_help();
            std::process::exit(1);
        }
        _ => {}
    }

    // Greeter mode. Set before any state is built so output mapping and
    // workspace chrome can consult it during construction.
    if std::env::args().any(|a| a == "--login") {
        otto::login::set_login_mode(true);
    }

    // Check for --systemd-notify flag (can appear as first or second argument)
    if std::env::args().any(|a| a == "--systemd-notify") {
        // SAFETY: setting env var before any threads are spawned
        unsafe { std::env::set_var("OTTO_SYSTEMD_NOTIFY", "1") };
    }

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

    // Load the string catalogues before any chrome is built — the dock's
    // context menus are assembled during construction. `config.locales`
    // rather than the environment: it is the setting the user edits, and it
    // is what the components are told over the portal.
    otto_kit::i18n::init(&otto::configured_locales());

    // The same language, in the form applications read. Otto's chrome follows
    // the `locales` setting; a GTK or Qt app follows the environment, and
    // without this an Italian desktop launches English apps.
    otto::locale_env::export();

    // Corner rounding travels the same way, and for the same reason: the top
    // bar and every otto-kit window draw their own corners, in their own
    // processes, and none of them reads Otto's configuration.
    otto::export_rounded_corners();
    otto::export_window_controls_side();
    otto::export_maximize_button();
    otto::export_color_scheme();

    #[cfg(feature = "profile-with-tracy")]
    profiling::tracy_client::Client::start();

    profiling::register_thread!("Main Thread");

    #[cfg(feature = "profile-with-puffin")]
    let _server = puffin_http::Server::new(&format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT));
    #[cfg(feature = "profile-with-puffin")]
    profiling::puffin::set_scopes_on(true);

    let arg = ::std::env::args()
        .skip(1)
        .find(|a| a != "--systemd-notify" && a != "--login");
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
        #[cfg(feature = "headless")]
        Some("--headless") => {
            tracing::info!("Starting otto with headless backend");
            std::env::set_var("OTTO_BACKEND", "headless");
            otto::headless::run_headless();
        }
        Some("--version") => {
            println!("otto {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--help") | Some("-h") => {
            print_help();
        }
        Some(other) => {
            eprintln!("Unknown argument: {}", other);
            eprintln!();
            print_help();
            std::process::exit(1);
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
                    print_help();
                }
            }
        }
    }
}
