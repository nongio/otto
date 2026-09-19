//! WebSocket transport: each text frame carries exactly one JSON-RPC message.

use std::fs::{self, DirBuilder, Permissions};
use std::io;
use std::net::SocketAddr;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, ToSocketAddrs, UnixListener};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message};
use tracing::{debug, info, warn};

use crate::agent::Backend;
use crate::dialog::{Islands, Prompter};
use crate::host::{Connection, Host, Outgoing};
use crate::rpc::{self, Incoming, RpcError};
use crate::store::Store;

/// How often changed sessions are written to the store. At most this much is
/// lost if the service dies without saving on the way out.
pub const SAVE_PERIOD: Duration = Duration::from_secs(1);

/// The loopback address the service listens on when it has no runtime
/// directory for its socket, and the one development builds are told to use.
pub const LOOPBACK: &str = "127.0.0.1:4800";

/// The service's socket, relative to the runtime directory.
pub const SOCKET_PATH: &str = "otto-agents/agents.sock";

/// `$XDG_RUNTIME_DIR/otto-agents/agents.sock`, or the same under
/// `/run/user/<uid>`; `None` when the session has neither.
pub fn default_socket_path() -> Option<PathBuf> {
    Some(crate::xdg::runtime_dir()?.join(SOCKET_PATH))
}

/// No runtime directory, so no socket to listen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoRuntimeDir;

impl std::fmt::Display for NoRuntimeDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no runtime directory for the socket: neither $XDG_RUNTIME_DIR nor \
             /run/user/<uid> is a directory. Pass --listen {LOOPBACK} to accept \
             connections over loopback TCP instead, which any local user can reach."
        )
    }
}

impl std::error::Error for NoRuntimeDir {}

/// Where the server accepts connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    /// A Unix socket at this path: mode 0600, in a directory of mode 0700, so
    /// only the user's own processes reach the service.
    Unix(PathBuf),
    /// A TCP address, `host:port`. Anything that can reach the port can drive
    /// the agents, so this is for development and tests.
    Tcp(String),
}

impl Listen {
    /// Reads a `--listen` value: a socket path (absolute, or after `unix:`),
    /// otherwise `host:port`.
    pub fn parse(value: &str) -> Self {
        if let Some(path) = value.strip_prefix("unix:") {
            let path = path.strip_prefix("//").unwrap_or(path);
            return Self::Unix(PathBuf::from(path));
        }
        if value.starts_with('/') {
            return Self::Unix(PathBuf::from(value));
        }
        Self::Tcp(value.to_owned())
    }

    /// The socket in the runtime directory.
    ///
    /// There is no fallback. The protocol has no authentication of its own —
    /// the socket's 0600 mode inside a 0700 directory is the whole access
    /// check — so a TCP port would hand every local user the run of the
    /// agents. A session without a runtime directory is unusual enough
    /// (a bare `su`, a container) that saying so beats guessing.
    pub fn default_endpoint() -> Result<Self, NoRuntimeDir> {
        default_socket_path().map(Self::Unix).ok_or(NoRuntimeDir)
    }

    /// The `--url` that reaches this endpoint.
    pub fn url(&self) -> String {
        match self {
            Self::Unix(path) => format!("unix://{}", path.display()),
            Self::Tcp(addr) => format!("ws://{addr}"),
        }
    }
}

impl std::str::FromStr for Listen {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse(value))
    }
}

/// A bound listener. The socket file goes with it.
enum Listener {
    Tcp(TcpListener),
    Unix(UnixListener, PathBuf),
}

impl Listener {
    fn endpoint(&self) -> io::Result<Listen> {
        match self {
            Self::Tcp(listener) => Ok(Listen::Tcp(listener.local_addr()?.to_string())),
            Self::Unix(_, path) => Ok(Listen::Unix(path.clone())),
        }
    }

    /// Accepts the next connection, with a name for the peer in logs.
    async fn accept(&self) -> io::Result<(Box<dyn IoStream>, String)> {
        match self {
            Self::Tcp(listener) => {
                let (stream, peer) = listener.accept().await?;
                Ok((Box::new(stream), peer.to_string()))
            }
            Self::Unix(listener, _) => loop {
                let (stream, _) = listener.accept().await?;
                let cred = stream.peer_cred().ok();
                // The socket's mode is the access check, but it costs nothing
                // to say so again here: a peer of another user has no business
                // driving this session's agents, whatever the mode on the
                // directory turned out to be.
                let uid = cred.as_ref().map(|cred| cred.uid());
                let ours = fs::metadata("/proc/self").ok().map(|us| us.uid());
                if let (Some(uid), Some(ours)) = (uid, ours)
                    && uid != ours
                {
                    tracing::warn!(peer_uid = uid, "refusing a connection from another user");
                    continue;
                }
                let peer = match cred.and_then(|cred| cred.pid()) {
                    Some(pid) => format!("pid {pid}"),
                    None => "unix".to_owned(),
                };
                return Ok((Box::new(stream), peer));
            },
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Self::Unix(_, path) = self {
            let _ = fs::remove_file(path);
        }
    }
}

/// What a connection runs over: a TCP or a Unix stream.
trait IoStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> IoStream for S {}

/// Binds the socket at `path` in a private directory: the directory is made
/// (or tightened to) 0700 and the socket 0600. A socket file left by an
/// earlier run is replaced, unless a service is still answering on it.
fn bind_unix(path: &Path) -> io::Result<UnixListener> {
    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    if let Some(dir) = dir {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    fs::set_permissions(path, Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    UnixListener::from_std(listener)
}

pub struct Server {
    listener: Listener,
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
        let listener = Listener::Tcp(TcpListener::bind(addr).await?);
        Ok(Self::serve(listener, backend, prompter, store))
    }

    /// Binds a server on `listen`, keeping sessions in `store` as
    /// [`Server::bind_with`] does. TCP is logged as unauthenticated.
    pub async fn listen(
        listen: &Listen,
        backend: Arc<dyn Backend>,
        prompter: Arc<dyn Prompter>,
        store: Option<Store>,
    ) -> std::io::Result<Self> {
        let listener = match listen {
            Listen::Unix(path) => {
                if is_answering(path) {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another service is listening on {}", path.display()),
                    ));
                }
                Listener::Unix(bind_unix(path)?, path.clone())
            }
            Listen::Tcp(addr) => {
                warn!(
                    "listening on TCP without authentication; use a Unix socket outside development"
                );
                Listener::Tcp(TcpListener::bind(addr).await?)
            }
        };
        Ok(Self::serve(listener, backend, prompter, store))
    }

    fn serve(
        listener: Listener,
        backend: Arc<dyn Backend>,
        prompter: Arc<dyn Prompter>,
        store: Option<Store>,
    ) -> Self {
        let host = Host::with_store(backend, prompter, store);
        host.save_periodically(SAVE_PERIOD);
        Self { listener, host }
    }

    /// The host behind the server, for saving on the way out.
    pub fn host(&self) -> Arc<Host> {
        Arc::clone(&self.host)
    }

    /// The bound endpoint: for TCP, with the port the system chose.
    pub fn endpoint(&self) -> std::io::Result<Listen> {
        self.listener.endpoint()
    }

    /// The TCP address bound; an error for a Unix socket.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        match &self.listener {
            Listener::Tcp(listener) => listener.local_addr(),
            Listener::Unix(_, path) => Err(io::Error::other(format!(
                "listening on the socket {}, not on TCP",
                path.display()
            ))),
        }
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

/// Whether a service still answers on the socket at `path`.
fn is_answering(path: &Path) -> bool {
    let answering = std::os::unix::net::UnixStream::connect(path).is_ok();
    if !answering && path.exists() {
        info!(path = %path.display(), "replacing a stale socket");
    }
    answering
}

async fn serve_connection(
    stream: Box<dyn IoStream>,
    host: Arc<Host>,
) -> Result<(), tungstenite::Error> {
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
