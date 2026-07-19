//! Hardware H.264 encoder for the EGFX / AVC420 path.
//!
//! Builds a GStreamer graph that pulls the virtual output straight off its
//! PipeWire node and encodes it on the GPU:
//!
//! ```text
//! pipewiresrc ! videorate ! vapostproc(NV12) ! vah264enc ! h264parse ! appsink
//! ```
//!
//! Each `appsink` sample is one H.264 access unit in Annex-B byte-stream form
//! (SPS/PPS re-emitted on every IDR via `h264parse config-interval=-1`), which
//! is exactly what `ironrdp-egfx`'s `send_avc420_frame()` wants. Frames are
//! forwarded over an unbounded channel; the sink itself caps the encoded
//! backlog (`max-buffers`/`drop`), and the EGFX side drops on backpressure, so
//! the queue never grows without bound.
//!
//! Unlike the raw-bitmap path (`pipewire_capture.rs`), nothing is read back to
//! the CPU or box-filtered here: the dmabuf goes GPU→encoder→small bitstream.

use anyhow::{anyhow, Context};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use tokio::sync::mpsc;

/// One encoded access unit.
pub struct EncodedFrame {
    /// Annex-B H.264 (`stream-format=byte-stream, alignment=au`).
    pub data: Vec<u8>,
    /// IDR / keyframe — SPS+PPS are prepended (config-interval=-1). The EGFX
    /// driver must send the first frame it forwards as a keyframe and must not
    /// resume with a P-frame after dropping one.
    pub keyframe: bool,
}

/// Encoder configuration. `width`/`height` are the desktop served to the RDP
/// client (its negotiated box, or `--desktop`); the native output is scaled and
/// aspect-fit into that size on the GPU (`vapostproc add-borders`), so the coded
/// picture matches the client's expected desktop exactly.
pub struct Config {
    pub node_id: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

impl Config {
    /// fps, then bitrate, from the environment (`OTTO_RDP_FPS`,
    /// `OTTO_RDP_BITRATE` in kbps), with sensible hardware-encode defaults.
    pub fn from_env(node_id: u32, width: u32, height: u32) -> Self {
        let fps = std::env::var("OTTO_RDP_FPS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|f| *f > 0)
            .unwrap_or(30);
        // A remote desktop over H.264 is comfortable at ~12–20 Mbit for a
        // laptop-sized panel; default in that range, scaled loosely by area.
        let default_kbps = {
            let mp = (width as u64 * height as u64) as f64 / (1920.0 * 1080.0);
            (12_000.0 * mp.max(0.5)).clamp(4_000.0, 40_000.0) as u32
        };
        let bitrate_kbps = std::env::var("OTTO_RDP_BITRATE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|b| *b > 0)
            .unwrap_or(default_kbps);
        Self {
            node_id,
            width,
            height,
            fps,
            bitrate_kbps,
        }
    }
}

/// Build and start the encode pipeline, forwarding access units to `tx`.
///
/// The caller owns the receiver and can start consuming (and driving EGFX)
/// before this is called, so proactive graphics setup isn't blocked by the
/// pipeline's ~100ms transition to PLAYING. The pipeline is owned by a
/// background thread that also watches the bus; when it hits EOS or an error
/// the thread logs and exits, dropping `tx` so the receiver observes `None`.
pub fn spawn(cfg: Config, tx: mpsc::UnboundedSender<EncodedFrame>) -> anyhow::Result<()> {
    gst::init().context("gstreamer init")?;

    // `vah264enc` = VA-API (hardware) H.264; `vah264lpenc` is the low-power
    // fixed-function variant. Overridable for boards where only one exists.
    let encoder = std::env::var("OTTO_RDP_H264_ENCODER").unwrap_or_else(|_| "vah264enc".into());
    // Keyframe every ~2 s bounds how long a client that joins mid-stream (or
    // recovers from a dropped frame) waits for something decodable.
    let key_int_max = (cfg.fps * 2).max(1);

    // `vapostproc` must consume the source's dmabuf DIRECTLY — Otto's virtual
    // output advertises dmabuf-only formats with a MANDATORY LINEAR modifier
    // (see pipewire_capture.rs), so any system-memory `video/x-raw` capsfilter
    // between pipewiresrc and vapostproc never intersects ("no more input
    // formats"). Convert+scale on the GPU, then rate-limit on the NV12 output.
    // No B-frames (latency, and AVC420 is progressive); CBR keeps the RDP link
    // at a predictable rate.
    let desc = format!(
        "pipewiresrc path={node} do-timestamp=true keepalive-time=1000 \
           ! vapostproc add-borders=true \
           ! video/x-raw,format=NV12,width={w},height={h},pixel-aspect-ratio=1/1 \
           ! videorate \
           ! video/x-raw,framerate={fps}/1 \
           ! {encoder} name=enc rate-control=cbr bitrate={kbps} b-frames=0 key-int-max={kim} \
           ! h264parse config-interval=-1 \
           ! video/x-h264,stream-format=byte-stream,alignment=au \
           ! appsink name=sink emit-signals=false sync=false max-buffers=3 drop=true",
        node = cfg.node_id,
        fps = cfg.fps,
        w = cfg.width,
        h = cfg.height,
        encoder = encoder,
        kbps = cfg.bitrate_kbps,
        kim = key_int_max,
    );

    tracing::info!(
        "starting H.264 pipeline: {}x{} @ {}fps, {} kbps, encoder {}",
        cfg.width,
        cfg.height,
        cfg.fps,
        cfg.bitrate_kbps,
        encoder
    );
    tracing::debug!("gst pipeline: {desc}");

    let pipeline = gst::parse::launch(&desc)
        .context("building the H.264 GStreamer pipeline (is gst-plugin-va installed?)")?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed pipeline was not a gst::Pipeline"))?;

    let sink = pipeline
        .by_name("sink")
        .context("appsink 'sink' missing from pipeline")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow!("'sink' was not an appsink"))?;

    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                // A buffer without DELTA_UNIT is a keyframe (IDR here).
                let keyframe = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
                let frame = EncodedFrame {
                    data: map.as_slice().to_vec(),
                    keyframe,
                };
                // Fails only once the EGFX driver has gone away — then tear the
                // pipeline down by signalling EOS to the streaming thread.
                match tx.send(frame) {
                    Ok(()) => Ok(gst::FlowSuccess::Ok),
                    Err(_) => Err(gst::FlowError::Eos),
                }
            })
            .build(),
    );

    pipeline
        .set_state(gst::State::Playing)
        .context("setting H.264 pipeline to Playing")?;

    // Own the pipeline on a dedicated thread and pump its bus so errors/EOS
    // are logged and state is cleaned up. The thread's lifetime keeps the
    // pipeline (and thus the appsink callback) alive.
    std::thread::Builder::new()
        .name("h264-bus".into())
        .spawn(move || {
            let bus = pipeline.bus().expect("pipeline has no bus");
            for msg in bus.iter_timed(gst::ClockTime::NONE) {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Eos(_) => {
                        tracing::info!("H.264 pipeline reached EOS");
                        break;
                    }
                    MessageView::Error(err) => {
                        tracing::error!(
                            "H.264 pipeline error from {:?}: {} ({:?})",
                            err.src().map(|s| s.path_string()),
                            err.error(),
                            err.debug()
                        );
                        break;
                    }
                    _ => {}
                }
            }
            let _ = pipeline.set_state(gst::State::Null);
        })
        .context("spawning H.264 bus thread")?;

    Ok(())
}
