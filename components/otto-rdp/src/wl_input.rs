//! Wayland input injection for the RDP bridge.
//!
//! Connects to the compositor (WAYLAND_DISPLAY), locates the target output
//! by name (via xdg-output), and injects remote input through the same
//! protocols wlrctl uses:
//! - `zwlr_virtual_pointer_v1`, created *with the target output* so
//!   absolute motion maps into that output's geometry (Otto resolves the
//!   bound output server-side).
//! - `zwp_virtual_keyboard_v1`, with a default xkb keymap uploaded from
//!   libxkbcommon.
//!
//! Runs on its own thread: waits for commands from the RDP input handler
//! with a short timeout, then flushes the Wayland queue.

use std::os::fd::AsFd;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use wayland_client::{
    protocol::{wl_output, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

/// Commands from the RDP input handler, in output-local pixel coordinates.
#[derive(Debug)]
pub enum InputCommand {
    Move {
        x: u32,
        y: u32,
    },
    /// Relative motion (touchpad-mode mobile clients send this).
    MoveRel {
        dx: f64,
        dy: f64,
    },
    Button {
        button: u32,
        pressed: bool,
    },
    /// Wayland axis units (positive = down/right).
    Scroll {
        vertical: f64,
        horizontal: f64,
    },
    /// Evdev keycode (wl_keyboard semantics) press/release.
    Key {
        key: u32,
        pressed: bool,
    },
    /// A Unicode codepoint from an on-screen/mobile keyboard. Injected by
    /// swapping in a one-key keymap and tapping it (see `type_unicode`);
    /// `pressed == false` is ignored (the press already taps down+up).
    Unicode {
        c: u16,
        pressed: bool,
    },
}

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
pub const BTN_SIDE: u32 = 0x113;
pub const BTN_EXTRA: u32 = 0x114;

struct FoundOutput {
    output: wl_output::WlOutput,
    xdg: zxdg_output_v1::ZxdgOutputV1,
    name: Option<String>,
    mode_size: Option<(u32, u32)>,
}

struct State {
    seat: Option<wl_seat::WlSeat>,
    vp_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    vk_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    xdg_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    outputs: Vec<FoundOutput>,
    pending_outputs: Vec<wl_output::WlOutput>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    state.seat = Some(registry.bind(name, version.min(5), qh, ()));
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.vp_manager = Some(registry.bind(name, version.min(2), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.vk_manager = Some(registry.bind(name, 1, qh, ()));
                }
                "zxdg_output_manager_v1" => {
                    state.xdg_manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                "wl_output" => {
                    let output: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                    state.pending_outputs.push(output);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode {
            flags,
            width,
            height,
            ..
        } = event
        {
            if flags
                .into_result()
                .map(|f| f.contains(wl_output::Mode::Current))
                .unwrap_or(false)
            {
                if let Some(found) = state.outputs.iter_mut().find(|o| &o.output == output) {
                    found.mode_size = Some((width as u32, height as u32));
                }
            }
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ()> for State {
    fn event(
        state: &mut Self,
        xdg: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_output_v1::Event::Name { name } = event {
            if let Some(found) = state.outputs.iter_mut().find(|o| &o.xdg == xdg) {
                found.name = Some(name);
            }
        }
    }
}

macro_rules! noop_dispatch {
    ($($iface:ty),+ $(,)?) => {
        $(impl Dispatch<$iface, ()> for State {
            fn event(
                _: &mut Self,
                _: &$iface,
                _: <$iface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        })+
    };
}

noop_dispatch!(
    wl_seat::WlSeat,
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
);

/// Run the input loop. Reports the target output's pixel size on `size_tx`
/// once discovered, then services `rx` until the channel closes.
pub fn run(
    output_name: &str,
    rx: Receiver<InputCommand>,
    size_tx: Sender<(u32, u32)>,
) -> anyhow::Result<()> {
    // The bridge is launched as soon as Otto's D-Bus service appears, which can
    // be a moment before its Wayland socket is accepting connections. Retry for
    // a few seconds instead of exiting on the first failure — a hard exit here
    // tears the whole bridge down (and run-rdp.sh then stops Otto).
    let conn = {
        let mut attempt = 0u32;
        loop {
            match Connection::connect_to_env() {
                Ok(conn) => break conn,
                Err(e) => {
                    attempt += 1;
                    if attempt >= 25 {
                        return Err(anyhow::anyhow!(
                            "could not connect to the Wayland compositor \
                             (WAYLAND_DISPLAY={:?}) after {attempt} attempts: {e}",
                            std::env::var("WAYLAND_DISPLAY").ok()
                        ));
                    }
                    if attempt == 1 {
                        tracing::info!("waiting for Otto's Wayland socket…");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    };
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());

    let mut state = State {
        seat: None,
        vp_manager: None,
        vk_manager: None,
        xdg_manager: None,
        outputs: Vec::new(),
        pending_outputs: Vec::new(),
    };

    // First roundtrip: globals. Second: output events after xdg-output setup.
    queue.roundtrip(&mut state)?;

    let xdg_manager = state
        .xdg_manager
        .clone()
        .ok_or_else(|| anyhow::anyhow!("compositor lacks zxdg_output_manager_v1"))?;
    for output in std::mem::take(&mut state.pending_outputs) {
        let xdg = xdg_manager.get_xdg_output(&output, &qh, ());
        state.outputs.push(FoundOutput {
            output,
            xdg,
            name: None,
            mode_size: None,
        });
    }
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;

    let target = state
        .outputs
        .iter()
        .find(|o| o.name.as_deref() == Some(output_name))
        .ok_or_else(|| {
            let names: Vec<_> = state
                .outputs
                .iter()
                .filter_map(|o| o.name.clone())
                .collect();
            anyhow::anyhow!("output '{output_name}' not found (available: {names:?})")
        })?;
    let size = target
        .mode_size
        .ok_or_else(|| anyhow::anyhow!("output '{output_name}' has no current mode"))?;
    let target_output = target.output.clone();
    tracing::info!("target output '{output_name}': {}x{}", size.0, size.1);
    let _ = size_tx.send(size);

    let seat = state
        .seat
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no wl_seat"))?;
    let vp_manager = state
        .vp_manager
        .clone()
        .ok_or_else(|| anyhow::anyhow!("compositor lacks zwlr_virtual_pointer_manager_v1"))?;
    let vk_manager = state
        .vk_manager
        .clone()
        .ok_or_else(|| anyhow::anyhow!("compositor lacks zwp_virtual_keyboard_manager_v1"))?;

    // Pointer bound to the target output: absolute coords map into it.
    let pointer =
        vp_manager.create_virtual_pointer_with_output(Some(&seat), Some(&target_output), &qh, ());

    // Keyboard with a default xkb keymap (RMLVO from the environment).
    let keyboard = vk_manager.create_virtual_keyboard(&seat, &qh, ());
    upload_keymap(&keyboard)?;
    queue.roundtrip(&mut state)?;

    let start = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(cmd) => {
                handle_command(&pointer, &keyboard, size, start, cmd);
                // Drain whatever queued up behind this command before flushing.
                while let Ok(cmd) = rx.try_recv() {
                    handle_command(&pointer, &keyboard, size, start, cmd);
                }
                queue.flush()?;
                // Non-blocking dispatch of any compositor replies.
                if let Some(guard) = queue.prepare_read() {
                    let _ = guard.read();
                }
                queue.dispatch_pending(&mut state)?;
            }
            Err(RecvTimeoutError::Timeout) => {
                queue.flush()?;
                if let Some(guard) = queue.prepare_read() {
                    let _ = guard.read();
                }
                queue.dispatch_pending(&mut state)?;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

/// Apply one input command to the virtual pointer / keyboard. Does not flush
/// the Wayland queue — the caller batches a flush after draining commands.
fn handle_command(
    pointer: &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    keyboard: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    size: (u32, u32),
    start: Instant,
    cmd: InputCommand,
) {
    use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
    let time = start.elapsed().as_millis() as u32;
    match cmd {
        InputCommand::Move { x, y } => {
            pointer.motion_absolute(time, x, y, size.0.max(1), size.1.max(1));
            pointer.frame();
        }
        InputCommand::MoveRel { dx, dy } => {
            pointer.motion(time, dx, dy);
            pointer.frame();
        }
        InputCommand::Button { button, pressed } => {
            let st = if pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            pointer.button(time, button, st);
            pointer.frame();
        }
        InputCommand::Scroll {
            vertical,
            horizontal,
        } => {
            pointer.axis_source(AxisSource::Wheel);
            if vertical != 0.0 {
                pointer.axis(time, Axis::VerticalScroll, vertical);
            }
            if horizontal != 0.0 {
                pointer.axis(time, Axis::HorizontalScroll, horizontal);
            }
            pointer.frame();
        }
        InputCommand::Key { key, pressed } => {
            keyboard.key(time, key, if pressed { 1 } else { 0 });
        }
        InputCommand::Unicode { c, pressed } => {
            // Only act on the press: type_char taps down+up itself.
            if pressed {
                type_char(keyboard, time, c);
            }
        }
    }
}

const KEY_LEFTSHIFT: u32 = 42;

/// Inject a single Unicode codepoint by tapping the matching key on the
/// standard US keymap (uploaded once at startup). For ASCII this is race-free
/// — unlike swapping in a per-character keymap, which the client may not apply
/// before interpreting the key. Non-ASCII characters have no US keycode and
/// are dropped (a follow-up could add a compose/keymap path).
fn type_char(keyboard: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1, time: u32, c: u16) {
    let Some(ch) = char::from_u32(c as u32) else {
        return;
    };
    let Some((keycode, shift)) = ascii_to_keycode(ch) else {
        tracing::debug!("no US keycode for U+{c:04X} ({ch:?}) — dropped");
        return;
    };
    if shift {
        keyboard.key(time, KEY_LEFTSHIFT, 1);
    }
    keyboard.key(time, keycode, 1);
    keyboard.key(time, keycode, 0);
    if shift {
        keyboard.key(time, KEY_LEFTSHIFT, 0);
    }
}

/// Map a printable ASCII character to its US-QWERTY evdev keycode and whether
/// Shift is required. Returns `None` for anything without a US key.
fn ascii_to_keycode(ch: char) -> Option<(u32, bool)> {
    // Unshifted keycodes for letters, digits, and symbols.
    let base = |c: char| -> Option<u32> {
        Some(match c {
            'a' => 30,
            'b' => 48,
            'c' => 46,
            'd' => 32,
            'e' => 18,
            'f' => 33,
            'g' => 34,
            'h' => 35,
            'i' => 23,
            'j' => 36,
            'k' => 37,
            'l' => 38,
            'm' => 50,
            'n' => 49,
            'o' => 24,
            'p' => 25,
            'q' => 16,
            'r' => 19,
            's' => 31,
            't' => 20,
            'u' => 22,
            'v' => 47,
            'w' => 17,
            'x' => 45,
            'y' => 21,
            'z' => 44,
            '1' => 2,
            '2' => 3,
            '3' => 4,
            '4' => 5,
            '5' => 6,
            '6' => 7,
            '7' => 8,
            '8' => 9,
            '9' => 10,
            '0' => 11,
            ' ' => 57,
            '\t' => 15,
            '\n' | '\r' => 28,
            '`' => 41,
            '-' => 12,
            '=' => 13,
            '[' => 26,
            ']' => 27,
            '\\' => 43,
            ';' => 39,
            '\'' => 40,
            ',' => 51,
            '.' => 52,
            '/' => 53,
            _ => return None,
        })
    };
    if let Some(k) = base(ch) {
        return Some((k, false));
    }
    // Uppercase letters: Shift + the lowercase key.
    if ch.is_ascii_uppercase() {
        return base(ch.to_ascii_lowercase()).map(|k| (k, true));
    }
    // Shifted symbols share their unshifted key.
    let (unshifted, _) = match ch {
        '!' => ('1', ()),
        '@' => ('2', ()),
        '#' => ('3', ()),
        '$' => ('4', ()),
        '%' => ('5', ()),
        '^' => ('6', ()),
        '&' => ('7', ()),
        '*' => ('8', ()),
        '(' => ('9', ()),
        ')' => ('0', ()),
        '~' => ('`', ()),
        '_' => ('-', ()),
        '+' => ('=', ()),
        '{' => ('[', ()),
        '}' => (']', ()),
        '|' => ('\\', ()),
        ':' => (';', ()),
        '"' => ('\'', ()),
        '<' => (',', ()),
        '>' => ('.', ()),
        '?' => ('/', ()),
        _ => return None,
    };
    base(unshifted).map(|k| (k, true))
}

/// Build a default xkb keymap and upload it to the virtual keyboard.
fn upload_keymap(keyboard: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1) -> anyhow::Result<()> {
    use xkbcommon::xkb;
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    // Force US QWERTY so the evdev keycodes in `ascii_to_keycode` land on the
    // characters they name regardless of the host's configured layout.
    let keymap = xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::COMPILE_NO_FLAGS)
        .ok_or_else(|| anyhow::anyhow!("failed to compile default xkb keymap"))?;
    let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    upload_keymap_str(keyboard, &text)
}

/// Upload a raw xkb keymap string to the virtual keyboard via a memfd.
fn upload_keymap_str(
    keyboard: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    text: &str,
) -> anyhow::Result<()> {
    let mfd = memfd::MemfdOptions::default()
        .close_on_exec(true)
        .create("otto-rdp-keymap")?;
    use std::io::Write;
    let mut f = mfd.as_file();
    f.write_all(text.as_bytes())?;
    f.write_all(&[0])?; // NUL terminator, as wl_keyboard keymaps expect
    let len = text.len() as u32 + 1;

    keyboard.keymap(
        wayland_client::protocol::wl_keyboard::KeymapFormat::XkbV1 as u32,
        mfd.as_file().as_fd(),
        len,
    );
    Ok(())
}

/// Translate an RDP (PC/AT set-1) scancode to an evdev keycode.
///
/// Base (non-extended) set-1 scancodes are numerically identical to evdev
/// keycodes. Extended (0xE0-prefixed) scancodes need a lookup.
pub fn scancode_to_evdev(code: u8, extended: bool) -> Option<u32> {
    if !extended {
        return Some(code as u32);
    }
    let key = match code {
        0x1C => 96,  // KP Enter
        0x1D => 97,  // Right Ctrl
        0x35 => 98,  // KP /
        0x37 => 99,  // PrintScreen / SysRq
        0x38 => 100, // Right Alt (AltGr)
        0x47 => 102, // Home
        0x48 => 103, // Up
        0x49 => 104, // PageUp
        0x4B => 105, // Left
        0x4D => 106, // Right
        0x4F => 107, // End
        0x50 => 108, // Down
        0x51 => 109, // PageDown
        0x52 => 110, // Insert
        0x53 => 111, // Delete
        0x5B => 125, // Left Super
        0x5C => 126, // Right Super
        0x5D => 127, // Menu
        _ => return None,
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::scancode_to_evdev;

    /// Set-1 scancodes without the 0xE0 prefix are already evdev codes.
    #[test]
    fn base_scancodes_pass_through() {
        assert_eq!(scancode_to_evdev(0x1E, false), Some(30)); // KEY_A
        assert_eq!(scancode_to_evdev(0x48, false), Some(72)); // KEY_KP8
    }

    /// The same byte means something else behind the 0xE0 prefix — keypad 8
    /// versus the arrow key above it.
    #[test]
    fn extended_scancodes_are_remapped() {
        assert_eq!(scancode_to_evdev(0x48, true), Some(103)); // KEY_UP
        assert_eq!(scancode_to_evdev(0x1D, true), Some(97)); // KEY_RIGHTCTRL
        assert_eq!(scancode_to_evdev(0x5B, true), Some(125)); // KEY_LEFTMETA
    }

    /// An extended scancode with no evdev equivalent is dropped rather than
    /// injected as some unrelated key.
    #[test]
    fn unknown_extended_scancodes_are_dropped() {
        assert_eq!(scancode_to_evdev(0x7F, true), None);
    }
}
