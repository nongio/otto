//! Live input overlays for screen recordings: one circle per finger on the
//! touchpad, and the keys being pressed — chords included, as `Super+Shift+K`.
//!
//! A touchpad's per-finger positions never reach a Wayland client: libinput
//! folds touchpad multitouch into pointer motion and gesture events before
//! any protocol carries it, exactly as it feeds Otto's own udev backend
//! (`wl_touch` is for touchscreens only). Keys only reach the focused
//! window. So both overlays read the kernel's evdev nodes directly — the
//! same technique `libinput debug-gui` uses — via `App::poll_fds`/`on_update`
//! rather than any Wayland input event.
//!
//! Both overlays are subsurfaces of one transparent layer surface that
//! covers the output on the overlay layer, so they stay above fullscreen
//! windows. That surface never moves and takes input only over the
//! overlays: pointer positions arrive in output coordinates however far an
//! overlay has been dragged, and every click beside them reaches the
//! windows underneath. The key overlay starts beside the touch overlay.
//!
//! An applet in the top bar switches either overlay on and off and quits.
//!
//! ```sh
//! cargo run -p otto-input-overlay -- \
//!     [--position POS] [--margin PT] [--width PT] [--height PT] \
//!     [--radius PT] [--colors LIST] [--bg COLOR] [/dev/input/eventN]
//! ```
//! With no device argument, the first entry in `/proc/bus/input/devices`
//! whose name contains "touchpad" is used; keyboards are always found there.
//! Needs read access to those nodes — the `input` group has it by default on
//! most distros; otherwise run with sudo.
//!
//! `--position` places the overlays on the output: `top-left`, `top`,
//! `top-right`, `left`, `center`, `right`, `bottom-left`, `bottom`
//! (default `bottom-right`), kept `--margin` points (default 16) from the
//! edges it touches, with the key overlay beside it. `--width` sets each
//! overlay's width in logical points (default 180); `--height` defaults to
//! the touchpad's own aspect ratio.
//!
//! `--radius` sets the base circle size in logical points (default 9).
//!
//! `--key-style` writes chords as `words` (`Super+Shift+K`, the default) or
//! `symbols` (`⇧⌘K`).
//!
//! `--colors` takes a comma-separated list cycled across fingers by slot
//! (default: the theme's `accent`),
//! and `--bg` takes a single one for the overlays (default: translucent
//! black; `none` draws no panel behind the fingers) — each entry is either a
//! hex color (`#34d399`, `#34d399cc` with alpha, or without the `#`) or an
//! Otto theme token resolved against the live system theme:
//! `accent`, `accent-red`, `accent-gray`, `fill-primary`, `fill-secondary`,
//! `fill-tertiary`, `fill-quaternary`, `text-primary`, `text-secondary`,
//! `text-tertiary`, `material-titlebar`, `material-sidebar`,
//! `material-medium`, `material-popup`, `material-highlight`,
//! `material-selection-focused`, `shadow`, `hairline`.
//! Example: `--position top-right --colors accent,accent-red,#34d399 --bg #00000080`.

mod indicator;
mod keys;

use otto_kit::surfaces::{LayerShellSurface, SubsurfaceSurface};
use otto_kit::theme::Theme;
use otto_kit::typography::{draw_runs, measure_runs, TextStyle};
use otto_kit::{App, AppContext, AppRunner, CursorShape};
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

// linux/input-event-codes.h — just the codes this example reads.
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_PRESSURE: u16 = 0x3a;

/// `sizeof(struct input_event)` on 64-bit Linux: a 16-byte `timeval` (two
/// `i64`s on this ABI) plus `u16 type, u16 code, i32 value`.
const EVENT_SIZE: usize = 24;

/// The default base circle radius, in logical points — overridden by
/// `--radius`.
const DEFAULT_RADIUS: f32 = 9.0;
/// The default panel width, in logical points — overridden by `--width`.
const DEFAULT_WIDTH: u32 = 180;
/// The default distance from the anchored edges — overridden by `--margin`.
const DEFAULT_MARGIN: i32 = 16;
/// The panel's corner radius, in logical points.
const CORNER_RADIUS: f32 = 10.0;
/// Space between the key overlay and the touch overlay it starts beside.
const PANEL_GAP: i32 = 8;

#[derive(Default, Clone, Copy)]
struct Contact {
    /// Position normalized to 0..1 across the device's reported axis range.
    nx: f32,
    ny: f32,
    pressure: i32,
}

fn normalize(value: i32, min: i32, max: i32) -> f32 {
    if max <= min {
        return 0.5;
    }
    ((value - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
}

/// `EVIOCGABS(abs) = _IOR('E', 0x40 + abs, struct input_absinfo)`.
fn eviocgabs(code: u16) -> libc::c_ulong {
    let nr = 0x40 + code as u32;
    let size = std::mem::size_of::<InputAbsInfo>() as u32;
    ((2u32 << 30) | (size << 16) | (('E' as u32) << 8) | nr) as libc::c_ulong
}

#[repr(C)]
#[derive(Default)]
struct InputAbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

fn abs_info(fd: RawFd, code: u16) -> std::io::Result<InputAbsInfo> {
    let mut info = InputAbsInfo::default();
    let ret = unsafe { libc::ioctl(fd, eviocgabs(code), &mut info as *mut InputAbsInfo) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info)
}

/// The touchpad's height over its width: in millimetres when the device
/// reports a resolution (units per mm) on both axes, in raw units otherwise.
fn aspect_ratio(x: &InputAbsInfo, y: &InputAbsInfo) -> f32 {
    let span = |info: &InputAbsInfo| (info.maximum - info.minimum).max(1) as f32;
    let (mut w, mut h) = (span(x), span(y));
    if x.resolution > 0 && y.resolution > 0 {
        w /= x.resolution as f32;
        h /= y.resolution as f32;
    }
    h / w
}

/// `--position` → the layer-shell edges the panel is anchored to.
fn parse_position(spec: &str) -> Option<Anchor> {
    Some(match spec {
        "top-left" => Anchor::Top | Anchor::Left,
        "top" => Anchor::Top,
        "top-right" => Anchor::Top | Anchor::Right,
        "left" => Anchor::Left,
        "center" | "centre" => Anchor::empty(),
        "right" => Anchor::Right,
        "bottom-left" => Anchor::Bottom | Anchor::Left,
        "bottom" => Anchor::Bottom,
        "bottom-right" => Anchor::Bottom | Anchor::Right,
        _ => return None,
    })
}

fn set_nonblocking(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Input devices worth reading, from `/proc/bus/input/devices`: the first
/// one whose name mentions a touchpad, and every keyboard — a `kbd` handler
/// on a device with autorepeat, which leaves out power buttons, lid switches
/// and the like.
fn find_devices() -> std::io::Result<(Option<String>, Vec<String>)> {
    let devices = std::fs::read_to_string("/proc/bus/input/devices")?;
    let mut touchpad = None;
    let mut keyboards = Vec::new();
    for block in devices.split("\n\n") {
        let mut name = String::new();
        let mut handlers = Vec::new();
        let mut ev_bits = 0u64;
        for line in block.lines() {
            if let Some(n) = line.strip_prefix("N: Name=") {
                name = n.trim_matches('"').to_lowercase();
            } else if let Some(h) = line.strip_prefix("H: Handlers=") {
                handlers = h.split_whitespace().map(str::to_string).collect();
            } else if let Some(bits) = line.strip_prefix("B: EV=") {
                ev_bits = u64::from_str_radix(bits.trim(), 16).unwrap_or(0);
            }
        }
        let Some(event) = handlers.iter().find(|h| h.starts_with("event")) else {
            continue;
        };
        let node = format!("/dev/input/{event}");
        const EV_REP_BIT: u64 = 1 << 0x14;
        if name.contains("touchpad") {
            touchpad.get_or_insert(node);
        } else if handlers.iter().any(|h| h == "kbd")
            && ev_bits & (1 << EV_KEY) != 0
            && ev_bits & EV_REP_BIT != 0
        {
            keyboards.push(node);
        }
    }
    Ok((touchpad, keyboards))
}

/// Read every whole `input_event` waiting on a non-blocking evdev node,
/// handing each to `f` as `(type, code, value)`.
fn drain(device: &mut File, mut f: impl FnMut(u16, u16, i32)) {
    let mut buf = [0u8; EVENT_SIZE];
    loop {
        match device.read(&mut buf) {
            Ok(EVENT_SIZE) => f(
                u16::from_ne_bytes([buf[16], buf[17]]),
                u16::from_ne_bytes([buf[18], buf[19]]),
                i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]),
            ),
            Ok(_) => break, // short read: shouldn't happen for a char device
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => {
                eprintln!("otto-input-overlay: read error: {e}");
                break;
            }
        }
    }
}

fn open_device(path: &str) -> Result<File, String> {
    let device = File::open(path)
        .map_err(|e| format!("open {path}: {e} (are you in the `input` group?)"))?;
    set_nonblocking(device.as_raw_fd()).map_err(|e| format!("{path}: {e}"))?;
    Ok(device)
}

/// A hex color: `#rrggbb`, `#rrggbbaa`, or the same without the `#`.
fn parse_hex_color(spec: &str) -> Option<skia_safe::Color> {
    let hex = spec.strip_prefix('#').unwrap_or(spec);
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => {
            let rgb = u32::from_str_radix(hex, 16).ok()?;
            Some(skia_safe::Color::from_rgb(
                (rgb >> 16) as u8,
                (rgb >> 8) as u8,
                rgb as u8,
            ))
        }
        8 => {
            let rgba = u32::from_str_radix(hex, 16).ok()?;
            Some(skia_safe::Color::from_argb(
                rgba as u8,
                (rgba >> 24) as u8,
                (rgba >> 16) as u8,
                (rgba >> 8) as u8,
            ))
        }
        _ => None,
    }
}

/// An Otto theme token, e.g. `accent` or `material-selection-focused`
/// (`_` and `-` are interchangeable, case-insensitive).
fn theme_color(theme: &Theme, name: &str) -> Option<skia_safe::Color> {
    let key = name.to_lowercase().replace('_', "-");
    Some(match key.as_str() {
        "accent" => theme.accent,
        "accent-gray" | "accent-grey" => theme.accent_gray,
        "accent-red" => theme.accent_red,
        "fill-primary" => theme.fill_primary,
        "fill-secondary" => theme.fill_secondary,
        "fill-tertiary" => theme.fill_tertiary,
        "fill-quaternary" => theme.fill_quaternary,
        "text-primary" => theme.text_primary,
        "text-secondary" => theme.text_secondary,
        "text-tertiary" => theme.text_tertiary,
        "material-titlebar" => theme.material_titlebar,
        "material-sidebar" => theme.material_sidebar,
        "material-medium" => theme.material_medium,
        "material-popup" => theme.material_popup,
        "material-highlight" => theme.material_highlight,
        "material-selection-focused" | "material-selection" => theme.material_selection_focused,
        "shadow" => theme.shadow,
        "hairline" => theme.hairline,
        _ => return None,
    })
}

fn parse_color(spec: &str, theme: &Theme) -> Result<skia_safe::Color, String> {
    parse_hex_color(spec)
        .or_else(|| theme_color(theme, spec))
        .ok_or_else(|| format!("unknown color `{spec}` — not a hex code or an otto-kit theme name"))
}

struct CliArgs {
    device: Option<String>,
    anchor: Anchor,
    margin: i32,
    width: u32,
    height: Option<u32>,
    radius: f32,
    color_specs: Vec<String>,
    bg_spec: Option<String>,
    key_style: keys::KeyStyle,
}

const HELP: &str = "\
otto-input-overlay — shows touchpad contacts and key presses on screen

USAGE:
    otto-input-overlay [OPTIONS] [DEVICE]

ARGS:
    <DEVICE>    touchpad /dev/input/eventN to read (default: auto-detect)

OPTIONS:
    --position <POS>        top-left, top, top-right, left, center, right,
                             bottom-left, bottom, bottom-right
                             (default: bottom-right)
    --margin <PT>           distance from the anchored edges (default: 16)
    --width <PT>            overlay width in logical points (default: 180)
    --height <PT>           touch overlay height (default: follows the
                             touchpad's aspect ratio)
    --radius, --size <PT>   base circle radius in logical points (default: 9)
    --colors <LIST>         comma-separated hex colors and/or otto-kit theme
                             names, cycled across fingers by slot (default:
                             accent) — see the
                             module doc comment for the full theme token list
    --key-style <STYLE>     how chords are written: `words` (Super+Shift+K,
                             the default) or `symbols` (⇧⌘K)
    --bg <COLOR>            overlay color — a hex color or otto-kit theme
                             name, same syntax as one entry of --colors, or
                             `none` for no panel behind the fingers
                             (default: translucent black)
    -h, --help               print this help
";

fn parse_args() -> Result<CliArgs, String> {
    let mut device = None;
    let mut anchor = Anchor::Bottom | Anchor::Right;
    let mut margin = DEFAULT_MARGIN;
    let mut width = DEFAULT_WIDTH;
    let mut height = None;
    let mut radius = DEFAULT_RADIUS;
    let mut color_specs = Vec::new();
    let mut bg_spec = None;
    let mut key_style = keys::KeyStyle::default();

    fn number<T: std::str::FromStr>(
        args: &mut impl Iterator<Item = String>,
        flag: &str,
    ) -> Result<T, String> {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        value
            .parse()
            .map_err(|_| format!("invalid {flag} value `{value}`"))
    }

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--position" => {
                let value = args.next().ok_or("--position needs a value")?;
                anchor = parse_position(&value)
                    .ok_or_else(|| format!("invalid --position value `{value}` (see --help)"))?;
            }
            "--margin" => margin = number(&mut args, "--margin")?,
            "--width" => width = number(&mut args, "--width")?,
            "--height" => height = Some(number(&mut args, "--height")?),
            "--radius" | "--size" => radius = number(&mut args, "--radius")?,
            "--colors" => {
                let value = args.next().ok_or("--colors needs a value")?;
                color_specs = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "--key-style" => {
                let value = args.next().ok_or("--key-style needs a value")?;
                key_style = keys::KeyStyle::parse(&value).ok_or_else(|| {
                    format!("invalid --key-style value `{value}` (words or symbols)")
                })?;
            }
            "--bg" => {
                let value = args.next().ok_or("--bg needs a value")?;
                bg_spec = Some(value);
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other if !other.starts_with('-') && device.is_none() => {
                device = Some(other.to_string());
            }
            other => return Err(format!("unrecognized argument `{other}` (see --help)")),
        }
    }

    Ok(CliArgs {
        device,
        anchor,
        margin,
        width,
        height,
        radius,
        color_specs,
        bg_spec,
        key_style,
    })
}

/// `BTN_LEFT` from linux/input-event-codes.h, as `wl_pointer` reports it.
const BTN_LEFT: u32 = 0x110;

/// One overlay: a subsurface of the full-output layer surface.
struct Panel {
    surface: SubsurfaceSurface,
    /// Top-left corner on the output, in logical points.
    position: (i32, i32),
    size: (u32, u32),
}

impl Panel {
    fn new(
        parent: &LayerShellSurface,
        size: (u32, u32),
        position: (i32, i32),
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let surface = SubsurfaceSurface::new(
            &parent.wl_surface(),
            position.0,
            position.1,
            size.0 as i32,
            size.1 as i32,
        )?;
        // The pointer goes to the parent, which never moves; see the module
        // doc comment.
        let region = Region::new(AppContext::compositor_state())?;
        surface
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
        Ok(Self {
            surface,
            position,
            size,
        })
    }

    /// Put the top-left corner at `position`, kept whole on an output of
    /// `bounds`. Takes effect when the parent commits.
    fn move_to(&mut self, position: (i32, i32), bounds: (i32, i32)) {
        let x = position.0.clamp(0, (bounds.0 - self.size.0 as i32).max(0));
        let y = position.1.clamp(0, (bounds.1 - self.size.1 as i32).max(0));
        self.position = (x, y);
        self.surface.set_position(x, y);
    }

    fn contains(&self, (x, y): (f64, f64)) -> bool {
        let (left, top) = (self.position.0 as f64, self.position.1 as f64);
        (left..left + self.size.0 as f64).contains(&x)
            && (top..top + self.size.1 as f64).contains(&y)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Which {
    Touches,
    Keys,
}

#[derive(Clone, Copy)]
struct Drag {
    which: Which,
    /// Where the press landed, relative to the overlay's corner.
    grab: (f64, f64),
}

/// Where an overlay of `size` goes on `output` for a `--position` anchor.
fn anchored_position(
    anchor: Anchor,
    size: (u32, u32),
    output: (i32, i32),
    margin: i32,
) -> (i32, i32) {
    let along = |start: Anchor, end: Anchor, extent: i32, length: i32| match (
        anchor.contains(start),
        anchor.contains(end),
    ) {
        (true, false) => margin,
        (false, true) => extent - margin - length,
        _ => (extent - length) / 2,
    };
    (
        along(Anchor::Left, Anchor::Right, output.0, size.0 as i32),
        along(Anchor::Top, Anchor::Bottom, output.1, size.1 as i32),
    )
}

struct InputOverlay {
    /// Transparent, covering the output; carries both overlays and takes
    /// the pointer over them.
    parent: Option<LayerShellSurface>,
    touch_panel: Option<Panel>,
    keys_panel: Option<Panel>,
    /// The output's logical size, from the parent's first configure; the
    /// overlays are created then.
    output: Option<(i32, i32)>,
    drag: Option<Drag>,
    touchpad: Option<File>,
    keyboards: Vec<File>,
    x_range: (i32, i32),
    y_range: (i32, i32),
    anchor: Anchor,
    margin: i32,
    size: (u32, u32),
    radius: f32,
    /// Resolved against the live theme in `on_app_ready`; empty means "use
    /// the default palette".
    color_specs: Vec<String>,
    /// Resolved the same way; `None` means "use the default panel color".
    bg_spec: Option<String>,
    palette: Vec<skia_safe::Color>,
    /// `None` draws no panel behind the fingers.
    background: Option<skia_safe::Color>,
    contacts: HashMap<i32, Contact>,
    /// Slot most recently selected by `ABS_MT_SLOT`; every following
    /// per-axis event until the next slot change belongs to it.
    current_slot: i32,
    keys: keys::KeyState,
    /// What the top-bar applet asks for; `touches_on`/`keys_on` are what's
    /// currently applied.
    toggles: Arc<indicator::Toggles>,
    touches_on: bool,
    keys_on: bool,
}

impl InputOverlay {
    fn apply_abs(&mut self, code: u16, value: i32) {
        if code == ABS_MT_SLOT {
            self.current_slot = value;
            return;
        }
        let contacts = &mut self.contacts;
        match code {
            ABS_MT_TRACKING_ID => {
                if value < 0 {
                    contacts.remove(&self.current_slot);
                } else {
                    contacts.entry(self.current_slot).or_default();
                }
            }
            ABS_MT_POSITION_X => {
                let (min, max) = self.x_range;
                contacts.entry(self.current_slot).or_default().nx = normalize(value, min, max);
            }
            ABS_MT_POSITION_Y => {
                let (min, max) = self.y_range;
                contacts.entry(self.current_slot).or_default().ny = normalize(value, min, max);
            }
            ABS_MT_PRESSURE => {
                contacts.entry(self.current_slot).or_default().pressure = value;
            }
            _ => {}
        }
    }

    /// Create the overlays once the parent's configure has given the output's
    /// size: the touch overlay at `--position`, the key overlay beside it on
    /// the side facing the middle of the screen.
    fn create_panels(&mut self, output: (i32, i32)) -> Result<(), Box<dyn std::error::Error>> {
        let Some(parent) = &self.parent else {
            return Ok(());
        };
        let first = anchored_position(self.anchor, self.size, output, self.margin);
        let mut keys_position = first;
        if self.touchpad.is_some() {
            self.touch_panel = Some(Panel::new(parent, self.size, first)?);
            let step = self.size.0 as i32 + PANEL_GAP;
            let toward_middle = if first.0 + self.size.0 as i32 / 2 > output.0 / 2 {
                -step
            } else {
                step
            };
            keys_position = (first.0 + toward_middle, first.1);
        }
        if !self.keyboards.is_empty() {
            let mut panel = Panel::new(parent, self.size, keys_position)?;
            panel.move_to(keys_position, output);
            self.keys_panel = Some(panel);
        }
        Ok(())
    }

    fn panel(&self, which: Which) -> Option<&Panel> {
        match which {
            Which::Touches => self.touch_panel.as_ref(),
            Which::Keys => self.keys_panel.as_ref(),
        }
    }

    fn panel_mut(&mut self, which: Which) -> Option<&mut Panel> {
        match which {
            Which::Touches => self.touch_panel.as_mut(),
            Which::Keys => self.keys_panel.as_mut(),
        }
    }

    /// The overlay a pointer position is over — the key overlay first, as it
    /// is stacked above.
    fn panel_at(&self, point: (f64, f64)) -> Option<Which> {
        [
            (Which::Keys, self.keys_on),
            (Which::Touches, self.touches_on),
        ]
        .into_iter()
        .find(|&(which, on)| on && self.panel(which).is_some_and(|p| p.contains(point)))
        .map(|(which, _)| which)
    }

    /// Take the pointer over the overlays that are showing and nowhere else,
    /// and commit the parent so that and any overlay moves take effect.
    fn commit_parent(&self) {
        let Some(parent) = &self.parent else {
            return;
        };
        let Ok(region) = Region::new(AppContext::compositor_state()) else {
            return;
        };
        for (which, on) in [
            (Which::Touches, self.touches_on),
            (Which::Keys, self.keys_on),
        ] {
            if let Some(panel) = self.panel(which).filter(|_| on) {
                region.add(
                    panel.position.0,
                    panel.position.1,
                    panel.size.0 as i32,
                    panel.size.1 as i32,
                );
            }
        }
        let surface = parent.wl_surface();
        surface.set_input_region(Some(region.wl_region()));
        surface.commit();
    }

    fn redraw_touches(&self) {
        let Some(panel) = &self.touch_panel else {
            return;
        };
        let (width, height) = panel.surface.dimensions();
        let (width, height) = (width as f32, height as f32);
        panel.surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            if !self.touches_on {
                return;
            }
            let bounds = skia_safe::RRect::new_rect_xy(
                skia_safe::Rect::from_wh(width, height),
                CORNER_RADIUS,
                CORNER_RADIUS,
            );
            if let Some(background) = self.background {
                draw_panel(canvas, bounds, background);
            }
            canvas.save();
            canvas.clip_rrect(bounds, None, true);
            for (slot, contact) in &self.contacts {
                let px = contact.nx * width;
                let py = contact.ny * height;
                // Pressure adds up to another 100% on top of the configured
                // base radius, so `--radius` stays the size fingers rest at.
                let r = self.radius + (contact.pressure as f32 / 8.0).clamp(0.0, self.radius);

                let color = self.palette[slot.rem_euclid(self.palette.len() as i32) as usize];
                let mut fill = skia_safe::Paint::new(skia_safe::Color4f::from(color), None);
                fill.set_anti_alias(true);
                canvas.draw_circle((px, py), r, &fill);

                let mut ring =
                    skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 1.0, 1.0, 0.5), None);
                ring.set_anti_alias(true);
                ring.set_style(skia_safe::paint::Style::Stroke);
                ring.set_stroke_width(1.5);
                canvas.draw_circle((px, py), r, &ring);
            }
            canvas.restore();
        });
    }

    fn redraw_keys(&self) {
        let Some(panel) = &self.keys_panel else {
            return;
        };
        let (width, height) = panel.surface.dimensions();
        let (width, height) = (width as f32, height as f32);
        panel.surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            if !self.keys_on {
                return;
            }
            let bounds = skia_safe::RRect::new_rect_xy(
                skia_safe::Rect::from_wh(width, height),
                CORNER_RADIUS,
                CORNER_RADIUS,
            );
            draw_panel(
                canvas,
                bounds,
                self.background.unwrap_or(DEFAULT_BACKGROUND),
            );

            let Some(label) = self.keys.label() else {
                return;
            };
            const PADDING: f32 = 14.0;
            let mut style = TextStyle {
                family: "Inter",
                weight: 600,
                size: (height * 0.34).min(40.0),
            };
            let mut font = style.font();
            let mut text_width = measure_runs(&font, label);
            // A long chord shrinks to fit rather than running off the panel.
            let room = width - 2.0 * PADDING;
            if text_width > room {
                style.size *= room / text_width;
                font = style.font();
                text_width = measure_runs(&font, label);
            }
            let (_, metrics) = font.metrics();
            let baseline = height / 2.0 - (metrics.ascent + metrics.descent) / 2.0;
            let mut paint =
                skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
            paint.set_anti_alias(true);
            draw_runs(
                canvas,
                label,
                ((width - text_width) / 2.0, baseline),
                &font,
                &paint,
            );
        });
    }
}

const DEFAULT_BACKGROUND: skia_safe::Color = skia_safe::Color::from_argb(0x80, 0, 0, 0);

fn draw_panel(canvas: &skia_safe::Canvas, bounds: skia_safe::RRect, color: skia_safe::Color) {
    let mut fill = skia_safe::Paint::new(skia_safe::Color4f::from(color), None);
    fill.set_anti_alias(true);
    canvas.draw_rrect(bounds, &fill);

    let mut outline = skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 1.0, 1.0, 0.25), None);
    outline.set_anti_alias(true);
    outline.set_style(skia_safe::paint::Style::Stroke);
    outline.set_stroke_width(1.0);
    canvas.draw_rrect(bounds.with_inset((0.5, 0.5)), &outline);
}

impl App for InputOverlay {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let theme = AppContext::current_theme();
        self.palette = if self.color_specs.is_empty() {
            vec![theme.accent]
        } else {
            self.color_specs
                .iter()
                .map(|spec| parse_color(spec, &theme))
                .collect::<Result<Vec<_>, _>>()?
        };
        self.background = match self.bg_spec.as_deref() {
            None => Some(DEFAULT_BACKGROUND),
            Some("none" | "transparent") => None,
            Some(spec) => Some(parse_color(spec, &theme)?),
        };

        let parent = LayerShellSurface::with_anchor(
            Layer::Overlay,
            "input-overlay",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            // -1: the whole output, not the part the top bar leaves free —
            // the overlays go over the bar too, and Otto draws the parent's
            // subsurfaces from the output's corner either way.
            Some(-1),
        )?;
        parent.set_keyboard_interactivity(KeyboardInteractivity::None);
        self.parent = Some(parent);
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        let Some(parent) = &self.parent else {
            return;
        };
        // Nothing is drawn on the parent itself; it needs a buffer to be
        // mapped at all.
        parent.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });
        let output = (width, height);
        if self.output.is_none() {
            if let Err(e) = self.create_panels(output) {
                eprintln!("otto-input-overlay: {e}");
                std::process::exit(1);
            }
        }
        self.output = Some(output);
        for which in [Which::Touches, Which::Keys] {
            if let Some(panel) = self.panel_mut(which) {
                let position = panel.position;
                panel.move_to(position, output);
            }
        }
        self.redraw_touches();
        self.redraw_keys();
        self.commit_parent();
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        let (Some(parent), Some(output)) = (&self.parent, self.output) else {
            return;
        };
        let parent = parent.wl_surface();
        for event in events {
            if event.surface != parent {
                continue;
            }
            // In output coordinates: the parent covers the output and never
            // moves.
            let point = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    AppContext::set_cursor_shape(CursorShape::Grab);
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    let Some(which) = self.panel_at(point) else {
                        continue;
                    };
                    let Some(panel) = self.panel(which) else {
                        continue;
                    };
                    let grab = (
                        point.0 - panel.position.0 as f64,
                        point.1 - panel.position.1 as f64,
                    );
                    self.drag = Some(Drag { which, grab });
                    AppContext::set_cursor_shape(CursorShape::Grabbing);
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    self.drag = None;
                    AppContext::set_cursor_shape(CursorShape::Grab);
                }
                PointerEventKind::Motion { .. } => {
                    let Some(Drag { which, grab }) = self.drag else {
                        continue;
                    };
                    // Absolute: the corner goes where the pointer is, less
                    // where it took hold.
                    let corner = (
                        (point.0 - grab.0).round() as i32,
                        (point.1 - grab.1).round() as i32,
                    );
                    if let Some(panel) = self.panel_mut(which) {
                        panel.move_to(corner, output);
                    }
                    self.commit_parent();
                    // Otto shows a subsurface's new position once the
                    // subsurface itself commits, not on the parent's commit
                    // alone.
                    match which {
                        Which::Touches => self.redraw_touches(),
                        Which::Keys => self.redraw_keys(),
                    }
                }
                _ => {}
            }
        }
    }

    fn poll_fds(&self) -> Vec<RawFd> {
        self.touchpad
            .iter()
            .chain(&self.keyboards)
            .map(AsRawFd::as_raw_fd)
            .collect()
    }

    fn idle_timeout(&self) -> Option<std::time::Duration> {
        self.keys.time_to_expiry(Instant::now())
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        if self.toggles.quit.load(Ordering::SeqCst) {
            std::process::exit(0);
        }
        // Every node is drained on every pass, overlay on or off: a readable
        // fd left unread would wake the loop again straight away.
        let mut touch_events = Vec::new();
        if let Some(device) = &mut self.touchpad {
            drain(device, |ty, code, value| {
                touch_events.push((ty, code, value))
            });
        }
        let mut key_events = Vec::new();
        for device in &mut self.keyboards {
            drain(device, |ty, code, value| {
                if ty == EV_KEY {
                    key_events.push((code, value));
                }
            });
        }

        let mut touches_dirty = false;
        let mut keys_dirty = false;
        let mut input_changed = false;

        let touches_wanted = self.toggles.touches.load(Ordering::SeqCst);
        if touches_wanted != self.touches_on {
            self.touches_on = touches_wanted;
            touches_dirty = true;
            input_changed = true;
            if self
                .drag
                .as_ref()
                .is_some_and(|d| d.which == Which::Touches)
            {
                self.drag = None;
            }
        }
        let keys_wanted = self.toggles.keys.load(Ordering::SeqCst);
        if keys_wanted != self.keys_on {
            self.keys_on = keys_wanted;
            self.keys.reset();
            keys_dirty = true;
            input_changed = true;
            if self.drag.as_ref().is_some_and(|d| d.which == Which::Keys) {
                self.drag = None;
            }
        }

        for (ty, code, value) in touch_events {
            match ty {
                EV_ABS => self.apply_abs(code, value),
                // Redraw once per frame of touch data rather than per axis,
                // so a finger never paints at a half-updated position.
                EV_SYN if code == SYN_REPORT => touches_dirty |= self.touches_on,
                _ => {}
            }
        }
        if self.keys_on {
            for (code, value) in key_events {
                keys_dirty |= self.keys.apply(code, value);
            }
        }
        keys_dirty |= self.keys.expire(Instant::now());

        if touches_dirty {
            self.redraw_touches();
        }
        if keys_dirty {
            self.redraw_keys();
        }
        if input_changed {
            self.commit_parent();
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = parse_args()?;

    let (found_touchpad, keyboard_paths) = find_devices()?;
    let touchpad_path = cli.device.or(found_touchpad);

    let mut x = InputAbsInfo::default();
    let mut y = InputAbsInfo::default();
    let touchpad = match &touchpad_path {
        Some(path) => {
            let device = open_device(path)?;
            x = abs_info(device.as_raw_fd(), ABS_MT_POSITION_X)?;
            y = abs_info(device.as_raw_fd(), ABS_MT_POSITION_Y)?;
            Some(device)
        }
        None => {
            eprintln!("otto-input-overlay: no touchpad found — showing key presses only");
            None
        }
    };
    let keyboards: Vec<File> = keyboard_paths
        .iter()
        .filter_map(|path| {
            open_device(path)
                .map_err(|e| eprintln!("otto-input-overlay: skipping keyboard: {e}"))
                .ok()
        })
        .collect();

    let height = cli
        .height
        .unwrap_or_else(|| (cli.width as f32 * aspect_ratio(&x, &y)).round() as u32);
    println!(
        "otto-input-overlay: touchpad {} (x: {}..{}, y: {}..{}), keyboards {:?}, overlays: {}x{height}",
        touchpad_path.as_deref().unwrap_or("none"),
        x.minimum,
        x.maximum,
        y.minimum,
        y.maximum,
        keyboard_paths,
        cli.width,
    );

    let toggles = Arc::new(indicator::Toggles {
        touches: AtomicBool::new(true),
        keys: AtomicBool::new(true),
        quit: AtomicBool::new(false),
    });
    indicator::spawn(toggles.clone());

    let app = InputOverlay {
        parent: None,
        touch_panel: None,
        keys_panel: None,
        output: None,
        drag: None,
        touchpad,
        keyboards,
        x_range: (x.minimum, x.maximum),
        y_range: (y.minimum, y.maximum),
        anchor: cli.anchor,
        margin: cli.margin,
        size: (cli.width, height),
        radius: cli.radius,
        color_specs: cli.color_specs,
        bg_spec: cli.bg_spec,
        palette: Vec::new(),
        background: None,
        contacts: HashMap::new(),
        current_slot: 0,
        keys: keys::KeyState::new(cli.key_style),
        toggles,
        touches_on: true,
        keys_on: true,
    };

    AppRunner::new(app).run()
}
