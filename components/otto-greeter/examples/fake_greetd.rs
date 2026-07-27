//! A stand-in greetd daemon for testing the greeter end to end.
//!
//! It speaks the real `greetd-ipc(7)` wire protocol over a real unix socket, so
//! the greeter exercises its `Client::Real` path — length-prefix framing and
//! all — without needing root, a spare VT, or a live PAM stack. Nothing is ever
//! authenticated or executed.
//!
//! ```sh
//! cargo run -p otto-greeter --example fake_greetd -- /tmp/fake-greetd.sock
//! GREETD_SOCK=/tmp/fake-greetd.sock cargo run -p otto-greeter
//! ```
//!
//! `FAKE_GREETD_PASSWORD` sets the accepted password (default `otto`).
//! `FAKE_GREETD_SCENARIO` picks the conversation:
//!
//! * `simple` — straight to the password prompt
//! * `fingerprint` — an unanswerable info message first, as `pam_fprintd`
//!   sends (the default; exercises the auto-acknowledge path)
//! * `two-factor` — password, then a visible one-time-code prompt
//! * `locked` — always rejects, whatever the password

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/fake-greetd.sock".to_string());
    let password = std::env::var("FAKE_GREETD_PASSWORD").unwrap_or_else(|_| "otto".to_string());
    let scenario = std::env::var("FAKE_GREETD_SCENARIO").unwrap_or_else(|_| "fingerprint".into());

    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    println!("fake-greetd listening on {path}");
    println!("  scenario: {scenario}");
    println!("  password: {password}");
    println!("Point the greeter at it with GREETD_SOCK={path}");

    for stream in listener.incoming() {
        let stream = stream?;
        println!("\n-- greeter connected --");
        if let Err(e) = serve(stream, &password, &scenario) {
            println!("connection ended: {e}");
        }
    }
    Ok(())
}

/// One greeter connection. State lives here, so a reconnect starts clean.
fn serve(mut stream: UnixStream, password: &str, scenario: &str) -> std::io::Result<()> {
    // Which prompt the greeter is currently answering.
    let mut stage = Stage::NoSession;

    loop {
        let request = match read_message(&mut stream) {
            Ok(request) => request,
            // The greeter closing the socket is a normal end of conversation.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        println!("<- {request}");

        let request: serde_json::Value = serde_json::from_str(&request)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let kind = request.get("type").and_then(|t| t.as_str()).unwrap_or("");

        let response = match kind {
            "create_session" => {
                let username = request
                    .get("username")
                    .and_then(|u| u.as_str())
                    .unwrap_or("");
                if username.is_empty() {
                    error("auth_error", "No username given")
                } else if scenario == "fingerprint" {
                    // An info message expects no answer — the greeter must
                    // acknowledge it with a null response before it gets the
                    // real prompt.
                    stage = Stage::AfterInfo;
                    auth_message("info", "Place your finger on the sensor")
                } else {
                    stage = Stage::Password;
                    auth_message("secret", "Password:")
                }
            }

            "post_auth_message_response" => {
                let answer = request.get("response").and_then(|r| r.as_str());
                match stage {
                    Stage::AfterInfo => {
                        stage = Stage::Password;
                        auth_message("secret", "Password:")
                    }
                    Stage::Password => {
                        if scenario == "locked" {
                            error("auth_error", "Account is locked")
                        } else if answer != Some(password) {
                            error("auth_error", "Login incorrect")
                        } else if scenario == "two-factor" {
                            stage = Stage::OneTimeCode;
                            auth_message("visible", "Verification code:")
                        } else {
                            stage = Stage::Authenticated;
                            success()
                        }
                    }
                    Stage::OneTimeCode => {
                        // Any code is accepted; this exercises the visible
                        // (unmasked) prompt rendering, not real 2FA.
                        stage = Stage::Authenticated;
                        success()
                    }
                    Stage::NoSession | Stage::Authenticated => {
                        error("error", "No auth message is pending")
                    }
                }
            }

            "start_session" => {
                if matches!(stage, Stage::Authenticated) {
                    let cmd = request
                        .get("cmd")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    println!("** would exec session: {cmd} **");
                    println!("** a real greetd would now kill the greeter **");
                    success()
                } else {
                    error("error", "Session is not authenticated")
                }
            }

            "cancel_session" => {
                stage = Stage::NoSession;
                success()
            }

            other => error("error", &format!("Unknown request type: {other}")),
        };

        println!("-> {response}");
        write_message(&mut stream, &response)?;
    }
}

enum Stage {
    NoSession,
    AfterInfo,
    Password,
    OneTimeCode,
    Authenticated,
}

fn auth_message(kind: &str, text: &str) -> String {
    serde_json::json!({
        "type": "auth_message",
        "auth_message_type": kind,
        "auth_message": text,
    })
    .to_string()
}

fn error(kind: &str, description: &str) -> String {
    serde_json::json!({
        "type": "error",
        "error_type": kind,
        "description": description,
    })
    .to_string()
}

fn success() -> String {
    r#"{"type":"success"}"#.to_string()
}

/// Read one length-prefixed message. The prefix is a native-endian `u32`.
fn read_message(stream: &mut UnixStream) -> std::io::Result<String> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_ne_bytes(len_buf) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    String::from_utf8(body).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn write_message(stream: &mut UnixStream, payload: &str) -> std::io::Result<()> {
    stream.write_all(&(payload.len() as u32).to_ne_bytes())?;
    stream.write_all(payload.as_bytes())?;
    stream.flush()
}
