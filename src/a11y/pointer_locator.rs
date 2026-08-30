//! `org.freedesktop.a11y.PointerLocator` — where the pointer is.
//!
//! The sibling of [`super::keyboard_monitor`] on the same object, and expected
//! there: at-spi2-core builds one device from both interfaces and asks this one
//! while starting up, so a compositor serving only the keyboard half answers
//! Orca with `UnknownInterface` — and Orca then crashes rather than degrading.
//!
//! A Wayland client cannot ask where the pointer is; it hears about the pointer
//! only while the pointer is over its own surfaces. Mouse review — reading
//! whatever is under the pointer — is what wants this.
//!
//! The shape is not ours to choose. `QueryPointer` answers with the position,
//! and `PointerPositionChanged` carries nothing at all: it says only that the
//! pointer moved, and the listener asks again. Both match mutter's, which is
//! what at-spi2-core was written against.
//!
//! Position is published rather than requested: the compositor writes it into
//! an atomic as the pointer moves, and the D-Bus task reads that. Nothing about
//! answering an assistive technology is allowed onto the input path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::trace;
use zbus::zvariant::OwnedValue;
use zbus::{fdo, interface, Connection, SignalContext};

use super::keyboard_monitor::OBJECT_PATH;

/// How often the "it moved" signal may go out. The pointer produces hundreds of
/// events a second and the signal carries no detail, so anything faster is
/// D-Bus traffic that tells a listener nothing it did not already know.
const POKE_INTERVAL: Duration = Duration::from_millis(50);

/// The pointer's position, shared between the compositor thread that writes it
/// and the D-Bus task that reads it.
#[derive(Clone)]
pub struct PointerLocatorHandle {
    /// Both coordinates in one atomic, so a reader can never take one from
    /// before a move and the other from after. `f32` bits: exact for every
    /// pixel coordinate a display can have.
    position: Arc<AtomicU64>,
    /// Set once something has asked where the pointer is. Until then the moves
    /// are not worth telling anyone about.
    wanted: Arc<AtomicBool>,
    /// When the last "it moved" went out, as nanoseconds since the handle was
    /// made — `Instant` is not something an atomic can hold. Zero means never.
    last_poke: Arc<AtomicU64>,
    started: Instant,
    pokes: UnboundedSender<()>,
}

impl PointerLocatorHandle {
    pub fn new() -> (Self, UnboundedReceiver<()>) {
        let (pokes, receiver) = unbounded_channel();
        (
            Self {
                position: Arc::new(AtomicU64::new(0)),
                wanted: Arc::new(AtomicBool::new(false)),
                last_poke: Arc::new(AtomicU64::new(0)),
                started: Instant::now(),
                pokes,
            },
            receiver,
        )
    }

    /// Records where the pointer now is, in logical pixels.
    ///
    /// Called from the input path: two relaxed atomic operations when nothing
    /// is listening, which is every session without an assistive technology.
    pub fn set(&self, x: f64, y: f64) {
        let packed = ((x as f32).to_bits() as u64) << 32 | (y as f32).to_bits() as u64;
        self.position.store(packed, Ordering::Relaxed);

        if !self.wanted.load(Ordering::Relaxed) {
            return;
        }

        // Zero means nothing has been announced yet: the first move after
        // something asks always goes out, however soon it lands.
        let now = self.started.elapsed().as_nanos().max(1) as u64;
        let last = self.last_poke.load(Ordering::Relaxed);
        if last != 0 && now.saturating_sub(last) < POKE_INTERVAL.as_nanos() as u64 {
            return;
        }
        self.last_poke.store(now, Ordering::Relaxed);
        let _ = self.pokes.send(());
    }

    fn get(&self) -> (f64, f64) {
        let packed = self.position.load(Ordering::Relaxed);
        (
            f32::from_bits((packed >> 32) as u32) as f64,
            f32::from_bits(packed as u32) as f64,
        )
    }
}

struct PointerLocator {
    handle: PointerLocatorHandle,
}

#[interface(name = "org.freedesktop.a11y.PointerLocator")]
impl PointerLocator {
    /// Where the pointer is, in logical pixels across the whole layout.
    ///
    /// `app_data` is the interface's room for anything a compositor wants to
    /// add; Otto has nothing to add, and sends it empty.
    async fn query_pointer(&self) -> fdo::Result<(HashMap<String, OwnedValue>, f64, f64)> {
        // The first ask is what turns the movement signal on: until something
        // has looked, the pointer moving is not news.
        self.handle.wanted.store(true, Ordering::Relaxed);

        let (x, y) = self.handle.get();
        trace!(x, y, "a11y: QueryPointer");
        Ok((HashMap::new(), x, y))
    }

    /// The pointer moved. Deliberately empty — a listener that cares calls
    /// `QueryPointer`, so a stale signal cannot report a stale position.
    #[zbus(signal)]
    async fn pointer_position_changed(context: &SignalContext<'_>) -> zbus::Result<()>;
}

/// Serves the interface alongside the keyboard monitor, on the same object, and
/// drains the movement pokes onto the bus.
pub async fn register(
    connection: &Connection,
    handle: PointerLocatorHandle,
    mut pokes: UnboundedReceiver<()>,
) -> zbus::Result<()> {
    connection
        .object_server()
        .at(OBJECT_PATH, PointerLocator { handle })
        .await?;

    let signal_connection = connection.clone();
    tokio::spawn(async move {
        let context = match SignalContext::new(&signal_connection, OBJECT_PATH) {
            Ok(context) => context,
            Err(err) => {
                tracing::warn!("a11y: no signal context for the pointer locator: {err}");
                return;
            }
        };

        while pokes.recv().await.is_some() {
            if let Err(err) = PointerLocator::pointer_position_changed(&context).await {
                tracing::warn!("a11y: could not report that the pointer moved: {err}");
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_survives_the_round_trip() {
        let (handle, _pokes) = PointerLocatorHandle::new();
        handle.set(1920.0, 1080.0);
        assert_eq!(handle.get(), (1920.0, 1080.0));
    }

    #[test]
    fn a_pointer_left_of_the_origin_stays_negative() {
        // A multi-output layout puts a screen at a negative x; losing the sign
        // would report the pointer on the far side of the desktop.
        let (handle, _pokes) = PointerLocatorHandle::new();
        handle.set(-100.5, -20.25);
        assert_eq!(handle.get(), (-100.5, -20.25));
    }

    #[test]
    fn nothing_is_told_about_movement_until_something_asks() {
        let (handle, mut pokes) = PointerLocatorHandle::new();
        for i in 0..100 {
            handle.set(i as f64, 0.0);
        }
        assert!(
            pokes.try_recv().is_err(),
            "moves were announced with nobody listening"
        );
    }

    #[test]
    fn movement_is_announced_once_something_has_asked() {
        let (handle, mut pokes) = PointerLocatorHandle::new();
        handle.wanted.store(true, Ordering::Relaxed);

        handle.set(10.0, 10.0);
        assert!(pokes.try_recv().is_ok());

        // The second move rides inside the interval, and is not worth a signal
        // of its own: the listener asks for the position anyway.
        handle.set(11.0, 11.0);
        assert!(pokes.try_recv().is_err());
    }
}
