//! Scripted touchpad scrolling through `zwlr_virtual_pointer_v1`.
//!
//! A benchmark driver, not an app: it parks the pointer at a point on the
//! first output and plays two-finger scroll gestures into whatever surface is
//! under it — finger-sourced axis events at touchpad rate, closed with an
//! `axis_stop` so the client's momentum takes over, exactly the stream a real
//! touchpad produces.
//!
//! Point it at a nested or headless compositor. Run against the session you
//! are sitting in, it scrolls whatever is under your pointer.
//!
//! ```sh
//! WAYLAND_DISPLAY=wayland-2 cargo run --release -p otto-kit --example virtual_fling -- \
//!     --at 0.6,0.5 --flings 6 --coast 1.2 --drag 3
//! ```
//!
//! - `--at X,Y`     where to park the pointer, as fractions of the output
//! - `--flings N`   down-then-up fling pairs (default 4)
//! - `--speed PT`   points per second at the finger's peak (default 2400)
//! - `--coast S`    seconds to let each fling's momentum run (default 1.2)
//! - `--drag S`     seconds of steady finger drag, down then up (default 0)
//!
//! Prints one line per phase with its wall-clock start, so a log sampled
//! alongside can be cut into the same phases.

use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{globals::registry_queue_init, Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// A touchpad's report rate.
const RATE_HZ: f64 = 120.0;
const EXTENT: u32 = 10_000;

struct State;

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(State: ignore ZwlrVirtualPointerV1);

struct Args {
    at: (f64, f64),
    flings: u32,
    speed: f64,
    coast: f64,
    drag: f64,
}

fn args() -> Args {
    let mut args = Args {
        at: (0.5, 0.5),
        flings: 4,
        speed: 2400.0,
        coast: 1.2,
        drag: 0.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().unwrap_or_default();
        match flag.as_str() {
            "--at" => {
                let (x, y) = value.split_once(',').expect("--at X,Y");
                args.at = (x.parse().unwrap(), y.parse().unwrap());
            }
            "--flings" => args.flings = value.parse().unwrap(),
            "--speed" => args.speed = value.parse().unwrap(),
            "--coast" => args.coast = value.parse().unwrap(),
            "--drag" => args.drag = value.parse().unwrap(),
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

struct Driver {
    conn: Connection,
    pointer: ZwlrVirtualPointerV1,
    epoch: Instant,
}

impl Driver {
    fn millis(&self) -> u32 {
        self.epoch.elapsed().as_millis() as u32
    }

    fn park(&self, at: (f64, f64)) {
        let t = self.millis();
        self.pointer.motion_absolute(
            t,
            (at.0 * EXTENT as f64) as u32,
            (at.1 * EXTENT as f64) as u32,
            EXTENT,
            EXTENT,
        );
        self.pointer.frame();
        self.conn.flush().unwrap();
    }

    /// One touchpad report: `delta` points of finger travel on the vertical
    /// axis. Positive scrolls the content down, as a touchpad does.
    fn report(&self, delta: f64) {
        let t = self.millis();
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        self.pointer
            .axis(t, wl_pointer::Axis::VerticalScroll, delta);
        self.pointer.frame();
        self.conn.flush().unwrap();
    }

    fn lift(&self) {
        let t = self.millis();
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        self.pointer.axis_stop(t, wl_pointer::Axis::VerticalScroll);
        self.pointer.frame();
        self.conn.flush().unwrap();
    }

    /// A flick: the finger accelerates to `speed` over ~120 ms and lifts.
    fn fling(&self, direction: f64, speed: f64) {
        let tick = Duration::from_secs_f64(1.0 / RATE_HZ);
        let reports = 14;
        for i in 0..reports {
            let ramp = (i + 1) as f64 / reports as f64;
            self.report(direction * speed * ramp / RATE_HZ);
            std::thread::sleep(tick);
        }
        self.lift();
    }

    /// A steady drag at `speed` for `seconds`, lifted at rest so nothing
    /// coasts afterwards.
    fn drag(&self, direction: f64, speed: f64, seconds: f64) {
        let tick = Duration::from_secs_f64(1.0 / RATE_HZ);
        let reports = (seconds * RATE_HZ) as u32;
        for _ in 0..reports {
            self.report(direction * speed / RATE_HZ);
            std::thread::sleep(tick);
        }
        // Decelerate to a stop before lifting, or the lift reads as a fling.
        for i in (0..12).rev() {
            self.report(direction * speed * i as f64 / 12.0 / RATE_HZ);
            std::thread::sleep(tick);
        }
        std::thread::sleep(Duration::from_millis(80));
        self.lift();
    }
}

fn phase(driver: &Driver, name: &str) {
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    println!(
        "phase {name} at {wall:.3} (+{:.3}s)",
        driver.epoch.elapsed().as_secs_f64()
    );
}

fn main() {
    let args = args();
    let conn = Connection::connect_to_env().expect("no Wayland display");
    let (globals, mut queue) = registry_queue_init::<State>(&conn).expect("registry");
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("wl_seat");
    let manager: ZwlrVirtualPointerManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .expect("the compositor has no zwlr_virtual_pointer_manager_v1");
    let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
    queue.roundtrip(&mut State).unwrap();

    let driver = Driver {
        conn,
        pointer,
        epoch: Instant::now(),
    };

    driver.park(args.at);
    std::thread::sleep(Duration::from_millis(400));

    if args.drag > 0.0 {
        phase(&driver, "drag");
        driver.drag(1.0, 900.0, args.drag);
        driver.drag(-1.0, 900.0, args.drag);
        std::thread::sleep(Duration::from_millis(500));
    }

    if args.flings > 0 {
        phase(&driver, "flings");
        for _ in 0..args.flings {
            driver.fling(1.0, args.speed);
            std::thread::sleep(Duration::from_secs_f64(args.coast));
        }
        for _ in 0..args.flings {
            driver.fling(-1.0, args.speed);
            std::thread::sleep(Duration::from_secs_f64(args.coast));
        }
    }
    phase(&driver, "done");
    queue.roundtrip(&mut State).ok();
}
