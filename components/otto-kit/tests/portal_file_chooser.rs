//! The file dialog's round trip against a stand-in portal frontend on a
//! private bus: the answer is found at whatever handle the frontend returns,
//! the wait ends when the frontend leaves the bus, and a cancel closes the
//! request.
//!
//! Its own test binary: the session bus address is the process's
//! environment, set once before anything connects. Skipped where there is no
//! `dbus-daemon` to start.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use otto_kit::portal_file_chooser::{Outcome, Request};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const WAIT: Duration = Duration::from_secs(10);

/// What the stand-in does once asked.
#[derive(Clone, Copy)]
enum Script {
    /// Answers at a handle of its own choosing, not the one the token names.
    AnswerElsewhere,
    /// Leaves the bus without answering.
    Vanish,
    /// Exports the request and waits for it to be closed.
    AwaitClose,
}

struct FileChooser {
    script: Script,
    events: mpsc::Sender<&'static str>,
}

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl FileChooser {
    async fn open_file(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        _parent: String,
        _title: String,
        options: HashMap<String, OwnedValue>,
    ) -> OwnedObjectPath {
        let sender = header
            .sender()
            .unwrap()
            .trim_start_matches(':')
            .replace('.', "_");
        let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
        let derived = format!("{PORTAL_PATH}/request/{sender}/{token}");
        match self.script {
            Script::AnswerElsewhere => {
                let elsewhere = format!("{PORTAL_PATH}/request/{sender}/elsewhere");
                let connection = connection.clone();
                let path = elsewhere.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let uris = vec!["file:///tmp/chosen".to_owned()];
                    let results: HashMap<&str, Value<'_>> =
                        HashMap::from([("uris", Value::from(uris))]);
                    connection
                        .emit_signal(
                            None::<()>,
                            path.as_str(),
                            "org.freedesktop.portal.Request",
                            "Response",
                            &(0u32, results),
                        )
                        .await
                        .unwrap();
                });
                OwnedObjectPath::try_from(elsewhere).unwrap()
            }
            Script::Vanish => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    events.send("vanish").unwrap();
                });
                OwnedObjectPath::try_from(derived).unwrap()
            }
            Script::AwaitClose => {
                let request = RequestObject {
                    events: self.events.clone(),
                };
                server.at(derived.as_str(), request).await.unwrap();
                self.events.send("asked").unwrap();
                OwnedObjectPath::try_from(derived).unwrap()
            }
        }
    }
}

struct RequestObject {
    events: mpsc::Sender<&'static str>,
}

#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl RequestObject {
    async fn close(&self, #[zbus(signal_emitter)] _emitter: SignalEmitter<'_>) {
        self.events.send("closed").unwrap();
    }
}

/// A private bus, the address of which is the session bus from here on.
fn private_bus() -> Option<Child> {
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut address = String::new();
    BufReader::new(daemon.stdout.take()?)
        .read_line(&mut address)
        .ok()?;
    // SAFETY: before any thread of this test connects to a bus.
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address.trim()) };
    Some(daemon)
}

/// Serve the stand-in on a thread of its own until `stop` says so, or, for
/// [`Script::Vanish`], until it has answered the call.
fn stand_in(script: Script) -> (mpsc::Receiver<&'static str>, mpsc::Sender<()>) {
    let (events, received) = mpsc::channel();
    let (stop, stopped) = mpsc::channel::<()>();
    let (ready, is_ready) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let (relay, relayed) = mpsc::channel();
            let connection = zbus::connection::Builder::session()
                .unwrap()
                .name(PORTAL_NAME)
                .unwrap()
                .serve_at(
                    PORTAL_PATH,
                    FileChooser {
                        script,
                        events: relay,
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            ready.send(()).unwrap();
            loop {
                if stopped.try_recv().is_ok() {
                    break;
                }
                if let Ok(event) = relayed.try_recv() {
                    if event == "vanish" {
                        break;
                    }
                    events.send(event).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            drop(connection);
        });
    });
    is_ready.recv_timeout(WAIT).unwrap();
    (received, stop)
}

/// Ask on a thread of its own, so a wait that never ends fails the test
/// rather than hanging it.
fn ask(request: Request) -> Outcome {
    let (answer, answered) = mpsc::channel();
    std::thread::spawn(move || answer.send(request.ask()).unwrap());
    answered
        .recv_timeout(WAIT)
        .expect("the dialog's wait ended")
}

#[test]
fn the_round_trip_follows_the_portal() {
    let Some(mut daemon) = private_bus() else {
        eprintln!("no dbus-daemon; skipping");
        return;
    };

    // The answer is found at the handle the frontend returned.
    let (_, stop) = stand_in(Script::AnswerElsewhere);
    assert_eq!(
        ask(Request::open_file("Open")),
        Outcome::Chosen(vec![PathBuf::from("/tmp/chosen")])
    );
    stop.send(()).unwrap();

    // Wait until the name is free again before the next stand-in claims it.
    std::thread::sleep(Duration::from_millis(200));

    // A frontend that leaves the bus ends the wait as a failure.
    let _ = stand_in(Script::Vanish);
    assert!(matches!(
        ask(Request::open_file("Open")),
        Outcome::Failed(_)
    ));
    std::thread::sleep(Duration::from_millis(200));

    // A cancel closes the request and answers "dismissed".
    let (events, stop) = stand_in(Script::AwaitClose);
    let (answer, answered) = mpsc::channel();
    let cancel = Request::open_file("Open")
        .spawn(move |outcome| answer.send(outcome).unwrap())
        .unwrap();
    assert_eq!(events.recv_timeout(WAIT).unwrap(), "asked");
    cancel.cancel();
    assert_eq!(events.recv_timeout(WAIT).unwrap(), "closed");
    assert_eq!(answered.recv_timeout(WAIT).unwrap(), Outcome::Dismissed);
    stop.send(()).unwrap();

    let _ = daemon.kill();
}
