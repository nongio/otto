//! The terminal: raw mode, the alternate screen, size and key input.
//!
//! Hand-rolled over termios and ANSI escapes. The terminal is put back as it
//! was when [`Terminal`] is dropped, and also when the program panics.

// Rust guideline compliant 2026-02-21

use std::io::{self, Write as _};
use std::sync::{Mutex, Once};

/// The terminal settings before raw mode, for the panic hook to restore.
static ORIGINAL: Mutex<Option<libc::termios>> = Mutex::new(None);

/// Installs the panic hook once per process.
static PANIC_HOOK: Once = Once::new();

/// Enter the alternate screen, hide the cursor, stop wrapping long lines.
const ENTER: &str = "\x1b[?1049h\x1b[?25l\x1b[?7l";
/// Undo [`ENTER`], in reverse order.
const LEAVE: &str = "\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l";

/// A key the interface reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Tab.
    Tab,
    /// Shift+Tab.
    BackTab,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Return or Enter.
    Enter,
    /// Escape on its own.
    Esc,
    /// Ctrl+C, which raw mode delivers as a byte.
    Interrupt,
    /// A printable character.
    Char(char),
}

/// The terminal in raw mode on the alternate screen, restored on drop.
#[derive(Debug)]
pub struct Terminal {
    _private: (),
}

impl Terminal {
    /// Switches the terminal on stdin and stdout to raw mode and the
    /// alternate screen.
    ///
    /// # Errors
    ///
    /// Fails when stdin is not a terminal or its settings cannot be changed.
    pub fn enter() -> io::Result<Self> {
        // SAFETY: an all-zero termios is valid storage for tcgetattr to fill.
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fd 0 is stdin; `original` is live, writable storage.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::ISTRIP | libc::INPCK);
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG | libc::IEXTEN);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: fd 0 is stdin; `raw` is a valid termios copied from it.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if let Ok(mut saved) = ORIGINAL.lock() {
            *saved = Some(original);
        }
        PANIC_HOOK.call_once(|| {
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore();
                default_hook(info);
            }));
        });
        let mut out = io::stdout();
        out.write_all(ENTER.as_bytes())?;
        out.flush()?;
        Ok(Self { _private: () })
    }

    /// Replaces the screen with `frame`, which must start at the top left.
    ///
    /// # Errors
    ///
    /// Fails when stdout cannot be written.
    pub fn draw(&mut self, frame: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        out.write_all(frame.as_bytes())?;
        out.flush()
    }

    /// The keys pressed within `timeout_ms`, possibly none.
    ///
    /// # Errors
    ///
    /// Fails when stdin cannot be polled or read.
    pub fn keys(&mut self, timeout_ms: i32) -> io::Result<Vec<Key>> {
        if !wait_readable(timeout_ms)? {
            return Ok(Vec::new());
        }
        let mut bytes = read_available()?;
        // A lone Escape and the start of an escape sequence look alike; give
        // the rest of a sequence a moment to arrive.
        if bytes == [0x1b] && wait_readable(ESCAPE_WAIT_MS)? {
            bytes.extend(read_available()?);
        }
        Ok(parse_keys(&bytes))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

/// How long to wait for the rest of an escape sequence after an Escape.
///
/// Terminals send a sequence in one write, so the rest is normally already
/// there; this only covers slow links such as SSH.
const ESCAPE_WAIT_MS: i32 = 30;

/// Leaves the alternate screen and puts the terminal settings back.
fn restore() {
    let mut out = io::stdout();
    let _ = out.write_all(LEAVE.as_bytes());
    let _ = out.flush();
    let original = ORIGINAL.lock().ok().and_then(|mut saved| saved.take());
    if let Some(original) = original {
        // SAFETY: fd 0 is stdin; `original` came from tcgetattr on it.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &original) };
    }
}

/// Whether stdin has input within `timeout_ms`.
fn wait_readable(timeout_ms: i32) -> io::Result<bool> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one live pollfd, and the count passed is one.
    let ready = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
    match ready {
        -1 => {
            let err = io::Error::last_os_error();
            // A resize interrupts the wait; the caller redraws.
            if err.kind() == io::ErrorKind::Interrupted {
                Ok(false)
            } else {
                Err(err)
            }
        }
        0 => Ok(false),
        _ => Ok(true),
    }
}

/// Whatever stdin has ready, without blocking.
fn read_available() -> io::Result<Vec<u8>> {
    let mut buf = [0_u8; 256];
    // SAFETY: `buf` is live, writable and as long as the length passed.
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buf[..n.unsigned_abs()].to_vec())
}

/// The keys in a chunk of terminal input.
///
/// Unknown escape sequences are skipped whole.
fn parse_keys(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            let (key, len) = parse_escape(&bytes[i..]);
            keys.extend(key);
            i += len;
            continue;
        }
        let key = match b {
            b'\r' | b'\n' => Some(Key::Enter),
            0x03 => Some(Key::Interrupt),
            b'\t' => Some(Key::Tab),
            0x20..=0x7e => Some(Key::Char(char::from(b))),
            _ => None,
        };
        keys.extend(key);
        i += 1;
    }
    keys
}

/// The key an escape sequence at the start of `bytes` stands for, and its
/// length.
fn parse_escape(bytes: &[u8]) -> (Option<Key>, usize) {
    match bytes.get(1) {
        None => (Some(Key::Esc), 1),
        Some(b'[') | Some(b'O') => {
            // Parameters, then one final byte in '@'..='~'.
            let end = bytes[2..]
                .iter()
                .position(|b| (0x40..=0x7e).contains(b))
                .map(|p| p + 2);
            let Some(end) = end else {
                return (None, bytes.len());
            };
            let key = match &bytes[2..=end] {
                b"A" => Some(Key::Up),
                b"B" => Some(Key::Down),
                b"C" => Some(Key::Right),
                b"D" => Some(Key::Left),
                b"Z" => Some(Key::BackTab),
                b"H" | b"1~" | b"7~" => Some(Key::Home),
                b"F" | b"4~" | b"8~" => Some(Key::End),
                b"5~" => Some(Key::PageUp),
                b"6~" => Some(Key::PageDown),
                _ => None,
            };
            (key, end + 1)
        }
        // Escape followed by an ordinary byte: Alt+key, taken as Escape.
        Some(_) => (Some(Key::Esc), 1),
    }
}

/// The terminal's size in columns and rows; 80×24 when it cannot be read.
pub fn size() -> (usize, usize) {
    // SAFETY: an all-zero winsize is valid storage for the ioctl to fill.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize into the pointer it is given.
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
    if ok && ws.ws_col > 0 && ws.ws_row > 0 {
        (usize::from(ws.ws_col), usize::from(ws.ws_row))
    } else {
        (80, 24)
    }
}

/// Whether file descriptor `fd` is a terminal.
pub fn is_tty(fd: libc::c_int) -> bool {
    // SAFETY: isatty only inspects the descriptor.
    unsafe { libc::isatty(fd) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arrows_pages_and_characters() {
        let input = b"\x1b[A\x1b[B\x1bOA\x1b[5~\x1b[6~\x1b[H\x1b[4~j \r\x03";
        assert_eq!(
            parse_keys(input),
            [
                Key::Up,
                Key::Down,
                Key::Up,
                Key::PageUp,
                Key::PageDown,
                Key::Home,
                Key::End,
                Key::Char('j'),
                Key::Char(' '),
                Key::Enter,
                Key::Interrupt,
            ]
        );
    }

    #[test]
    fn a_lone_escape_is_escape_and_unknown_sequences_are_skipped() {
        assert_eq!(parse_keys(b"\x1b"), [Key::Esc]);
        assert_eq!(parse_keys(b"\x1b[1;5Cq"), [Key::Char('q')]);
    }

    #[test]
    fn parses_sideways_keys_and_tabs() {
        assert_eq!(
            parse_keys(b"\x1b[D\x1b[C\t\x1b[Z"),
            [Key::Left, Key::Right, Key::Tab, Key::BackTab]
        );
    }
}
