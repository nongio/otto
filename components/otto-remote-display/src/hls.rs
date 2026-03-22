use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::path::PathBuf;
use tracing::{debug, info};
use warp::Filter;

const ENCODER_CANDIDATES: &[(&str, &str)] = &[
    ("vaapih264enc", "VAAPI H.264 (Intel/AMD)"),
    ("vah264enc", "VA-API stateless H.264"),
    ("nvh264enc", "NVENC H.264 (NVIDIA)"),
    ("x264enc", "x264 software H.264"),
];

fn find_encoder() -> Result<&'static str> {
    for (name, desc) in ENCODER_CANDIDATES {
        if gst::ElementFactory::find(name).is_some() {
            info!("Selected encoder: {} ({})", name, desc);
            return Ok(name);
        }
    }
    anyhow::bail!(
        "No H.264 encoder found. Install vaapi, nvenc, or x264 GStreamer plugins."
    );
}

fn encoder_props(name: &str, bitrate_kbps: u32) -> String {
    match name {
        "vaapih264enc" => format!("vaapih264enc rate-control=cbr bitrate={bitrate_kbps}"),
        "vah264enc" => format!("vah264enc rate-control=cbr bitrate={bitrate_kbps}"),
        "nvh264enc" => {
            format!("nvh264enc bitrate={bitrate_kbps} preset=low-latency-hq rc-mode=cbr")
        }
        "x264enc" => {
            format!("x264enc bitrate={bitrate_kbps} tune=zerolatency speed-preset=ultrafast")
        }
        _ => unreachable!(),
    }
}

pub struct HlsServer {
    pipeline: gst::Pipeline,
    hls_dir: PathBuf,
    http_port: u16,
    encoder_name: String,
}

impl HlsServer {
    /// Create an HLS pipeline that reads from PipeWire and writes .ts segments + .m3u8.
    pub fn new(
        pipewire_node_id: u32,
        fps: u32,
        bitrate_kbps: u32,
        http_port: u16,
    ) -> Result<Self> {
        gst::init()?;

        let hls_dir = std::env::temp_dir().join("otto-hls");
        std::fs::create_dir_all(&hls_dir)?;
        // Clean previous segments
        for entry in std::fs::read_dir(&hls_dir)? {
            if let Ok(e) = entry {
                let _ = std::fs::remove_file(e.path());
            }
        }

        let encoder_name = find_encoder()?;
        let encoder_element = encoder_props(encoder_name, bitrate_kbps);

        let playlist_location = hls_dir.join("live.m3u8");
        let segment_location = hls_dir.join("segment%05d.ts");

        // hlssink2 writes live HLS — short segments for low latency
        let pipeline_str = format!(
            "pipewiresrc path={node_id} do-timestamp=true \
             ! videoconvert \
             ! video/x-raw,framerate={fps}/1 \
             ! {encoder} \
             ! h264parse \
             ! mpegtsmux \
             ! hlssink2 name=hlssink \
                playlist-location={playlist} \
                location={segment} \
                target-duration=2 \
                playlist-length=5 \
                max-files=10 \
                send-keyframe-requests=true",
            node_id = pipewire_node_id,
            fps = fps,
            encoder = encoder_element,
            playlist = playlist_location.display(),
            segment = segment_location.display(),
        );

        info!("HLS GStreamer pipeline: {}", pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| anyhow::anyhow!("Failed to cast to Pipeline"))?;

        Ok(Self {
            pipeline,
            hls_dir,
            http_port,
            encoder_name: encoder_name.to_string(),
        })
    }

    /// Create a test pipeline (no PipeWire, generates test pattern).
    pub fn new_test(http_port: u16) -> Result<Self> {
        gst::init()?;

        let hls_dir = std::env::temp_dir().join("otto-hls");
        std::fs::create_dir_all(&hls_dir)?;
        for entry in std::fs::read_dir(&hls_dir)? {
            if let Ok(e) = entry {
                let _ = std::fs::remove_file(e.path());
            }
        }

        let playlist_location = hls_dir.join("live.m3u8");
        let segment_location = hls_dir.join("segment%05d.ts");

        let pipeline_str = format!(
            "videotestsrc is-live=true \
             ! video/x-raw,width=1920,height=1080,framerate=30/1 \
             ! x264enc bitrate=4000 tune=zerolatency speed-preset=ultrafast key-int-max=60 \
             ! h264parse \
             ! mpegtsmux \
             ! hlssink2 name=hlssink \
                playlist-location={playlist} \
                location={segment} \
                target-duration=2 \
                playlist-length=5 \
                max-files=10 \
                send-keyframe-requests=true",
            playlist = playlist_location.display(),
            segment = segment_location.display(),
        );

        info!("HLS test pipeline: {}", pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| anyhow::anyhow!("Failed to cast to Pipeline"))?;

        Ok(Self {
            pipeline,
            hls_dir,
            http_port,
            encoder_name: "x264enc (test)".to_string(),
        })
    }

    pub fn encoder_name(&self) -> &str {
        &self.encoder_name
    }

    pub fn playlist_url(&self, local_ip: &str) -> String {
        format!("http://{}:{}/live.m3u8", local_ip, self.http_port)
    }

    pub fn start(&self) -> Result<()> {
        self.pipeline.set_state(gst::State::Playing)?;
        info!("HLS encoder pipeline started");
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.pipeline.set_state(gst::State::Null)?;
        info!("HLS encoder pipeline stopped");
        Ok(())
    }

    /// Start HTTP server serving the HLS directory.
    /// Returns a handle that can be used to shut down the server.
    pub async fn start_http_server(&self) -> Result<tokio::task::JoinHandle<()>> {
        let dir = self.hls_dir.clone();
        let port = self.http_port;

        // Wait for the playlist file to appear (pipeline needs a moment)
        info!(
            "HTTP server will serve HLS from {} on port {}",
            dir.display(),
            port
        );

        let cors = warp::cors()
            .allow_any_origin()
            .allow_methods(vec!["GET", "HEAD", "OPTIONS"])
            .allow_headers(vec!["content-type", "range"]);

        let hls_route = warp::get()
            .and(warp::path::tail())
            .and(warp::any().map(move || dir.clone()))
            .and_then(serve_hls_file)
            .with(cors);

        let handle = tokio::spawn(async move {
            info!("HLS HTTP server listening on 0.0.0.0:{}", port);
            warp::serve(hls_route).run(([0, 0, 0, 0], port)).await;
        });

        Ok(handle)
    }
}

async fn serve_hls_file(
    tail: warp::path::Tail,
    dir: PathBuf,
) -> Result<impl warp::Reply, warp::Rejection> {
    let filename = tail.as_str();

    // Only serve .m3u8 and .ts files
    if !filename.ends_with(".m3u8") && !filename.ends_with(".ts") {
        return Err(warp::reject::not_found());
    }

    // Prevent path traversal
    if filename.contains("..") || filename.contains('/') {
        return Err(warp::reject::not_found());
    }

    let path = dir.join(filename);
    debug!("HLS request: {} -> {}", filename, path.display());

    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| warp::reject::not_found())?;

    let content_type = if filename.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else {
        "video/mp2t"
    };

    Ok(warp::reply::with_header(
        warp::reply::with_header(data, "content-type", content_type),
        "cache-control",
        "no-cache, no-store",
    ))
}

/// Get our local IP address (the one facing the network, not 127.0.0.1).
pub fn get_local_ip() -> Result<String> {
    // Connect to a public address (doesn't actually send data) to determine local IP
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let addr = socket.local_addr()?;
    Ok(addr.ip().to_string())
}
