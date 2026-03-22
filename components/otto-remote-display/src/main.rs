use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn};

mod airplay;
mod discovery;
mod encoder;
mod screenshare;

#[derive(Parser)]
#[command(
    name = "otto-remote-display",
    about = "Stream Otto outputs to AirPlay devices"
)]
struct Cli {
    /// Output connector to stream (e.g., "HDMI-A-1"). Lists available if omitted.
    #[arg(short, long)]
    output: Option<String>,

    /// AirPlay device name to connect to. Discovers if omitted.
    #[arg(short, long)]
    device: Option<String>,

    /// Target framerate
    #[arg(short, long, default_value = "30")]
    fps: u32,

    /// Target bitrate in kbps
    #[arg(short, long, default_value = "8000")]
    bitrate: u32,
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

    // Step 1: Discover AirPlay receivers
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

    // Step 2: Connect to Otto's screenshare D-Bus API
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

    let session = screenshare::start_recording(&connector).await?;
    info!("PipeWire node ID: {}", session.node_id);

    // Step 3: Set up GStreamer encoder pipeline
    let (pipeline, frame_rx) = encoder::EncoderPipeline::new(session.node_id, cli.fps, cli.bitrate)?;
    info!("Encoder ready: {}", pipeline.encoder_name());

    // Step 4: Connect to Apple TV and start streaming
    let mut airplay_session = airplay::AirPlaySession::new(device, frame_rx);

    info!("Connecting to Apple TV...");
    airplay_session.connect().await?;

    info!("Starting stream...");
    pipeline.start()?;

    // Run until Ctrl+C or error
    tokio::select! {
        result = airplay_session.run() => {
            if let Err(e) = result {
                warn!("AirPlay stream ended: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Interrupted, shutting down...");
        }
    }

    pipeline.stop()?;
    session.stop().await?;
    info!("Done.");

    Ok(())
}
