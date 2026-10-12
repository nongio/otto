# otto-peek

Peek, the Space-bar preview, as a library the file views embed. A host draws the preview into a subsurface of its own window, so Peek is not a service and has no window of its own. Untrusted files are parsed only in a sandboxed decode worker, which is the host's own binary re-executed. The crate also holds the shared thumbnail cache and a thumbnailer for apps that show files.

How it works: [File previews](../../docs/developer/file-previews.md).

- **Layer:** kit. Used by `otto-files` (browser, picker, desk), `otto-preview`, `otto-launcher` and `otto-stash` (thumbnails of attached files).

## Dependencies
- **Internal:** `otto-kit` (the drawing half lives in `otto_kit::preview`, canvas-pure), `otto-md-kit` (Markdown documents), `otto-media-kit` without default features (playback; GStreamer stays in `otto-media-worker`).
- **External:** no PDF, video or HEIF library is linked. The worker runs the first program it finds: `pdftoppm`, `pdftocairo`, `mutool` or `gs` for PDF; `ffmpegthumbnailer` or `ffmpeg` for video frames; `vipsthumbnail` or `magick` for HEIF. Text in pictures runs `tesseract`. Thumbnails follow the freedesktop Thumbnail Managing Standard.

## Develop
- The `otto-peek` binary needs no display: `cargo run -p otto-peek -- --describe PATH` prints what the decoders made of a file; `--render OUT.png PATH` draws it through the same `otto_kit::preview::draw` a host uses; `--filmstrip OUT.png PATH` samples the opening animation. `--help` lists the sizing flags.
- `cargo run -p otto-peek -- --sandbox-selftest` reports which containment measures hold on this machine.
- `cargo run -p otto-peek --example ocr_probe -- <picture> [recogniser command]` runs text recognition through the worker.
- To see it embedded, run a host: `cargo run -p otto-files`, select a file, press Space.
- `RUST_LOG` filters logging; the worker gets `RUST_LOG` or `error`. `OTTO_PEEK_BOUNCE` tunes the opening animation.
- Tests: `cargo test -p otto-peek`.

## Where things are
- `src/session.rs` is the embedded preview a host drives (zoom, pan, pages, selection, playback).
- The host side of decoding (spawn, deadline, payload) is `src/spawn.rs` and `src/payload.rs`; the decoders, one per kind, are in `src/decode/`.
- `src/sandbox.rs` is the worker's containment and self-test.
- Text in pictures (`ocr*.rs`, `badge.rs`) and thumbnails (`thumbcache.rs`, `thumbnailer.rs`) sit beside them.

## Rules
- Every host calls `otto_peek::run_worker_if_requested()` first thing in `main`, before any thread or Wayland connection. Without it the worker re-executes the host as a host.
- Parse untrusted file content only in `src/decode/`, inside the worker. Nothing on the host side interprets file bytes.
- The sandbox caps memory, CPU, writes, network and descriptors, but does not stop the worker reading other files. `src/sandbox.rs` says why; keep that comment true when changing containment.
- Add a decoder for a new format by running an installed program from the worker (`decode/external.rs`), not by linking a large library.

## See also
- [File previews](../../docs/developer/file-previews.md), [Text in pictures](../../docs/developer/peek-ocr.md).
- Specs: [Peek](../../specs/peek.md), [Peek OCR](../../specs/peek-ocr.md).
- [otto-media-kit](../otto-media-kit/AGENTS.md), [otto-files](../otto-files/AGENTS.md), [otto-preview](../otto-preview/AGENTS.md).
