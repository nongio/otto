# otto-peek

Peek, the Space-bar preview, as a library the file views embed. A host draws the preview into a subsurface of its own window, so Peek is not a service and has no window of its own. Untrusted files are parsed only in a sandboxed decode worker, which is the host's own binary re-executed. The crate also holds the shared thumbnail cache and a thumbnailer for apps that show files.

It sits in the **kit** layer (see [Architecture Rules](architecture.md)) and is used by `otto-files` (browser, picker, desk), `otto-preview` (one file in a window), `otto-launcher` and `otto-stash` (thumbnails of attached files).

## Dependencies
- **Internal:** `otto-kit` (the drawing half lives in `otto_kit::preview`, canvas-pure so a host draws it into its own surface), `otto-md-kit` (Markdown documents), `otto-media-kit` without default features (playback in the embedded session; the GStreamer worker is the separate `otto-media-worker` binary).
- **External:** no PDF, video or HEIF library is linked. The worker runs programs already on the system and uses the first it finds: `pdftoppm`, `pdftocairo`, `mutool` or `gs` for PDF; `ffmpegthumbnailer` or `ffmpeg` for video frames; `vipsthumbnail` or `magick` for HEIF photographs. Text in pictures runs `tesseract` by default. Thumbnails follow the freedesktop Thumbnail Managing Standard.

## Develop
- The `otto-peek` binary needs no display: `cargo run -p otto-peek -- --describe PATH` prints what the decoders made of a file; `--render OUT.png PATH` draws it through the same `otto_kit::preview::draw` a host uses; `--filmstrip OUT.png PATH` samples the opening animation. `--help` lists the sizing flags (`--page`, `--zoom`, `--width`, `--dark`, ...).
- `cargo run -p otto-peek -- --sandbox-selftest` reports which containment measures hold on this machine.
- `cargo run -p otto-peek --example ocr_probe -- <picture> [recogniser command]` runs text recognition through the worker.
- To see it embedded, run a host: `cargo run -p otto-files`, select a file, press Space.
- `RUST_LOG` filters logging; the worker gets `RUST_LOG` or `error`. `OTTO_PEEK_BOUNCE` tunes the opening animation.
- Tests: `cargo test -p otto-peek`.

## Code map
- `src/lib.rs`: what lives where, and `run_worker_if_requested`.
- `src/session.rs`: the embedded preview a host drives: zoom, pan, pages, selection, links, playback, with a cache of recent decodes (`src/decode_cache.rs`).
- `src/spawn.rs`: the host side of decoding: open the file, spawn the worker, enforce the deadline. `src/payload.rs` carries the result across.
- `src/decode/`: the decoders, one module per kind (`image`, `pdf`, `media`, `markdown`, `text`, `listing`, `external`). They run only inside the worker.
- `src/sandbox.rs`: the worker's containment and its self-test.
- `src/ocr.rs`, `src/ocrcache.rs`, `src/badge.rs`: text in pictures. `src/config.rs`: the `[peek]` section of `~/.config/otto/files.toml`.
- `src/thumbcache.rs`, `src/thumbnailer.rs`: the shared thumbnail cache and the off-thread thumbnailer.
- `src/opening.rs` (entrance geometry), `src/imagesize.rs` (size from the header), `src/uri.rs`.

## Rules
- Every host calls `otto_peek::run_worker_if_requested()` first thing in `main`, before any thread or Wayland connection. Without it the worker re-executes the host as a host.
- Parse untrusted file content only in `src/decode/`, inside the worker. Nothing on the host side interprets file bytes.
- The sandbox caps memory, CPU, writes, network and descriptors, but does not stop the worker reading other files. `src/sandbox.rs` says why; keep that comment true when changing containment.
- Add a decoder for a new format by running an installed program from the worker (`decode/external.rs`), not by linking a large library.

## See also
- [File previews](file-previews.md) (the pieces, the payload, the worker, thumbnails), [Text in pictures](peek-ocr.md), [otto-media-kit](otto-media-kit.md), [otto-files](otto-files.md), [otto-preview](otto-preview.md).
- Specs: [Peek](../../specs/peek.md), [Peek OCR](../../specs/peek-ocr.md).
