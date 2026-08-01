//! Stream metadata types for portal responses.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Str, Value};

use crate::portal::SOURCE_TYPE_MONITOR;

/// Describes a PipeWire stream for the portal response.
#[derive(Clone, Debug)]
pub struct StreamDescriptor {
    /// PipeWire node ID.
    pub node_id: u32,
    /// Unique identifier for this stream within the session.
    pub stream_id: String,
    /// Mapping ID for correlating with compositor outputs.
    pub mapping_id: Option<String>,
    /// Stream width in pixels.
    pub width: Option<u32>,
    /// Stream height in pixels.
    pub height: Option<u32>,
    /// Logical position in compositor space.
    pub position: Option<(i32, i32)>,
    /// Scale factor for HiDPI outputs.
    pub scale_factor: Option<f64>,
    /// Refresh rate in millihertz.
    pub refresh_millihz: Option<u32>,
    /// Buffer stride in bytes.
    pub stride: Option<u32>,
    /// FourCC pixel format code.
    pub fourcc: Option<u32>,
    /// DRM format modifier.
    pub modifier: Option<u64>,
    /// Buffer type (e.g., "DMA", "SHM").
    pub buffer_kind: Option<String>,
    /// Portal source type bit for this stream (monitor or window).
    pub source_type: u32,
}

impl Default for StreamDescriptor {
    fn default() -> Self {
        Self {
            node_id: 0,
            stream_id: String::new(),
            mapping_id: None,
            width: None,
            height: None,
            position: None,
            scale_factor: None,
            refresh_millihz: None,
            stride: None,
            fourcc: None,
            modifier: None,
            buffer_kind: None,
            source_type: SOURCE_TYPE_MONITOR,
        }
    }
}

/// Converts stream descriptors to the D-Bus vardict format.
///
/// Returns `a(ua{sv})` - array of (node_id, properties) tuples.
pub fn build_streams_value_from_descriptors(
    descriptors: &[StreamDescriptor],
) -> zbus::Result<OwnedValue> {
    let mut entries: Vec<(u32, HashMap<String, OwnedValue>)> =
        Vec::with_capacity(descriptors.len());

    for descriptor in descriptors {
        let mut dict: HashMap<String, OwnedValue> = HashMap::new();
        dict.insert(
            "source_type".to_string(),
            OwnedValue::from(descriptor.source_type),
        );
        dict.insert(
            "id".to_string(),
            OwnedValue::from(Str::from(descriptor.stream_id.clone())),
        );

        if let Some(mapping_id) = &descriptor.mapping_id {
            dict.insert(
                "mapping_id".to_string(),
                OwnedValue::from(Str::from(mapping_id.clone())),
            );
        }

        if let Some((x, y)) = descriptor.position {
            let value = OwnedValue::try_from(Value::new((x, y)))?;
            dict.insert("position".to_string(), value);
        }

        if let (Some(w), Some(h)) = (descriptor.width, descriptor.height) {
            let value = OwnedValue::try_from(Value::new((w as i32, h as i32)))?;
            dict.insert("size".to_string(), value);
        }

        if let Some(scale) = descriptor.scale_factor {
            dict.insert("scale-factor".to_string(), OwnedValue::from(scale));
        }

        if let Some(refresh) = descriptor.refresh_millihz {
            dict.insert("refresh-millihz".to_string(), OwnedValue::from(refresh));
        }

        if let Some(stride) = descriptor.stride {
            dict.insert("stride".to_string(), OwnedValue::from(stride));
        }

        if let Some(fourcc) = descriptor.fourcc {
            dict.insert("fourcc".to_string(), OwnedValue::from(fourcc));
        }

        if let Some(modifier) = descriptor.modifier {
            dict.insert("modifier".to_string(), OwnedValue::from(modifier));
        }

        if let Some(kind) = &descriptor.buffer_kind {
            dict.insert(
                "buffer-kind".to_string(),
                OwnedValue::from(Str::from(kind.clone())),
            );
        }

        entries.push((descriptor.node_id, dict));
    }

    OwnedValue::try_from(Value::new(entries)).map_err(Into::into)
}
