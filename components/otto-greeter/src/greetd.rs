//! greetd IPC client.
//!
//! greetd runs as root, owns the VT, and hands the greeter a unix socket on
//! `$GREETD_SOCK`. The greeter drives the authentication conversation over that
//! socket and never links PAM itself.
//!
//! Wire format is a native-endian `u32` length prefix followed by a JSON
//! payload, in both directions. See `greetd-ipc(7)`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// A request sent to greetd.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Begin an authentication conversation for `username`.
    CreateSession { username: String },
    /// Answer the most recent `AuthMessage`.
    PostAuthMessageResponse { response: Option<String> },
    /// Authentication succeeded — run `cmd` as the authenticated user.
    StartSession { cmd: Vec<String>, env: Vec<String> },
    /// Abandon the current conversation and start over.
    CancelSession,
}

/// The kind of answer greetd is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMessageType {
    /// Free text that should be echoed as the user types (e.g. an OTP).
    Visible,
    /// A secret — render it masked (e.g. a password).
    Secret,
    /// Informational, no answer expected (e.g. "Place your finger on the sensor").
    Info,
    /// An error from the PAM stack, no answer expected.
    Error,
}

/// Why a request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    /// Authentication failed — wrong password, locked account, etc.
    AuthError,
    /// Anything else (protocol misuse, greetd-internal failure).
    Error,
}

/// A response received from greetd.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// The request was accepted and the conversation is complete for now.
    Success,
    /// The request failed. The session must be cancelled before retrying.
    Error {
        error_type: ErrorType,
        description: String,
    },
    /// greetd needs an answer before it can continue.
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: String,
    },
}

/// Connection to greetd, or a self-contained stand-in for development.
///
/// The conversation is driven without ever blocking the event loop: a request
/// is [`Client::send`]-ed and the reply collected later by [`Client::poll`].
/// That matters because a PAM module answers when it is ready and not before —
/// `pam_fprintd` announces the reader and then waits for a finger, which can
/// take tens of seconds. A blocking client freezes the panel for exactly that
/// window: no repaint, no animation, and no way to give up and type a password
/// instead.
pub enum Client {
    Real {
        stream: UnixStream,
        /// Bytes received so far, which may be less than one frame or more
        /// than one — a stream socket makes no promises about the split.
        inbox: Vec<u8>,
        /// The peer has closed its end. Any frames already in `inbox` are still
        /// good and must be delivered before this is reported.
        closed: bool,
    },
    /// Used when `GREETD_SOCK` is unset — lets the greeter be developed and
    /// styled inside a normal Otto session without root or a spare VT.
    /// Accepts the password `otto` and never starts anything.
    Mock {
        awaiting_password: bool,
        /// The mock answers immediately, but through the same two-step
        /// interface, so both backends exercise the same code in the greeter.
        pending: Option<Response>,
    },
}

impl Client {
    /// Connect to greetd via `$GREETD_SOCK`, falling back to the mock backend
    /// when that variable is absent.
    pub fn connect() -> std::io::Result<Self> {
        match std::env::var("GREETD_SOCK") {
            Ok(path) => {
                let stream = UnixStream::connect(&path)?;
                // Non-blocking from the start: every read below assumes it.
                stream.set_nonblocking(true)?;
                tracing::info!(socket = %path, "Connected to greetd");
                Ok(Client::Real {
                    stream,
                    inbox: Vec::new(),
                    closed: false,
                })
            }
            Err(_) => {
                tracing::warn!(
                    "GREETD_SOCK is not set — running with the mock backend \
                     (password: \"otto\", no session will be started)"
                );
                Ok(Client::Mock {
                    awaiting_password: false,
                    pending: None,
                })
            }
        }
    }

    /// The socket to wait on, so the greeter learns that greetd has answered
    /// instead of asking it on a timer. The mock backend answers in `send` and
    /// has nothing to wait for.
    ///
    /// A closed socket is readable for good — waiting on that one would spin
    /// the loop rather than let it sleep — so it is withdrawn once the peer has
    /// hung up and the greeter has been told about it.
    pub fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;

        match self {
            Client::Real { stream, closed, .. } => (!closed).then(|| stream.as_raw_fd()),
            Client::Mock { .. } => None,
        }
    }

    /// Send `request`. The answer arrives through [`Client::poll`].
    pub fn send(&mut self, request: Request) -> std::io::Result<()> {
        match self {
            Client::Real { stream, .. } => Self::send_real(stream, request),
            Client::Mock {
                awaiting_password,
                pending,
            } => {
                *pending = Some(Self::answer_mock(awaiting_password, request));
                Ok(())
            }
        }
    }

    /// Take the next complete response, if one has arrived.
    ///
    /// `Ok(None)` means "nothing yet, ask again later" — the normal case while
    /// a PAM module is thinking.
    pub fn poll(&mut self) -> std::io::Result<Option<Response>> {
        match self {
            Client::Real {
                stream,
                inbox,
                closed,
            } => Self::poll_real(stream, inbox, closed),
            Client::Mock { pending, .. } => Ok(pending.take()),
        }
    }

    /// Send `request` and block until greetd answers.
    ///
    /// Only for tests and one-shot tools; the greeter itself must not block.
    #[cfg(test)]
    pub fn roundtrip(&mut self, request: Request) -> std::io::Result<Response> {
        self.send(request)?;
        loop {
            if let Some(response) = self.poll()? {
                return Ok(response);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn send_real(stream: &mut UnixStream, request: Request) -> std::io::Result<()> {
        let payload = serde_json::to_vec(&request)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Length prefix is native-endian, per greetd-ipc(7).
        let len = u32::try_from(payload.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "request too large")
        })?;

        // Requests are a few hundred bytes at most, well under a socket buffer,
        // so a short write here would mean something is badly wrong — treat it
        // as the error it is rather than carrying a write queue around.
        stream.write_all(&len.to_ne_bytes())?;
        stream.write_all(&payload)?;
        stream.flush()
    }

    fn poll_real(
        stream: &mut UnixStream,
        inbox: &mut Vec<u8>,
        closed: &mut bool,
    ) -> std::io::Result<Option<Response>> {
        let mut chunk = [0u8; 4096];
        while !*closed {
            match stream.read(&mut chunk) {
                // greetd closed the socket. On a successful login this is just
                // the handoff beginning; either way nothing more will arrive.
                Ok(0) => *closed = true,
                Ok(read) => inbox.extend_from_slice(&chunk[..read]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }

        // A reply already received outranks the close that followed it: greetd
        // answers `start_session` and then hangs up, and losing that answer
        // would turn a successful login into an error message.
        if let Some(response) = Self::take_frame(inbox)? {
            return Ok(Some(response));
        }

        if *closed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "greetd closed the connection",
            ));
        }
        Ok(None)
    }

    /// Split one complete frame off the front of `inbox`, if there is one.
    fn take_frame(inbox: &mut Vec<u8>) -> std::io::Result<Option<Response>> {
        if inbox.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_ne_bytes(inbox[..4].try_into().expect("4 bytes")) as usize;
        if inbox.len() < 4 + len {
            return Ok(None);
        }

        let body: Vec<u8> = inbox.drain(..4 + len).skip(4).collect();
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    fn answer_mock(awaiting_password: &mut bool, request: Request) -> Response {
        match request {
            Request::CreateSession { username } => {
                if username.trim().is_empty() {
                    return Response::Error {
                        error_type: ErrorType::AuthError,
                        description: "No username given".to_string(),
                    };
                }
                *awaiting_password = true;
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Secret,
                    auth_message: "Password:".to_string(),
                }
            }
            Request::PostAuthMessageResponse { response } => {
                *awaiting_password = false;
                if response.as_deref() == Some("otto") {
                    Response::Success
                } else {
                    Response::Error {
                        error_type: ErrorType::AuthError,
                        description: "Authentication failed".to_string(),
                    }
                }
            }
            Request::StartSession { cmd, .. } => {
                tracing::info!(?cmd, "Mock backend: would start session");
                Response::Success
            }
            Request::CancelSession => {
                *awaiting_password = false;
                Response::Success
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The payloads below are taken from greetd-ipc(7). Getting a tag name
    // wrong here fails at runtime as an opaque protocol error, so pin them.

    #[test]
    fn create_session_matches_the_documented_shape() {
        let json = serde_json::to_string(&Request::CreateSession {
            username: "riccardo".to_string(),
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"create_session","username":"riccardo"}"#);
    }

    #[test]
    fn auth_response_carries_a_nullable_string() {
        let with_answer = serde_json::to_string(&Request::PostAuthMessageResponse {
            response: Some("hunter2".to_string()),
        })
        .unwrap();
        assert_eq!(
            with_answer,
            r#"{"type":"post_auth_message_response","response":"hunter2"}"#
        );

        // Info/error messages are acknowledged with a null response.
        let empty =
            serde_json::to_string(&Request::PostAuthMessageResponse { response: None }).unwrap();
        assert_eq!(
            empty,
            r#"{"type":"post_auth_message_response","response":null}"#
        );
    }

    #[test]
    fn start_session_matches_the_documented_shape() {
        let json = serde_json::to_string(&Request::StartSession {
            cmd: vec!["otto".to_string()],
            env: vec!["XDG_SESSION_TYPE=wayland".to_string()],
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"start_session","cmd":["otto"],"env":["XDG_SESSION_TYPE=wayland"]}"#
        );
    }

    #[test]
    fn parses_every_response_variant() {
        let success: Response = serde_json::from_str(r#"{"type":"success"}"#).unwrap();
        assert!(matches!(success, Response::Success));

        let auth: Response = serde_json::from_str(
            r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}"#,
        )
        .unwrap();
        let Response::AuthMessage {
            auth_message_type,
            auth_message,
        } = auth
        else {
            panic!("expected an auth message");
        };
        assert_eq!(auth_message_type, AuthMessageType::Secret);
        assert_eq!(auth_message, "Password:");

        let error: Response = serde_json::from_str(
            r#"{"type":"error","error_type":"auth_error","description":"Login incorrect"}"#,
        )
        .unwrap();
        let Response::Error {
            error_type,
            description,
        } = error
        else {
            panic!("expected an error");
        };
        assert_eq!(error_type, ErrorType::AuthError);
        assert_eq!(description, "Login incorrect");

        // A fingerprint prompt arrives as an info message needing no answer.
        let info: Response = serde_json::from_str(
            r#"{"type":"auth_message","auth_message_type":"info","auth_message":"Place your finger"}"#,
        )
        .unwrap();
        assert!(matches!(
            info,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Info,
                ..
            }
        ));
    }

    /// Drive the real socket path against a stand-in server to prove the
    /// length-prefix framing round-trips — the JSON tests above would still
    /// pass with the prefix written in the wrong endianness or omitted.
    #[test]
    fn real_client_frames_messages_over_the_socket() {
        use std::os::unix::net::UnixListener;

        let path =
            std::env::temp_dir().join(format!("otto-greeter-ipc-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();

            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).unwrap();
            let len = u32::from_ne_bytes(len_buf) as usize;
            let mut body = vec![0u8; len];
            stream.read_exact(&mut body).unwrap();

            // The server sees exactly the bytes the client meant to send.
            assert_eq!(
                String::from_utf8(body).unwrap(),
                r#"{"type":"create_session","username":"riccardo"}"#
            );

            let reply = br#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}"#;
            stream
                .write_all(&(reply.len() as u32).to_ne_bytes())
                .unwrap();
            stream.write_all(reply).unwrap();
            stream.flush().unwrap();
        });

        let stream = UnixStream::connect(&path).unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut client = Client::Real {
            stream,
            inbox: Vec::new(),
            closed: false,
        };
        let response = client
            .roundtrip(Request::CreateSession {
                username: "riccardo".to_string(),
            })
            .unwrap();

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);

        let Response::AuthMessage {
            auth_message_type,
            auth_message,
        } = response
        else {
            panic!("expected an auth message");
        };
        assert_eq!(auth_message_type, AuthMessageType::Secret);
        assert_eq!(auth_message, "Password:");
    }

    /// The whole point of the two-step interface: a module that takes its time
    /// must leave the greeter free to do other things. `pam_fprintd` announces
    /// the reader and then waits for a finger, which can be tens of seconds —
    /// a blocking client freezes the panel for all of it.
    #[test]
    fn a_slow_answer_does_not_block_the_caller() {
        use std::io::{Read, Write};

        let path =
            std::env::temp_dir().join(format!("otto-greetd-slow-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn({
            let path = path.clone();
            move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut len_buf = [0u8; 4];
                stream.read_exact(&mut len_buf).unwrap();
                let len = u32::from_ne_bytes(len_buf) as usize;
                let mut body = vec![0u8; len];
                stream.read_exact(&mut body).unwrap();

                // Answer late, as a PAM module waiting on hardware would.
                std::thread::sleep(std::time::Duration::from_millis(120));
                let reply = br#"{"type":"success"}"#;
                stream
                    .write_all(&(reply.len() as u32).to_ne_bytes())
                    .unwrap();
                stream.write_all(reply).unwrap();
                stream.flush().unwrap();
                // Hold the connection open until the client has read it.
                std::thread::sleep(std::time::Duration::from_millis(200));
                drop(path);
            }
        });

        let stream = UnixStream::connect(&path).unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut client = Client::Real {
            stream,
            inbox: Vec::new(),
            closed: false,
        };
        client
            .send(Request::CreateSession {
                username: "riccardo".to_string(),
            })
            .unwrap();

        // Poll returns immediately with nothing while the answer is pending.
        let started = std::time::Instant::now();
        let mut polls = 0;
        let response = loop {
            match client.poll().unwrap() {
                Some(response) => break response,
                None => {
                    polls += 1;
                    assert!(
                        started.elapsed() < std::time::Duration::from_secs(5),
                        "the answer never arrived"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        };

        assert!(
            polls > 1,
            "the client should have returned control while waiting, not blocked"
        );
        assert!(matches!(response, Response::Success));

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mock_backend_walks_the_full_conversation() {
        let mut client = Client::Mock {
            awaiting_password: false,
            pending: None,
        };

        let response = client
            .roundtrip(Request::CreateSession {
                username: "riccardo".to_string(),
            })
            .unwrap();
        assert!(matches!(
            response,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                ..
            }
        ));

        let rejected = client
            .roundtrip(Request::PostAuthMessageResponse {
                response: Some("wrong".to_string()),
            })
            .unwrap();
        assert!(matches!(
            rejected,
            Response::Error {
                error_type: ErrorType::AuthError,
                ..
            }
        ));

        let accepted = client
            .roundtrip(Request::PostAuthMessageResponse {
                response: Some("otto".to_string()),
            })
            .unwrap();
        assert!(matches!(accepted, Response::Success));
    }
}
