# otto-rdp

`otto-rdp` serves an Otto output over RDP and injects the remote client's mouse and keyboard back into it. It captures a virtual output's PipeWire stream, or a physical output through `org.otto.ScreenCast`, encodes it to H.264 on the GPU (or sends bitmaps) and speaks RDP with ironrdp. It is a standalone process that runs next to Otto on its Wayland socket; the compositor does not start it.

How it works: [RDP bridge for virtual outputs](../../docs/developer/rdp-virtual-output.md).

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-dbus` (the `org.otto.ScreenCast` proxies), `otto-foundations` (the state directory for the TLS certificate).
- **External:** PipeWire; GStreamer with `pipewiresrc` and a hardware H.264 encoder (`vah264enc` through VA-API, or `nvh264enc`); EGL and GL on a render node for tiled dmabufs. From the compositor: `zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1` and `xdg-output`. It publishes a StatusNotifierItem while someone is watching. The RDP stack is `ironrdp-server` 0.13 with its EGFX, DVC and SVC crates; TLS is rustls.
- GStreamer raises the toolchain: building otto-rdp needs Rust 1.96, above the workspace minimum.

## Develop
- `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-rdp -- --output virtual-1` serves a virtual output; `--connector eDP-1` serves a physical one. `--help` lists every option.
- `./run-rdp.sh` (repository root) starts Otto and the bridge together on a VT with full logs.
- Environment: `OTTO_RDP_FPS`, `OTTO_RDP_BITRATE` (kbps), `OTTO_RDP_H264_ENCODER`, `OTTO_RDP_RENDER_NODE`, `OTTO_RDP_DUMP` (writes the first captured frame). Logs follow `RUST_LOG`.
- Tests: `cargo test -p otto-rdp`. The compositor half (the globals and requests the bridge uses) is `cargo test --features headless --test rdp_bridge`.
- CI builds and lints it in its own `rdp` job, which installs the GStreamer development packages.

## Where things are
- `src/main.rs` parses options, chooses the source and starts the server.
- Capture: `src/discover.rs` finds a virtual output's PipeWire node by its `otto.output.name` property; `src/screencast.rs` gets a physical output's node through `org.otto.ScreenCast`.
- Encoding: `src/h264.rs` and `src/egfx.rs` are the GStreamer encoder and the EGFX (AVC420) channel; `src/pipewire_capture.rs` and `src/gpu_import.rs` feed the bitmap path (also the fallback when a client turns AVC off).
- `src/rdp.rs` is the ironrdp-server glue; input injection, TLS and the sharing indicator have a module each.

## Rules
- `ironrdp-egfx`, `ironrdp-dvc` and `ironrdp-svc` versions must match the ones `ironrdp-server` pulls in.
- Decide AVC or bitmap from the client's advertised capabilities, not from what the server confirms: ironrdp confirms AVC even to clients that set `AVC_DISABLED`, and those clients then drop the connection.
- There is no authentication. The default listen address is `127.0.0.1:3389`; keep it on localhost by default.
- GStreamer and PipeWire are banned crates in `deny.toml`; otto-rdp is one of the listed wrappers.

## See also
- [Remote-desktop indicator](../../docs/developer/remote-desktop-indicator.md), [Screenshare](../../docs/developer/screenshare.md).
- [RDP bridge spec](../../specs/rdp-bridge.md), user guide [Remote Desktop](../../docs/user/remote-desktop.md).
