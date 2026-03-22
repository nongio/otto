use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
#[cfg(feature = "airplay")]
use tracing::warn;

#[cfg(feature = "airplay")]
mod airplay;
#[cfg(feature = "airplay")]
mod discovery;
#[cfg(feature = "airplay")]
mod encoder;
#[cfg(feature = "airplay")]
mod hls;
#[cfg(feature = "rdp")]
mod rdp;
mod screenshare;

#[derive(Clone, Debug, PartialEq)]
enum Protocol {
    #[cfg(feature = "airplay")]
    AirPlay,
    #[cfg(feature = "rdp")]
    Rdp,
}

impl std::str::FromStr for Protocol {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            #[cfg(feature = "airplay")]
            "airplay" => Ok(Protocol::AirPlay),
            #[cfg(feature = "rdp")]
            "rdp" => Ok(Protocol::Rdp),
            _ => {
                let mut opts = Vec::new();
                #[cfg(feature = "airplay")]
                opts.push("airplay");
                #[cfg(feature = "rdp")]
                opts.push("rdp");
                Err(format!("unknown protocol '{}'. Available: {}", s, opts.join(", ")))
            }
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "airplay")]
            Protocol::AirPlay => write!(f, "airplay"),
            #[cfg(feature = "rdp")]
            Protocol::Rdp => write!(f, "rdp"),
        }
    }
}

#[derive(Parser)]
#[command(
    name = "otto-remote-display",
    about = "Stream Otto outputs to remote displays via RDP or AirPlay"
)]
struct Cli {
    /// Protocol to use
    #[arg(short, long, default_value = "rdp")]
    protocol: Protocol,

    /// Output connector to stream (e.g., "HDMI-A-1"). Lists available if omitted.
    #[arg(short, long)]
    output: Option<String>,

    /// Target framerate
    #[arg(short, long, default_value = "30")]
    fps: u32,

    /// Target bitrate in kbps
    #[arg(short, long, default_value = "8000")]
    bitrate: u32,

    // --- RDP options ---
    /// RDP listen port
    #[arg(long, default_value = "3389")]
    port: u16,

    // --- AirPlay options ---
    /// AirPlay device name to connect to. Discovers if omitted.
    #[arg(short, long)]
    device: Option<String>,

    /// Path to credentials file (from pyatv pairing). Required for modern Apple TVs.
    #[arg(short, long)]
    credentials: Option<String>,

    /// HTTP port for HLS server
    #[arg(long, default_value = "8099")]
    hls_port: u16,

    /// Test AirPlay connection with test pattern (skip screenshare, use videotestsrc)
    #[arg(long)]
    test_connect: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.protocol {
        #[cfg(feature = "rdp")]
        Protocol::Rdp => run_rdp(cli).await,
        #[cfg(feature = "airplay")]
        Protocol::AirPlay => run_airplay(cli).await,
    }
}

#[cfg(feature = "rdp")]
async fn run_rdp(cli: Cli) -> Result<()> {
    info!("Starting RDP server on port {}...", cli.port);

    // Get output to stream
    let outputs = screenshare::list_outputs().await?;
    info!("Available outputs: {:?}", outputs);

    let connector = if let Some(ref out) = cli.output {
        if !outputs.contains(out) {
            anyhow::bail!("Output '{}' not found. Available: {:?}", out, outputs);
        }
        out.clone()
    } else {
        outputs
            .first()
            .context("No outputs available from Otto")?
            .clone()
    };

    info!("Streaming output: {}", connector);

    let screenshare_session = screenshare::start_recording(&connector).await?;
    info!("PipeWire node ID: {}", screenshare_session.node_id);

    // Run RDP server
    let result = rdp::run_server(
        cli.port,
        screenshare_session.node_id,
        cli.fps,
    ).await;

    screenshare_session.stop().await?;
    result
}

#[cfg(feature = "airplay")]
async fn run_airplay(cli: Cli) -> Result<()> {
    // Discover AirPlay receivers
    info!("Searching for AirPlay devices...");
    let devices = discovery::browse_airplay(std::time::Duration::from_secs(5))?;

    if devices.is_empty() {
        anyhow::bail!("No AirPlay devices found on the network");
    }

    let device = if let Some(name) = &cli.device {
        devices
            .iter()
            .find(|d| d.name.to_lowercase().contains(&name.to_lowercase()))
            .with_context(|| {
                format!(
                    "Device '{}' not found. Available: {}",
                    name,
                    devices
                        .iter()
                        .map(|d| d.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
            .clone()
    } else {
        println!("Found AirPlay devices:");
        for (i, d) in devices.iter().enumerate() {
            println!(
                "  {}. {} ({}:{}, model: {})",
                i + 1,
                d.name,
                d.ip,
                d.port,
                d.model
            );
        }
        if devices.len() == 1 {
            info!("Auto-selecting: {}", devices[0].name);
        } else {
            info!("Selecting first device: {}", devices[0].name);
        }
        devices[0].clone()
    };

    info!("Target: {} at {}:{}", device.name, device.ip, device.port);

    // Load credentials
    let creds = if let Some(creds_path) = &cli.credentials {
        let creds_str = std::fs::read_to_string(creds_path)
            .with_context(|| format!("Failed to read credentials from {}", creds_path))?;
        let c = airplay::auth::HapCredentials::from_pyatv_string(creds_str.trim())?;
        info!("Loaded credentials from {}", creds_path);
        Some(c)
    } else {
        None
    };

    let local_ip = hls::get_local_ip()?;
    info!("Local IP: {}", local_ip);

    if cli.test_connect {
        info!("=== TEST MODE: videotestsrc + HLS ===");

        let hls_server = hls::HlsServer::new_test(cli.hls_port)?;
        let hls_url = hls_server.playlist_url(&local_ip);
        info!("HLS URL: {}", hls_url);

        hls_server.start()?;
        let _http_handle = hls_server.start_http_server().await?;

        info!("Waiting for HLS segments...");
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;

        let (_, frame_rx) = std::sync::mpsc::channel();
        let mut session = airplay::AirPlaySession::new(device, frame_rx);
        if let Some(c) = creds {
            session = session.with_credentials(c);
        }

        info!("Connecting to Apple TV...");
        let conn = session.connect(&hls_url).await?;
        info!("=== HLS playback started! ===");

        tokio::select! {
            result = session.run(conn) => {
                if let Err(e) = result {
                    warn!("Session ended: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Interrupted, shutting down...");
            }
        }

        hls_server.stop()?;
        return Ok(());
    }

    // Full mode: PipeWire → HLS → AirPlay
    let outputs = screenshare::list_outputs().await?;
    info!("Available outputs: {:?}", outputs);

    let connector = if let Some(ref out) = cli.output {
        if !outputs.contains(out) {
            anyhow::bail!("Output '{}' not found. Available: {:?}", out, outputs);
        }
        out.clone()
    } else {
        outputs
            .first()
            .context("No outputs available from Otto")?
            .clone()
    };

    info!("Streaming output: {}", connector);

    let screenshare_session = screenshare::start_recording(&connector).await?;
    info!("PipeWire node ID: {}", screenshare_session.node_id);

    let hls_server =
        hls::HlsServer::new(screenshare_session.node_id, cli.fps, cli.bitrate, cli.hls_port)?;
    let hls_url = hls_server.playlist_url(&local_ip);
    info!("HLS URL: {}", hls_url);

    hls_server.start()?;
    let _http_handle = hls_server.start_http_server().await?;

    info!("Waiting for HLS segments...");
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let (_, frame_rx) = std::sync::mpsc::channel();
    let mut session = airplay::AirPlaySession::new(device, frame_rx);
    if let Some(c) = creds {
        session = session.with_credentials(c);
    }

    info!("Connecting to Apple TV...");
    let conn = session.connect(&hls_url).await?;
    info!("HLS playback started!");

    tokio::select! {
        result = session.run(conn) => {
            if let Err(e) = result {
                warn!("Session ended: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Interrupted, shutting down...");
        }
    }

    hls_server.stop()?;
    screenshare_session.stop().await?;
    info!("Done.");

    Ok(())
}
