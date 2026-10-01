//! Whether a program may have an agent seat.
//!
//! A program asking for a seat is named by its process (see
//! [`otto_kit::process_app`]), not by the agent name it gives. The first time
//! it asks, the user is asked through otto-islands' dialog, which no client
//! can draw or answer, and the answer is kept in xdg-permission-store's
//! `otto-agents` table: later requests are let through or refused without
//! asking. Settings › Privacy lists the table, switches a program off, and
//! forgets its answer; a program switched off loses the seats it holds.

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

const STORE_NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const STORE_INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";

/// The table the answers are kept in, and its one entry.
pub const TABLE: &str = "otto-agents";
pub const ID: &str = "seat";

/// What the user said, or that nobody could ask them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Allowed,
    Denied,
    /// No dialog could be shown, or it was withdrawn: nothing is recorded,
    /// and the request is refused.
    Unanswered,
}

/// The program on the bus connection `owner`.
pub async fn program_of(connection: &zbus::Connection, owner: &str) -> Option<String> {
    let owner = zbus::names::BusName::try_from(owner.to_string()).ok()?;
    let pid = zbus::fdo::DBusProxy::new(connection)
        .await
        .ok()?
        .get_connection_unix_process_id(owner)
        .await
        .ok()?;
    otto_kit::process_app::app_id_for_pid(pid)
}

/// Every program's answer: `true` allowed, `false` refused. `None` when the
/// store cannot be read.
pub async fn answers(connection: &zbus::Connection) -> Option<HashMap<String, bool>> {
    let reply = connection
        .call_method(
            Some(STORE_NAME),
            STORE_PATH,
            Some(STORE_INTERFACE),
            "Lookup",
            &(TABLE, ID),
        )
        .await;
    let apps: HashMap<String, Vec<String>> = match reply {
        Ok(reply) => {
            reply
                .body()
                .deserialize::<(HashMap<String, Vec<String>>, OwnedValue)>()
                .ok()?
                .0
        }
        // No entry yet: nobody has answered.
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with(".NotFound") => {
            HashMap::new()
        }
        Err(err) => {
            tracing::warn!(%err, "cannot read the agents' answers");
            return None;
        }
    };
    Some(
        apps.into_iter()
            .map(|(app, permissions)| {
                let allowed = permissions.iter().any(|p| p == "yes");
                (app, allowed)
            })
            .collect(),
    )
}

/// Keep `program`'s answer.
pub async fn record(connection: &zbus::Connection, program: &str, allowed: bool) {
    let permission = if allowed { "yes" } else { "no" };
    let recorded = connection
        .call_method(
            Some(STORE_NAME),
            STORE_PATH,
            Some(STORE_INTERFACE),
            "SetPermission",
            &(TABLE, true, ID, program, vec![permission]),
        )
        .await;
    if let Err(err) = recorded {
        tracing::warn!(%err, program, "cannot keep the answer about agent seats");
    }
}

/// Ask the user whether `program` may have a seat for the agent it calls
/// `agent_name`. Waits for as long as the user takes.
pub async fn ask(connection: &zbus::Connection, program: &str, agent_name: &str) -> Answer {
    let app = otto_kit::desktop_entry::display_name_for_app(program);
    let title = otto_kit::t_owned!("agent-prompt-title", app = app.clone());
    let body = otto_kit::t_owned!(
        "agent-prompt-body",
        app = app,
        agent = agent_name.to_string()
    );
    let reply = connection
        .call_method(
            Some("org.otto.Island"),
            "/org/otto/Dialog",
            Some("org.otto.Dialog1"),
            "PresentAccess",
            &(
                program,
                title.as_str(),
                "",
                body.as_str(),
                "input-mouse",
                otto_kit::t!("agent-prompt-allow"),
                otto_kit::t!("agent-prompt-deny"),
                true,
                Vec::<(String, String, Vec<(String, String, String)>, String)>::new(),
            ),
        )
        .await;
    // The answer counts only from Otto's own islands: anyone can own the bus
    // name once islands is gone, and answer every question with yes.
    let reply = match reply {
        Ok(reply) => reply,
        Err(err) => {
            tracing::warn!(%err, program, "cannot ask about an agent seat");
            return Answer::Unanswered;
        }
    };
    let answered_by = match reply.header().sender().map(|s| s.to_string()) {
        Some(sender) => otto_kit::trust::bus_peer(connection, &sender).await,
        None => None,
    };
    if !answered_by
        .as_ref()
        .is_some_and(|peer| otto_kit::trust::is_component(&peer.exe, &["otto-islands"]))
    {
        tracing::warn!(
            ?answered_by,
            program,
            "the answer did not come from Otto's islands"
        );
        return Answer::Unanswered;
    }
    match reply.body().deserialize::<(u32, Vec<(String, String)>)>() {
        Ok((0, _)) => Answer::Allowed,
        Ok((1, _)) => Answer::Denied,
        Ok(_) => Answer::Unanswered,
        Err(err) => {
            tracing::warn!(%err, program, "cannot read the answer about an agent seat");
            Answer::Unanswered
        }
    }
}
