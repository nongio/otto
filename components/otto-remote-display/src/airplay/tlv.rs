/// TLV8 encoding/decoding for HAP pair-verify protocol.

use anyhow::{Context, Result};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TlvTag {
    Method = 0,
    Identifier = 1,
    Salt = 2,
    PublicKey = 3,
    Proof = 4,
    EncryptedData = 5,
    SeqNo = 6,
    Error = 7,
    Signature = 10,
    Flags = 19,
}

impl TlvTag {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Method),
            1 => Some(Self::Identifier),
            2 => Some(Self::Salt),
            3 => Some(Self::PublicKey),
            4 => Some(Self::Proof),
            5 => Some(Self::EncryptedData),
            6 => Some(Self::SeqNo),
            7 => Some(Self::Error),
            10 => Some(Self::Signature),
            19 => Some(Self::Flags),
            _ => None,
        }
    }
}

pub struct TlvMap {
    entries: Vec<(TlvTag, Vec<u8>)>,
}

impl TlvMap {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn with(mut self, tag: TlvTag, value: Vec<u8>) -> Self {
        self.entries.push((tag, value));
        self
    }

    pub fn get(&self, tag: TlvTag) -> Option<&Vec<u8>> {
        self.entries.iter().find(|(t, _)| *t == tag).map(|(_, v)| v)
    }

    /// Encode to TLV8 bytes. Values > 255 bytes are split into 255-byte chunks.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (tag, value) in &self.entries {
            let tag_byte = *tag as u8;
            let mut pos = 0;
            let mut remaining = value.len();

            if remaining == 0 {
                out.push(tag_byte);
                out.push(0);
            } else {
                while pos < value.len() {
                    let chunk = remaining.min(255);
                    out.push(tag_byte);
                    out.push(chunk as u8);
                    out.extend_from_slice(&value[pos..pos + chunk]);
                    pos += chunk;
                    remaining -= chunk;
                }
            }
        }
        out
    }

    /// Decode TLV8 bytes. Consecutive entries with the same tag are concatenated.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut map: HashMap<u8, Vec<u8>> = HashMap::new();
        let mut order: Vec<u8> = Vec::new();
        let mut pos = 0;

        while pos + 1 < data.len() {
            let tag = data[pos];
            let length = data[pos + 1] as usize;
            pos += 2;

            if pos + length > data.len() {
                anyhow::bail!(
                    "TLV truncated: tag={}, length={}, remaining={}",
                    tag,
                    length,
                    data.len() - pos
                );
            }

            let entry = map.entry(tag).or_default();
            if entry.is_empty() {
                order.push(tag);
            }
            entry.extend_from_slice(&data[pos..pos + length]);
            pos += length;
        }

        let mut entries = Vec::new();
        for tag_byte in order {
            if let Some(tlv_tag) = TlvTag::from_u8(tag_byte) {
                if let Some(value) = map.remove(&tag_byte) {
                    entries.push((tlv_tag, value));
                }
            }
        }

        Ok(Self { entries })
    }
}
