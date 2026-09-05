//! Media playback for Otto applications.
//!
//! otto-kit draws things; it does not decode video, and it should not — a
//! media stack is an order of magnitude more code than the toolkit it would
//! be bolted onto, and one that parses untrusted bytes. This crate is that
//! stack, kept out of the toolkit and out of every application binary:
//!
//! * [`player::Player`] — the host-side handle. It runs `otto-media-worker`
//!   as a separate, contained process, feeds it commands (play, pause, seek,
//!   volume) and receives its frames through one shared buffer. The host
//!   never links GStreamer and never touches the file's bytes.
//! * [`view`] and [`transport`] — the drawing half, canvas-pure in the
//!   toolkit's draw/hit-test style: given a rect, a [`Player`](player::Player)
//!   and a theme, paint the current frame and the controls, and say what is
//!   under a point.
//! * [`protocol`] — the pipe and frame-buffer contract between the two, in
//!   one place so neither side can drift.
//!
//! The worker is `otto-media-worker`, built from this crate behind the
//! `worker` feature (on by default), and found at run time next to the host's
//! own executable, on `PATH`, or wherever `OTTO_MEDIA_WORKER` points.
//!
//! Why a process and not a thread: a demuxer fed a hostile file crashes, and
//! a crash in a thread takes the file browser with it. The worker also
//! carries the same containment Quick View's decoder does — no network, no
//! new privileges, hard descriptor and memory limits — for the same reason.

pub mod player;
pub mod protocol;
pub mod transport;
pub mod view;

pub use player::{Frame, Options, Playback, Player, State};
