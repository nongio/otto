//! otto-rdp — RDP bridge for an Otto virtual output.
//!
//! Serves the frames Otto renders for a virtual (PipeWire) output over RDP,
//! and injects the RDP client's mouse/keyboard back into the compositor via
//! virtual-pointer/virtual-keyboard, targeted at that output. Run it next to
//! Otto, on Otto's own Wayland socket:
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 otto-rdp --node <pipewire-node-id> [--output virtual-1] \
//!     [--listen 0.0.0.0:3389]
//! ```
//!
//! The PipeWire node id is logged by Otto at startup:
//! `Virtual output 'virtual-1' started (PipeWire node N)`.
//!
//! To serve a PHYSICAL output instead, pass its connector and let the bridge
//! obtain the node from Otto's screenshare service:
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 otto-rdp --connector eDP-1
//! ```
//!
//! Input then drives the real cursor and the actually-focused windows on that
//! screen — this mirrors and remote-controls the physical desktop.
//!
//! Security: MVP serves without TLS (RdpServerSecurity::None). Use it on a
//! trusted network. Connect with e.g.:
//! `xfreerdp /v:host:3389 /sec:rdp -clipboard`

mod pipewire_capture;
mod rdp;
mod screencast;
mod tls;
mod wl_input;

use anyhow::Context;
use ironrdp_server::RdpServer;

fn usage() -> ! {
    eprintln!(
        "usage: otto-rdp (--node <id> | --connector <name>) [--output <name>] [--listen <addr:port>]\n\
         \n\
         --node       PipeWire node id of a virtual-output stream (from Otto's log)\n\
         --connector  Physical output to capture via screenshare, e.g. eDP-1\n\
                      (mutually exclusive with --node; also the default --output)\n\
         --output     Wayland output name to bind input to (default: virtual-1,\n\
                      or the --connector value)\n\
         --listen     RDP listen address (default: 0.0.0.0:3389)\n\
         --desktop    Serve this desktop size (WxH) instead of the client's\n\
                      reported box. For clients that render 1:1 in physical\n\
                      pixels but report their box in points (mobile apps):\n\
                      set the device's physical screen resolution\n\
         --tls        Accept TLS-security connections with a self-signed\n\
                      certificate (persisted in ~/.local/state/otto-rdp).\n\
                      Required by mstsc and Microsoft's mobile clients;\n\
                      plain-RDP clients (xfreerdp /sec:rdp) then can't connect"
    );
    std::process::exit(2);
}

struct Args {
    node: Option<u32>,
    connector: Option<String>,
    output: Option<String>,
    listen: std::net::SocketAddr,
    desktop: Option<(u32, u32)>,
    tls: bool,
}

fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_args() -> Args {
    let mut node = None;
    let mut connector = None;
    let mut output = None;
    let mut desktop = None;
    let mut tls = false;
    let mut listen = "0.0.0.0:3389".parse().unwrap();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--node" => match it.next().and_then(|v| v.parse().ok()) {
                Some(v) => node = Some(v),
                None => usage(),
            },
            "--connector" => match it.next() {
                Some(v) => connector = Some(v),
                None => usage(),
            },
            "--output" => match it.next() {
                Some(v) => output = Some(v),
                None => usage(),
            },
            "--listen" => match it.next().and_then(|v| v.parse().ok()) {
                Some(v) => listen = Some(v).unwrap(),
                None => usage(),
            },
            "--desktop" => match it.next().as_deref().and_then(parse_size) {
                Some(v) => desktop = Some(v),
                None => usage(),
            },
            "--tls" => tls = true,
            _ => usage(),
        }
    }
    if node.is_some() == connector.is_some() {
        // Need exactly one source.
        usage();
    }
    Args {
        node,
        connector,
        output,
        listen,
        desktop,
        tls,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = parse_args();

    // Resolve the PipeWire node: given directly (--node) or obtained from
    // Otto's screenshare service for a physical connector (--connector).
    // --output defaults to the connector so input binds to that screen.
    let output = args
        .output
        .clone()
        .or_else(|| args.connector.clone())
        .unwrap_or_else(|| "virtual-1".to_string());
    let node = match args.node {
        Some(n) => n,
        None => {
            let connector = args.connector.clone().unwrap();
            tracing::info!("requesting screenshare of '{connector}' from Otto…");
            screencast::node_for_connector(&connector)
                .await
                .context("obtaining a PipeWire node for the connector")?
        }
    };

    // Input thread: discovers the output (reports its pixel size), then
    // injects commands. The size gates RDP startup so the advertised
    // desktop always matches the injection target.
    let (input_tx, input_rx) = std::sync::mpsc::channel();
    let (size_tx, size_rx) = std::sync::mpsc::channel();
    let output_name = output.clone();
    std::thread::Builder::new()
        .name("wl-input".into())
        .spawn(move || {
            if let Err(e) = wl_input::run(&output_name, input_rx, size_tx) {
                tracing::error!("wayland input thread terminated: {e:#}");
                std::process::exit(1);
            }
        })?;

    let size = size_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .context("timed out discovering the target output on the Wayland socket")?;

    // Frame capture from the resolved PipeWire node. The target starts at
    // native and narrows once a client negotiates its desktop size.
    let target = pipewire_capture::TargetSize::native();
    let frames = pipewire_capture::spawn(node, size, target.clone());

    let display = rdp::VirtualOutputDisplay {
        size,
        frames,
        target: target.clone(),
        served: size,
        desktop_override: args.desktop,
    };
    let input = rdp::InputForwarder {
        tx: input_tx,
        native: size,
        served: target,
    };

    tracing::info!(
        "serving RDP on {} for output '{}' ({}x{}, PipeWire node {})",
        args.listen,
        output,
        size.0,
        size.1,
        node
    );

    let builder = RdpServer::builder().with_addr(args.listen);
    let builder = if args.tls {
        tracing::info!("TLS security enabled (self-signed certificate)");
        builder.with_tls(tls::acceptor().context("setting up TLS")?)
    } else {
        builder.with_no_security()
    };
    let mut server = builder
        .with_input_handler(input)
        .with_display_handler(display)
        .build();

    server.run().await
}
