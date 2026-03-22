//! RDP server for otto-remote-display.
//!
//! Consumes PipeWire DMA-BUF frames directly (no GStreamer), following GNOME
//! Remote Desktop's approach, and serves them over RDP via ironrdp-server.
//!
//! ## Testing
//!
//! 1. Start Otto in windowed mode:
//!    ```sh
//!    cargo run -- --winit
//!    ```
//!
//! 2. In another terminal, start the RDP server:
//!    ```sh
//!    cargo run -p otto-remote-display -- --protocol rdp --port 3389
//!    ```
//!    Add `RUST_LOG=debug` for verbose output.
//!
//! 3. Connect with any RDP client:
//!    - **FreeRDP**: `xfreerdp /v:localhost:3389 /cert:ignore`
//!    - **Remmina**: New connection → RDP → Server: `localhost:3389`
//!    - **Windows**: `mstsc /v:localhost:3389`
//!
//!    Use `/cert:ignore` or accept the self-signed certificate warning.

use std::io::Cursor;
use std::num::{NonZeroU16, NonZeroUsize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
use ironrdp_server::tokio_rustls::{rustls, TlsAcceptor};
use ironrdp_server::{
    BitmapUpdate, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
};
use pipewire as pw;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::Value;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// DMA-BUF ioctl constants
// ---------------------------------------------------------------------------

const DMA_BUF_SYNC_READ: u64 = 1;
const DMA_BUF_SYNC_START: u64 = 0;
const DMA_BUF_SYNC_END: u64 = 4;
// _IOW('b', 0, struct dma_buf_sync) = (1<<30) | (0x62<<8) | (8<<16)
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x40086200;

unsafe fn dmabuf_sync(fd: i32, flags: u64) {
    libc::ioctl(fd, DMA_BUF_IOCTL_SYNC as libc::c_ulong, &flags as *const u64);
}

// ---------------------------------------------------------------------------
// TLS setup
// ---------------------------------------------------------------------------

fn make_tls_acceptor() -> Result<TlsAcceptor> {
    let subject_alt_names = vec!["localhost".to_string()];
    let cert = generate_simple_self_signed(subject_alt_names)
        .context("Failed to generate self-signed certificate")?;

    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .context("bad certificate/key")?;

    server_config.key_log = Arc::new(rustls::KeyLogFile::new());

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

// ---------------------------------------------------------------------------
// PipeWire frame capture (direct, no GStreamer)
// ---------------------------------------------------------------------------

struct RawFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
}

/// State shared between PipeWire stream callbacks.
struct CaptureCallbackData {
    tx: mpsc::UnboundedSender<RawFrame>,
    resolution_tx: Option<std::sync::mpsc::SyncSender<(u32, u32)>>,
    width: u32,
    height: u32,
}

/// Build SPA format params that accept BGRA with DMA-BUF (LINEAR modifier)
/// and a MemFd fallback, following GNOME Remote Desktop's approach.
fn build_consumer_format_params() -> Vec<Vec<u8>> {
    use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use pw::spa::param::ParamType;
    use pw::spa::pod::{ChoiceValue, Property, PropertyFlags, Value as PodValue};
    use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id, Rectangle, SpaTypes};

    let video_format = pw::spa::param::video::VideoFormat::BGRA;
    let mut params: Vec<Vec<u8>> = Vec::new();

    // 1) DMA-BUF format with LINEAR modifier (matches Otto's producer)
    let dmabuf_props = vec![
        Property {
            key: FormatProperties::MediaType.as_raw(),
            flags: PropertyFlags::empty(),
            value: PodValue::Id(Id(MediaType::Video.as_raw())),
        },
        Property {
            key: FormatProperties::MediaSubtype.as_raw(),
            flags: PropertyFlags::empty(),
            value: PodValue::Id(Id(MediaSubtype::Raw.as_raw())),
        },
        Property {
            key: FormatProperties::VideoFormat.as_raw(),
            flags: PropertyFlags::empty(),
            value: PodValue::Id(Id(video_format.as_raw())),
        },
        Property {
            key: FormatProperties::VideoModifier.as_raw(),
            flags: PropertyFlags::MANDATORY,
            value: PodValue::Choice(ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum {
                    default: 0i64, // DRM_FORMAT_MOD_LINEAR
                    alternatives: vec![0i64],
                },
            ))),
        },
    ];

    let dmabuf_obj = pw::spa::pod::Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: dmabuf_props,
    };

    if let Ok(serialized) =
        PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(dmabuf_obj))
    {
        params.push(serialized.0.into_inner());
    }

    // 2) SHM fallback (no modifier) — consumer-side only; the producer will
    //    ignore this if it only advertises DMA-BUF, but it's harmless to offer.
    let shm_format = pw::spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pw::spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pw::spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pw::spa::pod::property!(FormatProperties::VideoFormat, Id, video_format),
    );

    if let Ok(serialized) =
        PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(shm_format))
    {
        params.push(serialized.0.into_inner());
    }

    params
}

/// Extract width and height from a negotiated SPA format pod.
fn parse_video_size_from_pod(pod: &pw::spa::pod::Pod) -> Option<(u32, u32)> {
    use pw::spa::param::format::FormatProperties;

    let bytes = pod.as_bytes();
    let (_, value) = PodDeserializer::deserialize_from::<Value>(bytes).ok()?;

    if let Value::Object(obj) = value {
        for prop in &obj.properties {
            if prop.key == FormatProperties::VideoSize.as_raw() {
                if let Value::Rectangle(rect) = &prop.value {
                    return Some((rect.width, rect.height));
                }
            }
        }
    }
    None
}

/// Read a single DMA-BUF frame: sync → mmap → copy → munmap → end-sync.
unsafe fn read_dmabuf_frame(
    fd: i32,
    mapoffset: u32,
    maxsize: u32,
    chunk_offset: u32,
    chunk_size: u32,
) -> Option<Vec<u8>> {
    dmabuf_sync(fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_START);

    let ptr = libc::mmap(
        std::ptr::null_mut(),
        maxsize as usize,
        libc::PROT_READ,
        libc::MAP_SHARED,
        fd,
        mapoffset as libc::off_t,
    );

    if ptr == libc::MAP_FAILED {
        dmabuf_sync(fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_END);
        return None;
    }

    let src = std::slice::from_raw_parts(
        (ptr as *const u8).add(chunk_offset as usize),
        chunk_size as usize,
    );
    let data = src.to_vec();

    libc::munmap(ptr, maxsize as usize);
    dmabuf_sync(fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_END);

    Some(data)
}

/// Spawn a PipeWire main-loop thread that consumes frames from `node_id` and
/// sends them over the returned channel. Also returns the detected resolution
/// (blocks until the stream negotiates format).
fn start_pipewire_capture(
    node_id: u32,
    stop: Arc<AtomicBool>,
) -> Result<(mpsc::UnboundedReceiver<RawFrame>, u32, u32)> {
    let (frame_tx, frame_rx) = mpsc::unbounded_channel();
    // Bounded(1) so we block the PW thread until resolution is read exactly once
    let (res_tx, res_rx) = std::sync::mpsc::sync_channel::<(u32, u32)>(1);

    let _handle = std::thread::Builder::new()
        .name("pw-capture".into())
        .spawn(move || {
            pw::init();

            let mainloop = pw::main_loop::MainLoop::new(None).expect("pw MainLoop");
            let context = pw::context::Context::new(&mainloop).expect("pw Context");
            let core = context.connect(None).expect("pw Core::connect");

            let stream = pw::stream::Stream::new(
                &core,
                "otto-rdp-capture",
                pw::properties! {
                    *pw::keys::MEDIA_TYPE => "Video",
                    *pw::keys::MEDIA_CATEGORY => "Capture",
                    *pw::keys::MEDIA_ROLE => "Screen",
                },
            )
            .expect("pw Stream::new");

            let _listener = stream
                .add_local_listener_with_user_data(CaptureCallbackData {
                    tx: frame_tx,
                    resolution_tx: Some(res_tx),
                    width: 0,
                    height: 0,
                })
                .state_changed(|_stream, _data, old, new| {
                    info!("PipeWire stream state: {:?} → {:?}", old, new);
                })
                .param_changed(|_stream, data, id, param| {
                    if id != pw::spa::param::ParamType::Format.as_raw() {
                        return;
                    }
                    let Some(pod) = param else { return };

                    if let Some((w, h)) = parse_video_size_from_pod(pod) {
                        info!("Negotiated resolution: {}×{}", w, h);
                        data.width = w;
                        data.height = h;
                        // Signal resolution to the waiting main thread (once)
                        if let Some(tx) = data.resolution_tx.take() {
                            let _ = tx.send((w, h));
                        }
                    }
                })
                .process(|stream, data| {
                    let Some(mut buffer) = stream.dequeue_buffer() else {
                        return;
                    };

                    let datas = buffer.datas_mut();
                    let Some(d) = datas.first_mut() else { return };

                    let chunk = d.chunk();
                    let stride = chunk.stride() as usize;
                    let size = chunk.size() as usize;

                    if size == 0 || stride == 0 || data.width == 0 {
                        return;
                    }

                    let frame_data = match d.type_() {
                        pw::spa::buffer::DataType::DmaBuf => {
                            let raw = d.as_raw();
                            unsafe {
                                read_dmabuf_frame(
                                    raw.fd as i32,
                                    raw.mapoffset,
                                    raw.maxsize,
                                    chunk.offset(),
                                    chunk.size(),
                                )
                            }
                        }
                        pw::spa::buffer::DataType::MemFd | pw::spa::buffer::DataType::MemPtr => {
                            d.data().map(|mapped| {
                                let off = chunk.offset() as usize;
                                mapped[off..off + size].to_vec()
                            })
                        }
                        other => {
                            warn!("Unsupported buffer type: {:?}", other);
                            None
                        }
                    };

                    if let Some(pixels) = frame_data {
                        let _ = data.tx.send(RawFrame {
                            data: pixels,
                            width: data.width,
                            height: data.height,
                            stride,
                        });
                    }
                })
                .register()
                .expect("pw stream listener");

            // Build and connect
            let format_bytes = build_consumer_format_params();
            let mut format_pods: Vec<&pw::spa::pod::Pod> = format_bytes
                .iter()
                .map(|b| pw::spa::pod::Pod::from_bytes(b).unwrap())
                .collect();

            stream
                .connect(
                    pw::spa::utils::Direction::Input,
                    Some(node_id),
                    pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                    &mut format_pods,
                )
                .expect("pw stream connect");

            info!("PipeWire capture stream connected to node {}", node_id);

            let loop_ref = mainloop.loop_();
            while !stop.load(Ordering::Relaxed) {
                loop_ref.iterate(std::time::Duration::from_millis(16));
            }

            info!("PipeWire capture thread exiting");
        })
        .context("failed to spawn PipeWire capture thread")?;

    // Wait for the stream to negotiate format and report resolution
    let (width, height) = res_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .context("PipeWire stream did not negotiate format within 10s")?;

    Ok((frame_rx, width, height))
}

// ---------------------------------------------------------------------------
// RDP display handler
// ---------------------------------------------------------------------------

struct OttoDisplayUpdates {
    rx: mpsc::UnboundedReceiver<RawFrame>,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for OttoDisplayUpdates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        match self.rx.recv().await {
            Some(frame) => {
                let w = NonZeroU16::new(frame.width as u16).context("zero width")?;
                let h = NonZeroU16::new(frame.height as u16).context("zero height")?;
                let stride = NonZeroUsize::new(frame.stride).context("zero stride")?;

                // Otto produces BGRA; that maps to BgrA32 in ironrdp
                let update = BitmapUpdate {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                    format: PixelFormat::BgrA32,
                    data: Bytes::from(frame.data),
                    stride,
                };

                Ok(Some(DisplayUpdate::Bitmap(update)))
            }
            None => Ok(None),
        }
    }
}

struct OttoDisplay {
    width: u16,
    height: u16,
    capture_rx: Option<mpsc::UnboundedReceiver<RawFrame>>,
}

#[async_trait::async_trait]
impl RdpServerDisplay for OttoDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: self.width,
            height: self.height,
        }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        let rx = self
            .capture_rx
            .take()
            .context("display updates already consumed")?;
        Ok(Box::new(OttoDisplayUpdates { rx }))
    }
}

// ---------------------------------------------------------------------------
// RDP input handler (logging stub)
// ---------------------------------------------------------------------------

struct InputHandler;

impl RdpServerInputHandler for InputHandler {
    fn keyboard(&mut self, event: KeyboardEvent) {
        debug!(?event, "RDP keyboard event");
    }

    fn mouse(&mut self, event: MouseEvent) {
        debug!(?event, "RDP mouse event");
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the RDP server, streaming raw frames from the given PipeWire node.
pub async fn run_server(port: u16, node_id: u32, _fps: u32) -> Result<()> {
    let tls_acceptor = make_tls_acceptor()?;
    info!("Generated self-signed TLS certificate");

    let stop = Arc::new(AtomicBool::new(false));
    let (rx, width, height) = start_pipewire_capture(node_id, stop.clone())?;

    let display = OttoDisplay {
        width: width as u16,
        height: height as u16,
        capture_rx: Some(rx),
    };

    info!(
        "Starting RDP server on 0.0.0.0:{} ({}×{})",
        port, width, height
    );

    let mut server = RdpServer::builder()
        .with_addr(([0, 0, 0, 0], port))
        .with_tls(tls_acceptor)
        .with_input_handler(InputHandler)
        .with_display_handler(display)
        .build();

    let result = server.run().await;

    stop.store(true, Ordering::Relaxed);
    info!("RDP server stopped");

    result
}
