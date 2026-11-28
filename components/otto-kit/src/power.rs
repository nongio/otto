//! Sleep, hibernate, restart, shut down and lock, asked of logind.
//!
//! The login screen, the lock screen and the bar's Otto menu all offer these,
//! and all of them go through here: one set of actions, one call to logind,
//! one reading of which kind of sleep the person prefers.
//!
//! The actions go to logind over the system bus rather than through
//! `systemctl`: logind is what `systemctl` would have asked anyway, and it says
//! why it refused in a D-Bus error instead of on a stderr nobody reads.
//! Whether the caller may act is polkit's call; the requests are interactive,
//! so polkit can ask for a password where its policy says to.
//!
//! Every function here blocks on the bus. Call them off the thread that draws.

// Rust guideline compliant 2026-02-21

use std::sync::{Arc, OnceLock};

use crate::dbus::login1::Login1ManagerProxyBlocking;

/// Something the machine can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerAction {
    /// Suspend to RAM.
    Suspend,
    /// Write memory to disk and power off.
    Hibernate,
    Restart,
    Shutdown,
}

impl PowerAction {
    /// The action's name in catalogue keys and logs: `suspend`, `hibernate`,
    /// `restart`, `shutdown`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Suspend => "suspend",
            Self::Hibernate => "hibernate",
            Self::Restart => "restart",
            Self::Shutdown => "shutdown",
        }
    }

    /// The Solar glyph that stands for the action.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Suspend => "otto-suspend-symbolic",
            Self::Hibernate => "otto-hibernate-symbolic",
            Self::Restart => "otto-restart-symbolic",
            Self::Shutdown => "otto-shutdown-symbolic",
        }
    }
}

/// Which way of sleeping a sleep button takes, and which one holding Cmd
/// takes instead.
///
/// The preferred one is `[power_management] sleep` in Otto's config:
/// `"suspend"` (the default) or `"hibernate"`. Where the machine cannot
/// hibernate, sleeping is always a suspend and there is no alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sleep {
    /// What a sleep button does.
    pub preferred: PowerAction,
    /// What it does while Cmd is held, if there is another way to sleep.
    pub alternative: Option<PowerAction>,
}

impl Sleep {
    /// Read the preference and ask logind whether hibernating is possible.
    ///
    /// Blocks on the system bus the first time a process asks; the answer is
    /// kept after that, since swap and a resume device do not come and go
    /// while a session runs.
    pub fn current() -> Self {
        Self::resolve(preferred_sleep(), can_hibernate())
    }

    /// What sleeping is before [`Sleep::current`] has answered: a suspend,
    /// with nothing to swap to.
    pub const SUSPEND: Self = Self {
        preferred: PowerAction::Suspend,
        alternative: None,
    };

    /// [`Sleep::current`] on a thread of its own, for a client that must not
    /// wait on the bus before it draws (a lock screen going up as the machine
    /// suspends). The cell is filled once logind has answered, and the event
    /// loop is woken to redraw; until then, use [`Sleep::SUSPEND`].
    pub fn current_in_background() -> Arc<OnceLock<Self>> {
        let cell = Arc::new(OnceLock::new());
        let fill = Arc::clone(&cell);
        let spawned = std::thread::Builder::new()
            .name("sleep-lookup".to_string())
            .spawn(move || {
                let _ = fill.set(Self::current());
                crate::AppContext::request_wakeup();
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the sleep lookup; suspending only");
        }
        cell
    }

    /// The action for the button as drawn now: the alternative while Cmd is
    /// held, the preferred one otherwise.
    pub fn action(self, cmd_held: bool) -> PowerAction {
        match (cmd_held, self.alternative) {
            (true, Some(alternative)) => alternative,
            _ => self.preferred,
        }
    }

    fn resolve(preferred: PowerAction, can_hibernate: bool) -> Self {
        if !can_hibernate {
            return Self {
                preferred: PowerAction::Suspend,
                alternative: None,
            };
        }
        let alternative = match preferred {
            PowerAction::Hibernate => PowerAction::Suspend,
            _ => PowerAction::Hibernate,
        };
        Self {
            preferred: if preferred == PowerAction::Hibernate {
                PowerAction::Hibernate
            } else {
                PowerAction::Suspend
            },
            alternative: Some(alternative),
        }
    }
}

/// The sleep the config asks for. The system config is read first, so the
/// person's own overrides it; a greeter, running as a user with no config of
/// its own, sees only the system's.
fn preferred_sleep() -> PowerAction {
    let mut preferred = PowerAction::Suspend;
    for path in otto_foundations::xdg::otto_config_paths("config.toml") {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(sleep) = sleep_in_config(&text) {
            preferred = sleep;
        }
    }
    preferred
}

/// `[power_management] sleep` in one config file's text, if it is set to
/// something this understands.
fn sleep_in_config(text: &str) -> Option<PowerAction> {
    let table: toml::Table = text.parse().ok()?;
    match table.get("power_management")?.get("sleep")?.as_str()? {
        "suspend" => Some(PowerAction::Suspend),
        "hibernate" => Some(PowerAction::Hibernate),
        other => {
            tracing::warn!(value = other, "unknown power_management.sleep, suspending");
            None
        }
    }
}

/// Whether logind would hibernate for this caller, asked once per process.
///
/// `"challenge"` counts: polkit asking for a password is still a way to
/// hibernate. `"na"` means there is no swap or resume device to hibernate to.
pub fn can_hibernate() -> bool {
    static CAN: OnceLock<bool> = OnceLock::new();
    *CAN.get_or_init(|| {
        let answer = zbus::blocking::Connection::system()
            .and_then(|bus| Login1ManagerProxyBlocking::new(&bus)?.can_hibernate());
        match answer {
            Ok(answer) => {
                tracing::debug!(%answer, "logind CanHibernate");
                matches!(answer.as_str(), "yes" | "challenge")
            }
            Err(err) => {
                tracing::warn!(%err, "could not ask logind whether it can hibernate");
                false
            }
        }
    })
}

/// Why logind did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerError {
    /// polkit said no.
    Denied,
    /// logind could not be reached, or failed for a reason of its own.
    Failed(String),
}

impl std::fmt::Display for PowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => f.write_str("not allowed"),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for PowerError {}

/// Ask logind for `action` and wait for its answer.
///
/// logind answers a suspend or hibernate only once the machine is on its
/// way down, and polkit can take as long as it likes, so this belongs on a
/// thread of its own.
///
/// # Errors
///
/// [`PowerError::Denied`] when polkit refuses, [`PowerError::Failed`] when
/// logind cannot be reached or fails.
pub fn request(action: PowerAction) -> Result<(), PowerError> {
    let failed = |err: zbus::Error| {
        tracing::warn!(?action, %err, "logind unavailable");
        PowerError::Failed(err.to_string())
    };
    let connection = zbus::blocking::Connection::system().map_err(failed)?;
    let manager = Login1ManagerProxyBlocking::new(&connection).map_err(failed)?;
    tracing::info!(?action, "asking logind");
    let result = match action {
        PowerAction::Suspend => manager.suspend(true),
        PowerAction::Hibernate => manager.hibernate(true),
        PowerAction::Restart => manager.reboot(true),
        PowerAction::Shutdown => manager.power_off(true),
    };
    match result {
        Ok(()) => {
            tracing::info!(?action, "logind accepted");
            Ok(())
        }
        Err(zbus::Error::MethodError(name, message, _)) if is_refusal(name.as_str()) => {
            tracing::warn!(?action, %name, ?message, "logind refused");
            Err(PowerError::Denied)
        }
        Err(err) => Err(failed(err)),
    }
}

/// Lock this session.
///
/// logind passes the request on as `Lock` on the session, which Otto answers
/// by starting its locker, the same path `loginctl lock-session` takes.
///
/// # Errors
///
/// [`PowerError::Denied`] when polkit refuses, [`PowerError::Failed`] when
/// logind cannot be reached or fails.
pub fn lock_session() -> Result<(), PowerError> {
    let failed = |err: zbus::Error| {
        tracing::warn!(%err, "logind unavailable");
        PowerError::Failed(err.to_string())
    };
    let connection = zbus::blocking::Connection::system().map_err(failed)?;
    let manager = Login1ManagerProxyBlocking::new(&connection).map_err(failed)?;
    tracing::info!("asking logind to lock the session");
    match manager.lock_session("auto") {
        Ok(()) => Ok(()),
        Err(zbus::Error::MethodError(name, message, _)) if is_refusal(name.as_str()) => {
            tracing::warn!(%name, ?message, "logind refused to lock");
            Err(PowerError::Denied)
        }
        Err(err) => Err(failed(err)),
    }
}

/// The D-Bus errors polkit's refusals come back as.
fn is_refusal(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_swaps_to_the_other_sleep() {
        let sleep = Sleep::resolve(PowerAction::Suspend, true);
        assert_eq!(sleep.action(false), PowerAction::Suspend);
        assert_eq!(sleep.action(true), PowerAction::Hibernate);

        let sleep = Sleep::resolve(PowerAction::Hibernate, true);
        assert_eq!(sleep.action(false), PowerAction::Hibernate);
        assert_eq!(sleep.action(true), PowerAction::Suspend);
    }

    #[test]
    fn without_hibernate_sleep_is_always_a_suspend() {
        let sleep = Sleep::resolve(PowerAction::Hibernate, false);
        assert_eq!(sleep.action(false), PowerAction::Suspend);
        assert_eq!(sleep.action(true), PowerAction::Suspend);
        assert_eq!(sleep.alternative, None);
    }

    #[test]
    fn the_config_names_the_preferred_sleep() {
        let config = "[power_management]\nsleep = \"hibernate\"\n";
        assert_eq!(sleep_in_config(config), Some(PowerAction::Hibernate));
        assert_eq!(sleep_in_config("[power_management]\n"), None);
        assert_eq!(
            sleep_in_config("[power_management]\nsleep = \"nap\"\n"),
            None
        );
        assert_eq!(sleep_in_config("not toml ["), None);
    }
}
