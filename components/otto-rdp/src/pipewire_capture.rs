//! PipeWire consumer for an Otto virtual-output stream.
//!
//! Connects to the virtual output's PipeWire node (Otto logs the node id at
//! startup: "Virtual output 'name' started (PipeWire node N)"), negotiates
//! 32-bit BGRx/BGRA video, and forwards every frame as a tightly-packed
//! BGRx buffer over a tokio broadcast channel. Consumers that lag simply
//! skip frames — correct behavior for a live video feed.
//!
//! Buffer handling covers the data types Otto's stream produces:
//! - `MemFd` / `MemPtr`: already mapped by pipewire-rs (`data.data()`).
//! - `DmaBuf`: mapped manually with `mmap` — Otto allocates its virtual
//!   output swapchain with linear-friendly modifiers, and consumers that
//!   cannot negotiate modifiers get a mappable buffer.

use std::sync::Arc;

use bytes::Bytes;
use pipewire as pw;
use pw::spa;
use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use spa::param::video::VideoFormat;
use spa::pod::{Pod, Property};
use tokio::sync::broadcast;

/// One captured frame, tightly packed 4-byte-per-pixel BGRx.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed rows (stride == width * 4).
    pub data: Bytes,
}

/// Frame size the capture should deliver, shared with the RDP side.
///
/// How the served desktop is laid out relative to the native frame: the
/// desktop is exactly the client's negotiated box, the native image is
/// aspect-fit (never upscaled) and centered inside it, and the bars are
/// black. Serving the client's exact size matters — clients render a
/// desktop that matches their request scaled to fill their view, but fall
/// back to a 1:1 corner rendering for any other size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServedLayout {
    /// Full desktop advertised to the client.
    pub desktop: (u32, u32),
    /// Top-left of the picture inside the desktop.
    pub img_off: (u32, u32),
    /// Size of the picture inside the desktop.
    pub img_size: (u32, u32),
    /// The space mouse coordinates arrive in: the client's originally
    /// reported box. Identical to `desktop` unless a `--desktop` override
    /// serves a different size (mobile clients that render 1:1 in physical
    /// pixels report their box — and mouse — in logical points).
    pub input_box: (u32, u32),
}

/// The RDP client's desktop layout is only known once it connects
/// (`request_initial_size`), while capture starts earlier — so it is a
/// shared cell the capture re-reads per frame. `None` means "native size,
/// no scaling".
#[derive(Clone)]
pub struct TargetSize(Arc<std::sync::Mutex<Option<ServedLayout>>>);

impl TargetSize {
    pub fn native() -> Self {
        Self(Arc::new(std::sync::Mutex::new(None)))
    }
    pub fn set(&self, layout: ServedLayout) {
        *self.0.lock().unwrap() = Some(layout);
    }
    /// The negotiated layout, if a client has connected.
    pub fn get(&self) -> Option<ServedLayout> {
        *self.0.lock().unwrap()
    }
    /// Resolve against the source size: full-frame identity when unset or
    /// degenerate (zero-sized or out-of-bounds picture rect).
    fn resolve(&self, src: (u32, u32)) -> ServedLayout {
        let full = ServedLayout {
            desktop: src,
            img_off: (0, 0),
            img_size: src,
            input_box: src,
        };
        match self.get() {
            Some(l)
                if l.desktop.0 > 0
                    && l.desktop.1 > 0
                    && l.img_size.0 > 0
                    && l.img_size.1 > 0
                    && l.img_off.0 + l.img_size.0 <= l.desktop.0
                    && l.img_off.1 + l.img_size.1 <= l.desktop.1 =>
            {
                l
            }
            _ => full,
        }
    }
}

/// The most recently composed frame, kept so a client that connects between
/// two captures can be painted immediately instead of waiting for the next
/// one to arrive (an idle desktop produces them slowly, and a bitmap client
/// shows black until its first update).
#[derive(Clone, Default)]
pub struct LatestFrame(Arc<std::sync::Mutex<Option<Arc<Frame>>>>);

impl LatestFrame {
    pub fn new() -> Self {
        Self::default()
    }
    fn set(&self, frame: Arc<Frame>) {
        *self.0.lock().unwrap() = Some(frame);
    }
    /// The last frame, if it was composed for `size` (a client that
    /// negotiated a different desktop can't use it).
    pub fn get_for(&self, size: (u32, u32)) -> Option<Arc<Frame>> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .filter(|f| (f.width, f.height) == size)
    }
}

/// Spawn the PipeWire main-loop thread, connecting to `node_id`.
/// Frames are published on `tx`, already scaled to `target` (see `TargetSize`)
/// so the big native frame is never allocated. The caller owns the channel so
/// the same one can be shared with a display handler (and started lazily, e.g.
/// only when a client falls back from the H.264 path).
pub fn spawn(
    node_id: u32,
    expected: (u32, u32),
    target: TargetSize,
    tx: broadcast::Sender<Arc<Frame>>,
    latest: LatestFrame,
) {
    std::thread::Builder::new()
        .name("pw-capture".into())
        .spawn(move || {
            if let Err(e) = run(node_id, expected, tx, target, latest) {
                tracing::error!("pipewire capture terminated: {e:#}");
            }
        })
        .expect("failed to spawn pipewire capture thread");
}

/// Cap the delivered frame rate. A remote desktop does not need the output's
/// full 30 fps, and each 2880×1920 frame is ~22 MB — dropping frames *before*
/// the copy keeps allocation churn (and downstream RDP encode/send buffering)
/// from starving the machine. Overridable via OTTO_RDP_FPS.
fn target_frame_interval() -> std::time::Duration {
    let fps = std::env::var("OTTO_RDP_FPS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|f| *f > 0.0)
        .unwrap_or(12.0);
    std::time::Duration::from_secs_f64(1.0 / fps)
}

struct StreamData {
    format: Option<spa::param::video::VideoInfoRaw>,
    tx: broadcast::Sender<Arc<Frame>>,
    /// Wall-clock of the last delivered frame, for rate limiting.
    last_emit: Option<std::time::Instant>,
    min_interval: std::time::Duration,
    /// Size to deliver (the RDP client's desktop), re-read per frame.
    target: TargetSize,
    /// Layout the last emitted frame was composed for. A change means a
    /// client just negotiated its desktop, so that frame skips rate limiting.
    last_layout: Option<ServedLayout>,
    /// Newest frame, for clients that subscribe between two captures.
    latest: LatestFrame,
}

fn run(
    node_id: u32,
    expected: (u32, u32),
    tx: broadcast::Sender<Arc<Frame>>,
    target: TargetSize,
    latest: LatestFrame,
) -> anyhow::Result<()> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let stream = pw::stream::StreamRc::new(
        core,
        "otto-rdp-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let data = StreamData {
        format: None,
        tx,
        last_emit: None,
        min_interval: target_frame_interval(),
        target,
        last_layout: None,
        latest,
    };

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_, _, old, new| {
            tracing::info!("capture stream state: {old:?} -> {new:?}");
        })
        .param_changed(|_, data, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let (media_type, media_subtype) =
                match spa::param::format_utils::parse_format(param) {
                    Ok(v) => v,
                    Err(_) => return,
                };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut info = spa::param::video::VideoInfoRaw::default();
            if info.parse(param).is_ok() {
                tracing::info!(
                    "negotiated video format: {:?} {}x{}",
                    info.format(),
                    info.size().width,
                    info.size().height
                );
                data.format = Some(info);
            }
        })
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(info) = data.format else { return };
            let width = info.size().width;
            let height = info.size().height;

            // Scale straight out of the source buffer into the client's
            // letterboxed desktop — the full-res frame is never materialised
            // (22 MB -> ~4 MB for a phone-sized client).
            let layout = data.target.resolve((width, height));

            // Rate limit BEFORE the ~22 MB copy below: drop frames that arrive
            // sooner than the target interval. `buffer` is returned to PipeWire
            // when it drops at end of scope. Keeps allocation churn and RDP
            // send-buffer growth bounded (the OOM risk on a big display).
            // Exception: the first frame for a newly negotiated layout — a
            // client that just connected has nothing on screen, so it must not
            // wait out the interval for its first picture.
            let now = std::time::Instant::now();
            let layout_changed = data.last_layout != Some(layout);
            if !layout_changed {
                if let Some(last) = data.last_emit {
                    if now.duration_since(last) < data.min_interval {
                        return;
                    }
                }
            }
            data.last_emit = Some(now);

            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let d = &mut datas[0];
            let chunk_stride = d.chunk().stride() as usize;
            let stride = if chunk_stride > 0 {
                chunk_stride
            } else {
                (width as usize) * 4
            };
            let needed = height as usize * stride;
            let map_offset = d.as_raw().mapoffset as usize;

            // The producer's buffer may be a real dmabuf that MAP_BUFFERS did
            // NOT map (dmabufs aren't always mmap'd for the consumer). In that
            // case `data()` returns an empty/short slice — detect that and
            // mmap the fd ourselves rather than forwarding a truncated frame.
            let raw_fd = d.as_raw().fd as i32;
            let type_ = d.type_();
            let mapped_ok = d.data().map(|s| s.len() >= needed).unwrap_or(false);

            let (dw, dh) = layout.desktop;

            let pixels: Option<Vec<u8>> = if mapped_ok {
                d.data()
                    .map(|slice| compose_frame(slice, width, height, stride, layout))
            } else if raw_fd >= 0 {
                // Covers DmaBuf and any MemFd the flag didn't map for us.
                map_dmabuf(raw_fd, map_offset + needed).and_then(|m| {
                    let s = m.slice();
                    if s.len() >= map_offset + needed {
                        Some(compose_frame(&s[map_offset..], width, height, stride, layout))
                    } else {
                        None
                    }
                })
            } else {
                None
            };

            let Some(pixels) = pixels else {
                static WARNED: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(
                        "cannot read frame buffer (type {type_:?}, need {needed} B) — dropping frames"
                    );
                }
                return;
            };

            // Never forward a short frame: the RDP encoder reads
            // width*height*4 bytes and panics on anything smaller.
            if pixels.len() != dw as usize * dh as usize * 4 {
                return;
            }

            let frame = Arc::new(Frame {
                width: dw,
                height: dh,
                data: Bytes::from(pixels),
            });
            data.last_layout = Some(layout);
            data.latest.set(Arc::clone(&frame));
            // Send fails only when no RDP client is connected — fine.
            let receivers = data.tx.send(frame).unwrap_or(0);
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 3 || n.is_multiple_of(60) {
                tracing::info!("captured frame #{n} ({width}x{height}), {receivers} RDP subscriber(s)");
            }
        })
        .register()?;

    // Offer 32-bit formats WITH the LINEAR DRM modifier: Otto's stream
    // advertises dmabuf-only formats whose modifier property is MANDATORY
    // (linear is the only one offered — see screenshare/pipewire_stream.rs),
    // so a pod without a modifier never intersects ("no more input formats").
    // Linear dmabufs stay CPU-mappable, which the process callback relies on.
    let obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        Property::new(
            FormatProperties::MediaType.as_raw(),
            spa::pod::Value::Id(spa::utils::Id(MediaType::Video.as_raw()))
        ),
        Property::new(
            FormatProperties::MediaSubtype.as_raw(),
            spa::pod::Value::Id(spa::utils::Id(MediaSubtype::Raw.as_raw()))
        ),
        Property::new(
            FormatProperties::VideoFormat.as_raw(),
            spa::pod::Value::Choice(spa::pod::ChoiceValue::Id(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Enum {
                    default: spa::utils::Id(VideoFormat::BGRx.as_raw()),
                    alternatives: vec![
                        spa::utils::Id(VideoFormat::BGRA.as_raw()),
                        spa::utils::Id(VideoFormat::xRGB.as_raw()),
                        spa::utils::Id(VideoFormat::ARGB.as_raw()),
                    ],
                },
            )))
        ),
        Property {
            key: FormatProperties::VideoModifier.as_raw(),
            flags: spa::pod::PropertyFlags::MANDATORY,
            value: spa::pod::Value::Choice(spa::pod::ChoiceValue::Long(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Enum {
                    default: 0, // DRM_FORMAT_MOD_LINEAR
                    alternatives: vec![0],
                },
            ))),
        },
        Property::new(
            FormatProperties::VideoSize.as_raw(),
            spa::pod::Value::Rectangle(spa::utils::Rectangle {
                width: expected.0,
                height: expected.1,
            })
        ),
    );
    let values = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).ok_or_else(|| anyhow::anyhow!("bad pod"))?];

    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    tracing::info!("connecting to PipeWire node {node_id}");
    mainloop.run();
    Ok(())
}

/// Copy a strided BGRA source into a tightly-packed `dst_w`×`dst_h` buffer,
/// box-filtering when downscaling.
///
/// Averaging each destination pixel over its source footprint (rather than
/// nearest-neighbour) matters here: at a ~4x reduction nearest aliases text
/// into unreadable mush, and text is most of what a remote desktop shows. The
/// 1:1 case short-circuits to a straight row copy.
/// Produce the full served desktop: source scaled into the picture rect of
/// `layout`, remaining bars black (zeroed — BgrX ignores the X byte).
fn compose_frame(src: &[u8], src_w: u32, src_h: u32, stride: usize, l: ServedLayout) -> Vec<u8> {
    let img = scale_rows(src, src_w, src_h, stride, l.img_size.0, l.img_size.1);
    if l.desktop == l.img_size && l.img_off == (0, 0) {
        return img;
    }
    let (dw, dh) = (l.desktop.0 as usize, l.desktop.1 as usize);
    let (iw, ih) = (l.img_size.0 as usize, l.img_size.1 as usize);
    let (ox, oy) = (l.img_off.0 as usize, l.img_off.1 as usize);
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..ih {
        let src_row = y * iw * 4;
        let dst_row = ((y + oy) * dw + ox) * 4;
        out[dst_row..dst_row + iw * 4].copy_from_slice(&img[src_row..src_row + iw * 4]);
    }
    out
}

fn scale_rows(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    stride: usize,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    // Fast path: no scaling requested.
    if dst_w == src_w && dst_h == src_h {
        let row_bytes = src_w as usize * 4;
        let mut out = Vec::with_capacity(row_bytes * src_h as usize);
        for y in 0..src_h as usize {
            let start = y * stride;
            let end = start + row_bytes;
            if end > src.len() {
                break;
            }
            out.extend_from_slice(&src[start..end]);
        }
        return out;
    }

    let mut out = vec![0u8; dst_w as usize * dst_h as usize * 4];
    for dy in 0..dst_h as usize {
        // Source row span covered by this destination row.
        let y0 = dy * src_h as usize / dst_h as usize;
        let y1 = (((dy + 1) * src_h as usize).div_ceil(dst_h as usize)).max(y0 + 1);
        let y1 = y1.min(src_h as usize);

        for dx in 0..dst_w as usize {
            let x0 = dx * src_w as usize / dst_w as usize;
            let x1 = (((dx + 1) * src_w as usize).div_ceil(dst_w as usize)).max(x0 + 1);
            let x1 = x1.min(src_w as usize);

            let (mut b, mut g, mut r, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                let row = sy * stride;
                if row + x1 * 4 > src.len() {
                    break;
                }
                for sx in x0..x1 {
                    let p = row + sx * 4;
                    b += src[p] as u32;
                    g += src[p + 1] as u32;
                    r += src[p + 2] as u32;
                    a += src[p + 3] as u32;
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let o = (dy * dst_w as usize + dx) * 4;
            out[o] = (b / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (r / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    out
}

/// RAII mmap of a dmabuf fd.
struct Mmap {
    ptr: *mut libc::c_void,
    len: usize,
}

impl Mmap {
    fn slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

fn map_dmabuf(fd: i32, len: usize) -> Option<Mmap> {
    if len == 0 {
        return None;
    }
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        tracing::warn!("dmabuf mmap failed: {}", std::io::Error::last_os_error());
        return None;
    }
    Some(Mmap { ptr, len })
}

#[cfg(test)]
mod tests {
    use super::scale_rows;

    /// 1:1 must copy exactly, honouring stride padding.
    #[test]
    fn identity_copy_strips_stride_padding() {
        let (w, h) = (2u32, 2u32);
        let stride = 12; // 2px * 4B + 4B padding
        let mut src = vec![0u8; stride * h as usize];
        for (i, px) in [10u8, 20, 30, 40].iter().enumerate() {
            let (x, y) = (i % 2, i / 2);
            let p = y * stride + x * 4;
            src[p..p + 4].copy_from_slice(&[*px, *px, *px, 255]);
        }
        let out = scale_rows(&src, w, h, stride, w, h);
        assert_eq!(out.len(), 16);
        assert_eq!(out[0], 10);
        assert_eq!(out[4], 20);
        assert_eq!(out[8], 30);
        assert_eq!(out[12], 40);
    }

    /// 2x2 -> 1x1 must average all four pixels (box filter, not nearest).
    #[test]
    fn downscale_averages_footprint() {
        let stride = 8;
        // values 0, 100, 200, 255 -> mean 138 (integer div)
        let src = vec![
            0, 0, 0, 255, 100, 100, 100, 255, // row 0
            200, 200, 200, 255, 255, 255, 255, 255, // row 1
        ];
        let out = scale_rows(&src, 2, 2, stride, 1, 1);
        assert_eq!(out.len(), 4);
        let expected = [0, 100, 200, 255].iter().sum::<u32>() / 4;
        assert_eq!(out[0] as u32, expected);
        assert_eq!(out[3], 255, "alpha preserved");
    }

    /// Non-integer ratios must stay in bounds and fill every pixel.
    #[test]
    fn non_integer_ratio_is_in_bounds() {
        let (w, h) = (2880u32, 1920u32);
        let stride = w as usize * 4;
        let src = vec![77u8; stride * h as usize];
        let (dw, dh) = (736u32, 1374u32);
        let out = scale_rows(&src, w, h, stride, dw, dh);
        assert_eq!(out.len(), dw as usize * dh as usize * 4);
        // Uniform source -> every output pixel is the same value, none left 0.
        assert!(out.chunks(4).all(|p| p[0] == 77), "all pixels filled");
    }
}
