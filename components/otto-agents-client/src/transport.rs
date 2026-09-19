//! Reaching the service: its socket, and a URL that names it.
//!
//! A URL is `unix:///path/to/agents.sock`, a bare absolute path, or `ws://`.
//! The socket is the access check — its directory is the user's alone — so a
//! loopback URL is for development and tests only.

use std::io;
use std::path::{Path, PathBuf};

use ahp::{BoxedTransport, Transport, TransportError, TransportMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// The loopback address a development service is told to use.
pub const LOOPBACK: &str = "127.0.0.1:4800";

/// The service's socket, relative to the runtime directory.
pub const SOCKET_PATH: &str = "otto-agents/agents.sock";

/// `$XDG_RUNTIME_DIR`, or `/run/user/<uid>` when that is a directory.
pub fn runtime_dir() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .or_else(|| {
            let uid = std::fs::metadata("/proc/self").ok()?.uid();
            Some(PathBuf::from(format!("/run/user/{uid}"))).filter(|dir| dir.is_dir())
        })
}

/// `$XDG_RUNTIME_DIR/otto-agents/agents.sock`; `None` with no runtime
/// directory.
pub fn default_socket_path() -> Option<PathBuf> {
    Some(runtime_dir()?.join(SOCKET_PATH))
}

/// Where clients connect unless `OTTO_AGENTS_URL` says otherwise.
pub fn default_url() -> String {
    match default_socket_path() {
        Some(path) => format!("unix://{}", path.display()),
        None => format!("ws://{LOOPBACK}"),
    }
}

/// The socket path a URL names, if it names one.
pub fn socket_path(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("unix:") {
        let path = path.strip_prefix("//").unwrap_or(path);
        return Some(PathBuf::from(path));
    }
    url.starts_with('/').then(|| PathBuf::from(url))
}

/// Opens an AHP transport to the service at `url`.
pub async fn connect(url: &str) -> io::Result<BoxedTransport> {
    match socket_path(url) {
        Some(path) => Ok(BoxedTransport::new(connect_unix(&path).await?)),
        None => ahp_ws::WebSocketTransport::connect(url)
            .await
            .map(BoxedTransport::new)
            .map_err(io::Error::other),
    }
}

async fn connect_unix(path: &Path) -> io::Result<UnixWebSocket> {
    let stream = UnixStream::connect(path).await?;
    // The host name is a formality: the socket is the address.
    let (stream, _response) = tokio_tungstenite::client_async("ws://localhost/", stream)
        .await
        .map_err(io::Error::other)?;
    Ok(UnixWebSocket(stream))
}

/// A WebSocket over a Unix stream as an AHP transport, framed the way
/// `ahp-ws` frames its TCP one.
struct UnixWebSocket(WebSocketStream<UnixStream>);

impl Transport for UnixWebSocket {
    async fn send(&mut self, message: TransportMessage) -> Result<(), TransportError> {
        let frame = match message {
            TransportMessage::Parsed(message) => Message::Text(
                serde_json::to_string(&message)
                    .map_err(|err| TransportError::Protocol(err.to_string()))?
                    .into(),
            ),
            TransportMessage::Text(text) => Message::Text(text.into()),
            TransportMessage::Binary(bytes) => Message::Binary(bytes.into()),
        };
        self.0
            .send(frame)
            .await
            .map_err(|err| TransportError::Io(err.to_string()))
    }

    async fn recv(&mut self) -> Result<Option<TransportMessage>, TransportError> {
        loop {
            return match self.0.next().await {
                None | Some(Ok(Message::Close(_))) => Ok(None),
                Some(Err(err)) => Err(TransportError::Io(err.to_string())),
                Some(Ok(Message::Text(text))) => Ok(Some(TransportMessage::Text(text.to_string()))),
                Some(Ok(Message::Binary(bytes))) => {
                    Ok(Some(TransportMessage::Binary(bytes.into())))
                }
                // Pings and pongs are answered underneath.
                Some(Ok(_)) => continue,
            };
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.0
            .close(None)
            .await
            .map_err(|err| TransportError::Io(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_name_sockets_or_tcp() {
        assert_eq!(
            socket_path("unix:///run/user/1000/otto-agents/agents.sock"),
            Some(PathBuf::from("/run/user/1000/otto-agents/agents.sock"))
        );
        assert_eq!(
            socket_path("unix:/tmp/a.sock"),
            Some(PathBuf::from("/tmp/a.sock"))
        );
        assert_eq!(
            socket_path("/tmp/a.sock"),
            Some(PathBuf::from("/tmp/a.sock"))
        );
        assert_eq!(socket_path("ws://127.0.0.1:4800"), None);
    }
}
