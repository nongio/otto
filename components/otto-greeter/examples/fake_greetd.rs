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
//!   sends, and then silence until `FAKE_GREETD_READER_WAIT` seconds have
//!   passed (default 15), as `pam_fprintd` also does: the default scenario,
//!   and the one that exercises reaching the password through a reader that is
//!   holding the conversation
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
    // How long the `fingerprint` scenario holds the conversation before
    // reporting a miss — `pam_fprintd`'s own default is 30 seconds, which is a
    // long time to sit through while working on the panel.
    let reader_wait = std::time::Duration::from_secs(
        std::env::var("FAKE_GREETD_READER_WAIT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(15),
    );

    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    println!("fake-greetd listening on {path}");
    println!("  scenario: {scenario}");
    println!("  password: {password}");
    println!("Point the greeter at it with GREETD_SOCK={path}");

    for stream in listener.incoming() {
        let stream = stream?;
        println!("\n-- greeter connected --");
        if let Err(e) = serve(stream, &password, &scenario, reader_wait) {
            println!("connection ended: {e}");
        }
    }
    Ok(())
}

/// One greeter connection. State lives here, so a reconnect starts clean.
fn serve(
    mut stream: UnixStream,
    password: &str,
    scenario: &str,
    reader_wait: std::time::Duration,
) -> std::io::Result<()> {
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
                    // What `pam_fprintd` does with the acknowledgement: nothing,
                    // for as long as nobody touches the reader. Holding the
                    // reply here holds the whole conversation, which is the
                    // thing worth reproducing — it is what the greeter has to
                    // stay usable through.
                    Stage::AfterInfo => {
                        println!(
                            "   (holding the conversation for {reader_wait:?}, as a reader would)"
                        );
                        std::thread::sleep(reader_wait);
                        stage = Stage::AfterMiss;
                        auth_message("error", "Failed to match fingerprint")
                    }
                    // The reader gave up; the password stack takes over.
                    Stage::AfterMiss => {
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
    AfterMiss,
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
