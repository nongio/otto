//! `cargo run -p otto-media-kit --example probe -- FILE`: play a file for a
//! few seconds with no display, printing what the worker reports. The
//! integration check between the host half and the worker.
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("a media file"));
    let mut player =
        otto_media_kit::Player::open(&path, Default::default(), || {}).expect("worker starts");
    let started = Instant::now();
    let mut last_seq = 0;
    let mut frames = 0;
    while started.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
        let state = player.state();
        if state.frame_seq != last_seq {
            frames += state.frame_seq - last_seq;
            last_seq = state.frame_seq;
        }
        if started.elapsed() > Duration::from_secs(2)
            && started.elapsed() < Duration::from_millis(2100)
        {
            player.seek(Duration::from_secs(1), true);
        }
    }
    let state = player.state();
    let frame = player.frame();
    println!(
        "playback={:?} size={:?} duration={:?} position={:?} frames={frames} error={:?}",
        state.playback,
        state.size,
        state.duration,
        state.position(),
        state.error
    );
    if let Some(frame) = frame {
        let image = frame.to_image().expect("frame wraps");
        let png = image
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .unwrap();
        let out = std::env::var("PROBE_PNG").unwrap_or_else(|_| "/tmp/otto-media-probe.png".into());
        std::fs::write(&out, png.as_bytes()).unwrap();
        println!(
            "frame seq={} {}x{} -> {out}",
            frame.seq, frame.width, frame.height
        );
    }
}
