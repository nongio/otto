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

mod egfx;
mod h264;
mod indicator;
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
                      plain-RDP clients (xfreerdp /sec:rdp) then can't connect\n\
         --bitmap     Use the legacy raw-bitmap path (RemoteFX/RLE, software)\n\
                      instead of the default hardware H.264 (EGFX/AVC420).\n\
                      Needed for clients that don't support AVC420 graphics.\n\
                      \n\
         Env: OTTO_RDP_FPS (default 30 for H.264, 12 for bitmap),\n\
              OTTO_RDP_BITRATE (kbps, H.264 only),\n\
              OTTO_RDP_H264_ENCODER (default vah264enc; e.g. vah264lpenc)"
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
    /// Force the legacy raw-bitmap path instead of hardware H.264 (EGFX).
    bitmap: bool,
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
    let mut bitmap = false;
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
            "--bitmap" => bitmap = true,
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
        bitmap,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
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

    // Two paths: default hardware H.264 (GStreamer → EGFX/AVC420), or the
    // legacy raw-bitmap path (RemoteFX/RLE via ironrdp). They are mutually
    // exclusive per run — chosen here, before any client connects.
    let egfx_mode = !args.bitmap;

    // The input target maps client coordinates back to native pixels. The
    // bitmap path fills it from the letterbox layout at negotiation time; the
    // EGFX path serves native size, so the identity mapping is correct.
    let target = pipewire_capture::TargetSize::native();

    // The bitmap path streams raw frames over this channel; the EGFX path
    // captures directly in GStreamer, so the display handler gets an idle
    // (parked) channel and video flows over the graphics pipeline instead.
    let mut gfx_factory: Option<Box<dyn ironrdp_server::GfxServerFactory>> = None;
    let mut egfx_shared: Option<std::sync::Arc<egfx::GfxShared>> = None;
    let frames = if egfx_mode {
        let shared = std::sync::Arc::new(egfx::GfxShared::new());
        // The display subscribes to this either way; the raw capture only feeds
        // it if the client falls back from H.264 (started lazily below).
        let (frames_tx, _) =
            tokio::sync::broadcast::channel::<std::sync::Arc<pipewire_capture::Frame>>(4);
        let (enc_tx, enc_rx) = tokio::sync::mpsc::unbounded_channel::<h264::EncodedFrame>();

        // The EGFX driver runs from startup so its readiness poll is already
        // live when the graphics channel opens — the client drops within ~25ms
        // of the capability confirm if the first graphics PDU (ResetGraphics)
        // doesn't follow promptly, and the encoder can take ~150ms to spin up.
        // It only acts for AVC clients (its ensure_surface gates on the codec).
        tokio::spawn(egfx::drive(std::sync::Arc::clone(&shared), enc_rx));

        // Encoder coordinator: build the hardware H.264 pipeline only for an
        // AVC client, once its desktop size is negotiated. The GStreamer init
        // is blocking, so it runs on the blocking pool to avoid starving the
        // async workers the driver and RDP server need to respond in time.
        let shared_enc = std::sync::Arc::clone(&shared);
        tokio::spawn(async move {
            let (w, h) = shared_enc.wait_desktop().await;
            if shared_enc.wait_codec().await != egfx::Codec::Avc {
                return; // client disabled AVC → bitmap fallback, no encoder
            }
            let cfg = h264::Config::from_env(node, w as u32, h as u32);
            tokio::task::spawn_blocking(move || {
                if let Err(e) = h264::spawn(cfg, enc_tx) {
                    tracing::error!("failed to start hardware H.264 encoder: {e:#}");
                }
            });
        });

        // Bitmap fallback coordinator: if the client disabled AVC, start the
        // raw capture feeding the display's channel. Lazy, so an AVC client
        // never pays for the CPU capture/scale.
        let shared_bmp = std::sync::Arc::clone(&shared);
        let frames_bmp = frames_tx.clone();
        let target_bmp = target.clone();
        tokio::spawn(async move {
            if shared_bmp.wait_codec().await == egfx::Codec::Bitmap {
                tracing::info!("auto-fallback: starting bitmap capture for a non-AVC client");
                pipewire_capture::spawn(node, size, target_bmp, frames_bmp);
            }
        });

        gfx_factory = Some(Box::new(egfx::OttoGfxFactory::new(std::sync::Arc::clone(
            &shared,
        ))));
        egfx_shared = Some(shared);
        frames_tx
    } else {
        // Frame capture from the resolved PipeWire node. The target starts at
        // native and narrows once a client negotiates its desktop size.
        let (frames_tx, _) =
            tokio::sync::broadcast::channel::<std::sync::Arc<pipewire_capture::Frame>>(4);
        pipewire_capture::spawn(node, size, target.clone(), frames_tx.clone());
        frames_tx
    };

    // Privacy signal: a tray indicator, published only while a client is
    // actually being served. Nothing here can fail the bridge — a session bus
    // that isn't there (headless rigs) just means no icon is drawn.
    let (indicator, mut stop_rx) = indicator::Indicator::new(&output);
    let peer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

    let display = rdp::VirtualOutputDisplay {
        size,
        frames,
        target: target.clone(),
        served: size,
        desktop_override: args.desktop,
        egfx_mode,
        gfx_shared: egfx_shared,
        indicator: indicator.clone(),
        peer: std::sync::Arc::clone(&peer),
    };
    let input = rdp::InputForwarder {
        tx: input_tx,
        native: size,
        served: target,
    };

    tracing::info!(
        "serving RDP on {} for output '{}' ({}x{}, PipeWire node {}, {} codec)",
        args.listen,
        output,
        size.0,
        size.1,
        node,
        if egfx_mode {
            "hardware H.264 / AVC420"
        } else {
            "bitmap / RemoteFX"
        }
    );

    let builder = RdpServer::builder().with_addr(args.listen);
    let builder = if args.tls {
        tracing::info!("TLS security enabled (self-signed certificate)");
        builder.with_tls(tls::acceptor().context("setting up TLS")?)
    } else {
        builder.with_no_security()
    };
    let builder = builder
        .with_input_handler(input)
        .with_display_handler(display);
    let builder = match gfx_factory {
        Some(factory) => builder.with_gfx_factory(Some(factory)),
        None => builder,
    };
    let builder = builder.with_connection_handler(Some(Box::new(indicator::ConnectionStatus {
        indicator: indicator.clone(),
        peer: std::sync::Arc::clone(&peer),
    })));
    let mut server = builder.build();

    // "Stop Sharing" ends the whole bridge, not just the current client:
    // leaving the port listening would let the remote party reconnect
    // immediately, which is not what stopping sharing means. Quit drops any
    // live connection; the `stopping` flag then makes the connection handler
    // break the accept loop instead of waiting for the next client.
    {
        let events = server.event_sender().clone();
        tokio::spawn(async move {
            if stop_rx.recv().await.is_some() {
                indicator
                    .stopping
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                indicator.hide();
                let _ = events.send(ironrdp_server::ServerEvent::Quit(
                    "stopped by the user".to_owned(),
                ));
            }
        });
    }

    server.run().await
}
