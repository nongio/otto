use anyhow::Result;

/// AirPlay mirroring packet types (Section 6 of AirPlay spec)
pub const PACKET_TYPE_VIDEO: u16 = 0;
pub const PACKET_TYPE_CODEC_DATA: u16 = 1;
pub const PACKET_TYPE_HEARTBEAT: u16 = 2;

/// Build the 128-byte packet header for the AirPlay mirroring stream.
///
/// Layout (big-endian):
///   [0..4]   payload_size (u32)
///   [4..6]   payload_type (u16)
///   [6..8]   reserved
///   [8..16]  ntp_timestamp (u64)
///   [16..128] reserved (zeros)
pub fn build_packet_header(ptype: u16, payload_size: usize, ntp_time: u64) -> [u8; 128] {
    let mut header = [0u8; 128];

    header[0..4].copy_from_slice(&(payload_size as u32).to_be_bytes());
    header[4..6].copy_from_slice(&ptype.to_be_bytes());
    header[8..16].copy_from_slice(&ntp_time.to_be_bytes());

    header
}

/// Build the binary plist body for POST /stream (no FairPlay auth).
pub fn build_stream_plist() -> Result<Vec<u8>> {
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "deviceID".to_string(),
        plist::Value::String("AA:BB:CC:DD:EE:FF".to_string()),
    );
    dict.insert(
        "sessionID".to_string(),
        plist::Value::Integer(1.into()),
    );
    dict.insert(
        "version".to_string(),
        plist::Value::String("390.9.1".to_string()),
    );
    dict.insert(
        "latencyMs".to_string(),
        plist::Value::Integer(90.into()),
    );

    let value = plist::Value::Dictionary(dict);
    let mut buf = Vec::new();
    value.to_writer_binary(&mut buf)?;
    Ok(buf)
}

/// Extract SPS/PPS NALUs from an Annex B byte stream to send as codec data.
pub fn extract_codec_data(nalu_stream: &[u8]) -> Option<Vec<u8>> {
    let mut codec_data = Vec::new();
    let nalus = split_nalus(nalu_stream);

    for (start, end) in &nalus {
        if *start < nalu_stream.len() {
            let nalu_type = nalu_stream[*start] & 0x1F;
            // SPS = 7, PPS = 8
            if nalu_type == 7 || nalu_type == 8 {
                // Include the start code prefix
                codec_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                codec_data.extend_from_slice(&nalu_stream[*start..*end]);
            }
        }
    }

    if codec_data.is_empty() {
        None
    } else {
        Some(codec_data)
    }
}

/// Split an Annex B byte stream into NALU boundaries.
/// Returns Vec of (nalu_data_start, nalu_data_end) — after the start code.
fn split_nalus(data: &[u8]) -> Vec<(usize, usize)> {
    let mut nalus = Vec::new();
    let mut i = 0;

    while i + 2 < data.len() {
        let (sc_len, found) = if i + 3 < data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            (4, true)
        } else if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            (3, true)
        } else {
            (0, false)
        };

        if found {
            let nalu_start = i + sc_len;
            // Close the previous NALU
            if let Some(last) = nalus.last_mut() {
                let prev: &mut (usize, usize) = last;
                prev.1 = i;
            }
            nalus.push((nalu_start, data.len()));
            i = nalu_start;
        } else {
            i += 1;
        }
    }

    nalus
}
