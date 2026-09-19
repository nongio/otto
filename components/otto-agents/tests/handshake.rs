//! End-to-end tests: a real server on an ephemeral port, driven over WebSocket
//! by the official AHP Rust client (`ahp` + `ahp-ws`), an independent
//! implementation of the other side of the protocol.

use ahp::{Client, ClientConfig, ClientError};
use ahp_types::errors::{ahp_error_codes, json_rpc_error_codes};
use ahp_types::state::SnapshotState;
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use otto_agents::Server;
use otto_agents::agent::EchoBackend;
use otto_agents::dialog::Islands;
use otto_agents::server::Listen;
use serde_json::{Value, json};

async fn connect() -> Client {
    let server = Server::bind("127.0.0.1:0", Arc::new(EchoBackend))
        .await
        .expect("bind");
    let url = format!("ws://{}", server.local_addr().expect("local addr"));
    tokio::spawn(server.run());
    let transport = ahp_ws::WebSocketTransport::connect(&url)
        .await
        .expect("connect");
    Client::connect(transport, ClientConfig::default())
        .await
        .expect("client")
}

async fn initialize(client: &Client) {
    client
        .initialize("test-client".into(), vec![PROTOCOL_VERSION.into()], vec![])
        .await
        .expect("initialize");
}

fn rpc_error(err: ClientError) -> ahp_types::JsonRpcError {
    match err {
        ClientError::Rpc(err) => err,
        other => panic!("expected a JSON-RPC error, got: {other}"),
    }
}

#[tokio::test]
async fn initialize_negotiates_version_and_snapshots_root() {
    let client = connect().await;
    let init = client
        .initialize(
            "test-client".into(),
            vec!["99.0.0".into(), PROTOCOL_VERSION.into()],
            vec![ROOT_RESOURCE_URI.into()],
        )
        .await
        .expect("initialize");

    assert_eq!(init.protocol_version, PROTOCOL_VERSION);
    let [snapshot] = init.snapshots.as_slice() else {
        panic!("expected exactly one snapshot, got {:?}", init.snapshots);
    };
    assert_eq!(snapshot.resource, ROOT_RESOURCE_URI);
    assert!(
        matches!(snapshot.state, SnapshotState::Root(_)),
        "{:?}",
        snapshot.state
    );
}

#[tokio::test]
async fn ping_is_answered_before_initialize() {
    connect().await.ping().await.expect("ping");
}

#[tokio::test]
async fn unsupported_protocol_version_is_rejected() {
    let client = connect().await;
    let err = client
        .initialize("test-client".into(), vec!["0.0.1".into()], vec![])
        .await
        .unwrap_err();

    let err = rpc_error(err);
    assert_eq!(err.code, ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION);
    assert_eq!(
        err.data
            .and_then(|data| data.get("supportedVersions").cloned()),
        Some(json!([PROTOCOL_VERSION]))
    );
}

#[tokio::test]
async fn requests_before_initialize_are_rejected() {
    let client = connect().await;
    let err = client
        .request::<_, Value>("listSessions", json!({ "channel": ROOT_RESOURCE_URI }))
        .await
        .unwrap_err();
    assert_eq!(rpc_error(err).code, json_rpc_error_codes::INVALID_REQUEST);
}

#[tokio::test]
async fn subscribe_to_root_returns_a_snapshot() {
    let client = connect().await;
    initialize(&client).await;

    let (result, _subscription) = client
        .subscribe(ROOT_RESOURCE_URI.into())
        .await
        .expect("subscribe");
    assert!(matches!(
        result.snapshot.map(|s| s.state),
        Some(SnapshotState::Root(_))
    ));
}

#[tokio::test]
async fn unknown_methods_are_reported() {
    let client = connect().await;
    initialize(&client).await;

    let err = client
        .request::<_, Value>(
            "x-otto/doesNotExist",
            json!({ "channel": ROOT_RESOURCE_URI }),
        )
        .await
        .unwrap_err();
    assert_eq!(rpc_error(err).code, json_rpc_error_codes::METHOD_NOT_FOUND);
}

/// The socket the service listens on by default is the user's alone, and a
/// client reaches it through a `unix://` URL.
#[tokio::test]
async fn a_unix_socket_is_private_and_answers_pings() {
    let temp = tempfile::tempdir().expect("a temp dir");
    let path = temp.path().join("run").join("agents.sock");
    let listen = Listen::Unix(path.clone());
    let server = Server::listen(
        &listen,
        Arc::new(EchoBackend),
        Arc::new(Islands::default()),
        None,
    )
    .await
    .expect("bind the socket");
    assert_eq!(server.endpoint().expect("endpoint"), listen);
    let mode = |path: &std::path::Path| {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    tokio::spawn(server.run());

    let transport = otto_agents::client::connect(&listen.url())
        .await
        .expect("connect over the socket");
    let client = Client::connect(transport, ClientConfig::default())
        .await
        .expect("client");
    client.ping().await.expect("ping");
    initialize(&client).await;
    client.shutdown().await;
}
