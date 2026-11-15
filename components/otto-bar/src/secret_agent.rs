//! Otto's NetworkManager secret agent: the answer to "what is the password
//! for this Wi-Fi network?" when NetworkManager has none it can use.
//!
//! NetworkManager asks every registered agent of the session's user when a
//! connection it is activating lacks a secret: a saved network whose
//! password changed, or one that was saved without its password. This agent
//! answers Wi-Fi personal and WEP keys by putting up the bar's password
//! dialog ([`crate::join_dialog`]); everything else (802.1X, VPN plug-ins)
//! it declines at once, so another agent can answer. A password the person
//! types goes back to NetworkManager in the reply and nowhere else: the
//! agent stores nothing, and saving it is NetworkManager's choice.
//!
//! The interface is NetworkManager's `org.freedesktop.NetworkManager.SecretAgent`
//! (`man nm-dbus-api`), served on the system bus at
//! [`SECRET_AGENT_PATH`] and registered with the agent manager.

use std::sync::Mutex;

use futures_util::StreamExt;
use otto_kit::dbus::networkmanager::{AgentManagerProxy, SECRET_AGENT_PATH, SERVICE};
use otto_kit::AppContext;
use otto_network::{SecretRequest, Settings};
use zbus::zvariant::OwnedObjectPath;
use zbus::Connection;

/// How the agent is known to NetworkManager.
const IDENTIFIER: &str = "org.otto.bar";

/// `NMSecretAgentGetSecretsFlags`.
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;

/// A question waiting for the dialog.
pub struct Ask {
    /// The connection asked about, which a cancel names.
    pub path: String,
    pub request: SecretRequest,
    /// The password NetworkManager had was refused.
    pub retry: bool,
    pub reply: tokio::sync::oneshot::Sender<Option<String>>,
}

static ASKS: Mutex<Vec<Ask>> = Mutex::new(Vec::new());
static CANCELLED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The next question for the dialog, if one is waiting.
pub fn take_ask() -> Option<Ask> {
    let mut asks = ASKS.lock().unwrap();
    (!asks.is_empty()).then(|| asks.remove(0))
}

/// Whether NetworkManager withdrew its question about `path`, once.
pub fn take_cancelled(path: &str) -> bool {
    let mut cancelled = CANCELLED.lock().unwrap();
    match cancelled.iter().position(|p| p == path) {
        Some(index) => {
            cancelled.remove(index);
            true
        }
        None => false,
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
}

struct Agent;

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
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
        ASKS.lock().unwrap().push(Ask {
            path: connection_path.as_str().to_string(),
            request: request.clone(),
            retry: flags & REQUEST_NEW != 0,
            reply,
        });
        AppContext::request_wakeup();
        match answer.await {
            Ok(Some(password)) => Ok(request.reply(&password)),
            _ => Err(AgentError::UserCanceled(setting_name)),
        }
    }

    fn cancel_get_secrets(&self, connection_path: OwnedObjectPath, _setting_name: String) {
        let path = connection_path.as_str().to_string();
        // Still queued: answer it now. Already on screen: the bar takes it
        // down on its next pass.
        let mut asks = ASKS.lock().unwrap();
        asks.retain(|ask| ask.path != path);
        drop(asks);
        CANCELLED.lock().unwrap().push(path);
        AppContext::request_wakeup();
    }

    /// NetworkManager keeps the secrets it is given itself (the connections
    /// Otto makes are not agent-owned), so there is nothing to save.
    fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}

    fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
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

    #[tokio::test]
    async fn a_password_question_waits_for_the_dialog() {
        let agent = Agent;
        let path = OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/4").unwrap();
        let ask = agent.get_secrets(
            wifi_settings("wpa-psk"),
            path,
            "802-11-wireless-security".into(),
            Vec::new(),
            ALLOW_INTERACTION | REQUEST_NEW,
        );
        let answer = async {
            // The dialog's side: take the question, answer it.
            loop {
                if let Some(ask) = take_ask() {
                    assert!(ask.retry);
                    assert_eq!(ask.request.ssid, b"Home");
                    ask.reply.send(Some("hunter22".into())).unwrap();
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        let (reply, ()) = tokio::join!(ask, answer);
        let reply = reply.unwrap();
        assert!(reply["802-11-wireless-security"].contains_key("psk"));
    }

    #[tokio::test]
    async fn what_it_cannot_answer_is_declined_at_once() {
        let agent = Agent;
        let path = OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/5").unwrap();
        let enterprise = agent
            .get_secrets(
                wifi_settings("wpa-eap"),
                path.clone(),
                "802-11-wireless-security".into(),
                Vec::new(),
                ALLOW_INTERACTION,
            )
            .await;
        assert!(matches!(enterprise, Err(AgentError::NoSecrets(_))));
        // Nobody to ask during a background connect.
        let quiet = agent
            .get_secrets(
                wifi_settings("wpa-psk"),
                path,
                "802-11-wireless-security".into(),
                Vec::new(),
                0,
            )
            .await;
        assert!(matches!(quiet, Err(AgentError::NoSecrets(_))));
    }
}
