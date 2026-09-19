//! An agent's `colour` reaches clients: the root state's `_meta` says, under
//! `otto.colours`, which frosted material each coloured agent wears.

use std::sync::Arc;

use ahp::Client;
use ahp_types::ROOT_RESOURCE_URI;
use ahp_types::state::SnapshotState;
use otto_agents::acp::AcpBackend;
use otto_agents::agent::EchoBackend;

mod common;
use otto_agents::config::{AgentConfig, Colour};
use serde_json::json;

async fn root_meta(client: &Client) -> Option<serde_json::Value> {
    let (result, _subscription) = client
        .subscribe(ROOT_RESOURCE_URI.into())
        .await
        .expect("subscribe");
    match result.snapshot.map(|s| s.state) {
        Some(SnapshotState::Root(root)) => root.meta.map(serde_json::Value::Object),
        _ => panic!("the root snapshot"),
    }
}

#[tokio::test]
async fn coloured_agents_are_listed_with_their_material() {
    let claude = AgentConfig {
        colour: Some(Colour::Orange),
        ..AgentConfig::claude()
    };
    let plain = AgentConfig {
        id: "plain".into(),
        name: "Plain".into(),
        colour: None,
        ..AgentConfig::claude()
    };
    let backend = AcpBackend::with_plugins(vec![claude, plain], Vec::new(), Vec::new());
    let client = common::serving(Arc::new(backend)).await.client;

    let meta = root_meta(&client)
        .await
        .expect("coloured agents put a _meta on the root");
    assert_eq!(
        meta["otto"]["colours"],
        json!({ "claude": "orange" }),
        "only the agents with a colour are listed"
    );
}

#[tokio::test]
async fn a_root_without_coloured_agents_has_no_meta() {
    let client = common::serving(Arc::new(EchoBackend)).await.client;
    assert_eq!(root_meta(&client).await, None);
}
