# otto-media-kit

Video playback for Otto apps, kept out of otto-kit: a `Player` and its view in the library, and GStreamer in the sandboxed `otto-media-worker` binary built from the same crate. The host talks to the worker over a pipe (commands on stdin, events on stdout) and a shared-memory frame ring.

How it works: [otto-media-kit](../../docs/developer/otto-media-kit.md).

- **Layer:** kit. Used by `otto-files`, `otto-preview` and `otto-peek`.

## Dependencies
- **Internal:** `otto-kit`, for the theme and the draw and hit-test conventions the view and transport bar follow.
- **External:** GStreamer and its decoder plugins, in the worker only (feature `worker`, on by default). `deny.toml` lets only this crate and `otto-rdp` wrap GStreamer. The library draws with `skia-safe`.

## Develop
- Tests: `cargo test -p otto-media-kit` covers the protocol parsers, the transport layout and the view.
- `cargo run -p otto-media-kit --example probe -- FILE` plays four seconds with no display, seeks to 1 s around the two-second mark, and writes the last frame as a PNG (`PROBE_PNG`, default `/tmp/otto-media-probe.png`). Examples land in `target/<profile>/examples/`, away from the worker, so build it (`cargo build -p otto-media-kit --bin otto-media-worker`) and point `OTTO_MEDIA_WORKER` at it.
- `OTTO_MEDIA_TRACE=1` lets the worker's stderr through and adds GStreamer's debug string to error events. `GST_DEBUG=3` works as usual.

## Where things are
- The library a host embeds: `src/player.rs` (spawn the worker, commands, events, frames), `src/transport.rs` (the control bar) and `src/view.rs` (the frame above it).
- `src/protocol.rs` is the pipe and ring contract both halves share.
- `src/bin/otto-media-worker.rs` is the worker and its self-containment (`contain()`).

## Rules
- Hosts depend on the library with `default-features = false`, so GStreamer never enters their link.
- The worker's descriptors are fixed: the media file on 3, the frame ring on 4. `protocol.rs` holds the grammar and both parsers; its tests keep them round-tripping.
- The host clears the worker's environment and passes a whitelist. A new variable the worker needs goes on that list.

## See also
- [File previews](../../docs/developer/file-previews.md), [Peek spec](../../specs/peek.md#video-playback).
- [otto-kit](../otto-kit/AGENTS.md), [otto-peek](../otto-peek/AGENTS.md).
