//! Talking to `polkit-agent-helper-1`.
//!
//! An authentication agent does not run PAM itself: it cannot tell polkitd
//! that someone authenticated, since anyone could say that. polkit's helper
//! does both. It runs the `polkit-1` PAM stack for the user it is given and,
//! if that succeeds, reports the cookie to polkitd itself — the agent only
//! relays the conversation. So nothing this process does can authorize
//! anything on its own: a password it never forwarded, or a cancel, leaves
//! polkitd with no answer and the request refused.
//!
//! Two ways to reach it, tried in the order libpolkit-agent tries them:
//!
//! * the socket-activated helper at [`SOCKET`] (polkit 126 and later, no
//!   setuid bit): connect, send the user name and the cookie, a line each;
//! * the setuid binary, started with the user name as its argument and the
//!   cookie on its standard input.
//!
//! From there the protocol is the same, one line at a time. The helper
//! writes `PAM_PROMPT_ECHO_OFF <text>`, `PAM_PROMPT_ECHO_ON <text>`,
//! `PAM_ERROR_MSG <text>` or `PAM_TEXT_INFO <text>`, the text escaped as
//! GLib's `g_strescape` does, and finally `SUCCESS` or `FAILURE`. A prompt
//! is answered with one line. See polkit's `polkitagenthelper-pam.c` and
//! `polkitagentsession.c`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use otto_auth_ui::pam::{self, Event, Message, Outcome};

/// The socket-activated helper.
pub const SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Where distributions put the setuid helper: Arch and Fedora, Debian and
/// Ubuntu, older Debian.
pub const HELPER_PATHS: &[&str] = &[
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
];

/// The longest line read from the helper. PAM messages are a sentence; this
/// only bounds a helper that has gone wrong.
const MAX_LINE: usize = 8192;

/// One line from the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Prompt {
        text: String,
        secret: bool,
    },
    Error(String),
    Info(String),
    Success,
    Failure,
    /// Anything else. libpolkit-agent ends the session on one; so does this.
    Unknown,
}

/// Parse one line the helper wrote, with or without its newline.
///
/// The whole line is unescaped first, as `g_strcompress` does in
/// libpolkit-agent, then matched on its prefix.
pub fn parse_line(raw: &[u8]) -> Line {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    let unescaped = unescape(raw);
    let text = String::from_utf8_lossy(&unescaped);
    let rest = |prefix: &str| text.strip_prefix(prefix).map(str::to_string);
    if let Some(text) = rest("PAM_PROMPT_ECHO_OFF ") {
        Line::Prompt { text, secret: true }
    } else if let Some(text) = rest("PAM_PROMPT_ECHO_ON ") {
        Line::Prompt {
            text,
            secret: false,
        }
    } else if let Some(text) = rest("PAM_ERROR_MSG ") {
        Line::Error(text)
    } else if let Some(text) = rest("PAM_TEXT_INFO ") {
        Line::Info(text)
    } else if text.starts_with("SUCCESS") {
        Line::Success
    } else if text.starts_with("FAILURE") {
        Line::Failure
    } else {
        Line::Unknown
    }
}

/// Undo `g_strescape`: `\b \f \n \r \t \v \\ \"`, and `\NNN` octal for
/// every other byte it escaped — which is every byte outside printable
/// ASCII, so a UTF-8 prompt arrives entirely as octal.
pub fn unescape(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 == bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        match next {
            b'0'..=b'7' => {
                let mut value: u32 = 0;
                let mut len = 0;
                while len < 3 && i + 1 + len < bytes.len() {
                    let digit = bytes[i + 1 + len];
                    if !(b'0'..=b'7').contains(&digit) {
                        break;
                    }
                    value = value * 8 + u32::from(digit - b'0');
                    len += 1;
                }
                out.push((value & 0xFF) as u8);
                i += 1 + len;
                continue;
            }
            b'b' => out.push(0x08),
            b'f' => out.push(0x0C),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(0x0B),
            other => out.push(other),
        }
        i += 2;
    }
    out
}

/// The two ends of the helper, however it was reached.
enum Link {
    Socket(UnixStream),
    Child(Child),
}

impl Link {
    /// Reach the helper for `user` and hand it `cookie`. Returns the link and
    /// what to read its lines from.
    fn open(user: &str, cookie: &str) -> std::io::Result<(Self, Box<dyn Read + Send>)> {
        if std::path::Path::new(SOCKET).exists() {
            match UnixStream::connect(SOCKET) {
                Ok(mut stream) => {
                    // What `polkitagentsession.c` writes on connecting: the
                    // user, then the cookie, a line each. The helper reads
                    // both with `getline` before it starts PAM.
                    stream.write_all(format!("{user}\n{cookie}\n").as_bytes())?;
                    let reader = stream.try_clone()?;
                    tracing::info!(%user, "polkit helper: connected to {SOCKET}");
                    return Ok((Self::Socket(stream), Box::new(reader)));
                }
                Err(err) => {
                    tracing::warn!(%err, "cannot reach {SOCKET}; starting the setuid helper");
                }
            }
        }
        let helper = HELPER_PATHS
            .iter()
            .find(|path| std::path::Path::new(path).is_file())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "polkit-agent-helper-1 is not installed",
                )
            })?;
        let mut child = Command::new(helper)
            .arg(user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        tracing::info!(%user, pid = child.id(), "polkit helper: started {helper}");
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::other("the helper was started without a standard output")
        })?;
        let mut link = Self::Child(child);
        link.send_line(cookie)?;
        Ok((link, Box::new(stdout)))
    }

    fn send_line(&mut self, text: &str) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(text.len() + 1);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(b'\n');
        let result = match self {
            Self::Socket(stream) => stream.write_all(&bytes).and_then(|()| stream.flush()),
            Self::Child(child) => match child.stdin.as_mut() {
                Some(stdin) => stdin.write_all(&bytes).and_then(|()| stdin.flush()),
                None => Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
            },
        };
        wipe_bytes(&mut bytes);
        result
    }

    /// Stop the helper, whatever it is doing, and reap it.
    ///
    /// A socket-activated helper cannot be killed from here; closing the
    /// connection is all there is. Its next read or write fails, which fails
    /// the PAM conversation — though a module busy with its own device (a
    /// fingerprint reader waiting on a finger) finishes that first.
    fn end(self, abandon: bool) {
        match self {
            Self::Socket(stream) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            Self::Child(mut child) => {
                if abandon {
                    let _ = child.kill();
                }
                drop(child.stdin.take());
                let _ = child.wait();
            }
        }
    }
}

/// Overwrite an answer's bytes before they are freed, as [`pam::wipe`] does.
fn wipe_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
}

/// Read the helper's lines on a thread of their own, so the conversation can
/// notice that the dialog went away while the helper is still busy — a
/// fingerprint reader waiting on a finger holds it for as long as it likes.
/// `None` is the helper hanging up.
fn spawn_reader(reader: Box<dyn Read + Send>) -> Receiver<Option<Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("polkit-helper".into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                let mut line = Vec::new();
                let read = (&mut reader)
                    .take(MAX_LINE as u64)
                    .read_until(b'\n', &mut line);
                match read {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(None);
                        return;
                    }
                    Ok(_) => {
                        if tx.send(Some(line)).is_err() {
                            return;
                        }
                    }
                }
            }
        });
    rx
}

/// How often the conversation looks at both sides — the helper's lines and
/// the dialog's answers — while neither has anything to say.
const POLL: Duration = Duration::from_millis(100);

/// How long the helper may stay silent while it is not waiting on the user
/// before the connection counts as stuck and is closed. `pam_fprintd` waits
/// 30 seconds for a finger, up to three times; this is well past that.
const SILENCE_LIMIT: Duration = Duration::from_secs(180);

/// The whole conversation with the helper, on the attempt's thread; see
/// [`otto_auth_ui::pam::Attempt::with_conversation`].
pub fn converse(
    user: &str,
    cookie: &str,
    events: &Sender<Event>,
    answers: &Receiver<String>,
) -> Outcome {
    let unusable = |text: &str| text.is_empty() || text.contains(['\n', '\r', '\0']);
    if unusable(user) || unusable(cookie) {
        tracing::error!("refusing a user name or cookie the helper cannot be given");
        return Outcome::Denied(otto_kit::t_owned!("lock-error-service-failed"));
    }

    let (link, reader) = match Link::open(user, cookie) {
        Ok(opened) => opened,
        Err(err) => {
            tracing::error!(%err, "cannot start the polkit helper");
            return Outcome::Denied(otto_kit::t_owned!("lock-error-unavailable"));
        }
    };
    relay(link, spawn_reader(reader), events, answers, SILENCE_LIMIT)
}

/// How the conversation ended.
enum End {
    /// The helper said `SUCCESS` or `FAILURE`.
    Answered(Outcome),
    /// The dialog went away: stop the helper.
    Abandoned,
    /// The helper stopped making sense, hung up, or went quiet for too long.
    Broken,
}

/// Relay between the helper and the dialog until one of them ends it.
///
/// Both sides are watched the whole time: an answer is written the moment
/// it arrives, a helper that hangs up while a prompt is on screen is noticed
/// straight away, and so is a dialog that goes away while the helper is busy
/// with a fingerprint reader.
fn relay(
    mut link: Link,
    lines: Receiver<Option<Vec<u8>>>,
    events: &Sender<Event>,
    answers: &Receiver<String>,
    silence: Duration,
) -> Outcome {
    let say = |message: Message| {
        let _ = events.send(Event::Said(message));
        otto_kit::AppContext::request_wakeup();
    };
    // A prompt is on screen and the helper is blocked reading its answer.
    let mut asking = false;
    let mut heard = Instant::now();

    let end = loop {
        // The dialog's side.
        if asking {
            match answers.recv_timeout(POLL) {
                Ok(mut answer) => {
                    asking = false;
                    // A line is the whole answer: one with a line break in
                    // it would answer the next prompt too.
                    if answer.contains(['\n', '\r', '\0']) {
                        pam::wipe(&mut answer);
                        tracing::warn!("polkit helper: refusing an answer with a line break in it");
                        break End::Broken;
                    }
                    let sent = link.send_line(&answer);
                    pam::wipe(&mut answer);
                    if let Err(err) = sent {
                        tracing::warn!(%err, "polkit helper: cannot write the answer");
                        break End::Broken;
                    }
                    tracing::info!("polkit helper: answer written");
                    heard = Instant::now();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break End::Abandoned,
            }
        } else {
            match answers.try_recv() {
                // Typed before anything was asked; the dialog holds on to
                // what it means to send, so this is only dropped.
                Ok(mut stray) => pam::wipe(&mut stray),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => break End::Abandoned,
            }
        }

        // The helper's side. While a prompt waits, the helper writes nothing
        // until it is answered, so only a hang-up can come.
        let next = if asking {
            lines.try_recv().map_err(|err| match err {
                TryRecvError::Empty => RecvTimeoutError::Timeout,
                TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            })
        } else {
            lines.recv_timeout(POLL)
        };
        let line = match next {
            Ok(Some(line)) => line,
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                tracing::warn!(asking, "polkit helper: hung up without SUCCESS or FAILURE");
                break End::Broken;
            }
            Err(RecvTimeoutError::Timeout) => {
                if !asking && heard.elapsed() >= silence {
                    tracing::warn!(
                        seconds = heard.elapsed().as_secs(),
                        "polkit helper: silent for too long; closing it"
                    );
                    break End::Broken;
                }
                continue;
            }
        };
        heard = Instant::now();

        match parse_line(&line) {
            Line::Prompt { text, secret } => {
                // The prompt is PAM's question ("Password: "), never an answer.
                tracing::info!(%text, secret, "polkit helper: prompt");
                asking = true;
                say(Message::Prompt { text, secret });
            }
            Line::Error(text) => {
                tracing::info!(%text, "polkit helper: PAM_ERROR_MSG");
                say(Message::Error(text));
            }
            Line::Info(text) => {
                tracing::info!(%text, "polkit helper: PAM_TEXT_INFO");
                say(Message::Info(text));
            }
            Line::Success => {
                tracing::info!("polkit helper: SUCCESS");
                break End::Answered(Outcome::Authenticated);
            }
            Line::Failure => {
                tracing::info!("polkit helper: FAILURE");
                break End::Answered(Outcome::Denied(otto_kit::t_owned!(
                    "authorize-error-failed"
                )));
            }
            Line::Unknown => {
                tracing::warn!(bytes = line.len(), "polkit helper: unexpected line");
                break End::Broken;
            }
        }
    };

    match end {
        End::Answered(outcome) => {
            link.end(false);
            outcome
        }
        End::Abandoned => {
            tracing::info!("polkit helper: the dialog ended first; closing the helper");
            link.end(true);
            // Nobody is left to read it.
            Outcome::Denied(String::new())
        }
        End::Broken => {
            link.end(true);
            Outcome::Denied(otto_kit::t_owned!("lock-error-service-failed"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_and_messages_are_told_apart() {
        assert_eq!(
            parse_line(b"PAM_PROMPT_ECHO_OFF Password: \n"),
            Line::Prompt {
                text: "Password: ".into(),
                secret: true
            }
        );
        assert_eq!(
            parse_line(b"PAM_PROMPT_ECHO_ON login:"),
            Line::Prompt {
                text: "login:".into(),
                secret: false
            }
        );
        assert_eq!(
            parse_line(b"PAM_TEXT_INFO Place your finger on the reader\n"),
            Line::Info("Place your finger on the reader".into())
        );
        assert_eq!(
            parse_line(b"PAM_ERROR_MSG Failed to match fingerprint\n"),
            Line::Error("Failed to match fingerprint".into())
        );
        assert_eq!(parse_line(b"SUCCESS\n"), Line::Success);
        assert_eq!(parse_line(b"FAILURE\n"), Line::Failure);
        assert_eq!(parse_line(b"PAM_PROMPT_ECHO_OFFPassword"), Line::Unknown);
        assert_eq!(parse_line(b""), Line::Unknown);
    }

    /// `g_strescape` sends everything outside printable ASCII as octal, so a
    /// translated prompt only reads right once unescaped.
    #[test]
    fn escaped_text_is_restored() {
        assert_eq!(
            parse_line(b"PAM_PROMPT_ECHO_OFF Passwort f\\303\\274r \\\"root\\\":\n"),
            Line::Prompt {
                text: "Passwort für \"root\":".into(),
                secret: true
            }
        );
        assert_eq!(unescape(b"a\\tb\\\\c\\nd"), b"a\tb\\c\nd");
        assert_eq!(unescape(b"trailing\\"), b"trailing\\");
        assert_eq!(unescape(b"\\101\\0x"), b"A\0x");
    }

    // -- A fake helper on the far end of a socketpair -----------------------

    use otto_auth_ui::pam::Attempt;
    use std::io::BufRead;

    /// One step of a scripted helper.
    enum Step {
        /// Write this line, as the helper would.
        Say(&'static str),
        /// Read one line from the agent, and record it.
        Read,
        /// Hang up.
        HangUp,
    }

    /// What the fake helper saw: the lines it read, and whether the agent
    /// closed the connection after the script ran out.
    struct Seen {
        read: Vec<String>,
        closed_by_agent: bool,
    }

    /// Start a helper that follows `script` on one end of a socketpair, and
    /// a conversation relaying to it on the other, as the dialog runs one.
    fn start(script: Vec<Step>, silence: Duration) -> (Attempt, std::thread::JoinHandle<Seen>) {
        let (agent, helper) = UnixStream::pair().unwrap();
        let fake = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(helper.try_clone().unwrap());
            let mut writer = helper;
            let mut read = Vec::new();
            for step in script {
                match step {
                    Step::Say(line) => {
                        writer.write_all(format!("{line}\n").as_bytes()).unwrap();
                    }
                    Step::Read => {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        read.push(line);
                    }
                    Step::HangUp => {
                        let _ = writer.shutdown(std::net::Shutdown::Both);
                        return Seen {
                            read,
                            closed_by_agent: false,
                        };
                    }
                }
            }
            // Wait for the agent to close its end, as a helper blocked in
            // `fgets` would.
            let mut rest = String::new();
            let closed_by_agent = matches!(reader.read_line(&mut rest), Ok(0));
            Seen {
                read,
                closed_by_agent,
            }
        });
        let reader: Box<dyn Read + Send> = Box::new(agent.try_clone().unwrap());
        let link = Link::Socket(agent);
        let lines = spawn_reader(reader);
        let attempt = Attempt::with_conversation(move |events, answers| {
            relay(link, lines, events, answers, silence)
        });
        (attempt, fake)
    }

    /// The next event from the conversation, within a few seconds.
    fn next(attempt: &mut Attempt) -> Event {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = attempt.poll() {
                return event;
            }
            assert!(Instant::now() < until, "the conversation went quiet");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    const FINGER: &str = "PAM_TEXT_INFO Place your right index finger on the fingerprint reader";

    /// pam_fprintd first, then pam_unix: the reader's hint is passed on, the
    /// password prompt that follows is shown, and the answer reaches the
    /// helper as one line.
    #[test]
    fn helper_finger_then_password_succeeds() {
        let (mut attempt, fake) = start(
            vec![
                Step::Say(FINGER),
                Step::Say("PAM_ERROR_MSG Failed to match fingerprint"),
                Step::Say("PAM_PROMPT_ECHO_OFF Password: "),
                Step::Read,
                Step::Say("SUCCESS"),
            ],
            SILENCE_LIMIT,
        );
        assert!(
            matches!(next(&mut attempt), Event::Said(Message::Info(text)) if text.contains("finger"))
        );
        assert!(matches!(next(&mut attempt), Event::Said(Message::Error(_))));
        assert_eq!(
            next(&mut attempt),
            Event::Said(Message::Prompt {
                text: "Password: ".into(),
                secret: true
            })
        );
        attempt.answer("correct horse".into());
        assert_eq!(next(&mut attempt), Event::Ended(Outcome::Authenticated));
        let seen = fake.join().unwrap();
        assert_eq!(seen.read, vec!["correct horse\n".to_string()]);
        assert!(seen.closed_by_agent);
    }

    /// The reader gives up and the stack fails: a refusal, and the
    /// connection is closed.
    #[test]
    fn helper_finger_then_failure_is_a_refusal() {
        let (mut attempt, fake) =
            start(vec![Step::Say(FINGER), Step::Say("FAILURE")], SILENCE_LIMIT);
        assert!(matches!(next(&mut attempt), Event::Said(Message::Info(_))));
        assert!(
            matches!(next(&mut attempt), Event::Ended(Outcome::Denied(reason)) if !reason.is_empty())
        );
        assert!(fake.join().unwrap().closed_by_agent);
    }

    /// The dialog goes away with the prompt on screen: nothing is written,
    /// and the helper's connection is closed rather than left waiting.
    #[test]
    fn helper_prompt_then_cancel_closes_the_helper() {
        let (mut attempt, fake) = start(
            vec![Step::Say("PAM_PROMPT_ECHO_OFF Password: ")],
            SILENCE_LIMIT,
        );
        assert!(matches!(
            next(&mut attempt),
            Event::Said(Message::Prompt { .. })
        ));
        drop(attempt);
        let seen = fake.join().unwrap();
        assert!(seen.read.is_empty());
        assert!(seen.closed_by_agent);
    }

    /// Likewise while the helper is still busy with the reader.
    #[test]
    fn helper_cancel_while_waiting_on_the_reader_closes_the_helper() {
        let (mut attempt, fake) = start(vec![Step::Say(FINGER)], SILENCE_LIMIT);
        assert!(matches!(next(&mut attempt), Event::Said(Message::Info(_))));
        drop(attempt);
        assert!(fake.join().unwrap().closed_by_agent);
    }

    /// A helper that hangs up while a prompt is on screen ends the attempt
    /// at once, not when someone types.
    #[test]
    fn helper_hang_up_during_a_prompt_is_noticed() {
        let (mut attempt, fake) = start(
            vec![Step::Say("PAM_PROMPT_ECHO_OFF Password: "), Step::HangUp],
            SILENCE_LIMIT,
        );
        assert!(matches!(
            next(&mut attempt),
            Event::Said(Message::Prompt { .. })
        ));
        assert!(
            matches!(next(&mut attempt), Event::Ended(Outcome::Denied(reason)) if !reason.is_empty())
        );
        fake.join().unwrap();
    }

    /// A helper that goes quiet is closed after the silence limit.
    #[test]
    fn helper_silent_too_long_is_closed() {
        let (mut attempt, fake) = start(vec![Step::Say(FINGER)], Duration::from_millis(300));
        assert!(matches!(next(&mut attempt), Event::Said(Message::Info(_))));
        assert!(matches!(
            next(&mut attempt),
            Event::Ended(Outcome::Denied(_))
        ));
        assert!(fake.join().unwrap().closed_by_agent);
    }
}
