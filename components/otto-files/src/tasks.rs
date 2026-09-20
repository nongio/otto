//! Long jobs, and how the rest of the desktop hears about them.
//!
//! A copy of ten thousand files outlives the window it was started from, the
//! folder it was started in, and quite often the user's attention. So a job
//! reports itself in three places at once, each good at something different:
//!
//! - the **status bar** of the window it belongs to, which has the room to say
//!   which file is being copied and to offer a cancel;
//! - the **island**, which announces the job starting and comes back when it
//!   ends, and carries a ring while it runs;
//! - the **dock icon**, which fills quietly for as long as the work lasts and
//!   is visible from anywhere.
//!
//! This module owns the last two. Both go through `org.otto.Island`: the
//! island draws the activity, and otto-islands publishes the same activity to
//! the dock, so one report lights up both and they can never disagree.
//!
//! # Only when the user is elsewhere
//!
//! An island is for surfacing something the user is not already looking at.
//! A job started from the window in front of them is not that: they can see
//! the status line, and a bubble would only be a second copy of it to deal
//! with. So while this app's window is activated its jobs are *quiet* — the
//! dock icon fills, and nothing appears on the row. Leave the window and the
//! island comes up; come back and it goes away again. The job itself is never
//! affected either way.
//!
//! # Nothing for a job that is over before it is seen
//!
//! An operation shorter than [`ANNOUNCE_AFTER`] shows nothing at all. Pasting
//! one small file is done before an island could finish animating in, and a
//! bubble that appears and leaves in the same breath reads as a glitch rather
//! than as progress. The activity is therefore not created when the job
//! starts, only once it has lasted long enough to be worth saying.
//!
//! # A job that fails
//!
//! An ending with something to report holds its island for [`REPORT_FOR`]
//! before dismissing it, so "Copied 340 items" or a failure is readable. One
//! with nothing to say goes without a trace.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedSender};

/// What the desktop calls this app, for grouping the activity and for finding
/// the dock icon to fill.
const APP_ID: &str = "otto-files";

/// How long a job must last before it is worth announcing.
const ANNOUNCE_AFTER: Duration = Duration::from_millis(500);

/// How long the island holds an ending before dismissing itself.
const REPORT_FOR: Duration = Duration::from_millis(2500);

/// The smallest change worth sending. The ring is a few dozen pixels around
/// and the dock bar smaller still, so a copy of ten thousand files has no
/// reason to send ten thousand updates.
const PROGRESS_STEP: f64 = 0.005;

// ---------------------------------------------------------------------------
// The handle a job holds
// ---------------------------------------------------------------------------

/// A job's report of itself, for as long as it runs.
///
/// Cheap to hold and safe to drop: dropping one without [`Task::end`] takes
/// its island away, which is the right outcome for a job whose thread died.
pub struct Task {
    handle: u64,
    tx: Option<UnboundedSender<Message>>,
    sent: f64,
}

impl Task {
    /// Announce a job that is starting, with what to call it.
    pub fn start(title: impl Into<String>) -> Self {
        let handle = next_handle();
        let tx = publisher();
        if let Some(tx) = &tx {
            let _ = tx.send(Message::Start {
                handle,
                title: title.into(),
            });
        }
        Self {
            handle,
            tx,
            sent: -1.0,
        }
    }

    /// Say where the job has got to: `done` of `total` items, currently
    /// working on `item`.
    ///
    /// A total of zero means the work has not been counted yet, and the job
    /// reports itself without a fraction.
    pub fn progress(&mut self, done: usize, total: usize, item: &str) {
        let progress = if total == 0 {
            -1.0
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        };
        // The end of a job is always worth sending, however small the step
        // into it: a ring stopping just short of closed looks stuck.
        if progress < 1.0 && (progress - self.sent).abs() < PROGRESS_STEP {
            return;
        }
        self.sent = progress;
        if let Some(tx) = &self.tx {
            let _ = tx.send(Message::Progress {
                handle: self.handle,
                title: item.to_string(),
                progress,
            });
        }
    }

    /// The job is over. `summary` is held on screen for a moment and is what
    /// the user is left with; `None` leaves nothing behind.
    pub fn end(mut self, summary: Option<String>) {
        self.finish(summary);
    }

    fn finish(&mut self, summary: Option<String>) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Message::End {
                handle: self.handle,
                summary,
            });
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        // A job whose thread went away without a word must not leave a ring
        // turning for the rest of the session.
        self.finish(None);
    }
}

// ---------------------------------------------------------------------------
// The publisher
// ---------------------------------------------------------------------------

enum Message {
    Start {
        handle: u64,
        title: String,
    },
    /// The window came forward, or went away. See [`set_watched`].
    Watched(bool),
    /// The announcing delay for this job is up: show it if it is still going.
    Announce {
        handle: u64,
    },
    Progress {
        handle: u64,
        title: String,
        progress: f64,
    },
    End {
        handle: u64,
        summary: Option<String>,
    },
}

/// Say whether this app's own window is in front of the user.
///
/// Called as the window's activated state changes. While it is watched every
/// running job is quiet; when it stops being watched, whatever is still
/// running comes up on the island.
pub fn set_watched(watched: bool) {
    static LAST: AtomicBool = AtomicBool::new(false);
    if LAST.swap(watched, Ordering::Relaxed) == watched {
        return;
    }
    if let Some(tx) = publisher() {
        let _ = tx.send(Message::Watched(watched));
    }
}

fn next_handle() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The channel into the publisher thread, started on first use.
///
/// `None` when there is no session bus to talk to, which is the ordinary case
/// under a test or on another desktop: a job then simply runs unannounced.
fn publisher() -> Option<UnboundedSender<Message>> {
    static PUBLISHER: OnceLock<Option<UnboundedSender<Message>>> = OnceLock::new();
    PUBLISHER.get_or_init(spawn_publisher).clone()
}

fn spawn_publisher() -> Option<UnboundedSender<Message>> {
    // Under test there is a real session bus and a real island on the other
    // side of it, and a test run has no business putting bubbles on the
    // developer's desktop.
    if cfg!(test) {
        return None;
    }

    let (tx, rx) = mpsc::unbounded_channel();
    // The loop's own way of hearing that a job has run long enough to show.
    let announcements = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("otto-files-tasks".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::warn!(?err, "no runtime for task reporting");
                    return;
                }
            };
            runtime.block_on(publish(rx, announcements));
        });
    match spawned {
        Ok(_) => Some(tx),
        Err(err) => {
            tracing::warn!(?err, "could not start task reporting");
            None
        }
    }
}

/// One activity on the bus per job, for as long as the job runs.
async fn publish(
    mut rx: mpsc::UnboundedReceiver<Message>,
    announcements: UnboundedSender<Message>,
) {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(err) => {
            tracing::info!(?err, "no session bus: jobs run unannounced");
            return;
        }
    };

    let mut activities: std::collections::HashMap<u64, Activity> = Default::default();
    // Assume the window is in front until told otherwise: a job is nearly
    // always started from it.
    let mut watched = true;

    while let Some(message) = rx.recv().await {
        match message {
            Message::Watched(now_watched) => {
                watched = now_watched;
                for activity in activities.values_mut() {
                    activity.set_quiet(&connection, watched).await;
                }
            }
            Message::Start { handle, title } => {
                activities.insert(
                    handle,
                    Activity::Waiting {
                        title,
                        progress: -1.0,
                    },
                );
                // Come back when the delay is up. A job still running then is
                // worth showing whether or not it has reported anything since,
                // which is what a long single-file copy needs.
                let announcements = announcements.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ANNOUNCE_AFTER).await;
                    let _ = announcements.send(Message::Announce { handle });
                });
            }
            Message::Announce { handle } => {
                let Some(Activity::Waiting { title, progress }) = activities.remove(&handle) else {
                    continue;
                };
                // Created either way: the dock icon fills for a job the user
                // is watching too. Quiet is only about the island.
                let next = match create(&connection, &title, progress, watched).await {
                    Some(id) => Activity::Live { id },
                    None => Activity::Waiting { title, progress },
                };
                activities.insert(handle, next);
            }
            Message::Progress {
                handle,
                title,
                progress,
            } => {
                let Some(activity) = activities.remove(&handle) else {
                    continue;
                };
                let next = match activity {
                    // Still inside the announcing delay: keep the latest word
                    // for when it is up, so a job that ends before then never
                    // appears at all.
                    Activity::Waiting { .. } => Activity::Waiting { title, progress },
                    Activity::Live { id } => {
                        update(&connection, id, &title, progress).await;
                        Activity::Live { id }
                    }
                };
                activities.insert(handle, next);
            }
            Message::End { handle, summary } => {
                let Some(activity) = activities.remove(&handle) else {
                    continue;
                };
                let Activity::Live { id } = activity else {
                    // Never announced, so there is nothing to take back. A
                    // summary with no island to put it on is dropped: the
                    // window's own status bar has already said it.
                    continue;
                };
                match summary {
                    Some(summary) => {
                        update(&connection, id, &summary, 1.0).await;
                        let connection = connection.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(REPORT_FOR).await;
                            dismiss(&connection, id).await;
                        });
                    }
                    None => dismiss(&connection, id).await,
                }
            }
        }
    }
}

/// A job's activity while it runs.
enum Activity {
    /// Started, but not yet worth reporting. Holds the latest thing the job
    /// said about itself, which is what it opens with when it is.
    Waiting { title: String, progress: f64 },
    /// Reported, under this activity id.
    Live { id: u64 },
}

impl Activity {
    /// Follow the window: quiet while it is in front, on the island once it
    /// is not. A job not yet reported has nothing to hide.
    async fn set_quiet(&self, connection: &zbus::Connection, quiet: bool) {
        let Self::Live { id } = self else {
            return;
        };
        let _ = connection
            .call_method(
                Some("org.otto.Island"),
                "/org/otto/Island",
                Some("org.otto.Island1"),
                "SetActivityQuiet",
                &(*id, quiet),
            )
            .await;
    }
}

async fn create(
    connection: &zbus::Connection,
    title: &str,
    progress: f64,
    quiet: bool,
) -> Option<u64> {
    let reply = connection
        .call_method(
            Some("org.otto.Island"),
            "/org/otto/Island",
            Some("org.otto.Island1"),
            "CreateActivity",
            // No timeout: a job ends when it ends, not when a clock says so.
            &(
                APP_ID, title, APP_ID, progress, 0_u32, "normal", true, quiet,
            ),
        )
        .await
        .ok()?;
    reply.body().deserialize().ok()
}

async fn update(connection: &zbus::Connection, id: u64, title: &str, progress: f64) {
    let _ = connection
        .call_method(
            Some("org.otto.Island"),
            "/org/otto/Island",
            Some("org.otto.Island1"),
            "UpdateActivity",
            &(id, title, progress),
        )
        .await;
}

async fn dismiss(connection: &zbus::Connection, id: u64) {
    let _ = connection
        .call_method(
            Some("org.otto.Island"),
            "/org/otto/Island",
            Some("org.otto.Island1"),
            "DismissActivity",
            &(id,),
        )
        .await;
}
