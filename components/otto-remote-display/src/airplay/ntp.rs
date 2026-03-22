use anyhow::Result;
use tokio::net::UdpSocket;
use tracing::{debug, info};

/// Seconds from 1900-01-01 to 1970-01-01.
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

/// Start an NTP responder on UDP port 7010 for AirPlay clock sync.
///
/// Apple TV sends NTP-style timing requests every few seconds.
/// We respond with the current wall clock timestamps so it can synchronize
/// the video stream timing.
pub async fn start_ntp_server() -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:7010").await?;
    info!("NTP server listening on :7010");

    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    debug!("NTP request from {} ({} bytes)", addr, len);

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap();
                    let ntp_secs = (now.as_secs() + NTP_EPOCH_OFFSET) as u32;
                    let ntp_frac = ((now.subsec_nanos() as u64) << 32) / 1_000_000_000;

                    let mut response = [0u8; 32];
                    // Copy origin timestamp from request (offset 16..24 → 0..8)
                    if len >= 24 {
                        response[0..8].copy_from_slice(&buf[16..24]);
                    }
                    // Receive timestamp (offset 8..16)
                    response[8..12].copy_from_slice(&ntp_secs.to_be_bytes());
                    response[12..16].copy_from_slice(&(ntp_frac as u32).to_be_bytes());
                    // Transmit timestamp (offset 16..24)
                    response[16..20].copy_from_slice(&ntp_secs.to_be_bytes());
                    response[20..24].copy_from_slice(&(ntp_frac as u32).to_be_bytes());

                    if let Err(e) = socket.send_to(&response, addr).await {
                        debug!("NTP send error: {}", e);
                    }
                }
                Err(e) => {
                    debug!("NTP recv error: {}", e);
                }
            }
        }
    });

    Ok(())
}

/// Convert elapsed Duration since session start to a 64-bit NTP-style timestamp.
/// Upper 32 bits = whole seconds, lower 32 bits = fractional seconds.
pub fn elapsed_to_ntp(elapsed: std::time::Duration) -> u64 {
    let secs = elapsed.as_secs() as u32;
    let frac = ((elapsed.subsec_nanos() as u64) << 32) / 1_000_000_000;
    ((secs as u64) << 32) | (frac & 0xFFFF_FFFF)
}
