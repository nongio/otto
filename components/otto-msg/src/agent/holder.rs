//! The process that holds an agent's seat for `otto-msg agent`.
//!
//! A seat lasts as long as the bus connection that asked for it, and each
//! `otto-msg` run is a process of its own, so `agent start` leaves this one
//! behind. It asks Otto for the seat and the workspace, opens the agent's own
//! Wayland connection (`ConnectAgent`), and keeps one virtual pointer and one
//! virtual keyboard on it for as long as it lives. The other `otto-msg agent`
//! commands reach it over a socket only the user can open. It ends on
//! `stop`, or when Otto takes the seat away (Stop on the frame, the secure
//! attention key, consent withdrawn in Settings).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::wp::security_context::v1::client::{
    wp_security_context_manager_v1::WpSecurityContextManagerV1,
    wp_security_context_v1::WpSecurityContextV1,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use zbus::blocking::{Connection as Bus, Proxy as BusProxy};

use super::keys::{self, Layout};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// What `agent start` asked for.
pub struct Start {
    pub name: String,
    /// `Some("")` for the workspace the user is looking at.
    pub lend: Option<String>,
    pub socket: PathBuf,
}

/// Run the holder. Its first line on stdout is `ready <json>` or
/// `error <message>`, for `agent start` to pass on; after that it writes
/// only to its log.
pub fn serve(start: Start) -> std::process::ExitCode {
    let mut holder = match Holder::new(&start) {
        Ok(holder) => holder,
        Err(message) => {
            println!("error {message}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let listener = match bind(&start.socket) {
        Ok(listener) => listener,
        Err(message) => {
            let _ = holder.release();
            println!("error {message}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!("ready {}", holder.describe(&start.socket));
    let _ = std::io::stdout().flush();
    // Nobody reads stdout from here on; a write would fail.
    redirect_stdout_to_null();

    let ended = holder.run(&listener);
    let _ = std::fs::remove_file(&start.socket);
    let _ = std::fs::remove_file(&holder.display);
    eprintln!("otto-msg agent: {ended}");
    std::process::ExitCode::SUCCESS
}

fn bind(path: &Path) -> Result<UnixListener, String> {
    if UnixStream::connect(path).is_ok() {
        return Err(format!("an agent is already running at {}", path.display()));
    }
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).map_err(|err| format!("cannot listen at {}: {err}", path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("cannot listen: {err}"))?;
    Ok(listener)
}

fn redirect_stdout_to_null() {
    if let Ok(null) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
        // SAFETY: both descriptors are open; dup2 replaces stdout atomically.
        unsafe {
            libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO);
        }
    }
}

/// A window on the agent's workspace, as the window list tells it.
#[derive(Default, Clone)]
struct Window {
    title: String,
    app_id: String,
}

/// The agent's side of its Wayland connection.
#[derive(Default)]
struct Wayland {
    /// Every seat offered, by registry name: the agent's and the user's.
    seats: Vec<(u32, wl_seat::WlSeat, Option<String>)>,
    pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    toplevel_manager: Option<ZwlrForeignToplevelManagerV1>,
    security_contexts: Option<WpSecurityContextManagerV1>,
    /// The registry and the window list's global, to open the list anew.
    registry: Option<wl_registry::WlRegistry>,
    toplevel_global: Option<(u32, u32)>,
    windows: HashMap<ObjectId, (ZwlrForeignToplevelHandleV1, Window)>,
    /// The registry name of the agent's seat, once known.
    agent_seat_global: Option<u32>,
    /// Set when Otto withdrew the agent's seat.
    seat_gone: bool,
}

struct Holder {
    bus: Bus,
    seat_name: String,
    color: String,
    output: String,
    /// The workspace in logical pixels, and the output's scale.
    size: (i32, i32),
    scale: f64,
    lent: bool,
    /// The pixel size of a capture, which pointer coordinates address.
    pixels: (u32, u32),
    _connection: Connection,
    queue: EventQueue<Wayland>,
    qh: QueueHandle<Wayland>,
    state: Wayland,
    seat: wl_seat::WlSeat,
    pointer: ZwlrVirtualPointerV1,
    keyboard: Option<ZwpVirtualKeyboardV1>,
    layout: Layout,
    started: Instant,
    /// The agent's own Wayland display: every client that connects there
    /// is the agent's. Programs it launches use it.
    display: PathBuf,
    /// Held open for as long as the holder lives; Otto stops accepting on
    /// the display when it closes.
    _display_alive: std::os::fd::OwnedFd,
    /// Programs it launched, reaped as they exit.
    children: Vec<std::process::Child>,
    /// When the agent last sent input, so a capture waits for apps to draw
    /// what it did.
    last_input: Option<Instant>,
}

impl Holder {
    fn new(start: &Start) -> Result<Self, String> {
        let bus = Bus::session().map_err(|err| format!("cannot reach the session bus: {err}"))?;
        let compositor = compositor(&bus)?;
        let (seat_name, color): (String, String) = compositor
            .call("RequestAgentSeat", &(start.name.as_str(),))
            .map_err(|err| reason(&err))?;
        let workspace: (String, i32, i32, i32, i32, f64) = match &start.lend {
            Some(name) => compositor.call("RequestWorkspace", &(name.as_str(),)),
            None => compositor.call("RequestOwnWorkspace", &()),
        }
        .map_err(|err| reason(&err))?;
        let (output, _x, _y, width, height, scale) = workspace;

        let fd: zbus::zvariant::OwnedFd = compositor
            .call("ConnectAgent", &())
            .map_err(|err| reason(&err))?;
        let fd: std::os::fd::OwnedFd = fd.into();
        let stream = UnixStream::from(fd);
        let connection = Connection::from_socket(stream)
            .map_err(|err| format!("cannot open the agent's connection: {err}"))?;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = Wayland::default();
        // Globals, then the seats' names.
        for _ in 0..2 {
            queue
                .roundtrip(&mut state)
                .map_err(|err| format!("the agent's connection failed: {err}"))?;
        }
        let (global, seat) = state
            .seats
            .iter()
            .find(|(_, _, name)| name.as_deref() == Some(seat_name.as_str()))
            .map(|(global, seat, _)| (*global, seat.clone()))
            .ok_or("Otto did not offer the agent's seat")?;
        state.agent_seat_global = Some(global);
        let pointer = state
            .pointer_manager
            .as_ref()
            .ok_or("Otto offered no virtual pointer")?
            .create_virtual_pointer(Some(&seat), &qh, ());
        if state.keyboard_manager.is_none() {
            return Err("Otto offered no virtual keyboard".to_string());
        }
        let (display, display_alive) = open_display(&mut queue, &mut state, &qh, &start.socket)?;
        let pixels = (
            (width as f64 * scale).round().max(1.0) as u32,
            (height as f64 * scale).round().max(1.0) as u32,
        );
        Ok(Self {
            bus,
            seat_name,
            color,
            output,
            size: (width, height),
            scale,
            lent: start.lend.is_some(),
            pixels,
            _connection: connection,
            queue,
            qh,
            state,
            seat,
            pointer,
            keyboard: None,
            layout: Layout::default(),
            started: Instant::now(),
            display,
            _display_alive: display_alive,
            children: Vec::new(),
            last_input: None,
        })
    }

    fn describe(&self, socket: &Path) -> Value {
        json!({
            "seat": self.seat_name,
            "color": self.color,
            "output": self.output,
            "lent": self.lent,
            "logical": [self.size.0, self.size.1],
            "scale": self.scale,
            "pixels": [self.pixels.0, self.pixels.1],
            "socket": socket.display().to_string(),
            "display": self.display.display().to_string(),
        })
    }

    /// Serve requests until `stop`, or until Otto withdraws the seat or the
    /// connection breaks. Returns why it ended.
    fn run(&mut self, listener: &UnixListener) -> String {
        loop {
            if self.state.seat_gone {
                return "Otto ended the agent's seat".to_string();
            }
            self.children
                .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
            if let Err(err) = self.queue.dispatch_pending(&mut self.state) {
                return format!("the agent's connection failed: {err}");
            }
            if let Err(err) = self.queue.flush() {
                return format!("the agent's connection failed: {err}");
            }
            let guard = self.queue.prepare_read();
            let mut fds = [
                libc::pollfd {
                    fd: listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: guard
                        .as_ref()
                        .map(|guard| guard.connection_fd().as_raw_fd())
                        .unwrap_or(-1),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: `fds` is a valid array of two pollfd for the call.
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, 1000) };
            if ready < 0 {
                continue;
            }
            if let Some(guard) = guard {
                if fds[1].revents & libc::POLLIN != 0 {
                    if let Err(err) = guard.read() {
                        return format!("the agent's connection failed: {err}");
                    }
                } else if fds[1].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                    return "the agent's connection closed".to_string();
                }
            }
            if fds[0].revents & libc::POLLIN != 0 {
                if let Ok((stream, _)) = listener.accept() {
                    if self.answer(stream) {
                        return "stopped".to_string();
                    }
                }
            }
        }
    }

    /// Answer one request on `stream`. Returns whether the agent stopped.
    fn answer(&mut self, stream: UnixStream) -> bool {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            return false;
        }
        let request: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let command = request["cmd"].as_str().unwrap_or("").to_string();
        let args: Vec<String> = request["args"]
            .as_array()
            .map(|args| {
                args.iter()
                    .filter_map(|arg| arg.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let stop = command == "stop";
        let reply = match self.handle(&command, &args) {
            Ok(out) => json!({"ok": true, "out": out}),
            Err(error) => json!({"ok": false, "error": error}),
        };
        let mut stream = stream;
        let _ = writeln!(stream, "{reply}");
        stop
    }

    fn handle(&mut self, command: &str, args: &[String]) -> Result<String, String> {
        if self.state.seat_gone {
            return Err("Otto ended the agent's seat".to_string());
        }
        match command {
            "info" => Ok(self.info()),
            "capture" => self.capture(),
            "move" => {
                let (x, y) = self.point(args, 0)?;
                self.move_to(x, y);
                self.sync()
            }
            "click" => {
                let (x, y) = self.point(args, 0)?;
                let button = match args.get(2).map(String::as_str) {
                    None | Some("left") => BTN_LEFT,
                    Some("right") => BTN_RIGHT,
                    Some("middle") => BTN_MIDDLE,
                    Some(other) => return Err(format!("unknown button '{other}'")),
                };
                self.move_to(x, y);
                self.press(button);
                self.sync()
            }
            "double-click" => {
                let (x, y) = self.point(args, 0)?;
                self.move_to(x, y);
                self.press(BTN_LEFT);
                self.press(BTN_LEFT);
                self.sync()
            }
            "drag" => {
                let (x, y) = self.point(args, 0)?;
                let (to_x, to_y) = self.point(args, 2)?;
                self.drag((x, y), (to_x, to_y));
                self.sync()
            }
            "scroll" => {
                let (x, y) = self.point(args, 0)?;
                let direction = match args.get(2).map(String::as_str) {
                    Some("up") => -1.0,
                    Some("down") => 1.0,
                    _ => return Err("scroll needs up or down".to_string()),
                };
                let steps: u32 = match args.get(3) {
                    Some(steps) => steps.parse().map_err(|_| "steps must be a number")?,
                    None => 3,
                };
                self.move_to(x, y);
                self.scroll(direction, steps);
                self.sync()
            }
            "type" => {
                let text = args.join(" ");
                if text.is_empty() {
                    return Err("nothing to type".to_string());
                }
                self.type_text(&text)?;
                self.sync()
            }
            "key" => {
                if args.is_empty() {
                    return Err("key needs a key, such as Return or ctrl+s".to_string());
                }
                for combo in args {
                    let combo = keys::parse_combo(combo)?;
                    self.press_combo(&combo);
                }
                self.sync()
            }
            "windows" => Ok(self.list_windows()),
            "focus" => {
                let handle = self.find_window(args)?;
                handle.activate(&self.seat);
                self.sync()
            }
            "close" => {
                let handle = self.find_window(args)?;
                handle.close();
                self.sync()
            }
            "launch" => {
                if args.is_empty() {
                    return Err("launch needs a program".to_string());
                }
                let pid = self.launch(args)?;
                Ok(format!("started {} (pid {pid})", args[0]))
            }
            "stop" => {
                self.release()?;
                Ok("stopped: the workspace and its windows are the user's".to_string())
            }
            "" => Err("no command".to_string()),
            other => Err(format!("unknown command '{other}'")),
        }
    }

    /// Start `argv` on the agent's display, detached from the holder.
    fn launch(&mut self, argv: &[String]) -> Result<u32, String> {
        let mut command = std::process::Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .env("WAYLAND_DISPLAY", &self.display)
            .env_remove("WAYLAND_SOCKET")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut command, || {
                libc::setsid();
                Ok(())
            });
        }
        let child = command
            .spawn()
            .map_err(|err| format!("cannot start {}: {err}", argv[0]))?;
        let pid = child.id();
        self.children.push(child);
        Ok(pid)
    }

    fn info(&self) -> String {
        format!(
            "seat {} ({}) on {}, {}\nscreen {}x{} pixels (scale {}); click, move, drag and scroll take these coordinates\nWAYLAND_DISPLAY={} runs any program as yours",
            self.seat_name,
            self.color,
            self.output,
            if self.lent {
                "a workspace the user lent"
            } else {
                "a workspace of its own"
            },
            self.pixels.0,
            self.pixels.1,
            self.scale,
            self.display.display()
        )
    }

    fn release(&self) -> Result<(), String> {
        let _: bool = compositor(&self.bus)?
            .call("ReleaseAgentSeat", &())
            .map_err(|err| reason(&err))?;
        Ok(())
    }

    fn capture(&mut self) -> Result<String, String> {
        // Apps draw what the last input did a frame or more later.
        const SETTLE: Duration = Duration::from_millis(400);
        if let Some(elapsed) = self.last_input.map(|at| at.elapsed()) {
            if elapsed < SETTLE {
                std::thread::sleep(SETTLE - elapsed);
            }
        }
        let path: String = compositor(&self.bus)?
            .call("CaptureWorkspace", &("",))
            .map_err(|err| reason(&err))?;
        if let Some(size) = png_size(Path::new(&path)) {
            self.pixels = size;
        }
        Ok(format!(
            "{path}\n{}x{} pixels; click, move, drag and scroll take these coordinates",
            self.pixels.0, self.pixels.1
        ))
    }

    /// The point at `args[at]`, `args[at + 1]`, in capture pixels.
    fn point(&self, args: &[String], at: usize) -> Result<(u32, u32), String> {
        let coordinate = |index: usize, limit: u32| -> Result<u32, String> {
            let text = args
                .get(index)
                .ok_or("needs x and y, in the pixels of a capture")?;
            let value: f64 = text
                .parse()
                .map_err(|_| format!("'{text}' is not a coordinate"))?;
            if value < 0.0 || value >= limit as f64 {
                return Err(format!(
                    "{value} is off the screen, which is {}x{} pixels",
                    self.pixels.0, self.pixels.1
                ));
            }
            Ok(value.round() as u32)
        };
        Ok((coordinate(at, self.pixels.0)?, coordinate(at + 1, self.pixels.1)?))
    }

    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn move_to(&self, x: u32, y: u32) {
        self.pointer
            .motion_absolute(self.time(), x, y, self.pixels.0, self.pixels.1);
        self.pointer.frame();
    }

    fn press(&self, button: u32) {
        self.pointer
            .button(self.time(), button, wl_pointer::ButtonState::Pressed);
        self.pointer.frame();
        self.pointer
            .button(self.time(), button, wl_pointer::ButtonState::Released);
        self.pointer.frame();
    }

    fn drag(&mut self, from: (u32, u32), to: (u32, u32)) {
        self.move_to(from.0, from.1);
        self.pointer
            .button(self.time(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
        self.pointer.frame();
        // In steps, so the app sees the motion and not only the end.
        let steps = 10;
        for step in 1..=steps {
            let along = |a: u32, b: u32| {
                (a as f64 + (b as f64 - a as f64) * step as f64 / steps as f64).round() as u32
            };
            self.move_to(along(from.0, to.0), along(from.1, to.1));
            let _ = self.queue.flush();
            std::thread::sleep(Duration::from_millis(10));
        }
        self.pointer
            .button(self.time(), BTN_LEFT, wl_pointer::ButtonState::Released);
        self.pointer.frame();
    }

    fn scroll(&self, direction: f64, steps: u32) {
        for _ in 0..steps {
            let time = self.time();
            self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
            self.pointer
                .axis(time, wl_pointer::Axis::VerticalScroll, 15.0 * direction);
            self.pointer.axis_discrete(
                time,
                wl_pointer::Axis::VerticalScroll,
                15.0 * direction,
                direction as i32,
            );
            self.pointer.frame();
        }
    }

    /// The agent's keyboard, created with the first keymap.
    fn keyboard(&mut self) -> Result<ZwpVirtualKeyboardV1, String> {
        if let Some(keyboard) = &self.keyboard {
            return Ok(keyboard.clone());
        }
        let manager = self
            .state
            .keyboard_manager
            .as_ref()
            .ok_or("Otto offered no virtual keyboard")?;
        let keyboard = manager.create_virtual_keyboard(&self.seat, &self.qh, ());
        self.keyboard = Some(keyboard.clone());
        self.upload_keymap()?;
        Ok(keyboard)
    }

    fn upload_keymap(&self) -> Result<(), String> {
        let Some(keyboard) = &self.keyboard else {
            return Ok(());
        };
        let mut text = self.layout.keymap().into_bytes();
        text.push(0);
        // SAFETY: a fresh memfd with a static name; the result is checked.
        let fd = unsafe { libc::memfd_create(c"otto-agent-keymap".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err("cannot make the keymap".to_string());
        }
        // SAFETY: `fd` was just created and is owned here alone.
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(&text)
            .map_err(|err| format!("cannot write the keymap: {err}"))?;
        // 1 is WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1.
        keyboard.keymap(1, file.as_fd(), text.len() as u32);
        Ok(())
    }

    fn key(&mut self, keysym: xkbcommon::xkb::Keysym) -> Result<keys::Key, String> {
        let (key, changed) = self.layout.key_for(keysym);
        if changed {
            self.upload_keymap()?;
        }
        Ok(key)
    }

    fn tap(&self, keyboard: &ZwpVirtualKeyboardV1, key: keys::Key) {
        keyboard.key(self.time(), key, 1);
        keyboard.key(self.time(), key, 0);
    }

    fn type_text(&mut self, text: &str) -> Result<(), String> {
        let keyboard = self.keyboard()?;
        for c in text.chars() {
            let keysym = keys::keysym_for_char(c)
                .ok_or_else(|| format!("cannot type '{}'", c.escape_default()))?;
            let key = self.key(keysym)?;
            self.tap(&keyboard, key);
            let _ = self.queue.flush();
        }
        Ok(())
    }

    fn press_combo(&mut self, combo: &keys::Combo) {
        let Ok(keyboard) = self.keyboard() else {
            return;
        };
        let Ok(key) = self.key(combo.keysym) else {
            return;
        };
        for bit in keys::modifier_bits(combo.modifiers) {
            if let Some(modifier) = Layout::modifier_key(bit) {
                keyboard.key(self.time(), modifier, 1);
            }
        }
        keyboard.modifiers(combo.modifiers, 0, 0, 0);
        self.tap(&keyboard, key);
        for bit in keys::modifier_bits(combo.modifiers) {
            if let Some(modifier) = Layout::modifier_key(bit) {
                keyboard.key(self.time(), modifier, 0);
            }
        }
        keyboard.modifiers(0, 0, 0, 0);
    }

    /// Wait until Otto has taken everything sent so far.
    fn sync(&mut self) -> Result<String, String> {
        self.last_input = Some(Instant::now());
        self.queue
            .roundtrip(&mut self.state)
            .map_err(|err| format!("the agent's connection failed: {err}"))?;
        if self.state.seat_gone {
            return Err("Otto ended the agent's seat".to_string());
        }
        Ok("done".to_string())
    }

    fn windows(&self) -> Vec<(ZwlrForeignToplevelHandleV1, Window)> {
        let mut windows: Vec<_> = self.state.windows.values().cloned().collect();
        windows.sort_by(|a, b| a.1.title.cmp(&b.1.title));
        windows
    }

    /// Open the window list anew: Otto lists the windows on the workspace
    /// when the list is opened, and later only those the agent's own
    /// connection opens, not those a launch started.
    fn refresh_windows(&mut self) {
        if let (Some(registry), Some((name, version))) =
            (&self.state.registry, self.state.toplevel_global)
        {
            if let Some(old) = self.state.toplevel_manager.take() {
                old.stop();
            }
            for (handle, _) in self.state.windows.values() {
                handle.destroy();
            }
            self.state.windows.clear();
            self.state.toplevel_manager = Some(registry.bind(name, version, &self.qh, ()));
        }
        let _ = self.queue.roundtrip(&mut self.state);
    }

    fn list_windows(&mut self) -> String {
        self.refresh_windows();
        let windows = self.windows();
        if windows.is_empty() {
            return "no windows on the agent's workspace".to_string();
        }
        windows
            .iter()
            .map(|(_, window)| format!("{}  [{}]", window.title, window.app_id))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The one window whose title or app id holds the words in `args`.
    fn find_window(&mut self, args: &[String]) -> Result<ZwlrForeignToplevelHandleV1, String> {
        let wanted = args.join(" ").to_lowercase();
        if wanted.is_empty() {
            return Err("name the window: part of its title or app id".to_string());
        }
        self.refresh_windows();
        let windows = self.windows();
        if let Some((handle, _)) = windows
            .iter()
            .find(|(_, window)| window.title.to_lowercase() == wanted)
        {
            return Ok(handle.clone());
        }
        let matches: Vec<_> = windows
            .iter()
            .filter(|(_, window)| {
                window.title.to_lowercase().contains(&wanted)
                    || window.app_id.to_lowercase().contains(&wanted)
            })
            .collect();
        match matches.as_slice() {
            [(handle, _)] => Ok(handle.clone()),
            [] => Err(format!(
                "no window matches '{wanted}'; `otto-msg agent windows` lists them"
            )),
            many => Err(format!(
                "several windows match '{wanted}':\n{}",
                many.iter()
                    .map(|(_, window)| format!("{}  [{}]", window.title, window.app_id))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
        }
    }
}

/// Make the agent's own display beside its control socket: a security
/// context listener on the agent's connection, so Otto takes every client
/// that connects there as the agent's.
fn open_display(
    queue: &mut EventQueue<Wayland>,
    state: &mut Wayland,
    qh: &QueueHandle<Wayland>,
    socket: &Path,
) -> Result<(PathBuf, std::os::fd::OwnedFd), String> {
    let manager = state
        .security_contexts
        .clone()
        .ok_or("Otto offered no security contexts to the agent")?;
    let path = socket.with_extension("wayland");
    let _ = std::fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).map_err(|err| format!("cannot make the agent's display: {err}"))?;
    let (alive_read, alive_write) =
        std::io::pipe().map_err(|err| format!("cannot make the agent's display: {err}"))?;
    let context: WpSecurityContextV1 =
        manager.create_listener(listener.as_fd(), alive_read.as_fd(), qh, ());
    context.set_sandbox_engine("org.otto.msg".to_string());
    context.commit();
    queue
        .roundtrip(state)
        .map_err(|err| format!("Otto refused the agent's display: {err}"))?;
    // Otto holds its own copies from here on.
    drop(listener);
    drop(alive_read);
    Ok((path, alive_write.into()))
}

fn compositor(bus: &Bus) -> Result<BusProxy<'static>, String> {
    BusProxy::new(
        bus,
        "org.otto.Compositor",
        "/org/otto/Compositor",
        "org.otto.Compositor",
    )
    .map_err(|err| format!("cannot reach Otto: {err}"))
}

/// The message of a D-Bus error, without the error name in front.
fn reason(err: &zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(_, Some(message), _) => message.clone(),
        zbus::Error::FDO(fdo) => match fdo.as_ref() {
            zbus::fdo::Error::AccessDenied(message) | zbus::fdo::Error::Failed(message) => {
                message.clone()
            }
            other => other.to_string(),
        },
        other => other.to_string(),
    }
}

/// A PNG's width and height, from its header.
fn png_size(path: &Path) -> Option<(u32, u32)> {
    let mut header = [0u8; 24];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut file, &mut header).ok()?;
    if &header[1..4] != b"PNG" {
        return None;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?);
    Some((width, height))
}

impl Dispatch<wl_registry::WlRegistry, ()> for Wayland {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_seat" => {
                    let seat = registry.bind(name, version.min(5), qh, name);
                    state.seats.push((name, seat, None));
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.pointer_manager = Some(registry.bind(name, version.min(2), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboard_manager = Some(registry.bind(name, 1, qh, ()));
                }
                "wp_security_context_manager_v1" => {
                    state.security_contexts = Some(registry.bind(name, 1, qh, ()));
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    state.registry = Some(registry.clone());
                    state.toplevel_global = Some((name, version.min(3)));
                    state.toplevel_manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } if state.agent_seat_global == Some(name) => {
                state.seat_gone = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, u32> for Wayland {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Name { name } = event {
            if let Some(entry) = state.seats.iter_mut().find(|(g, _, _)| g == global) {
                entry.2 = Some(name);
            }
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        manager: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            // A list being replaced may still announce one; only the
            // current list's count.
            if state.toplevel_manager.as_ref() != Some(manager) {
                toplevel.destroy();
                return;
            }
            state
                .windows
                .insert(toplevel.id(), (toplevel, Window::default()));
        }
    }

    event_created_child!(Wayland, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some((_, window)) = state.windows.get_mut(&handle.id()) {
                    window.title = title;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some((_, window)) = state.windows.get_mut(&handle.id()) {
                    window.app_id = app_id;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.windows.remove(&handle.id());
                handle.destroy();
            }
            _ => {}
        }
    }
}

delegate_noop!(Wayland: ignore WpSecurityContextManagerV1);
delegate_noop!(Wayland: ignore WpSecurityContextV1);
delegate_noop!(Wayland: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Wayland: ignore ZwlrVirtualPointerV1);
delegate_noop!(Wayland: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Wayland: ignore ZwpVirtualKeyboardV1);
