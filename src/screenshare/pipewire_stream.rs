//! PipeWire stream management for screencast.
//!
//! Format negotiation-first approach: advertise capabilities based on backend,
//! negotiate format, then route to appropriate buffer handling path.

use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

use smithay::backend::allocator::{
    dmabuf::{AsDmabuf, Dmabuf},
    gbm::GbmDevice,
    Fourcc,
};
use smithay::backend::drm::DrmDeviceFd;

/// Get current monotonic time in nanoseconds
fn get_monotonic_time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

/// Buffer pool shared between PipeWire thread and main thread
#[derive(Default)]
pub struct BufferPool {
    /// Allocated dmabufs keyed by fd
    pub dmabufs: HashMap<i64, Dmabuf>,
    /// Buffers available for rendering (dequeued from PipeWire)
    pub available: VecDeque<AvailableBuffer>,
    /// Raw PW buffer pointers to queue back (keyed by fd)
    pub to_queue: HashMap<i64, *mut pipewire::sys::pw_buffer>,
    /// Track last rendered buffer FD to detect buffer changes
    pub last_rendered_fd: Option<i64>,
}

// SAFETY: pw_buffer pointers are only accessed from PipeWire thread
unsafe impl Send for BufferPool {}
unsafe impl Sync for BufferPool {}

pub struct AvailableBuffer {
    pub fd: i64,
    pub dmabuf: Dmabuf,
    pub pw_buffer: *mut pipewire::sys::pw_buffer,
}

/// Backend capabilities for format negotiation.
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    /// Whether the backend can provide DMA-BUF buffers.
    pub supports_dmabuf: bool,
    /// Available pixel formats (as FourCC codes).
    pub formats: Vec<Fourcc>,
    /// Available modifiers (for DMA-BUF) - stored as i64 for PipeWire compatibility.
    pub modifiers: Vec<i64>,
}

impl Default for BackendCapabilities {
    fn default() -> Self {
        // Default: SHM-only with ARGB8888
        Self {
            supports_dmabuf: false,
            formats: vec![Fourcc::Argb8888],
            modifiers: vec![],
        }
    }
}

/// Configuration for a PipeWire stream.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Output dimensions
    pub width: u32,
    pub height: u32,
    /// Target framerate
    pub framerate_num: u32,
    pub framerate_denom: u32,
    /// Backend capabilities (determines what we advertise)
    pub capabilities: BackendCapabilities,
    /// GBM device (if backend supports DMA-BUF)
    pub gbm_device: Option<GbmDevice<DrmDeviceFd>>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            framerate_num: 60,
            framerate_denom: 1,
            capabilities: BackendCapabilities::default(),
            gbm_device: None,
        }
    }
}

/// Negotiated format information.
#[derive(Debug, Clone)]
pub struct NegotiatedFormat {
    /// Video format (BGRx, RGBA, etc.)
    pub format: pipewire::spa::param::video::VideoFormat,
    /// Pixel dimensions
    pub size: (u32, u32),
    /// Framerate
    pub framerate: (u32, u32),
    /// Whether this is a DMA-BUF format (has modifier)
    pub is_dmabuf: bool,
    /// DRM modifier (if DMA-BUF) - stored as i64 for PipeWire compatibility
    pub modifier: Option<i64>,
}

/// Shared state between threads.
struct SharedState {
    node_id: AtomicU32,
    active: AtomicBool,
    should_stop: AtomicBool,
    /// Buffer pool shared with main thread for blitting
    buffer_pool: Arc<Mutex<BufferPool>>,
    /// Raw stream pointer for triggering from main thread
    stream_ptr: Arc<Mutex<Option<*mut pipewire::sys::pw_stream>>>,
    /// Frame sequence counter for actual rendered frames (shared between threads)
    frame_sequence: AtomicU64,
    /// Start time for calculating PTS (nanoseconds since CLOCK_MONOTONIC)
    start_time_ns: AtomicU64,
}

// SAFETY: pw_stream pointer is only used to call pw_stream_trigger_process
unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

/// Stream state for the PipeWire thread.
struct PwStreamState {
    /// Current negotiation status
    negotiated: Option<NegotiatedFormat>,
    /// DMA-BUF buffers indexed by fd
    dmabufs: HashMap<i64, Dmabuf>,
}

/// A PipeWire stream for screen casting.
pub struct PipeWireStream {
    shared: Arc<SharedState>,
    config: StreamConfig,
}

impl PipeWireStream {
    /// Create a new PipeWire stream.
    pub fn new(config: StreamConfig) -> Self {
        let shared = Arc::new(SharedState {
            node_id: AtomicU32::new(0),
            active: AtomicBool::new(false),
            should_stop: AtomicBool::new(false),
            buffer_pool: Arc::new(Mutex::new(BufferPool::default())),
            #[allow(clippy::arc_with_non_send_sync)]
            stream_ptr: Arc::new(Mutex::new(None)),
            frame_sequence: AtomicU64::new(0),
            start_time_ns: AtomicU64::new(0),
        });

        Self { shared, config }
    }

    /// Start the PipeWire stream synchronously.
    pub fn start_sync(&mut self) -> Result<u32, PipeWireError> {
        if self.shared.active.load(Ordering::SeqCst) {
            return Err(PipeWireError::AlreadyActive);
        }

        tracing::debug!(
            "Starting PipeWire stream: {}x{} @ {}/{}fps, backend: {} formats, dmabuf={}",
            self.config.width,
            self.config.height,
            self.config.framerate_num,
            self.config.framerate_denom,
            self.config.capabilities.formats.len(),
            self.config.capabilities.supports_dmabuf
        );

        let config = self.config.clone();
        let shared = self.shared.clone();

        // Channel for initialization result
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();

        // Spawn PipeWire thread
        let _handle = std::thread::spawn(move || {
            if let Err(e) = run_pipewire_thread(config, shared.clone(), ready_tx) {
                tracing::error!("PipeWire thread error: {}", e);
            }
            shared.active.store(false, Ordering::SeqCst);
        });

        // Wait for stream to be ready
        let node_id = ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| PipeWireError::InitFailed(format!("PipeWire init timeout: {}", e)))??;

        self.shared.node_id.store(node_id, Ordering::SeqCst);
        self.shared.active.store(true, Ordering::SeqCst);

        tracing::debug!("PipeWire stream started, node_id={}", node_id);
        Ok(node_id)
    }

    /// Get the PipeWire node ID.
    pub fn node_id(&self) -> u32 {
        self.shared.node_id.load(Ordering::SeqCst)
    }

    /// Check if the stream is active.
    pub fn is_active(&self) -> bool {
        self.shared.active.load(Ordering::SeqCst)
    }

    /// Get access to the buffer pool for rendering from main thread.
    pub fn buffer_pool(&self) -> Arc<Mutex<BufferPool>> {
        self.shared.buffer_pool.clone()
    }

    /// Trigger the process callback (call after rendering a new frame)
    pub fn trigger_frame(&self) {
        if let Some(ptr) = *self.shared.stream_ptr.lock().unwrap() {
            unsafe {
                pipewire::sys::pw_stream_trigger_process(ptr);
            }
            tracing::trace!("Triggered pw_stream_trigger_process");
        } else {
            tracing::warn!("trigger_frame called but stream_ptr not set");
        }
    }

    /// Increment the frame sequence counter (call when a frame is actually rendered)
    pub fn increment_frame_sequence(&self) {
        self.shared.frame_sequence.fetch_add(1, Ordering::Relaxed);
    }
}

/// PipeWire error types.
#[derive(Debug)]
pub enum PipeWireError {
    NotImplemented,
    InitFailed(String),
    AlreadyActive,
    NotActive,
    StreamError(String),
}

impl std::fmt::Display for PipeWireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotImplemented => write!(f, "Not implemented"),
            Self::InitFailed(msg) => write!(f, "Initialization failed: {}", msg),
            Self::AlreadyActive => write!(f, "Stream already active"),
            Self::NotActive => write!(f, "Stream not active"),
            Self::StreamError(msg) => write!(f, "Stream error: {}", msg),
        }
    }
}

impl std::error::Error for PipeWireError {}

/// Run the PipeWire thread.
fn run_pipewire_thread(
    config: StreamConfig,
    shared: Arc<SharedState>,
    ready_tx: std::sync::mpsc::Sender<Result<u32, PipeWireError>>,
) -> Result<(), PipeWireError> {
    use pipewire as pw;
    use std::cell::RefCell;
    use std::rc::Rc;

    // Initialize PipeWire
    pw::init();

    // Create main loop
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to create mainloop: {}", e)))?;

    // Create context
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to create context: {}", e)))?;

    // Connect to daemon
    let core = context
        .connect_rc(None)
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to connect: {}", e)))?;

    // Create stream
    let stream = pw::stream::StreamRc::new(
        core,
        "otto-screencast",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| PipeWireError::InitFailed(format!("Failed to create stream: {}", e)))?;

    // Stream state
    let stream_state = Rc::new(RefCell::new(PwStreamState {
        negotiated: None,
        dmabufs: HashMap::new(),
    }));

    // Ready signal tracking
    let ready_sent = Rc::new(RefCell::new(false));
    let ready_tx = Rc::new(RefCell::new(Some(ready_tx)));

    // Set up stream listener
    let _listener = stream
        .add_local_listener_with_user_data(stream_state.clone())
        .state_changed({
            let ready_tx = ready_tx.clone();
            let ready_sent = ready_sent.clone();
            move |stream, _state, old, new| {
                use pw::stream::StreamState as PwState;

                tracing::debug!("PipeWire stream state: {:?} -> {:?}", old, new);

                match new {
                    PwState::Paused => {
                        let node_id = stream.node_id();
                        tracing::debug!("Stream paused (ready), node_id={}", node_id);

                        if !*ready_sent.borrow() {
                            *ready_sent.borrow_mut() = true;
                            if let Some(tx) = ready_tx.borrow_mut().take() {
                                let _ = tx.send(Ok(node_id));
                            }
                        }
                    }
                    PwState::Streaming => {
                        tracing::debug!("Stream now streaming");

                        // Trigger first frame render
                        unsafe {
                            use pipewire::sys::pw_stream_trigger_process;
                            pw_stream_trigger_process(stream.as_raw_ptr());
                        }
                        tracing::debug!("Triggered first frame render");
                    }
                    PwState::Error(ref err) => {
                        tracing::error!("Stream error: {}", err);

                        if !*ready_sent.borrow() {
                            *ready_sent.borrow_mut() = true;
                            if let Some(tx) = ready_tx.borrow_mut().take() {
                                let _ = tx.send(Err(PipeWireError::StreamError(err.clone())));
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .param_changed({
            let stream_for_update = stream.clone();
            move |_stream, state, id, param| {
                use pw::spa::param::ParamType;

                let Some(param) = param else { return };
                if id != ParamType::Format.as_raw() {
                    return;
                }

                // Parse the negotiated format
                if let Ok(negotiated) = parse_negotiated_format(param) {
                    tracing::info!(
                        "Format negotiated: {:?} {}x{} @ {}/{}, dmabuf={}, modifier={:?}",
                        negotiated.format,
                        negotiated.size.0,
                        negotiated.size.1,
                        negotiated.framerate.0,
                        negotiated.framerate.1,
                        negotiated.is_dmabuf,
                        negotiated.modifier
                    );

                    state.borrow_mut().negotiated = Some(negotiated.clone());

                    // If dmabuf, send buffer allocation params
                    if negotiated.is_dmabuf {
                        tracing::debug!("Sending buffer allocation params for dmabuf");

                        // Determine plane count based on format
                        let plane_count = match negotiated.format {
                            pipewire::spa::param::video::VideoFormat::BGRA
                            | pipewire::spa::param::video::VideoFormat::BGRx
                            | pipewire::spa::param::video::VideoFormat::RGBA
                            | pipewire::spa::param::video::VideoFormat::RGBx => 1,
                            _ => 1, // Default to 1 plane for unknown formats
                        };

                        if let Err(e) = send_buffer_params(&stream_for_update, plane_count) {
                            tracing::error!("Failed to send buffer params: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("Failed to parse negotiated format");
                }
            }
        })
        .add_buffer({
            let state = stream_state.clone();
            let gbm_device = config.gbm_device.clone();
            let buffer_pool = shared.buffer_pool.clone(); // ADD: Share buffer pool
            move |_stream, _user_data, buffer| {
                let mut state = state.borrow_mut();
                let Some(ref negotiated) = state.negotiated else {
                    tracing::warn!("add_buffer called but no negotiated format");
                    return;
                };

                // Only handle dmabuf buffers
                if !negotiated.is_dmabuf {
                    tracing::debug!("add_buffer called for SHM buffer, skipping");
                    return;
                }

                let Some(ref gbm) = gbm_device else {
                    tracing::error!("add_buffer called but no GBM device");
                    return;
                };

                tracing::debug!(
                    "Allocating dmabuf {}x{}",
                    negotiated.size.0,
                    negotiated.size.1
                );

                // Allocate GBM buffer
                let (width, height) = negotiated.size;
                let fourcc = video_format_to_fourcc(negotiated.format);
                let modifier = negotiated
                    .modifier
                    .map(|m| smithay::backend::allocator::Modifier::from(m as u64))
                    .unwrap_or(smithay::backend::allocator::Modifier::Linear);

                use smithay::backend::allocator::gbm::{GbmBuffer, GbmBufferFlags};
                let buffer_flags = GbmBufferFlags::RENDERING;

                let bo = match gbm.create_buffer_object_with_modifiers2::<()>(
                    width,
                    height,
                    fourcc,
                    std::iter::once(modifier),
                    buffer_flags,
                ) {
                    Ok(bo) => bo,
                    Err(e) => {
                        tracing::error!("Failed to create GBM buffer: {:?}", e);
                        return;
                    }
                };

                let gbm_buffer = GbmBuffer::from_bo(bo, false);
                let dmabuf = match gbm_buffer.export() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("Failed to export dmabuf: {:?}", e);
                        return;
                    }
                };

                let plane_count = dmabuf.num_planes();
                tracing::debug!("Exported dmabuf with {} planes", plane_count);

                unsafe {
                    use pipewire::spa::buffer::DataType;
                    use pipewire::spa::sys::SPA_DATA_FLAG_READWRITE;
                    use std::os::fd::AsRawFd;

                    let spa_buffer = (*buffer).buffer;

                    // Verify plane count matches what PipeWire allocated
                    assert_eq!((*spa_buffer).n_datas as usize, plane_count);

                    for (i, (fd, (stride, offset))) in std::iter::zip(
                        dmabuf.handles(),
                        std::iter::zip(dmabuf.strides(), dmabuf.offsets()),
                    )
                    .enumerate()
                    {
                        let spa_data = (*spa_buffer).datas.add(i);
                        // Verify PipeWire allocated this as a DmaBuf type
                        assert!((*spa_data).type_ & (1 << DataType::DmaBuf.as_raw()) > 0);

                        (*spa_data).type_ = DataType::DmaBuf.as_raw();
                        (*spa_data).maxsize = 1;
                        (*spa_data).fd = fd.as_raw_fd() as i64;
                        (*spa_data).flags = SPA_DATA_FLAG_READWRITE;

                        let chunk = (*spa_data).chunk;
                        (*chunk).stride = stride as i32;
                        (*chunk).offset = offset;

                        tracing::debug!(
                            "Plane {}: fd={}, stride={}, offset={}",
                            i,
                            (*spa_data).fd,
                            stride,
                            offset
                        );
                    }

                    let fd = (*(*spa_buffer).datas).fd;

                    // Store in local state (for remove_buffer)
                    state.dmabufs.insert(fd, dmabuf.clone());

                    // Also store in shared pool (for main thread access)
                    buffer_pool.lock().unwrap().dmabufs.insert(fd, dmabuf);

                    tracing::debug!("Buffer added fd={}", fd);
                }
            }
        })
        .remove_buffer({
            let state = stream_state.clone();
            move |_stream, _user_data, buffer| unsafe {
                let fd = (*(*buffer).buffer).datas.read().fd;
                let removed = state.borrow_mut().dmabufs.remove(&fd);
                if removed.is_some() {
                    tracing::debug!("Buffer removed fd={}", fd);
                }
            }
        })
        .process({
            let _state = stream_state.clone();
            let buffer_pool = shared.buffer_pool.clone();
            let shared_for_process = shared.clone();
            let _gbm_device = config.gbm_device.clone();
            let framerate = (config.framerate_num, config.framerate_denom);
            move |stream, _user_data| {
                use pipewire::sys::pw_stream_dequeue_buffer;
                use pipewire::sys::pw_stream_queue_buffer;

                // 1. Queue any buffers that main thread finished rendering
                {
                    let mut pool = buffer_pool.lock().unwrap();
                    let to_queue: Vec<_> = pool.to_queue.drain().collect();
                    for (fd, pw_buffer) in to_queue {
                        unsafe {
                            let spa_buffer = (*pw_buffer).buffer;
                            let chunk = (*(*spa_buffer).datas).chunk;
                            (*chunk).size = 1;

                            // Set timestamp metadata
                            let meta_header = pipewire::spa::sys::spa_buffer_find_meta_data(
                                spa_buffer,
                                pipewire::spa::sys::SPA_META_Header,
                                std::mem::size_of::<pipewire::spa::sys::spa_meta_header>(),
                            );

                            if !meta_header.is_null() {
                                let header =
                                    meta_header as *mut pipewire::spa::sys::spa_meta_header;

                                // Get current frame sequence and calculate PTS
                                let frame_seq =
                                    shared_for_process.frame_sequence.load(Ordering::Relaxed);
                                let start_time =
                                    shared_for_process.start_time_ns.load(Ordering::Relaxed);

                                // Calculate PTS based on framerate and frame sequence
                                // PTS = start_time + (frame_seq * 1_000_000_000 * framerate_denom) / framerate_num
                                let pts = if start_time == 0 {
                                    // First frame - initialize start time
                                    let now = get_monotonic_time_ns();
                                    shared_for_process
                                        .start_time_ns
                                        .store(now, Ordering::Relaxed);
                                    0
                                } else {
                                    let elapsed_ns =
                                        (frame_seq * 1_000_000_000 * framerate.1 as u64)
                                            / framerate.0 as u64;
                                    elapsed_ns as i64
                                };

                                (*header).pts = pts;
                                (*header).flags = 0;
                                (*header).seq = frame_seq;
                                (*header).dts_offset = 0;

                                tracing::trace!(
                                    "Set metadata for buffer fd={}: pts={}, seq={}",
                                    fd,
                                    pts,
                                    frame_seq
                                );
                            } else {
                                tracing::warn!("No metadata header found for buffer fd={}", fd);
                            }

                            pw_stream_queue_buffer(stream.as_raw_ptr(), pw_buffer);
                        }
                        tracing::trace!("Queued buffer fd={}", fd);
                    }
                }

                // 2. Dequeue all available buffers
                loop {
                    let buffer = unsafe { pw_stream_dequeue_buffer(stream.as_raw_ptr()) };
                    if buffer.is_null() {
                        break;
                    }

                    unsafe {
                        let spa_buffer = (*buffer).buffer;
                        let fd = (*(*spa_buffer).datas).fd;

                        let mut pool = buffer_pool.lock().unwrap();
                        if let Some(dmabuf) = pool.dmabufs.get(&fd).cloned() {
                            pool.available.push_back(AvailableBuffer {
                                fd,
                                dmabuf,
                                pw_buffer: buffer,
                            });
                            tracing::trace!("Buffer fd={} available", fd);
                        } else {
                            tracing::warn!("Unknown buffer fd={}", fd);
                            pw_stream_queue_buffer(stream.as_raw_ptr(), buffer);
                        }
                    }
                }
            }
        })
        .register()
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to register listener: {}", e)))?;

    // Build format parameters based on backend capabilities
    let format_params_bytes = build_format_params(&config)?;

    let mut format_params: Vec<&pipewire::spa::pod::Pod> = format_params_bytes
        .iter()
        .map(|bytes| pipewire::spa::pod::Pod::from_bytes(bytes).unwrap())
        .collect();

    // Connect stream
    // Use DRIVER and ALLOC_BUFFERS like niri
    let flags = pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::ALLOC_BUFFERS;

    tracing::debug!(
        "Connecting stream with flags: {:?}, dmabuf={}",
        flags,
        config.capabilities.supports_dmabuf
    );

    stream
        .connect(
            pw::spa::utils::Direction::Output,
            None,
            flags,
            &mut format_params,
        )
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to connect stream: {}", e)))?;

    // Store stream pointer for triggering from main thread
    *shared.stream_ptr.lock().unwrap() = Some(stream.as_raw_ptr());

    // Run main loop
    let loop_ref = mainloop.loop_();
    while !shared.should_stop.load(Ordering::SeqCst) {
        loop_ref.iterate(std::time::Duration::from_millis(16));
    }

    tracing::debug!("PipeWire thread shutting down");
    Ok(())
}

/// Send buffer allocation parameters to PipeWire stream
fn send_buffer_params(
    stream: &pipewire::stream::StreamRc,
    plane_count: i32,
) -> Result<(), PipeWireError> {
    use pipewire::spa::buffer::DataType;
    use pipewire::spa::param::ParamType;
    use pipewire::spa::pod::serialize::PodSerializer;
    use pipewire::spa::pod::{self, ChoiceValue, Property};
    use pipewire::spa::sys::*;
    use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, SpaTypes};
    use std::io::Cursor;

    // Create Buffers param
    let buffers_param = pod::object!(
        SpaTypes::ObjectParamBuffers,
        ParamType::Buffers,
        Property::new(
            SPA_PARAM_BUFFERS_buffers,
            pod::Value::Choice(ChoiceValue::Int(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Range {
                    default: 1,
                    min: 1,
                    max: 1
                }
            ))),
        ),
        Property::new(SPA_PARAM_BUFFERS_blocks, pod::Value::Int(plane_count)),
        Property::new(
            SPA_PARAM_BUFFERS_dataType,
            pod::Value::Choice(ChoiceValue::Int(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Flags {
                    default: 1 << DataType::DmaBuf.as_raw(),
                    flags: vec![1 << DataType::DmaBuf.as_raw()],
                },
            ))),
        ),
    );

    // Create Meta param for header
    let meta_header_param = pod::object!(
        SpaTypes::ObjectParamMeta,
        ParamType::Meta,
        Property::new(
            SPA_PARAM_META_type,
            pod::Value::Id(pipewire::spa::utils::Id(SPA_META_Header))
        ),
        Property::new(
            SPA_PARAM_META_size,
            pod::Value::Int(std::mem::size_of::<spa_meta_header>() as i32)
        ),
    );

    // Create Meta param for VideoDamage
    let meta_damage_param = pod::object!(
        SpaTypes::ObjectParamMeta,
        ParamType::Meta,
        Property::new(
            SPA_PARAM_META_type,
            pod::Value::Id(pipewire::spa::utils::Id(
                pipewire::spa::sys::SPA_META_VideoDamage
            ))
        ),
        Property::new(
            SPA_PARAM_META_size,
            // Size for spa_meta_region with up to 16 damage rectangles
            pod::Value::Int(
                (std::mem::size_of::<pipewire::spa::sys::spa_meta_region>()
                    + 16 * std::mem::size_of::<pipewire::spa::sys::spa_rectangle>())
                    as i32
            )
        ),
    );

    // Serialize params
    let mut buf1 = Vec::new();
    let mut buf2 = Vec::new();
    let mut buf3 = Vec::new();
    PodSerializer::serialize(Cursor::new(&mut buf1), &pod::Value::Object(buffers_param)).map_err(
        |e| PipeWireError::InitFailed(format!("Failed to serialize buffers param: {:?}", e)),
    )?;
    PodSerializer::serialize(
        Cursor::new(&mut buf2),
        &pod::Value::Object(meta_header_param),
    )
    .map_err(|e| {
        PipeWireError::InitFailed(format!("Failed to serialize meta header param: {:?}", e))
    })?;
    PodSerializer::serialize(
        Cursor::new(&mut buf3),
        &pod::Value::Object(meta_damage_param),
    )
    .map_err(|e| {
        PipeWireError::InitFailed(format!("Failed to serialize meta damage param: {:?}", e))
    })?;

    let pod1 = pipewire::spa::pod::Pod::from_bytes(&buf1).unwrap();
    let pod2 = pipewire::spa::pod::Pod::from_bytes(&buf2).unwrap();
    let pod3 = pipewire::spa::pod::Pod::from_bytes(&buf3).unwrap();
    let mut params = [pod1, pod2, pod3];

    tracing::debug!(
        "Updating stream params with Buffers (plane_count={}), Meta Header, and Meta VideoDamage",
        plane_count
    );

    stream
        .update_params(&mut params)
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to update params: {}", e)))?;

    Ok(())
}

/// Build format parameters based on backend capabilities.
fn build_format_params(config: &StreamConfig) -> Result<Vec<Vec<u8>>, PipeWireError> {
    use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use pipewire::spa::param::ParamType;
    use pipewire::spa::pod::serialize::PodSerializer;
    use pipewire::spa::pod::Value;
    use pipewire::spa::utils::{Fraction, Rectangle, SpaTypes};
    use std::io::Cursor;

    let caps = &config.capabilities;
    let mut params: Vec<Vec<u8>> = Vec::new();

    tracing::debug!(
        "Building format params: {} formats, dmabuf={}, modifiers={}",
        caps.formats.len(),
        caps.supports_dmabuf,
        caps.modifiers.len()
    );

    // For each format, create a format object
    for fourcc in &caps.formats {
        let video_format = fourcc_to_video_format(*fourcc);

        tracing::debug!(
            "Processing format {:?} (supports_dmabuf={}, modifiers.len()={})",
            video_format,
            caps.supports_dmabuf,
            caps.modifiers.len()
        );

        // Build format with or without modifiers based on backend capabilities
        if caps.supports_dmabuf && !caps.modifiers.is_empty() {
            // DMA-BUF path: advertise format with modifiers as an enum choice
            tracing::debug!(
                "Advertising DMA-BUF format {:?} with {} modifiers: {:?}",
                video_format,
                caps.modifiers.len(),
                &caps.modifiers
            );

            use pipewire::spa::pod::{ChoiceValue, Property, PropertyFlags, Value as PodValue};
            use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id};

            // For simplicity, only offer LINEAR modifier (0x0) to avoid DONT_FIXATE complexity
            // This matches what OBS negotiates anyway
            let modifier_to_offer = vec![0i64]; // LINEAR modifier

            // Create properties vector manually to include modifier choice
            let properties = vec![
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
                            default: modifier_to_offer[0],
                            alternatives: modifier_to_offer,
                        },
                    ))),
                },
                Property {
                    key: FormatProperties::VideoSize.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: PodValue::Rectangle(Rectangle {
                        width: config.width,
                        height: config.height,
                    }),
                },
                Property {
                    key: FormatProperties::VideoFramerate.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: PodValue::Fraction(Fraction {
                        num: config.framerate_num,
                        denom: config.framerate_denom,
                    }),
                },
            ];

            let format = pipewire::spa::pod::Object {
                type_: SpaTypes::ObjectParamFormat.as_raw(),
                id: ParamType::EnumFormat.as_raw(),
                properties,
            };

            let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(format))
                .map_err(|e| {
                    PipeWireError::InitFailed(format!("Failed to serialize format: {:?}", e))
                })?
                .0
                .into_inner();
            params.push(bytes);
        } else {
            // SHM path: format without modifiers
            tracing::debug!("Advertising SHM format {:?}", video_format);

            let format = pipewire::spa::pod::object!(
                SpaTypes::ObjectParamFormat,
                ParamType::EnumFormat,
                pipewire::spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
                pipewire::spa::pod::property!(
                    FormatProperties::MediaSubtype,
                    Id,
                    MediaSubtype::Raw
                ),
                pipewire::spa::pod::property!(FormatProperties::VideoFormat, Id, video_format),
                pipewire::spa::pod::property!(
                    FormatProperties::VideoSize,
                    Rectangle,
                    Rectangle {
                        width: config.width,
                        height: config.height,
                    }
                ),
                pipewire::spa::pod::property!(
                    FormatProperties::VideoFramerate,
                    Fraction,
                    Fraction {
                        num: config.framerate_num,
                        denom: config.framerate_denom,
                    }
                ),
            );

            let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(format))
                .map_err(|e| {
                    PipeWireError::InitFailed(format!("Failed to serialize format: {:?}", e))
                })?
                .0
                .into_inner();
            params.push(bytes);
        }
    }

    tracing::debug!("Built {} format params", params.len());

    // Log first param bytes for debugging
    if !params.is_empty() {
        tracing::debug!("First format param size: {} bytes", params[0].len());
    }

    Ok(params)
}

/// Parse negotiated format from PipeWire param.
fn parse_negotiated_format(
    param: &pipewire::spa::pod::Pod,
) -> Result<NegotiatedFormat, PipeWireError> {
    use pipewire::spa::param::format::FormatProperties;
    use pipewire::spa::param::format_utils;

    // Parse media type/subtype
    let (media_type, media_subtype) = format_utils::parse_format(param)
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to parse format: {:?}", e)))?;

    use pipewire::spa::param::format::{MediaSubtype, MediaType};
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return Err(PipeWireError::InitFailed(
            "Not a raw video format".to_string(),
        ));
    }

    // Parse as VideoInfoRaw to get dimensions and format
    let mut video_info = pipewire::spa::param::video::VideoInfoRaw::default();
    video_info
        .parse(param)
        .map_err(|e| PipeWireError::InitFailed(format!("Failed to parse video info: {:?}", e)))?;

    let size_rect = video_info.size();
    let size = (size_rect.width, size_rect.height);
    let framerate_frac = video_info.framerate();
    let framerate = (framerate_frac.num, framerate_frac.denom);
    let format = video_info.format();

    // Check if a modifier was negotiated (indicates DMA-BUF)
    // Parse modifier from the param object if present
    use pipewire::spa::utils::Id;

    // DRM_FORMAT_MOD_INVALID = 0x00ffffffffffffff (indicates implicit modifier)
    const DRM_FORMAT_MOD_INVALID: i64 = 0x00ffffffffffffff_u64 as i64;

    tracing::debug!("Parsing negotiated format, looking for VideoModifier property");
    let (is_dmabuf, modifier) = if let Ok(obj) = param.as_object() {
        tracing::debug!("Successfully parsed param as object, searching for modifier property");
        let prop = obj.find_prop(Id(FormatProperties::VideoModifier.as_raw()));
        if let Some(p) = prop {
            let value = p.value();
            tracing::debug!(
                "Found VideoModifier property, raw type: {:?}",
                value.type_()
            );

            // If VideoModifier property exists, dmabuf was negotiated
            // Try to extract the actual modifier value
            let modifier_val = if let Ok(long_val) = value.get_long() {
                tracing::debug!("Read modifier as Long: 0x{:x}", long_val);
                Some(long_val)
            } else {
                // Property exists but we can't read it (probably a Choice type)
                // This still means dmabuf was negotiated
                tracing::debug!(
                    "VideoModifier exists but couldn't read value - dmabuf is still active"
                );
                Some(0) // Default to LINEAR modifier
            };

            (true, modifier_val)
        } else {
            tracing::debug!("VideoModifier property not found - using SHM");
            (false, None)
        }
    } else {
        tracing::warn!("Failed to parse param as object");
        (false, None)
    };

    if let Some(mod_value) = modifier {
        if mod_value == DRM_FORMAT_MOD_INVALID {
            tracing::debug!(
                "Negotiated with DMA-BUF using implicit modifier (DRM_FORMAT_MOD_INVALID)"
            );
        } else {
            tracing::debug!("Negotiated with DMA-BUF modifier: 0x{:x}", mod_value);
        }
    } else if is_dmabuf {
        tracing::debug!("Negotiated with DMA-BUF (modifier value unknown, defaulting to LINEAR)");
    } else {
        tracing::debug!("No modifier in negotiated format - using SHM");
    }

    Ok(NegotiatedFormat {
        format,
        size,
        framerate,
        is_dmabuf,
        modifier,
    })
}

/// Convert PipeWire VideoFormat to Smithay Fourcc.
fn video_format_to_fourcc(format: pipewire::spa::param::video::VideoFormat) -> Fourcc {
    use pipewire::spa::param::video::VideoFormat;

    match format {
        VideoFormat::BGRA => Fourcc::Argb8888, // BGRA in memory = AR24 in DRM
        VideoFormat::RGBA => Fourcc::Abgr8888, // RGBA in memory = AB24 in DRM
        VideoFormat::BGRx => Fourcc::Xrgb8888,
        VideoFormat::RGBx => Fourcc::Xbgr8888,
        _ => {
            tracing::warn!("Unknown video format {:?}, defaulting to Argb8888", format);
            Fourcc::Argb8888
        }
    }
}

/// Convert Smithay Fourcc to PipeWire VideoFormat.
fn fourcc_to_video_format(fourcc: Fourcc) -> pipewire::spa::param::video::VideoFormat {
    use pipewire::spa::param::video::VideoFormat;

    match fourcc {
        Fourcc::Argb8888 => VideoFormat::BGRA, // AR24 in DRM = BGRA in memory
        Fourcc::Abgr8888 => VideoFormat::RGBA, // AB24 in DRM = RGBA in memory
        Fourcc::Xrgb8888 => VideoFormat::BGRx,
        Fourcc::Xbgr8888 => VideoFormat::RGBx,
        _ => {
            tracing::warn!("Unknown fourcc {:?}, defaulting to BGRA", fourcc);
            VideoFormat::BGRA
        }
    }
}
