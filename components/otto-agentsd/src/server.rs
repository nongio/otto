//! WebSocket transport: each text frame carries exactly one JSON-RPC message.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message};
use tracing::{debug, warn};

use crate::agent::Backend;
use crate::dialog::{Islands, Prompter};
use crate::host::{Connection, Host, Outgoing};
use crate::rpc::{self, Incoming, RpcError};
use crate::store::Store;

/// How often changed sessions are written to the store. At most this much is
/// lost if the service dies without saving on the way out.
pub const SAVE_PERIOD: Duration = Duration::from_secs(1);

pub struct Server {
    listener: TcpListener,
    host: Arc<Host>,
}

impl Server {
    /// Binds a server whose unwatched permission questions go to otto-islands.
    pub async fn bind(
        addr: impl ToSocketAddrs,
        backend: Arc<dyn Backend>,
    ) -> std::io::Result<Self> {
        Self::bind_with_prompter(addr, backend, Arc::new(Islands::default())).await
    }

    /// Like [`Server::bind`], asking unwatched questions through `prompter`.
    pub async fn bind_with_prompter(
        addr: impl ToSocketAddrs,
        backend: Arc<dyn Backend>,
        prompter: Arc<dyn Prompter>,
    ) -> std::io::Result<Self> {
        Self::bind_with(addr, backend, prompter, None).await
    }

    /// Like [`Server::bind_with_prompter`], keeping sessions in `store`
    /// between runs: the stored sessions are served again, and changes are
    /// saved every [`SAVE_PERIOD`]. Call [`Host::save`] on the way out for
    /// the last of them.
    pub async fn bind_with(
        addr: impl ToSocketAddrs,
        backend: Arc<dyn Backend>,
        prompter: Arc<dyn Prompter>,
        store: Option<Store>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let host = Host::with_store(backend, prompter, store);
        host.save_periodically(SAVE_PERIOD);
        Ok(Self { listener, host })
    }

    /// The host behind the server, for saving on the way out.
    pub fn host(&self) -> Arc<Host> {
        Arc::clone(&self.host)
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accepts connections until the listener fails.
    pub async fn run(self) -> std::io::Result<()> {
        loop {
            let (stream, peer) = self.listener.accept().await?;
            let host = Arc::clone(&self.host);
            tokio::spawn(async move {
                match serve_connection(stream, host).await {
                    Ok(()) => debug!(%peer, "connection closed"),
                    Err(err) => warn!(%peer, "connection failed: {err}"),
                }
            });
        }
    }
}

async fn serve_connection(stream: TcpStream, host: Arc<Host>) -> Result<(), tungstenite::Error> {
    let (mut sink, mut frames) = tokio_tungstenite::accept_async(stream).await?.split();
    let (outbox, mut queued) = mpsc::unbounded_channel();

    // The writer drains everything queued for this connection, by the host or
    // by the reader, and ends once every sender is gone.
    let writer = tokio::spawn(async move {
        while let Some(message) = queued.recv().await {
            match message {
                Outgoing::Message(message) => {
                    sink.send(Message::Text(message.to_string().into())).await?
                }
                Outgoing::Close => return sink.close().await,
            }
        }
        Ok(())
    });

    let mut conn = host.connect(outbox.clone());
    let mut result = Ok(());
    while let Some(frame) = frames.next().await {
        match frame {
            Ok(Message::Text(text)) => handle_text(&host, &mut conn, &outbox, text.as_str()),
            Ok(Message::Binary(_)) => {
                let err = RpcError::invalid_request("AHP messages must be sent as text frames");
                let _ = outbox.send(Outgoing::Message(rpc::error_response(Value::Null, &err)));
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(err) => {
                result = Err(err);
                break;
            }
        }
        if conn.close_requested() {
            let _ = outbox.send(Outgoing::Close);
            break;
        }
    }

    host.disconnect(&conn);
    drop(outbox);
    match writer.await {
        Ok(Err(err)) if result.is_ok() => Err(err),
        _ => result,
    }
}

fn handle_text(
    host: &Arc<Host>,
    conn: &mut Connection,
    outbox: &mpsc::UnboundedSender<Outgoing>,
    text: &str,
) {
    match rpc::parse(text) {
        Ok(Incoming::Request { id, method, params }) => {
            host.handle_request(conn, id, &method, params)
        }
        Ok(Incoming::Notification { method, params }) => {
            host.handle_notification(conn, &method, params)
        }
        Ok(Incoming::Response { id }) => {
            debug!(%id, "ignoring a response to an unknown server request")
        }
        Err(error_response) => {
            let _ = outbox.send(Outgoing::Message(error_response));
        }
    }
}
