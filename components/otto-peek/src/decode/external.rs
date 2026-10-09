//! Pictures made by programs already on the system, run over the file the
//! worker holds.
//!
//! The PDF path hands its rasteriser the document's bytes on stdin. That does
//! not scale to a two-gigabyte video, and a video decoder has to seek anyway —
//! an MP4's index is as often at the end of the file as at the front. So here
//! the child's stdin *is* the file: a duplicate of the worker's one read-only
//! descriptor, which the tool opens as `/dev/stdin`. That reopens the same
//! file through `/proc/self/fd/0`, so it is seekable, and it is still never a
//! name the child resolves in the filesystem — nothing can be substituted
//! between the worker's stat and the tool's read.
//!
//! Everything else is the PDF arrangement: the tool inherits the worker's
//! rlimits and namespaces, writes one PNG to a pipe, and its complaints go
//! nowhere. A table of tools is tried in order and the first that answers
//! wins, so a format is supported by whichever program a distribution ships
//! and a new one is a table row. See `specs/peek.md`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};

use skia_safe::{Codec, Data};

use crate::payload::Pixels;

use super::on_path;

/// What a tool is asked to make.
#[derive(Debug, Clone, Copy)]
pub struct Ask {
    /// The box the picture must fit inside, in pixels. A source smaller than
    /// the box is not grown into it.
    pub width: u32,
    pub height: u32,
    /// Where in a video to take the frame, in seconds. `None` leaves it to
    /// the tool.
    pub at: Option<f64>,
}

/// A program, and how to ask it for a PNG of `/dev/stdin` on stdout.
pub struct Tool {
    /// The binary, looked up on `PATH`.
    pub command: &'static str,
    pub args: fn(&Ask) -> Vec<String>,
}

/// The most PNG a tool may hand back. Every tool here is asked for a box and
/// writes a picture that size; this is the backstop for one that does not.
const MAX_OUTPUT: u64 = 256 * 1024 * 1024;

/// The first picture one of `tools` makes of `file`. `None` when none of them
/// is installed, or every one that is failed.
pub fn picture(tools: &[Tool], file: &mut File, ask: &Ask) -> Option<Pixels> {
    tools
        .iter()
        .filter(|tool| on_path(tool.command))
        .find_map(|tool| decode_png(&run(tool.command, &(tool.args)(ask), file)?))
}

/// One frame of video as a PNG, from `ffmpeg`: the row a poster or a still
/// falls back on when nothing more specific is installed.
///
/// Single-threaded on both sides. Its default is a thread per core for the
/// decoder and again for the encoder, and under the worker's address-space
/// cap the encoder's pool is what fails to start.
pub fn ffmpeg_frame(ask: &Ask) -> Vec<String> {
    let mut args: Vec<String> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-threads",
        "1",
    ]
    .map(String::from)
    .to_vec();
    if let Some(at) = ask.at {
        // Before `-i`, so the demuxer seeks rather than decoding up to it.
        args.extend(["-ss".into(), format!("{at:.3}")]);
    }
    args.extend(
        [
            "-i",
            "/dev/stdin",
            "-frames:v",
            "1",
            "-vf",
            // Quoted so the commas inside `min` are not read as the filter
            // chain's own.
            &format!(
                "scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=decrease",
                w = ask.width,
                h = ask.height
            ),
            "-threads",
            "1",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "-",
        ]
        .map(String::from),
    );
    args
}

/// Run one tool with the file as its stdin and collect what it writes.
fn run(command: &str, args: &[String], file: &mut File) -> Option<Vec<u8>> {
    // The duplicate shares this offset, and a tool that reads stdin as a
    // stream rather than reopening it has to start at the beginning.
    file.seek(SeekFrom::Start(0)).ok()?;
    let stdin = file.try_clone().ok()?;
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::piped())
        // A tool's complaints belong in the log, not interleaved with the
        // payload on our own stdout.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut out = Vec::new();
    let read = child
        .stdout
        .take()
        .map(|sink| sink.take(MAX_OUTPUT).read_to_end(&mut out));
    // Closing our end first: a tool still writing past the cap gets a broken
    // pipe rather than a reader that will never come.
    let _ = child.wait();
    let _ = file.seek(SeekFrom::Start(0));
    read?.ok()?;
    (!out.is_empty()).then_some(out)
}

/// A PNG from one of these tools, as pixels.
pub fn decode_png(png: &[u8]) -> Option<Pixels> {
    let mut codec = Codec::from_data(Data::new_copy(png))?;
    let info = codec
        .info()
        .with_color_type(skia_safe::ColorType::RGBA8888)
        .with_alpha_type(skia_safe::AlphaType::Premul);
    let dimensions = codec.dimensions();
    let image = codec.get_image(info, None).ok()?;
    super::image::to_pixels(&image, dimensions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_seeks_before_the_input_and_never_grows_the_frame() {
        let args = ffmpeg_frame(&Ask {
            width: 320,
            height: 240,
            at: Some(4.5),
        });
        let seek = args.iter().position(|a| a == "-ss").expect("a seek");
        let input = args.iter().position(|a| a == "-i").expect("an input");
        assert!(seek < input);
        assert_eq!(args[seek + 1], "4.500");
        assert!(args
            .iter()
            .any(|a| a.starts_with("scale='min(320,iw)':'min(240,ih)'")));

        let from_start = ffmpeg_frame(&Ask {
            width: 320,
            height: 240,
            at: None,
        });
        assert!(!from_start.iter().any(|a| a == "-ss"));
    }
}
