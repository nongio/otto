//! Otto's NetworkManager secret agent: the answer to "what is the password
//! for this Wi-Fi network?" when NetworkManager has none it can use.
//!
//! NetworkManager asks every registered agent of the session's user when a
//! connection it is activating lacks a secret: a saved network whose
//! password changed, or one that was saved without its password. This agent
//! answers Wi-Fi personal and WEP keys by putting up the bar's password
//! dialog ([`crate::join_dialog`]); everything else (802.1X, VPN plug-ins)
//! it declines at once, so another agent can answer. A password the person
//! types goes back to NetworkManager and nowhere else: the agent keeps no
//! store of its own.
//!
//! A profile whose key is agent-owned (`psk-flags` or `wep-key-flags` with
//! `AGENT_OWNED`, as a keyring-backed applet saves them) expects the agent
//! to store the key, and is stuck once that agent is gone: every
//! auto-connect finds no key, and NetworkManager never offers the answer
//! back through `SaveSecrets`. So when the person answers for such a
//! profile, the agent also updates the profile to keep the key itself
//! (the `AGENT_OWNED` flag cleared, the key included), and later connects
//! need no agent at all.
//!
//! The interface is NetworkManager's `org.freedesktop.NetworkManager.SecretAgent`
//! (`man nm-dbus-api`), served on the system bus at
//! [`SECRET_AGENT_PATH`] and registered with the agent manager.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use otto_kit::dbus::networkmanager::{
    AgentManagerProxy, SettingsConnectionProxy, SECRET_AGENT_PATH, SERVICE,
};
use otto_kit::AppContext;
use otto_network::{SecretRequest, Settings};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::Connection;

/// How the agent is known to NetworkManager.
const IDENTIFIER: &str = "org.otto.bar";

/// `NMSecretAgentGetSecretsFlags`.
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;

/// `NMSettingSecretFlags`: a user-session agent stores the secret.
const AGENT_OWNED: u32 = 0x1;
/// `NMSettingSecretFlags`: the secret is asked for every time, never kept.
const NOT_SAVED: u32 = 0x2;

/// The setting that holds Wi-Fi keys.
const WIFI_SECURITY: &str = "802-11-wireless-security";

/// A question waiting for the dialog.
pub struct Ask {
    pub request: SecretRequest,
    /// The password NetworkManager had was refused.
    pub retry: bool,
    pub reply: tokio::sync::oneshot::Sender<Option<String>>,
    /// Set when NetworkManager withdraws this question. It belongs to this
    /// one request and goes with it, so a withdrawal can never close a
    /// dialog asked for later.
    pub cancelled: Arc<AtomicBool>,
}

/// Questions waiting for the dialog: one is on screen at a time, and the
/// rest wait their turn rather than being refused, since NetworkManager
/// asks about one connection at a time and a second question is rare.
static ASKS: Mutex<Vec<(String, Ask)>> = Mutex::new(Vec::new());
/// Every question not yet answered, queued or on screen, by connection path.
static OPEN: Mutex<Vec<(String, Arc<AtomicBool>)>> = Mutex::new(Vec::new());

/// The next question for the dialog, if one is waiting.
pub fn take_ask() -> Option<Ask> {
    let mut asks = ASKS.lock().unwrap();
    (!asks.is_empty()).then(|| asks.remove(0).1)
}

/// Withdraw every question about `path`: a queued one is answered no at
/// once (its reply is dropped), and one on screen is marked for the bar to
/// take down.
fn withdraw(path: &str) {
    ASKS.lock().unwrap().retain(|(p, _)| p != path);
    for (p, cancelled) in OPEN.lock().unwrap().iter() {
        if p == path {
            cancelled.store(true, Ordering::Relaxed);
        }
    }
    AppContext::request_wakeup();
}

/// Ask the person, through the dialog, for the secret `setting_name` of
/// `connection`, and wait for the answer.
async fn ask(
    connection: Settings,
    path: &str,
    setting_name: String,
    flags: u32,
) -> Result<Settings, AgentError> {
    let Some(request) = SecretRequest::from_settings(&setting_name, &connection) else {
        return Err(AgentError::NoSecrets(setting_name));
    };
    if flags & ALLOW_INTERACTION == 0 {
        // Asked during a background auto-connect: nobody to ask.
        return Err(AgentError::NoSecrets(setting_name));
    }
    let (reply, answer) = tokio::sync::oneshot::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    OPEN.lock()
        .unwrap()
        .push((path.to_string(), cancelled.clone()));
    ASKS.lock().unwrap().push((
        path.to_string(),
        Ask {
            request: request.clone(),
            retry: flags & REQUEST_NEW != 0,
            reply,
            cancelled: cancelled.clone(),
        },
    ));
    AppContext::request_wakeup();
    let answer = answer.await;
    OPEN.lock()
        .unwrap()
        .retain(|(_, flag)| !Arc::ptr_eq(flag, &cancelled));
    match answer {
        Ok(Some(password)) => Ok(request.reply(&password)),
        _ => Err(AgentError::UserCanceled(setting_name)),
    }
}

/// The errors NetworkManager understands from an agent.
#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// Not a secret this agent answers: ask another.
    NoSecrets(String),
    /// The person said no.
    UserCanceled(String),
    /// The caller is not NetworkManager.
    PermissionDenied(String),
}

/// Whether `header` is a message from NetworkManager itself: the system bus
/// lets any peer call the agent's path, and only NetworkManager may be told
/// a password.
async fn from_networkmanager(conn: &Connection, header: &zbus::message::Header<'_>) -> bool {
    let Some(sender) = header.sender() else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(SERVICE) else {
        return false;
    };
    dbus.get_name_owner(name)
        .await
        .is_ok_and(|owner| owner.as_str() == sender.as_str())
}

struct Agent;

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
    ) -> Result<Settings, AgentError> {
        if !from_networkmanager(conn, &header).await {
            return Err(AgentError::PermissionDenied(setting_name));
        }
        let path = connection_path.to_string();
        let secrets = ask(connection.clone(), &path, setting_name, flags).await?;
        // Decided on what NetworkManager sent, so a profile that keeps its
        // own key costs no extra round trip.
        if store_with_profile(connection, &secrets).is_some() {
            tokio::spawn(keep_with_networkmanager(
                conn.clone(),
                path,
                secrets.clone(),
            ));
        }
        Ok(secrets)
    }

    async fn cancel_get_secrets(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connection_path: OwnedObjectPath,
        _setting_name: String,
    ) {
        if from_networkmanager(conn, &header).await {
            withdraw(connection_path.as_str());
        }
    }

    /// Accepted and dropped: the agent keeps no store. Keys it answers end
    /// up stored by NetworkManager (see [`store_with_profile`]), so there is
    /// nothing of its own to save.
    fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}

    /// Accepted with nothing to do, for the same reason as `save_secrets`.
    fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
}

/// The flag property governing the key in `secrets`, the reply to a Wi-Fi
/// key request.
fn flags_key(secrets: &Settings) -> Option<&'static str> {
    let security = secrets.get(WIFI_SECURITY)?;
    if security.contains_key("psk") {
        Some("psk-flags")
    } else if security.contains_key("wep-key0") {
        Some("wep-key-flags")
    } else {
        None
    }
}

/// `settings` changed to keep the key in `secrets` with NetworkManager, when
/// the key is agent-owned and meant to be saved; `None` otherwise.
///
/// The `AGENT_OWNED` flag is cleared and every other flag kept. Only the
/// reply's own keys (`psk`, `wep-key0`, `wep-key-type`) are added, since
/// the reply carries nothing else.
fn store_with_profile(mut settings: Settings, secrets: &Settings) -> Option<Settings> {
    let flags_key = flags_key(secrets)?;
    let security = settings.get_mut(WIFI_SECURITY)?;
    let flags = security
        .get(flags_key)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    if flags & AGENT_OWNED == 0 || flags & NOT_SAVED != 0 {
        return None;
    }
    let cleared = OwnedValue::from(flags & !AGENT_OWNED);
    security.insert(flags_key.to_string(), cleared);
    for (key, value) in secrets.get(WIFI_SECURITY)? {
        security.insert(key.clone(), value.clone());
    }
    Some(settings)
}

/// Have NetworkManager keep the key in `secrets` with the profile at
/// `path`, when the profile leaves it to an agent.
///
/// Spawned beside the reply, which does not wait on it. The profile is read
/// fresh and updated, so its other settings are kept as NetworkManager has
/// them. A refusal (no
/// permission to change the profile) leaves things as they were, and the
/// next connect asks again. The key is never logged.
async fn keep_with_networkmanager(conn: Connection, path: String, secrets: Settings) {
    let result = async {
        let proxy = SettingsConnectionProxy::builder(&conn)
            .path(path.as_str())?
            .build()
            .await?;
        let settings = proxy.get_settings().await?;
        match store_with_profile(settings, &secrets) {
            Some(updated) => proxy.update(updated).await,
            None => Ok(()),
        }
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("network key left with the agent: {e}");
    }
}

/// Serve the agent on `conn` and register it, again whenever NetworkManager
/// comes back on the bus. Runs until the connection closes.
pub async fn run(conn: Connection) -> zbus::Result<()> {
    conn.object_server().at(SECRET_AGENT_PATH, Agent).await?;
    let manager = AgentManagerProxy::new(&conn).await?;
    let mut owners = zbus::fdo::DBusProxy::new(&conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, SERVICE)])
        .await?;
    register(&manager).await;
    while let Some(change) = owners.next().await {
        let arrived = change
            .args()
            .is_ok_and(|args| args.new_owner().as_ref().is_some());
        if arrived {
            register(&manager).await;
        }
    }
    Ok(())
}

async fn register(manager: &AgentManagerProxy<'_>) {
    match manager.register_with_capabilities(IDENTIFIER, 0).await {
        Ok(()) => tracing::debug!("network secret agent registered"),
        // NetworkManager not running yet: the owner watch tries again.
        Err(e) => tracing::debug!("network secret agent not registered: {e}"),
    }
}

/// What the agent was sent, for a test without NetworkManager.
#[cfg(test)]
fn wifi_settings(key_mgmt: &str) -> Settings {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};
    let owned = |v: Value<'_>| OwnedValue::try_from(v).unwrap();
    HashMap::from([
        (
            "802-11-wireless".to_string(),
            HashMap::from([("ssid".to_string(), owned(Value::from(b"Home".to_vec())))]),
        ),
        (
            "802-11-wireless-security".to_string(),
            HashMap::from([("key-mgmt".to_string(), owned(Value::from(key_mgmt)))]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The queued question about `path`, taken off the queue. Tests share
    /// the queue and run side by side, so each takes only its own.
    fn take_ask_for(path: &str) -> Option<Ask> {
        let mut asks = ASKS.lock().unwrap();
        let index = asks.iter().position(|(p, _)| p == path)?;
        Some(asks.remove(index).1)
    }

    #[tokio::test]
    async fn a_password_question_waits_for_the_dialog() {
        let asked = ask(
            wifi_settings("wpa-psk"),
            "/org/freedesktop/NetworkManager/Settings/4",
            "802-11-wireless-security".into(),
            ALLOW_INTERACTION | REQUEST_NEW,
        );
        let answer = async {
            // The dialog's side: take the question, answer it.
            loop {
                if let Some(ask) = take_ask_for("/org/freedesktop/NetworkManager/Settings/4") {
                    assert!(ask.retry);
                    assert_eq!(ask.request.ssid, b"Home");
                    ask.reply.send(Some("hunter22".into())).unwrap();
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        let (reply, ()) = tokio::join!(asked, answer);
        let reply = reply.unwrap();
        assert!(reply["802-11-wireless-security"].contains_key("psk"));
    }

    #[tokio::test]
    async fn a_withdrawal_is_for_its_own_request_only() {
        let path = "/org/freedesktop/NetworkManager/Settings/6";
        // Queued, then withdrawn: answered no, and nothing left behind.
        let asked = ask(
            wifi_settings("wpa-psk"),
            path,
            "802-11-wireless-security".into(),
            ALLOW_INTERACTION,
        );
        let withdraw_it = async {
            tokio::task::yield_now().await;
            withdraw(path);
        };
        let (answer, ()) = tokio::join!(asked, withdraw_it);
        assert!(matches!(answer, Err(AgentError::UserCanceled(_))));
        assert!(OPEN.lock().unwrap().iter().all(|(p, _)| p != path));

        // The next question about the same connection is not withdrawn.
        let asked = ask(
            wifi_settings("wpa-psk"),
            path,
            "802-11-wireless-security".into(),
            ALLOW_INTERACTION,
        );
        let answer_it = async {
            loop {
                if let Some(ask) = take_ask_for(path) {
                    assert!(!ask.cancelled.load(Ordering::Relaxed));
                    ask.reply.send(Some("hunter22".into())).unwrap();
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        let (answer, ()) = tokio::join!(asked, answer_it);
        assert!(answer.is_ok());
    }

    #[tokio::test]
    async fn what_it_cannot_answer_is_declined_at_once() {
        let path = "/org/freedesktop/NetworkManager/Settings/5";
        let enterprise = ask(
            wifi_settings("wpa-eap"),
            path,
            "802-11-wireless-security".into(),
            ALLOW_INTERACTION,
        )
        .await;
        assert!(matches!(enterprise, Err(AgentError::NoSecrets(_))));
        // Nobody to ask during a background connect.
        let quiet = ask(
            wifi_settings("wpa-psk"),
            path,
            "802-11-wireless-security".into(),
            0,
        )
        .await;
        assert!(matches!(quiet, Err(AgentError::NoSecrets(_))));
    }

    /// `settings` with `flags` set on the security setting's `key`.
    fn with_flags(mut settings: Settings, key: &str, flags: u32) -> Settings {
        settings
            .get_mut(WIFI_SECURITY)
            .unwrap()
            .insert(key.to_string(), OwnedValue::from(flags));
        settings
    }

    /// The reply the dialog sends for `settings`.
    fn answer(settings: &Settings, password: &str) -> Settings {
        SecretRequest::from_settings(WIFI_SECURITY, settings)
            .unwrap()
            .reply(password)
    }

    fn flags(settings: &Settings, key: &str) -> u32 {
        u32::try_from(&settings[WIFI_SECURITY][key]).unwrap()
    }

    #[test]
    fn an_agent_owned_key_moves_to_networkmanager() {
        let profile = with_flags(wifi_settings("wpa-psk"), "psk-flags", AGENT_OWNED);
        let secrets = answer(&profile, "hunter22");
        let updated = store_with_profile(profile, &secrets).unwrap();
        assert_eq!(flags(&updated, "psk-flags"), 0);
        assert_eq!(
            String::try_from(updated[WIFI_SECURITY]["psk"].clone()).unwrap(),
            "hunter22"
        );
        // The rest of the profile is untouched.
        assert!(updated["802-11-wireless"].contains_key("ssid"));
        assert_eq!(
            String::try_from(updated[WIFI_SECURITY]["key-mgmt"].clone()).unwrap(),
            "wpa-psk"
        );
    }

    #[test]
    fn a_wep_key_moves_with_its_type() {
        let profile = with_flags(wifi_settings("none"), "wep-key-flags", AGENT_OWNED);
        let secrets = answer(&profile, "abcde");
        let updated = store_with_profile(profile, &secrets).unwrap();
        assert_eq!(flags(&updated, "wep-key-flags"), 0);
        assert!(updated[WIFI_SECURITY].contains_key("wep-key0"));
        assert!(updated[WIFI_SECURITY].contains_key("wep-key-type"));
    }

    #[test]
    fn only_the_agent_owned_flag_is_cleared() {
        const NOT_REQUIRED: u32 = 0x4;
        let profile = with_flags(
            wifi_settings("sae"),
            "psk-flags",
            AGENT_OWNED | NOT_REQUIRED,
        );
        let secrets = answer(&profile, "correct horse");
        let updated = store_with_profile(profile, &secrets).unwrap();
        assert_eq!(flags(&updated, "psk-flags"), NOT_REQUIRED);
    }

    #[test]
    fn a_key_kept_elsewhere_or_never_saved_stays_put() {
        // NetworkManager already keeps it: nothing to change.
        let system = wifi_settings("wpa-psk");
        let secrets = answer(&system, "hunter22");
        assert!(store_with_profile(system, &secrets).is_none());
        // Asked for every time: the person chose that.
        let never = with_flags(
            wifi_settings("wpa-psk"),
            "psk-flags",
            AGENT_OWNED | NOT_SAVED,
        );
        let secrets = answer(&never, "hunter22");
        assert!(store_with_profile(never, &secrets).is_none());
    }
}
