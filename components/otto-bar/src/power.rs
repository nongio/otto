//! Battery and CPU power state for the bar's battery indicator.
//!
//! The bar reads this itself rather than hosting a tray applet for it. SNI
//! carries an icon and a tooltip — there is no label in the spec that every
//! host renders — so a percentage published through the tray would be a
//! percentage nobody can see. UPower is the source every other panel uses.
//!
//! Two things live here: the battery, polled from UPower (falling back to
//! sysfs when UPower is not running), and the CPU's current frequency and
//! policy, read straight from `sysfs` at the moment the menu is built. The
//! CPU numbers are deliberately not polled — they move every few
//! milliseconds, and nothing shows them until a menu is open.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use otto_kit::AppContext;

use futures_util::StreamExt;
use zbus::zvariant::OwnedValue;
use zbus::{proxy, Connection};

use crate::config::{battery_config, ProfileEntry};

/// Battery state shared with the draw loop.
static POWER_STATE: LazyLock<Mutex<Battery>> = LazyLock::new(|| Mutex::new(Battery::default()));

/// Power profiles offered by the menu, and which one is live.
static PROFILE_STATE: LazyLock<Mutex<Profiles>> = LazyLock::new(|| Mutex::new(Profiles::default()));

/// Bumped whenever anything the bar draws changes.
static POWER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// System bus connection, kept for setting the active profile.
static SYSTEM_BUS: LazyLock<Mutex<Option<Connection>>> = LazyLock::new(|| Mutex::new(None));

/// What the battery is doing. Mirrors UPower's `State` enum, narrowed to the
/// cases the indicator draws differently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChargeState {
    Charging,
    Discharging,
    Full,
    #[default]
    Unknown,
}

impl ChargeState {
    /// UPower `Device.State`: 1 charging, 2 discharging, 3 empty, 4 fully
    /// charged, 5 pending charge, 6 pending discharge.
    fn from_upower(raw: u32) -> Self {
        match raw {
            1 | 5 => Self::Charging,
            2 | 6 => Self::Discharging,
            4 => Self::Full,
            _ => Self::Unknown,
        }
    }

    fn from_sysfs(raw: &str) -> Self {
        match raw.trim() {
            "Charging" => Self::Charging,
            "Discharging" => Self::Discharging,
            "Full" => Self::Full,
            _ => Self::Unknown,
        }
    }

    pub fn is_charging(self) -> bool {
        matches!(self, Self::Charging)
    }
}

/// Everything the indicator draws.
#[derive(Clone, Debug, Default)]
pub struct Battery {
    /// False on a desktop, and until the first reading lands.
    pub present: bool,
    /// 0.0–100.0.
    pub percentage: f64,
    pub state: ChargeState,
    /// Seconds to empty when discharging, to full when charging. 0 = unknown.
    pub seconds_left: i64,
}

impl Battery {
    /// Whether two readings would draw the same bar. Percentage is compared
    /// as a whole number because that is all the indicator shows — a battery
    /// drifting from 87.4% to 87.3% is not a repaint.
    fn looks_same_as(&self, other: &Self) -> bool {
        self.present == other.present
            && self.state == other.state
            && self.percentage.round() as i64 == other.percentage.round() as i64
            // A changed estimate shows up in the menu, but only the menu, and
            // the menu re-reads this when it opens.
            && (self.seconds_left == 0) == (other.seconds_left == 0)
    }
}

/// A selectable power profile: what the menu lists.
#[derive(Clone, Debug)]
pub struct Profile {
    /// Identifier passed back to whichever backend owns it.
    pub id: String,
    /// What the menu shows.
    pub label: String,
    pub active: bool,
}

/// Where the profile list came from — which decides what selecting one does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// power-profiles-daemon over D-Bus.
    PowerProfiles,
    /// `[[battery.profiles]]` entries from the config file, each a command.
    Commands,
    /// Nothing can switch: the menu lists the governors read-only.
    #[default]
    ReadOnly,
}

#[derive(Clone, Debug, Default)]
pub struct Profiles {
    pub backend: Backend,
    pub entries: Vec<Profile>,
    /// Which D-Bus name answered, so the setter talks to the same one.
    ppd_service: Option<(&'static str, &'static str, &'static str)>,
}

/// What the CPU is doing right now, read on demand.
#[derive(Clone, Debug, Default)]
pub struct Cpu {
    /// Mean of every core's current frequency, in GHz.
    pub avg_ghz: f64,
    /// The busiest core's, in GHz.
    pub max_ghz: f64,
    pub cores: usize,
    pub governor: Option<String>,
    /// intel_pstate's energy/performance preference, where the driver has one.
    pub epp: Option<String>,
}

/// Current battery reading.
pub fn battery() -> Battery {
    POWER_STATE.lock().unwrap().clone()
}

/// Current profile list.
pub fn profiles() -> Profiles {
    PROFILE_STATE.lock().unwrap().clone()
}

/// Changes whenever the drawn state changes.
pub fn generation() -> u64 {
    POWER_GENERATION.load(Ordering::Relaxed)
}

fn bump() {
    POWER_GENERATION.fetch_add(1, Ordering::Relaxed);
    AppContext::request_wakeup();
}

// ---------------------------------------------------------------------------
// CPU — read from sysfs, on demand
// ---------------------------------------------------------------------------

const CPUFREQ: &str = "/sys/devices/system/cpu";

/// Read the CPU's current frequency and policy.
///
/// Called when the menu opens, not on a timer: `scaling_cur_freq` changes far
/// faster than anything could usefully display, and reading it walks one file
/// per core.
pub fn cpu() -> Cpu {
    let mut total_khz = 0u64;
    let mut max_khz = 0u64;
    let mut cores = 0usize;

    let Ok(entries) = std::fs::read_dir(CPUFREQ) else {
        return Cpu::default();
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // cpu0, cpu1, … — but not cpuidle or cpufreq, which are siblings.
        if !name.starts_with("cpu") || !name[3..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let path = entry.path().join("cpufreq/scaling_cur_freq");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(khz) = raw.trim().parse::<u64>() else {
            continue;
        };
        total_khz += khz;
        max_khz = max_khz.max(khz);
        cores += 1;
    }

    let read = |leaf: &str| {
        std::fs::read_to_string(format!("{CPUFREQ}/cpu0/cpufreq/{leaf}"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    Cpu {
        avg_ghz: if cores > 0 {
            total_khz as f64 / cores as f64 / 1_000_000.0
        } else {
            0.0
        },
        max_ghz: max_khz as f64 / 1_000_000.0,
        cores,
        governor: read("scaling_governor"),
        epp: read("energy_performance_preference"),
    }
}

/// The governors this kernel will accept, in the order it lists them.
fn available_governors() -> Vec<String> {
    std::fs::read_to_string(format!(
        "{CPUFREQ}/cpu0/cpufreq/scaling_available_governors"
    ))
    .map(|s| s.split_whitespace().map(str::to_string).collect())
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// UPower
// ---------------------------------------------------------------------------

/// The aggregate device: one battery's worth of numbers however many cells
/// the machine actually has, and the only device present on a desktop.
#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait UPowerDevice {
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    /// 2 is a battery; a desktop's DisplayDevice reports 0 (unknown).
    #[zbus(property, name = "Type")]
    fn device_type(&self) -> zbus::Result<u32>;
}

async fn read_upower(proxy: &UPowerDeviceProxy<'_>) -> Option<Battery> {
    let device_type = proxy.device_type().await.ok()?;
    let present = proxy.is_present().await.unwrap_or(false) && device_type == 2;
    if !present {
        return Some(Battery::default());
    }
    let state = ChargeState::from_upower(proxy.state().await.unwrap_or(0));
    let seconds_left = match state {
        ChargeState::Charging => proxy.time_to_full().await.unwrap_or(0),
        ChargeState::Discharging => proxy.time_to_empty().await.unwrap_or(0),
        _ => 0,
    };
    Some(Battery {
        present: true,
        percentage: proxy.percentage().await.unwrap_or(0.0),
        state,
        seconds_left,
    })
}

/// Last-resort reading for a machine with no UPower. Loses the time estimate,
/// which sysfs does not carry: it has energy counters and a current, and
/// turning those into a duration is what UPower is for.
fn read_sysfs() -> Battery {
    read_sysfs_in(std::path::Path::new("/sys/class/power_supply"))
}

fn read_sysfs_in(root: &std::path::Path) -> Battery {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Battery::default();
    };
    let read = |path: &std::path::Path, name: &str| {
        std::fs::read_to_string(path.join(name))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if read(&path, "type") != "Battery" {
            continue;
        }
        // A wireless mouse, keyboard or gamepad is a `Battery` too, marked
        // `scope=Device`. On a desktop it is the only one there is, and its
        // charge is not the machine's — UPower leaves it out of the
        // DisplayDevice for the same reason.
        if read(&path, "scope") == "Device" || read(&path, "present") == "0" {
            continue;
        }
        let Ok(capacity) = std::fs::read_to_string(path.join("capacity")) else {
            continue;
        };
        let Ok(percentage) = capacity.trim().parse::<f64>() else {
            continue;
        };
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        return Battery {
            present: true,
            percentage,
            state: ChargeState::from_sysfs(&status),
            seconds_left: 0,
        };
    }
    Battery::default()
}

fn store(reading: Battery) {
    let mut slot = POWER_STATE.lock().unwrap();
    if slot.looks_same_as(&reading) {
        *slot = reading;
        return;
    }
    *slot = reading;
    drop(slot);
    bump();
}

// ---------------------------------------------------------------------------
// Power profiles
// ---------------------------------------------------------------------------

/// power-profiles-daemon, under both names it has answered to: the
/// `net.hadess` one it shipped with, and the `org.freedesktop.UPower` one it
/// moved to. Which is live differs by distribution and version.
const PPD_NAMES: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    (
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    ),
];

/// Ask one of the daemon's names for its profiles. `Ok(None)` means that name
/// is not there — try the next; an error means it is there and unhappy.
async fn read_ppd(
    conn: &Connection,
    (service, path, interface): (&'static str, &'static str, &'static str),
) -> zbus::Result<Option<(Vec<Profile>, (&'static str, &'static str, &'static str))>> {
    let proxy = zbus::Proxy::new(conn, service, path, interface).await?;

    // Profiles is an array of dicts, each with at least a "Profile" key:
    // power-saver, balanced, performance.
    let raw: Vec<std::collections::HashMap<String, OwnedValue>> =
        match proxy.get_property("Profiles").await {
            Ok(v) => v,
            // A masked or absent daemon fails to activate. That is a "not there",
            // not a fault worth logging on every poll.
            Err(_) => return Ok(None),
        };
    let active: String = proxy
        .get_property("ActiveProfile")
        .await
        .unwrap_or_default();

    let entries = raw
        .iter()
        .filter_map(|dict| {
            let id = dict.get("Profile")?;
            let id = <&str>::try_from(id).ok()?.to_string();
            Some(Profile {
                label: profile_label(&id),
                active: id == active,
                id,
            })
        })
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some((entries, (service, path, interface))))
}

/// A daemon profile id turned into something to read. Falls back to the id
/// itself for a profile this does not know, rather than hiding it.
fn profile_label(id: &str) -> String {
    match id {
        "power-saver" => otto_kit::t!("bar-power-saver").to_string(),
        "balanced" => otto_kit::t!("bar-power-balanced").to_string(),
        "performance" => otto_kit::t!("bar-power-performance").to_string(),
        other => other.to_string(),
    }
}

/// Build the profile list from whichever backend the config asks for.
async fn read_profiles(conn: Option<&Connection>) -> Profiles {
    let cfg = battery_config();

    // Configured commands win when there are any: they are an explicit
    // statement about this machine, and the daemon may well be masked
    // precisely because something else is managing the CPU.
    if !cfg.profiles.is_empty() && cfg.profile_backend != "power-profiles" {
        let live = cpu();
        let entries = cfg
            .profiles
            .iter()
            .enumerate()
            .map(|(i, entry)| Profile {
                id: i.to_string(),
                label: entry.label.clone(),
                active: profile_entry_is_live(entry, &live),
            })
            .collect();
        return Profiles {
            backend: Backend::Commands,
            entries,
            ppd_service: None,
        };
    }

    if cfg.profile_backend != "commands" {
        if let Some(conn) = conn {
            for name in PPD_NAMES {
                match read_ppd(conn, name).await {
                    Ok(Some((entries, service))) => {
                        return Profiles {
                            backend: Backend::PowerProfiles,
                            entries,
                            ppd_service: Some(service),
                        }
                    }
                    Ok(None) => continue,
                    Err(e) => tracing::debug!("power profiles via {}: {e}", name.0),
                }
            }
        }
    }

    // Nothing can switch. List what the kernel has so the menu still says
    // what the CPU is set to, greyed out.
    let live = cpu();
    let entries = available_governors()
        .into_iter()
        .map(|id| Profile {
            active: live.governor.as_deref() == Some(id.as_str()),
            label: id.clone(),
            id,
        })
        .collect();
    Profiles {
        backend: Backend::ReadOnly,
        entries,
        ppd_service: None,
    }
}

/// Whether a configured command entry describes what the CPU is doing now.
/// Only entries that say how to recognise themselves can be check-marked.
fn profile_entry_is_live(entry: &ProfileEntry, live: &Cpu) -> bool {
    let governor_matches = entry
        .governor
        .as_ref()
        .is_some_and(|want| live.governor.as_deref() == Some(want.as_str()));
    let epp_matches = entry
        .epp
        .as_ref()
        .is_some_and(|want| live.epp.as_deref() == Some(want.as_str()));

    match (entry.governor.is_some(), entry.epp.is_some()) {
        // Both stated: both must hold, so two entries sharing a governor but
        // differing in EPP do not both light up.
        (true, true) => governor_matches && epp_matches,
        (true, false) => governor_matches,
        (false, true) => epp_matches,
        (false, false) => false,
    }
}

/// Select a profile — the menu's one action.
pub fn activate_profile(id: &str) {
    let state = profiles();
    match state.backend {
        Backend::PowerProfiles => {
            let Some((service, path, interface)) = state.ppd_service else {
                return;
            };
            let conn = SYSTEM_BUS.lock().unwrap().clone();
            let Some(conn) = conn else { return };
            let id = id.to_string();
            tokio::runtime::Handle::current().spawn(async move {
                let proxy = match zbus::Proxy::new(&conn, service, path, interface).await {
                    Ok(p) => p,
                    Err(e) => return tracing::warn!("power profiles: {e}"),
                };
                if let Err(e) = proxy.set_property("ActiveProfile", id.as_str()).await {
                    tracing::warn!("could not switch power profile: {e}");
                    return;
                }
                refresh_profiles().await;
            });
        }
        Backend::Commands => {
            let Ok(index) = id.parse::<usize>() else {
                return;
            };
            let cfg = battery_config();
            let Some(entry) = cfg.profiles.get(index) else {
                return;
            };
            let Some((program, args)) = entry.command.split_first() else {
                tracing::warn!("battery profile {:?} has an empty command", entry.label);
                return;
            };
            // Detached: a profile switch that needs a polkit prompt can take
            // as long as the person takes to answer it, and the bar cannot
            // wait for that.
            match std::process::Command::new(program).args(args).spawn() {
                Ok(mut child) => {
                    tokio::runtime::Handle::current().spawn(async move {
                        // Reaped on a blocking thread so a command that sits
                        // on a password prompt cannot stall the runtime.
                        let _ = tokio::task::spawn_blocking(move || child.wait()).await;
                        // Whatever it did, the governor may now be different.
                        refresh_profiles().await;
                    });
                }
                Err(e) => tracing::warn!("battery profile {:?}: {e}", entry.label),
            }
        }
        Backend::ReadOnly => {}
    }
}

async fn refresh_profiles() {
    let conn = SYSTEM_BUS.lock().unwrap().clone();
    let next = read_profiles(conn.as_ref()).await;
    let mut slot = PROFILE_STATE.lock().unwrap();
    let changed = slot.backend != next.backend
        || slot.entries.len() != next.entries.len()
        || slot
            .entries
            .iter()
            .zip(&next.entries)
            .any(|(a, b)| a.id != b.id || a.active != b.active);
    *slot = next;
    drop(slot);
    if changed {
        bump();
    }
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// Start watching the battery. Idempotent, like the tray's watcher.
pub fn spawn_power_watcher() {
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    tokio::spawn(async move {
        run_watcher().await;
    });
}

async fn run_watcher() {
    let conn = match Connection::system().await {
        Ok(conn) => {
            *SYSTEM_BUS.lock().unwrap() = Some(conn.clone());
            Some(conn)
        }
        Err(e) => {
            // No system bus is survivable: sysfs still has a percentage.
            tracing::warn!("no system bus, reading the battery from sysfs: {e}");
            None
        }
    };

    refresh_profiles().await;

    let proxy = match conn.as_ref() {
        Some(conn) => UPowerDeviceProxy::new(conn).await.ok(),
        None => None,
    };

    // First reading before the first frame, so the bar draws a battery rather
    // than an empty one that fills in a moment later.
    store(match proxy.as_ref() {
        Some(proxy) => read_upower(proxy).await.unwrap_or_else(read_sysfs),
        None => read_sysfs(),
    });

    // UPower signals every change, which is what makes the indicator move the
    // instant the cable comes out. The poll behind it is for the sysfs path
    // and for anything the daemon forgets to announce.
    let mut changes = match proxy.as_ref() {
        Some(proxy) => proxy.receive_percentage_changed().await.boxed(),
        None => futures_util::stream::empty().boxed(),
    };
    let mut states = match proxy.as_ref() {
        Some(proxy) => proxy.receive_state_changed().await.boxed(),
        None => futures_util::stream::empty().boxed(),
    };

    let interval = Duration::from_secs(battery_config().update_interval.max(1));
    loop {
        tokio::select! {
            _ = changes.next() => {}
            _ = states.next() => {}
            _ = tokio::time::sleep(interval) => {}
        }
        store(match proxy.as_ref() {
            Some(proxy) => read_upower(proxy).await.unwrap_or_else(read_sysfs),
            None => read_sysfs(),
        });
    }
}

/// A frequency in GHz to two places, with the locale's decimal separator:
/// "2.80" in English, "2,80" in German. Passed to the catalogue already
/// formatted, because Fluent's own number formatting has no locale data
/// behind it here and would write a point everywhere.
pub fn format_ghz(value: f64) -> String {
    let posix = otto_kit::i18n::posix_locale();
    let language = posix.split(['_', '-']).next().unwrap_or("");
    let text = format!("{value:.2}");
    if uses_decimal_comma(language) {
        text.replace('.', ",")
    } else {
        text
    }
}

/// Whether a language writes its decimals with a comma. Covers the
/// catalogues Otto ships; anything else keeps the point.
fn uses_decimal_comma(language: &str) -> bool {
    matches!(
        language,
        "de" | "es" | "fr" | "it" | "pl" | "pt" | "ru" | "uk"
    )
}

/// "2:14" — a duration the way a battery estimate is written. Returns None
/// for an unknown estimate, which is what UPower reports while it works one
/// out after a state change.
pub fn format_duration(seconds: i64) -> Option<String> {
    if seconds <= 0 {
        return None;
    }
    let minutes = seconds / 60;
    Some(format!("{}:{:02}", minutes / 60, minutes % 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upower_states_map_to_what_the_indicator_draws() {
        assert_eq!(ChargeState::from_upower(1), ChargeState::Charging);
        // Pending charge is still "plugged in and heading up" as far as the
        // bolt is concerned.
        assert_eq!(ChargeState::from_upower(5), ChargeState::Charging);
        assert_eq!(ChargeState::from_upower(2), ChargeState::Discharging);
        assert_eq!(ChargeState::from_upower(6), ChargeState::Discharging);
        assert_eq!(ChargeState::from_upower(4), ChargeState::Full);
        assert_eq!(ChargeState::from_upower(3), ChargeState::Unknown);
        assert_eq!(ChargeState::from_upower(99), ChargeState::Unknown);
    }

    fn supply(root: &std::path::Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, contents) in files {
            std::fs::write(dir.join(file), format!("{contents}\n")).unwrap();
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("otto-bar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_desktop_with_a_wireless_mouse_has_no_battery() {
        let root = scratch("desktop");
        supply(&root, "ACAD", &[("type", "Mains")]);
        supply(
            &root,
            "hidpp_battery_0",
            &[("type", "Battery"), ("scope", "Device"), ("capacity", "80")],
        );
        assert!(!read_sysfs_in(&root).present);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_laptop_reports_its_own_battery_not_the_mouse() {
        let root = scratch("laptop");
        supply(
            &root,
            "hidpp_battery_0",
            &[("type", "Battery"), ("scope", "Device"), ("capacity", "80")],
        );
        supply(
            &root,
            "BAT1",
            &[
                ("type", "Battery"),
                ("present", "1"),
                ("capacity", "51"),
                ("status", "Discharging"),
            ],
        );
        let battery = read_sysfs_in(&root);
        assert!(battery.present);
        assert_eq!(battery.percentage, 51.0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fraction_of_a_percent_is_not_a_repaint() {
        let a = Battery {
            present: true,
            percentage: 87.4,
            state: ChargeState::Discharging,
            seconds_left: 8000,
        };
        let b = Battery {
            percentage: 87.3,
            ..a.clone()
        };
        assert!(a.looks_same_as(&b));

        // A whole percent is.
        let c = Battery {
            percentage: 86.0,
            ..a.clone()
        };
        assert!(!a.looks_same_as(&c));

        // So is the cable coming out.
        let d = Battery {
            state: ChargeState::Charging,
            ..a.clone()
        };
        assert!(!a.looks_same_as(&d));

        // And so is losing the estimate entirely, because the menu shows it.
        let e = Battery {
            seconds_left: 0,
            ..a.clone()
        };
        assert!(!a.looks_same_as(&e));
    }

    #[test]
    fn decimals_follow_the_language() {
        for comma in ["de", "es", "fr", "it", "pl", "pt", "ru", "uk"] {
            assert!(uses_decimal_comma(comma), "{comma}");
        }
        for point in ["en", "ja", "zh", ""] {
            assert!(!uses_decimal_comma(point), "{point}");
        }
    }

    #[test]
    fn durations_read_as_time_remaining() {
        assert_eq!(format_duration(8040).as_deref(), Some("2:14"));
        assert_eq!(format_duration(60).as_deref(), Some("0:01"));
        // Unknown, which UPower reports as zero while it recalculates.
        assert_eq!(format_duration(0), None);
        assert_eq!(format_duration(-1), None);
    }

    #[test]
    fn only_an_entry_that_says_how_to_recognise_itself_is_checked() {
        let live = Cpu {
            governor: Some("powersave".into()),
            epp: Some("balance_power".into()),
            ..Cpu::default()
        };

        let by_governor = ProfileEntry {
            label: "Saver".into(),
            command: vec!["true".into()],
            governor: Some("powersave".into()),
            epp: None,
        };
        assert!(profile_entry_is_live(&by_governor, &live));

        // Same governor, different EPP: two entries that differ only in EPP
        // must not both light up.
        let both = ProfileEntry {
            epp: Some("power".into()),
            ..by_governor.clone()
        };
        assert!(!profile_entry_is_live(&both, &live));

        // Says nothing about itself, so it is never the active one.
        let silent = ProfileEntry {
            governor: None,
            epp: None,
            ..by_governor.clone()
        };
        assert!(!profile_entry_is_live(&silent, &live));
    }
}
