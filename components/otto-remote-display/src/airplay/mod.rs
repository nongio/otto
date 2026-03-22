pub mod auth;
pub mod fairplay;
pub mod ntp;
pub mod stream;
pub mod tlv;

use anyhow::{Context, Result};
use std::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::discovery::AirPlayDevice;
use crate::encoder::EncodedFrame;

use auth::HapCredentials;

pub struct AirPlaySession {
    device: AirPlayDevice,
    frame_rx: mpsc::Receiver<EncodedFrame>,
    credentials: Option<HapCredentials>,
    cseq: u32,
    session_id: u32,
    hap_send_counter: u64,
    hap_recv_counter: u64,
}

impl AirPlaySession {
    pub fn new(device: AirPlayDevice, frame_rx: mpsc::Receiver<EncodedFrame>) -> Self {
        Self {
            device,
            frame_rx,
            credentials: None,
            cseq: 0,
            session_id: rand::random(),
            hap_send_counter: 0,
            hap_recv_counter: 0,
        }
    }

    pub fn with_credentials(mut self, creds: HapCredentials) -> Self {
        self.credentials = Some(creds);
        self
    }

    fn next_cseq(&mut self) -> u32 {
        self.cseq += 1;
        self.cseq
    }

    /// Full AirPlay 2 connection flow (HLS URL playback):
    /// 1. HAP pair-verify (establishes encrypted control channel)
    /// 2. RTSP SETUP (session init, no FairPlay needed for URL playback)
    /// 3. RTSP RECORD (start session)
    /// 4. POST /play (send HLS URL to Apple TV)
    pub async fn connect(&mut self, hls_url: &str) -> Result<TcpStream> {
        let addr = format!("{}:{}", self.device.ip, self.device.port);

        let creds = self.credentials.clone().context(
            "Credentials required for AirPlay 2. Pair with pyatv first.",
        )?;

        let mut conn = TcpStream::connect(&addr).await?;

        // Step 1: HAP pair-verify
        info!("Step 1: HAP pair-verify...");
        let shared_secret = auth::pair_verify(&mut conn, &creds).await?;
        info!("Authentication successful");

        let (output_key, input_key) = auth::derive_encryption_keys(&shared_secret)?;
        debug!("Derived HAP encryption keys");

        // Start NTP time server
        ntp::start_ntp_server().await?;

        // Step 2: RTSP SETUP — session init (no FairPlay, no ekey/eiv)
        info!("Step 2: RTSP SETUP (session init)...");
        let (timing_port, event_port) = self
            .rtsp_setup_session(&mut conn, &output_key, &input_key)
            .await?;
        info!(
            "Session initialized (timing_port={}, event_port={})",
            timing_port, event_port
        );

        // Step 3: RTSP RECORD
        info!("Step 3: RTSP RECORD...");
        self.rtsp_record(&mut conn, &output_key, &input_key).await?;
        info!("Recording started");

        // Step 4: POST /play with HLS URL
        info!("Step 4: POST /play (HLS URL)...");
        self.play_url(&mut conn, &output_key, &input_key, hls_url)
            .await?;
        info!("Playing HLS stream on Apple TV");

        Ok(conn)
    }

    /// RTSP SETUP: Session init (matching pyatv's format — no ekey/eiv, no FairPlay).
    async fn rtsp_setup_session(
        &mut self,
        conn: &mut TcpStream,
        output_key: &[u8],
        input_key: &[u8],
    ) -> Result<(u16, u16)> {
        let mut dict = plist::Dictionary::new();
        dict.insert("deviceID".into(), plist::Value::String("AA:BB:CC:DD:EE:FF".into()));
        dict.insert("sessionUUID".into(), plist::Value::String(uuid_v4()));
        dict.insert("timingPort".into(), plist::Value::Integer(7010.into()));
        dict.insert("timingProtocol".into(), plist::Value::String("NTP".into()));
        dict.insert("isMultiSelectAirPlay".into(), plist::Value::Boolean(true));
        dict.insert("groupContainsGroupLeader".into(), plist::Value::Boolean(false));
        dict.insert("macAddress".into(), plist::Value::String("AA:BB:CC:DD:EE:FF".into()));
        dict.insert("model".into(), plist::Value::String("iPhone14,3".into()));
        dict.insert("name".into(), plist::Value::String("Otto".into()));
        dict.insert("osBuildVersion".into(), plist::Value::String("20F66".into()));
        dict.insert("osName".into(), plist::Value::String("iPhone OS".into()));
        dict.insert("osVersion".into(), plist::Value::String("16.5".into()));
        dict.insert("senderSupportsRelay".into(), plist::Value::Boolean(false));
        dict.insert("sourceVersion".into(), plist::Value::String("690.7.1".into()));
        dict.insert("statsCollectionEnabled".into(), plist::Value::Boolean(false));

        let body = plist_to_binary(&plist::Value::Dictionary(dict))?;
        debug!("RTSP SETUP body: {} bytes", body.len());
        let resp_body = self
            .hap_rtsp_request(conn, "SETUP", output_key, input_key, &body)
            .await?;

        let resp_plist = plist::Value::from_reader(std::io::Cursor::new(&resp_body))
            .context("Failed to parse SETUP response plist")?;
        debug!("SETUP response: {:?}", resp_plist);
        let resp_dict = resp_plist.as_dictionary().context("SETUP response not a dict")?;

        let timing_port = resp_dict.get("timingPort")
            .and_then(|v| v.as_unsigned_integer()).unwrap_or(0) as u16;
        let event_port = resp_dict.get("eventPort")
            .and_then(|v| v.as_unsigned_integer()).unwrap_or(0) as u16;

        Ok((timing_port, event_port))
    }

    /// RTSP RECORD: Start the session.
    async fn rtsp_record(
        &mut self,
        conn: &mut TcpStream,
        output_key: &[u8],
        input_key: &[u8],
    ) -> Result<()> {
        self.hap_rtsp_request(conn, "RECORD", output_key, input_key, &[])
            .await?;
        Ok(())
    }

    /// POST /play: Tell Apple TV to play our HLS URL.
    async fn play_url(
        &mut self,
        conn: &mut TcpStream,
        output_key: &[u8],
        input_key: &[u8],
        hls_url: &str,
    ) -> Result<()> {
        let mut dict = plist::Dictionary::new();
        dict.insert("Content-Location".into(), plist::Value::String(hls_url.to_string()));
        dict.insert("Start-Position-Seconds".into(), plist::Value::Real(0.0));
        dict.insert("uuid".into(), plist::Value::String(uuid_v4()));
        dict.insert("streamType".into(), plist::Value::Integer(1.into()));
        dict.insert("mediaType".into(), plist::Value::String("file".into()));
        dict.insert("volume".into(), plist::Value::Real(1.0));
        dict.insert("rate".into(), plist::Value::Real(1.0));
        dict.insert("SenderMACAddress".into(), plist::Value::String("AA:BB:CC:DD:EE:FF".into()));
        dict.insert("model".into(), plist::Value::String("iPhone14,3".into()));
        dict.insert("clientBundleID".into(), plist::Value::String("dev.otto.remote".into()));
        dict.insert("clientProcName".into(), plist::Value::String("otto-remote-display".into()));
        dict.insert("osBuildVersion".into(), plist::Value::String("20F66".into()));

        let body = plist_to_binary(&plist::Value::Dictionary(dict))?;
        self.hap_post(conn, "/play", "application/x-apple-binary-plist", &body, output_key, input_key)
            .await?;
        Ok(())
    }

    /// Send an HTTP POST over the HAP-encrypted channel.
    async fn hap_post(
        &mut self,
        conn: &mut TcpStream,
        path: &str,
        content_type: &str,
        body: &[u8],
        output_key: &[u8],
        input_key: &[u8],
    ) -> Result<Vec<u8>> {
        let request = format!(
            "POST {} HTTP/1.1\r\n\
             User-Agent: AirPlay/320.20\r\n\
             Connection: keep-alive\r\n\
             X-Apple-HKP: 3\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             \r\n",
            path, content_type, body.len(),
        );

        let mut http_data = request.as_bytes().to_vec();
        http_data.extend_from_slice(body);

        let encrypted = auth::hap_encrypt(&http_data, output_key, self.hap_send_counter)?;
        conn.write_all(&encrypted).await?;
        self.hap_send_counter += (http_data.len() as u64 + 1023) / 1024;

        // Read response
        let mut buf = vec![0u8; 65536];
        let mut total = 0;
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_millis(1000),
                conn.read(&mut buf[total..]),
            ).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    total += n;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Ok(Err(e)) => anyhow::bail!("Read error: {}", e),
                Err(_) => break,
            }
        }
        if total == 0 {
            anyhow::bail!("Connection closed after POST {}", path);
        }

        let decrypted = auth::hap_decrypt(&buf[..total], input_key, self.hap_recv_counter)?;
        self.hap_recv_counter += count_hap_frames(&buf[..total]);
        let response = String::from_utf8_lossy(&decrypted);
        let status_line = response.lines().next().unwrap_or("(empty)");
        debug!("POST {} -> {}", path, status_line);

        if !status_line.contains("200") {
            anyhow::bail!("POST {} failed: {}", path, status_line);
        }

        extract_http_body(&decrypted)
    }

    /// Send an RTSP request over the HAP-encrypted channel.
    async fn hap_rtsp_request(
        &mut self,
        conn: &mut TcpStream,
        method: &str,
        output_key: &[u8],
        input_key: &[u8],
        body: &[u8],
    ) -> Result<Vec<u8>> {
        let local_ip = conn.local_addr()?.ip();
        let cseq = self.next_cseq();

        let content_header = if body.is_empty() {
            String::new()
        } else {
            format!(
                "Content-Type: application/x-apple-binary-plist\r\n\
                 Content-Length: {}\r\n",
                body.len()
            )
        };

        let request = format!(
            "{} rtsp://{}/{} RTSP/1.0\r\n\
             CSeq: {}\r\n\
             User-Agent: AirPlay/690.7.1\r\n\
             DACP-ID: AABBCCDDEEFF0011\r\n\
             Active-Remote: 1234567890\r\n\
             Client-Instance: AABBCCDDEEFF0011\r\n\
             {}\
             \r\n",
            method, local_ip, self.session_id, cseq, content_header,
        );

        debug!("RTSP {} request:\n{}", method, request.trim());

        let mut http_data = request.as_bytes().to_vec();
        http_data.extend_from_slice(body);

        let encrypted = auth::hap_encrypt(&http_data, output_key, self.hap_send_counter)?;
        conn.write_all(&encrypted).await?;
        self.hap_send_counter += (http_data.len() as u64 + 1023) / 1024;

        // Read response
        let mut buf = vec![0u8; 65536];
        let mut total = 0;
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_millis(2000),
                conn.read(&mut buf[total..]),
            ).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    total += n;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Ok(Err(e)) => anyhow::bail!("Read error: {}", e),
                Err(_) => break,
            }
        }

        if total == 0 {
            anyhow::bail!("Connection closed after RTSP {}", method);
        }

        let decrypted = auth::hap_decrypt(&buf[..total], input_key, self.hap_recv_counter)?;
        self.hap_recv_counter += count_hap_frames(&buf[..total]);
        let response = String::from_utf8_lossy(&decrypted);
        let status_line = response.lines().next().unwrap_or("(empty)");
        info!("RTSP {} -> {}", method, status_line);

        if !status_line.contains("200") {
            debug!("Full {} response:\n{}", method, response);
            anyhow::bail!("RTSP {} failed: {}", method, status_line);
        }

        extract_http_body(&decrypted)
    }

    /// Keep the AirPlay session alive until signalled to stop.
    /// For HLS URL playback, Apple TV fetches segments from our HTTP server.
    /// We just monitor the control connection for status/events.
    pub async fn run(&mut self, mut conn: TcpStream) -> Result<()> {
        info!("HLS playback active — Apple TV will fetch segments from HTTP server");
        info!("Press Ctrl+C to stop");

        loop {
            let mut buf = vec![0u8; 4096];
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                conn.read(&mut buf),
            )
            .await
            {
                Ok(Ok(0)) => {
                    info!("Apple TV closed connection");
                    break;
                }
                Ok(Ok(n)) => {
                    debug!("Received {} bytes from Apple TV", n);
                }
                Ok(Err(e)) => {
                    warn!("Connection error: {}", e);
                    break;
                }
                Err(_) => {
                    // Timeout — send NOP/keepalive if needed
                    debug!("Keepalive tick");
                }
            }
        }

        Ok(())
    }
}

fn extract_http_body(response: &[u8]) -> Result<Vec<u8>> {
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("No HTTP header separator found")?;
    Ok(response[header_end + 4..].to_vec())
}

fn plist_to_binary(value: &plist::Value) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    value.to_writer_binary(&mut buf)?;
    Ok(buf)
}

fn uuid_v4() -> String {
    let bytes: [u8; 16] = rand::random();
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        (bytes[6] & 0x0F) | 0x40, bytes[7],
        (bytes[8] & 0x3F) | 0x80, bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

fn count_hap_frames(data: &[u8]) -> u64 {
    let mut count = 0u64;
    let mut offset = 0;
    while offset + 2 < data.len() {
        let length = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        let frame_total = 2 + length + 16;
        if offset + frame_total > data.len() {
            break;
        }
        count += 1;
        offset += frame_total;
    }
    count
}
