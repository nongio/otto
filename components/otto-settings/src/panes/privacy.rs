//! The Privacy pane: what apps have been allowed to do, read from
//! xdg-permission-store, with a way to take each answer back.
//!
//! - **Screen sharing**:
//!   - Recording and remote desktop list the shares that were remembered —
//!     restore tokens in the `screencast` and `remote-desktop` tables, kept
//!     by the portal frontend or, for an app that brings no token of its own,
//!     by Otto's portal when "Remember for <app>" was ticked. Each says what
//!     it shares; Forget deletes it, so the app is asked again.
//!   - Screenshots: the portal's own `screenshot` table, which
//!     xdg-desktop-portal checks itself: **Ask / Allow / Don't Allow** and
//!     Forget.
//! - **Notifications** lists the apps in the `notifications` table, each
//!   with a switch. An app is recorded there the first time it notifies, and
//!   its notifications are dropped while the entry says `no`: by the portal's
//!   Notification interface for apps that notify through it (Flatpak apps),
//!   and by otto-islands' notification daemon for every app that calls it
//!   directly, named by its desktop entry (see islands'
//!   `notification_permission`).
//!
//! Settings writes the store directly and asks for nothing: a sandboxed app
//! cannot reach the store, and a program running as the user can anyway.
//! Unsandboxed apps share the empty app id, so their row speaks for all of
//! them, and says so.
//!
//! The store is read on a thread of its own when the pane comes on screen and
//! whenever it signals `Changed`, never while drawing: the model is rebuilt
//! every frame.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use crate::model::{group, Control, Pane, Row};
use crate::settings_client;
use otto_kit::permission_store::{self as store, Entry, Restored, Store};

/// The prefix of a grant row's identifier. The rest is table, resource id
/// and app id, separated by [`SEP`].
const GRANT_ROW: &str = "privacy.grant:";
/// The prefix of a notification switch's identifier; the rest is the app id.
const SENDER_ROW: &str = "privacy.notifications:";
const SEP: char = '\u{1f}';

/// What a row grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    ScreenCast,
    RemoteDesktop,
    Screenshot,
}

/// One app's decision on one entry.
#[derive(Debug, Clone, PartialEq)]
struct Grant {
    kind: Kind,
    table: String,
    id: String,
    /// Empty for every unsandboxed app, which the portal cannot tell apart.
    app: String,
    /// Whether other apps hold a decision on the same entry, so forgetting
    /// this one must leave theirs.
    shared: bool,
    allowed: bool,
    restored: Option<Restored>,
    /// Who remembered it, when that was another desktop's portal.
    foreign_vendor: Option<String>,
    /// The program it was remembered for, when the app is unsandboxed and
    /// Otto's portal could tell which program asked.
    program: Option<String>,
}

/// An answer on a Select row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Ask,
    Allow,
    Deny,
}

impl Decision {
    const ALL: [Decision; 3] = [Decision::Ask, Decision::Allow, Decision::Deny];

    fn label(self) -> &'static str {
        match self {
            Decision::Ask => otto_kit::t!("privacy-decision-ask"),
            Decision::Allow => otto_kit::t!("privacy-decision-allow"),
            Decision::Deny => otto_kit::t!("privacy-decision-deny"),
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.label() == label)
    }
}

/// How a screenshot decision is kept in the portal's `screenshot` table:
/// `None` is no answer at all, which the portal asks about.
fn screenshot_value(decision: Decision) -> Option<&'static str> {
    match decision {
        Decision::Ask => None,
        Decision::Allow => Some("yes"),
        Decision::Deny => Some("no"),
    }
}

fn screenshot_decision(permissions: &[String]) -> Decision {
    match store::answer(permissions) {
        Some(true) => Decision::Allow,
        Some(false) => Decision::Deny,
        None => Decision::Ask,
    }
}

/// One app's screenshot row.
#[derive(Debug, Clone, PartialEq)]
struct AppRow {
    app: String,
    decision: Decision,
    /// Whether other apps share the `screenshot` entry, so forgetting this
    /// one must leave theirs.
    shared: bool,
}

/// An app in the `notifications` table, and whether it may notify.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sender {
    app: String,
    allowed: bool,
}

/// The store as last read.
#[derive(Debug, Clone, PartialEq)]
enum Snapshot {
    /// Not read yet.
    Pending,
    /// The store could not be read.
    Unavailable,
    Ready {
        /// Remembered shares in `screencast` and `remote-desktop`, which can
        /// only be forgotten.
        shares: Vec<Grant>,
        screenshots: Vec<AppRow>,
        senders: Vec<Sender>,
    },
}

static SNAPSHOT: RwLock<Snapshot> = RwLock::new(Snapshot::Pending);
static SHOWN: AtomicBool = AtomicBool::new(false);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// The screen-sharing decisions in `entries`, one per app per entry: capture
/// and remote desktop first, then screenshots, each in store order.
fn screen_grants(entries: &[Entry]) -> Vec<Grant> {
    let kind_of = |table: &str| match table {
        store::SCREENCAST => Some(Kind::ScreenCast),
        store::REMOTE_DESKTOP => Some(Kind::RemoteDesktop),
        store::SCREENSHOT => Some(Kind::Screenshot),
        _ => None,
    };
    let mut grants: Vec<Grant> = Vec::new();
    for entry in entries {
        let Some(kind) = kind_of(&entry.table) else {
            continue;
        };
        if kind == Kind::Screenshot && entry.id != store::SCREENSHOT_ID {
            continue;
        }
        for (app, permissions) in &entry.apps {
            grants.push(Grant {
                kind,
                table: entry.table.clone(),
                id: entry.id.clone(),
                app: app.clone(),
                shared: entry.apps.len() > 1,
                allowed: permissions.iter().any(|p| p == "yes"),
                restored: entry.restored.clone(),
                foreign_vendor: entry
                    .vendor
                    .clone()
                    .filter(|vendor| !vendor.eq_ignore_ascii_case("otto")),
                program: entry.program.clone().filter(|_| app.is_empty()),
            });
        }
    }
    grants.sort_by_key(|grant| grant.kind == Kind::Screenshot);
    grants
}

/// The screenshot rows: every app in the `screenshot` entry.
fn screenshot_rows(entries: &[Entry]) -> Vec<AppRow> {
    entries
        .iter()
        .filter(|e| e.table == store::SCREENSHOT && e.id == store::SCREENSHOT_ID)
        .flat_map(|e| {
            e.apps.iter().map(|(app, permissions)| AppRow {
                app: app.clone(),
                decision: screenshot_decision(permissions),
                shared: e.apps.len() > 1,
            })
        })
        .collect()
}

/// The apps in the `notifications` table's one entry, by app id. An app
/// whose entry says anything but `no` may notify: the portal treats a
/// missing answer as yes.
fn senders(entries: &[Entry]) -> Vec<Sender> {
    let mut senders: Vec<Sender> = entries
        .iter()
        .filter(|entry| entry.table == store::NOTIFICATIONS && entry.id == store::NOTIFICATION_ID)
        .flat_map(|entry| &entry.apps)
        .map(|(app, permissions)| Sender {
            app: app.clone(),
            allowed: !permissions.iter().any(|p| p == "no"),
        })
        .collect();
    senders.sort_by(|a, b| a.app.cmp(&b.app));
    senders.dedup_by(|a, b| a.app == b.app);
    senders
}

/// Read the store and keep what it says, then have the pane redrawn.
fn reload() {
    let snapshot = match Store::connect().and_then(|store| {
        let mut entries = Vec::new();
        for table in [
            store::SCREENCAST,
            store::REMOTE_DESKTOP,
            store::SCREENSHOT,
            store::NOTIFICATIONS,
        ] {
            entries.extend(store.entries(table)?);
        }
        Ok(entries)
    }) {
        Ok(entries) => Snapshot::Ready {
            shares: screen_grants(&entries)
                .into_iter()
                .filter(|g| g.kind != Kind::Screenshot)
                .collect(),
            screenshots: screenshot_rows(&entries),
            senders: senders(&entries),
        },
        Err(why) => {
            eprintln!("privacy: cannot read the permission store: {why}");
            Snapshot::Unavailable
        }
    };
    *SNAPSHOT.write().unwrap() = snapshot;
    settings_client::request_redraw();
}

/// Run `work` on a thread of its own: every store call blocks on the bus.
fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("privacy: could not start {name}: {err}");
    }
}

/// Follow the store's `Changed` signal for as long as the app runs, so an
/// answer given elsewhere — a portal prompt, GNOME Settings, `flatpak
/// permission-reset` — shows up without reopening the pane.
fn watch() {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    in_background("privacy-watch", || {
        let Ok(connection) = zbus::blocking::Connection::session() else {
            return;
        };
        let proxy = match zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.impl.portal.PermissionStore",
            "/org/freedesktop/impl/portal/PermissionStore",
            "org.freedesktop.impl.portal.PermissionStore",
        ) {
            Ok(proxy) => proxy,
            Err(err) => {
                eprintln!("privacy: cannot watch the permission store: {err}");
                return;
            }
        };
        let Ok(changes) = proxy.receive_signal("Changed") else {
            return;
        };
        // Off screen, a change waits for the pane to be shown, which reads
        // the store anyway.
        for _ in changes {
            if SHOWN.load(Ordering::Relaxed) {
                reload();
            }
        }
    });
}

/// Tell the pane whether it is on screen. Coming on screen reads the store
/// afresh, showing what was read last until the answer is in.
pub fn set_shown(shown: bool) {
    let was = SHOWN.swap(shown, Ordering::Relaxed);
    if shown && !was {
        watch();
        in_background("privacy-read", reload);
    }
}

/// A press on a row's Forget or Reset: forget that decision, so the app is
/// asked again.
pub fn remove(id: &str) {
    if let Some(app) = id.strip_prefix(SENDER_ROW) {
        forget_sender(app.to_string());
        return;
    }
    if let Some(target) = slot_target(id) {
        let Some(row) = find_row(&target) else {
            return;
        };
        in_background("privacy-forget", move || {
            if let Err(why) = Store::connect().and_then(|store| forget_row(&store, &row)) {
                eprintln!(
                    "privacy: could not forget screenshots for {:?}: {why}",
                    row.app
                );
            }
            reload();
        });
        return;
    }
    let Some(grant) = parse_row_id(id) else {
        return;
    };
    let shared = match &*SNAPSHOT.read().unwrap() {
        Snapshot::Ready { shares, .. } => shares
            .iter()
            .find(|g| g.table == grant.0 && g.id == grant.1 && g.app == grant.2)
            .map(|g| g.shared),
        _ => None,
    };
    let Some(shared) = shared else {
        return;
    };
    in_background("privacy-forget", move || {
        let (table, entry, app) = grant;
        if let Err(why) =
            Store::connect().and_then(|store| store.forget(&table, &entry, &app, shared))
        {
            eprintln!("privacy: could not forget {table}/{entry} for {app:?}: {why}");
        }
        reload();
    });
}

/// Forget an app's answer about notifications, so it is asked again the next
/// time it sends one. The other apps' answers share the entry and stay.
fn forget_sender(app: String) {
    let others = match &*SNAPSHOT.read().unwrap() {
        Snapshot::Ready { senders, .. } => senders.iter().any(|sender| sender.app != app),
        _ => return,
    };
    in_background("privacy-forget", move || {
        if let Err(why) = Store::connect().and_then(|store| {
            store.forget(store::NOTIFICATIONS, store::NOTIFICATION_ID, &app, others)
        }) {
            eprintln!("privacy: could not forget notifications for {app:?}: {why}");
        }
        reload();
    });
}

/// Forget an app's screenshot answer, so it is asked again.
fn forget_row(store: &Store, row: &AppRow) -> Result<(), String> {
    store.forget(
        store::SCREENSHOT,
        store::SCREENSHOT_ID,
        &row.app,
        row.shared,
    )
}

/// Write `decision` for a screenshot row.
fn write_decision(store: &Store, row: &AppRow, decision: Decision) -> Result<(), String> {
    match screenshot_value(decision) {
        Some(value) => store.set(store::SCREENSHOT, store::SCREENSHOT_ID, &row.app, &[value]),
        None => store.forget(
            store::SCREENSHOT,
            store::SCREENSHOT_ID,
            &row.app,
            row.shared,
        ),
    }
}

/// How many Select rows the pane can hold. A pop-up's menu is built at
/// startup, keyed by the row's identifier, so the rows borrow these fixed
/// identifiers in order; any past the last show their answer as text.
const SLOTS: usize = 32;

/// Every pop-up this module owns, for the menu pool built at startup.
pub fn slot_ids() -> &'static [&'static str] {
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| {
        (0..SLOTS)
            .map(|i| &*format!("privacy.decision.{i}").leak())
            .collect()
    })
}

/// Which row each slot was given when the pane was last built.
static SLOT_TARGETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn slot_target(id: &str) -> Option<String> {
    let index = slot_ids().iter().position(|slot| *slot == id)?;
    SLOT_TARGETS.lock().unwrap().get(index).cloned()
}

fn find_row(app: &str) -> Option<AppRow> {
    match &*SNAPSHOT.read().unwrap() {
        Snapshot::Ready { screenshots, .. } => {
            screenshots.iter().find(|row| row.app == app).cloned()
        }
        _ => None,
    }
}

/// The choices of one of this module's pop-ups. `None` for any other.
pub fn menu_choices(id: &str) -> Option<Vec<crate::discovery::Choice>> {
    slot_target(id)?;
    Some(
        Decision::ALL
            .into_iter()
            .map(|decision| crate::discovery::Choice {
                label: decision.label().to_string(),
                value: decision.label().to_string(),
            })
            .collect(),
    )
}

/// A choice was picked in one of this module's pop-ups. Whether `id` is one
/// of them. The row shows the new answer straight away; the store is written
/// on a thread of its own and read back after.
pub fn choose(id: &str, value: &str) -> bool {
    let Some(target) = slot_target(id) else {
        return false;
    };
    let Some(decision) = Decision::from_label(value) else {
        return true;
    };
    let Some(row) = find_row(&target) else {
        return true;
    };
    if let Snapshot::Ready { screenshots, .. } = &mut *SNAPSHOT.write().unwrap() {
        if let Some(shown) = screenshots.iter_mut().find(|r| r.app == target) {
            shown.decision = decision;
        }
    }
    in_background("privacy-decide", move || {
        if let Err(why) = Store::connect().and_then(|store| write_decision(&store, &row, decision))
        {
            eprintln!(
                "privacy: could not save {decision:?} for {:?}: {why}",
                row.app
            );
        }
        reload();
    });
    true
}

fn row_id(grant: &Grant) -> String {
    format!(
        "{GRANT_ROW}{}{SEP}{}{SEP}{}",
        grant.table, grant.id, grant.app
    )
}

fn parse_row_id(id: &str) -> Option<(String, String, String)> {
    let rest = id.strip_prefix(GRANT_ROW)?;
    let mut parts = rest.splitn(3, SEP);
    Some((
        parts.next()?.to_string(),
        parts.next()?.to_string(),
        parts.next()?.to_string(),
    ))
}

/// `text` as a `&'static str`, kept once however often it is asked for.
fn intern(text: String) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut interned = INTERNED.get_or_init(Default::default).lock().unwrap();
    interned
        .entry(text)
        .or_insert_with_key(|text| text.clone().leak())
}

/// What an app id is called: its desktop entry's name, the id itself when it
/// has none, and one name for every unsandboxed app.
fn app_name(app: &str) -> String {
    if app.is_empty() {
        return otto_kit::t_owned!("privacy-app-unsandboxed");
    }
    otto_kit::desktop_entry::lookup_app(app)
        .map(|info| info.name)
        .unwrap_or_else(|| app.to_string())
}

/// What a grant's app is called. An unsandboxed app's share is a restore
/// token only the program that asked holds, so when that program is known it
/// is named rather than every unsandboxed app.
fn grant_name(grant: &Grant) -> String {
    match &grant.program {
        Some(program) => otto_kit::desktop_entry::lookup_app_by_binary(program)
            .map(|info| info.name)
            .unwrap_or_else(|| program.clone()),
        None => app_name(&grant.app),
    }
}

/// The line under a grant's name: what it lets the app do.
fn grant_detail(grant: &Grant) -> String {
    let what = match (grant.kind, &grant.restored, grant.allowed) {
        (Kind::ScreenCast, Some(Restored::Monitor(screen)), _) => {
            otto_kit::t_owned!("privacy-screencast-monitor", screen = screen.clone())
        }
        (Kind::ScreenCast, Some(Restored::Window), _) => {
            otto_kit::t_owned!("privacy-screencast-window")
        }
        (Kind::ScreenCast, None, _) => otto_kit::t_owned!("privacy-screencast"),
        (Kind::RemoteDesktop, _, _) => otto_kit::t_owned!("privacy-remote-desktop"),
        (Kind::Screenshot, _, true) => otto_kit::t_owned!("privacy-screenshot-allowed"),
        (Kind::Screenshot, _, false) => otto_kit::t_owned!("privacy-screenshot-denied"),
    };
    match &grant.foreign_vendor {
        Some(vendor) => format!(
            "{what} · {}",
            otto_kit::t_owned!("privacy-remembered-by", desktop = vendor.clone())
        ),
        None => what,
    }
}

/// A row that says something rather than holding a value.
fn note(text: &'static str) -> Row {
    Row::new(text, Control::Value(String::new()))
}

fn screenshot_detail(row: &AppRow) -> String {
    let mut detail = otto_kit::t_owned!("privacy-screenshot");
    if row.app.is_empty() {
        detail = format!(
            "{detail} · {}",
            otto_kit::t!("privacy-applies-to-unsandboxed")
        );
    }
    detail
}

/// A Select row, given the next free slot, or its answer as text when
/// there is none left.
fn decision_row(row: &AppRow, detail: String, slots: &mut Vec<String>) -> Row {
    let shown = row.decision.label();
    let label = intern(app_name(&row.app));
    let mut out = match slot_ids().get(slots.len()) {
        Some(slot) => {
            slots.push(row.app.clone());
            let mut out = Row::new(label, Control::Select(shown.to_string()));
            out.id = Some(slot);
            out
        }
        None => Row::new(label, Control::Value(shown.to_string())),
    };
    out = out.detail(detail);
    // A row past the last slot has no identifier for Forget to name.
    let removable = out.id.is_some();
    out.removable(removable).remove_label(remove_word(&row.app))
}

/// What a row's remove button says. One app's answer is forgotten, and it is
/// asked again; the unsandboxed row is every such app's answer at once, which
/// is put back to asking rather than forgotten for one of them.
fn remove_word(app: &str) -> &'static str {
    if app.is_empty() {
        otto_kit::t!("privacy-reset")
    } else {
        otto_kit::t!("privacy-forget")
    }
}

fn screen_rows() -> Vec<Row> {
    let snapshot = SNAPSHOT.read().unwrap();
    let (shares, screenshots) = match &*snapshot {
        Snapshot::Pending => return vec![note(otto_kit::t!("privacy-reading"))],
        Snapshot::Unavailable => {
            return vec![note(otto_kit::t!("privacy-store-unavailable"))
                .detail(otto_kit::t!("privacy-store-unavailable-detail"))]
        }
        Snapshot::Ready {
            shares,
            screenshots,
            ..
        } => (shares, screenshots),
    };
    if shares.is_empty() && screenshots.is_empty() {
        return vec![note(otto_kit::t!("privacy-screen-none"))];
    }
    let mut slots = Vec::new();
    let mut rows: Vec<Row> = shares
        .iter()
        .map(|grant| {
            let mut detail = grant_detail(grant);
            if grant.app.is_empty() && grant.program.is_none() {
                detail = format!(
                    "{detail} · {}",
                    otto_kit::t!("privacy-applies-to-unsandboxed")
                );
            }
            let mut row = Row::new(intern(grant_name(grant)), Control::Value(String::new()))
                .detail(detail)
                .removable(true)
                .remove_label(otto_kit::t!("privacy-forget"));
            row.id = Some(intern(row_id(grant)));
            row
        })
        .collect();
    rows.extend(
        screenshots
            .iter()
            .map(|row| decision_row(row, screenshot_detail(row), &mut slots)),
    );
    *SLOT_TARGETS.lock().unwrap() = slots;
    rows
}

/// A switch was flipped. Whether `id` is one of this pane's, so `main.rs`
/// knows not to send it to the compositor.
///
/// The switch shows the new answer straight away and the store is written
/// on a thread of its own; the read that follows puts the switch back if
/// the write did not take.
pub fn apply(id: &str, value: &settings_client::Value) -> bool {
    let Some(app) = id.strip_prefix(SENDER_ROW) else {
        return false;
    };
    let settings_client::Value::Bool(allowed) = *value else {
        return true;
    };
    if let Snapshot::Ready { senders, .. } = &mut *SNAPSHOT.write().unwrap() {
        if let Some(sender) = senders.iter_mut().find(|s| s.app == app) {
            sender.allowed = allowed;
        }
    }
    let app = app.to_string();
    in_background("privacy-notifications", move || {
        let answer = if allowed { "yes" } else { "no" };
        if let Err(why) = Store::connect().and_then(|store| {
            store.set(
                store::NOTIFICATIONS,
                store::NOTIFICATION_ID,
                &app,
                &[answer],
            )
        }) {
            eprintln!("privacy: could not turn notifications {answer} for {app:?}: {why}");
        }
        reload();
    });
    true
}

fn notification_rows() -> Vec<Row> {
    match &*SNAPSHOT.read().unwrap() {
        // Said once, in the group above.
        Snapshot::Pending | Snapshot::Unavailable => Vec::new(),
        Snapshot::Ready { senders, .. } if senders.is_empty() => {
            vec![note(otto_kit::t!("privacy-notifications-none"))]
        }
        Snapshot::Ready { senders, .. } => senders
            .iter()
            .map(|sender| {
                let mut row = Row::new(
                    intern(app_name(&sender.app)),
                    Control::Toggle(sender.allowed),
                );
                row.id = Some(intern(format!("{SENDER_ROW}{}", sender.app)));
                // Forget drops the answer: the app asks again next time.
                row.removable(true).remove_label(remove_word(&sender.app))
            })
            .collect(),
    }
}

pub fn build() -> Pane {
    let mut groups = vec![group(otto_kit::t!("privacy-group-screen"), screen_rows())];

    let notifications = notification_rows();
    if !notifications.is_empty() {
        groups.push(group(
            otto_kit::t!("privacy-group-notifications"),
            notifications,
        ));
    }
    Pane {
        name: otto_kit::t!("settings-pane-privacy"),
        icon: "hand",
        intro: None,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(table: &str, id: &str, apps: &[(&str, &str)]) -> Entry {
        Entry {
            table: table.into(),
            id: id.into(),
            apps: apps
                .iter()
                .map(|(app, permission)| (app.to_string(), vec![permission.to_string()]))
                .collect(),
            restored: None,
            vendor: None,
            program: None,
        }
    }

    #[test]
    fn every_app_on_every_entry_is_a_grant_screenshots_last() {
        let mut cast = entry(
            store::SCREENCAST,
            "token-1",
            &[("com.obsproject.Studio", "yes")],
        );
        cast.restored = Some(Restored::Monitor("eDP-1".into()));
        cast.vendor = Some("otto".into());
        let mut remote = entry(
            store::REMOTE_DESKTOP,
            "token-2",
            &[("org.kde.kdeconnect.daemon", "yes")],
        );
        remote.vendor = Some("KDE".into());
        let entries = [
            entry(
                store::SCREENSHOT,
                store::SCREENSHOT_ID,
                &[("", "yes"), ("org.foo.Snap", "no")],
            ),
            cast,
            remote,
        ];
        let grants = screen_grants(&entries);
        let summary: Vec<(Kind, &str, bool, bool)> = grants
            .iter()
            .map(|g| (g.kind, g.app.as_str(), g.allowed, g.shared))
            .collect();
        assert_eq!(
            summary,
            [
                (Kind::ScreenCast, "com.obsproject.Studio", true, false),
                (
                    Kind::RemoteDesktop,
                    "org.kde.kdeconnect.daemon",
                    true,
                    false
                ),
                (Kind::Screenshot, "", true, true),
                (Kind::Screenshot, "org.foo.Snap", false, true),
            ]
        );
        assert_eq!(grants[0].restored, Some(Restored::Monitor("eDP-1".into())));
        // Otto's own payload is not "remembered by" anyone else.
        assert_eq!(grants[0].foreign_vendor, None);
        assert_eq!(grants[1].foreign_vendor.as_deref(), Some("KDE"));
    }

    #[test]
    fn an_unsandboxed_share_is_named_for_its_program() {
        let mut obs = entry(store::SCREENCAST, "token-1", &[("", "yes")]);
        obs.program = Some("obs".into());
        let mut flatpak = entry(
            store::SCREENCAST,
            "token-2",
            &[("com.google.Chrome", "yes")],
        );
        flatpak.program = Some("chrome".into());
        let grants = screen_grants(&[obs, flatpak]);
        assert_eq!(grants[0].program.as_deref(), Some("obs"));
        // A sandboxed app is named by its app id, whatever the payload says.
        assert_eq!(grants[1].program, None);
    }

    #[test]
    fn tables_and_ids_that_grant_no_screen_access_are_left_out() {
        let entries = [
            entry(
                store::NOTIFICATIONS,
                store::NOTIFICATION_ID,
                &[("org.foo", "yes")],
            ),
            entry(store::SCREENSHOT, "something-else", &[("org.foo", "yes")]),
        ];
        assert!(screen_grants(&entries).is_empty());
    }

    #[test]
    fn every_app_in_the_notification_entry_is_a_sender() {
        let entries = [
            entry(
                store::NOTIFICATIONS,
                store::NOTIFICATION_ID,
                &[
                    ("com.spotify.Client", "no"),
                    ("com.github.x.WhatsApp", "yes"),
                ],
            ),
            entry(
                store::SCREENSHOT,
                store::SCREENSHOT_ID,
                &[("org.foo", "yes")],
            ),
        ];
        assert_eq!(
            senders(&entries),
            [
                Sender {
                    app: "com.github.x.WhatsApp".into(),
                    allowed: true
                },
                Sender {
                    app: "com.spotify.Client".into(),
                    allowed: false
                },
            ]
        );
    }

    #[test]
    fn a_sender_with_no_answer_may_notify() {
        let mut unanswered = entry(store::NOTIFICATIONS, store::NOTIFICATION_ID, &[]);
        unanswered.apps = vec![("org.foo".into(), Vec::new())];
        assert!(senders(&[unanswered])[0].allowed);
    }

    fn apps(rows: &[AppRow]) -> Vec<(&str, Decision)> {
        rows.iter().map(|r| (r.app.as_str(), r.decision)).collect()
    }

    #[test]
    fn a_screenshot_row_is_each_app_in_the_portal_entry() {
        let entries = [entry(
            store::SCREENSHOT,
            store::SCREENSHOT_ID,
            &[("", "yes"), ("org.foo", "no")],
        )];
        let rows = screenshot_rows(&entries);
        assert_eq!(
            apps(&rows),
            [("", Decision::Allow), ("org.foo", Decision::Deny)]
        );
        assert!(rows[0].shared);
    }

    #[test]
    fn screenshot_ask_is_no_answer_at_all() {
        assert_eq!(screenshot_value(Decision::Allow), Some("yes"));
        assert_eq!(screenshot_value(Decision::Deny), Some("no"));
        assert_eq!(screenshot_value(Decision::Ask), None);
        assert_eq!(screenshot_decision(&["yes".into()]), Decision::Allow);
        assert_eq!(screenshot_decision(&["no".into()]), Decision::Deny);
        assert_eq!(screenshot_decision(&[]), Decision::Ask);
    }

    #[test]
    fn a_row_id_names_its_entry_and_app_even_when_the_app_is_empty() {
        let grant = &screen_grants(&[entry(store::SCREENCAST, "a-token", &[("", "yes")])])[0];
        assert_eq!(
            parse_row_id(&row_id(grant)),
            Some((store::SCREENCAST.into(), "a-token".into(), String::new()))
        );
        assert_eq!(parse_row_id("privacy.notifications:org.foo"), None);
    }
}
