# otto-audio

Audio: the PipeWire graph as a patchbay, and the sound server's rate and buffer size. Two binaries:
- `otto-audio`, the window. The Graph section shows every node as a card and links as wires: drag from a port to a port to link, select a wire to save, forget or remove it. The Devices section switches each sound device's profile, ports and Bluetooth codec through the device's params, which the server keeps. The Engine section forces the sample rate and quantum through the `settings` metadata. The Virtual devices section lists the outputs, inputs and loopbacks kept in the routing file and adds, renames and removes them.
- `otto-audio-service`, a headless daemon. It reads the saved routing and links both ends of every saved link whenever they exist and are not linked, and puts forced engine settings back when the server starts. It makes the virtual devices the file lists and removes ones taken out of it; they last while it runs.

The two share nothing at run time but the routing file, `$XDG_CONFIG_HOME/otto/audio/routing.toml`. The window writes it; the service reads it again whenever it changes. The window tells whether the service runs by looking for a PipeWire client named `otto-audio-service`.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` (window, drawing, controls, i18n, logging, the XDG state directory), `otto-audio-graph` (the graph, its connection, identity and the routing file).
- **External:** a PipeWire server. `systemctl --user` for the Start buttons (the service unit, PipeWire's units). `libc` for the service's inotify watch. `toml` and `serde` for the card layout in `$XDG_STATE_HOME/otto/audio/layout.toml`.

## Develop
- `cargo run -p otto-audio` opens the window; against a dev compositor, `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-audio`.
- `cargo run -p otto-audio -- --png out.png [--engine | --devices | --virtual]` draws the window offscreen, light and dark side by side, from the live graph. It only listens. With `--virtual` and no virtual devices saved it shows three sample ones (Stream mix, Call input, Mic monitor); run your own service on a temp file naming those three to see their live state.
- `cargo run -p otto-audio --bin otto-audio-service` runs the service; `--routing PATH` keeps another file instead of the person's. Its unit is `otto-audio-service.service`, off until enabled with `systemctl --user enable --now otto-audio-service`.
- To try virtual devices, run your own service on a temp file (`--routing /tmp/…/routing.toml`) with `[[virtual]]` entries; everything it made goes when it stops.
- To try linking, make throwaway nodes: `pactl load-module module-null-sink sink_name=otto-audio-test-a`, and `pactl unload-module ID` afterwards.
- Logs: `RUST_LOG=otto_audio=debug,otto_audio_service=debug`. Tests: `cargo test -p otto-audio`.

## Where things are
- `src/main.rs` is the app: the window, the PipeWire inbox drained in `on_update`, keys and pinch. `src/preview.rs` is `--png`.
- `src/ui/mod.rs` is the chrome, pointer handling and the actions; each section has its own file: `ui/graph_pane.rs` (the toolbar, banner and details panel), `ui/devices_pane.rs` (the device cards and what each offers) `ui/engine_pane.rs`, and `ui/virtual_pane.rs` (the Virtual devices form, with otto-kit's `TextInput` for the name fed by `Ui::text_key` before the window's own keys, and its cards). A pop-up a press asks for is a `MenuRequest` that `main.rs` opens with the kit's `DropdownMenu`.
- `src/graph_view.rs` (in the lib, with its tests) lays the canvas out: columns, folded rows, wires, hit-testing and drawing.
- `src/keeper.rs` decides what the service links; `src/bin/otto-audio-service.rs` runs it, watches the routing file and hands its virtual devices to `Connection::keep_virtual_devices`.
- `src/layout.rs` remembers card positions by node identity.

## Rules
- Nothing polls. The window repaints when an event arrives or the person does something; the service acts on graph events and file changes.
- Every control records its rect and action while it is drawn (`targets` in `ui/mod.rs`), and presses test that list.
- Removing a saved wire forgets it in the routing file before unlinking, so the service does not put it back.
- The window never makes virtual devices itself: it writes the file and the service makes them.
- Do not link or unlink the person's nodes while testing, and do not change their engine settings for long; use null sinks. Never switch the profile or port of their real devices from a test: the `--png` preview only reads them.
- The behaviour is specified in `specs/audio-routing.md`. A change to it updates the spec.

## See also
- [Audio routing spec](../../specs/audio-routing.md), [otto-audio-graph](../otto-audio-graph/AGENTS.md), [otto-kit](../otto-kit/AGENTS.md).
