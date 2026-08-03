//! otto-rdp — RDP bridge for an Otto virtual output.
//!
//! Serves the frames Otto renders for a virtual (PipeWire) output over RDP,
//! and injects the RDP client's mouse/keyboard back into the compositor via
//! virtual-pointer/virtual-keyboard, targeted at that output. Run it next to
//! Otto, on Otto's own Wayland socket — no need to look up the PipeWire node
//! id, the bridge resolves it from the output name via the PipeWire registry:
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 otto-rdp [--output virtual-1] [--listen 0.0.0.0:3389]
//! ```
//!
//! Pass `--node <id>` directly only if you already have it (e.g. from
//! Otto's startup log: `Virtual output 'virtual-1' started (PipeWire node N)`)
//! and want to skip discovery.
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
//! Security: TLS (self-signed) is on by default — required by mstsc and
//! Microsoft's mobile clients, and harmless for FreeRDP (`/cert:ignore`).
//! Pass `--no-tls` for the plain-RDP security layer instead
//! (`xfreerdp /sec:rdp`); note plain-RDP clients then can't use TLS-only
//! clients and vice versa.

mod discover;
mod egfx;
mod h264;
mod pipewire_capture;
mod rdp;
mod screencast;
mod tls;
mod wl_input;

use anyhow::Context;
use ironrdp_server::RdpServer;

fn usage() -> ! {
    eprintln!(
        "usage: otto-rdp [--output <name>] [--node <id> | --connector <name>] [--port <n> | --listen <addr:port>]\n\
         \n\
         --list       List every output Otto exposes (physical and virtual,\n\
                      with size and which is already streaming), then exit\n\
         --output     Wayland output name to serve and bind input to\n\
                      (default: virtual-1). The PipeWire node is discovered\n\
                      automatically — no need to read it out of Otto's log.\n\
         --node       Skip discovery: use this PipeWire node id directly\n\
                      (from Otto's log, or a virtual-output stream you\n\
                      already know)\n\
         --connector  Serve a PHYSICAL output via screenshare instead of a\n\
                      virtual one, e.g. eDP-1 (mutually exclusive with\n\
                      --node; also the default --output)\n\
         --port       RDP listen port on 0.0.0.0 (default: 3389)\n\
         --listen     Full RDP listen address, overrides --port\n\
                      (default: 0.0.0.0:3389)\n\
         --desktop    Serve this desktop size (WxH) instead of the client's\n\
                      reported box. For clients that render 1:1 in physical\n\
                      pixels but report their box in points (mobile apps):\n\
                      set the device's physical screen resolution\n\
         --no-tls     Use the plain-RDP security layer instead of TLS\n\
                      (`xfreerdp /sec:rdp`). TLS (self-signed certificate,\n\
                      persisted in ~/.local/state/otto-rdp) is on by default\n\
                      — required by mstsc and Microsoft's mobile clients.\n\
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
    list: bool,
    node: Option<u32>,
    connector: Option<String>,
    output: Option<String>,
    listen: std::net::SocketAddr,
    desktop: Option<(u32, u32)>,
    tls: bool,
    /// Force the legacy raw-bitmap path instead of hardware H.264 (EGFX).
    bitmap: bool,
}

const DEFAULT_PORT: u16 = 3389;

fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_args() -> Args {
    let mut list = false;
    let mut node = None;
    let mut connector = None;
    let mut output = None;
    let mut desktop = None;
    // TLS defaults on: mstsc and Microsoft's mobile clients refuse the
    // plain-RDP security layer outright, and TLS is harmless for FreeRDP
    // (`/cert:ignore`) — so the safe default serves the widest client set.
    let mut tls = true;
    let mut bitmap = false;
    let mut port = DEFAULT_PORT;
    let mut listen_override = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--list" => list = true,
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
            "--port" => match it.next().and_then(|v| v.parse().ok()) {
                Some(v) => port = v,
                None => usage(),
            },
            "--listen" => match it.next().and_then(|v| v.parse().ok()) {
                Some(v) => listen_override = Some(v),
                None => usage(),
            },
            "--desktop" => match it.next().as_deref().and_then(parse_size) {
                Some(v) => desktop = Some(v),
                None => usage(),
            },
            "--tls" => tls = true,
            "--no-tls" => tls = false,
            "--bitmap" => bitmap = true,
            _ => usage(),
        }
    }
    if node.is_some() && connector.is_some() {
        // --node is an explicit virtual-output node id; --connector routes
        // through the physical-output screencast path. Can't do both.
        usage();
    }
    let listen =
        listen_override.unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], port)));
    Args {
        list,
        node,
        connector,
        output,
        listen,
        desktop,
        tls,
        bitmap,
    }
}

/// `--list`: print every output Otto currently exposes, physical and
/// virtual, with what's needed to serve it. Wayland (`wl_output`/xdg-output)
/// is the source of truth for names+sizes — it covers both kinds, since
/// Otto exposes virtual outputs as `wl_output` globals too. PipeWire is
/// cross-referenced only to say which ones are *already streaming* (virtual
/// outputs stream from startup; physical connectors start a capture session
/// on demand via `--connector`, so they never show up here beforehand).
async fn list_outputs() -> anyhow::Result<()> {
    let wl_outputs = tokio::task::spawn_blocking(wl_input::list_outputs)
        .await
        .context("output-listing task panicked")?
        .context("listing Wayland outputs")?;

    let pw_outputs =
        tokio::task::spawn_blocking(|| discover::list_outputs(std::time::Duration::from_secs(2)))
            .await
            .context("PipeWire-listing task panicked")?
            .unwrap_or_default();

    if wl_outputs.is_empty() {
        println!("no outputs found — is Otto running on this Wayland socket?");
        return Ok(());
    }

    println!("{:<14} {:<11} {:<9} node", "OUTPUT", "SIZE", "KIND");
    for (name, (w, h)) in &wl_outputs {
        let node = pw_outputs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id);
        let (kind, node_col) = match node {
            Some(id) => ("virtual", id.to_string()),
            None => ("physical", "-".to_string()),
        };
        println!(
            "{:<14} {:<11} {:<9} {}",
            name,
            format!("{w}x{h}"),
            kind,
            node_col
        );
    }
    println!(
        "\nvirtual outputs: `otto-rdp --output <name>` (node discovered automatically)\n\
         physical outputs: `otto-rdp --connector <name>` (starts a capture session on connect)"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = parse_args();

    if args.list {
        return list_outputs().await;
    }

    // Resolve the PipeWire node. Three ways, in priority order:
    //  1. --node: use the given id directly, no lookup.
    //  2. --connector: obtain it from Otto's screenshare service (creates an
    //     on-demand capture session for that physical output).
    //  3. Otherwise: look up a virtual output by name (--output, default
    //     "virtual-1") via the PipeWire registry — Otto tags each virtual
    //     output's node with an `otto.output.name` property, so no manual
    //     node id from the log is needed.
    // --output defaults to the connector so input binds to that screen.
    let output = args
        .output
        .clone()
        .or_else(|| args.connector.clone())
        .unwrap_or_else(|| "virtual-1".to_string());
    let node = match (args.node, &args.connector) {
        (Some(n), _) => n,
        (None, Some(connector)) => {
            tracing::info!("requesting screenshare of '{connector}' from Otto…");
            screencast::node_for_connector(connector)
                .await
                .context("obtaining a PipeWire node for the connector")?
        }
        (None, None) => {
            tracing::info!("looking up PipeWire node for virtual output '{output}'…");
            let output_owned = output.clone();
            tokio::task::spawn_blocking(move || {
                discover::node_for_output(&output_owned, std::time::Duration::from_secs(5))
            })
            .await
            .context("discovery task panicked")?
            .context("discovering the PipeWire node for the virtual output")?
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

    // Newest captured frame, so a client that subscribes for display updates
    // gets a picture right away instead of waiting for the next capture.
    let latest = pipewire_capture::LatestFrame::new();

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
        // Lets the EGFX driver pull an IDR out of the encoder the moment a
        // client's surface is mapped, so a fresh connection paints the desktop
        // immediately instead of showing black until the next scheduled
        // keyframe (which the encoder counts in frames, not seconds).
        let keyframe = h264::KeyframeRequester::new();

        // The EGFX driver runs from startup so its readiness poll is already
        // live when the graphics channel opens — the client drops within ~25ms
        // of the capability confirm if the first graphics PDU (ResetGraphics)
        // doesn't follow promptly, and the encoder can take ~150ms to spin up.
        // It only acts for AVC clients (its ensure_surface gates on the codec).
        tokio::spawn(egfx::drive(
            std::sync::Arc::clone(&shared),
            enc_rx,
            keyframe.clone(),
        ));

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
                if let Err(e) = h264::spawn(cfg, enc_tx, keyframe) {
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
        let latest_bmp = latest.clone();
        tokio::spawn(async move {
            if shared_bmp.wait_codec().await == egfx::Codec::Bitmap {
                tracing::info!("auto-fallback: starting bitmap capture for a non-AVC client");
                pipewire_capture::spawn(node, size, target_bmp, frames_bmp, latest_bmp);
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
        pipewire_capture::spawn(
            node,
            size,
            target.clone(),
            frames_tx.clone(),
            latest.clone(),
        );
        frames_tx
    };

    let display = rdp::VirtualOutputDisplay {
        size,
        frames,
        target: target.clone(),
        served: size,
        latest,
        desktop_override: args.desktop,
        egfx_mode,
        gfx_shared: egfx_shared,
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
    let mut server = builder.build();

    server.run().await
}
