pub mod ntp;
pub mod stream;

use anyhow::{Context, Result};
use std::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::discovery::AirPlayDevice;
use crate::encoder::EncodedFrame;

#[derive(Debug, Default)]
pub struct StreamInfo {
    pub width: u32,
    pub height: u32,
}

pub struct AirPlaySession {
    device: AirPlayDevice,
    frame_rx: mpsc::Receiver<EncodedFrame>,
    video_conn: Option<TcpStream>,
}

impl AirPlaySession {
    pub fn new(device: AirPlayDevice, frame_rx: mpsc::Receiver<EncodedFrame>) -> Self {
        Self {
            device,
            frame_rx,
            video_conn: None,
        }
    }

    /// Attempt to connect and set up mirroring. Tries the /stream endpoint without auth.
    pub async fn connect(&mut self) -> Result<()> {
        // Query capabilities (optional, may fail on some devices)
        match self.get_stream_info().await {
            Ok(info) => info!("Receiver caps: {}x{}", info.width, info.height),
            Err(e) => warn!("Could not query /stream.xml ({}), proceeding anyway", e),
        }

        // Start NTP time server for the Apple TV
        ntp::start_ntp_server().await?;

        // Open the mirroring connection
        self.start_mirroring().await?;

        Ok(())
    }

    /// Stream encoded H.264 frames until the encoder stops or an error occurs.
    pub async fn run(&mut self) -> Result<()> {
        info!("Streaming... (Ctrl+C to stop)");

        let mut codec_sent = false;
        let mut frame_count: u64 = 0;
        let session_start = std::time::Instant::now();

        loop {
            let frame = match self.frame_rx.recv() {
                Ok(f) => f,
                Err(_) => {
                    info!("Encoder stopped, ending stream");
                    break;
                }
            };

            let ntp_time = ntp::elapsed_to_ntp(session_start.elapsed());

            // On first keyframe, extract and send SPS/PPS as codec data
            if frame.is_keyframe && !codec_sent {
                if let Some(codec_data) = stream::extract_codec_data(&frame.data) {
                    self.send_packet(stream::PACKET_TYPE_CODEC_DATA, &codec_data, ntp_time)
                        .await?;
                    codec_sent = true;
                    info!("Sent codec data ({} bytes)", codec_data.len());
                }
            }

            self.send_packet(stream::PACKET_TYPE_VIDEO, &frame.data, ntp_time)
                .await?;

            frame_count += 1;
            if frame_count % 300 == 0 {
                let elapsed = session_start.elapsed().as_secs_f64();
                info!(
                    "Sent {} frames ({:.1} fps avg)",
                    frame_count,
                    frame_count as f64 / elapsed
                );
            }

            // Heartbeat every ~60 frames
            if frame_count % 60 == 0 {
                self.send_packet(stream::PACKET_TYPE_HEARTBEAT, &[], ntp_time)
                    .await?;
            }
        }

        Ok(())
    }

    async fn get_stream_info(&self) -> Result<StreamInfo> {
        let addr = format!("{}:{}", self.device.ip, self.device.port);
        let mut conn = TcpStream::connect(&addr).await?;

        let request = format!(
            "GET /stream.xml HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: otto-remote-display/0.1\r\n\
             Connection: close\r\n\
             \r\n",
            addr
        );

        conn.write_all(request.as_bytes()).await?;

        let mut buf = vec![0u8; 8192];
        let n = conn.read(&mut buf).await?;
        let response = String::from_utf8_lossy(&buf[..n]);
        debug!("stream.xml response:\n{}", response);

        // Basic XML parsing for width/height
        let mut info = StreamInfo::default();
        if let Some(w) = extract_xml_value(&response, "width") {
            info.width = w.parse().unwrap_or(1920);
        }
        if let Some(h) = extract_xml_value(&response, "height") {
            info.height = h.parse().unwrap_or(1080);
        }

        Ok(info)
    }

    async fn start_mirroring(&mut self) -> Result<()> {
        let addr = format!("{}:{}", self.device.ip, self.device.port);
        let mut conn = TcpStream::connect(&addr).await?;

        let body = stream::build_stream_plist()?;

        let request = format!(
            "POST /stream HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: otto-remote-display/0.1\r\n\
             Content-Type: application/x-apple-binary-plist\r\n\
             Content-Length: {}\r\n\
             \r\n",
            addr,
            body.len(),
        );

        conn.write_all(request.as_bytes()).await?;
        conn.write_all(&body).await?;

        // Read HTTP response headers
        let mut header_buf = vec![0u8; 4096];
        let n = conn.read(&mut header_buf).await?;
        let response = String::from_utf8_lossy(&header_buf[..n]);
        info!("Mirroring response: {}", response.lines().next().unwrap_or("(empty)"));

        // Check for success
        if !response.contains("200") {
            anyhow::bail!(
                "Apple TV rejected mirroring request (may need auth disabled):\n{}",
                response
            );
        }

        self.video_conn = Some(conn);
        info!("Mirroring connection established");

        Ok(())
    }

    async fn send_packet(&mut self, ptype: u16, data: &[u8], ntp_time: u64) -> Result<()> {
        let conn = self.video_conn.as_mut().context("Not connected")?;
        let header = stream::build_packet_header(ptype, data.len(), ntp_time);
        conn.write_all(&header).await?;
        if !data.is_empty() {
            conn.write_all(data).await?;
        }
        Ok(())
    }
}

fn extract_xml_value<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}
