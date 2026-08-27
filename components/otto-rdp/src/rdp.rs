//! ironrdp-server glue: display updates from the PipeWire frame channel,
//! input events into the Wayland injection thread.

use std::num::{NonZeroU16, NonZeroUsize};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use anyhow::Result;
use ironrdp_server::{
    BitmapUpdate, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
};
use tokio::sync::broadcast;

use crate::pipewire_capture::Frame;
use crate::wl_input::{self, InputCommand, BTN_EXTRA, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_SIDE};

// ── Display ────────────────────────────────────────────────────────────────

pub struct VirtualOutputDisplay {
    /// The output's native size — the upper bound on what we serve.
    pub size: (u32, u32),
    pub frames: broadcast::Sender<Arc<Frame>>,
    /// Shared with the capture thread; setting it makes frames arrive
    /// pre-scaled to the client's desktop size.
    pub target: crate::pipewire_capture::TargetSize,
    /// Size actually being served (native until a client negotiates smaller).
    pub served: (u32, u32),
    /// Serve this desktop size instead of the client's reported box
    /// (`--desktop WxH`). For clients that render the desktop 1:1 in
    /// physical pixels but report their box in logical points: set it to
    /// the device's physical screen resolution to fill the screen.
    pub desktop_override: Option<(u32, u32)>,
    /// Hardware H.264 path: video flows over EGFX/AVC420 (see `egfx.rs`), not
    /// through this handler. `next_update` parks — the legacy bitmap path is
    /// bypassed entirely — but size negotiation and the letterbox layout are
    /// shared with the bitmap path so input mapping stays correct.
    pub egfx_mode: bool,
    /// Newest captured frame, replayed to a client the moment it subscribes so
    /// it isn't left on a black screen until the next capture arrives.
    pub latest: crate::pipewire_capture::LatestFrame,
    /// EGFX only: shared state with the graphics driver. The negotiated desktop
    /// size is recorded here (synchronously, before the channel is ready) so the
    /// driver can size its surface and the encoder can letterbox native into it.
    pub gfx_shared: Option<std::sync::Arc<crate::egfx::GfxShared>>,
    /// Raises the desktop's screen-sharing indicator while a client is served.
    pub indicator: crate::indicator::Indicator,
    /// Peer address of the connection currently being served, filled in by the
    /// connection handler at accept time.
    pub peer: std::sync::Arc<std::sync::Mutex<String>>,
}

pub struct Updates {
    rx: broadcast::Receiver<Arc<Frame>>,
    /// EGFX mode: the codec decision for this client. `None` on the pure bitmap
    /// path, or once we've committed to serving bitmaps. While `Some`, the first
    /// `next_update` waits for the decision — AVC parks here (video flows over
    /// the graphics pipeline), everything else falls through to bitmaps.
    codec_rx: Option<tokio::sync::watch::Receiver<crate::egfx::Codec>>,
    /// Frame to send before waiting on the channel: the newest capture at
    /// subscribe time, so the client paints as soon as it asks for updates
    /// rather than after the next compositor frame. Taken once.
    initial: Option<Arc<Frame>>,
}

/// Lay the native picture out inside the desktop served to a client.
///
/// `desktop` is the client's own box verbatim (or the `--desktop` override),
/// because clients scale a size-matched desktop to fill their view but render
/// any other size unscaled in a corner. The native picture is aspect-fit inside
/// it — never upscaled — and centered, leaving black bars. `input_box` stays the
/// client's reported box: mouse coordinates arrive in that space and are mapped
/// back through the picture rect by [`InputForwarder::map_abs`].
///
/// On the EGFX path the desktop is rounded down to even dimensions: H.264
/// macroblocks (and the AVC420 encoder) cannot represent odd sizes, and the
/// encoded picture has to match the desktop exactly.
fn served_layout(
    native: (u32, u32),
    box_size: (u32, u32),
    desktop_override: Option<(u32, u32)>,
    egfx_mode: bool,
) -> crate::pipewire_capture::ServedLayout {
    let (nw, nh) = (native.0 as f64, native.1 as f64);
    let (dw, dh) = desktop_override.unwrap_or(box_size);
    let (dw, dh) = if egfx_mode {
        (dw & !1, dh & !1)
    } else {
        (dw, dh)
    };
    let scale = (dw as f64 / nw).min(dh as f64 / nh).min(1.0);
    let iw = (((nw * scale).round() as u32).max(1)).min(dw);
    let ih = (((nh * scale).round() as u32).max(1)).min(dh);
    crate::pipewire_capture::ServedLayout {
        desktop: (dw, dh),
        img_off: ((dw - iw) / 2, (dh - ih) / 2),
        img_size: (iw, ih),
        input_box: box_size,
    }
}

#[async_trait::async_trait]
impl RdpServerDisplay for VirtualOutputDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: self.served.0 as u16,
            height: self.served.1 as u16,
        }
    }

    /// Serve **exactly the client's box**, letterboxing server-side.
    ///
    /// Clients render a desktop that matches their requested size scaled to
    /// fill their view, but fall back to an unscaled 1:1 corner rendering for
    /// any other size (the "tiny desktop" bug). So the desktop is the client
    /// box verbatim (rounded to even — some RDP bitmap codecs dislike odd
    /// sizes), the native picture is aspect-fit inside it (never upscaled),
    /// and the bars are black. Mouse coordinates arrive in this same box
    /// space and are mapped back through the picture rect.
    async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
        // The client's box must be honored VERBATIM — clients (notably
        // mobile apps with a fixed resolution list) scale a size-matched
        // desktop to fill their screen but render any other size, even one
        // pixel off, unscaled in a corner.
        let box_size = if client_size.width > 0 && client_size.height > 0 {
            (client_size.width as u32, client_size.height as u32)
        } else {
            self.size
        };

        // The served desktop: the client's box by default, or the --desktop
        // override (the device's physical screen). Either way the native
        // picture is aspect-fit and centered inside it, bars black. Clients
        // that stretch a size-mismatched desktop to fill their view stretch
        // uniformly when the desktop's aspect matches the view's — the
        // override should therefore be the device's screen resolution.
        let layout = served_layout(self.size, box_size, self.desktop_override, self.egfx_mode);
        let (dw, dh) = layout.desktop;
        let (iw, ih) = layout.img_size;

        self.served = (dw, dh);
        // Always record the layout — the input path maps mouse coordinates
        // through it, and a reconnecting client must replace the previous
        // connection's layout.
        self.target.set(layout);
        tracing::info!(
            "client box {}x{}; serving {dw}x{dh} with {iw}x{ih} picture at {:?} (native {}x{}){}",
            client_size.width,
            client_size.height,
            layout.img_off,
            self.size.0,
            self.size.1,
            if self.egfx_mode { " over AVC420" } else { "" }
        );
        // EGFX: record the negotiated desktop (synchronously, before the
        // graphics channel is ready) so the driver can size its surface and the
        // encoder can letterbox native into it. The picture the encoder produces
        // then matches the `layout` the input path maps through.
        if let Some(shared) = &self.gfx_shared {
            shared.set_desktop(dw as u16, dh as u16);
        }
        DesktopSize {
            width: dw as u16,
            height: dh as u16,
        }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        if self.egfx_mode {
            tracing::info!(
                "RDP client requested display updates — EGFX mode (AVC or bitmap fallback)"
            );
        } else {
            tracing::info!("RDP client requested display updates — subscribing to frames");
        }

        // A client asking for display updates is the first moment a remote
        // party can actually see the screen — not the TCP accept, which a port
        // scan also reaches. Raise the privacy signal here.
        let peer = self.peer.lock().unwrap().clone();
        self.indicator.show(&peer);

        Ok(Box::new(Updates {
            rx: self.frames.subscribe(),
            codec_rx: self.gfx_shared.as_ref().map(|s| s.codec_rx()),
            initial: self.latest.get_for(self.served),
        }))
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        // EGFX mode: resolve the codec decision before touching the bitmap path.
        // An AVC client gets video over the graphics pipeline, so this main-
        // channel path parks forever; a client that disabled AVC (or one that
        // never opens EGFX) falls through to bitmap delivery below.
        if let Some(codec_rx) = &mut self.codec_rx {
            loop {
                let codec = *codec_rx.borrow_and_update();
                match codec {
                    crate::egfx::Codec::Avc => std::future::pending::<()>().await,
                    crate::egfx::Codec::Bitmap => break,
                    crate::egfx::Codec::Unknown => {
                        // No EGFX caps yet; if none arrive (client without EGFX),
                        // default to bitmaps after a short grace period.
                        let changed = tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            codec_rx.changed(),
                        )
                        .await;
                        if !matches!(changed, Ok(Ok(()))) {
                            break; // timed out or sender gone → serve bitmaps
                        }
                    }
                }
            }
            // Committed to bitmaps — don't re-wait on subsequent calls.
            self.codec_rx = None;
        }
        // Paint whatever was on screen at subscribe time first: on an idle
        // desktop the next capture can be a long way off, and until then the
        // client shows black.
        if let Some(frame) = self.initial.take() {
            if let Some(update) = frame_to_bitmap(&frame) {
                tracing::info!(
                    "sending the cached frame ({}x{}) as the client's first bitmap",
                    frame.width,
                    frame.height
                );
                return Ok(Some(DisplayUpdate::Bitmap(update)));
            }
        }
        loop {
            match self.rx.recv().await {
                Ok(frame) => {
                    let Some(update) = frame_to_bitmap(&frame) else {
                        tracing::warn!(
                            "frame {}x{} rejected by frame_to_bitmap",
                            frame.width,
                            frame.height
                        );
                        continue;
                    };
                    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if n < 3 || n.is_multiple_of(60) {
                        tracing::info!(
                            "sending bitmap #{n} to RDP client ({}x{})",
                            frame.width,
                            frame.height
                        );
                    }
                    return Ok(Some(DisplayUpdate::Bitmap(update)));
                }
                // Dropped frames while we were busy — video feed, just go on.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return Ok(None),
            }
        }
    }
}

fn frame_to_bitmap(frame: &Frame) -> Option<BitmapUpdate> {
    let width = NonZeroU16::new(frame.width as u16)?;
    let height = NonZeroU16::new(frame.height as u16)?;
    let stride = NonZeroUsize::new(frame.width as usize * 4)?;
    // The encoder reads stride*height bytes; a short buffer would panic it.
    if frame.data.len() < stride.get() * height.get() as usize {
        return None;
    }
    Some(BitmapUpdate {
        x: 0,
        y: 0,
        width,
        height,
        // Otto renders [X/A]RGB8888 little-endian: bytes are B,G,R,X.
        format: PixelFormat::BgrX32,
        data: frame.data.clone(),
        stride,
    })
}

// ── Input ──────────────────────────────────────────────────────────────────

pub struct InputForwarder {
    pub tx: Sender<InputCommand>,
    /// The output's native pixel size — the space `InputCommand` coordinates
    /// live in.
    pub native: (u32, u32),
    /// The desktop size negotiated with the client (see `VirtualOutputDisplay`).
    /// Mouse events arrive in this space and must be scaled up to native.
    pub served: crate::pipewire_capture::TargetSize,
}

impl InputForwarder {
    /// Map absolute client coordinates to native output pixels. Coordinates
    /// arrive in the client's box space (`input_box`), are normalized per
    /// axis into the served desktop, then mapped through the letterbox
    /// picture rect; positions in the bars clamp to the picture edge.
    fn map_abs(&self, x: u16, y: u16) -> (u32, u32) {
        let Some(l) = self.served.get() else {
            return (
                (x as u32).min(self.native.0.saturating_sub(1)),
                (y as u32).min(self.native.1.saturating_sub(1)),
            );
        };
        let dx = x as f64 * l.desktop.0 as f64 / l.input_box.0.max(1) as f64;
        let dy = y as f64 * l.desktop.1 as f64 / l.input_box.1.max(1) as f64;
        let (iw, ih) = (l.img_size.0.max(1), l.img_size.1.max(1));
        let fx = (dx - l.img_off.0 as f64).clamp(0.0, (iw - 1) as f64);
        let fy = (dy - l.img_off.1 as f64).clamp(0.0, (ih - 1) as f64);
        let nx = (fx * self.native.0 as f64 / iw as f64).round() as u32;
        let ny = (fy * self.native.1 as f64 / ih as f64).round() as u32;
        (
            nx.min(self.native.0.saturating_sub(1)),
            ny.min(self.native.1.saturating_sub(1)),
        )
    }

    /// Box-space→native scale factor for relative deltas (uniform enough —
    /// x axis; 1.0 while no client has negotiated).
    fn rel_scale(&self) -> f64 {
        match self.served.get() {
            Some(l) if l.img_size.0 > 0 && l.input_box.0 > 0 => {
                (self.native.0 as f64 / l.img_size.0 as f64)
                    * (l.desktop.0 as f64 / l.input_box.0 as f64)
            }
            _ => 1.0,
        }
    }
}

impl RdpServerInputHandler for InputForwarder {
    fn keyboard(&mut self, event: KeyboardEvent) {
        tracing::debug!("RDP keyboard event: {event:?}");
        let cmd = match event {
            KeyboardEvent::Pressed { code, extended } => {
                wl_input::scancode_to_evdev(code, extended)
                    .map(|key| InputCommand::Key { key, pressed: true })
            }
            KeyboardEvent::Released { code, extended } => {
                wl_input::scancode_to_evdev(code, extended).map(|key| InputCommand::Key {
                    key,
                    pressed: false,
                })
            }
            // Mobile / on-screen keyboards send Unicode instead of scancodes.
            KeyboardEvent::UnicodePressed(c) => Some(InputCommand::Unicode { c, pressed: true }),
            KeyboardEvent::UnicodeReleased(c) => Some(InputCommand::Unicode { c, pressed: false }),
            other => {
                tracing::debug!("unhandled keyboard event: {other:?}");
                None
            }
        };
        if let Some(cmd) = cmd {
            let _ = self.tx.send(cmd);
        }
    }

    fn mouse(&mut self, event: MouseEvent) {
        static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
        if FIRST.swap(false, std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("first RDP mouse event received: {event:?}");
        }
        // Client coordinates are in the negotiated desktop (box) space —
        // map them back to native pixels through the letterbox layout.
        let rel_scale = self.rel_scale();
        let cmd = match event {
            MouseEvent::Move { x, y } => {
                let (nx, ny) = self.map_abs(x, y);
                Some(InputCommand::Move { x: nx, y: ny })
            }
            MouseEvent::LeftPressed => Some(button(BTN_LEFT, true)),
            MouseEvent::LeftReleased => Some(button(BTN_LEFT, false)),
            MouseEvent::RightPressed => Some(button(BTN_RIGHT, true)),
            MouseEvent::RightReleased => Some(button(BTN_RIGHT, false)),
            MouseEvent::MiddlePressed => Some(button(BTN_MIDDLE, true)),
            MouseEvent::MiddleReleased => Some(button(BTN_MIDDLE, false)),
            MouseEvent::Button4Pressed => Some(button(BTN_SIDE, true)),
            MouseEvent::Button4Released => Some(button(BTN_SIDE, false)),
            MouseEvent::Button5Pressed => Some(button(BTN_EXTRA, true)),
            MouseEvent::Button5Released => Some(button(BTN_EXTRA, false)),
            MouseEvent::VerticalScroll { value } => {
                // RDP: ±120 per notch, positive = wheel up.
                // Wayland: ~15 units per notch, positive = scroll down.
                let notches = value as f64 / 120.0;
                Some(InputCommand::Scroll {
                    vertical: -notches * 15.0,
                    horizontal: 0.0,
                })
            }
            // Touchpad-mode / mobile clients send relative motion and 2-axis scroll.
            MouseEvent::RelMove { x, y } => Some(InputCommand::MoveRel {
                dx: x as f64 * rel_scale,
                dy: y as f64 * rel_scale,
            }),
            MouseEvent::Scroll { x, y } => Some(InputCommand::Scroll {
                vertical: -(y as f64) / 120.0 * 15.0,
                horizontal: (x as f64) / 120.0 * 15.0,
            }),
        };
        if let Some(cmd) = cmd {
            let _ = self.tx.send(cmd);
        }
    }
}

fn button(btn: u32, pressed: bool) -> InputCommand {
    InputCommand::Button {
        button: btn,
        pressed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipewire_capture::TargetSize;

    const NATIVE: (u32, u32) = (1920, 1080);

    fn forwarder(served: TargetSize) -> InputForwarder {
        let (tx, _rx) = std::sync::mpsc::channel();
        InputForwarder {
            tx,
            native: NATIVE,
            served,
        }
    }

    /// A client whose box matches the output's aspect gets its box verbatim,
    /// filled edge to edge — no bars, nothing to map around.
    #[test]
    fn a_matching_aspect_fills_the_desktop() {
        let layout = served_layout(NATIVE, (1280, 720), None, false);
        assert_eq!(layout.desktop, (1280, 720));
        assert_eq!(layout.img_size, (1280, 720));
        assert_eq!(layout.img_off, (0, 0));
        assert_eq!(layout.input_box, (1280, 720));
    }

    /// A squarer box keeps the picture's aspect and centers it, leaving equal
    /// black bars above and below.
    #[test]
    fn a_mismatched_aspect_is_letterboxed_and_centred() {
        let layout = served_layout(NATIVE, (800, 800), None, false);
        assert_eq!(layout.desktop, (800, 800));
        assert_eq!(layout.img_size, (800, 450), "16:9 inside an 800px-wide box");
        assert_eq!(layout.img_off, (0, 175), "bars split evenly");
    }

    /// A client bigger than the output is not served an upscaled picture — the
    /// native image sits pillar- and letter-boxed in the middle.
    #[test]
    fn a_larger_client_never_upscales_the_picture() {
        let layout = served_layout((1280, 720), (3840, 2160), None, false);
        assert_eq!(layout.desktop, (3840, 2160));
        assert_eq!(layout.img_size, (1280, 720));
        assert_eq!(layout.img_off, (1280, 720));
    }

    /// H.264 cannot encode odd dimensions, so the EGFX desktop rounds down —
    /// but mouse coordinates still arrive in the box the client reported, so
    /// `input_box` keeps the odd size.
    #[test]
    fn the_egfx_desktop_is_rounded_down_to_even() {
        let layout = served_layout(NATIVE, (1281, 721), None, true);
        assert_eq!(layout.desktop, (1280, 720));
        assert_eq!(layout.input_box, (1281, 721));

        let bitmap = served_layout(NATIVE, (1281, 721), None, false);
        assert_eq!(
            bitmap.desktop,
            (1281, 721),
            "the bitmap path serves odd sizes as-is"
        );
    }

    /// `--desktop` replaces the served size, but not the space mouse
    /// coordinates arrive in.
    #[test]
    fn the_desktop_override_leaves_the_input_box_alone() {
        let layout = served_layout(NATIVE, (800, 600), Some((1600, 1200)), false);
        assert_eq!(layout.desktop, (1600, 1200));
        assert_eq!(layout.input_box, (800, 600));
    }

    /// The remote cursor lands where the user pointed: the middle of the
    /// client's box is the middle of the output.
    #[test]
    fn the_centre_of_the_client_box_is_the_centre_of_the_output() {
        let served = TargetSize::native();
        served.set(served_layout(NATIVE, (800, 800), None, false));
        assert_eq!(forwarder(served).map_abs(400, 400), (960, 540));
    }

    /// A click in a black bar has no picture under it; it clamps to the
    /// nearest edge of the picture rather than mapping off-screen.
    #[test]
    fn positions_in_the_bars_clamp_to_the_picture_edge() {
        let served = TargetSize::native();
        served.set(served_layout(NATIVE, (800, 800), None, false));
        let forwarder = forwarder(served);

        assert_eq!(forwarder.map_abs(400, 10), (960, 0), "top bar");
        assert_eq!(forwarder.map_abs(400, 790), (960, 1078), "bottom bar");
    }

    /// Every position maps inside the output, corner included.
    #[test]
    fn the_far_corner_stays_inside_the_output() {
        let served = TargetSize::native();
        served.set(served_layout(NATIVE, (800, 800), None, false));
        let (x, y) = forwarder(served).map_abs(799, 799);
        assert!(x < NATIVE.0, "{x} inside {}", NATIVE.0);
        assert!(y < NATIVE.1, "{y} inside {}", NATIVE.1);
    }

    /// Before any client has negotiated a layout, coordinates pass through
    /// unscaled and are clamped to the output.
    #[test]
    fn coordinates_pass_through_until_a_client_negotiates() {
        let forwarder = forwarder(TargetSize::native());
        assert_eq!(forwarder.map_abs(100, 200), (100, 200));
        assert_eq!(forwarder.map_abs(5000, 5000), (1919, 1079));
    }
}
