//! The one client of the freedesktop Settings portal in otto-kit.
//!
//! Colour scheme, accent, icon theme, sound theme, Otto's own appearance keys
//! and the preferred locales all come from `org.freedesktop.portal.Settings`.
//! They used to open a session connection each, with a proxy and a
//! `SettingChanged` subscription of their own: six connections and six match
//! rules for one interface. Here there is one of each.
//!
//! A background thread owns the connection, the proxy and the signal stream.
//! A watcher registers the `(namespace, key)` pairs it follows together with a
//! handler: the hub reads each pair once, hands the answers to the handler,
//! and from then on hands it every `SettingChanged` for one of its pairs. The
//! handlers run on the hub's thread, one at a time, so a handler can keep
//! state of its own without locking it.
//!
//! The thread is the hub's own rather than the app's runtime: a one-off
//! [`read_blocking`] has to work from a synchronous `main`, from inside a
//! `#[tokio::main]` and from a single-threaded runtime alike, and blocking on
//! a task that needs the caller's own thread to make progress would deadlock
//! the last of those.

use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt as _;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use zbus::zvariant::{OwnedValue, Value};

/// `org.freedesktop.portal.Settings`, the frontend interface every toolkit
/// reads desktop settings from.
#[zbus::proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop",
    gen_blocking = false
)]
trait Settings {
    fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;

    #[zbus(signal)]
    fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;
}

/// What a watcher is told: the namespace, the key and the value, still
/// wrapped in whatever variants the portal put round it.
pub(crate) type Handler = Box<dyn FnMut(&str, &str, Value<'_>) + Send>;

enum Request {
    /// Read one key and answer on `reply`; `None` when it could not be read.
    Read {
        namespace: &'static str,
        key: &'static str,
        reply: SyncSender<Option<OwnedValue>>,
    },
    /// Read `keys` in order, then follow them.
    Watch {
        name: &'static str,
        keys: &'static [(&'static str, &'static str)],
        handler: Handler,
    },
}

/// Follow `keys`: `handler` gets each one's current value (in the order
/// given, where the portal answers) and then every change to any of them.
///
/// `name` only labels the log lines.
pub(crate) fn watch(
    name: &'static str,
    keys: &'static [(&'static str, &'static str)],
    handler: impl FnMut(&str, &str, Value<'_>) + Send + 'static,
) {
    send(Request::Watch {
        name,
        keys,
        handler: Box::new(handler),
    });
}

/// Read one key, waiting at most `timeout` for the answer.
///
/// `None` when there is no portal, no session bus, or no answer in time: a
/// portal that is absent answers by not answering.
pub(crate) fn read_blocking(
    namespace: &'static str,
    key: &'static str,
    timeout: Duration,
) -> Option<OwnedValue> {
    let (reply, answer) = sync_channel(1);
    send(Request::Read {
        namespace,
        key,
        reply,
    });
    answer.recv_timeout(timeout).ok().flatten()
}

fn send(request: Request) {
    static HUB: OnceLock<Option<UnboundedSender<Request>>> = OnceLock::new();
    let hub = HUB.get_or_init(|| {
        let (sender, requests) = unbounded_channel();
        let spawned = std::thread::Builder::new()
            .name("portal-settings".to_string())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run(requests)),
                    Err(err) => tracing::warn!("portal settings: no runtime ({err})"),
                }
            });
        match spawned {
            Ok(_) => Some(sender),
            Err(err) => {
                tracing::warn!("portal settings: could not start ({err})");
                None
            }
        }
    });
    // With no hub, dropping the request answers a read with "no value" and
    // leaves a watcher on whatever it had before.
    if let Some(hub) = hub {
        let _ = hub.send(request);
    }
}

struct Watcher {
    name: &'static str,
    keys: &'static [(&'static str, &'static str)],
    handler: Handler,
}

impl Watcher {
    fn follows(&self, namespace: &str, key: &str) -> bool {
        self.keys
            .iter()
            .any(|(ns, k)| *ns == namespace && *k == key)
    }
}

async fn run(mut requests: UnboundedReceiver<Request>) {
    let proxy = match connect().await {
        Ok(proxy) => proxy,
        Err(err) => {
            tracing::warn!("portal settings: unavailable ({err})");
            // Keep answering, so a blocking read returns at once instead of
            // waiting out its timeout.
            while let Some(request) = requests.recv().await {
                if let Request::Read { reply, .. } = request {
                    let _ = reply.try_send(None);
                }
            }
            return;
        }
    };

    // Subscribed before the first read, so a change that lands between the
    // read and the subscription is not lost.
    let mut changes = match proxy.receive_setting_changed().await {
        Ok(changes) => Some(changes),
        Err(err) => {
            tracing::warn!("portal settings: cannot follow changes ({err})");
            None
        }
    };
    let mut watchers: Vec<Watcher> = Vec::new();

    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { break };
                match request {
                    Request::Read { namespace, key, reply } => {
                        let value = proxy
                            .read(namespace, key)
                            .await
                            .inspect_err(|err| {
                                tracing::debug!("{namespace} {key} read failed (portal absent?): {err}")
                            })
                            .ok();
                        let _ = reply.try_send(value);
                    }
                    Request::Watch { name, keys, mut handler } => {
                        for (namespace, key) in keys {
                            match proxy.read(namespace, key).await {
                                Ok(owned) => handler(namespace, key, owned.into()),
                                Err(err) => tracing::debug!(
                                    "{name}: {namespace} {key} read failed (portal absent?): {err}"
                                ),
                            }
                        }
                        watchers.push(Watcher { name, keys, handler });
                    }
                }
            }
            signal = next_change(&mut changes) => {
                let Some(signal) = signal else {
                    tracing::debug!("portal settings: change stream ended");
                    changes = None;
                    continue;
                };
                let Ok(args) = signal.args() else { continue };
                for watcher in watchers.iter_mut() {
                    if !watcher.follows(args.namespace, args.key) {
                        continue;
                    }
                    // A copy per watcher; only a file descriptor cannot be
                    // copied, and no setting is one.
                    if let Ok(value) = args.value.try_clone() {
                        tracing::trace!("{}: {} {} changed", watcher.name, args.namespace, args.key);
                        (watcher.handler)(args.namespace, args.key, value);
                    }
                }
            }
        }
    }
}

async fn connect() -> zbus::Result<SettingsProxy<'static>> {
    let connection = zbus::Connection::session().await?;
    SettingsProxy::new(&connection).await
}

/// The next change, or never when there is no stream to read.
async fn next_change(changes: &mut Option<SettingChangedStream>) -> Option<SettingChanged> {
    match changes {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}
