# otto-canvas

The Agents panel in Otto's side canvas: a prompt field over the list of `otto-agents` sessions. It is a Wayland client that places one frosted item in the compositor's canvas column through `otto-canvas-v1`. It hands the work on to the launcher: opening a session runs `otto-launcher --session <URI>`, and asking runs `otto-launcher --ask --send -- <request>`.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies
- **Internal:** `otto-agents-kit` (session feed, rows, field keys, opener), `otto-kit` (the app loop, `CanvasItemSurface`, the frosted material).
- **External:** the compositor's `otto-canvas-v1` protocol ([protocols/otto-canvas-v1.xml](../../protocols/otto-canvas-v1.xml)); a running `otto-agents` for the list; `otto-launcher` on `$PATH`.

## Develop
- Otto starts it from `[[exec_once]]` in the shipped config ([otto_config.example.toml](../../otto_config.example.toml)). To try a change, stop the running one (`pkill -x otto-canvas`) and run `cargo run -p otto-canvas`.
- Under a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-canvas`.
- Show the canvas with a two-finger swipe in from the touchpad's right edge (udev backend only) or a key bound to `CanvasToggle`.
- `RUST_LOG` filters logging (default `info`); `OTTO_AGENTS_URL` picks another service.
- No tests.

## Code map
- `src/main.rs`: the whole app: the panel, its keys, its height negotiation with the column, and the feed while shown.

## Rules
- Hold the session feed only between `shown` and `hidden`. A hidden canvas has no connection.
- Rows, ranking and keys come from `otto-agents-kit`, so the panel matches the launcher's agents mode. Change them there.

## See also
- [Side canvas](side-canvas.md), [otto-agents-kit](otto-agents-kit.md), [spec](../../specs/side-canvas.md), user guide [Side canvas](../user/side-canvas.md).
