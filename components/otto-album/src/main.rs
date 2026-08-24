//! Otto Album — a vinyl session on your desktop.
//!
//! A layer-shell widget: the album standing behind a turntable, the record
//! turning at 33⅓ while a player plays. What is on the platter comes from
//! MPRIS; with no player running it falls back to a bundled example so the
//! scene can be worked on offline.

mod cover;
mod disc;
mod motion;
mod mpris;
mod shrinkwrap;
mod stage;
mod tonearm;
mod track;
mod turntable;
mod vinyl;

use motion::Motion;
use mpris::Mpris;
use otto_kit::prelude::*;
use otto_kit::surfaces::LayerShellSurface;
use skia_safe::Image;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use track::Track;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
    zwlr_layer_surface_v1::KeyboardInteractivity,
};

const W: i32 = stage::W as i32;
const H: i32 = stage::H as i32;

struct MusicApp {
    layer: Option<LayerShellSurface>,
    /// Set once the surface has been configured and its frame loop started.
    running: Rc<RefCell<bool>>,
    mpris: Mpris,
    track: Arc<Mutex<Track>>,
    motion: Arc<Mutex<Motion>>,
    /// The art generation already decoded into `track.cover`.
    art_generation: u64,
    /// What was on screen last paint, so a change repaints a still deck.
    painted: Option<(String, String, u64, bool)>,
    /// When that paint happened, to notice frame callbacks drying up.
    last_paint: std::cell::Cell<std::time::Instant>,
    /// Paints since start. The first buffer a layer surface commits can be
    /// dropped — it races the configure/ack handshake — and a deck with
    /// nothing playing never animates, so a single lost paint leaves the
    /// widget blank forever. A short warm-up guarantees a live buffer.
    paints: std::cell::Cell<u32>,
}

impl App for MusicApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        // Desktop widgets belong on the bottom layer; OTTO_ALBUM_LAYER lifts
        // it above windows while working on the visuals.
        let which = match std::env::var("OTTO_ALBUM_LAYER").as_deref() {
            Ok("overlay") => Layer::Overlay,
            Ok("top") => Layer::Top,
            _ => Layer::Bottom,
        };
        let layer = LayerShellSurface::new(which, "otto-album", W as u32, H as u32)?;
        layer.set_anchor(Anchor::Bottom | Anchor::Right);
        layer.set_margin(0, 36, 36, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        self.layer = Some(layer);
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, _w: i32, _h: i32, _serial: u32) {
        if *self.running.borrow() {
            return;
        }
        if self.layer.is_none() {
            return;
        }
        *self.running.borrow_mut() = true;
        tracing::info!("layer configured at {}x{}", _w, _h);

        // Painting is driven from `on_update`, not from a frame callback:
        // `draw` itself asks for the next frame, and drawing from inside a
        // frame callback re-enters the runner's callback map.
        self.paint();
    }

    /// Poll the player and keep the scene in step with it.
    fn on_update(&mut self, _ctx: &AppContext) {
        let snapshot = self.mpris.snapshot();
        tracing::trace!(
            connected = snapshot.connected,
            title = %snapshot.title,
            "update"
        );

        if snapshot.connected && !snapshot.title.is_empty() {
            if let Ok(mut track) = self.track.lock() {
                track.title = snapshot.title.clone();
                track.artist = snapshot.artist.clone();
                track.album = snapshot.album.clone();
                track.length = snapshot.length;
                track.position = snapshot.position();
                track.playing = snapshot.playing;

                // Decode cover art only when the player moved to a new one.
                if snapshot.art_generation != self.art_generation {
                    self.art_generation = snapshot.art_generation;
                    track.cover = snapshot
                        .art
                        .as_ref()
                        .and_then(|bytes| Image::from_encoded(skia_safe::Data::new_copy(bytes)))
                        .or_else(cover::bundled_cover);
                    // The label scan belongs to the pressing, not the stream:
                    // until that lookup exists, only show it for the record it
                    // actually came from.
                    track.label = None;
                }
            }

            if let Ok(mut motion) = self.motion.lock() {
                motion.playing = snapshot.playing;
            }
        }

        // OTTO_ALBUM_DEMO spins the deck with no player attached, so the
        // widget can be used as a known-good source of continuous damage
        // when testing the compositor.
        if std::env::var("OTTO_ALBUM_DEMO").is_ok() {
            if let Ok(mut track) = self.track.lock() {
                track.playing = true;
                track.position += 16_000;
            }
            if let Ok(mut motion) = self.motion.lock() {
                motion.playing = true;
            }
        }

        // Advance the platter and the arm, then paint if anything moved.
        let animating = match self.motion.lock() {
            Ok(mut motion) => {
                motion.step();
                motion.is_animating()
            }
            Err(_) => false,
        };

        // A stopped deck still has to repaint when the track changes: nothing
        // is moving, so `is_animating` alone would leave the last frame up.
        let current = self.track.lock().ok().map(|track| {
            (
                track.title.clone(),
                track.album.clone(),
                track.position / 1_000_000,
                track.playing,
            )
        });
        let changed = current != self.painted;
        if changed {
            self.painted = current;
        }

        // WARMUP_PAINTS covers the handshake window described on `paints`.
        const WARMUP_PAINTS: u32 = 3;
        let warming_up = self.paints.get() < WARMUP_PAINTS;
        if animating || changed || warming_up {
            self.paint();
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        Some(Duration::from_millis(16))
    }

    fn on_pointer_event(
        &mut self,
        _ctx: &AppContext,
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        use smithay_client_toolkit::seat::pointer::PointerEventKind;

        let mut repaint = false;
        for event in events {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if let Ok(mut motion) = self.motion.lock() {
                        let over = stage::play_hit(x, y);
                        repaint |= over != motion.hovering_play;
                        motion.hovering_play = over;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if let Ok(mut motion) = self.motion.lock() {
                        repaint |= motion.hovering_play;
                        motion.hovering_play = false;
                    }
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    if stage::play_hit(x, y) {
                        // Move now, and let the player's next report confirm:
                        // waiting a poll for the platter to react feels broken.
                        if let Ok(mut motion) = self.motion.lock() {
                            motion.toggle();
                        }
                        self.mpris.play_pause();
                        repaint = true;
                    }
                }
                _ => {}
            }
        }
        if repaint {
            self.paint();
        }
    }
}

impl MusicApp {
    fn paint(&self) {
        let Some(layer) = &self.layer else { return };
        // Normally we wait for the last frame to be presented before painting
        // the next one. But the compositor only sends frame callbacks to layer
        // surfaces it actually rendered, so a widget that is occluded — or one
        // whose callbacks stall for any other reason — would never paint again
        // and freeze on whatever was up when it went quiet.
        //
        // So the wait has a deadline: with callbacks flowing this throttles to
        // the refresh rate, and without them it still paints at ~20fps.
        // A record at 33rpm does not need the refresh rate, and this widget
        // spends most of its life behind other windows, so the paint rate is
        // capped whatever the compositor says.
        // 15fps, not the refresh rate: a 33rpm label turns once every 1.8s,
        // so a frame every 66ms moves it 13° — smooth at this size, and half
        // the compositing work of painting at 30. This widget animates
        // continuously whenever something is playing, so the rate it picks is
        // a cost the whole desktop pays.
        const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(66);
        const STALL_DEADLINE: std::time::Duration = std::time::Duration::from_millis(132);
        let since = self.last_paint.get().elapsed();
        if since < MIN_INTERVAL {
            return;
        }
        // OTTO_ALBUM_STRICT=1 drops the deadline and paints purely on frame
        // callbacks, which is what a well-behaved widget should do: occluded
        // means no callbacks means no GPU. It is a diagnostic until the
        // compositor delivers those callbacks reliably on every layer.
        let strict = std::env::var("OTTO_ALBUM_STRICT").is_ok();
        if layer.base_surface().frame_in_flight() && (strict || since < STALL_DEADLINE) {
            return;
        }
        self.last_paint.set(std::time::Instant::now());
        self.paints.set(self.paints.get().saturating_add(1));
        tracing::debug!("paint");
        let (Ok(track), Ok(motion)) = (self.track.lock(), self.motion.lock()) else {
            return;
        };
        layer.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            stage::draw_with(canvas, &track, &motion, false);
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "otto_album=info".into()),
        )
        .init();

    let mpris = Mpris::spawn();
    let playing = mpris.snapshot().playing;

    AppRunner::new(MusicApp {
        layer: None,
        running: Rc::new(RefCell::new(false)),
        mpris,
        track: Arc::new(Mutex::new(Track::example())),
        motion: Arc::new(Mutex::new(Motion::new(playing))),
        art_generation: 0,
        painted: None,
        paints: std::cell::Cell::new(0),
        // Far enough back that the first paint — which happens as soon as
        // the layer is configured, well inside one frame of construction —
        // is not swallowed by the min-interval gate below.
        last_paint: std::cell::Cell::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        ),
    })
    .run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dev helper: render the scene offscreen so the layout can be checked
    /// without a compositor.
    ///     WINDOW_OUT=/tmp/win.png [NO_BACKDROP=1] [PAUSED=1] [ANGLE=…] \
    ///         cargo test -p otto-album window
    #[test]
    fn window() {
        let Ok(out) = std::env::var("WINDOW_OUT") else {
            return;
        };
        let track = Track::example();
        let mut motion = Motion::new(std::env::var("PAUSED").is_err());
        motion.angle = std::env::var("ANGLE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        motion.lift = if std::env::var("PAUSED").is_ok() {
            1.0
        } else {
            0.0
        };

        let mut surface = skia_safe::surfaces::raster_n32_premul((W, H)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let backdrop = std::env::var("NO_BACKDROP").is_err();
        stage::draw_with(surface.canvas(), &track, &motion, backdrop);

        let data = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .expect("encode window");
        std::fs::write(out, data.as_bytes()).expect("write window");
    }
}
