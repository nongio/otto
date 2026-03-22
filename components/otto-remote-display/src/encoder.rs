use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::mpsc;
use tracing::info;

pub struct EncoderPipeline {
    pipeline: gst::Pipeline,
    encoder_name: String,
}

pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub pts: u64,
    pub is_keyframe: bool,
}

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
        "nvh264enc" => format!("nvh264enc bitrate={bitrate_kbps} preset=low-latency-hq rc-mode=cbr"),
        "x264enc" => format!("x264enc bitrate={bitrate_kbps} tune=zerolatency speed-preset=ultrafast"),
        _ => unreachable!(),
    }
}

impl EncoderPipeline {
    pub fn new(
        pipewire_node_id: u32,
        fps: u32,
        bitrate_kbps: u32,
    ) -> Result<(Self, mpsc::Receiver<EncodedFrame>)> {
        gst::init()?;

        let encoder_name = find_encoder()?;
        let encoder_element = encoder_props(encoder_name, bitrate_kbps);

        let pipeline_str = format!(
            "pipewiresrc path={node_id} do-timestamp=true \
             ! videoconvert \
             ! video/x-raw,framerate={fps}/1 \
             ! {encoder} \
             ! video/x-h264,stream-format=byte-stream,alignment=au \
             ! appsink name=sink emit-signals=true sync=false",
            node_id = pipewire_node_id,
            fps = fps,
            encoder = encoder_element,
        );

        info!("GStreamer pipeline: {}", pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| anyhow::anyhow!("Failed to cast to Pipeline"))?;

        let appsink = pipeline
            .by_name("sink")
            .context("No appsink in pipeline")?
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| anyhow::anyhow!("Failed to cast to AppSink"))?;

        let (tx, rx) = mpsc::channel();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    let pts = buffer.pts().map(|p| p.nseconds()).unwrap_or(0);
                    let is_keyframe = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);

                    let frame = EncodedFrame {
                        data: map.to_vec(),
                        pts,
                        is_keyframe,
                    };

                    let _ = tx.send(frame);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        Ok((
            Self {
                pipeline,
                encoder_name: encoder_name.to_string(),
            },
            rx,
        ))
    }

    pub fn encoder_name(&self) -> &str {
        &self.encoder_name
    }

    pub fn start(&self) -> Result<()> {
        self.pipeline.set_state(gst::State::Playing)?;
        info!("Encoder pipeline started");
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.pipeline.set_state(gst::State::Null)?;
        info!("Encoder pipeline stopped");
        Ok(())
    }
}
