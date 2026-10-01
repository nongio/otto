//! The agent's side of the system bus: registering with polkitd for this
//! session, and serving `org.freedesktop.PolicyKit1.AuthenticationAgent`.
//!
//! Runs on a tokio runtime of its own thread. Requests are handed to the
//! dialog loop over a channel ([`Request`]), and each `BeginAuthentication`
//! call is held open until the dialog answers it — that reply is what tells
//! polkitd the agent is done, not whether anyone authenticated: that the
//! helper reports to polkitd directly ([`super::helper`]).

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use futures_util::StreamExt;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

/// polkitd's well-known name.
const POLKIT_NAME: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";
/// Where this agent's object lives on its own connection.
pub const OBJECT_PATH: &str = "/org/otto/PolicyKit1/AuthenticationAgent";

/// What polkitd hears back from `BeginAuthentication`.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// The request could not be completed.
    Failed(String),
    /// The user dismissed it, or it was never shown.
    Cancelled(String),
}

/// Who polkit will accept a password from, as `BeginAuthentication` lists
/// them. polkitd expands groups into their users before it asks, so a user
/// is all an agent needs to handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    User(u32),
    Other,
}

/// One `BeginAuthentication`, as the dialog loop needs it.
pub struct Begin {
    pub action: String,
    pub message: String,
    pub details: HashMap<String, String>,
    pub cookie: String,
    pub identities: Vec<Identity>,
    pub reply: tokio::sync::oneshot::Sender<Result<(), AgentError>>,
}

pub enum Request {
    Begin(Begin),
    /// polkitd withdrew the request with this cookie.
    Cancel {
        cookie: String,
    },
    /// The agent cannot serve this session; the process should end, with
    /// this exit status.
    Stop(i32),
}

struct Agent {
    requests: Sender<Request>,
    connection: Connection,
}

impl Agent {
    fn send(&self, request: Request) -> bool {
        let sent = self.requests.send(request).is_ok();
        otto_kit::AppContext::request_wakeup();
        sent
    }

    /// Whether the call came from polkitd itself. Anyone on the system bus
    /// can call this object; only polkitd's requests have a cookie the
    /// helper will accept, but a forged one would still put a panel asking
    /// for a password on screen.
    async fn sent_by_polkitd(&self, header: &zbus::message::Header<'_>) -> bool {
        let Some(sender) = header.sender() else {
            return false;
        };
        let Ok(proxy) = zbus::fdo::DBusProxy::new(&self.connection).await else {
            return false;
        };
        let Ok(name) = zbus::names::BusName::try_from(POLKIT_NAME) else {
            return false;
        };
        match proxy.get_name_owner(name).await {
            Ok(owner) => owner.as_str() == sender.as_str(),
            Err(_) => false,
        }
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        action_id: String,
        message: String,
        _icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        if !self.sent_by_polkitd(&header).await {
            tracing::warn!(sender = ?header.sender(), "BeginAuthentication not from polkitd; refused");
            return Err(AgentError::Failed("only polkitd may ask".into()));
        }
        tracing::info!(
            action = %action_id,
            subject_pid = details.get("polkit.subject-pid").map(String::as_str).unwrap_or("?"),
            caller_pid = details.get("polkit.caller-pid").map(String::as_str).unwrap_or("?"),
            identities = identities.len(),
            "BeginAuthentication from polkitd"
        );
        let (reply, answer) = tokio::sync::oneshot::channel();
        let begin = Begin {
            action: action_id.clone(),
            message,
            details,
            cookie,
            identities: identities.iter().map(identity).collect(),
            reply,
        };
        if !self.send(Request::Begin(begin)) {
            return Err(AgentError::Failed("the agent is stopping".into()));
        }
        // A dialog loop that drops the sender without answering has shown
        // nothing that could be confirmed.
        let result = answer
            .await
            .unwrap_or_else(|_| Err(AgentError::Cancelled("the agent is stopping".into())));
        match &result {
            Ok(()) => tracing::info!(action = %action_id, "BeginAuthentication answered: done"),
            Err(err) => {
                tracing::info!(action = %action_id, error = %err, "BeginAuthentication answered with an error")
            }
        }
        result
    }

    async fn cancel_authentication(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        cookie: String,
    ) -> Result<(), AgentError> {
        if !self.sent_by_polkitd(&header).await {
            return Err(AgentError::Failed("only polkitd may cancel".into()));
        }
        tracing::info!("CancelAuthentication from polkitd");
        self.send(Request::Cancel { cookie });
        Ok(())
    }
}

/// An identity from the `a(sa{sv})` polkitd sends.
fn identity((kind, fields): &(String, HashMap<String, OwnedValue>)) -> Identity {
    if kind != "unix-user" {
        return Identity::Other;
    }
    fields
        .get("uid")
        .and_then(|value| u32::try_from(value).ok())
        .map(Identity::User)
        .unwrap_or(Identity::Other)
}

/// The logind session this agent registers for: the one the compositor
/// runs in, from `$XDG_SESSION_ID` or logind itself.
async fn subject(
    connection: &Connection,
) -> zbus::Result<(String, HashMap<String, Value<'static>>)> {
    let id = session_id(connection).await.ok_or_else(|| {
        zbus::Error::Failure("this process is not in a logind session".to_string())
    })?;
    tracing::info!(session = %id, "registering for the session");
    let mut fields = HashMap::new();
    fields.insert("session-id".to_string(), Value::from(id));
    Ok(("unix-session".to_string(), fields))
}

async fn session_id(connection: &Connection) -> Option<String> {
    if let Some(id) = std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
    {
        return Some(id);
    }
    let reply = connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetSessionByPID",
            &(std::process::id()),
        )
        .await
        .ok()?;
    let path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize().ok()?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.login1"),
            path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.Session", "Id"),
        )
        .await
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    String::try_from(value).ok()
}

/// The locale polkitd should write its messages in.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "en_US.UTF-8".to_string())
}

/// Register this connection's agent object for the session.
async fn register(connection: &Connection) -> zbus::Result<()> {
    let subject = subject(connection).await?;
    connection
        .call_method(
            Some(POLKIT_NAME),
            AUTHORITY_PATH,
            Some(AUTHORITY_IFACE),
            "RegisterAuthenticationAgent",
            &(subject, locale(), OBJECT_PATH),
        )
        .await?;
    Ok(())
}

/// Serve the agent until the process ends. Runs on a thread of its own.
pub fn spawn(requests: Sender<Request>) {
    let spawned = std::thread::Builder::new()
        .name("polkit-dbus".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::error!(%err, "no runtime for the polkit agent");
                    let _ = requests.send(Request::Stop(1));
                    otto_kit::AppContext::request_wakeup();
                    return;
                }
            };
            runtime.block_on(serve(requests));
        });
    if let Err(err) = spawned {
        tracing::error!(%err, "cannot start the polkit agent's bus thread");
    }
}

async fn serve(requests: Sender<Request>) {
    let stop = |status: i32| {
        let _ = requests.send(Request::Stop(status));
        otto_kit::AppContext::request_wakeup();
    };

    let connection = match Connection::system().await {
        Ok(connection) => connection,
        Err(err) => {
            tracing::error!(%err, "no system bus; the polkit agent cannot run");
            stop(1);
            return;
        }
    };
    let agent = Agent {
        requests: requests.clone(),
        connection: connection.clone(),
    };
    if let Err(err) = connection.object_server().at(OBJECT_PATH, agent).await {
        tracing::error!(%err, "cannot serve the agent object");
        stop(1);
        return;
    }

    // Refused — most often because another agent already serves this
    // session (a nested Otto inside a desktop that has one, or an agent the
    // user starts themselves). Exit cleanly: starting again cannot help.
    if let Err(err) = register(&connection).await {
        tracing::warn!(%err, "polkit did not accept this agent; leaving it to the one it has");
        stop(0);
        return;
    }
    tracing::info!("registered as the session's polkit authentication agent");

    // polkitd forgets its agents when it restarts; register again when it
    // comes back.
    let Ok(proxy) = zbus::fdo::DBusProxy::new(&connection).await else {
        std::future::pending::<()>().await;
        return;
    };
    let Ok(mut changes) = proxy
        .receive_name_owner_changed_with_args(&[(0, POLKIT_NAME)])
        .await
    else {
        std::future::pending::<()>().await;
        return;
    };
    while let Some(change) = changes.next().await {
        let Ok(args) = change.args() else { continue };
        if args.new_owner().is_some() {
            tracing::info!("polkitd restarted; registering again");
            if let Err(err) = register(&connection).await {
                tracing::warn!(%err, "polkit did not accept this agent again");
                stop(0);
                return;
            }
        }
    }
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_users_with_a_uid_are_identities() {
        let user = (
            "unix-user".to_string(),
            HashMap::from([("uid".to_string(), OwnedValue::from(1000u32))]),
        );
        assert_eq!(identity(&user), Identity::User(1000));
        let group = (
            "unix-group".to_string(),
            HashMap::from([("gid".to_string(), OwnedValue::from(998u32))]),
        );
        assert_eq!(identity(&group), Identity::Other);
        assert_eq!(
            identity(&("unix-user".to_string(), HashMap::new())),
            Identity::Other
        );
    }
}
