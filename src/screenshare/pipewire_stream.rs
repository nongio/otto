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
    /// Rendered buffers whose GPU fence has not yet signaled. Held here (NOT in
    /// `to_queue`) so the async process callback can't hand a still-rendering
    /// buffer to the consumer. Moved into `to_queue` once the fence is reached.
    pub pending: HashMap<i64, *mut pipewire::sys::pw_buffer>,
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
    /// True while a consumer is linked and the stream is in `Streaming`
    /// state (e.g. an RDP bridge or portal client pulling frames), as
    /// opposed to `active` which only means the stream was started.
    streaming: AtomicBool,
    should_stop: AtomicBool,
    /// Buffer pool shared with main thread for blitting
    buffer_pool: Arc<Mutex<BufferPool>>,
    /// Raw stream pointer for triggering from main thread
    stream_ptr: Arc<Mutex<Option<*mut pipewire::sys::pw_stream>>>,
    /// Frame sequence counter for actual rendered frames (shared between threads)
    frame_sequence: AtomicU64,
    /// Start time for calculating PTS (nanoseconds since CLOCK_MONOTONIC)
    start_time_ns: AtomicU64,
    /// Size the buffers currently in the pool were allocated at. Written by
    /// the PipeWire thread once a format is negotiated, read by the main
    /// thread to know what it may blit into.
    width: AtomicU32,
    height: AtomicU32,
    /// Size the capture target wants, set by the main thread when it changes.
    /// The PipeWire thread picks it up and renegotiates the format.
    pending_size: Mutex<Option<(u32, u32)>>,
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
            streaming: AtomicBool::new(false),
            should_stop: AtomicBool::new(false),
            buffer_pool: Arc::new(Mutex::new(BufferPool::default())),
            #[allow(clippy::arc_with_non_send_sync)]
            stream_ptr: Arc::new(Mutex::new(None)),
            frame_sequence: AtomicU64::new(0),
            start_time_ns: AtomicU64::new(0),
            width: AtomicU32::new(config.width),
            height: AtomicU32::new(config.height),
            pending_size: Mutex::new(None),
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
            shared.streaming.store(false, Ordering::SeqCst);
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

    /// Check if a consumer is currently linked and pulling frames
    /// (PipeWire stream state is `Streaming`).
    pub fn is_streaming(&self) -> bool {
        self.shared.streaming.load(Ordering::SeqCst)
    }

    /// The size the stream's buffers are currently allocated at, in pixels.
    ///
    /// Follows [`Self::request_size`] once renegotiation completes, so it lags
    /// a resized window by a few frames.
    pub fn stream_size(&self) -> (u32, u32) {
        (
            self.shared.width.load(Ordering::Relaxed),
            self.shared.height.load(Ordering::Relaxed),
        )
    }

    /// Ask the stream to renegotiate its format at `size`.
    ///
    /// Called every frame with the capture target's current size; a request
    /// that matches what is already negotiated (or already queued) is dropped,
    /// so only a real resize reaches PipeWire. The PipeWire thread debounces
    /// the rest, since an interactive drag produces a new size every frame.
    pub fn request_size(&self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (width, height) == self.stream_size() {
            // Cancel a queued request that this frame just made stale.
            let mut pending = self.shared.pending_size.lock().unwrap();
            if pending.is_some() {
                *pending = None;
            }
            return;
        }
        let mut pending = self.shared.pending_size.lock().unwrap();
        if *pending != Some((width, height)) {
            *pending = Some((width, height));
        }
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
    mut config: StreamConfig,
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
            let shared = shared.clone();
            move |stream, _state, old, new| {
                use pw::stream::StreamState as PwState;

                tracing::debug!("PipeWire stream state: {:?} -> {:?}", old, new);

                shared
                    .streaming
                    .store(matches!(new, PwState::Streaming), Ordering::SeqCst);

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
            let shared = shared.clone();
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

                    // Publish the size the main thread may blit into. PipeWire
                    // re-runs remove_buffer/add_buffer for the new dimensions
                    // right after this, so the pool never mixes sizes.
                    shared.width.store(negotiated.size.0, Ordering::Relaxed);
                    shared.height.store(negotiated.size.1, Ordering::Relaxed);

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
            let buffer_pool = shared.buffer_pool.clone();
            move |_stream, _user_data, buffer| unsafe {
                let fd = (*(*buffer).buffer).datas.read().fd;
                let removed = state.borrow_mut().dmabufs.remove(&fd);

                // Drop every trace of this buffer from the shared pool too —
                // a renegotiation (window resize) frees the whole set, and a
                // stale entry would have the main thread blit into a buffer of
                // the previous size that PipeWire no longer owns.
                let mut pool = buffer_pool.lock().unwrap();
                pool.dmabufs.remove(&fd);
                pool.available.retain(|b| b.fd != fd);
                pool.to_queue.remove(&fd);
                pool.pending.remove(&fd);
                if pool.last_rendered_fd == Some(fd) {
                    pool.last_rendered_fd = None;
                }

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
    // An interactive resize changes the target size every frame; renegotiating
    // that often would thrash buffer allocation, so settle for a moment first.
    const RESIZE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
    let mut last_resize = std::time::Instant::now();
    while !shared.should_stop.load(Ordering::SeqCst) {
        loop_ref.iterate(std::time::Duration::from_millis(16));

        let pending = *shared.pending_size.lock().unwrap();
        let Some((width, height)) = pending else {
            continue;
        };
        if (width, height) == (config.width, config.height) {
            *shared.pending_size.lock().unwrap() = None;
            continue;
        }
        if last_resize.elapsed() < RESIZE_DEBOUNCE {
            continue;
        }
        *shared.pending_size.lock().unwrap() = None;
        last_resize = std::time::Instant::now();

        config.width = width;
        config.height = height;

        tracing::debug!("Renegotiating stream format at {}x{}", width, height);

        let format_params_bytes = match build_format_params(&config) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to build format params for resize: {}", e);
                continue;
            }
        };
        let mut format_params: Vec<&pipewire::spa::pod::Pod> = format_params_bytes
            .iter()
            .filter_map(|bytes| pipewire::spa::pod::Pod::from_bytes(bytes))
            .collect();
        if let Err(e) = stream.update_params(&mut format_params) {
            tracing::error!("Failed to update format params for resize: {}", e);
        }
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
                // At least 2 buffers so a new frame can render into a spare
                // while the previous frame's GPU fence drains off the main
                // loop (deferred trigger in render_virtual_outputs). A single
                // buffer would serialize render and hand-off, halving fps.
                ChoiceEnum::Range {
                    default: 2,
                    min: 2,
                    max: 3
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
            // DMA-BUF path: one pod per modifier, each with a single fixed
            // value. Separate fixed pods sidestep the DONT_FIXATE fixation
            // dance while still letting clients whose importer only takes
            // tiled layouts (gst vapostproc) find a match beyond LINEAR.
            // Pod order is the preference order (LINEAR first, see caller).
            tracing::debug!(
                "Advertising DMA-BUF format {:?} with {} modifiers: {:x?}",
                video_format,
                caps.modifiers.len(),
                &caps.modifiers
            );

            use pipewire::spa::pod::{Property, PropertyFlags, Value as PodValue};
            use pipewire::spa::utils::Id;

            for &modifier in &caps.modifiers {
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
                        value: PodValue::Long(modifier),
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

                let bytes =
                    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(format))
                        .map_err(|e| {
                            PipeWireError::InitFailed(format!(
                                "Failed to serialize format: {:?}",
                                e
                            ))
                        })?
                        .0
                        .into_inner();
                params.push(bytes);
            }
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
            } else if value.is_choice() {
                // Choice pod (client proposal not yet fixated): the first
                // value in the body is the default — the one the client will
                // use. Guessing anything else here means we allocate one
                // layout and the consumer reads another (tiled-vs-linear
                // garbage on screen), so a wrong value is worse than failing.
                let modifier = unsafe {
                    use pipewire::spa::sys::{spa_pod, spa_pod_choice, SPA_TYPE_Long};
                    let choice = value.as_raw_ptr() as *const spa_pod_choice;
                    let child = &(*choice).body.child as *const spa_pod;
                    if (*child).type_ == SPA_TYPE_Long {
                        let first =
                            (child as *const u8).add(std::mem::size_of::<spa_pod>()) as *const i64;
                        Some(first.read_unaligned())
                    } else {
                        None
                    }
                };
                let Some(modifier) = modifier else {
                    return Err(PipeWireError::InitFailed(
                        "VideoModifier choice is not of type Long".to_string(),
                    ));
                };
                tracing::debug!("Read modifier from Choice default: 0x{:x}", modifier);
                Some(modifier)
            } else {
                return Err(PipeWireError::InitFailed(format!(
                    "VideoModifier present but unreadable (pod type {:?})",
                    value.type_()
                )));
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
