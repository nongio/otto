# otto-rdp

`otto-rdp` serves an Otto output over RDP and injects the remote client's mouse and keyboard back into it. It captures a virtual output's PipeWire stream, or a physical output through `org.otto.ScreenCast`, encodes it to H.264 on the GPU (or sends bitmaps) and speaks RDP with ironrdp. It is a standalone process that runs next to Otto on its Wayland socket; the compositor does not start it.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** `otto-dbus` (the `org.otto.ScreenCast` proxies), `otto-foundations` (the state directory for the TLS certificate).
- **External:** PipeWire, GStreamer with `pipewiresrc` and a hardware H.264 encoder (`vah264enc` through VA-API, or `nvh264enc`), EGL and GL on a render node for tiled dmabufs. From the compositor: `zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1` and `xdg-output`. It publishes a StatusNotifierItem while someone is watching. The RDP stack is `ironrdp-server` 0.13 with its EGFX, DVC and SVC crates, and TLS is rustls.
- GStreamer raises the toolchain: building otto-rdp needs Rust 1.96, above the workspace minimum.

## Develop

- `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-rdp -- --output virtual-1` serves a virtual output; `--connector eDP-1` serves a physical one. `--help` lists every option.
- `./run-rdp.sh` (repository root) starts Otto and the bridge together on a VT with full logs.
- Environment: `OTTO_RDP_FPS`, `OTTO_RDP_BITRATE` (kbps), `OTTO_RDP_H264_ENCODER`, `OTTO_RDP_RENDER_NODE`, `OTTO_RDP_DUMP` (writes the first captured frame). Logs follow `RUST_LOG`.
- Tests: `cargo test -p otto-rdp`. The compositor half (the globals and requests the bridge uses) is `cargo test --features headless --test rdp_bridge`.
- CI builds and lints it in its own `rdp` job, which installs the GStreamer development packages.

## Code map

- [src/main.rs](../../components/otto-rdp/src/main.rs): options, choosing the source, starting the server.
- [src/discover.rs](../../components/otto-rdp/src/discover.rs): finds a virtual output's PipeWire node by its `otto.output.name` property.
- [src/screencast.rs](../../components/otto-rdp/src/screencast.rs): a physical output's node through `org.otto.ScreenCast`.
- [src/pipewire_capture.rs](../../components/otto-rdp/src/pipewire_capture.rs) and [src/gpu_import.rs](../../components/otto-rdp/src/gpu_import.rs): raw frames for the bitmap path (also the fallback when a client turns AVC off), reading back dmabufs the CPU cannot map.
- [src/h264.rs](../../components/otto-rdp/src/h264.rs) and [src/egfx.rs](../../components/otto-rdp/src/egfx.rs): the GStreamer encoder and the EGFX (AVC420) channel.
- [src/rdp.rs](../../components/otto-rdp/src/rdp.rs): ironrdp-server glue. [src/wl_input.rs](../../components/otto-rdp/src/wl_input.rs): input injection. [src/tls.rs](../../components/otto-rdp/src/tls.rs): the self-signed certificate. [src/indicator.rs](../../components/otto-rdp/src/indicator.rs): the sharing indicator.

## Rules

- `ironrdp-egfx`, `ironrdp-dvc` and `ironrdp-svc` versions must match the ones `ironrdp-server` pulls in.
- Decide AVC or bitmap from the client's advertised capabilities, not from what the server confirms: ironrdp confirms AVC even to clients that set `AVC_DISABLED`, and those clients then drop the connection.
- There is no authentication. The default listen address is `127.0.0.1:3389`; keep it on localhost by default.
- GStreamer and PipeWire are banned crates in `deny.toml`; otto-rdp is one of the listed wrappers.

## See also

- [RDP bridge for virtual outputs](rdp-virtual-output.md)
- [Remote-desktop indicator](remote-desktop-indicator.md)
- [RDP bridge spec](../../specs/rdp-bridge.md)
- [Remote Desktop user guide](../user/remote-desktop.md)
- [Screenshare](screenshare.md)
