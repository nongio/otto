//! What the host and the worker agree on.
//!
//! Three channels, all of them plain:
//!
//! * **commands** — lines on the worker's stdin, host → worker;
//! * **events** — lines on the worker's stdout, worker → host;
//! * **frames** — a memfd the host creates and the worker sizes, holding a
//!   small ring of decoded frames. The worker announces each new frame with
//!   an event naming the slot it landed in.
//!
//! Lines rather than a binary framing because they are debuggable with
//! `cat`, and because the volume is tiny: tens of lines a second, with the
//! pixels going through shared memory instead.
//!
//! Descriptors are fixed rather than negotiated, so nothing has to be told a
//! number and nothing can be substituted after the fact.

use std::time::Duration;

/// The media file, read-only, on this descriptor in the worker.
pub const FILE_FD: i32 = 3;
/// The frame ring, read-write, on this descriptor in the worker.
pub const FRAMES_FD: i32 = 4;

/// Frames in the ring. Three is one being written, one being read, and one
/// of slack, which is what keeps the writer from lapping the reader at the
/// rate a video decodes and a host paints.
pub const SLOTS: u32 = 3;

/// Bytes per pixel: RGBx, the alpha byte unused. Video has no alpha and a
/// format with a spare byte converts fastest.
pub const BYTES_PER_PIXEL: u32 = 4;

/// The ring's header size. Slot data starts here.
pub const HEADER_BYTES: u64 = 4096;

/// A frame's whole size in the ring, rows tightly packed.
pub fn slot_bytes(width: u32, height: u32) -> u64 {
    width as u64 * height as u64 * BYTES_PER_PIXEL as u64
}

/// Where a slot's pixels start.
pub fn slot_offset(slot: u32, width: u32, height: u32) -> u64 {
    HEADER_BYTES + slot as u64 * slot_bytes(width, height)
}

/// The ring's total size for a frame size.
pub fn ring_bytes(width: u32, height: u32) -> u64 {
    HEADER_BYTES + SLOTS as u64 * slot_bytes(width, height)
}

/// Host → worker.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play,
    Pause,
    /// Seek to a position. `accurate` asks for the exact frame, which is slow
    /// on long GOPs; a scrub in progress passes `false` and lands on a
    /// keyframe instead.
    Seek {
        position: Duration,
        accurate: bool,
    },
    /// 0.0 … 1.0.
    Volume(f64),
    Quit,
}

impl Command {
    pub fn encode(&self) -> String {
        match self {
            Command::Play => "play\n".into(),
            Command::Pause => "pause\n".into(),
            Command::Seek { position, accurate } => format!(
                "seek {} {}\n",
                position.as_nanos(),
                if *accurate { "accurate" } else { "fast" }
            ),
            Command::Volume(volume) => format!("volume {:.4}\n", volume.clamp(0.0, 1.0)),
            Command::Quit => "quit\n".into(),
        }
    }

    pub fn parse(line: &str) -> Option<Command> {
        let mut words = line.split_whitespace();
        Some(match words.next()? {
            "play" => Command::Play,
            "pause" => Command::Pause,
            "seek" => Command::Seek {
                position: Duration::from_nanos(words.next()?.parse().ok()?),
                accurate: words.next() == Some("accurate"),
            },
            "volume" => Command::Volume(words.next()?.parse().ok()?),
            "quit" => Command::Quit,
            _ => return None,
        })
    }
}

/// Worker → host.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The stream is known: frame size as it will arrive in the ring, and
    /// the duration when the container declares one. The ring has been sized
    /// for these dimensions by the time this is sent. Sent again if the
    /// stream changes size mid-way, which re-sizes the ring.
    Ready {
        width: u32,
        height: u32,
        duration: Option<Duration>,
    },
    /// A frame landed in `slot`. `seq` counts frames from one; `position` is
    /// the frame's presentation time.
    Frame {
        slot: u32,
        seq: u64,
        position: Duration,
    },
    /// Where playback is, sent periodically while playing and after a seek.
    Position(Duration),
    Playing,
    Paused,
    Ended,
    /// Playback cannot continue. The worker exits after sending it.
    Error(String),
}

impl Event {
    pub fn encode(&self) -> String {
        match self {
            Event::Ready {
                width,
                height,
                duration,
            } => format!(
                "ready {width} {height} {}\n",
                duration.map(|d| d.as_nanos() as i128).unwrap_or(-1)
            ),
            Event::Frame {
                slot,
                seq,
                position,
            } => format!("frame {slot} {seq} {}\n", position.as_nanos()),
            Event::Position(position) => format!("position {}\n", position.as_nanos()),
            Event::Playing => "playing\n".into(),
            Event::Paused => "paused\n".into(),
            Event::Ended => "ended\n".into(),
            Event::Error(reason) => format!("error {}\n", reason.replace(['\n', '\r'], " ")),
        }
    }

    pub fn parse(line: &str) -> Option<Event> {
        let line = line.trim_end();
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let mut words = rest.split_whitespace();
        Some(match word {
            "ready" => {
                let width = words.next()?.parse().ok()?;
                let height = words.next()?.parse().ok()?;
                let nanos: i128 = words.next()?.parse().ok()?;
                Event::Ready {
                    width,
                    height,
                    duration: (nanos >= 0).then(|| Duration::from_nanos(nanos as u64)),
                }
            }
            "frame" => Event::Frame {
                slot: words.next()?.parse().ok()?,
                seq: words.next()?.parse().ok()?,
                position: Duration::from_nanos(words.next()?.parse().ok()?),
            },
            "position" => Event::Position(Duration::from_nanos(words.next()?.parse().ok()?)),
            "playing" => Event::Playing,
            "paused" => Event::Paused,
            "ended" => Event::Ended,
            "error" => Event::Error(rest.to_string()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        for command in [
            Command::Play,
            Command::Pause,
            Command::Seek {
                position: Duration::from_millis(1500),
                accurate: true,
            },
            Command::Seek {
                position: Duration::from_secs(3),
                accurate: false,
            },
            Command::Volume(0.25),
            Command::Quit,
        ] {
            assert_eq!(Command::parse(&command.encode()), Some(command));
        }
    }

    #[test]
    fn events_round_trip() {
        for event in [
            Event::Ready {
                width: 1280,
                height: 720,
                duration: Some(Duration::from_secs(5)),
            },
            Event::Ready {
                width: 1,
                height: 1,
                duration: None,
            },
            Event::Frame {
                slot: 2,
                seq: 17,
                position: Duration::from_millis(566),
            },
            Event::Position(Duration::from_secs(1)),
            Event::Playing,
            Event::Paused,
            Event::Ended,
            Event::Error("no decoder for video/x-h266".into()),
        ] {
            assert_eq!(Event::parse(&event.encode()), Some(event));
        }
    }

    #[test]
    fn an_error_stays_one_line() {
        let event = Event::Error("first\nsecond".into());
        assert_eq!(event.encode().matches('\n').count(), 1);
    }

    #[test]
    fn slots_do_not_overlap() {
        let (w, h) = (640, 480);
        assert_eq!(slot_offset(0, w, h), HEADER_BYTES);
        assert_eq!(
            slot_offset(1, w, h) - slot_offset(0, w, h),
            slot_bytes(w, h)
        );
        assert_eq!(ring_bytes(w, h), slot_offset(SLOTS, w, h));
    }
}
