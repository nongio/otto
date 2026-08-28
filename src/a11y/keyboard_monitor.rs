//! `org.freedesktop.a11y.KeyboardMonitor` — key snooping and key grabs for
//! assistive technologies.
//!
//! On Wayland an AT cannot read the keyboard itself, so a screen reader has no
//! way to receive its own keybindings unless the compositor hands them over.
//! The interface implemented here is the one at-spi2-core defines for that, and
//! is what Orca talks to. The contract lives in
//! [`specs/accessibility.md`](../../specs/accessibility.md).
//!
//! **Threading.** The decision of whether to swallow a key has to be made
//! synchronously, on the compositor thread, in the middle of the input filter —
//! a D-Bus round trip there would stall every keystroke in the session. So the
//! grab table is a plain shared structure: the compositor matches against it
//! directly ([`KeyboardMonitorHandle::process_key`]) and pushes the outgoing
//! `KeyEvent` signals onto a channel that the D-Bus task drains and emits.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::input::keyboard::Keysym;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{trace, warn};
use zbus::message::Header;
use zbus::names::{BusName, OwnedUniqueName};
use zbus::{fdo, interface, Connection, SignalContext};

/// Well-known name the compositor owns for the a11y manager.
pub const BUS_NAME: &str = "org.freedesktop.a11y.Manager";
/// Object path both a11y manager interfaces live at.
pub const OBJECT_PATH: &str = "/org/freedesktop/a11y/Manager";

/// A `KeyEvent` signal on its way out, already addressed to the clients that
/// asked to see this key. Built on the compositor thread, emitted on the D-Bus
/// task.
#[derive(Debug)]
pub struct PendingKeyEvent {
    /// Unique bus names to send this to, one signal each.
    pub destinations: Vec<OwnedUniqueName>,
    pub released: bool,
    pub modifiers: u32,
    pub keysym: u32,
    pub unichar: u32,
    pub keycode: u16,
}

/// What one client asked for.
#[derive(Debug, Default)]
struct ClientGrabs {
    /// `WatchKeyboard`: every key is reported, none are taken.
    watch_all: bool,
    /// `GrabKeyboard`: every key is reported *and* taken.
    grab_all: bool,
    /// `SetKeyGrabs` modifiers: these keys, and anything pressed while one of
    /// them is down, belong to the client.
    modifiers: HashSet<Keysym>,
    /// `SetKeyGrabs` keystrokes: (keysym, xkb modifier mask).
    keystrokes: Vec<(Keysym, u32)>,
}

impl ClientGrabs {
    /// Nothing left to remember about this client.
    fn is_inert(&self) -> bool {
        !self.watch_all && !self.grab_all && self.modifiers.is_empty() && self.keystrokes.is_empty()
    }

    /// Does this key belong to the client rather than to the session?
    fn grabs(&self, held_grabbed: &HashSet<Keysym>, modifiers: u32, keysym: Keysym) -> bool {
        if self.grab_all {
            return true;
        }

        // The grabbed modifier itself, or any key pressed while it is held.
        for modifier in &self.modifiers {
            if *modifier == keysym || held_grabbed.contains(modifier) {
                return true;
            }
        }

        self.keystrokes
            .iter()
            .any(|(sym, mask)| *sym == keysym && *mask == modifiers)
    }

    /// Should the client be told about this key, whoever ends up handling it?
    fn watches(&self, held_grabbed: &HashSet<Keysym>, modifiers: u32, keysym: Keysym) -> bool {
        self.watch_all || self.grabs(held_grabbed, modifiers, keysym)
    }
}

/// The grab table, shared between the compositor thread and the D-Bus task.
#[derive(Debug, Default)]
pub struct MonitorState {
    clients: HashMap<OwnedUniqueName, ClientGrabs>,
    /// Union of every client's grabbed modifiers, so the hot path does not have
    /// to walk the clients to recognise one.
    grabbed_modifiers: HashSet<Keysym>,
    /// Keys whose press was taken by a client, so their release must be taken
    /// too — the client saw a press the session never did.
    held_grabbed: HashSet<Keysym>,
    /// When each grabbed modifier was last pressed, for the double-tap rule.
    modifier_last_press: HashMap<Keysym, Duration>,
}

impl MonitorState {
    fn client_mut(&mut self, name: OwnedUniqueName) -> &mut ClientGrabs {
        self.clients.entry(name).or_default()
    }

    fn rebuild_grabbed_modifiers(&mut self) {
        self.grabbed_modifiers = self
            .clients
            .values()
            .flat_map(|client| client.modifiers.iter().copied())
            .collect();
    }

    fn forget_if_inert(&mut self, name: &OwnedUniqueName) {
        if self.clients.get(name).is_some_and(ClientGrabs::is_inert) {
            self.clients.remove(name);
        }
    }
}

/// The compositor's end of the monitor. Cheap to clone and to ask.
#[derive(Clone)]
pub struct KeyboardMonitorHandle {
    state: Arc<Mutex<MonitorState>>,
    events: UnboundedSender<PendingKeyEvent>,
    /// Set while at least one client is registered. Read on every key press, so
    /// the common case — no AT running — costs an atomic load and nothing else.
    active: Arc<AtomicBool>,
}

impl KeyboardMonitorHandle {
    /// Builds the monitor. The receiver belongs to the D-Bus task, which emits
    /// what it drains as `KeyEvent` signals.
    pub fn new() -> (Self, UnboundedReceiver<PendingKeyEvent>) {
        let (events, rx) = unbounded_channel();
        let handle = Self {
            state: Arc::new(Mutex::new(MonitorState::default())),
            events,
            active: Arc::new(AtomicBool::new(false)),
        };
        (handle, rx)
    }

    /// Offers a key to the assistive technologies. Returns `true` when the key
    /// belongs to one of them and must not be handled by the session — no
    /// shortcut, no delivery to the focused client, no toggle state change.
    ///
    /// `time` is the event timestamp and `repeat_delay` the configured key
    /// repeat delay; together they decide whether a second press of a grabbed
    /// modifier counts as a double-tap.
    #[allow(clippy::too_many_arguments)]
    pub fn process_key(
        &self,
        repeat_delay: Duration,
        time: Duration,
        keycode: u16,
        released: bool,
        modifiers: u32,
        keysym: Keysym,
        unichar: u32,
    ) -> bool {
        // No AT attached: the overwhelmingly common case, and the one that must
        // stay free.
        if !self.active.load(Ordering::Relaxed) {
            return false;
        }

        let mut state = self.state.lock().unwrap();

        // Who wants to be told about this key.
        let destinations: Vec<OwnedUniqueName> = state
            .clients
            .iter()
            .filter(|(_, client)| client.watches(&state.held_grabbed, modifiers, keysym))
            .map(|(name, _)| name.clone())
            .collect();

        if !destinations.is_empty() {
            let event = PendingKeyEvent {
                destinations,
                released,
                modifiers,
                keysym: keysym.raw(),
                unichar,
                keycode,
            };
            if self.events.send(event).is_err() {
                warn!("a11y keyboard monitor: signal task is gone");
            }
        }

        // A grabbed modifier pressed twice in quick succession is the user
        // asking for the modifier itself — pass the second press, and its
        // release, through to the session.
        if state.grabbed_modifiers.contains(&keysym) {
            if released {
                // Not suppressed means this is the release of a press that was
                // already handed to the session.
                if !state.held_grabbed.contains(&keysym) {
                    trace!(?keysym, "a11y: release of a passed-through modifier");
                    return false;
                }
            } else {
                // Only a *second* press counts: with no previous press there is
                // nothing to have double-tapped.
                let previous = state.modifier_last_press.insert(keysym, time);
                if previous.is_some_and(|last| time <= last.saturating_add(repeat_delay)) {
                    trace!(?keysym, "a11y: double-tapped modifier, passing through");
                    return false;
                }
            }
        }

        if released {
            // Take the release exactly when the press was taken.
            let blocked = state.held_grabbed.remove(&keysym);
            if blocked {
                trace!(?keysym, "a11y: blocking release of a grabbed key");
            }
            return blocked;
        }

        // A repeat, or the same key from a second keyboard, while it is held by
        // a client.
        if state.held_grabbed.contains(&keysym) {
            return true;
        }

        let grabbed = {
            let held = &state.held_grabbed;
            state
                .clients
                .values()
                .any(|client| client.grabs(held, modifiers, keysym))
        };
        if grabbed {
            trace!(?keysym, "a11y: blocking grabbed key");
            state.held_grabbed.insert(keysym);
        }
        grabbed
    }

    fn with_state(&self, f: impl FnOnce(&mut MonitorState)) {
        let mut state = self.state.lock().unwrap();
        f(&mut state);
        self.active
            .store(!state.clients.is_empty(), Ordering::Relaxed);
    }

    /// Drops everything a client asked for. Called when it disconnects.
    pub fn forget_client(&self, name: &OwnedUniqueName) {
        self.with_state(|state| {
            if state.clients.remove(name).is_some() {
                trace!(%name, "a11y: dropped disconnected client");
                state.rebuild_grabbed_modifiers();
                // Keys the departed client was holding would otherwise stay
                // held forever: their release is the event that clears them,
                // and it is now nobody's. Anything still physically down is
                // judged again on its next event.
                state.held_grabbed.clear();
            }
        });
    }
}

/// The D-Bus face of the monitor. Every method mutates the same table
/// [`KeyboardMonitorHandle`] reads from.
pub struct KeyboardMonitor {
    handle: KeyboardMonitorHandle,
}

impl KeyboardMonitor {
    fn sender(header: &Header<'_>) -> fdo::Result<OwnedUniqueName> {
        header
            .sender()
            .map(|name| OwnedUniqueName::from(name.to_owned()))
            .ok_or_else(|| fdo::Error::Failed("message has no sender".to_owned()))
    }
}

#[interface(name = "org.freedesktop.a11y.KeyboardMonitor")]
impl KeyboardMonitor {
    /// Take every key: the caller receives it through `KeyEvent` and the
    /// session does not see it at all, toggles included. In effect until the
    /// same client calls `UngrabKeyboard` or disconnects.
    async fn grab_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let sender = Self::sender(&header)?;
        trace!(%sender, "a11y: GrabKeyboard");
        self.handle
            .with_state(|state| state.client_mut(sender).grab_all = true);
        Ok(())
    }

    /// Undoes `GrabKeyboard`. Any grabs from `SetKeyGrabs` stay in effect, and
    /// a client that called `WatchKeyboard` keeps receiving key events.
    async fn ungrab_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let sender = Self::sender(&header)?;
        trace!(%sender, "a11y: UngrabKeyboard");
        self.handle.with_state(|state| {
            if let Some(client) = state.clients.get_mut(&sender) {
                client.grab_all = false;
            }
            state.forget_if_inert(&sender);
        });
        Ok(())
    }

    /// Report every key to the caller, but let the session handle it as usual.
    async fn watch_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let sender = Self::sender(&header)?;
        trace!(%sender, "a11y: WatchKeyboard");
        self.handle
            .with_state(|state| state.client_mut(sender).watch_all = true);
        Ok(())
    }

    /// Undoes `WatchKeyboard`. Grabbed keys are still reported.
    async fn unwatch_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let sender = Self::sender(&header)?;
        trace!(%sender, "a11y: UnwatchKeyboard");
        self.handle.with_state(|state| {
            if let Some(client) = state.clients.get_mut(&sender) {
                client.watch_all = false;
            }
            state.forget_if_inert(&sender);
        });
        Ok(())
    }

    /// Replaces the caller's key grabs.
    ///
    /// `modifiers` are XKB keysyms: each is grabbed, and so is anything pressed
    /// while one of them is held. `keystrokes` pairs a non-modifier keysym with
    /// the XKB modifier mask it must be pressed under.
    async fn set_key_grabs(
        &self,
        #[zbus(header)] header: Header<'_>,
        modifiers: Vec<u32>,
        keystrokes: Vec<(u32, u32)>,
    ) -> fdo::Result<()> {
        let sender = Self::sender(&header)?;
        trace!(
            %sender,
            modifiers = modifiers.len(),
            keystrokes = keystrokes.len(),
            "a11y: SetKeyGrabs"
        );
        self.handle.with_state(|state| {
            let client = state.client_mut(sender.clone());
            client.modifiers = modifiers.into_iter().map(Keysym::new).collect();
            client.keystrokes = keystrokes
                .into_iter()
                .map(|(sym, mask)| (Keysym::new(sym), mask))
                .collect();
            state.forget_if_inert(&sender);
            state.rebuild_grabbed_modifiers();
        });
        Ok(())
    }

    /// Emitted for each key press and release the client watches or grabs.
    ///
    /// - `released`: this is a key-up
    /// - `state`: XKB modifier mask of the modifiers currently down
    /// - `keysym`: XKB keysym of this key
    /// - `unichar`: Unicode character it produces, or 0
    /// - `keycode`: hardware keycode
    #[zbus(signal)]
    pub async fn key_event(
        context: &SignalContext<'_>,
        released: bool,
        state: u32,
        keysym: u32,
        unichar: u32,
        keycode: u16,
    ) -> zbus::Result<()>;
}

/// Registers the interface and starts the two tasks it needs: one emitting
/// queued `KeyEvent` signals, one dropping clients that disconnect.
pub async fn register(
    connection: &Connection,
    handle: KeyboardMonitorHandle,
    mut events: UnboundedReceiver<PendingKeyEvent>,
) -> zbus::Result<()> {
    connection
        .object_server()
        .at(
            OBJECT_PATH,
            KeyboardMonitor {
                handle: handle.clone(),
            },
        )
        .await?;

    let signal_connection = connection.clone();
    tokio::spawn(async move {
        let context = match SignalContext::new(&signal_connection, OBJECT_PATH) {
            Ok(context) => context,
            Err(err) => {
                warn!("a11y: cannot build a signal context: {err}");
                return;
            }
        };

        while let Some(event) = events.recv().await {
            for destination in &event.destinations {
                let context = context
                    .clone()
                    .set_destination(BusName::Unique(destination.clone().into_inner()));
                if let Err(err) = KeyboardMonitor::key_event(
                    &context,
                    event.released,
                    event.modifiers,
                    event.keysym,
                    event.unichar,
                    event.keycode,
                )
                .await
                {
                    warn!("a11y: failed to emit KeyEvent to {destination}: {err}");
                }
            }
        }
    });

    let watch_connection = connection.clone();
    tokio::spawn(async move {
        if let Err(err) = watch_disconnects(&watch_connection, handle).await {
            warn!("a11y: client disconnect watch stopped: {err}");
        }
    });

    Ok(())
}

/// A client's grabs die with its connection — otherwise a crashed screen reader
/// would leave the session with keys that go nowhere.
async fn watch_disconnects(
    connection: &Connection,
    handle: KeyboardMonitorHandle,
) -> zbus::Result<()> {
    use zbus::export::futures_util::StreamExt;

    let proxy = fdo::DBusProxy::new(connection).await?;
    let mut changes = proxy.receive_name_owner_changed().await?;

    while let Some(change) = changes.next().await {
        let Ok(args) = change.args() else {
            continue;
        };
        // A unique name losing its owner and gaining none is a disconnect.
        if args.new_owner.is_some() {
            continue;
        }
        let BusName::Unique(name) = &args.name else {
            continue;
        };
        handle.forget_client(&OwnedUniqueName::from(name.to_owned()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPEAT_DELAY: Duration = Duration::from_millis(300);

    fn client(name: &str) -> OwnedUniqueName {
        OwnedUniqueName::try_from(name.to_owned()).unwrap()
    }

    /// Press or release `keysym` under `modifiers`, `millis` into the session.
    fn key(
        handle: &KeyboardMonitorHandle,
        millis: u64,
        released: bool,
        modifiers: u32,
        keysym: Keysym,
    ) -> bool {
        handle.process_key(
            REPEAT_DELAY,
            Duration::from_millis(millis),
            30,
            released,
            modifiers,
            keysym,
            0,
        )
    }

    #[test]
    fn no_client_takes_nothing() {
        let (handle, _events) = KeyboardMonitorHandle::new();
        assert!(!key(&handle, 0, false, 0, Keysym::a));
        assert!(!key(&handle, 10, true, 0, Keysym::a));
    }

    #[test]
    fn grab_all_takes_press_and_release() {
        let (handle, mut events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| state.client_mut(client(":1.5")).grab_all = true);

        assert!(key(&handle, 0, false, 0, Keysym::a));
        assert!(key(&handle, 10, true, 0, Keysym::a));

        // Both went out as signals, addressed to the one client.
        let event = events.try_recv().expect("press was not reported");
        assert_eq!(event.destinations, vec![client(":1.5")]);
        assert!(!event.released);
        assert!(
            events
                .try_recv()
                .expect("release was not reported")
                .released
        );
    }

    #[test]
    fn watch_reports_without_taking() {
        let (handle, mut events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| state.client_mut(client(":1.5")).watch_all = true);

        assert!(!key(&handle, 0, false, 0, Keysym::a));
        assert!(events.try_recv().is_ok());
    }

    #[test]
    fn keystroke_grab_matches_its_modifiers_only() {
        let (handle, _events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| {
            state.client_mut(client(":1.5")).keystrokes = vec![(Keysym::a, 0x4)];
        });

        assert!(key(&handle, 0, false, 0x4, Keysym::a));
        assert!(key(&handle, 10, true, 0x4, Keysym::a));
        // Same key, no modifier: the session's.
        assert!(!key(&handle, 20, false, 0, Keysym::a));
    }

    #[test]
    fn grabbed_modifier_takes_what_is_pressed_under_it() {
        let (handle, _events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| {
            state.client_mut(client(":1.5")).modifiers = HashSet::from([Keysym::Insert]);
            state.rebuild_grabbed_modifiers();
        });

        // The modifier itself, held down.
        assert!(key(&handle, 0, false, 0, Keysym::Insert));
        // Anything pressed while it is held belongs to the client too.
        assert!(key(&handle, 50, false, 0, Keysym::h));
        assert!(key(&handle, 60, true, 0, Keysym::h));
        // A key pressed after it is released does not.
        assert!(key(&handle, 100, true, 0, Keysym::Insert));
        assert!(!key(&handle, 150, false, 0, Keysym::h));
    }

    #[test]
    fn double_tapped_modifier_reaches_the_session() {
        let (handle, _events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| {
            state.client_mut(client(":1.5")).modifiers = HashSet::from([Keysym::Insert]);
            state.rebuild_grabbed_modifiers();
        });

        // First press is swallowed...
        assert!(key(&handle, 0, false, 0, Keysym::Insert));
        assert!(key(&handle, 50, true, 0, Keysym::Insert));
        // ...but a second within the repeat delay is the user asking for the
        // key itself, and so is its release.
        assert!(!key(&handle, 200, false, 0, Keysym::Insert));
        assert!(!key(&handle, 250, true, 0, Keysym::Insert));

        // Long after, it is the screen reader's modifier again.
        assert!(key(&handle, 5_000, false, 0, Keysym::Insert));
    }

    #[test]
    fn a_disconnected_client_keeps_nothing() {
        let (handle, _events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| {
            state.client_mut(client(":1.5")).modifiers = HashSet::from([Keysym::Insert]);
            state.rebuild_grabbed_modifiers();
        });
        assert!(key(&handle, 0, false, 0, Keysym::Insert));

        handle.forget_client(&client(":1.5"));

        assert!(!key(&handle, 100, false, 0, Keysym::Insert));
        assert!(!handle.active.load(Ordering::Relaxed));
    }

    #[test]
    fn one_clients_grabs_do_not_leak_into_another() {
        let (handle, _events) = KeyboardMonitorHandle::new();
        handle.with_state(|state| {
            state.client_mut(client(":1.5")).keystrokes = vec![(Keysym::a, 0)];
            state.client_mut(client(":1.6")).keystrokes = vec![(Keysym::b, 0)];
        });

        assert!(key(&handle, 0, false, 0, Keysym::a));
        assert!(key(&handle, 10, false, 0, Keysym::b));
        assert!(!key(&handle, 20, false, 0, Keysym::c));

        handle.forget_client(&client(":1.6"));
        assert!(key(&handle, 30, false, 0, Keysym::a));
        assert!(!key(&handle, 40, false, 0, Keysym::b));
    }
}
