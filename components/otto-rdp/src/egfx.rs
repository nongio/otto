//! EGFX (RDP Graphics Pipeline) driver for the hardware H.264 path.
//!
//! `ironrdp-server`'s EGFX support gives us the AVC420 *wire path* but does no
//! encoding — we feed it an already-encoded H.264 bitstream (from `h264.rs`).
//! This module wires the two together:
//!
//! * [`OttoGfxFactory`] plugs into `RdpServer::builder().with_gfx_factory(..)`.
//!   The server calls [`ServerEventSender::set_sender`] to hand us the event
//!   channel, and [`GfxServerFactory::build_server_with_handle`] once a client
//!   negotiates the channel — we stash the shared [`GfxServerHandle`] so the
//!   driver can push frames into it.
//! * [`drive`] is the pump: for every encoded access unit it creates/maps a
//!   surface (once), calls `send_avc420_frame`, drains the queued PDUs, frames
//!   them for the DVC, and emits them via [`ServerEvent::Egfx`].
//!
//! Frame sending is keyframe-gated: the first forwarded frame — and the first
//! after any drop (backpressure, not-ready) — must be an IDR, so the client's
//! decoder never receives a P-frame referencing a frame it never got.

use std::sync::{Arc, Mutex};

use ironrdp_dvc::{encode_dvc_messages, DvcMessage};
use ironrdp_egfx::pdu::{
    Avc420Region, CapabilitiesAdvertisePdu, CapabilitiesV103Flags, CapabilitiesV104Flags,
    CapabilitiesV107Flags, CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV8Flags,
    CapabilitySet,
};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_server::{
    EgfxServerMessage, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent,
    ServerEventSender,
};
use ironrdp_svc::ChannelFlags;
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::h264::{EncodedFrame, KeyframeRequester};

/// Region-metadata quantization hint sent alongside each AVC420 frame. The real
/// quality lives in the encoded H.264; this is advisory (0–51, lower = better).
const REGION_QP: u8 = 26;

/// Which transport a connected client gets, decided from its EGFX capability
/// advertisement. Clients that disable AVC (e.g. Microsoft's mobile / Windows
/// App clients) can't decode our H.264, so they auto-fall back to the bitmap
/// path served on the main channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Codec {
    /// No client connected yet / capabilities not advertised.
    Unknown,
    /// Client accepts H.264/AVC420 → hardware EGFX path.
    Avc,
    /// Client disabled AVC → legacy bitmap path.
    Bitmap,
}

/// Does this advertised capability set enable AVC420/AVC444?
///
/// V8.1 opts *in* via `AVC420_ENABLED`; V10+ is opt-*out* via `AVC_DISABLED`.
fn cap_enables_avc(cap: &CapabilitySet) -> bool {
    match cap {
        CapabilitySet::V8_1 { flags } => flags.contains(CapabilitiesV81Flags::AVC420_ENABLED),
        CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => {
            !flags.contains(CapabilitiesV10Flags::AVC_DISABLED)
        }
        CapabilitySet::V10_3 { flags } => !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
        CapabilitySet::V10_4 { flags }
        | CapabilitySet::V10_5 { flags }
        | CapabilitySet::V10_6 { flags }
        | CapabilitySet::V10_6Err { flags } => !flags.contains(CapabilitiesV104Flags::AVC_DISABLED),
        CapabilitySet::V10_7 { flags } => !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
        CapabilitySet::V8 { .. } | CapabilitySet::V10_1 => false,
    }
}

/// State shared between the RDP server threads (which build the graphics-
/// pipeline server) and the async driver task (which pushes frames into it).
pub struct GfxShared {
    /// The server event channel, handed over by `set_sender`.
    sender: Mutex<Option<UnboundedSender<ServerEvent>>>,
    /// The per-connection graphics-pipeline server. Replaced on reconnect; the
    /// driver detects the swap by pointer identity and rebuilds its surface.
    handle: Mutex<Option<GfxServerHandle>>,
    /// The negotiated desktop size (the client's box). Set synchronously during
    /// `request_initial_size`, before the EGFX channel is ready, so the driver
    /// (already polling) can stand the surface up the instant it becomes ready.
    desktop: Mutex<Option<(u16, u16)>>,
    /// Signals that `desktop` has been set (wakes the encoder coordinator).
    desktop_ready: tokio::sync::Notify,
    /// Which transport the connected client gets (AVC vs bitmap fallback),
    /// decided from its capability advertisement. Watched by the encoder
    /// coordinator, the bitmap coordinator, and the display's update loop.
    codec: tokio::sync::watch::Sender<Codec>,
}

impl Default for GfxShared {
    fn default() -> Self {
        Self::new()
    }
}

impl GfxShared {
    pub fn new() -> Self {
        Self {
            sender: Mutex::new(None),
            handle: Mutex::new(None),
            desktop: Mutex::new(None),
            desktop_ready: tokio::sync::Notify::new(),
            codec: tokio::sync::watch::channel(Codec::Unknown).0,
        }
    }

    fn sender(&self) -> Option<UnboundedSender<ServerEvent>> {
        self.sender.lock().unwrap().clone()
    }
    fn handle(&self) -> Option<GfxServerHandle> {
        self.handle.lock().unwrap().clone()
    }
    fn desktop(&self) -> Option<(u16, u16)> {
        *self.desktop.lock().unwrap()
    }

    /// Record the negotiated desktop size and wake the encoder coordinator.
    pub fn set_desktop(&self, width: u16, height: u16) {
        *self.desktop.lock().unwrap() = Some((width, height));
        self.desktop_ready.notify_one();
    }

    /// Await the first negotiated desktop size (returns immediately if already set).
    pub async fn wait_desktop(&self) -> (u16, u16) {
        loop {
            if let Some(d) = self.desktop() {
                return d;
            }
            self.desktop_ready.notified().await;
        }
    }

    /// Current codec decision (`Unknown` until the client advertises caps).
    fn codec(&self) -> Codec {
        *self.codec.borrow()
    }

    /// Record the codec decision from the client's capability advertisement.
    fn set_codec(&self, codec: Codec) {
        self.codec.send_replace(codec);
    }

    /// A receiver for the codec decision (for the display's update loop).
    pub fn codec_rx(&self) -> tokio::sync::watch::Receiver<Codec> {
        self.codec.subscribe()
    }

    /// Await the resolved codec decision (blocks while `Unknown`).
    pub async fn wait_codec(&self) -> Codec {
        let mut rx = self.codec.subscribe();
        loop {
            let c = *rx.borrow_and_update();
            if c != Codec::Unknown {
                return c;
            }
            if rx.changed().await.is_err() {
                return Codec::Bitmap;
            }
        }
    }
}

/// Factory handed to `RdpServer::builder().with_gfx_factory(..)`.
pub struct OttoGfxFactory {
    shared: Arc<GfxShared>,
}

impl OttoGfxFactory {
    pub fn new(shared: Arc<GfxShared>) -> Self {
        Self { shared }
    }
}

impl ServerEventSender for OttoGfxFactory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.shared.sender.lock().unwrap() = Some(sender);
    }
}

impl GfxServerFactory for OttoGfxFactory {
    fn build_gfx_handler(&self) -> Box<dyn GraphicsPipelineHandler> {
        Box::new(OttoGfxHandler::new(Arc::clone(&self.shared)))
    }

    fn build_server_with_handle(&self) -> Option<(GfxDvcBridge, GfxServerHandle)> {
        let server =
            GraphicsPipelineServer::new(Box::new(OttoGfxHandler::new(Arc::clone(&self.shared))));
        let handle: GfxServerHandle = Arc::new(Mutex::new(server));
        *self.shared.handle.lock().unwrap() = Some(Arc::clone(&handle));
        Some((GfxDvcBridge::new(Arc::clone(&handle)), handle))
    }
}

/// Callbacks from the graphics-pipeline server. The key one is
/// `capabilities_advertise`: it decides AVC vs bitmap for this client from the
/// client's own advertisement (the server's confirmed caps can't be trusted —
/// ironrdp confirms V10 with AVC enabled even when the client disabled it).
struct OttoGfxHandler {
    shared: Arc<GfxShared>,
}

impl OttoGfxHandler {
    fn new(shared: Arc<GfxShared>) -> Self {
        Self { shared }
    }
}

impl GraphicsPipelineHandler for OttoGfxHandler {
    fn capabilities_advertise(&mut self, pdu: &CapabilitiesAdvertisePdu) {
        let mut avc = false;
        for raw in &pdu.0 {
            if let Ok(Some(cap)) = raw.parsed() {
                avc |= cap_enables_avc(&cap);
                tracing::debug!("EGFX client advertised {cap:?}");
            }
        }
        if avc {
            tracing::info!("client accepts AVC420 → serving hardware H.264 over EGFX");
            self.shared.set_codec(Codec::Avc);
        } else {
            tracing::info!("client disabled AVC → auto-falling back to the bitmap path");
            self.shared.set_codec(Codec::Bitmap);
        }
    }

    /// Choose what the server confirms. For a client that disabled AVC, confirm
    /// a matching AVC-disabled capability — confirming an AVC-enabled version to
    /// such a client contradicts its advertisement and makes it close the
    /// channel (and the connection). `capabilities_advertise` runs first, so the
    /// codec decision is already recorded here.
    fn preferred_capabilities(&self) -> Vec<CapabilitySet> {
        if self.shared.codec() == Codec::Bitmap {
            vec![
                CapabilitySet::V10 {
                    flags: CapabilitiesV10Flags::SMALL_CACHE | CapabilitiesV10Flags::AVC_DISABLED,
                },
                CapabilitySet::V8_1 {
                    flags: CapabilitiesV81Flags::SMALL_CACHE,
                },
                CapabilitySet::V8 {
                    flags: CapabilitiesV8Flags::SMALL_CACHE,
                },
            ]
        } else {
            vec![
                CapabilitySet::V10_7 {
                    flags: CapabilitiesV107Flags::SMALL_CACHE,
                },
                CapabilitySet::V10 {
                    flags: CapabilitiesV10Flags::SMALL_CACHE,
                },
                CapabilitySet::V8_1 {
                    flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
                },
                CapabilitySet::V8 {
                    flags: CapabilitiesV8Flags::SMALL_CACHE,
                },
            ]
        }
    }

    fn on_ready(&mut self, negotiated: &CapabilitySet) {
        tracing::info!("EGFX channel ready; server CONFIRMED {negotiated:?} to the client");
    }

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32, total_frames_decoded: u32) {
        tracing::trace!(
            "EGFX frame {frame_id} acked (queue {queue_depth}, decoded {total_frames_decoded})"
        );
    }
}

/// Pump encoded access units into the graphics-pipeline server for as long as
/// the encoder feeds us. Returns when the encoded-frame channel closes.
///
/// A short timer also runs alongside the frame stream: as soon as the channel
/// is ready, the surface is created and `ResetGraphics`/`CreateSurface`/
/// `MapSurfaceToOutput` are pushed — *without* waiting for the first encoded
/// frame. Some clients (notably the Microsoft mobile / Windows App clients)
/// disconnect if graphics setup lags the capability confirm, and the first
/// encoded frame can be tens of milliseconds out.
pub async fn drive(
    shared: Arc<GfxShared>,
    mut frames: mpsc::UnboundedReceiver<EncodedFrame>,
    keyframe: KeyframeRequester,
) {
    let mut driver = Driver::new(shared, keyframe);
    let mut ready_tick = tokio::time::interval(std::time::Duration::from_millis(5));
    ready_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!("EGFX driver started; polling for an AVC420 client");

    loop {
        tokio::select! {
            // Proactive: stand the surface up the instant the channel is ready.
            _ = ready_tick.tick() => {
                driver.ensure_surface();
            }
            frame = frames.recv() => {
                match frame {
                    Some(frame) => driver.on_frame(&frame),
                    None => break,
                }
            }
        }
    }

    tracing::info!("EGFX driver stopping: encoder channel closed");
}

/// Per-connection EGFX pump state. Reset whenever the graphics-pipeline server
/// handle is swapped (a client reconnect builds a fresh one).
struct Driver {
    shared: Arc<GfxShared>,
    /// Surface dimensions, taken from the negotiated desktop when the surface
    /// is created; the encoder produces frames at exactly this size.
    dims: (u16, u16),
    start: std::time::Instant,
    current: Option<GfxServerHandle>,
    surface: Option<u16>,
    awaiting_keyframe: bool,
    warned_no_avc: bool,
    sent: u64,
    /// Asks the encoder for an IDR when the client has nothing decodable yet.
    keyframe: KeyframeRequester,
    /// When the last IDR was requested, so a long keyframe wait re-asks
    /// without spamming the encoder every frame.
    last_keyframe_request: Option<std::time::Instant>,
}

/// Don't re-ask the encoder for an IDR more often than this.
const KEYFRAME_REQUEST_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

impl Driver {
    fn new(shared: Arc<GfxShared>, keyframe: KeyframeRequester) -> Self {
        Self {
            shared,
            dims: (0, 0),
            start: std::time::Instant::now(),
            current: None,
            surface: None,
            awaiting_keyframe: true,
            warned_no_avc: false,
            sent: 0,
            keyframe,
            last_keyframe_request: None,
        }
    }

    /// Ask the encoder for an immediate IDR (rate-limited). Called whenever the
    /// client has nothing it can decode: a surface was just mapped, or a frame
    /// was dropped and the stream must resume on a keyframe. Without this the
    /// client waits for the encoder's own `key-int-max`, which counts *frames* —
    /// on an idle desktop that is seconds to minutes of black screen.
    fn request_keyframe(&mut self) {
        let now = std::time::Instant::now();
        if self
            .last_keyframe_request
            .is_some_and(|t| now.duration_since(t) < KEYFRAME_REQUEST_INTERVAL)
        {
            return;
        }
        if self.keyframe.request() {
            self.last_keyframe_request = Some(now);
            tracing::debug!("requested an IDR from the encoder (client needs a full frame)");
        }
    }

    /// Ensure a mapped surface exists for the current connection, creating it
    /// (and emitting the setup PDUs) if the channel is ready. Returns the
    /// handle + sender + surface id when graphics can be sent, else `None`.
    fn ensure_surface(&mut self) -> Option<(GfxServerHandle, UnboundedSender<ServerEvent>, u16)> {
        // Only the AVC path uses EGFX surfaces. A client that disabled AVC is
        // served bitmaps on the main channel, so leave its EGFX channel idle.
        if self.shared.codec() != Codec::Avc {
            return None;
        }
        let handle = self.shared.handle()?;
        let sender = self.shared.sender()?;
        // The desktop size is negotiated before the channel is ready; without it
        // there is nothing to size the surface to yet.
        let (w, h) = self.shared.desktop()?;

        // Reconnect: a fresh server replaced the old one — drop stale state.
        if self
            .current
            .as_ref()
            .is_none_or(|c| !Arc::ptr_eq(c, &handle))
        {
            self.current = Some(Arc::clone(&handle));
            self.surface = None;
            self.awaiting_keyframe = true;
            self.warned_no_avc = false;
        }

        // Fast path: the surface is already up.
        if let Some(id) = self.surface {
            return Some((handle, sender, id));
        }

        let (messages, channel_id) = {
            let mut server = handle.lock().unwrap();
            if !server.is_ready() {
                self.awaiting_keyframe = true;
                return None;
            }
            if !server.supports_avc420() {
                if !self.warned_no_avc {
                    self.warned_no_avc = true;
                    tracing::warn!(
                        "connected client did not negotiate AVC420 over EGFX — no video will \
                         render on the H.264 path. Reconnect with an AVC420-capable client \
                         (mstsc, or FreeRDP with /gfx:avc420), or restart otto-rdp with --bitmap."
                    );
                }
                return None;
            }

            server.set_output_dimensions(w, h);
            let id = server.create_surface(w, h)?;
            server.map_surface_to_output(id, 0, 0);
            self.surface = Some(id);
            self.dims = (w, h);
            tracing::info!("EGFX surface {id} created and mapped ({w}x{h})");
            (server.drain_output(), server.channel_id())
        };

        // Emit ResetGraphics + CreateSurface + MapSurfaceToOutput now.
        Self::emit(&sender, channel_id, messages);
        // The surface is empty (black) until it receives a decodable frame, so
        // ask for an IDR immediately instead of waiting for the encoder's next
        // scheduled one — that is what makes a fresh connection show the
        // desktop without the user having to move the mouse first.
        self.request_keyframe();
        Some((handle, sender, self.surface.unwrap()))
    }

    /// Encode one access unit into the graphics pipeline.
    fn on_frame(&mut self, frame: &EncodedFrame) {
        let Some((handle, sender, surface_id)) = self.ensure_surface() else {
            self.awaiting_keyframe = true;
            return;
        };

        // Never begin or resume on a P-frame the client can't decode.
        if self.awaiting_keyframe {
            if !frame.keyframe {
                // Nothing is reaching the client meanwhile — keep nudging the
                // encoder (rate-limited) rather than waiting out key-int-max.
                self.request_keyframe();
                return;
            }
            self.awaiting_keyframe = false;
        }

        let (w, h) = self.dims;
        let (messages, channel_id) = {
            let mut server = handle.lock().unwrap();
            let ts = self.start.elapsed().as_millis() as u32;
            let region = Avc420Region::full_frame(w, h, REGION_QP);
            if server
                .send_avc420_frame(surface_id, &frame.data, &[region], ts)
                .is_none()
            {
                // Backpressure or not-ready: skip this AU and wait for the next
                // keyframe so the decoder never sees a dangling reference.
                self.awaiting_keyframe = true;
                return;
            }
            (server.drain_output(), server.channel_id())
        };

        Self::emit(&sender, channel_id, messages);
        if self.sent < 3 || self.sent.is_multiple_of(120) {
            tracing::info!(
                "sent AVC420 frame #{} to RDP client ({} bytes{})",
                self.sent,
                frame.data.len(),
                if frame.keyframe { ", keyframe" } else { "" }
            );
        }
        self.sent += 1;
    }

    /// Frame the drained DVC messages and hand them to the server event loop.
    fn emit(
        sender: &UnboundedSender<ServerEvent>,
        channel_id: Option<u32>,
        messages: Vec<DvcMessage>,
    ) {
        let Some(channel_id) = channel_id else {
            return;
        };
        match encode_dvc_messages(channel_id, messages, ChannelFlags::empty()) {
            Ok(messages) => {
                let _ = sender.send(ServerEvent::Egfx(EgfxServerMessage::SendMessages {
                    messages,
                }));
            }
            Err(e) => tracing::warn!("failed to frame EGFX DVC messages: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// V8.1 opts *in* to AVC420: without the flag the client gets bitmaps.
    #[test]
    fn v8_1_opts_in_to_avc() {
        assert!(cap_enables_avc(&CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
        }));
        assert!(!cap_enables_avc(&CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::SMALL_CACHE,
        }));
    }

    /// V10 and later opt *out*: silence means AVC is fine. Microsoft's mobile
    /// clients set `AVC_DISABLED`, and confirming AVC to them drops the
    /// connection — so this is the flag that decides their transport.
    #[test]
    fn v10_and_later_opt_out_of_avc() {
        assert!(cap_enables_avc(&CapabilitySet::V10 {
            flags: CapabilitiesV10Flags::SMALL_CACHE,
        }));
        assert!(!cap_enables_avc(&CapabilitySet::V10 {
            flags: CapabilitiesV10Flags::AVC_DISABLED,
        }));
        assert!(cap_enables_avc(&CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::empty(),
        }));
        assert!(!cap_enables_avc(&CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::AVC_DISABLED,
        }));
    }

    /// Versions with no AVC concept at all never take the hardware path.
    #[test]
    fn versions_without_avc_fall_back_to_bitmaps() {
        assert!(!cap_enables_avc(&CapabilitySet::V8 {
            flags: CapabilitiesV8Flags::empty(),
        }));
        assert!(!cap_enables_avc(&CapabilitySet::V10_1));
    }
}
