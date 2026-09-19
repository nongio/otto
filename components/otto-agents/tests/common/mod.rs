//! What every integration test here needs: a server on an ephemeral port and a
//! real `ahp` client talking to it.
//!
//! Each test file used to carry its own copy of the bind-connect-initialize
//! dance: five copies, five places to fix a flake, five chances for one of
//! them to drift. The per-file waits stay where they are — each applies its
//! own kind of state, so they are not the same code twice.

// Each test binary compiles this module on its own, so whatever a given file
// does not reach for looks unused from in here.
#![allow(dead_code)]

use std::sync::Arc;

use ahp::{Client, ClientConfig};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use otto_agents::Server;
use otto_agents::agent::Backend;
use otto_agents::dialog::Prompter;
use otto_agents::host::Host;

/// A running server and a client connected to it.
pub struct Harness {
    pub client: Client,
    pub host: Arc<Host>,
}

/// A server on an ephemeral port with `backend`, and a client that has
/// finished its handshake. The dialog is the real one, which is safe only
/// because nothing here escalates — pass a fake with [`with_prompter`] for
/// anything that does.
pub async fn serving(backend: Arc<dyn Backend>) -> Harness {
    let server = Server::bind("127.0.0.1:0", backend).await.expect("bind");
    finish(server).await
}

/// As [`serving`], with the prompter escalated questions go to.
pub async fn with_prompter(backend: Arc<dyn Backend>, prompter: Arc<dyn Prompter>) -> Harness {
    let server = Server::bind_with_prompter("127.0.0.1:0", backend, prompter)
        .await
        .expect("bind");
    finish(server).await
}

async fn finish(server: Server) -> Harness {
    // The wording the tests assert is the source catalogue's, not whatever
    // locale the machine running them happens to be in.
    otto_agents::i18n::pin_source_locale();
    let url = format!("ws://{}", server.local_addr().expect("local addr"));
    let host = server.host();
    tokio::spawn(server.run());
    let transport = ahp_ws::WebSocketTransport::connect(&url)
        .await
        .expect("connect");
    let client = Client::connect(transport, ClientConfig::default())
        .await
        .expect("client");
    client
        .initialize(
            "test-client".into(),
            vec![PROTOCOL_VERSION.into()],
            vec![ROOT_RESOURCE_URI.into()],
        )
        .await
        .expect("initialize");
    Harness { client, host }
}

/// A session URI nothing else will use.
pub fn new_session_uri() -> String {
    format!("ahp-session:/{}", uuid::Uuid::new_v4())
}
