//! otto-media-worker — the process that plays.
//!
//! Spawned by [`otto_media_kit::Player`] with a media file on descriptor 3
//! and a frame ring on descriptor 4, commands on stdin and events on stdout.
//! It runs one GStreamer pipeline, hands every decoded frame to the host
//! through the ring, plays audio itself, and exits when told to or when the
//! host goes away.
//!
//! It is the only part of a host application that ever links GStreamer or
//! parses a media container, and it is contained accordingly: no network, no
//! new privileges, no core dumps, and a descriptor ceiling. A demuxer that
//! crashes on a hostile file takes this process and nothing else.

use std::io::{self, BufRead, Write};
use std::os::fd::FromRawFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;

use otto_media_kit::protocol::{self, Command, Event, FILE_FD, FRAMES_FD};

/// The events pipe, shared by the streaming thread and the bus loop.
struct Events(Mutex<io::Stdout>);

impl Events {
    fn send(&self, event: Event) {
        let mut out = self.0.lock().unwrap();
        if out.write_all(event.encode().as_bytes()).is_err() || out.flush().is_err() {
            // The host closed its end: nothing to play for.
            std::process::exit(0);
        }
    }
}

/// The frame ring as the worker sees it: sized to the current frame, and
/// written one slot at a time.
struct Ring {
    file: std::fs::File,
    size: (u32, u32),
    next_slot: u32,
    seq: u64,
}

impl Ring {
    fn resize(&mut self, width: u32, height: u32) -> io::Result<()> {
        self.file.set_len(protocol::ring_bytes(width, height))?;
        self.size = (width, height);
        Ok(())
    }

    fn write(&mut self, rows: impl Iterator<Item = impl AsRef<[u8]>>) -> io::Result<(u32, u64)> {
        use std::os::unix::fs::FileExt;
        let (width, height) = self.size;
        let slot = self.next_slot;
        let mut at = protocol::slot_offset(slot, width, height);
        let row_bytes = (width * protocol::BYTES_PER_PIXEL) as usize;
        for row in rows.take(height as usize) {
            let row = &row.as_ref()[..row_bytes.min(row.as_ref().len())];
            self.file.write_all_at(row, at)?;
            at += row_bytes as u64;
        }
        self.next_slot = (slot + 1) % protocol::SLOTS;
        self.seq += 1;
        Ok((slot, self.seq))
    }
}

fn main() {
    let mut max_width = 1920u32;
    let mut max_height = 1080u32;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--max-width" => {
                max_width = arguments
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(max_width)
            }
            "--max-height" => {
                max_height = arguments
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(max_height)
            }
            _ => {}
        }
    }

    let events = Arc::new(Events(Mutex::new(io::stdout())));
    let fail = |events: &Events, reason: String| -> ! {
        events.send(Event::Error(reason));
        std::process::exit(1);
    };

    // SAFETY: nothing else is running in this process yet.
    if let Err(err) = unsafe { contain() } {
        fail(&events, format!("cannot contain the media worker: {err}"));
    }

    if let Err(err) = gst::init() {
        fail(&events, format!("GStreamer would not start: {err}"));
    }

    // SAFETY: the parent guarantees this descriptor is open and read-write.
    let ring = Arc::new(Mutex::new(Ring {
        file: unsafe { std::fs::File::from_raw_fd(FRAMES_FD) },
        size: (0, 0),
        next_slot: 0,
        seq: 0,
    }));

    // The file is opened by GStreamer through its descriptor rather than a
    // path: the worker never learns where the file is, and nothing can be
    // swapped under it.
    let pipeline = match build(
        max_width,
        max_height,
        Arc::clone(&events),
        Arc::clone(&ring),
    ) {
        Ok(pipeline) => pipeline,
        Err(err) => fail(&events, err),
    };

    // Preroll now, so the first frame and the duration are known before the
    // host asks for anything.
    if pipeline.set_state(gst::State::Paused).is_err() {
        fail(&events, "the file could not be opened for playback".into());
    }

    // Commands come in on their own thread and act on the pipeline directly;
    // GStreamer is thread-safe for everything done here.
    {
        let pipeline = pipeline.clone();
        let events = Arc::clone(&events);
        std::thread::spawn(move || read_commands(pipeline, events));
    }

    let bus = pipeline.bus().expect("a pipeline has a bus");
    let mut playing = false;
    loop {
        let message = bus.timed_pop(gst::ClockTime::from_mseconds(100));
        let Some(message) = message else {
            if playing {
                if let Some(position) = pipeline.query_position::<gst::ClockTime>() {
                    events.send(Event::Position(Duration::from_nanos(position.nseconds())));
                }
            }
            continue;
        };
        use gst::MessageView;
        match message.view() {
            MessageView::Eos(_) => {
                playing = false;
                events.send(Event::Ended);
            }
            MessageView::Error(err) => {
                let reason = match err.debug() {
                    Some(debug) if std::env::var_os("OTTO_MEDIA_TRACE").is_some() => {
                        format!("{} ({debug})", err.error())
                    }
                    _ => err.error().to_string(),
                };
                fail(&events, reason);
            }
            MessageView::StateChanged(change)
                if message.src().is_some_and(|src| *src == pipeline) =>
            {
                match change.current() {
                    gst::State::Playing => {
                        playing = true;
                        events.send(Event::Playing);
                    }
                    gst::State::Paused => {
                        playing = false;
                        events.send(Event::Paused);
                    }
                    _ => {}
                }
            }
            MessageView::AsyncDone(_) | MessageView::DurationChanged(_) => {
                // The duration is often only known once prerolled, and
                // sometimes only later than that.
                let (width, height) = ring.lock().unwrap().size;
                if width > 0 {
                    events.send(Event::Ready {
                        width,
                        height,
                        duration: duration_of(&pipeline),
                    });
                }
                if let Some(position) = pipeline.query_position::<gst::ClockTime>() {
                    events.send(Event::Position(Duration::from_nanos(position.nseconds())));
                }
            }
            _ => {}
        }
    }
}

fn duration_of(pipeline: &gst::Element) -> Option<Duration> {
    pipeline
        .query_duration::<gst::ClockTime>()
        .map(|d| Duration::from_nanos(d.nseconds()))
}

/// `playbin3` with our own video sink: convert and scale into RGBx no larger
/// than the host asked for, then hand each frame to the ring.
fn build(
    max_width: u32,
    max_height: u32,
    events: Arc<Events>,
    ring: Arc<Mutex<Ring>>,
) -> Result<gst::Element, String> {
    let playbin = gst::ElementFactory::make("playbin3")
        .build()
        .or_else(|_| gst::ElementFactory::make("playbin").build())
        .map_err(|_| "no playbin element: is gst-plugins-base installed?".to_string())?;

    let sink = gst::parse::bin_from_description(
        &format!(
            "videoconvert ! videoscale ! \
             video/x-raw,format=RGBx,width=(int)[1,{max_width}],height=(int)[1,{max_height}],pixel-aspect-ratio=(fraction)1/1 ! \
             appsink name=frames sync=true max-buffers=2 drop=true"
        ),
        true,
    )
    .map_err(|err| format!("cannot build the video sink: {err}"))?;
    let appsink = sink
        .by_name("frames")
        .and_then(|element| element.downcast::<gst_app::AppSink>().ok())
        .ok_or("the video sink has no appsink")?;

    // A preroll frame arrives while the pipeline is only PAUSED — which is
    // how a preview starts when the host does not autoplay. Delivering it too
    // is what lets a paused player show its first frame rather than black.
    let preroll_events = Arc::clone(&events);
    let preroll_ring = Arc::clone(&ring);
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                deliver(&sample, &events, &ring, appsink);
                Ok(gst::FlowSuccess::Ok)
            })
            .new_preroll(move |appsink| {
                let sample = appsink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                deliver(&sample, &preroll_events, &preroll_ring, appsink);
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    playbin.set_property("video-sink", &sink);
    playbin.set_property("uri", format!("file:///proc/self/fd/{FILE_FD}"));
    Ok(playbin)
}

/// One decoded frame: into the ring, then announced.
fn deliver(sample: &gst::Sample, events: &Events, ring: &Mutex<Ring>, appsink: &gst_app::AppSink) {
    let Some(buffer) = sample.buffer() else {
        return;
    };
    let Some(caps) = sample.caps() else {
        return;
    };
    let Some(structure) = caps.structure(0) else {
        return;
    };
    let (Ok(width), Ok(height)) = (
        structure.get::<i32>("width"),
        structure.get::<i32>("height"),
    ) else {
        return;
    };
    if width <= 0 || height <= 0 {
        return;
    }
    let (width, height) = (width as u32, height as u32);

    let Ok(map) = buffer.map_readable() else {
        return;
    };
    let mut ring = ring.lock().unwrap();
    if ring.size != (width, height) {
        if let Err(err) = ring.resize(width, height) {
            events.send(Event::Error(format!("cannot size the frame ring: {err}")));
            std::process::exit(1);
        }
        let pipeline = appsink
            .parent()
            .and_then(|bin| bin.parent())
            .and_then(|element| element.downcast::<gst::Element>().ok());
        events.send(Event::Ready {
            width,
            height,
            duration: pipeline.as_ref().and_then(duration_of),
        });
    }

    // Rows are usually tightly packed; when the buffer carries padding the
    // stride is whatever divides its size, which is right for every
    // single-plane RGB layout GStreamer produces.
    let tight = (width * protocol::BYTES_PER_PIXEL) as usize;
    let stride = if map.len() >= tight * height as usize {
        map.len() / height as usize
    } else {
        return;
    };
    let position = buffer
        .pts()
        .map(|pts| Duration::from_nanos(pts.nseconds()))
        .unwrap_or_default();
    match ring.write(map.as_slice().chunks(stride)) {
        Ok((slot, seq)) => events.send(Event::Frame {
            slot,
            seq,
            position,
        }),
        Err(err) => {
            events.send(Event::Error(format!("cannot write a frame: {err}")));
            std::process::exit(1);
        }
    }
}

fn read_commands(pipeline: gst::Element, events: Arc<Events>) {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Some(command) = Command::parse(&line) else {
            continue;
        };
        match command {
            Command::Play => {
                let _ = pipeline.set_state(gst::State::Playing);
            }
            Command::Pause => {
                let _ = pipeline.set_state(gst::State::Paused);
            }
            Command::Seek { position, accurate } => {
                let flags = if accurate {
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE
                } else {
                    gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT
                };
                let _ = pipeline.seek_simple(
                    flags,
                    gst::ClockTime::from_nseconds(position.as_nanos() as u64),
                );
            }
            Command::Volume(volume) => {
                pipeline.set_property("volume", volume.clamp(0.0, 1.0));
                if pipeline.has_property("mute") {
                    pipeline.set_property("mute", volume <= 0.0);
                }
            }
            Command::Quit => break,
        }
    }
    // Either told to quit or the host is gone. Tear the pipeline down so the
    // audio device is released before exiting.
    let _ = pipeline.set_state(gst::State::Null);
    drop(events);
    std::process::exit(0);
}

/// Drop what the worker will not need.
///
/// The same shape as Quick View's decode sandbox, minus the limits a media
/// stack cannot live under: no `RLIMIT_FSIZE` (GStreamer writes its plugin
/// registry cache) and a much larger address-space ceiling (hardware
/// decoders map device memory generously).
///
/// # Safety
///
/// Call before any thread exists.
unsafe fn contain() -> io::Result<()> {
    if libc::chdir(c"/".as_ptr()) != 0 {
        return Err(io::Error::last_os_error());
    }
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
        return Err(io::Error::last_os_error());
    }
    set_limit(libc::RLIMIT_AS, 8 * 1024 * 1024 * 1024)?;
    set_limit(libc::RLIMIT_NOFILE, 512)?;
    set_limit(libc::RLIMIT_CORE, 0)?;
    // Advisory, as in the decoder: a kernel that refuses unprivileged
    // namespaces is not a reason to refuse to play.
    if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) != 0 {
        libc::unshare(libc::CLONE_NEWNET);
    }
    Ok(())
}

fn set_limit(resource: libc::__rlimit_resource_t, value: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: `limit` is fully initialised and `resource` is a valid constant.
    if unsafe { libc::setrlimit(resource, &limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
