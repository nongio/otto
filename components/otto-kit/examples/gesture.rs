//! A scripted pointer, for driving a running compositor from a test.
//!
//! `wlrctl` can click, but a click is a press and a release with nothing in
//! between — and everything interesting about a drag happens in between. This
//! speaks `zwlr_virtual_pointer_v1` directly, so a script can put the pointer
//! somewhere absolute, hold a button down, travel, and let go.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-1 cargo run -p otto-kit --example gesture -- \
//!     move 300 400 down move 900 400 sleep 200 up
//! ```
//!
//! Commands, read left to right:
//!
//! | command            | what it does                                        |
//! |--------------------|-----------------------------------------------------|
//! | `move X Y`         | jump to an absolute point, in output pixels          |
//! | `glide X Y STEPS`  | travel there in `STEPS` motions, 8ms apart           |
//! | `down [BUTTON]`    | press (`left`, `right`, `middle`; default `left`)    |
//! | `up [BUTTON]`      | release                                              |
//! | `click [BUTTON]`   | press and release                                    |
//! | `scroll DY DX`     | one axis frame                                       |
//! | `sleep MS`         | wait                                                 |
//!
//! Absolute coordinates are output pixels: the pointer is bound to the first
//! output the compositor advertises, and `motion_absolute` is sent with that
//! output's current mode as the extent.

use std::time::{Duration, Instant};

use wayland_client::{
    protocol::{wl_output, wl_pointer, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    output: Option<wl_output::WlOutput>,
    /// The first output's current mode, which is the extent absolute motion
    /// is expressed in.
    size: Option<(u32, u32)>,
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
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_seat" => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "zwlr_virtual_pointer_manager_v1" => {
                state.manager = Some(registry.bind(name, version.min(2), qh, ()))
            }
            // The first one only: a nested or single-screen session has one,
            // and a script's coordinates belong to a screen either way.
            "wl_output" if state.output.is_none() => {
                state.output = Some(registry.bind(name, version.min(4), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
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
                .is_ok_and(|f| f.contains(wl_output::Mode::Current))
            {
                state.size = Some((width as u32, height as u32));
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
);

fn button_code(name: &str) -> Option<u32> {
    match name {
        "left" => Some(BTN_LEFT),
        "right" => Some(BTN_RIGHT),
        "middle" => Some(BTN_MIDDLE),
        _ => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());

    let mut state = State::default();
    // Twice: the first pass binds the globals, the second collects the
    // output's mode, which only arrives once the output is bound.
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;

    let seat = state.seat.clone().ok_or("no wl_seat")?;
    let manager = state
        .manager
        .clone()
        .ok_or("the compositor has no zwlr_virtual_pointer_manager_v1")?;
    let size = state.size.ok_or("no output with a current mode")?;

    // Bound to the output, so absolute coordinates land where the script
    // means them to on a multi-output session too.
    let pointer = match state.output.clone() {
        Some(output) => {
            manager.create_virtual_pointer_with_output(Some(&seat), Some(&output), &qh, ())
        }
        None => manager.create_virtual_pointer(Some(&seat), &qh, ()),
    };
    queue.roundtrip(&mut state)?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = Instant::now();
    let now = |start: &Instant| start.elapsed().as_millis() as u32;
    // Where the script last put the pointer, so `glide` knows where it is
    // travelling from.
    let mut at = (size.0 as f64 / 2.0, size.1 as f64 / 2.0);

    let mut i = 0;
    while i < args.len() {
        let word = args[i].as_str();
        let take = |n: usize| -> Result<Vec<String>, String> {
            if i + n >= args.len() {
                return Err(format!("`{word}` wants {n} argument(s)"));
            }
            Ok(args[i + 1..=i + n].to_vec())
        };
        match word {
            "move" => {
                let a = take(2)?;
                at = (a[0].parse()?, a[1].parse()?);
                pointer.motion_absolute(now(&start), at.0 as u32, at.1 as u32, size.0, size.1);
                pointer.frame();
                i += 3;
            }
            "glide" => {
                let a = take(3)?;
                let (tx, ty): (f64, f64) = (a[0].parse()?, a[1].parse()?);
                let steps: u32 = a[2].parse()?;
                let steps = steps.max(1);
                let (fx, fy) = at;
                for step in 1..=steps {
                    let t = step as f64 / steps as f64;
                    let (x, y) = (fx + (tx - fx) * t, fy + (ty - fy) * t);
                    pointer.motion_absolute(now(&start), x as u32, y as u32, size.0, size.1);
                    pointer.frame();
                    queue.flush()?;
                    std::thread::sleep(Duration::from_millis(8));
                }
                at = (tx, ty);
                i += 4;
            }
            "down" | "up" | "click" => {
                // The button is optional, so it is only consumed when the next
                // word actually names one.
                let button = args
                    .get(i + 1)
                    .and_then(|name| button_code(name))
                    .unwrap_or(BTN_LEFT);
                let named = args.get(i + 1).is_some_and(|n| button_code(n).is_some());
                let press = |t: u32, down: bool| {
                    pointer.button(
                        t,
                        button,
                        if down {
                            wl_pointer::ButtonState::Pressed
                        } else {
                            wl_pointer::ButtonState::Released
                        },
                    );
                    pointer.frame();
                };
                match word {
                    "down" => press(now(&start), true),
                    "up" => press(now(&start), false),
                    _ => {
                        press(now(&start), true);
                        press(now(&start), false);
                    }
                }
                i += 1 + usize::from(named);
            }
            "scroll" => {
                let a = take(2)?;
                let (dy, dx): (f64, f64) = (a[0].parse()?, a[1].parse()?);
                pointer.axis_source(wl_pointer::AxisSource::Wheel);
                if dy != 0.0 {
                    pointer.axis(now(&start), wl_pointer::Axis::VerticalScroll, dy);
                }
                if dx != 0.0 {
                    pointer.axis(now(&start), wl_pointer::Axis::HorizontalScroll, dx);
                }
                pointer.frame();
                i += 3;
            }
            "sleep" => {
                let a = take(1)?;
                queue.flush()?;
                std::thread::sleep(Duration::from_millis(a[0].parse()?));
                i += 2;
            }
            other => return Err(format!("unknown command `{other}`").into()),
        }
        queue.flush()?;
    }

    queue.roundtrip(&mut state)?;
    Ok(())
}
